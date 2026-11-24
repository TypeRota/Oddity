//! Sunshine 应用注册与串流会话.
//!
//! 应用条目直接写在 Sunshine 的 apps.json 里, 按名字定位, 开关关闭时移除.
//! 串流期间的显示切换由 prep-cmd 与主命令分担, 两者都以会话为界, 不改动 Oddity 的设置.

mod display;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::io::{BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::service::ServiceHandle;

/// 注册在 Sunshine 应用列表里的名字前缀.
const APP_PREFIX: &str = "Oddity - ";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub(crate) enabled: bool,
    pub(crate) supported: bool,
    pub(crate) needs_restore: bool,
    pub(crate) missing: Vec<String>,
    pub(crate) message: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Registration {
    pub(crate) game: String,
    pub(crate) name: String,
}

fn registration_path(game: &str) -> PathBuf {
    crate::paths::config_dir().join(format!("sunshine-{game}.json"))
}

pub(crate) fn load(game: &str) -> Result<Option<Registration>> {
    let path = registration_path(game);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
}

fn save(registration: &Registration) -> Result<()> {
    let path = registration_path(&registration.game);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(registration)?)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

fn forget(game: &str) -> Result<()> {
    let path = registration_path(game);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Sunshine 的配置目录; 与它自身的 XDG 规则一致.
fn sunshine_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("CONFIGURATION_DIRECTORY").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("sunshine"));
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("sunshine"));
    }
    let home = std::env::var_os("HOME").context("无法确定 Sunshine 配置目录")?;
    Ok(PathBuf::from(home).join(".config/sunshine"))
}

/// sunshine.conf 的键值; Sunshine 每行一个 "键 = 值", 井号起注释.
fn config() -> Result<Map<String, Value>> {
    let path = sunshine_dir()?.join("sunshine.conf");
    let mut map = Map::new();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(map);
    };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        map.insert(key.trim().into(), Value::String(value.trim().into()));
    }
    Ok(map)
}

pub(crate) fn config_value(key: &str) -> Result<String> {
    Ok(config()?
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned())
}

/// 应用列表路径; Sunshine 允许在配置里改写.
pub(crate) fn apps_path() -> Result<PathBuf> {
    let custom = config_value("file_apps")?;
    if !custom.is_empty() {
        return Ok(PathBuf::from(custom));
    }
    Ok(sunshine_dir()?.join("apps.json"))
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Applications {
    #[serde(default)]
    pub(crate) apps: Vec<Value>,
    #[serde(flatten)]
    pub(crate) rest: Map<String, Value>,
}

/// 按名字写入一条应用; 已存在时原位替换, 保持用户在 Sunshine 里排好的顺序.
pub(crate) fn upsert(apps: &mut Applications, name: &str, entry: Value) {
    let existing = apps
        .apps
        .iter()
        .position(|app| app.get("name").and_then(Value::as_str) == Some(name));
    match existing {
        Some(index) => apps.apps[index] = entry,
        None => apps.apps.push(entry),
    }
}

pub(crate) fn remove(apps: &mut Applications, name: &str) -> bool {
    let before = apps.apps.len();
    apps.apps
        .retain(|app| app.get("name").and_then(Value::as_str) != Some(name));
    apps.apps.len() != before
}

pub(crate) fn read_apps(path: &Path) -> Result<Applications> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("无法读取 Sunshine 应用列表: {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn write_apps(path: &Path, apps: &Applications) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(apps)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// Sunshine 只在启动时读取应用列表, 改完要让它重新载入.
pub(crate) fn reload_sunshine() {
    // 未在运行时不变更服务状态.
    let _ = Command::new("systemctl")
        .args(["--user", "try-restart", "sunshine"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status();
}

fn supported() -> bool {
    desktop_supported(
        &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}

pub(crate) fn desktop_supported(desktop: &str, wayland: bool) -> bool {
    wayland && desktop.split(':').any(|s| s.eq_ignore_ascii_case("KDE"))
}

fn missing_components() -> Vec<String> {
    ["sunshine", "kscreen-doctor", "gamescope"]
        .into_iter()
        .filter(|name| oddity_platform::process::find_executable(name).is_none())
        .map(str::to_owned)
        .collect()
}

fn active_game(app: &ServiceHandle) -> Result<crate::identity::GameId> {
    app.state
        .settings
        .lock()
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .map(|settings| settings.active_game.clone())
}

fn game_busy(app: &ServiceHandle) -> Result<bool> {
    app.state
        .game
        .lock()
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .map(|game| {
            game.starting
                || game
                    .session
                    .as_ref()
                    .is_some_and(crate::session::session_alive)
        })
}

pub fn status(app: &ServiceHandle) -> Result<Status> {
    let game = active_game(app)?;
    let registration = load(game.as_str())?;
    let missing = missing_components();
    let supported = supported();
    let mut enabled = false;
    let mut message = if !supported {
        "当前仅支持 KDE Plasma Wayland".into()
    } else if !missing.is_empty() {
        "需要先安装 Sunshine 与 Gamescope".into()
    } else {
        "开启后在 Moonlight 中选择游戏即可开始".into()
    };
    if supported && missing.is_empty() {
        if let Some(registration) = registration {
            enabled = read_apps(&apps_path()?).is_ok_and(|apps| {
                apps.apps.iter().any(|app| {
                    app.get("name").and_then(Value::as_str) == Some(registration.name.as_str())
                })
            });
            if let Err(error) = display::verify(&config_value("output_name")?) {
                enabled = false;
                message = error.to_string();
            } else if enabled {
                message = format!(
                    "已就绪. 在 Moonlight 中选择「{}」即可开始",
                    registration.name
                );
            }
        }
    }
    Ok(Status {
        enabled,
        supported,
        needs_restore: display::state_path().exists(),
        missing,
        message,
    })
}

/// 打开或关闭当前游戏的跨屏游玩.
pub fn configure(app: &ServiceHandle, enabled: bool) -> Result<Status> {
    if game_busy(app)? {
        bail!("请先结束游戏再更改跨屏游玩设置");
    }
    let settings = app
        .state
        .settings
        .lock()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .clone();
    let game = settings.active_game.as_str();
    let name = format!(
        "{APP_PREFIX}{}",
        crate::install::display_name(&app.state.packages, &settings.active_game)
    );
    let path = apps_path()?;
    let mut apps = read_apps(&path)?;
    if !enabled {
        if remove(&mut apps, &name) {
            write_apps(&path, &apps)?;
            reload_sunshine();
        }
        forget(game)?;
        return status(app);
    }
    if !supported() {
        bail!("当前仅支持 KDE Plasma Wayland");
    }
    let missing = missing_components();
    if !missing.is_empty() {
        bail!("请先安装串流组件");
    }
    // 只有 Sunshine 自己的 KWin 捕获受 KScreen 控制, 其它方式改不了显示模式.
    if config_value("capture")? != "kwin" {
        bail!("请把 Sunshine 的 capture 设置为 kwin");
    }
    display::verify(&config_value("output_name")?)?;
    crate::environment_commands::resolve(&app.state.packages, &settings)
        .map_err(anyhow::Error::msg)?;
    crate::install::launcher_root(&app.state.packages, &settings).context("请先安装游戏")?;
    let entry = cross_screen_entry()?;
    let command = format!("{entry} run --game {}", quote(game));
    if conflict(&apps, &name, &command) {
        bail!("Sunshine 中已有同名应用, 请先重命名该应用");
    }
    let entry = app_entry(&name, &entry, game);
    upsert(&mut apps, &name, entry);
    write_apps(&path, &apps)?;
    save(&Registration {
        game: game.into(),
        name,
    })?;
    reload_sunshine();
    status(app)
}

/// 串流入口: 已安装的应用走自身路径, 桌面目录直接调用同一分支.
fn cross_screen_entry() -> Result<String> {
    if let Some(image) = std::env::var_os("APPIMAGE") {
        return Ok(quote(image.to_string_lossy().as_ref()));
    }
    Ok(quote(
        crate::recovery::sibling("nte-cli")?
            .to_string_lossy()
            .as_ref(),
    ))
}

/// Sunshine 里的应用条目; 命令与 prep-cmd 都指向同一个入口.
pub(crate) fn app_entry(name: &str, entry: &str, game: &str) -> Value {
    json!({
        "name": name,
        "cmd": format!("{entry} run --game {}", quote(game)),
        "auto-detach": false,
        "wait-all": false,
        "exit-timeout": 5,
        "prep-cmd": [{
            "do": format!("{entry} prepare --game {}", quote(game)),
            "undo": format!("{entry} restore"),
        }],
        "detached": [],
        "image-path": ""
    })
}

/// 已有同名应用时拒绝覆盖: 用户可能自己建过同名条目.
///
/// 没有命令的条目可以直接接管.
pub(crate) fn conflict(apps: &Applications, name: &str, command: &str) -> bool {
    apps.apps.iter().any(|app| {
        app.get("name").and_then(Value::as_str) == Some(name)
            && app
                .get("cmd")
                .and_then(Value::as_str)
                .is_some_and(|existing| existing != command)
    })
}

/// 按发行版给出安装串流组件的命令; 只覆盖有软件包可用的发行版.
pub(crate) fn install_args(release: &str) -> Result<Vec<&'static str>> {
    let fields: Vec<&str> = release
        .lines()
        .filter(|line| line.starts_with("ID=") || line.starts_with("ID_LIKE="))
        .filter_map(|line| line.split_once('='))
        .flat_map(|(_, value)| value.trim_matches('"').split_whitespace())
        .collect();
    if fields.contains(&"arch") {
        Ok(vec![
            "/usr/bin/pacman",
            "-S",
            "--needed",
            "--noconfirm",
            "sunshine",
            "libkscreen",
            "gamescope",
        ])
    } else if fields.contains(&"fedora") {
        Ok(vec![
            "/usr/bin/dnf",
            "install",
            "-y",
            "sunshine",
            "libkscreen",
            "gamescope",
        ])
    } else {
        bail!("当前发行版暂不支持自动安装串流组件");
    }
}

pub fn install() -> Result<()> {
    let args = install_args(&std::fs::read_to_string("/etc/os-release")?)?;
    let output = Command::new("pkexec").args(args).output()?;
    if !output.status.success() {
        bail!(
            "组件安装未完成: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let started = Command::new("systemctl")
        .args(["--user", "start", "sunshine"])
        .status()?;
    if !started.success() {
        bail!("组件已安装, Sunshine 服务未能启动");
    }
    Ok(())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Client {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
    pub(crate) output_width: u32,
    pub(crate) output_height: u32,
}

impl Client {
    /// 从 Sunshine 注入的客户端环境读取尺寸; 缺失或非法时给出可执行的说明.
    pub(crate) fn from_env(variable: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let number = |key: &str| -> Result<u32> {
            variable(key)
                .with_context(|| format!("缺少 {key}, 请从 Moonlight 启动游戏"))?
                .parse()
                .with_context(|| format!("{key} 不是有效的数值"))
        };
        let client = Self {
            width: number("SUNSHINE_CLIENT_WIDTH")?,
            height: number("SUNSHINE_CLIENT_HEIGHT")?,
            fps: number("SUNSHINE_CLIENT_FPS")?,
            output_width: 1920,
            output_height: 1080,
        };
        client.validate()?;
        Ok(client)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !(320..=7680).contains(&self.width)
            || !(240..=4320).contains(&self.height)
            || !(1..=240).contains(&self.fps)
            || !(320..=16384).contains(&self.output_width)
            || !(240..=16384).contains(&self.output_height)
        {
            bail!("Moonlight 请求的分辨率或帧率超出支持范围");
        }
        Ok(())
    }

    pub(crate) fn apply(&self, settings: &mut crate::settings::Settings) {
        let mode = &mut settings.current_mut().game_mode;
        mode.inhibit = true;
        mode.gamescope.enabled = true;
        mode.gamescope.window_mode = crate::game_mode::gamescope::WindowMode::Fullscreen;
        mode.gamescope.game_width = self.width;
        mode.gamescope.game_height = self.height;
        mode.gamescope.output_width = self.output_width;
        mode.gamescope.output_height = self.output_height;
        mode.gamescope.fps = self.fps;
    }
}

pub(crate) fn connect(path: &Path) -> Result<(UnixStream, Client)> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut bytes = Vec::new();
    std::io::BufReader::new((&mut stream).take(4096)).read_until(b'\n', &mut bytes)?;
    let client: Client = serde_json::from_slice(&bytes)?;
    client.validate()?;
    Ok((stream, client))
}

pub(crate) fn watch(app: ServiceHandle, mut stream: UnixStream, id: u64) {
    std::thread::spawn(move || {
        let _ = stream.write_all(b"started\n");
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        loop {
            let active = app.state.game.lock().is_ok_and(|game| {
                game.session.as_ref().is_some_and(|session| {
                    session.id == id && crate::session::session_alive(session)
                }) || (game.starting && game.next_id == id)
            });
            if !active || app.stopping.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            let mut byte = [0];
            match stream.read(&mut byte) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue
                }
                _ => {
                    let _ = crate::service::block_on(crate::session::cancel_launch(
                        app.clone(),
                        &app.state,
                    ));
                    break;
                }
            }
        }
        let _ = stream.write_all(b"finished\n");
    });
}

/// 串流入口的子命令; restore 单独返回, 调用方可以直接恢复显示.
pub(crate) enum Entry<'a> {
    Restore,
    Stream { mode: &'a str, game: &'a str },
}

pub(crate) fn parse_entry(args: &[String]) -> Result<Entry<'_>> {
    if args == ["restore"] {
        return Ok(Entry::Restore);
    }
    if args.len() != 3 || !matches!(args[0].as_str(), "run" | "prepare") || args[1] != "--game" {
        bail!("用法: nte-cli cross-screen run --game <游戏 id> | restore");
    }
    if args[2].is_empty()
        || !args[2]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        bail!("游戏 id 无效");
    }
    Ok(Entry::Stream {
        mode: &args[0],
        game: &args[2],
    })
}

/// Sunshine 启动的前台进程, 生命周期与游戏会话绑定.
pub fn run_cli(args: &[String]) -> Result<()> {
    let (mode, game) = match parse_entry(args)? {
        Entry::Restore => return display::restore(),
        Entry::Stream { mode, game } => (mode, game),
    };
    let registration = load(game)?.context("请先在 Oddity 中开启跨屏游玩")?;
    let output_name = config_value("output_name")?;
    let mut client = Client::from_env(|key| std::env::var(key).ok())?;
    if mode == "prepare" {
        return display::prepare(&output_name, client.width, client.height, client.fps).map(|_| ());
    }
    let (output_width, output_height) = display::size(&output_name)?;
    client.output_width = output_width;
    client.output_height = output_height;
    let result = (|| -> Result<()> {
        let dir = tempfile::tempdir()?;
        let socket = dir.path().join("session.sock");
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let mut launcher = Command::new(launcher_executable()?)
            .args([
                "--launch",
                "--game",
                &registration.game,
                "--cross-screen-socket",
            ])
            .arg(&socket)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    bail!("等待 Oddity 启动超时")
                }
                Err(error) => return Err(error.into()),
            }
        };
        handshake(stream, &client)?;
        let _ = launcher.try_wait();
        Ok(())
    })();
    let restored = display::restore();
    result.and(restored)
}

/// 把本次会话的分辨率交给启动器, 并等它报告会话开始与结束.
pub(crate) fn handshake(mut stream: UnixStream, client: &Client) -> Result<()> {
    stream.write_all(&serde_json::to_vec(client)?)?;
    stream.write_all(b"\n")?;
    stream.set_read_timeout(Some(Duration::from_secs(180)))?;
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim() != "started" {
        bail!("Oddity 未能启动游戏, 请查看启动器中的错误信息");
    }
    reader.get_ref().set_read_timeout(None)?;
    line.clear();
    reader.read_line(&mut line)?;
    if line.trim() != "finished" {
        bail!("Oddity 会话连接已中断");
    }
    Ok(())
}

fn launcher_executable() -> Result<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| crate::recovery::sibling("oddity"))
}
