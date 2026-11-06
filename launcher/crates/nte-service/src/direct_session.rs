//! 独立SDK宿主与本轮游戏共存, 凭据仅经认证的本机连接交接.
use nte_core::game_auth::{self, Client, Session};
use nte_core::game_profile::{GameProfile, RegionProfile};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct DirectSession {
    control: TcpStream,
    host: Mutex<Child>,
}
impl std::fmt::Debug for DirectSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectSession")
    }
}
impl DirectSession {
    pub fn start(
        client: Client,
        session: Session,
        command: &mut Command,
        log: &std::fs::File,
    ) -> Result<(Arc<Self>, u32), String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "无法创建本机游戏登录连接.".to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let nonce = zeroize::Zeroizing::new(game_auth::nonce().map_err(|e| e.to_string())?);
        command
            .arg(
                listener
                    .local_addr()
                    .map_err(|e| e.to_string())?
                    .port()
                    .to_string(),
            )
            .env("ODDITY_HANDOFF_NONCE", nonce.as_str())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动游戏登录宿主.".to_string())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let accepted = loop {
            match listener.accept() {
                Ok((stream, _)) => break Ok(stream),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                        break Err("游戏登录宿主提前退出.".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                _ => break Err("等待游戏登录宿主超时.".to_string()),
            }
        };
        let mut control = match accepted {
            Ok(s) => s,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let host = Arc::new(Self {
            control: control.try_clone().map_err(|e| e.to_string())?,
            host: Mutex::new(child),
        });
        control
            .set_read_timeout(Some(Duration::from_secs(20)))
            .map_err(|e| e.to_string())?;
        control
            .set_write_timeout(Some(Duration::from_secs(20)))
            .map_err(|e| e.to_string())?;
        let mut received = [0u8; 64];
        control
            .read_exact(&mut received)
            .map_err(|_| "游戏登录宿主未完成握手.".to_string())?;
        // 固定长度比较, 不向错误连接发送游戏票据.
        if received
            .iter()
            .zip(nonce.bytes())
            .fold(0u8, |v, (a, b)| v | (*a ^ b))
            != 0
        {
            return Err("游戏登录连接标识不匹配.".into());
        }
        control
            .write_all(&session.packet().map_err(|e| e.to_string())?)
            .map_err(|_| "无法交接游戏登录状态.".to_string())?;
        let mut pid = [0u8; 4];
        control
            .read_exact(&mut pid)
            .map_err(|_| "游戏登录宿主初始化失败.".to_string())?;
        let pid = u32::from_le_bytes(pid);
        if pid == 0 {
            return Err("游戏登录宿主编号无效.".into());
        }
        // 游戏空闲时不限制查询间隔, 关闭连接会唤醒此读取.
        control.set_read_timeout(None).map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            while let Ok((kind, id, parameters)) = read_query(&mut control) {
                let (code, message, body) = client
                    .sdk_query(&session, kind, parameters)
                    .unwrap_or_else(|_| (-1, "SDK query failed".into(), Vec::new()));
                let Ok(reply) = game_auth::query_reply(kind, id, code, &message, &body) else {
                    break;
                };
                if control.write_all(&reply).is_err() {
                    break;
                }
            }
            let _ = control.shutdown(Shutdown::Both);
        });
        Ok((host, pid))
    }
    pub fn configure_game(
        command: &mut Command,
        profile: &GameProfile,
        region: &RegionProfile,
        root: &Path,
        install: &Path,
        pid: u32,
        patcher_log: &Path,
    ) {
        let (Some(layout), Some(recipe)) = (profile.layout(), profile.launch()) else {
            return;
        };
        let userdata = root.join("UserData");
        command.env("ArcGameP", format!("OdditySdkPipe_{pid}")).env(
            "ArcGameSDKSharePath",
            crate::windows_path(&root.join(recipe.sdk_share_dir)),
        );
        for (name, path) in [
            (
                "_PATCHER_CONFIG_PATH_",
                layout.patcher_config_path(root, region.app_id),
            ),
            ("_PATCHER_USERDATA_PATH_", userdata.clone()),
            ("_PATCHER_RES_PATH_", install.to_path_buf()),
            ("_PATCHER_SDK_PATH_", layout.patcher_sdk_path(root)),
            ("_PATCHER_LOG_PATH_", patcher_log.to_path_buf()),
        ] {
            command.env(name, format!("{}/", crate::windows_path(&path)));
        }
    }
    pub fn close(&self) {
        let _ = self.control.shutdown(Shutdown::Both);
        if let Ok(mut child) = self.host.lock() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
impl Drop for DirectSession {
    fn drop(&mut self) {
        self.close();
    }
}
fn read_query(control: &mut TcpStream) -> Result<(u32, u32, serde_json::Value), String> {
    let mut size = [0u8; 4];
    control
        .read_exact(&mut size)
        .map_err(|_| "SDK连接关闭".to_string())?;
    if u32::from_le_bytes(size) != 0x214 {
        return Err("SDK查询帧长度不符".into());
    }
    let mut frame = [0u8; 0x214];
    control
        .read_exact(&mut frame)
        .map_err(|_| "SDK查询帧被截断".to_string())?;
    let word = |at: usize| u32::from_le_bytes(frame[at..at + 4].try_into().expect("定长SDK帧"));
    if word(0) != 0x6b647331 || word(4) != 40 || word(8) != 0x214 {
        return Err("SDK查询帧头不符".into());
    }
    let end = frame[16..0x210]
        .iter()
        .position(|b| *b == 0)
        .ok_or("SDK查询参数缺少终止符")?;
    let parameters = serde_json::from_slice(&frame[16..16 + end]).map_err(|_| "SDK查询JSON无效")?;
    Ok((word(12), word(0x210), parameters))
}
