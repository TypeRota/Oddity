//! 宿主侧的安装位置适配.
//!
//! 宿主只保存只读设置; 目录探测、默认位置与游戏文件定位都由当前游戏的
//! 安装域负责. 游戏包没有安装域时返回空位置, 宿主不自行猜测.

use std::path::{Path, PathBuf};

use crate::channel::{self, ChannelId};
use crate::identity::GameId;
use crate::settings::Settings;
use oddity_runtime::{InstallLocator, InstallQuery, LauncherInfo, Registry};

/// 当前渠道设置的安装根目录; 未配置时 None.
pub(crate) fn configured_install_root<'a>(
    registry: &Registry,
    settings: &'a Settings,
) -> Option<&'a str> {
    let game = settings.current();
    let channel = channel::effective(
        registry,
        settings.active_game.as_str(),
        game.distribution.as_ref(),
    )?;
    game.install_root_for_key(channel.install_root_key)
}

/// 安装探测的只读输入; 配置里的 `~` 在这里展开.
pub(crate) fn query(registry: &Registry, settings: &Settings) -> InstallQuery {
    InstallQuery {
        distribution: channel::effective_id(
            registry,
            settings.active_game.as_str(),
            settings.current().distribution.as_ref(),
        ),
        configured_install_root: configured_install_root(registry, settings)
            .map(crate::paths::expand_path),
    }
}

/// 当前游戏的安装域.
pub(crate) fn locator<'a>(
    registry: &'a Registry,
    settings: &Settings,
) -> Option<&'a dyn InstallLocator> {
    locator_for(registry, &settings.active_game)
}

/// 指定游戏的安装域.
pub(crate) fn locator_for<'a>(
    registry: &'a Registry,
    game: &GameId,
) -> Option<&'a dyn InstallLocator> {
    registry.install_locator(game.as_str())
}

/// 包的展示名; 未装配时退回游戏 id.
pub(crate) fn display_name<'a>(registry: &'a Registry, game: &'a GameId) -> &'a str {
    registry
        .packages()
        .iter()
        .find(|package| package.id == game.as_str())
        .map_or(game.as_str(), |package| package.name)
}

/// 该游戏支持的发行渠道; 未装配或未声明时为空.
pub(crate) fn regions(registry: &Registry, game: &GameId) -> Vec<&'static str> {
    registry
        .channels(game.as_str())
        .map_or_else(Vec::new, |channels| {
            channels.iter().map(|channel| channel.id).collect()
        })
}

/// 该游戏在某渠道下的官方启动器目录名.
pub(crate) fn launcher_dir_for(
    registry: &Registry,
    game: &GameId,
    distribution: &str,
) -> Option<&'static str> {
    locator_for(registry, game)?.launcher_dir(&InstallQuery {
        distribution: distribution.to_string(),
        configured_install_root: None,
    })
}

/// 该游戏在某渠道下未安装时的默认安装目录名.
pub(crate) fn default_install_name(
    registry: &Registry,
    game: &GameId,
    distribution: &str,
) -> Option<String> {
    let root = locator_for(registry, game)?.selected_install_root(&InstallQuery {
        distribution: distribution.to_string(),
        configured_install_root: None,
    });
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

pub(crate) fn launcher_root(registry: &Registry, settings: &Settings) -> Option<PathBuf> {
    locator(registry, settings)?.launcher_root(&query(registry, settings))
}

#[cfg(test)]
pub(crate) fn launcher_dir(registry: &Registry, settings: &Settings) -> Option<&'static str> {
    locator(registry, settings)?.launcher_dir(&query(registry, settings))
}

pub(crate) fn selected_install_root(registry: &Registry, settings: &Settings) -> PathBuf {
    locator(registry, settings)
        .map(|locator| locator.selected_install_root(&query(registry, settings)))
        .unwrap_or_default()
}

pub(crate) fn effective_install_root(
    registry: &Registry,
    settings: &Settings,
    launcher_root: Option<&Path>,
) -> PathBuf {
    locator(registry, settings)
        .map(|locator| locator.effective_install_root(&query(registry, settings), launcher_root))
        .unwrap_or_default()
}

/// 把一个候选目录规范成该游戏的官方启动器根目录.
pub(crate) fn normalize_launcher_root(
    registry: &Registry,
    game: &GameId,
    path: &Path,
    distribution: &ChannelId,
) -> Option<PathBuf> {
    locator_for(registry, game)?.launcher_root(&InstallQuery {
        distribution: distribution.as_str().to_string(),
        configured_install_root: Some(path.to_path_buf()),
    })
}

pub(crate) fn game_exe(
    registry: &Registry,
    game: &GameId,
    launcher_root: &Path,
) -> Option<PathBuf> {
    locator_for(registry, game)?.game_exe(launcher_root)
}

pub(crate) fn game_version(registry: &Registry, game: &GameId, launcher_root: &Path) -> String {
    locator_for(registry, game)
        .map(|locator| locator.game_version(launcher_root))
        .unwrap_or_default()
}

/// 官方启动器记录的版本信息; 游戏包读不到时返回 None.
pub(crate) fn launcher_info(
    registry: &Registry,
    game: &GameId,
    launcher_root: &Path,
) -> Option<LauncherInfo> {
    locator_for(registry, game)?.launcher_info(launcher_root)
}

pub(crate) fn game_exe_hint(registry: &Registry, game: &GameId) -> String {
    locator_for(registry, game)
        .map(|locator| locator.game_exe_hint())
        .unwrap_or_else(|| "未找到游戏可执行文件".to_string())
}

/// SDK 组件目录; 包未提供或未安装时为空.
pub(crate) fn sdk_dir(registry: &Registry, game: &GameId, launcher_root: &Path) -> Option<PathBuf> {
    locator_for(registry, game)?.sdk_dir(launcher_root)
}

/// 官方启动器写入的启动器配置文件.
pub(crate) fn launcher_config_path(
    registry: &Registry,
    game: &GameId,
    install_root: &Path,
) -> Option<PathBuf> {
    locator_for(registry, game)?.launcher_config_path(install_root)
}

/// 官方客户端可执行文件; bootstrap 的官方启动器检查使用.
pub(crate) fn official_client_exe(
    registry: &Registry,
    game: &GameId,
    launcher_root: &Path,
) -> Option<PathBuf> {
    locator_for(registry, game)?.official_client_exe(launcher_root)
}

/// Wine prefix 内的游戏安装链接名; 游戏不迁移前缀时 None.
pub(crate) fn prefix_link_name(registry: &Registry, game: &GameId) -> Option<&'static str> {
    locator_for(registry, game)?.prefix_link_name()
}

#[cfg(test)]
mod sdk_tests {
    use super::*;
    use oddity_runtime::{GamePackage, PackageDescriptor};

    /// 只提供安装域、所有可选查询都返回 None 的测试包.
    struct OptionalLocator;

    impl InstallLocator for OptionalLocator {
        fn launcher_root(&self, _query: &InstallQuery) -> Option<PathBuf> {
            None
        }

        fn launcher_dir(&self, _query: &InstallQuery) -> Option<&'static str> {
            None
        }

        fn selected_install_root(&self, _query: &InstallQuery) -> PathBuf {
            PathBuf::new()
        }

        fn effective_install_root(
            &self,
            _query: &InstallQuery,
            _launcher_root: Option<&Path>,
        ) -> PathBuf {
            PathBuf::new()
        }

        fn game_exe(&self, _launcher_root: &Path) -> Option<PathBuf> {
            None
        }

        fn game_version(&self, _launcher_root: &Path) -> String {
            String::new()
        }

        fn launcher_info(&self, _launcher_root: &Path) -> Option<LauncherInfo> {
            None
        }

        fn game_exe_hint(&self) -> String {
            String::new()
        }

        fn official_client_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
            Some(launcher_root.join("Client.exe"))
        }

        fn prefix_link_name(&self) -> Option<&'static str> {
            Some("link")
        }
    }

    static OPTIONAL_LOCATOR: OptionalLocator = OptionalLocator;

    static OPTIONAL_CHANNELS: &[oddity_runtime::ChannelDescriptor] =
        &[oddity_runtime::ChannelDescriptor {
            id: "cn",
            label: "测试服",
            is_default: true,
            install_root_key: "cn",
        }];

    struct OptionalPackage;

    impl GamePackage for OptionalPackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "nte",
                name: "可选安装域",
            }
        }

        fn register(&self, _registry: &mut Registry) {}

        fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
            Some(&OPTIONAL_LOCATOR)
        }

        fn channels(&self) -> &'static [oddity_runtime::ChannelDescriptor] {
            OPTIONAL_CHANNELS
        }
    }

    struct BarePackage;

    impl GamePackage for BarePackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "fixture",
                name: "无安装域",
            }
        }

        fn register(&self, _registry: &mut Registry) {}
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.install(&OptionalPackage);
        registry.install(&BarePackage);
        registry
    }

    #[test]
    fn sdk_and_launcher_config_paths_come_from_the_locator() {
        let registry = registry();
        assert!(sdk_dir(&registry, &GameId::from("nte"), Path::new("/missing")).is_none());
        assert!(
            launcher_config_path(&registry, &GameId::from("nte"), Path::new("/missing")).is_none()
        );
    }

    #[test]
    fn only_packages_with_an_install_domain_are_selectable() {
        let registry = registry();
        assert_eq!(registry.selectable_games(), ["nte"]);
        assert_eq!(display_name(&registry, &GameId::from("nte")), "可选安装域");
        assert_eq!(display_name(&registry, &GameId::from("missing")), "missing");
    }

    #[test]
    fn installed_regions_and_display_names_come_from_the_locator() {
        let registry = registry();
        let game = GameId::from("nte");
        assert_eq!(regions(&registry, &game), ["cn"]);
        assert_eq!(
            official_client_exe(&registry, &game, Path::new("/install/FakeLauncher")),
            Some(PathBuf::from("/install/FakeLauncher/Client.exe"))
        );
        assert_eq!(prefix_link_name(&registry, &game), Some("link"));
        assert!(locator_for(&registry, &GameId::from("fixture")).is_none());
    }
}
