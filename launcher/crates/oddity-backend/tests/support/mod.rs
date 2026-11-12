//! 集成用例共用的测试夹具: 临时目录, 环境变量与可执行脚本.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use oddity_host::channel::ChannelId;
use oddity_host::environment::RuntimeEnvironment;
use oddity_host::settings::Settings;

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

pub fn managed_environment(
    proton: &Path,
    install: &Path,
    distribution: ChannelId,
) -> RuntimeEnvironment {
    oddity_host::environment::create(
        &oddity_host::paths::data_dir(),
        None,
        proton,
        install,
        oddity_host::environment::EnvironmentTarget {
            game: oddity_host::identity::GameId::from("nte"),
            distribution,
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
