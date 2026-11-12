//! 游戏身份.
//!
//! 合法 id 来自装配表, 宿主不保留枚举; 字符串形式保证设置与协议载荷不变.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// 未指定游戏时使用的 id.
///
/// 线上设置里已有 `"activeGame": "nte"`; 这里保持该默认值, 是否装配由命令
/// 入口按注册表校验.
const DEFAULT_GAME: &str = "nte";

/// 游戏 id: 与协议 `game` 字段、设置里的 `activeGame` 和 `games` 键一致.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GameId(String);

impl GameId {
    /// 规范化一个 id: 去掉首尾空白并转小写.
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(value.as_ref().trim().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 按当前装配的合法 id 解析; 不在集合里时错误文案列出可选值.
    pub fn parse_known(value: &str, known: &[&str]) -> Result<Self, String> {
        let game = Self::from_str(value)?;
        if known.contains(&game.as_str()) {
            Ok(game)
        } else {
            Err(format!(
                "无法识别游戏: {value}. 可选值: {}.",
                known_list(known)
            ))
        }
    }
}

/// 合法 id 的错误文案用; 没有可选游戏时给出明确说明.
pub fn known_list(known: &[&str]) -> String {
    if known.is_empty() {
        "无".to_string()
    } else {
        known.join(", ")
    }
}

impl Default for GameId {
    fn default() -> Self {
        Self(DEFAULT_GAME.to_string())
    }
}

impl FromStr for GameId {
    type Err = String;

    /// 只校验格式; 是否属于当前装配由 [`GameId::parse_known`] 判断.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let game = Self::new(value);
        if game.as_str().is_empty() {
            return Err("游戏 id 不能为空".to_string());
        }
        Ok(game)
    }
}

impl fmt::Display for GameId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for GameId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for GameId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for GameId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_a_plain_string() {
        assert_eq!(
            serde_json::to_string(&GameId::default()).unwrap(),
            "\"nte\""
        );
        assert_eq!(
            serde_json::to_string(&GameId::from("hsr")).unwrap(),
            "\"hsr\""
        );
        assert_eq!(
            serde_json::from_str::<GameId>("\"nte\"").unwrap(),
            GameId::from("nte")
        );
    }

    #[test]
    fn normalizes_and_orders_ids_as_strings() {
        assert_eq!(GameId::from(" HSR "), GameId::from("hsr"));
        assert_eq!(GameId::from("hsr").to_string(), "hsr");
        assert!(GameId::from("hsr") < GameId::from("nte"));
        assert_eq!(" NTE ".parse::<GameId>().unwrap(), GameId::from("nte"));
        assert!("  ".parse::<GameId>().unwrap_err().contains("不能为空"));
    }

    #[test]
    fn parse_known_lists_the_assembled_ids() {
        assert_eq!(
            GameId::parse_known(" NTE ", &["nte", "hsr"]).unwrap(),
            GameId::from("nte")
        );
        let error = GameId::parse_known("genshin", &["nte"]).unwrap_err();
        assert_eq!(error, "无法识别游戏: genshin. 可选值: nte.");
        let error = GameId::parse_known("genshin", &[]).unwrap_err();
        assert!(error.ends_with("可选值: 无."), "{error}");
    }
}
