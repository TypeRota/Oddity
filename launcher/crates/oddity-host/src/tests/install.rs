//! 宿主安装适配: 用假游戏包验证调用方向.

use crate::channel::ChannelId;
use crate::identity::GameId;
use crate::install;
use crate::settings::Settings;
use crate::tests::settings_with_game;
use oddity_runtime::{
    ChannelDescriptor, GamePackage, InstallLocator, InstallQuery, LauncherInfo, PackageDescriptor,
    Registry,
};
use std::path::{Path, PathBuf};

/// 把宿主传入的输入直接映射成路径, 便于断言宿主没有丢掉字段.
struct FakeInstall;

static FAKE_INSTALL: FakeInstall = FakeInstall;

impl InstallLocator for FakeInstall {
    fn launcher_root(&self, query: &InstallQuery) -> Option<PathBuf> {
        let base = query
            .configured_install_root
            .clone()
            .unwrap_or_else(|| PathBuf::from("/fake"));
        Some(base.join(&query.distribution))
    }

    fn launcher_dir(&self, query: &InstallQuery) -> Option<&'static str> {
        match query.distribution.as_str() {
            "cn" => Some("FakeLauncher"),
            "global" => Some("FakeGlobal"),
            _ => None,
        }
    }

    fn selected_install_root(&self, query: &InstallQuery) -> PathBuf {
        query
            .configured_install_root
            .clone()
            .unwrap_or_else(|| PathBuf::from("/fake").join(&query.distribution))
    }

    fn effective_install_root(
        &self,
        query: &InstallQuery,
        launcher_root: Option<&Path>,
    ) -> PathBuf {
        if query.configured_install_root.is_some() {
            return self.selected_install_root(query);
        }
        launcher_root.and_then(Path::parent).map_or_else(
            || PathBuf::from("/fake").join(&query.distribution),
            Path::to_path_buf,
        )
    }

    fn game_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        Some(launcher_root.join("FakeGame.exe"))
    }

    fn game_version(&self, _launcher_root: &Path) -> String {
        "9.9.9".into()
    }

    fn launcher_info(&self, launcher_root: &Path) -> Option<LauncherInfo> {
        // 版本取目录名, 用来确认宿主把启动器根目录原样交给了安装域.
        Some(LauncherInfo {
            version: launcher_root.file_name()?.to_string_lossy().into_owned(),
            build: "20250101".into(),
        })
    }

    fn game_exe_hint(&self) -> String {
        "未找到 FakeGame.exe".into()
    }
}

static FAKE_CHANNELS: &[ChannelDescriptor] = &[
    ChannelDescriptor {
        id: "cn",
        label: "国服",
        is_default: true,
        install_root_key: "cn",
    },
    ChannelDescriptor {
        id: "global",
        label: "国际服",
        is_default: false,
        install_root_key: "global",
    },
];

/// 声明了安装域的假游戏包.
struct FakePackage;

impl GamePackage for FakePackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "nte",
            name: "假异环",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
        Some(&FAKE_INSTALL)
    }

    fn channels(&self) -> &'static [ChannelDescriptor] {
        FAKE_CHANNELS
    }
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.install(&FakePackage);
    registry
}

#[test]
fn host_passes_read_only_settings_to_the_game_install_domain() {
    let settings = settings_with_game(
        ChannelId::new("cn"),
        Some(PathBuf::from("/games/nte")),
        None,
    );
    let registry = registry();

    assert_eq!(
        install::launcher_dir(&registry, &settings),
        Some("FakeLauncher")
    );
    assert_eq!(
        install::launcher_root(&registry, &settings),
        Some(PathBuf::from("/games/nte/cn"))
    );
    assert_eq!(
        install::selected_install_root(&registry, &settings),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        install::effective_install_root(&registry, &settings, None),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        install::game_exe(
            &registry,
            &GameId::from("nte"),
            Path::new("/games/nte/FakeLauncher")
        ),
        Some(PathBuf::from("/games/nte/FakeLauncher/FakeGame.exe"))
    );
    assert_eq!(
        install::game_version(
            &registry,
            &GameId::from("nte"),
            Path::new("/games/nte/FakeLauncher")
        ),
        "9.9.9"
    );
    assert_eq!(
        install::launcher_info(
            &registry,
            &GameId::from("nte"),
            Path::new("/games/nte/FakeLauncher")
        ),
        Some(LauncherInfo {
            version: "FakeLauncher".into(),
            build: "20250101".into(),
        })
    );
    assert_eq!(
        install::game_exe_hint(&registry, &GameId::from("nte")),
        "未找到 FakeGame.exe"
    );
}

#[test]
fn host_expands_home_paths_and_respects_the_region_root() {
    let mut env = crate::tests::EnvGuard::new();
    let home = crate::tests::TempDir::new("install-expand-home");
    env.set("HOME", home.path());
    let mut settings = Settings::default();
    settings
        .current_mut()
        .install_roots
        .insert("cn".into(), "~/games/nte".into());

    assert_eq!(
        install::launcher_root(&registry(), &settings),
        Some(home.join("games/nte/cn"))
    );

    // 国际服读取 global 安装根键.
    let global = settings_with_game(
        ChannelId::new("global"),
        None,
        Some(PathBuf::from("/games/global")),
    );
    assert_eq!(
        install::launcher_dir(&registry(), &global),
        Some("FakeGlobal")
    );
    assert_eq!(
        install::launcher_root(&registry(), &global),
        Some(PathBuf::from("/games/global/global"))
    );
}

#[test]
fn host_derives_the_install_root_from_the_detected_launcher() {
    let settings = settings_with_game(ChannelId::new("cn"), None, None);
    let registry = registry();
    assert_eq!(
        install::effective_install_root(
            &registry,
            &settings,
            Some(Path::new("/games/nte/FakeLauncher"))
        ),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        install::effective_install_root(&registry, &settings, None),
        PathBuf::from("/fake/cn")
    );
    assert_eq!(
        install::selected_install_root(&registry, &settings),
        PathBuf::from("/fake/cn")
    );
}

#[test]
fn normalize_uses_the_requested_game_and_distribution() {
    let registry = registry();
    assert_eq!(
        install::normalize_launcher_root(
            &registry,
            &GameId::from("nte"),
            Path::new("/candidate"),
            &ChannelId::new("global")
        ),
        Some(PathBuf::from("/candidate/global"))
    );
    assert_eq!(
        install::normalize_launcher_root(
            &registry,
            &GameId::from("hsr"),
            Path::new("/candidate"),
            &ChannelId::new("cn")
        ),
        None
    );
}

#[test]
fn host_returns_empty_paths_when_no_package_owns_the_game() {
    let registry = Registry::new();
    let settings = Settings::default();
    assert_eq!(install::launcher_root(&registry, &settings), None);
    assert_eq!(install::launcher_dir(&registry, &settings), None);
    assert_eq!(
        install::selected_install_root(&registry, &settings),
        PathBuf::new()
    );
    assert_eq!(
        install::effective_install_root(
            &registry,
            &settings,
            Some(Path::new("/games/nte/NTELauncher"))
        ),
        PathBuf::new()
    );
    assert_eq!(
        install::game_exe(
            &registry,
            &GameId::from("nte"),
            Path::new("/games/nte/NTELauncher")
        ),
        None
    );
    assert_eq!(
        install::game_version(
            &registry,
            &GameId::from("nte"),
            Path::new("/games/nte/NTELauncher")
        ),
        ""
    );
    assert_eq!(
        install::launcher_info(
            &registry,
            &GameId::from("nte"),
            Path::new("/games/nte/NTELauncher")
        ),
        None
    );
    assert_eq!(
        install::game_exe_hint(&registry, &GameId::from("nte")),
        "未找到游戏可执行文件"
    );
}
