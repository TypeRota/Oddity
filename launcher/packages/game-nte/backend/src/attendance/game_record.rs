//! 塔吉多角色面板协议参考 NTEUID, 来源和许可见 THIRD_PARTY.md。
use super::{id, now, Failure, FailureKind, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub role_id: String,
    pub role_name: String,
    pub server_name: String,
    pub stamina: Option<u32>,
    pub stamina_max: Option<u32>,
    #[serde(default)]
    pub stamina_backup: Option<u32>,
    pub city_stamina: Option<u32>,
    pub city_stamina_max: Option<u32>,
    #[serde(default)]
    pub city_stamina_backup: Option<u32>,
    pub activity: Option<u32>,
    pub weekly_remaining: Option<u32>,
    /// 大亨计划本期已开采激励金.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incentive_obtained: Option<u64>,
    /// 当前角色生效的激励金上限.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incentive_maximum: Option<u64>,
    /// 结算周期天数.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incentive_period_days: Option<u32>,
    /// 距下次结算的毫秒数, 由读数与结算时间计算.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incentive_remaining: Option<u64>,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<crate::player_state::Recovery>,
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| s.chars().count() <= 128 && !s.chars().any(char::is_control))
        .unwrap_or_default()
        .to_string()
}
fn number(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|n| n.try_into().ok())
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}
impl GameRecord {
    pub(super) fn parse(data: &Value, expected_role: &str) -> Result<Self> {
        let role_id = id(&data["roleid"])
            .filter(|s| s == expected_role)
            .ok_or_else(|| {
                Failure::new(
                    FailureKind::Protocol,
                    "角色面板身份不匹配, 请在塔吉多检查绑定角色",
                )
            })?;
        Ok(Self {
            role_id,
            role_name: text(&data["rolename"]),
            server_name: text(&data["servername"]),
            stamina: number(&data["staminaValue"]),
            stamina_max: Some(match number(&data["staminaMaxValue"]) {
                // 基础值不包含家具加成, 最终上限缺失时采用 360.
                None | Some(0 | 240) => 360,
                Some(value) => value,
            }),
            stamina_backup: None,
            city_stamina_backup: None,
            city_stamina: number(&data["citystaminaValue"]),
            city_stamina_max: number(&data["citystaminaMaxValue"]),
            activity: number(&data["dayvalue"]),
            weekly_remaining: number(&data["weekcopiesremainCnt"]),
            incentive_obtained: None,
            incentive_maximum: None,
            incentive_period_days: None,
            incentive_remaining: None,
            updated_at: now(),
            recovery: None,
        })
    }
}
