//! 运行环境目录的孤儿扫描与安全清理.
//!
//! 只读扫描 data/environments 与 data/logs/environments 中未登记到设置的目录,
//! 删除必须按类型和名称重新计算路径, 不跟随符号链接.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::environment;

/// 未登记目录的用途分类.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrphanKind {
    Environment,
    Logs,
    Pending,
}

impl OrphanKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Logs => "logs",
            Self::Pending => "pending",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "environment" => Some(Self::Environment),
            "logs" => Some(Self::Logs),
            "pending" => Some(Self::Pending),
            _ => None,
        }
    }
}

/// 仍占用磁盘但未登记的目录.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Orphan {
    pub kind: OrphanKind,
    /// 目录名; pending 为 .creating-<id> 或 .deleting-<id>.
    pub name: String,
    pub id: Option<String>,
    pub bytes: u64,
    /// 目录最后修改时间, 单位秒.
    pub modified_at: u64,
    pub identity_valid: bool,
    pub prefix_valid: bool,
}

/// 单次扫描最多统计的目录条目数; 超出后保留已统计的体积.
const MAX_ENTRIES: u64 = 100_000;

/// 扫描未登记的环境与日志目录; 结果按 (类型, 名称) 排序.
pub fn scan(data: &Path, registered: &BTreeSet<String>) -> Result<Vec<Orphan>, String> {
    let mut orphans = Vec::new();
    scan_environments(&data.join("environments"), registered, &mut orphans)?;
    scan_logs(&data.join("logs/environments"), registered, &mut orphans)?;
    orphans.sort_by(|left, right| {
        (left.kind, left.name.as_str()).cmp(&(right.kind, right.name.as_str()))
    });
    Ok(orphans)
}

/// 删除一个未登记目录; 名称只接受扫描会返回的固定形状.
pub fn remove(
    data: &Path,
    registered: &BTreeSet<String>,
    kind: OrphanKind,
    name: &str,
) -> Result<(), String> {
    let path = orphan_path(data, registered, kind, name)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("读取 {} 失败: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("目标不是普通目录: {}", path.display()));
    }
    // prefix 仍可能被运行中的程序使用, 删除前先取锁.
    let _lock = if kind != OrphanKind::Logs && environment::prefix_ready(&path) {
        Some(environment::lock_prefix(&path.join("prefix")).map_err(|error| format!("{error:#}"))?)
    } else {
        None
    };
    let mounts = oddity_platform::fs_tree::mount_points_under(&path)
        .map_err(|error| format!("检查挂载点失败: {error}"))?;
    if !mounts.is_empty() {
        return Err(format!("目标内含挂载点, 无法清理: {}", path.display()));
    }
    oddity_platform::fs_tree::remove_tree(&path)
        .map_err(|error| format!("清理 {} 失败: {error}", path.display()))?;
    Ok(())
}

/// 严格按类型与名称还原待清理目录, 拒绝任意路径.
fn orphan_path(
    data: &Path,
    registered: &BTreeSet<String>,
    kind: OrphanKind,
    name: &str,
) -> Result<PathBuf, String> {
    let (root, id) = match kind {
        OrphanKind::Environment => (data.join("environments"), hex_id(name)),
        OrphanKind::Logs => (data.join("logs/environments"), hex_id(name)),
        OrphanKind::Pending => (data.join("environments"), pending_id(name)),
    };
    let id = id.ok_or_else(|| format!("未登记目录名称无效: {name}"))?;
    // .creating-<id> 只是上次创建中断留下的残名, 不是登记目录本体.
    let protected = match kind {
        OrphanKind::Pending => !name.starts_with(".creating-"),
        _ => true,
    };
    if protected && registered.contains(id) {
        return Err(format!("运行环境 {id} 已登记, 不能清理"));
    }
    Ok(root.join(name))
}

fn scan_environments(
    root: &Path,
    registered: &BTreeSet<String>,
    orphans: &mut Vec<Orphan>,
) -> Result<(), String> {
    for (name, path, metadata) in read_directories(root)? {
        let Some((kind, id)) = classify(&name, registered) else {
            continue;
        };
        let identity_valid = environment::owned_identity(&path, &id);
        let prefix_valid = environment::prefix_ready(&path);
        orphans.push(Orphan {
            kind,
            name,
            id: Some(id),
            bytes: directory_bytes(&path),
            modified_at: modified_seconds(&metadata),
            identity_valid,
            prefix_valid,
        });
    }
    Ok(())
}

/// 已登记环境的日志随登记保留, 只报告未登记 id 的目录.
fn scan_logs(
    root: &Path,
    registered: &BTreeSet<String>,
    orphans: &mut Vec<Orphan>,
) -> Result<(), String> {
    for (name, path, metadata) in read_directories(root)? {
        let Some(id) = hex_id(&name).map(str::to_string) else {
            continue;
        };
        if registered.contains(&id) {
            continue;
        }
        orphans.push(Orphan {
            kind: OrphanKind::Logs,
            name,
            id: Some(id),
            bytes: directory_bytes(&path),
            modified_at: modified_seconds(&metadata),
            identity_valid: false,
            prefix_valid: false,
        });
    }
    Ok(())
}

/// 目录名分类: 32 位 id 为已建环境, .creating-/.deleting- 为未完成操作.
fn classify(name: &str, registered: &BTreeSet<String>) -> Option<(OrphanKind, String)> {
    if let Some(id) = hex_id(name) {
        return (!registered.contains(id)).then(|| (OrphanKind::Environment, id.to_string()));
    }
    if let Some(id) = name.strip_prefix(".creating-").and_then(hex_id) {
        return Some((OrphanKind::Pending, id.to_string()));
    }
    if let Some(id) = name.strip_prefix(".deleting-").and_then(hex_id) {
        // 已登记目录的删除会由启动恢复完成, 不按孤儿处理.
        return (!registered.contains(id)).then(|| (OrphanKind::Pending, id.to_string()));
    }
    None
}

fn read_directories(root: &Path) -> Result<Vec<(String, PathBuf, fs::Metadata)>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取 {} 失败: {error}", root.display())),
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取 {} 失败: {error}", root.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        directories.push((name, path, metadata));
    }
    Ok(directories)
}

fn hex_id(name: &str) -> Option<&str> {
    environment::valid_id(name).then_some(name)
}

fn pending_id(name: &str) -> Option<&str> {
    let id = name
        .strip_prefix(".creating-")
        .or_else(|| name.strip_prefix(".deleting-"))?;
    hex_id(id)
}

fn modified_seconds(metadata: &fs::Metadata) -> u64 {
    u64::try_from(metadata.mtime()).unwrap_or(0)
}

/// 递归累加真实文件大小; 符号链接按 0 计, 不跟随到安装目录.
fn directory_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut visited = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_ENTRIES {
                return total;
            }
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn id(byte: char) -> String {
        byte.to_string().repeat(32)
    }

    fn registered(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn environment_dir(data: &Path, name: &str, marker: Option<&str>, prefix: bool) -> PathBuf {
        let directory = data.join("environments").join(name);
        fs::create_dir_all(&directory).unwrap();
        if let Some(marker) = marker {
            fs::write(directory.join(".oddity-environment"), marker).unwrap();
        }
        if prefix {
            fs::create_dir_all(directory.join("prefix/drive_c")).unwrap();
        }
        directory
    }

    #[test]
    fn scan_reports_unregistered_environment_directories() {
        let dir = TempDir::new("environment-inventory-scan");
        let data = dir.join("data");
        let first = id('a');
        let second = id('b');
        environment_dir(&data, &second, Some(&second), true);
        environment_dir(&data, &first, Some(&first), true);
        fs::create_dir_all(data.join("environments/notes")).unwrap();

        let orphans = scan(&data, &BTreeSet::new()).unwrap();
        assert_eq!(orphans.len(), 2);
        assert_eq!(orphans[0].kind, OrphanKind::Environment);
        assert_eq!(orphans[0].name, first);
        assert_eq!(orphans[0].id.as_deref(), Some(first.as_str()));
        assert!(orphans[0].identity_valid);
        assert!(orphans[0].prefix_valid);
        assert!(orphans[0].modified_at > 0);
        assert_eq!(orphans[1].name, second);
        assert_eq!(orphans[0].kind.as_str(), "environment");
        assert_eq!(orphans[1].kind.as_str(), "environment");

        let orphans = scan(&data, &registered(&[&first])).unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].name, second);
    }

    #[test]
    fn scan_treats_missing_directories_as_empty() {
        let dir = TempDir::new("environment-inventory-missing");
        assert!(scan(&dir.join("data"), &BTreeSet::new())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn scan_reports_creating_and_deleting_places_but_skips_registered_ones() {
        let dir = TempDir::new("environment-inventory-pending");
        let data = dir.join("data");
        let creating = id('a');
        let deleting = id('b');
        let registered_id = id('c');
        environment_dir(&data, &format!(".creating-{creating}"), None, false);
        // 已登记环境也可能残留创建中断的名字, 仍要作为孤儿报告.
        environment_dir(&data, &format!(".creating-{registered_id}"), None, false);
        environment_dir(
            &data,
            &format!(".deleting-{deleting}"),
            Some(&deleting),
            true,
        );
        environment_dir(
            &data,
            &format!(".deleting-{registered_id}"),
            Some(&registered_id),
            true,
        );

        let orphans = scan(&data, &registered(&[&registered_id])).unwrap();
        assert_eq!(orphans.len(), 3);
        assert!(orphans
            .iter()
            .all(|orphan| orphan.kind == OrphanKind::Pending));
        assert_eq!(orphans[0].name, format!(".creating-{creating}"));
        assert_eq!(orphans[0].id.as_deref(), Some(creating.as_str()));
        assert!(!orphans[0].identity_valid);
        assert!(!orphans[0].prefix_valid);
        assert_eq!(orphans[1].name, format!(".creating-{registered_id}"));
        assert_eq!(orphans[2].name, format!(".deleting-{deleting}"));
        assert!(orphans[2].identity_valid);
        assert!(orphans[2].prefix_valid);
        assert_eq!(orphans[2].kind.as_str(), "pending");
        assert_eq!(OrphanKind::parse("pending"), Some(OrphanKind::Pending));
        assert_eq!(OrphanKind::parse("other"), None);
    }

    #[test]
    fn scan_reports_logs_of_unregistered_environments() {
        let dir = TempDir::new("environment-inventory-logs");
        let data = dir.join("data");
        let orphan = id('a');
        let registered_id = id('b');
        fs::create_dir_all(data.join("logs/environments").join(&orphan).join("run")).unwrap();
        fs::create_dir_all(data.join("logs/environments").join(&registered_id)).unwrap();
        fs::create_dir_all(data.join("logs/environments/notes")).unwrap();

        let orphans = scan(&data, &registered(&[&registered_id])).unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].kind, OrphanKind::Logs);
        assert_eq!(orphans[0].name, orphan);
        assert!(!orphans[0].identity_valid);
        assert!(!orphans[0].prefix_valid);
        assert_eq!(OrphanKind::parse("logs"), Some(OrphanKind::Logs));
    }

    #[test]
    fn scan_does_not_follow_links_when_measuring_usage() {
        let dir = TempDir::new("environment-inventory-bytes");
        let data = dir.join("data");
        let environment = id('a');
        let directory = environment_dir(&data, &environment, Some(&environment), true);
        fs::write(directory.join("prefix/drive_c/game.pak"), b"1234567890").unwrap();
        let external = dir.write("external/blob.bin", vec![7u8; 1_000_000]);
        std::os::unix::fs::symlink(&external, directory.join("prefix/drive_c/blob.bin")).unwrap();
        let game = dir.write("install/game.pak", vec![1u8; 2_000_000]);
        std::os::unix::fs::symlink(&game, directory.join("prefix/drive_c/game")).unwrap();

        let orphans = scan(&data, &BTreeSet::new()).unwrap();
        // 标记文件 32 字节加真实数据文件 10 字节, 链接目标不计入.
        assert_eq!(orphans[0].bytes, 42);
    }

    #[test]
    fn remove_rejects_foreign_names_and_registered_environments() {
        let dir = TempDir::new("environment-inventory-remove-reject");
        let data = dir.join("data");
        let registered_id = id('b');
        environment_dir(&data, &registered_id, Some(&registered_id), true);
        let set = registered(&[&registered_id]);
        let attempts = [
            (OrphanKind::Environment, "../source".to_string()),
            (OrphanKind::Environment, "notes".to_string()),
            (OrphanKind::Environment, registered_id.clone()),
            (OrphanKind::Logs, registered_id.clone()),
            (OrphanKind::Pending, registered_id.clone()),
            (OrphanKind::Pending, format!(".deleting-{registered_id}")),
        ];
        for (kind, name) in attempts {
            assert!(remove(&data, &set, kind, &name).is_err(), "{kind:?} {name}");
        }

        let link_name = id('c');
        std::os::unix::fs::symlink(
            dir.join("missing"),
            data.join("environments").join(&link_name),
        )
        .unwrap();
        assert!(remove(&data, &BTreeSet::new(), OrphanKind::Environment, &link_name).is_err());
        assert!(data.join("environments").join(&registered_id).exists());
    }

    #[test]
    fn remove_deletes_unregistered_real_directories() {
        let dir = TempDir::new("environment-inventory-remove");
        let data = dir.join("data");
        let environment = id('a');
        let creating = id('b');
        let logs = id('c');
        let environment_path = environment_dir(&data, &environment, Some(&environment), true);
        let pending_path = environment_dir(&data, &format!(".creating-{creating}"), None, false);
        let logs_path = data.join("logs/environments").join(&logs);
        fs::create_dir_all(logs_path.join("run")).unwrap();

        let set = BTreeSet::new();
        remove(&data, &set, OrphanKind::Environment, &environment).unwrap();
        assert!(!environment_path.exists());
        remove(
            &data,
            &set,
            OrphanKind::Pending,
            &format!(".creating-{creating}"),
        )
        .unwrap();
        assert!(!pending_path.exists());
        remove(&data, &set, OrphanKind::Logs, &logs).unwrap();
        assert!(!logs_path.exists());
        assert!(scan(&data, &set).unwrap().is_empty());
    }

    #[test]
    fn remove_deletes_the_creating_place_of_a_registered_environment() {
        let dir = TempDir::new("environment-inventory-remove-creating");
        let data = dir.join("data");
        let registered_id = id('a');
        let creating = environment_dir(&data, &format!(".creating-{registered_id}"), None, false);
        let deleting = environment_dir(
            &data,
            &format!(".deleting-{registered_id}"),
            Some(&registered_id),
            true,
        );
        let set = registered(&[&registered_id]);

        // .deleting-<id> 是启动恢复目标, 已登记时不能清理.
        assert!(remove(
            &data,
            &set,
            OrphanKind::Pending,
            &format!(".deleting-{registered_id}")
        )
        .is_err());
        assert!(deleting.exists());

        remove(
            &data,
            &set,
            OrphanKind::Pending,
            &format!(".creating-{registered_id}"),
        )
        .unwrap();
        assert!(!creating.exists());
        assert!(deleting.exists());
        assert!(scan(&data, &set).unwrap().is_empty());
    }
}
