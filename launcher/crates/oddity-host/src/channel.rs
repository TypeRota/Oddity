//! 发行渠道标识与按包声明的解析.
//!
//! 渠道集合来自 GamePackage::channels(); 宿主只保存渠道 id, 不在类型里
//! 枚举具体渠道, 也不提供默认值.

use std::fmt;
use std::str::FromStr;

use oddity_runtime::{ChannelDescriptor, Registry};
use serde::{Deserialize, Deserializer, Serialize};

/// 发行渠道 id.
///
/// 构造时去空白并转小写, 不校验是否在包声明集合里; 显式输入由
/// [resolve] 校验, 已持久化的值由 [effective] 回落到默认渠道.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ChannelId(String);

impl ChannelId {
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(value.as_ref().trim().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ChannelId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let id = Self::new(value);
        if id.as_str().is_empty() {
            return Err("渠道 id 不能为空".into());
        }
        Ok(id)
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// 反序列化只做规范化; 未知或空值留给读路径回落, 避免历史文件整体载入失败.
impl<'de> Deserialize<'de> for ChannelId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// 显式输入的渠道不在声明集合里.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    /// 游戏没有声明任何渠道.
    None { game: String },
    /// 渠道 id 不在该游戏的声明集合里.
    Unknown {
        game: String,
        channel: String,
        allowed: Vec<&'static str>,
    },
}

impl fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None { game } => write!(formatter, "《{game}》没有声明渠道"),
            Self::Unknown {
                game,
                channel,
                allowed,
            } => write!(
                formatter,
                "《{game}》不支持渠道 {channel}. 可选值: {}.",
                allowed.join(", ")
            ),
        }
    }
}

/// 游戏包的展示名; 未装配时退回 id.
pub fn package_name<'a>(registry: &'a Registry, game: &'a str) -> &'a str {
    registry.package(game).map_or(game, |package| package.name)
}

/// 游戏声明的渠道.
pub fn channels(registry: &Registry, game: &str) -> &'static [ChannelDescriptor] {
    registry.channels(game).unwrap_or(&[])
}

/// 按 id 查找已声明的渠道.
pub fn find(
    registry: &Registry,
    game: &str,
    channel: &ChannelId,
) -> Option<&'static ChannelDescriptor> {
    channels(registry, game)
        .iter()
        .find(|descriptor| descriptor.id == channel.as_str())
}

/// 已验证的默认渠道; 没有声明或装配未校验时 None.
pub fn default(registry: &Registry, game: &str) -> Option<&'static ChannelDescriptor> {
    channels(registry, game)
        .iter()
        .find(|descriptor| descriptor.is_default)
}

/// 显式输入使用的严格解析: 未声明渠道报错并列出可选值.
pub fn resolve(
    registry: &Registry,
    game: &str,
    requested: Option<&ChannelId>,
) -> Result<&'static ChannelDescriptor, ChannelError> {
    let declared = channels(registry, game);
    if declared.is_empty() {
        return Err(ChannelError::None {
            game: package_name(registry, game).to_string(),
        });
    }
    match requested {
        Some(channel) => find(registry, game, channel).ok_or_else(|| ChannelError::Unknown {
            game: package_name(registry, game).to_string(),
            channel: channel.as_str().to_string(),
            allowed: declared.iter().map(|descriptor| descriptor.id).collect(),
        }),
        None => default(registry, game).ok_or_else(|| ChannelError::None {
            game: package_name(registry, game).to_string(),
        }),
    }
}

/// 已持久化的渠道使用宽松解析: 未知值回落到默认渠道.
pub fn effective(
    registry: &Registry,
    game: &str,
    stored: Option<&ChannelId>,
) -> Option<&'static ChannelDescriptor> {
    let declared = channels(registry, game);
    if declared.is_empty() {
        return None;
    }
    stored
        .and_then(|channel| find(registry, game, channel))
        .or_else(|| default(registry, game))
}

/// 当前有效渠道 id; 游戏没有声明渠道时为空字符串.
pub fn effective_id(registry: &Registry, game: &str, stored: Option<&ChannelId>) -> String {
    effective(registry, game, stored)
        .map(|channel| channel.id.to_string())
        .unwrap_or_default()
}

/// 渠道展示名; 已不存在的渠道回落到原始 id.
pub fn label(registry: &Registry, game: &str, channel: &ChannelId) -> String {
    find(registry, game, channel)
        .map(|descriptor| descriptor.label.to_string())
        .unwrap_or_else(|| channel.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oddity_runtime::{GamePackage, PackageDescriptor};

    struct Package {
        channels: &'static [ChannelDescriptor],
    }

    impl GamePackage for Package {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "test",
                name: "测试游戏",
            }
        }

        fn register(&self, _registry: &mut Registry) {}

        fn channels(&self) -> &'static [ChannelDescriptor] {
            self.channels
        }
    }

    static CHANNELS: &[ChannelDescriptor] = &[
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

    fn registry(channels: &'static [ChannelDescriptor]) -> Registry {
        let mut registry = Registry::new();
        registry.install(&Package { channels });
        registry
    }

    #[test]
    fn ids_are_trimmed_and_lowercased() {
        assert_eq!(ChannelId::new(" Global ").as_str(), "global");
        assert_eq!(" CN ".parse::<ChannelId>().unwrap().as_str(), "cn");
        assert!("  ".parse::<ChannelId>().is_err());
        assert_eq!(
            serde_json::to_string(&ChannelId::new("CN")).unwrap(),
            r#""cn""#
        );
        assert_eq!(
            serde_json::from_str::<ChannelId>(r#"" Global ""#)
                .unwrap()
                .as_str(),
            "global"
        );
    }

    #[test]
    fn strict_resolution_lists_declared_channels() {
        let registry = registry(CHANNELS);
        let default = resolve(&registry, "test", None).unwrap();
        assert_eq!(default.id, "cn");
        let global = resolve(&registry, "test", Some(&ChannelId::new("global"))).unwrap();
        assert_eq!(global.label, "国际服");
        let error = resolve(&registry, "test", Some(&ChannelId::new("eu"))).unwrap_err();
        assert_eq!(
            error.to_string(),
            "《测试游戏》不支持渠道 eu. 可选值: cn, global."
        );
    }

    #[test]
    fn packages_without_channels_report_a_stable_error() {
        let registry = registry(&[]);
        let error = resolve(&registry, "test", Some(&ChannelId::new("cn"))).unwrap_err();
        assert_eq!(error.to_string(), "《测试游戏》没有声明渠道");
        assert!(effective(&registry, "test", None).is_none());
        assert!(effective_id(&registry, "test", None).is_empty());
    }

    #[test]
    fn persisted_unknown_channels_fall_back_to_the_default() {
        let registry = registry(CHANNELS);
        let channel = effective(&registry, "test", Some(&ChannelId::new("eu"))).unwrap();
        assert_eq!(channel.id, "cn");
        assert_eq!(
            effective_id(&registry, "test", Some(&ChannelId::new("global"))),
            "global"
        );
        assert_eq!(label(&registry, "test", &ChannelId::new("eu")), "eu");
    }
}
