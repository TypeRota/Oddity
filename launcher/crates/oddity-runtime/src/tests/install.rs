//! 安装域注册与查找测试.

use super::TEST_PACKAGE;
use crate::{
    ChannelDescriptor, GamePackage, InstallLocator, InstallQuery, LauncherInfo, PackageDescriptor,
    Registry,
};
use std::path::{Path, PathBuf};

/// 只返回固定路径的假安装域.
struct FakeInstall;

static FAKE_INSTALL: FakeInstall = FakeInstall;

impl InstallLocator for FakeInstall {
    fn launcher_root(&self, query: &InstallQuery) -> Option<PathBuf> {
        query
            .configured_install_root
            .clone()
            .or_else(|| Some(PathBuf::from(&query.distribution)))
    }

    fn launcher_dir(&self, query: &InstallQuery) -> Option<&'static str> {
        (query.distribution == "cn").then_some("LauncherCn")
    }

    fn selected_install_root(&self, query: &InstallQuery) -> PathBuf {
        query
            .configured_install_root
            .clone()
            .unwrap_or_else(|| PathBuf::from("/default"))
    }

    fn effective_install_root(
        &self,
        query: &InstallQuery,
        launcher_root: Option<&Path>,
    ) -> PathBuf {
        if query.configured_install_root.is_some() {
            return self.selected_install_root(query);
        }
        launcher_root
            .and_then(Path::parent)
            .map_or_else(|| PathBuf::from("/default"), Path::to_path_buf)
    }

    fn game_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        Some(launcher_root.join("Game.exe"))
    }

    fn game_version(&self, _launcher_root: &Path) -> String {
        "1.0".into()
    }

    fn launcher_info(&self, _launcher_root: &Path) -> Option<LauncherInfo> {
        Some(LauncherInfo {
            version: "2.0".into(),
            build: "20250101".into(),
        })
    }

    fn game_exe_hint(&self) -> String {
        "未找到 Game.exe".into()
    }
}

static FAKE_CHANNELS: &[ChannelDescriptor] = &[ChannelDescriptor {
    id: "cn",
    label: "国服",
    is_default: true,
    install_root_key: "cn",
}];

/// 声明了安装域的假游戏包.
struct FakePackage;

impl GamePackage for FakePackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "test",
            name: "测试包",
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
fn packages_without_an_install_domain_stay_unregistered() {
    let mut registry = Registry::new();
    registry.install(&TEST_PACKAGE);
    assert!(registry.install_locator("test").is_none());
    assert!(registry.install_locator("missing").is_none());
}

#[test]
fn install_domains_are_looked_up_by_game_id() {
    let registry = registry();
    let locator = registry
        .install_locator("test")
        .expect("测试包应提供安装域");
    let query = InstallQuery {
        distribution: "cn".into(),
        configured_install_root: Some(PathBuf::from("/games/nte")),
    };

    assert_eq!(locator.launcher_dir(&query), Some("LauncherCn"));
    assert_eq!(
        locator.launcher_root(&query),
        Some(PathBuf::from("/games/nte"))
    );
    assert_eq!(
        locator.selected_install_root(&query),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        locator.effective_install_root(&query, None),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        locator.game_exe(Path::new("/games/nte/launcher")),
        Some(PathBuf::from("/games/nte/launcher/Game.exe"))
    );
    assert_eq!(
        locator.game_version(Path::new("/games/nte/launcher")),
        "1.0"
    );
    assert_eq!(
        locator.launcher_info(Path::new("/games/nte/launcher")),
        Some(LauncherInfo {
            version: "2.0".into(),
            build: "20250101".into(),
        })
    );
    assert_eq!(locator.game_exe_hint(), "未找到 Game.exe");
}

#[test]
fn install_domains_follow_the_query_without_a_configured_root() {
    let registry = registry();
    let locator = registry.install_locator("test").expect("安装域");
    let query = InstallQuery {
        distribution: "global".into(),
        configured_install_root: None,
    };

    assert_eq!(locator.launcher_dir(&query), None);
    assert_eq!(locator.launcher_root(&query), Some(PathBuf::from("global")));
    assert_eq!(
        locator.selected_install_root(&query),
        PathBuf::from("/default")
    );
    assert_eq!(
        locator.effective_install_root(&query, Some(Path::new("/games/nte/launcher"))),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        locator.effective_install_root(&query, None),
        PathBuf::from("/default")
    );
}

#[test]
fn default_helpers_are_optional() {
    use std::path::Path;
    assert!(FAKE_INSTALL.sdk_dir(Path::new("/fake")).is_none());
    assert!(FAKE_INSTALL
        .launcher_config_path(Path::new("/fake"))
        .is_none());
    assert!(FAKE_INSTALL
        .official_client_exe(Path::new("/fake"))
        .is_none());
    assert!(FAKE_INSTALL.prefix_link_name().is_none());
}
