//! 游戏身份与内置 profile.
//!
//! 游戏定义随二进制一起评审和发布, 不引入远程清单. 每个游戏对应一个
//! [`GameProfile`], 地区相关的静态配置放在 [`RegionProfile`] 里, 与地区无关的
//! 安装布局和启动配方放在 [`GameLayout`] 与 [`LaunchRecipe`] 里.
//! 新增游戏只需增加一份 profile 并登记到 [`GameId`].
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::distribution::Distribution;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum GameId {
    #[default]
    Nte,
    Hsr,
}

impl GameId {
    pub const ALL: [Self; 2] = [Self::Nte, Self::Hsr];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nte => "nte",
            Self::Hsr => "hsr",
        }
    }

    pub const fn profile(self) -> &'static GameProfile {
        match self {
            Self::Nte => &NTE,
            Self::Hsr => &HSR,
        }
    }
}

impl FromStr for GameId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "nte" => Ok(Self::Nte),
            "hsr" => Ok(Self::Hsr),
            _ => Err(format!("无法识别游戏: {value}. 可选值: nte, hsr.")),
        }
    }
}

/// 编译进二进制的游戏定义.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameProfile {
    pub id: GameId,
    pub name: &'static str,
    pub regions: &'static [RegionProfile],
    /// 安装布局; 尚无实现的游戏为 `None`.
    pub layout: Option<&'static GameLayout>,
    /// 启动配方; 尚无实现的游戏为 `None`.
    pub launch: Option<&'static LaunchRecipe>,
}

impl GameProfile {
    pub fn region(&self, distribution: Distribution) -> Option<&'static RegionProfile> {
        self.regions
            .iter()
            .find(|region| region.distribution == distribution)
    }

    pub fn region_for_app_id(&self, app_id: i64) -> Option<&'static RegionProfile> {
        self.regions
            .iter()
            .find(|region| region.app_id == app_id && region.app_id != 0)
    }

    pub fn supports(&self, distribution: Distribution) -> bool {
        self.region(distribution).is_some()
    }

    /// 安装布局; 未实现的游戏返回 `None`.
    pub fn layout(&self) -> Option<&'static GameLayout> {
        self.layout
    }

    /// 启动配方; 未实现的游戏返回 `None`.
    pub fn launch(&self) -> Option<&'static LaunchRecipe> {
        self.launch
    }
}

/// 与地区无关的安装布局, 路径均相对安装根目录或启动器根目录.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameLayout {
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
    /// 背景素材目录名, 位于 `res_dir/<appId>/` 下.
    pub background_dir: &'static str,
}

impl GameLayout {
    pub fn content_path(&self, install_root: &Path) -> PathBuf {
        install_root.join(self.content_dir)
    }

    pub fn game_exe_path(&self, install_root: &Path) -> PathBuf {
        install_root.join(self.game_exe)
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

    pub fn background_path(&self, launcher_root: &Path, app_id: i64) -> PathBuf {
        self.res_path(launcher_root)
            .join(app_id.to_string())
            .join(self.background_dir)
    }
}

/// 与地区无关的启动配方.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchRecipe {
    /// 传给游戏可执行文件的参数.
    pub args: &'static [&'static str],
    /// 官方启动器写入 SDK 路径的配置文件名.
    pub launcher_config: &'static str,
    /// 官方启动器加载的 SDK DLL 文件名.
    pub sdk_dll: &'static str,
    /// 直接启动所需的 OneSDK DLL 文件名.
    pub one_sdk_dll: &'static str,
    /// 游戏目录下需要可写的服务器列表缓存名.
    pub serverlist: &'static str,
    /// Wine prefix 中指向安装目录的链接, 相对 `drive_c`.
    pub prefix_install_link: &'static str,
    /// `ArcGameSDKSharePath` 指向的目录名, 相对启动器根目录.
    pub sdk_share_dir: &'static str,
}

impl LaunchRecipe {
    /// 助手进程按单行环境变量接收参数.
    pub fn args_line(&self) -> String {
        self.args.join(" ")
    }
}

/// 一个游戏在某个地区下的安装与更新配置.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionProfile {
    pub distribution: Distribution,
    /// 官方 Config.ini 里的 GameID.
    pub app_id: i64,
    /// 错误提示里使用的游戏名.
    pub display_name: &'static str,
    pub default_install_name: &'static str,
    pub launcher_dir: &'static str,
    pub launcher_exe: &'static str,
    pub patcher_exe: &'static str,
    pub client_exe: &'static str,
    pub launcher_branch: &'static str,
    pub update_branch: &'static str,
    pub version_urls: [&'static str; 2],
    pub resource_cdns: [&'static str; 2],
    /// 主备 CDN 主机名; 清单拉取失败时用备用主机替换.
    pub cdn_hosts: [&'static str; 2],
}

const NTE_CN: RegionProfile = RegionProfile {
    distribution: Distribution::Cn,
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

const NTE_GLOBAL: RegionProfile = RegionProfile {
    distribution: Distribution::Global,
    app_id: 3000001,
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

static NTE_LAYOUT: GameLayout = GameLayout {
    content_dir: "Client/WindowsNoEditor",
    game_exe: "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
    res_dir: "ResFilesM",
    patcher_config_dir: "PatcherConfig",
    patcher_sdk_dir: "UserData/Patcher/PatcherSDK",
    background_dir: "bgimgs",
};

static NTE_LAUNCH: LaunchRecipe = LaunchRecipe {
    // UE map 与额外参数, 取自官方启动器拉起游戏时的 NTEGame.log.
    args: &[
        "/Game/LoginAndCreate/Map/Updater/Updater_P",
        "-SAVEWINPOS=1",
    ],
    // 官方启动器在拉起 HTGame.exe 前把 WPGameClientSDK_64.dll 的路径写进这个文件.
    launcher_config: "GameLauncher.config",
    sdk_dll: "WPGameClientSDK_64.dll",
    one_sdk_dll: "WPOneSDK_64.dll",
    serverlist: "patchersdk_serverlist",
    prefix_install_link: "Program Files (x86)/Neverness To Everness",
    sdk_share_dir: "WPGameClientSDK",
};

static NTE: GameProfile = GameProfile {
    id: GameId::Nte,
    name: "异环",
    regions: &[NTE_CN, NTE_GLOBAL],
    layout: Some(&NTE_LAYOUT),
    launch: Some(&NTE_LAUNCH),
};

/// 占位 profile: 安装与更新配方随《崩坏:星穹铁道》的实现补齐.
static HSR: GameProfile = GameProfile {
    id: GameId::Hsr,
    name: "崩坏:星穹铁道",
    regions: &[],
    layout: None,
    launch: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_names_case_insensitively() {
        assert_eq!("nte".parse::<GameId>().unwrap(), GameId::Nte);
        assert_eq!(" HSR ".parse::<GameId>().unwrap(), GameId::Hsr);
        assert_eq!(GameId::default(), GameId::Nte);
        let error = "genshin".parse::<GameId>().unwrap_err();
        assert!(error.contains("nte, hsr"));
    }

    #[test]
    fn every_game_id_has_a_matching_profile() {
        for id in GameId::ALL {
            let profile = id.profile();
            assert_eq!(profile.id, id);
            assert!(!profile.name.is_empty());
        }
    }

    #[test]
    fn nte_profile_covers_both_regions_and_app_ids() {
        let profile = GameId::Nte.profile();
        for distribution in [Distribution::Cn, Distribution::Global] {
            let region = profile.region(distribution).expect("NTE region");
            assert_eq!(region.distribution, distribution);
            assert!(profile.supports(distribution));
            assert_eq!(profile.region_for_app_id(region.app_id), Some(region));
            assert!(!region.default_install_name.is_empty());
            assert!(!region.launcher_dir.is_empty());
            assert!(!region.launcher_exe.is_empty());
            assert!(!region.patcher_exe.is_empty());
            assert!(!region.client_exe.is_empty());
            assert!(!region.launcher_branch.is_empty());
            assert!(!region.update_branch.is_empty());
            assert!(region.version_urls.iter().all(|url| !url.is_empty()));
            assert!(region.resource_cdns.iter().all(|url| !url.is_empty()));
            assert!(region.cdn_hosts.iter().all(|host| !host.is_empty()));
        }
        assert!(profile.region_for_app_id(0).is_none());
        assert!(profile.region_for_app_id(-1).is_none());
    }

    #[test]
    fn nte_profile_carries_the_install_layout_and_launch_recipe() {
        let profile = GameId::Nte.profile();
        let layout = profile.layout().expect("NTE layout");
        assert_eq!(layout.content_dir, "Client/WindowsNoEditor");
        assert_eq!(
            layout.game_exe,
            "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe"
        );
        assert_eq!(layout.res_dir, "ResFilesM");
        assert_eq!(layout.patcher_config_dir, "PatcherConfig");
        assert_eq!(layout.patcher_sdk_dir, "UserData/Patcher/PatcherSDK");
        assert_eq!(layout.background_dir, "bgimgs");

        let install = Path::new("/games/nte");
        let launcher = install.join("NTELauncher");
        assert_eq!(
            layout.content_path(install),
            install.join("Client/WindowsNoEditor")
        );
        assert_eq!(
            layout.game_exe_path(install),
            install.join("Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe")
        );
        assert_eq!(layout.res_path(&launcher), launcher.join("ResFilesM"));
        assert_eq!(
            layout.patcher_config_path(&launcher, 1289),
            launcher.join("ResFilesM/1289/PatcherConfig")
        );
        assert_eq!(
            layout.patcher_sdk_path(&launcher),
            launcher.join("UserData/Patcher/PatcherSDK")
        );
        assert_eq!(
            layout.background_path(&launcher, 1289),
            launcher.join("ResFilesM/1289/bgimgs")
        );

        let launch = profile.launch().expect("NTE launch");
        assert_eq!(
            launch.args,
            [
                "/Game/LoginAndCreate/Map/Updater/Updater_P",
                "-SAVEWINPOS=1"
            ]
        );
        assert_eq!(
            launch.args_line(),
            "/Game/LoginAndCreate/Map/Updater/Updater_P -SAVEWINPOS=1"
        );
        assert_eq!(launch.launcher_config, "GameLauncher.config");
        assert_eq!(launch.sdk_dll, "WPGameClientSDK_64.dll");
        assert_eq!(launch.one_sdk_dll, "WPOneSDK_64.dll");
        assert_eq!(launch.serverlist, "patchersdk_serverlist");
        assert_eq!(
            launch.prefix_install_link,
            "Program Files (x86)/Neverness To Everness"
        );
        assert_eq!(launch.sdk_share_dir, "WPGameClientSDK");
    }

    #[test]
    fn hsr_profile_is_a_placeholder_until_its_install_layout_lands() {
        let profile = GameId::Hsr.profile();
        assert_eq!(profile.name, "崩坏:星穹铁道");
        assert!(profile.regions.is_empty());
        assert!(!profile.supports(Distribution::Cn));
        assert!(profile.layout().is_none());
        assert!(profile.launch().is_none());
    }

    #[test]
    fn game_ids_serialize_as_lowercase_names() {
        assert_eq!(serde_json::to_string(&GameId::Nte).unwrap(), "\"nte\"");
        assert_eq!(serde_json::to_string(&GameId::Hsr).unwrap(), "\"hsr\"");
        assert_eq!(GameId::Nte.as_str(), "nte");
        assert_eq!(GameId::Hsr.as_str(), "hsr");
    }
}
