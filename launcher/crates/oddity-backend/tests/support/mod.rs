//! 集成用例共用的测试夹具: 临时目录, 环境变量, 可执行脚本与服务客户端.
//!
//! 各用例按需使用其中一部分, 未用到的条目在单个用例里不算问题.

#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use oddity_host::channel::ChannelId;
use oddity_host::environment::RuntimeEnvironment;
use oddity_host::settings::Settings;

use serde_json::{json, Value};

/// 装配《异环》包的注册表; 只读接口需要与真实装配一致.
pub fn packages() -> std::sync::Arc<oddity_runtime::Registry> {
    let mut registry = oddity_runtime::Registry::new();
    registry.install(&oddity_game_nte::NTE_PACKAGE);
    std::sync::Arc::new(registry)
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 独占进程环境变量, 并在离开作用域时恢复原值.
pub struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    pub fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        Self {
            saved: Vec::new(),
            _lock: lock,
        }
    }

    pub fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.remember(key);
        std::env::set_var(key, value);
    }

    pub fn remove(&mut self, key: &'static str) {
        self.remember(key);
        std::env::remove_var(key);
    }

    fn remember(&mut self, key: &'static str) {
        if !self.saved.iter().any(|(saved, _)| *saved == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// 析构时自动删除的临时目录.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "oddity-launch-{tag}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("创建临时目录");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.path.join(rel)
    }

    pub fn write(&self, rel: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("创建父目录");
        }
        fs::write(&full, contents).expect("写入文件");
        full
    }

    pub fn write_text(&self, rel: impl AsRef<Path>, contents: &str) -> PathBuf {
        self.write(rel, contents.as_bytes())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// 写一个可执行脚本, 用来替代外部命令.
pub fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("创建脚本目录");
    let path = dir.join(name);
    fs::write(&path, body).expect("写入脚本");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("设置脚本权限");
    path
}

/// 构造一个可用的 Proton 目录 (包含 proton 与 toolmanifest.vdf).
pub fn fake_proton(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).expect("创建 Proton 目录");
    fs::write(dir.join("proton"), b"").expect("写入 proton");
    fs::write(dir.join("toolmanifest.vdf"), b"").expect("写入 toolmanifest");
    dir
}

pub fn managed_environment(proton: &Path, install: &Path) -> RuntimeEnvironment {
    oddity_host::environment::create(
        &oddity_host::paths::data_dir(),
        None,
        proton,
        install,
        oddity_host::environment::EnvironmentTarget {
            game: oddity_host::identity::GameId::from("nte"),
            link_name: None,
        },
        "测试环境",
    )
    .unwrap()
}

/// 构造当前游戏的设置, 避免每个用例重复写 games.nte.
pub fn settings_with_game(
    distribution: ChannelId,
    install_root: Option<PathBuf>,
    global_install_root: Option<PathBuf>,
) -> Settings {
    let mut settings = Settings::default();
    let game = settings.current_mut();
    game.distribution = Some(distribution);
    if let Some(path) = install_root {
        game.install_roots
            .insert("cn".into(), path.display().to_string());
    }
    if let Some(path) = global_install_root {
        game.install_roots
            .insert("global".into(), path.display().to_string());
    }
    settings
}
/// 读取入口文件里的真实套接字路径.
fn read_socket(entry: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(entry).ok()?;
    let path = PathBuf::from(text.trim());
    (!path.as_os_str().is_empty()).then_some(path)
}

/// 一条已连接的服务会话.
pub struct Session {
    child: Option<Child>,
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next: u64,
    directory: PathBuf,
}

impl Session {
    /// 在给定 HOME 下启动服务并连上套接字.
    pub fn start(directory: &Path) -> Self {
        let backend =
            std::env::var("CARGO_BIN_EXE_oddity-backend").expect("CARGO_BIN_EXE_oddity-backend");
        // 服务的标准错误落到文件: 启动失败时用它说明原因, 不污染测试输出.
        let error_log = directory.join("service-stderr.log");
        let stderr = std::fs::File::create(&error_log).expect("创建服务日志");
        let mut child = Command::new(backend)
            .arg("--ipc")
            .env("HOME", directory)
            .env("XDG_CONFIG_HOME", directory.join("config"))
            .env("XDG_DATA_HOME", directory.join("data"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/oddity-test-bus",
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("启动会话服务");
        let entry = directory.join("data/oddity/ipc-path");
        let deadline = Instant::now() + Duration::from_secs(30);
        let stream = loop {
            // 入口文件记录真实套接字路径; 数据目录再深也不影响连接.
            if let Some(socket) = read_socket(&entry) {
                if let Ok(stream) = UnixStream::connect(&socket) {
                    break stream;
                }
            }
            if let Some(status) = child.try_wait().expect("读取服务状态") {
                let log = std::fs::read_to_string(&error_log).unwrap_or_default();
                panic!("会话服务提前退出: {status:?}\n{log}");
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("会话服务未在超时前就绪");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("设置读取超时");
        let reader = BufReader::new(stream.try_clone().expect("复制连接"));
        Self {
            child: Some(child),
            stream,
            reader,
            next: 0,
            directory: directory.to_path_buf(),
        }
    }

    /// 发一条命令并等待回复, 途中出现的事件交给回调.
    pub fn call_with_events<E>(&mut self, command: &str, args: Value, mut event: E) -> Value
    where
        E: FnMut(Value),
    {
        self.next += 1;
        let id = self.next;
        let request = json!({"id": id, "command": command, "args": args});
        writeln!(self.stream, "{request}").expect("发送请求");
        self.stream.flush().expect("刷新请求");
        loop {
            let mut line = String::new();
            let count = self.reader.read_line(&mut line).expect("读取回复");
            assert!(count > 0, "会话服务关闭了连接");
            let value: Value = serde_json::from_str(&line).expect("回复是 JSON");
            if value.get("event").is_some() {
                event(value);
                continue;
            }
            assert_eq!(value.get("id"), Some(&json!(id)), "{value}");
            return value;
        }
    }

    /// 发一条命令并等待回复, 丢弃途中的事件.
    pub fn call(&mut self, command: &str, args: Value) -> Value {
        self.call_with_events(command, args, |_| {})
    }

    /// 发送已经构造好的请求, 用于覆盖 game 字段等完整形状.
    pub fn call_value(&mut self, request: Value) -> Value {
        let id = request["id"].as_u64().unwrap_or(0);
        writeln!(self.stream, "{request}").expect("发送请求");
        self.stream.flush().expect("刷新请求");
        loop {
            let mut line = String::new();
            let count = self.reader.read_line(&mut line).expect("读取回复");
            assert!(count > 0, "会话服务关闭了连接");
            let value: Value = serde_json::from_str(&line).expect("回复是 JSON");
            if value.get("event").is_some() {
                continue;
            }
            assert_eq!(value.get("id"), Some(&json!(id)), "{value}");
            return value;
        }
    }

    /// 服务进程 id; 由本客户端拉起的服务才有.
    pub fn pid(&self) -> u32 {
        self.child.as_ref().map(|child| child.id()).unwrap_or(0)
    }

    /// 只断开连接, 不等待服务退出; 用来验证服务是否独立存活.
    pub fn close_connection(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// 连到已经在运行的服务, 不拉起新进程.
    pub fn connect(directory: &Path) -> Self {
        let socket = read_socket(&directory.join("data/oddity/ipc-path")).expect("入口文件应存在");
        let stream = UnixStream::connect(&socket).expect("连接已运行的服务");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("设置读取超时");
        let reader = BufReader::new(stream.try_clone().expect("复制连接"));
        Self {
            child: None,
            stream,
            reader,
            next: 0,
            directory: directory.to_path_buf(),
        }
    }

    /// 关闭连接; 服务在空闲后自行退出.
    pub fn disconnect(mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        // 服务启动后有最短存活时间, 再加上空闲宽限才退出.
        self.await_exit(Duration::from_secs(30));
    }

    /// 等待服务进程退出并断言退出码.
    pub fn wait_for_exit(mut self, timeout: Duration) {
        self.await_exit(timeout);
    }

    fn await_exit(&mut self, timeout: Duration) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().expect("读取服务状态") {
                assert!(status.success(), "{status:?}");
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("会话服务在超时后仍未退出");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
