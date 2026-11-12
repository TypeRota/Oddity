//! 游戏资源自动下载策略与时间窗口.

use serde::{Deserialize, Serialize};

/// 自动下载相关设置.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    /// 发现更新后自动下载.
    pub auto_download: bool,
    /// 计费网络下的处理方式.
    pub metered: MeteredPolicy,
    /// 下载限速, 单位为字节每秒; 0 表示不限速.
    pub limit_bytes_per_second: u64,
    /// 允许自动下载的本地时间窗口.
    pub window: Option<Window>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auto_download: false,
            metered: MeteredPolicy::Ask,
            limit_bytes_per_second: 0,
            window: None,
        }
    }
}

impl Options {
    /// 限速上限, 同时作为配置校验边界.
    pub const MAX_LIMIT: u64 = 1024 * 1024 * 1024;

    pub fn validate(&self) -> Result<(), String> {
        if self.limit_bytes_per_second > Self::MAX_LIMIT {
            return Err("下载限速超出允许范围".into());
        }
        if let Some(window) = &self.window {
            window.validate()?;
        }
        Ok(())
    }

    /// 修正来自旧配置或手工编辑的越界值, 避免任务因配置错误无法启动.
    pub fn normalize(&mut self) {
        self.limit_bytes_per_second = self.limit_bytes_per_second.min(Self::MAX_LIMIT);
        if self
            .window
            .as_ref()
            .is_some_and(|window| window.minutes().is_none())
        {
            self.window = None;
        }
    }
}

/// 计费网络下的自动下载策略.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeteredPolicy {
    /// 等待用户确认.
    #[default]
    Ask,
    /// 暂停, 等切换到非计费网络后再继续.
    Pause,
    /// 照常下载.
    Allow,
}

/// 自动下载的本地时间窗口, 格式为 HH:MM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub start: String,
    pub end: String,
}

impl Window {
    pub fn validate(&self) -> Result<(), String> {
        if self.minutes().is_none() {
            return Err("自动下载时段无效, 请使用 HH:MM 且起止时间不同".into());
        }
        Ok(())
    }

    /// 起止分钟数; 起止相同视为非法.
    pub fn minutes(&self) -> Option<(u16, u16)> {
        let start = parse_minutes(&self.start)?;
        let end = parse_minutes(&self.end)?;
        (start != end).then_some((start, end))
    }

    /// 判断本地时间是否落在窗口内, 支持跨零点.
    ///
    /// 窗口无效时返回 true, 避免错误配置阻止自动下载.
    pub fn contains(&self, minute: u16) -> bool {
        let Some((start, end)) = self.minutes() else {
            return true;
        };
        if start < end {
            minute >= start && minute < end
        } else {
            minute >= start || minute < end
        }
    }
}

fn parse_minutes(value: &str) -> Option<u16> {
    let value = value.trim();
    let (hour, minute) = value.split_once(':')?;
    if hour.is_empty()
        || hour.len() > 2
        || minute.len() != 2
        || !hour.bytes().all(|byte| byte.is_ascii_digit())
        || !minute.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let hour: u16 = hour.parse().ok()?;
    let minute: u16 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_supports_same_day_and_overnight_ranges() {
        let day = Window {
            start: "02:00".into(),
            end: "07:30".into(),
        };
        assert!(day.contains(120));
        assert!(day.contains(449));
        assert!(!day.contains(119));
        assert!(!day.contains(450));

        let night = Window {
            start: "23:00".into(),
            end: "06:00".into(),
        };
        assert!(night.contains(23 * 60));
        assert!(night.contains(0));
        assert!(night.contains(359));
        assert!(!night.contains(360));
        assert!(!night.contains(22 * 60 + 59));
    }

    #[test]
    fn invalid_windows_are_reported_and_do_not_block_downloads() {
        for (start, end) in [
            ("", "07:00"),
            ("24:00", "07:00"),
            ("02:60", "07:00"),
            ("2:0", "07:00"),
            ("+2:00", "07:00"),
            ("02:00", "02:00"),
        ] {
            let window = Window {
                start: start.into(),
                end: end.into(),
            };
            assert!(window.validate().is_err(), "{start}-{end}");
            assert!(window.contains(0), "{start}-{end}");
        }
    }

    #[test]
    fn options_serialize_with_camel_case_and_defaults() {
        let options: Options = serde_json::from_str("{}").unwrap();
        assert_eq!(options, Options::default());
        let value = serde_json::to_value(Options {
            auto_download: true,
            metered: MeteredPolicy::Pause,
            limit_bytes_per_second: 1024,
            window: Some(Window {
                start: "02:00".into(),
                end: "07:00".into(),
            }),
        })
        .unwrap();
        assert_eq!(value["autoDownload"], true);
        assert_eq!(value["metered"], "pause");
        assert_eq!(value["limitBytesPerSecond"], 1024);
        assert_eq!(value["window"]["start"], "02:00");
    }

    #[test]
    fn validation_rejects_oversized_limits_and_normalization_drops_bad_windows() {
        let base = Options {
            limit_bytes_per_second: Options::MAX_LIMIT + 1,
            window: Some(Window {
                start: "02:00".into(),
                end: "02:00".into(),
            }),
            ..Options::default()
        };
        assert!(base.validate().is_err());
        let mut normalized = base.clone();
        normalized.normalize();
        assert_eq!(normalized.limit_bytes_per_second, Options::MAX_LIMIT);
        assert!(normalized.window.is_none());
        assert!(normalized.validate().is_ok());
    }
}
