//! 区服与安装布局常量.
//!
//! 官方启动器自更新与后续的游戏资源更新共用同一套区服知识; 宿主经
//! [`oddity_runtime::InstallLocator`] 读取安装与区服信息, 不再保留副本.

use std::path::{Path, PathBuf};

/// 发行渠道; 取值与宿主设置里的 distribution 一致.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Region {
    #[default]
    Cn,
    Global,
}

impl Region {
    /// 解析设置或命令载荷里的区服; 未知取值返回 None.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cn" => Some(Self::Cn),
            "global" => Some(Self::Global),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cn => "cn",
            Self::Global => "global",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cn => "国服",
            Self::Global => "国际服",
        }
    }

    /// 同一个安装根下的另一地区.
    pub fn other(self) -> Self {
        match self {
            Self::Cn => Self::Global,
            Self::Global => Self::Cn,
        }
    }

    pub fn profile(self) -> &'static RegionProfile {
        profile(self)
    }
}

/// 一个地区下的官方安装与更新配置.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionProfile {
    pub region: Region,
    /// 官方 Config.ini 里的 GameID.
    pub app_id: i64,
    /// 错误提示与界面展示使用的游戏名.
    pub display_name: &'static str,
    pub default_install_name: &'static str,
    pub launcher_dir: &'static str,
    pub launcher_exe: &'static str,
    pub patcher_exe: &'static str,
    pub client_exe: &'static str,
    pub launcher_branch: &'static str,
    pub update_branch: &'static str,
    /// Version.ini 的主备地址.
    pub version_urls: [&'static str; 2],
    /// 游戏资源 CDN 的主备前缀.
    pub resource_cdns: [&'static str; 2],
    /// 主备 CDN 主机名; 清单拉取失败时用备用主机替换.
    pub cdn_hosts: [&'static str; 2],
}

pub const CN: RegionProfile = RegionProfile {
    region: Region::Cn,
    app_id: 1289,
    display_name: "异环",
    default_install_name: "Neverness To Everness",
    launcher_dir: "NTELauncher",
    launcher_exe: "NTELauncher.exe",
    patcher_exe: "NTEUpdate.exe",
    client_exe: "NTEGame.exe",
    launcher_branch: "publish_ob",
    update_branch: "publish_Updata",
    version_urls: [
        "https://yhcdn1.wmupd.com/hd/publish_ob/launcher/Version.ini",
        "https://yhcdn2.wmupd.com/hd/publish_ob/launcher/Version.ini",
    ],
    resource_cdns: [
        "https://yhcdn1.wmupd.com/clientRes",
        "https://yhcdn2.wmupd.com/clientRes",
    ],
    cdn_hosts: ["yhcdn1.wmupd.com", "yhcdn2.wmupd.com"],
};

pub const GLOBAL: RegionProfile = RegionProfile {
    region: Region::Global,
    app_id: 3_000_001,
    display_name: "NTE",
    default_install_name: "Neverness To Everness Global",
    launcher_dir: "NTEGlobal",
    launcher_exe: "NTEGlobalLauncher.exe",
    patcher_exe: "NTEGlobalUpdate.exe",
    client_exe: "NTEGlobalGame.exe",
    launcher_branch: "publish_PC",
    update_branch: "Publish_Update",
    version_urls: [
        "https://ntecdn1.perfectworld.com/hd/publish_PC/launcher/Version.ini",
        "https://ntecdn2.perfectworld.com/hd/publish_PC/launcher/Version.ini",
    ],
    resource_cdns: [
        "https://ntecdn1.perfectworld.com/clientRes",
        "https://ntecdn2.perfectworld.com/clientRes",
    ],
    cdn_hosts: ["ntecdn1.perfectworld.com", "ntecdn2.perfectworld.com"],
};

/// 某个地区的官方配置.
pub fn profile(region: Region) -> &'static RegionProfile {
    match region {
        Region::Cn => &CN,
        Region::Global => &GLOBAL,
    }
}

/// 按官方 Config.ini 的 GameID 定位地区; 0 与未知值不匹配.
pub fn region_for_app_id(app_id: i64) -> Option<Region> {
    [CN, GLOBAL]
        .iter()
        .find(|profile| profile.app_id != 0 && profile.app_id == app_id)
        .map(|profile| profile.region)
}

/// 按安装目录名定位地区.
pub fn region_for_dir_name(name: &str) -> Option<Region> {
    [CN, GLOBAL]
        .iter()
        .find(|profile| profile.launcher_dir == name)
        .map(|profile| profile.region)
}

/// 与地区无关的安装布局, 路径均相对安装根目录或启动器根目录.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// 游戏内容目录, 相对安装根目录.
    pub content_dir: &'static str,
    /// 游戏可执行文件, 相对安装根目录.
    pub game_exe: &'static str,
    /// 官方资源根目录, 相对启动器根目录.
    pub res_dir: &'static str,
    /// 单个地区的资源配置目录名, 位于 `res_dir/<appId>/` 下.
    pub patcher_config_dir: &'static str,
    /// PatcherSDK 工作目录, 相对启动器根目录.
    pub patcher_sdk_dir: &'static str,
    /// 官方启动器加载的 SDK DLL 文件名.
    pub sdk_dll: &'static str,
    /// 官方启动器写入 SDK 路径的配置文件名.
    pub launcher_config: &'static str,
}

pub const LAYOUT: Layout = Layout {
    content_dir: "Client/WindowsNoEditor",
    game_exe: "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
    res_dir: "ResFilesM",
    patcher_config_dir: "PatcherConfig",
    patcher_sdk_dir: "UserData/Patcher/PatcherSDK",
    sdk_dll: "WPGameClientSDK_64.dll",
    launcher_config: "GameLauncher.config",
};

impl Layout {
    pub fn content_path(&self, install_root: &Path) -> PathBuf {
        install_root.join(self.content_dir)
    }

    pub fn res_path(&self, launcher_root: &Path) -> PathBuf {
        launcher_root.join(self.res_dir)
    }

    pub fn patcher_config_path(&self, launcher_root: &Path, app_id: i64) -> PathBuf {
        self.res_path(launcher_root)
            .join(app_id.to_string())
            .join(self.patcher_config_dir)
    }

    pub fn patcher_sdk_path(&self, launcher_root: &Path) -> PathBuf {
        launcher_root.join(self.patcher_sdk_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_region_names_case_insensitively() {
        assert_eq!(Region::parse("cn"), Some(Region::Cn));
        assert_eq!(Region::parse(" Global "), Some(Region::Global));
        assert_eq!(Region::parse("eu"), None);
        assert_eq!(Region::default(), Region::Cn);
        assert_eq!(Region::Cn.other(), Region::Global);
        assert_eq!(Region::Global.other(), Region::Cn);
        assert_eq!(Region::Cn.as_str(), "cn");
        assert_eq!(Region::Global.label(), "国际服");
    }

    #[test]
    fn regions_are_looked_up_by_app_id_and_launcher_directory() {
        for region in [Region::Cn, Region::Global] {
            let profile = region.profile();
            assert_eq!(profile.region, region);
            assert_eq!(region_for_app_id(profile.app_id), Some(region));
            assert_eq!(region_for_dir_name(profile.launcher_dir), Some(region));
            assert!(!profile.display_name.is_empty());
            assert!(!profile.default_install_name.is_empty());
            assert!(!profile.launcher_exe.is_empty());
            assert!(!profile.patcher_exe.is_empty());
            assert!(!profile.client_exe.is_empty());
            assert!(!profile.launcher_branch.is_empty());
            assert!(!profile.update_branch.is_empty());
            assert!(profile.version_urls.iter().all(|url| !url.is_empty()));
            assert!(profile
                .resource_cdns
                .iter()
                .all(|url| url.starts_with("https://")));
            assert!(profile.cdn_hosts.iter().all(|host| !host.is_empty()));
        }
        assert_eq!(region_for_app_id(0), None);
        assert_eq!(region_for_app_id(-1), None);
        assert_eq!(region_for_dir_name("NTEOther"), None);
    }

    #[test]
    fn layout_paths_are_relative_to_the_install_and_launcher_roots() {
        let install = Path::new("/games/nte");
        let launcher = install.join("NTELauncher");
        assert_eq!(
            LAYOUT.content_path(install),
            install.join("Client/WindowsNoEditor")
        );
        assert_eq!(LAYOUT.res_path(&launcher), launcher.join("ResFilesM"));
        assert_eq!(
            LAYOUT.patcher_config_path(&launcher, 1289),
            launcher.join("ResFilesM/1289/PatcherConfig")
        );
        assert_eq!(
            LAYOUT.patcher_sdk_path(&launcher),
            launcher.join("UserData/Patcher/PatcherSDK")
        );
    }
}
