//! 签到诊断日志: 每次 run 与会话刷新的结局写入宿主日志目录.
//!
//! 目录与宿主导出的 .log 收集一致, 只记录步骤分类与业务码, 凭据一律不落盘.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use oddity_platform::clock;

use super::{Failure, FailureKind, Report, Step};

/// 活动日志; 轮转文件沿用 .log 后缀, 保持被导出链路收集.
const ACTIVE: &str = "attendance.log";
/// 单个文件的字节上限.
const LIMIT: u64 = 1024 * 1024;
/// 保留的轮转文件数量.
const KEPT: usize = 3;
/// 单条消息在日志里的最大字符数.
const MESSAGE: usize = 160;

/// 用户数据目录下的日志目录, 与 oddity-host 的用户路径一致.
pub(crate) fn logs_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("oddity/logs")
}

/// 追加一行记录; 写入失败不影响签到结果.
pub(crate) fn write(dir: &Path, record: &str) {
    let line = format!("{} {}\n", stamp(), flat(record));
    let _ = append(dir, line.as_bytes());
}

pub(crate) fn run_line(
    trigger: &str,
    account: &str,
    report: &Report,
    attention: Option<&Failure>,
) -> String {
    let steps = report
        .steps
        .iter()
        .map(step_text)
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "run trigger={trigger} account={account} ok={} attention={} steps={steps}",
        report.ok,
        failure_text(attention)
    )
}

/// 会话三段回退的结局; 触发方式随每一步一起记录.
pub(crate) fn session_line(
    trigger: &str,
    account: &str,
    stages: &[(&'static str, Option<Failure>)],
) -> String {
    let stages = stages
        .iter()
        .map(|(name, failure)| match failure {
            None => format!("{name}=ok"),
            Some(failure) => format!("{name}=fail({})", failure_text(Some(failure))),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("session trigger={trigger} account={account} {stages}")
}

pub(crate) fn skip_line(trigger: &str, reason: &str) -> String {
    format!("run trigger={trigger} skipped reason={reason}")
}

pub(crate) fn summary_line(trigger: &str, accounts: usize, processed: u64) -> String {
    format!("run trigger={trigger} accounts={accounts} processed={processed}")
}

fn step_text(step: &Step) -> String {
    if step.ok {
        return format!("{}=ok", step.label);
    }
    let kind = step.kind.map_or("", kind_text);
    let mut detail = kind.to_string();
    if let Some(status) = step.status {
        detail.push_str(&format!(" http={status}"));
    }
    if let Some(code) = step.business {
        detail.push_str(&format!(" code={code}"));
    }
    format!("{}=fail({detail}, {})", step.label, truncate(&step.message))
}

/// 失败写成分类加 HTTP 状态或业务码, 便于直接判断是不是真的掉登录.
fn failure_text(failure: Option<&Failure>) -> String {
    let Some(failure) = failure else {
        return "-".into();
    };
    let mut text = kind_text(failure.kind).to_string();
    if let Some(status) = failure.status {
        text.push_str(&format!(" http={status}"));
    }
    if let Some(code) = failure.business {
        text.push_str(&format!(" code={code}"));
    }
    text.push(' ');
    text.push_str(&truncate(&failure.message));
    text
}

fn kind_text(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Network => "network",
        FailureKind::Auth => "auth",
        FailureKind::Verification => "verification",
        FailureKind::Keyring => "keyring",
        FailureKind::Storage => "storage",
        FailureKind::Busy => "busy",
        FailureKind::Protocol => "protocol",
        FailureKind::Service => "service",
    }
}

fn truncate(text: &str) -> String {
    let mut output: String = text.chars().take(MESSAGE).collect();
    if output.chars().count() < text.chars().count() {
        output.push('…');
    }
    output
}

/// 一条记录只占一行: 步骤文案里的换行等控制字符替换为空格.
fn flat(record: &str) -> String {
    record
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn append(dir: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    let path = dir.join(ACTIVE);
    rotate(dir, &path, bytes.len() as u64)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

/// 超出上限时把活动文件转成按时间戳命名的副本, 只保留最近若干份.
fn rotate(dir: &Path, path: &Path, incoming: u64) -> std::io::Result<()> {
    let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if size == 0 || size + incoming <= LIMIT {
        return Ok(());
    }
    let at = clock::unix_millis();
    let mut index = 0;
    let target = loop {
        let name = if index == 0 {
            format!("attendance-{at}.log")
        } else {
            format!("attendance-{at}-{index}.log")
        };
        let target = dir.join(name);
        if !target.exists() {
            break target;
        }
        index += 1;
    };
    fs::rename(path, target)?;
    prune(dir);
    Ok(())
}

fn prune(dir: &Path) {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| rotated_name(path))
            .collect(),
        Err(_) => return,
    };
    if files.len() <= KEPT {
        return;
    }
    files.sort();
    for path in &files[..files.len() - KEPT] {
        let _ = fs::remove_file(path);
    }
}

fn rotated_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(stem) = name
        .strip_prefix("attendance-")
        .and_then(|rest| rest.strip_suffix(".log"))
    else {
        return false;
    };
    stem.split('-')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// 记录时刻按签到域的日界线 (UTC+8) 写出, 与每日判断口径一致.
fn stamp() -> String {
    stamp_of(clock::unix_seconds())
}

fn stamp_of(at: u64) -> String {
    let local = at as i64 + 8 * 3600;
    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);
    let (year, month, day) = date_of(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+08:00",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// 把 Unix 纪元起的天数换算成公历年月日.
fn date_of(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn timestamps_follow_the_china_day_boundary() {
        assert_eq!(stamp_of(0), "1970-01-01T08:00:00+08:00");
        assert_eq!(stamp_of(1_700_000_000), "2023-11-15T06:13:20+08:00");
    }

    #[test]
    fn rotation_keeps_each_file_under_the_limit_and_the_count_bounded() {
        let dir = TempDir::new("attendance-log-rotation");
        let record = "x".repeat(2048);
        let writes = (LIMIT as usize * 6) / record.len();
        for _ in 0..writes {
            write(dir.path(), &record);
        }
        let files: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .collect();
        assert!(files.len() <= KEPT + 1, "{}", files.len());
        for file in &files {
            assert!(
                file.metadata().unwrap().len() <= LIMIT,
                "{}",
                file.path().display()
            );
        }
    }

    #[test]
    fn unrelated_files_are_never_rotated_or_removed() {
        let dir = TempDir::new("attendance-log-keep");
        dir.write("other.log", "keep");
        dir.write("attendance.log.bak", "keep");
        let record = "y".repeat(2048);
        for _ in 0..(LIMIT as usize * 12) / record.len() {
            write(dir.path(), &record);
        }
        assert!(dir.is_file("other.log"));
        assert!(dir.is_file("attendance.log.bak"));
        assert_eq!(dir.read_text("other.log"), "keep");
    }

    #[test]
    fn control_characters_never_split_a_record() {
        let dir = TempDir::new("attendance-log-flat");
        write(dir.path(), "run first line\nrun second line");
        let text = dir.read_text("attendance.log");
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("run first line run second line"));
    }
}
