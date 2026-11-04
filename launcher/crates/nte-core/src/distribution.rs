//! 游戏地区. 游戏身份与安装配置见 [`crate::game_profile`].
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distribution {
    #[default]
    Cn,
    Global,
}

impl FromStr for Distribution {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cn" => Ok(Self::Cn),
            "global" => Ok(Self::Global),
            _ => Err(format!("无法识别游戏地区: {value}. 可选值: cn, global.")),
        }
    }
}

impl Distribution {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_region_names_case_insensitively() {
        assert_eq!("cn".parse::<Distribution>().unwrap(), Distribution::Cn);
        assert_eq!(
            " Global ".parse::<Distribution>().unwrap(),
            Distribution::Global
        );
        let error = "eu".parse::<Distribution>().unwrap_err();
        assert!(error.contains("cn, global"));
    }

    #[test]
    fn labels_match_the_two_distributions() {
        assert_eq!(Distribution::Cn.label(), "国服");
        assert_eq!(Distribution::Global.label(), "国际服");
        assert_eq!(Distribution::Cn.as_str(), "cn");
        assert_eq!(Distribution::Global.as_str(), "global");
    }
}
