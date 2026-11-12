//! 系统时钟读取.
//!
//! 系统时钟早于 Unix 纪元时按 0 处理, 避免时间戳计算中断调用链.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix 纪元以来的秒数.
pub fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Unix 纪元以来的毫秒数.
pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 当前本地时间的分钟数 (0..1440), 用于时间窗口判断.
pub fn local_minute() -> u16 {
    let time = unix_seconds() as libc::time_t;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: localtime_r 只读取 time 并写入 local, 返回空指针时 local 不读取.
    let result = unsafe { libc::localtime_r(&time, local.as_mut_ptr()) };
    if result.is_null() {
        return 0;
    }
    // SAFETY: 上面的调用成功时已初始化整个 tm.
    let local = unsafe { local.assume_init() };
    (local.tm_hour as u16) * 60 + local.tm_min as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_system_clock_in_both_units() {
        let millis = unix_millis();
        let seconds = unix_seconds();
        assert!(millis > 1_600_000_000_000, "millis {millis}");
        assert!(seconds > 1_600_000_000, "seconds {seconds}");
        assert!(millis / 1000 >= seconds, "millis {millis} < {seconds}s");
        assert!(millis / 1000 - seconds <= 1, "两次读取相差过大");
    }

    #[test]
    fn local_minute_is_a_valid_time_of_day() {
        assert!(local_minute() < 24 * 60);
    }
}
