use crate::attendance::GameRecord;
use serde::{Deserialize, Serialize};

/// 服务器日期序号; 北京时间 05:00 换日, 因此偏移 3 小时后按自然日取整.
pub(super) fn activity_day(time: u64) -> u64 {
    time.saturating_add(10_800_000) / 86_400_000
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Recovery {
    pub stamina_at: Option<u64>,
    pub backup_at: Option<u64>,
    pub backup_progress_ms: Option<u64>,
    pub city_at: Option<u64>,
    pub weekly_at: Option<u64>,
    pub activity_at: Option<u64>,
    pub incentive_at: Option<u64>,
    pub incentive_remaining_ms: Option<u64>,
}

impl GameRecord {
    pub(super) fn fill_missing(&mut self, source: &Self) {
        self.stamina = self.stamina.or(source.stamina);
        self.stamina_max = self.stamina_max.or(source.stamina_max);
        self.stamina_backup = self.stamina_backup.or(source.stamina_backup);
        self.city_stamina = self.city_stamina.or(source.city_stamina);
        self.city_stamina_max = self.city_stamina_max.or(source.city_stamina_max);
        self.city_stamina_backup = self.city_stamina_backup.or(source.city_stamina_backup);
        self.activity = self.activity.or(source.activity);
        self.weekly_remaining = self.weekly_remaining.or(source.weekly_remaining);
        self.incentive_obtained = self.incentive_obtained.or(source.incentive_obtained);
        self.incentive_maximum = self.incentive_maximum.or(source.incentive_maximum);
        self.incentive_period_days = self.incentive_period_days.or(source.incentive_period_days);
        self.incentive_remaining = self.incentive_remaining.or(source.incentive_remaining);
        if self.role_name.is_empty() {
            self.role_name.clone_from(&source.role_name);
        }
        if let Some(source) = &source.recovery {
            let recovery = self.recovery.get_or_insert_default();
            recovery.stamina_at = recovery.stamina_at.or(source.stamina_at);
            recovery.backup_at = recovery.backup_at.or(source.backup_at);
            recovery.backup_progress_ms = recovery.backup_progress_ms.or(source.backup_progress_ms);
            recovery.city_at = recovery.city_at.or(source.city_at);
            recovery.weekly_at = recovery.weekly_at.or(source.weekly_at);
            recovery.activity_at = recovery.activity_at.or(source.activity_at);
            recovery.incentive_at = recovery.incentive_at.or(source.incentive_at);
            recovery.incentive_remaining_ms = recovery
                .incentive_remaining_ms
                .or(source.incentive_remaining_ms);
        }
    }

    pub(super) fn projected(&self, now: u64) -> Self {
        let mut record = self.clone();
        let Some(recovery) = &self.recovery else {
            return record;
        };
        if let (Some(current), Some(maximum), Some(anchor)) =
            (self.stamina, self.stamina_max, recovery.stamina_at)
        {
            let elapsed = now.saturating_sub(anchor);
            record.stamina = Some(if current >= maximum {
                current
            } else {
                current
                    .saturating_add((elapsed / 360_000).min(u64::from(u32::MAX)) as u32)
                    .min(maximum)
            });
            let full_at =
                anchor.saturating_add(u64::from(maximum.saturating_sub(current)) * 360_000);
            if let (Some(backup), Some(backup_at)) = (self.stamina_backup, recovery.backup_at) {
                let started = full_at.max(backup_at);
                let elapsed = now.saturating_sub(started);
                let progress = if now >= full_at {
                    recovery.backup_progress_ms.unwrap_or(0)
                } else {
                    0
                };
                record.stamina_backup = Some(
                    backup
                        .saturating_add(
                            ((elapsed + progress) / 1_080_000).min(u64::from(u32::MAX)) as u32
                        )
                        .min(2400),
                );
            }
        }
        // Unix 时间的第一个北京时间周一 05:00 是 1970-01-05.
        let week = |time: u64| time.saturating_sub(334_800_000) / 604_800_000;
        if let (Some(current), Some(maximum), Some(anchor)) =
            (self.city_stamina, self.city_stamina_max, recovery.city_at)
        {
            let weeks = week(now).saturating_sub(week(anchor));
            if weeks > 0 {
                record.city_stamina = Some(maximum);
                if let Some(backup) = self.city_stamina_backup {
                    let carried = u64::from(current / 3) + u64::from(maximum / 3) * (weeks - 1);
                    record.city_stamina_backup =
                        Some((u64::from(backup) + carried).min(1000) as u32);
                }
            }
        }
        if self.weekly_remaining.is_some()
            && recovery
                .weekly_at
                .is_some_and(|anchor| week(now) > week(anchor))
        {
            record.weekly_remaining = Some(3);
        }
        // 活跃度在服务器日期切换时清零, 国服对应北京时间每日 05:00.
        if self.activity.is_some()
            && recovery
                .activity_at
                .is_some_and(|anchor| activity_day(now) > activity_day(anchor))
        {
            record.activity = Some(0);
        }
        // 激励金按结算时刻归零, 之后按周期天数递推.
        if let (Some(anchor), Some(remaining), Some(period), Some(_)) = (
            recovery.incentive_at,
            recovery.incentive_remaining_ms,
            self.incentive_period_days,
            self.incentive_obtained,
        ) {
            let period_ms = u64::from(period).saturating_mul(86_400_000);
            if let Some(period_ms) = std::num::NonZeroU64::new(period_ms) {
                let period_ms = period_ms.get();
                let mut deadline = anchor.saturating_add(remaining);
                if now >= deadline {
                    record.incentive_obtained = Some(0);
                    deadline = deadline
                        .saturating_add((now - deadline) / period_ms * period_ms + period_ms);
                }
                record.incentive_remaining = Some(deadline.saturating_sub(now));
            }
        }
        record
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> GameRecord {
        serde_json::from_value(serde_json::json!({
            "roleId":"900000001", "roleName":"测试角色", "serverName":"国服",
            "stamina":358, "staminaMax":360, "staminaBackup":2398,
            "cityStamina":425, "cityStaminaMax":700, "cityStaminaBackup":900,
            "activity":null, "weeklyRemaining":0, "updatedAt":1,
            "recovery":{"staminaAt":1000000,"backupAt":1000000,"backupProgressMs":0,
            "cityAt":1000000,"weeklyAt":1000000}
        }))
        .unwrap()
    }
    #[test]
    fn stamina_recovers_on_the_anchor_and_backup_only_after_full() {
        let source = record();
        assert_eq!(source.projected(1_359_999).stamina, Some(358));
        assert_eq!(source.projected(1_360_000).stamina, Some(359));
        let full = source.projected(1_720_000);
        assert_eq!(full.stamina, Some(360));
        assert_eq!(full.stamina_backup, Some(2398));
        assert_eq!(source.projected(2_800_000).stamina_backup, Some(2399));
        assert_eq!(source.projected(9_000_000).stamina_backup, Some(2400));
        assert_eq!(source.stamina, Some(358));
    }
    #[test]
    fn monday_five_in_china_resets_current_values_and_transfers_city_backup() {
        let monday = 334_800_000 + 2900 * 604_800_000;
        let mut source = record();
        let recovery = source.recovery.as_mut().unwrap();
        recovery.city_at = Some(monday - 1);
        recovery.weekly_at = Some(monday - 1);
        assert_eq!(source.projected(monday - 1).city_stamina, Some(425));
        assert_eq!(source.projected(monday - 1).weekly_remaining, Some(0));
        let reset = source.projected(monday);
        assert_eq!(reset.city_stamina, Some(700));
        assert_eq!(reset.city_stamina_backup, Some(1000));
        assert_eq!(reset.weekly_remaining, Some(3));
        source.city_stamina_backup = Some(0);
        assert_eq!(source.projected(monday).city_stamina_backup, Some(141));
        assert_eq!(
            source.projected(monday + 604_800_000).city_stamina_backup,
            Some(374)
        );
    }

    #[test]
    fn activity_resets_daily_and_incentive_rolls_at_the_reported_deadline() {
        let mut source = record();
        source.activity = Some(40);
        source.recovery.as_mut().unwrap().activity_at = Some(334_800_000);
        assert_eq!(
            source.projected(334_800_000 + 86_400_000 - 1).activity,
            Some(40)
        );
        assert_eq!(source.projected(334_800_000 + 86_400_000).activity, Some(0));
        // 北京时间 05:00 是边界: 之前的读数保留, 之后的读数清零.
        let snapshot = 1_795_007_129_364; // 2026-11-18 21:05:29
        source.recovery.as_mut().unwrap().activity_at = Some(snapshot);
        assert_eq!(source.projected(1_795_035_599_000).activity, Some(40));
        assert_eq!(source.projected(1_795_035_601_000).activity, Some(0));
        assert_eq!(source.projected(1_795_118_401_000).activity, Some(0));

        source.incentive_obtained = Some(350_000);
        source.incentive_maximum = Some(1_150_000);
        source.incentive_period_days = Some(14);
        let recovery = source.recovery.as_mut().unwrap();
        recovery.incentive_at = Some(1_000_000);
        recovery.incentive_remaining_ms = Some(1_000_000);
        let open = source.projected(1_999_999);
        assert_eq!(open.incentive_obtained, Some(350_000));
        assert_eq!(open.incentive_maximum, Some(1_150_000));
        assert_eq!(open.incentive_remaining, Some(1));
        let settled = source.projected(2_000_000);
        assert_eq!(settled.incentive_obtained, Some(0));
        assert_eq!(settled.incentive_remaining, Some(14 * 86_400_000));
        assert_eq!(source.incentive_obtained, Some(350_000));
    }

    #[test]
    fn clock_reversal_missing_anchors_and_item_overflow_preserve_known_values() {
        let mut source = record();
        assert_eq!(source.projected(0).stamina, Some(358));
        source.stamina = Some(420);
        assert_eq!(source.projected(1_360_000).stamina, Some(420));
        source.recovery = None;
        assert_eq!(source.projected(u64::MAX).city_stamina, Some(425));
        assert_eq!(source.projected(u64::MAX).weekly_remaining, Some(0));
    }
}
