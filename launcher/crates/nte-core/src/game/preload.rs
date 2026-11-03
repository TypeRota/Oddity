//! 预下载缓存目录、完成记录与清理.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::config::GameUpdateConfig;
use super::manifest::{PreloadConfig, RemoteConfig};
use crate::{Error, Result};

const MARKER: &str = "complete.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub version: String,
    pub files: usize,
    pub bytes: u64,
    pub completed_at: u64,
}

/// 未配置时沿用官方 PatcherSDK 下的隔离目录.
pub fn root(config: &GameUpdateConfig) -> PathBuf {
    config
        .preload_root
        .clone()
        .unwrap_or_else(|| config.patcher_dir.join("preload"))
}

/// 创建并返回规范化后的根目录, 供暂存路径和符号链接检查使用.
pub fn prepare_root(config: &GameUpdateConfig) -> Result<PathBuf> {
    let root = root(config);
    fs::create_dir_all(&root)?;
    root.canonicalize().map_err(Error::Io)
}

fn version_dir(config: &GameUpdateConfig, branch: &str, version: &str) -> PathBuf {
    root(config).join(branch).join(version)
}

fn checked_dir(config: &GameUpdateConfig, branch: &str, version: &str) -> Result<PathBuf> {
    if !crate::paths::valid_segment(branch) || !crate::paths::valid_segment(version) {
        return Err(Error::Integrity("无效的预下载路径段".into()));
    }
    let branch_dir = root(config).join(branch);
    let dir = version_dir(config, branch, version);
    for path in [&branch_dir, &dir] {
        if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(Error::Integrity("预下载缓存路径是符号链接".into()));
        }
    }
    Ok(dir)
}

pub fn completion(config: &GameUpdateConfig, branch: &str, version: &str) -> Option<Completion> {
    let dir = checked_dir(config, branch, version).ok()?;
    let bytes = fs::read(dir.join(MARKER)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn mark_complete(
    config: &GameUpdateConfig,
    branch: &str,
    version: &str,
    files: usize,
    bytes: u64,
) -> Result<()> {
    let dir = checked_dir(config, branch, version)?;
    fs::create_dir_all(&dir)?;
    let completion = Completion {
        version: version.to_string(),
        files,
        bytes,
        completed_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    };
    let encoded =
        serde_json::to_vec_pretty(&completion).map_err(|error| Error::Parse(error.to_string()))?;
    let path = dir.join(MARKER);
    let mut output = crate::paths::TemporaryFile::new(&path)?;
    use std::io::Write;
    output.file.write_all(&encoded)?;
    output.commit(&path, &|| false)
}

/// 版本目录当前占用的字节数; 目录不存在时为 0.
pub fn cache_bytes(config: &GameUpdateConfig, branch: &str, version: &str) -> u64 {
    let Ok(dir) = checked_dir(config, branch, version) else {
        return 0;
    };
    cache_bytes_in(&dir)
}

fn cache_bytes_in(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        // 不跟随符号链接, 避免统计缓存根之外的目录.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            total = total.saturating_add(cache_bytes_in(&entry.path()));
        } else if kind.is_file() {
            total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    total
}

/// 最近写入的非空预下载缓存, 用于公告消失后继续提供清理入口.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedVersion {
    pub branch: String,
    pub version: String,
    pub bytes: u64,
    pub completed: bool,
}

/// 列举缓存根下的有效非空版本及修改时间.
fn cached_versions(config: &GameUpdateConfig) -> Vec<(u64, CachedVersion)> {
    let root = root(config);
    let Ok(branches) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut candidates: Vec<(u64, String, String)> = Vec::new();
    for branch in branches.flatten().take(64) {
        if !branch.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(name) = branch.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !crate::paths::valid_segment(&name) {
            continue;
        }
        let Ok(versions) = fs::read_dir(branch.path()) else {
            continue;
        };
        for version in versions.flatten().take(64) {
            if !version.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            // 只认下载器写入的目录结构, 避免把误设的缓存根当成可删除缓存.
            let native = version.path().join("nte-native");
            let marker = version.path().join(MARKER);
            let native_dir = fs::symlink_metadata(&native).is_ok_and(|meta| meta.is_dir());
            let marker_file = fs::symlink_metadata(&marker).is_ok_and(|meta| meta.is_file());
            if !native_dir && !marker_file {
                continue;
            }
            let Some(version_name) = version.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !crate::paths::valid_segment(&version_name) {
                continue;
            }
            let modified = version
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|time| time.as_millis() as u64)
                .unwrap_or(0);
            candidates.push((modified, name.clone(), version_name));
        }
    }
    let mut versions = Vec::new();
    for (modified, branch, version) in candidates {
        let bytes = cache_bytes(config, &branch, &version);
        if bytes == 0 {
            continue;
        }
        versions.push((
            modified,
            CachedVersion {
                completed: completion(config, &branch, &version).is_some(),
                branch,
                version,
                bytes,
            },
        ));
    }
    versions
}

/// 最近写入的缓存用于清理入口, 不决定未来版本的优先级.
pub fn cached_version(config: &GameUpdateConfig) -> Option<CachedVersion> {
    cached_versions(config)
        .into_iter()
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.version.cmp(&b.1.version)))
        .map(|(_, cached)| cached)
}

/// 从全部非空缓存和公告中选择最高未来版本; 同版本优先公告分支.
pub fn target(config: &GameUpdateConfig, remote: &RemoteConfig) -> Option<PreloadConfig> {
    let mut cached = cached_versions(config);
    cached.sort_by_key(|(modified, _)| *modified);
    cached
        .into_iter()
        .map(|(_, cached)| PreloadConfig {
            branch: cached.branch,
            version: cached.version,
        })
        .chain(remote.preload.clone())
        .filter(|candidate| {
            crate::version::compare(&candidate.version, &remote.res_version)
                == std::cmp::Ordering::Greater
        })
        .max_by(|a, b| crate::version::compare(&a.version, &b.version))
}

/// 删除一个版本的预下载缓存; 返回删除前的字节数.
pub fn delete(config: &GameUpdateConfig, branch: &str, version: &str) -> Result<u64> {
    let dir = checked_dir(config, branch, version)?;
    if !dir.exists() {
        return Ok(0);
    }
    let bytes = cache_bytes(config, branch, version);
    fs::remove_dir_all(&dir)?;
    // 只清理空的父目录, 保留用户选择的自定义根目录.
    let branch_dir = dir.parent();
    if let Some(branch_dir) = branch_dir {
        if fs::read_dir(branch_dir).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(branch_dir);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn config_for(dir: &TempDir) -> GameUpdateConfig {
        GameUpdateConfig {
            launcher_root: dir.join("install/NTELauncher"),
            install_root: dir.join("install"),
            config_dir: dir.join("install/NTELauncher/ResFilesM/1289/PatcherConfig"),
            patcher_dir: dir.join("install/NTELauncher/UserData/Patcher/PatcherSDK"),
            branch: "publish_ob".to_string(),
            update_branch: "publish_Updata".to_string(),
            app_id: "1289".to_string(),
            cdns: vec!["https://cdn.test/clientRes".to_string()],
            rules: Vec::new(),
            check_dirs: Vec::new(),
            max_threads: 5,
            preload_root: None,
        }
    }

    #[test]
    fn completion_marker_round_trips_and_delete_removes_the_version() {
        let dir = TempDir::new("preload-record");
        let config = config_for(&dir);
        assert!(completion(&config, "future", "2.0").is_none());
        dir.write(
            "install/NTELauncher/UserData/Patcher/PatcherSDK/preload/future/2.0/nte-native/Res/a.part",
            b"cache",
        );
        mark_complete(&config, "future", "2.0", 1, 5).unwrap();
        let record = completion(&config, "future", "2.0").unwrap();
        assert_eq!(record.version, "2.0");
        assert_eq!(record.files, 1);
        assert_eq!(record.bytes, 5);
        assert!(cache_bytes(&config, "future", "2.0") > 0);
        assert!(delete(&config, "future", "2.0").unwrap() > 0);
        assert!(completion(&config, "future", "2.0").is_none());
        assert!(!dir
            .join("install/NTELauncher/UserData/Patcher/PatcherSDK/preload/future")
            .exists());
    }

    #[test]
    fn rejects_path_segments_outside_the_cache_root() {
        let dir = TempDir::new("preload-traversal");
        let config = config_for(&dir);
        for (branch, version) in [("..", "2.0"), ("future", "../../escape"), ("/abs", "2.0")] {
            assert!(mark_complete(&config, branch, version, 0, 0).is_err());
            assert!(delete(&config, branch, version).is_err());
            assert!(completion(&config, branch, version).is_none());
            assert_eq!(cache_bytes(&config, branch, version), 0);
        }
        assert!(!dir.join("escape").exists());
    }

    #[test]
    fn symlinked_cache_versions_are_ignored() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("preload-symlink");
        let config = config_for(&dir);
        let outside = dir.join("outside");
        fs::create_dir_all(outside.join("nte-native/Res")).unwrap();
        fs::write(outside.join("nte-native/Res/a.part"), b"outside").unwrap();
        let branch = config
            .patcher_dir
            .join("preload")
            .join("future")
            .join("2.0");
        fs::create_dir_all(branch.parent().unwrap()).unwrap();
        symlink(&outside, &branch).unwrap();
        assert_eq!(cache_bytes(&config, "future", "2.0"), 0);
        assert!(completion(&config, "future", "2.0").is_none());
        assert!(delete(&config, "future", "2.0").is_err());
        assert!(mark_complete(&config, "future", "2.0", 0, 0).is_err());
        assert!(outside.join("nte-native/Res/a.part").is_file());
    }

    #[test]
    fn highest_future_target_is_independent_of_the_latest_cleanup_target() {
        let dir = TempDir::new("preload-future-target");
        let config = config_for(&dir);
        for (version, seconds) in [("3.10", 1), ("3.9", 2)] {
            let cache = root(&config).join("cached").join(version);
            fs::create_dir_all(cache.join("nte-native/Res")).unwrap();
            fs::write(cache.join("nte-native/Res/a.part"), b"cache").unwrap();
            fs::File::open(&cache)
                .unwrap()
                .set_times(
                    fs::FileTimes::new()
                        .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
                )
                .unwrap();
        }
        let mut remote = RemoteConfig {
            res_version: "3.0".into(),
            ..Default::default()
        };
        assert_eq!(cached_version(&config).unwrap().version, "3.9");
        assert_eq!(target(&config, &remote).unwrap().version, "3.10");
        remote.preload = Some(PreloadConfig {
            branch: "announced".into(),
            version: "3.10".into(),
        });
        assert_eq!(target(&config, &remote).unwrap().branch, "announced");
        remote.preload.as_mut().unwrap().version = "3.11".into();
        assert_eq!(target(&config, &remote).unwrap().version, "3.11");
        remote.res_version = "3.11".into();
        assert!(target(&config, &remote).is_none());
    }

    #[test]
    fn cached_version_prefers_the_newest_nonempty_version() {
        let dir = TempDir::new("preload-cached-version");
        let config = config_for(&dir);
        dir.write(
            "install/NTELauncher/UserData/Patcher/PatcherSDK/preload/future/1.0/nte-native/Res/a.part",
            b"old",
        );
        dir.write(
            "install/NTELauncher/UserData/Patcher/PatcherSDK/preload/future/2.0/nte-native/Res/b.part",
            b"new",
        );
        // 非预下载目录不参与清理.
        dir.write(
            "install/NTELauncher/UserData/Patcher/PatcherSDK/preload/random/9.0/file",
            b"x",
        );
        let cached = cached_version(&config).unwrap();
        assert_eq!(cached.version, "2.0");
        assert_eq!(cached.bytes, 3);
        assert!(!cached.completed);
        assert!(cached_version(&config).is_some());
        delete(&config, "future", "2.0").unwrap();
        assert_eq!(cached_version(&config).unwrap().version, "1.0");
    }

    #[test]
    fn custom_root_is_used_for_the_cache() {
        let dir = TempDir::new("preload-custom-root");
        let mut config = config_for(&dir);
        config.preload_root = Some(dir.join("cache"));
        mark_complete(&config, "future", "2.0", 0, 0).unwrap();
        assert!(dir.join("cache/future/2.0/complete.json").is_file());
        assert!(!dir
            .join("install/NTELauncher/UserData/Patcher/PatcherSDK/preload")
            .exists());
    }
}
