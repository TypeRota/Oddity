//! 单元测试共用的临时目录与环境变量守卫.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 独占进程级环境操作, 离开作用域时恢复原值.
pub struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    pub fn new() -> Self {
        // 之前的测试 panic 过也不影响后续用例
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

    /// 同一个变量只记第一次的原始值, 之后覆盖都还原到它.
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

/// 启动隔离的会话总线, 把 DBUS_SESSION_BUS_ADDRESS 指向它, 析构时恢复.
pub struct TestBus {
    daemon: Child,
    _env: EnvGuard,
}

impl TestBus {
    pub fn start() -> Self {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("启动隔离测试总线");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().expect("读取测试总线地址"))
            .read_line(&mut address)
            .expect("读取测试总线地址");
        let address = address.trim().to_string();
        let mut env = EnvGuard::new();
        env.set("DBUS_SESSION_BUS_ADDRESS", &address);
        Self { daemon, _env: env }
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
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
        let path =
            std::env::temp_dir().join(format!("oddity-test-{tag}-{}-{unique}", std::process::id()));
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

    pub fn read_text(&self, rel: impl AsRef<Path>) -> String {
        fs::read_to_string(self.path.join(rel)).expect("读取文本文件")
    }
}

/// 把可执行脚本写到临时目录, 用于外部命令查找测试.
pub fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    fs::write(&path, body).expect("写入脚本");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("设置脚本权限");
    path
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
