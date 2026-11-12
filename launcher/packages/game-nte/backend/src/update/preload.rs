//! 预下载缓存目录、完成记录与清理.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::config::GameUpdateConfig;
use super::manifest::{PreloadConfig, RemoteConfig};
use oddity_platform::{Error, Result};

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
    if !crate::update::paths::valid_segment(branch) || !crate::update::paths::valid_segment(version)
    {
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
    let mut output = crate::update::paths::TemporaryFile::new(&path)?;
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
        if !crate::update::paths::valid_segment(&name) {
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
            if !crate::update::paths::valid_segment(&version_name) {
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
            oddity_platform::version::compare(&candidate.version, &remote.res_version)
                == std::cmp::Ordering::Greater
        })
        .max_by(|a, b| oddity_platform::version::compare(&a.version, &b.version))
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

/// 按未来版本清单删除一个可选包的独占缓存, 不修改正式安装文件.
/// config 与 bundle 必须来自同一次 load_preload_remote.
fn optional_cache_files(
    config: &GameUpdateConfig,
    bundle: &super::check::RemoteBundle,
    tag: &str,
) -> Result<Vec<(PathBuf, u64)>> {
    use std::collections::BTreeSet;

    let selected = bundle
        .manifest
        .base_versions
        .iter()
        .find(|base| base.tag == tag)
        .ok_or_else(|| Error::Parse(format!("未知的可选资源: {tag}")))?;
    let shared_paths: BTreeSet<_> = bundle
        .manifest
        .res
        .iter()
        .map(|entry| super::check::ledger_key(&entry.filename))
        .chain(bundle.manifest.packages.iter().flat_map(|package| {
            package.paks.iter().flat_map(|pak| {
                pak.entries
                    .iter()
                    .map(|entry| super::check::ledger_key(&entry.name))
            })
        }))
        .chain(
            bundle
                .manifest
                .base_versions
                .iter()
                .filter(|base| base.tag != tag)
                .flat_map(super::optional::entries)
                .map(|entry| super::check::ledger_key(entry.path())),
        )
        .collect();
    let mut hashes = BTreeSet::new();
    for entry in super::optional::entries(selected) {
        oddity_platform::paths::relative_path(entry.path())?;
        if !shared_paths.contains(&super::check::ledger_key(entry.path())) {
            let key = super::download::item_tag_for(entry.path(), 0);
            hashes.insert(key.trim_end_matches("-0").to_owned());
        }
    }
    let dir = checked_dir(config, &config.branch, &bundle.config.res_version)?;
    let cache_root = root(config);
    let mut files = Vec::new();
    for kind in ["Res", "Pak", "Patch"] {
        let directory = dir.join("nte-native").join(kind);
        let relative = directory
            .strip_prefix(&cache_root)
            .map_err(|_| Error::Integrity("预下载缓存超出缓存目录".into()))?;
        crate::update::paths::ensure_no_symlink(&cache_root, relative)?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if ![".part", ".cb.idx", ".tmp"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
            {
                continue;
            }
            // 全量, 差分及分块索引共用路径哈希; 同一路径的全部序号一起清理.
            let belongs = name.split('.').enumerate().any(|(index, part)| {
                (index == 0 || index == 2)
                    && part.rsplit_once('-').is_some_and(|(hash, ordinal)| {
                        hashes.contains(hash)
                            && !ordinal.is_empty()
                            && ordinal.bytes().all(|byte| byte.is_ascii_digit())
                    })
            });
            if !belongs {
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(&cache_root)
                .map_err(|_| Error::Integrity("预下载缓存超出缓存目录".into()))?;
            crate::update::paths::ensure_no_symlink(&cache_root, relative)?;
            if entry.file_type()?.is_file() {
                files.push((path, entry.metadata()?.len()));
            }
        }
    }
    Ok(files)
}

/// 返回该语种实际存在且可独立删除的缓存大小.
pub fn optional_cached_bytes(
    config: &GameUpdateConfig,
    bundle: &super::check::RemoteBundle,
    tag: &str,
) -> Result<u64> {
    Ok(optional_cache_files(config, bundle, tag)?
        .iter()
        .map(|(_, bytes)| bytes)
        .sum())
}

pub fn remove_optional(
    config: &GameUpdateConfig,
    bundle: &super::check::RemoteBundle,
    tag: &str,
) -> Result<u64> {
    let files = optional_cache_files(config, bundle, tag)?;
    let bytes = files.iter().map(|(_, bytes)| bytes).sum();
    let dir = checked_dir(config, &config.branch, &bundle.config.res_version)?;
    let cache_root = root(config);
    if files.is_empty() {
        return Ok(0);
    }
    let marker = dir.join(MARKER);
    let relative = marker
        .strip_prefix(&cache_root)
        .map_err(|_| Error::Integrity("预下载缓存超出缓存目录".into()))?;
    crate::update::paths::ensure_no_symlink(&cache_root, relative)?;
    // 先使完成状态失效, 中途失败后仍会重新检查剩余缓存.
    match fs::remove_file(marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for (path, _) in files {
        fs::remove_file(path)?;
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
    fn optional_cleanup_preserves_base_other_languages_and_shared_paths() {
        use crate::update::{
            check::RemoteBundle,
            manifest::{BaseVersion, GameManifest, Package, Pak, PakEntry, ResEntry},
        };
        let dir = TempDir::new("preload-optional-cleanup");
        let mut config = config_for(&dir);
        config.branch = "future".into();
        let voice = "Client/voice.pak";
        let signature = "Client/voice.sig";
        let shared = "Client/shared.pak";
        let other = "Client/other.pak";
        let bundle = RemoteBundle {
            config: RemoteConfig {
                res_version: "2.0".into(),
                ..Default::default()
            },
            manifest: GameManifest {
                res: vec![ResEntry {
                    filename: shared.into(),
                    ..Default::default()
                }],
                base_versions: vec![
                    BaseVersion {
                        tag: "pakchunk101".into(),
                        res: vec![
                            ResEntry {
                                filename: voice.into(),
                                ..Default::default()
                            },
                            ResEntry {
                                filename: shared.into(),
                                ..Default::default()
                            },
                        ],
                        packages: vec![Package {
                            paks: vec![Pak {
                                entries: vec![PakEntry {
                                    name: signature.into(),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            }],
                        }],
                        ..Default::default()
                    },
                    BaseVersion {
                        tag: "pakchunk102".into(),
                        res: vec![ResEntry {
                            filename: other.into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            diff: None,
        };
        let voice_tag = super::super::download::item_tag_for(voice, 0);
        let signature_tag = super::super::download::item_tag_for(signature, 0);
        let shared_tag = super::super::download::item_tag_for(shared, 0);
        let other_tag = super::super::download::item_tag_for(other, 0);
        let cache = root(&config).join("future/2.0/nte-native");
        let removed = [
            format!("Res/abcd.4.{voice_tag}.part"),
            format!("Res/abcd.4.{voice_tag}.cb.idx"),
            format!(
                "Res/abcd.4.{}.part",
                super::super::download::item_tag_for(voice, 1)
            ),
            format!("Pak/abcd.4.{signature_tag}.part"),
            format!("Patch/abcd.4.{voice_tag}.part"),
            format!("Patch/{voice_tag}.0.abcd.4.tmp"),
        ];
        let kept = [
            format!("Res/abcd.4.{shared_tag}.part"),
            format!("Res/abcd.4.{other_tag}.part"),
        ];
        for path in removed.iter().chain(&kept) {
            let path = cache.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"test").unwrap();
        }
        fs::create_dir_all(config.install_root.join("Client")).unwrap();
        fs::write(config.install_root.join(voice), b"installed").unwrap();
        mark_complete(&config, "future", "2.0", 8, 32).unwrap();
        assert_eq!(
            optional_cached_bytes(&config, &bundle, "pakchunk101").unwrap(),
            24
        );
        assert_eq!(
            optional_cached_bytes(&config, &bundle, "pakchunk102").unwrap(),
            4
        );
        assert!(completion(&config, "future", "2.0").is_some());
        assert_eq!(
            remove_optional(&config, &bundle, "pakchunk101").unwrap(),
            24
        );
        assert!(removed.iter().all(|path| !cache.join(path).exists()));
        assert!(kept.iter().all(|path| cache.join(path).is_file()));
        assert!(completion(&config, "future", "2.0").is_none());
        assert_eq!(
            fs::read(config.install_root.join(voice)).unwrap(),
            b"installed"
        );
        assert_eq!(
            optional_cached_bytes(&config, &bundle, "pakchunk101").unwrap(),
            0
        );
        assert_eq!(
            optional_cached_bytes(&config, &bundle, "pakchunk102").unwrap(),
            4
        );
        assert_eq!(remove_optional(&config, &bundle, "pakchunk101").unwrap(), 0);
        assert!(remove_optional(&config, &bundle, "unknown").is_err());
    }

    #[test]
    fn optional_cleanup_rejects_symlinked_cache_before_removing_files() {
        use crate::update::{
            check::RemoteBundle,
            manifest::{BaseVersion, GameManifest, ResEntry},
        };
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("preload-optional-symlink");
        let mut config = config_for(&dir);
        config.branch = "future".into();
        let path = "Client/voice.pak";
        let bundle = RemoteBundle {
            config: RemoteConfig {
                res_version: "2.0".into(),
                ..Default::default()
            },
            manifest: GameManifest {
                base_versions: vec![BaseVersion {
                    tag: "pakchunk101".into(),
                    res: vec![ResEntry {
                        filename: path.into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            diff: None,
        };
        let key = super::super::download::item_tag_for(path, 0);
        let cache = root(&config).join("future/2.0/nte-native/Res");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join(format!("aaaa.4.{key}.part")), b"safe").unwrap();
        let outside = dir.join("outside");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, cache.join(format!("bbbb.7.{key}.part"))).unwrap();
        mark_complete(&config, "future", "2.0", 2, 11).unwrap();
        assert!(remove_optional(&config, &bundle, "pakchunk101").is_err());
        assert!(cache.join(format!("aaaa.4.{key}.part")).is_file());
        assert!(completion(&config, "future", "2.0").is_some());
        assert_eq!(fs::read(outside).unwrap(), b"outside");
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
