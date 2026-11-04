//! 游戏身份与内置 profile.
//!
//! 游戏定义随二进制一起评审和发布, 不引入远程清单. 每个游戏对应一个
//! [`GameProfile`], 地区相关的静态配置放在 [`RegionProfile`] 里.
//! 新增游戏只需增加一份 profile 并登记到 [`GameId`].
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
};

static NTE: GameProfile = GameProfile {
    id: GameId::Nte,
    name: "异环",
    regions: &[NTE_CN, NTE_GLOBAL],
};

/// 占位 profile: 安装与更新配方随《崩坏:星穹铁道》的实现补齐.
static HSR: GameProfile = GameProfile {
    id: GameId::Hsr,
    name: "崩坏:星穹铁道",
    regions: &[],
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
        }
        assert!(profile.region_for_app_id(0).is_none());
        assert!(profile.region_for_app_id(-1).is_none());
    }

    #[test]
    fn hsr_profile_is_a_placeholder_until_its_install_layout_lands() {
        let profile = GameId::Hsr.profile();
        assert_eq!(profile.name, "崩坏:星穹铁道");
        assert!(profile.regions.is_empty());
        assert!(!profile.supports(Distribution::Cn));
    }

    #[test]
    fn game_ids_serialize_as_lowercase_names() {
        assert_eq!(serde_json::to_string(&GameId::Nte).unwrap(), "\"nte\"");
        assert_eq!(serde_json::to_string(&GameId::Hsr).unwrap(), "\"hsr\"");
        assert_eq!(GameId::Nte.as_str(), "nte");
        assert_eq!(GameId::Hsr.as_str(), "hsr");
    }
}
