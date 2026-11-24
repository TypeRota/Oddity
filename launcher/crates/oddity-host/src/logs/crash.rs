//! 当前运行环境 prefix 内的 UE 崩溃现场.
//!
//! 崩溃上下文, 转储和游戏日志写在 Wine prefix 里, 不属于启动器日志目录.
//! 这里只负责发现和读出内容, 文本仍交给统一的日志脱敏通道.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use oddity_runtime::Registry;

use crate::settings::Settings;

/// 最多带走的崩溃目录数量.
const MAX_SITES: usize = 3;
/// 崩溃上下文等文本文件的上限.
pub(crate) const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// 游戏日志只保留尾部, 覆盖崩溃或卡死前后的上下文.
const MAX_LOG_TAIL: u64 = 256 * 1024;
/// 脱敏按行处理; 超长单行在收集阶段就跳过, 不拖垮其他内容.
const MAX_LINE: usize = 4 * 1024 * 1024;
/// 崩溃现场在导出包里的总上限.
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024;

/// 收集结果; 没有崩溃现场时 `files` 为空.
pub(crate) struct CrashExport {
    /// 找到的崩溃目录数量.
    pub sites: usize,
    pub files: Vec<CrashFile>,
    used: u64,
}

impl CrashExport {
    fn new() -> Self {
        Self {
            sites: 0,
            files: Vec::new(),
            used: 0,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

pub(crate) struct CrashFile {
    /// 压缩包内路径.
    pub name: String,
    pub data: CrashData,
}

pub(crate) enum CrashData {
    /// 文本内容, 交给日志脱敏通道.
    Text(Vec<u8>),
    /// 转储原样写入, 不按文本解码.
    Binary(Vec<u8>),
}

enum Added {
    Included { truncated: bool },
    Skipped(&'static str),
}

/// 收集当前环境的崩溃现场; 环境缺失或读取失败都只是少收内容, 不影响导出.
pub(crate) fn collect(
    registry: &Registry,
    settings: &Settings,
    include_dumps: bool,
) -> CrashExport {
    match crate::environment_commands::prefix(registry, settings) {
        Ok(prefix) => collect_prefix(&prefix, include_dumps),
        Err(_) => CrashExport::new(),
    }
}

/// prefix 内全部 UE 崩溃现场目录, 按路径排序.
pub(crate) fn crash_sites(prefix: &Path) -> Vec<PathBuf> {
    let mut sites = Vec::new();
    for user in user_dirs(&prefix.join("drive_c/users")) {
        sites.extend(crash_dirs(&user.join("AppData/Local/HT/Saved/Crashes")));
    }
    sites.sort();
    sites
}

/// 读取崩溃现场内的文本文件并规整成 UTF-8; 不是普通文件或超过上限时返回 None.
pub(crate) fn read_context_text(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_TEXT_BYTES {
        return None;
    }
    let bytes = read_capped(path, MAX_TEXT_BYTES, false).ok()?;
    Some(String::from_utf8_lossy(&as_text(&bytes)).into_owned())
}

pub(crate) fn collect_prefix(prefix: &Path, include_dumps: bool) -> CrashExport {
    let users = user_dirs(&prefix.join("drive_c/users"));
    let mut sites = Vec::new();
    for user in &users {
        for site in crash_dirs(&user.join("AppData/Local/HT/Saved/Crashes")) {
            sites.push((user.clone(), site));
        }
    }
    // 只带走最近几次现场, 避免历史崩溃把导出包拖大.
    sites.sort_by_key(|(_, site)| std::cmp::Reverse(modified(site)));
    sites.truncate(MAX_SITES);

    let mut export = CrashExport::new();
    let mut reports = Vec::new();
    for (user, site) in &sites {
        let (Some(user_name), Some(site_name)) = (file_name(user), file_name(site)) else {
            continue;
        };
        let mut report = SiteReport {
            title: format!("{user_name}/{site_name}"),
            files: Vec::new(),
        };
        for (name, path, size) in regular_files(site) {
            let archive_name = format!("crashes/{user_name}/Crashes/{site_name}/{name}");
            let added = if is_dump(&name) {
                if include_dumps {
                    export.add_dump(&path, archive_name, size)
                } else {
                    Added::Skipped("未包含 (二进制转储)")
                }
            } else if is_text(&name) {
                let (limit, tail) = if is_log(&name) {
                    (MAX_LOG_TAIL, true)
                } else {
                    (MAX_TEXT_BYTES, false)
                };
                export.add_text(&path, archive_name, size, limit, tail)
            } else {
                Added::Skipped("未包含 (未知格式)")
            };
            report.files.push(Listed::new(&name, size, added));
        }
        reports.push(report);
    }

    // 卡死等场景可能没有崩溃目录, 游戏日志尾部仍能说明现场.
    let mut logs = Vec::new();
    for user in &users {
        let Some(user_name) = file_name(user) else {
            continue;
        };
        let Some((path, size)) = newest_log(&user.join("AppData/Local/HT/Saved/Logs")) else {
            continue;
        };
        let Some(name) = file_name(&path) else {
            continue;
        };
        let added = export.add_text(
            &path,
            format!("crashes/{user_name}/Logs/{name}"),
            size,
            MAX_LOG_TAIL,
            true,
        );
        logs.push(Listed::new(&format!("{user_name}/{name}"), size, added));
    }

    export.sites = reports.len();
    // 只有被跳过的转储时也要留下清单, 让用户知道现场在哪里.
    let has_skipped = reports
        .iter()
        .any(|report| report.files.iter().any(|file| file.added.skipped()));
    if export.files.is_empty() && !has_skipped {
        return CrashExport::new();
    }
    let manifest = manifest(&reports, &logs);
    export.files.insert(
        0,
        CrashFile {
            name: "crashes/manifest.txt".into(),
            data: CrashData::Text(manifest.into_bytes()),
        },
    );
    export
}

impl CrashExport {
    fn add_text(&mut self, path: &Path, name: String, size: u64, limit: u64, tail: bool) -> Added {
        let Ok(bytes) = read_capped(path, limit, tail) else {
            return Added::Skipped("读取失败");
        };
        if self.used.saturating_add(bytes.len() as u64) > MAX_TOTAL_BYTES {
            return Added::Skipped("未包含 (超出体积上限)");
        }
        let text = as_text(&bytes);
        if text
            .split(|byte| *byte == b'\n')
            .any(|line| line.len() > MAX_LINE)
        {
            return Added::Skipped("未包含 (单行过长)");
        }
        self.used += text.len() as u64;
        self.files.push(CrashFile {
            name,
            data: CrashData::Text(text),
        });
        Added::Included {
            truncated: size > limit,
        }
    }

    fn add_dump(&mut self, path: &Path, name: String, size: u64) -> Added {
        if self.used.saturating_add(size) > MAX_TOTAL_BYTES {
            return Added::Skipped("未包含 (超出体积上限)");
        }
        match fs::read(path) {
            Ok(bytes) => {
                self.used += bytes.len() as u64;
                self.files.push(CrashFile {
                    name,
                    data: CrashData::Binary(bytes),
                });
                Added::Included { truncated: false }
            }
            Err(_) => Added::Skipped("读取失败"),
        }
    }
}

struct SiteReport {
    title: String,
    files: Vec<Listed>,
}

struct Listed {
    name: String,
    size: u64,
    added: Added,
}

impl Listed {
    fn new(name: &str, size: u64, added: Added) -> Self {
        Self {
            name: name.to_string(),
            size,
            added,
        }
    }
}

impl Added {
    fn skipped(&self) -> bool {
        matches!(self, Self::Skipped(_))
    }

    fn describe(&self) -> &'static str {
        match self {
            Self::Included { truncated: false } => "已包含",
            Self::Included { truncated: true } => "已包含 (截断)",
            Self::Skipped(reason) => reason,
        }
    }
}

fn manifest(reports: &[SiteReport], logs: &[Listed]) -> String {
    let mut text = String::from("UE 崩溃现场清单\n");
    text.push_str(&format!(
        "崩溃目录: {} 个, 按修改时间从新到旧排列.\n",
        reports.len()
    ));
    for report in reports {
        text.push_str(&format!("\n{}\n", report.title));
        for file in &report.files {
            push_listed(&mut text, file);
        }
    }
    if !logs.is_empty() {
        text.push_str("\n游戏日志:\n");
        for file in logs {
            push_listed(&mut text, file);
        }
    }
    if reports
        .iter()
        .flat_map(|report| &report.files)
        .any(|file| matches!(&file.added, Added::Skipped("未包含 (二进制转储)")))
    {
        text.push_str("\n转储文件未写入导出包, 如需完整转储请从上述目录单独提取.\n");
    }
    text
}

fn push_listed(text: &mut String, file: &Listed) {
    text.push_str(&format!(
        "  {}  {}  {}\n",
        file.name,
        size_text(file.size),
        file.added.describe()
    ));
}

fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// drive_c/users 下的实际目录, 不假设用户名.
fn user_dirs(users: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(entries) = fs::read_dir(users) else {
        return dirs;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    dirs
}

fn crash_dirs(crashes: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(entries) = fs::read_dir(crashes) else {
        return dirs;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("UECC-")
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    dirs
}

/// 目录内的普通文件; 符号链接不跟随.
fn regular_files(dir: &Path) -> Vec<(String, PathBuf, u64)> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let size = entry.metadata().map_or(0, |meta| meta.len());
        files.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
            size,
        ));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// 最近的游戏日志.
fn newest_log(logs: &Path) -> Option<(PathBuf, u64)> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(logs).ok()?.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        if !file_name(&path).is_some_and(|name| is_log(&name)) {
            continue;
        }
        let size = entry.metadata().map_or(0, |meta| meta.len());
        candidates.push((path, size));
    }
    candidates.sort_by_key(|(log, _)| std::cmp::Reverse(modified(log)));
    candidates.into_iter().next()
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// 目录或文件的修改时间; 读不到时返回 None.
pub(crate) fn modified(path: &Path) -> Option<SystemTime> {
    fs::symlink_metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn is_dump(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".dmp")
}

fn is_log(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".log")
}

/// 崩溃目录里值得带走的文本文件.
fn is_text(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [".xml", ".runtime-xml", ".log", ".ini", ".txt"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// 读取上限内的内容; 只接受普通文件, 不跟随符号链接.
fn read_capped(path: &Path, limit: u64, tail: bool) -> io::Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "不是普通文件"));
    }
    let truncated = tail && metadata.len() > limit;
    if tail {
        file.seek(SeekFrom::Start(metadata.len().saturating_sub(limit)))?;
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    // 从中间截断时丢掉开头不完整的行, 避免首行出现半截内容.
    if truncated {
        if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=end);
        }
    }
    Ok(bytes)
}

/// 崩溃现场偶尔混入 NUL 或写成 UTF-16, 先规整成可按行解码的 UTF-8.
fn as_text(bytes: &[u8]) -> Vec<u8> {
    let (be, rest) = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (false, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (true, rest)
    } else {
        return bytes.iter().copied().filter(|byte| *byte != 0).collect();
    };
    let units: Vec<u16> = rest
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| {
            if be {
                u16::from_be_bytes([pair[0], pair[1]])
            } else {
                u16::from_le_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units).into_bytes()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{crash_site, TempDir};
    use std::os::unix::fs::{symlink, PermissionsExt};

    /// 测试里 prefix 固定在临时目录的 prefix 子目录.
    fn site(dir: &TempDir, user: &str, name: &str) -> PathBuf {
        crash_site(&dir.join("prefix"), user, name)
    }

    #[test]
    fn missing_prefix_and_users_quietly_yield_nothing() {
        let dir = TempDir::new("crash-missing");
        assert!(collect_prefix(&dir.join("absent"), false).is_empty());
        fs::create_dir_all(dir.join("prefix/drive_c")).unwrap();
        assert!(collect_prefix(&dir.join("prefix"), false).is_empty());
        fs::create_dir_all(dir.join("prefix/drive_c/users/player")).unwrap();
        assert!(collect_prefix(&dir.join("prefix"), false).is_empty());
    }

    #[test]
    fn finds_crash_directories_for_any_user_name_and_keeps_the_newest() {
        let dir = TempDir::new("crash-sites");
        for index in 0..4 {
            let name = format!("UECC-fixture{index}_0000");
            let site = site(&dir, "steamuser", &name);
            dir.write(
                site.join("CrashContext.runtime-xml"),
                format!("<CrashContext><CrashType>Hang</CrashType><Index>{index}</Index></CrashContext>"),
            );
        }
        // 只保留最近三次, 旧的那个不进导出包.
        fs::File::open(site(&dir, "steamuser", "UECC-fixture0_0000"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        let export = collect_prefix(&dir.join("prefix"), false);
        assert_eq!(export.sites, 3);
        let names: Vec<&str> = export.files.iter().map(|file| file.name.as_str()).collect();
        assert!(names
            .contains(&"crashes/steamuser/Crashes/UECC-fixture1_0000/CrashContext.runtime-xml"));
        assert!(names
            .contains(&"crashes/steamuser/Crashes/UECC-fixture3_0000/CrashContext.runtime-xml"));
        assert!(!names.iter().any(|name| name.contains("UECC-fixture0")));
        // 用户目录从 drive_c/users 枚举, 不假设 steamuser.
        let other = site(&dir, "player", "UECC-player_0000");
        dir.write(other.join("CrashContext.runtime-xml"), "<CrashContext/>");
        let export = collect_prefix(&dir.join("prefix"), false);
        assert!(export
            .files
            .iter()
            .any(|file| file.name.starts_with("crashes/player/Crashes/")));
    }

    #[test]
    fn dumps_are_listed_but_not_decoded_as_text() {
        let dir = TempDir::new("crash-dumps");
        let site = site(&dir, "player", "UECC-dump_0000");
        // 转储含 NUL, 走文本通道会直接失败.
        dir.write(site.join("UEMinidump.dmp"), b"MDMP\0\0\0\x93\xa7");
        dir.write(site.join("CrashContext.runtime-xml"), "<CrashContext/>");
        let export = collect_prefix(&dir.join("prefix"), false);
        assert!(export.files.iter().all(|file| !file.name.ends_with(".dmp")));
        let manifest = manifest_text(&export);
        assert!(manifest.contains("UEMinidump.dmp"));
        assert!(manifest.contains("未包含 (二进制转储)"));
        // 只有明确要求时才原样写入.
        let included = collect_prefix(&dir.join("prefix"), true);
        let dump = included
            .files
            .iter()
            .find(|file| file.name.ends_with(".dmp"))
            .unwrap();
        assert!(matches!(dump.data, CrashData::Binary(_)));
    }

    #[test]
    fn logs_are_tail_truncated_and_links_are_not_followed() {
        let dir = TempDir::new("crash-logs");
        let site = site(&dir, "player", "UECC-tail_0000");
        dir.write(site.join("CrashContext.runtime-xml"), "<CrashContext/>");
        let logs = dir.join("prefix/drive_c/users/player/AppData/Local/HT/Saved/Logs");
        fs::create_dir_all(&logs).unwrap();
        let long = "x".repeat(MAX_LOG_TAIL as usize + 1024);
        fs::write(logs.join("HT.log"), format!("head{}{long}", "y".repeat(8))).unwrap();
        symlink("/etc/passwd", logs.join("leak.log")).unwrap();
        let export = collect_prefix(&dir.join("prefix"), false);
        let log = export
            .files
            .iter()
            .find(|file| file.name.ends_with("/Logs/HT.log"))
            .expect("保留游戏日志");
        let CrashData::Text(bytes) = &log.data else {
            panic!("游戏日志应走文本通道");
        };
        assert!(bytes.len() as u64 <= MAX_LOG_TAIL + 1);
        assert!(!bytes.starts_with(b"head"));
        let manifest = manifest_text(&export);
        assert!(manifest.contains("HT.log"));
        assert!(manifest.contains("已包含 (截断)"));
        // 符号链接不进入收集结果.
        assert!(export.files.iter().all(|file| !file.name.contains("leak")));
        let leak = export.files.iter().find(|file| file.name.ends_with(".log"));
        assert_eq!(leak.unwrap().name, "crashes/player/Logs/HT.log");
    }

    #[test]
    fn unreadable_site_is_skipped_without_failing_the_export() {
        let dir = TempDir::new("crash-unreadable");
        let site = site(&dir, "player", "UECC-private_0000");
        dir.write(site.join("CrashContext.runtime-xml"), "<CrashContext/>");
        fs::set_permissions(&site, fs::Permissions::from_mode(0o000)).unwrap();
        let export = collect_prefix(&dir.join("prefix"), false);
        fs::set_permissions(&site, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(export.is_empty());
    }

    /// 清单是收集结果里的第一个条目.
    fn manifest_text(export: &CrashExport) -> String {
        let CrashData::Text(bytes) = &export.files[0].data else {
            panic!("清单应是文本");
        };
        String::from_utf8(bytes.clone()).unwrap()
    }
}
