//! 可选资源目录、安装状态及安装目录内的选择记录.

use std::collections::BTreeSet;
use std::fs;

use serde::{Deserialize, Serialize};

use super::check::{RemoteBundle, UpdateKind};
use super::config::GameUpdateConfig;
use super::manifest::BaseVersion;
use crate::{Error, Result};

#[derive(Debug, Clone, Default)]
pub enum OptionalSelection {
    /// 维护已存在的资源, 并下载本安装目录显式选择的资源.
    #[default]
    Configured,
    Tags(Vec<String>),
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceState {
    Absent,
    Partial,
    Installed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionalResource {
    pub tag: String,
    pub name: String,
    pub version: String,
    pub size: u64,
    /// 文件存在状态, 不代表已完成本轮完整性校验.
    pub state: ResourceState,
    /// 根据标签版本和文件大小判断是否需要更新, 不替代完整性校验.
    pub needs_update: bool,
    pub selected: bool,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    tags: Vec<String>,
}

/// 已确认语义的 tag 使用面向玩家的名称, 其余保留官方标识.
fn display_name(tag: &str) -> String {
    match tag {
        "pakchunk101" => "中文语音",
        "pakchunk102" => "日语语音",
        "pakchunk103" => "英语语音",
        "pakchunk104" => "韩语语音",
        _ => tag,
    }
    .into()
}

pub(crate) fn entries(base: &BaseVersion) -> impl Iterator<Item = UpdateKind> + '_ {
    base.res
        .iter()
        .cloned()
        .map(|res| UpdateKind::Res { res })
        .chain(
            base.packages
                .iter()
                .flat_map(|package| &package.paks)
                .flat_map(|pak| {
                    pak.entries
                        .iter()
                        .cloned()
                        .map(|entry| UpdateKind::PakEntry {
                            pak_md5: pak.md5.clone(),
                            pak_size: pak.filesize,
                            entry,
                        })
                }),
        )
}

pub fn catalog(config: &GameUpdateConfig, bundle: &RemoteBundle) -> Result<Vec<OptionalResource>> {
    let selection = match fs::read(config.patcher_dir.join("nte-resource-selection.json")) {
        Ok(bytes) => serde_json::from_slice::<Selection>(&bytes)
            .map_err(|error| Error::Parse(format!("可选资源选择记录无效: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Selection::default(),
        Err(error) => return Err(error.into()),
    };
    let local = super::check::load_local_ledger(config)?;
    let mut seen = BTreeSet::new();
    let mut resources = Vec::new();
    for base in &bundle.manifest.base_versions {
        if base.tag.is_empty() || base.tag == bundle.manifest.tag || !seen.insert(&base.tag) {
            return Err(Error::Parse("可选资源清单的 tag 为空或重复".into()));
        }
        let mut count = 0;
        let mut present = 0;
        let mut matching_sizes = 0;
        let mut size = 0;
        for entry in entries(base) {
            let relative = crate::paths::relative_path(entry.path())?;
            count += 1;
            size += entry.size();
            if let Ok(metadata) = fs::metadata(config.install_root.join(relative)) {
                if metadata.is_file() {
                    present += 1;
                    if metadata.len() == entry.size() {
                        matching_sizes += 1;
                    }
                }
            }
        }
        let state = if present == 0 {
            ResourceState::Absent
        } else if present == count {
            ResourceState::Installed
        } else {
            ResourceState::Partial
        };
        resources.push(OptionalResource {
            tag: base.tag.clone(),
            name: display_name(&base.tag),
            version: base.version.clone(),
            size,
            state,
            needs_update: present > 0
                && (matching_sizes != count
                    || local
                        .as_ref()
                        .and_then(|manifest| {
                            manifest
                                .base_versions
                                .iter()
                                .find(|record| record.tag == base.tag)
                        })
                        .map(|record| record.version.as_str())
                        != Some(base.version.as_str())),
            selected: present > 0 || selection.tags.contains(&base.tag),
        });
    }
    resources.sort_by(|a, b| a.tag.cmp(&b.tag));
    Ok(resources)
}

/// 完整检查单个标签, 不检查基础资源或其他语种, 不修改文件和账本.
/// 应用该计划时须保留主版本, 通过 update_after_apply(..., false) 独立结算标签.
pub fn plan(
    config: &GameUpdateConfig,
    bundle: &RemoteBundle,
    tag: &str,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<super::check::UpdatePlan> {
    let mut scoped = bundle.clone();
    scoped.manifest.base_versions.retain(|base| base.tag == tag);
    if scoped.manifest.base_versions.is_empty() {
        return Err(Error::Parse(format!("未知的可选资源: {tag}")));
    }
    scoped.manifest.res.clear();
    scoped.manifest.packages.clear();
    super::check::check_bundle_cancellable(
        config,
        &scoped,
        super::check::CheckOptions {
            full: true,
            optional: OptionalSelection::All,
        },
        cancelled,
    )
}

pub fn save_selection(
    config: &GameUpdateConfig,
    bundle: &RemoteBundle,
    tags: &[String],
) -> Result<()> {
    let known: BTreeSet<_> = bundle
        .manifest
        .base_versions
        .iter()
        .map(|base| base.tag.as_str())
        .collect();
    if let Some(tag) = tags.iter().find(|tag| !known.contains(tag.as_str())) {
        return Err(Error::Parse(format!("未知的可选资源: {tag}")));
    }
    let path = config.patcher_dir.join("nte-resource-selection.json");
    let relative = path
        .strip_prefix(&config.install_root)
        .map_err(|_| Error::Integrity("可选资源选择记录超出安装目录".into()))?;
    crate::paths::ensure_no_symlink(&config.install_root, relative)?;
    fs::create_dir_all(&config.patcher_dir)?;
    let selection = Selection {
        tags: tags
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    let bytes =
        serde_json::to_vec_pretty(&selection).map_err(|error| Error::Parse(error.to_string()))?;
    let mut output = crate::paths::TemporaryFile::new(&path)?;
    use std::io::Write;
    output.file.write_all(&bytes)?;
    output.commit(&path, &|| false)
}

/// 删除标签独占的安装文件和账本记录; 调用方须持有安装锁并确认游戏已退出.
pub fn remove(config: &GameUpdateConfig, bundle: &RemoteBundle, tag: &str) -> Result<()> {
    let resources = catalog(config, bundle)?;
    if !resources.iter().any(|resource| resource.tag == tag) {
        return Err(Error::Parse(format!("未知的可选资源: {tag}")));
    }
    let ledger_path = config.patcher_dir.join("ResList.xml");
    let ledger_relative = ledger_path
        .strip_prefix(&config.install_root)
        .map_err(|_| Error::Integrity("资源记录超出安装目录".into()))?;
    crate::paths::ensure_no_symlink(&config.install_root, ledger_relative)?;
    let mut local = super::check::load_local_ledger(config)?;
    if local.is_none() && ledger_path.exists() {
        return Err(Error::Parse("资源账本损坏, 请先校验游戏资源.".into()));
    }
    let mut targets = std::collections::BTreeMap::new();
    let mut retained = BTreeSet::new();
    for manifest in std::iter::once(&bundle.manifest).chain(local.iter()) {
        let root = BaseVersion {
            res: manifest.res.clone(),
            packages: manifest.packages.clone(),
            ..Default::default()
        };
        for base in std::iter::once(&root).chain(&manifest.base_versions) {
            for entry in entries(base) {
                let relative = crate::paths::relative_path(entry.path())?;
                let key = super::check::ledger_key(entry.path());
                if base.tag == tag {
                    crate::paths::ensure_no_symlink(&config.install_root, &relative)?;
                    targets.insert(key, relative);
                } else {
                    retained.insert(key);
                }
            }
        }
    }
    if targets.keys().any(|key| retained.contains(key)) {
        return Err(Error::Integrity(
            "可选资源与其他资源共享文件, 无法删除.".into(),
        ));
    }
    let tags = resources
        .into_iter()
        .filter(|resource| resource.tag != tag && resource.selected)
        .map(|resource| resource.tag)
        .collect::<Vec<_>>();
    // 先取消显式选择, 成功删除后下一轮检查不再请求该标签.
    save_selection(config, bundle, &tags)?;
    for relative in targets.values() {
        match fs::remove_file(config.install_root.join(relative)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(manifest) = &mut local {
        manifest.base_versions.retain(|base| base.tag != tag);
        let encoded = super::pxml::encode(
            super::ledger::serialize(manifest).as_bytes(),
            &config.app_id,
        )?;
        let mut output = crate::paths::TemporaryFile::new(&ledger_path)?;
        use std::io::Write;
        output.file.write_all(&encoded)?;
        output.commit(&ledger_path, &|| false)?;
    }
    Ok(())
}

pub(crate) fn selected_tags(
    config: &GameUpdateConfig,
    bundle: &RemoteBundle,
    selection: &OptionalSelection,
) -> Result<BTreeSet<String>> {
    if matches!(selection, OptionalSelection::None) {
        return Ok(BTreeSet::new());
    }
    let resources = match catalog(config, bundle) {
        Ok(resources) => resources,
        // 选择记录或可选清单异常时继续检查基础资源, 目录接口会单独报告错误.
        Err(_) if matches!(selection, OptionalSelection::Configured) => {
            return Ok(BTreeSet::new());
        }
        Err(error) => return Err(error),
    };
    if let OptionalSelection::Tags(tags) = selection {
        if let Some(tag) = tags
            .iter()
            .find(|tag| !resources.iter().any(|resource| resource.tag == **tag))
        {
            return Err(Error::Parse(format!("未知的可选资源: {tag}")));
        }
    }
    Ok(resources
        .into_iter()
        .filter(|resource| {
            resource.state != ResourceState::Absent
                || match selection {
                    OptionalSelection::Configured => resource.selected,
                    OptionalSelection::Tags(tags) => tags.contains(&resource.tag),
                    OptionalSelection::All => true,
                    OptionalSelection::None => false,
                }
        })
        .map(|resource| resource.tag)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::manifest::{GameManifest, RemoteConfig, ResEntry};
    use crate::testing::TempDir;

    fn fixture(dir: &TempDir) -> (GameUpdateConfig, RemoteBundle) {
        let config = GameUpdateConfig {
            launcher_root: dir.join("NTELauncher"),
            install_root: dir.join(""),
            config_dir: dir.join("config"),
            patcher_dir: dir.join("patcher"),
            branch: "test".into(),
            update_branch: "test".into(),
            app_id: "1289".into(),
            cdns: Vec::new(),
            rules: Vec::new(),
            check_dirs: Vec::new(),
            max_threads: 1,
            preload_root: None,
            layout: crate::game_profile::GameId::Nte.profile().layout().unwrap(),
        };
        let manifest = GameManifest {
            version: "1.0".into(),
            tag: "baseTag".into(),
            res: vec![ResEntry {
                filename: "base.pak".into(),
                ..Default::default()
            }],
            base_versions: vec![
                BaseVersion {
                    tag: "pakchunk101".into(),
                    version: "0.101.0".into(),
                    res: vec![ResEntry {
                        filename: "voice/chinese.pak".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                BaseVersion {
                    tag: "pakchunk102".into(),
                    version: "0.102.0".into(),
                    res: vec![ResEntry {
                        filename: "voice/japanese.pak".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        fs::create_dir_all(&config.patcher_dir).unwrap();
        fs::create_dir_all(config.install_root.join("voice")).unwrap();
        for path in ["base.pak", "voice/chinese.pak", "voice/japanese.pak"] {
            fs::write(config.install_root.join(path), b"data").unwrap();
        }
        (
            config,
            RemoteBundle {
                config: RemoteConfig::default(),
                manifest,
                diff: None,
            },
        )
    }

    #[test]
    fn catalog_reports_updates_from_tag_version_and_file_sizes() {
        let dir = TempDir::new("voice-update-status");
        let (config, mut bundle) = fixture(&dir);
        bundle.manifest.base_versions[0].res[0].filesize = 4;
        let mut local = bundle.manifest.clone();
        let path = config.patcher_dir.join("ResList.xml");
        let resource = &catalog(&config, &bundle).unwrap()[0];
        assert_eq!(resource.state, ResourceState::Installed);
        assert!(resource.needs_update);
        fs::write(
            &path,
            super::super::pxml::encode(
                super::super::ledger::serialize(&local).as_bytes(),
                &config.app_id,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(!catalog(&config, &bundle).unwrap()[0].needs_update);
        local.base_versions[0].version = "0.101.old".into();
        fs::write(
            &path,
            super::super::pxml::encode(
                super::super::ledger::serialize(&local).as_bytes(),
                &config.app_id,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(catalog(&config, &bundle).unwrap()[0].needs_update);
        local.base_versions[0].version = bundle.manifest.base_versions[0].version.clone();
        fs::write(
            &path,
            super::super::pxml::encode(
                super::super::ledger::serialize(&local).as_bytes(),
                &config.app_id,
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(config.install_root.join("voice/chinese.pak"), b"short").unwrap();
        let resource = &catalog(&config, &bundle).unwrap()[0];
        assert_eq!(resource.state, ResourceState::Installed);
        assert!(resource.needs_update);
        fs::remove_file(config.install_root.join("voice/chinese.pak")).unwrap();
        let resource = &catalog(&config, &bundle).unwrap()[0];
        assert_eq!(resource.state, ResourceState::Absent);
        assert!(!resource.needs_update);
    }

    #[test]
    fn single_tag_plan_checks_only_its_files_without_writing_or_bumping_main_version() {
        let dir = TempDir::new("voice-scoped-plan");
        let (config, mut bundle) = fixture(&dir);
        bundle.config.res_version = "1.0".into();
        bundle.manifest.base_versions[0].res[0].filesize = 4;
        bundle.manifest.base_versions[0].res[0].md5 = crate::update::md5_bytes(b"data");
        bundle.manifest.base_versions[0]
            .packages
            .push(super::super::manifest::Package {
                paks: vec![super::super::manifest::Pak {
                    md5: crate::update::md5_bytes(b"signature"),
                    filesize: 9,
                    entries: vec![super::super::manifest::PakEntry {
                        name: "voice/chinese.sig".into(),
                        md5: crate::update::md5_bytes(b"signature"),
                        size: 9,
                        ..Default::default()
                    }],
                }],
            });
        fs::remove_file(config.install_root.join("base.pak")).unwrap();
        fs::write(config.install_root.join("voice/japanese.pak"), b"bad").unwrap();
        fs::write(config.install_root.join("voice/chinese.pak"), b"evil").unwrap();
        let plan = plan(&config, &bundle, "pakchunk101", &|| false).unwrap();
        assert_eq!(plan.remote_version, "1.0");
        assert_eq!(plan.remote_tag, "baseTag");
        assert_eq!(plan.checked, 2);
        assert_eq!(plan.optional_versions.len(), 1);
        assert_eq!(plan.optional_versions[0].tag, "pakchunk101");
        assert_eq!(plan.optional_versions[0].version, "0.101.0");
        assert_eq!(plan.items.len(), 2);
        assert!(plan
            .items
            .iter()
            .all(|item| item.path.starts_with("voice/chinese.")));
        assert!(plan
            .items
            .iter()
            .any(|item| item.path == "voice/chinese.pak"
                && item.reason == super::super::check::UpdateReason::Md5));
        assert!(plan.unverified.is_empty());
        assert!(!config.patcher_dir.join("ResList.xml").exists());
        assert!(!config
            .patcher_dir
            .join("nte-resource-selection.json")
            .exists());
        assert_eq!(
            fs::read(config.install_root.join("voice/chinese.pak")).unwrap(),
            b"evil"
        );
        assert!(super::plan(&config, &bundle, "unknown", &|| false).is_err());
        assert!(matches!(
            super::plan(&config, &bundle, "pakchunk101", &|| true),
            Err(Error::Cancelled)
        ));
        fs::write(config.install_root.join("voice/chinese.pak"), b"data").unwrap();
        fs::write(config.install_root.join("voice/chinese.sig"), b"signature").unwrap();
        let complete = super::plan(&config, &bundle, "pakchunk101", &|| false).unwrap();
        assert!(complete.items.is_empty());
        assert_eq!(complete.verified.len(), 2);
        let local = GameManifest {
            version: "0.9".into(),
            tag: "baseTag".into(),
            res: bundle.manifest.res.clone(),
            ..Default::default()
        };
        fs::write(
            config.patcher_dir.join("ResList.xml"),
            super::super::pxml::encode(
                super::super::ledger::serialize(&local).as_bytes(),
                &config.app_id,
            )
            .unwrap(),
        )
        .unwrap();
        let verified = complete
            .verified
            .iter()
            .map(|entry| entry.path().to_owned())
            .collect::<Vec<_>>();
        super::super::ledger::update_after_apply(&config, &complete, &verified, false).unwrap();
        let local = super::super::check::load_local_ledger(&config)
            .unwrap()
            .unwrap();
        assert_eq!(local.version, "0.9");
        assert_eq!(local.base_versions[0].version, "0.101.0");
    }

    #[test]
    fn removes_current_and_old_voice_files_and_preserves_other_resources() {
        let dir = TempDir::new("remove-voice");
        let (config, bundle) = fixture(&dir);
        let mut local = bundle.manifest.clone();
        local.base_versions[0].res[0].filename = "voice/old-chinese.pak".into();
        fs::write(config.install_root.join("voice/old-chinese.pak"), b"old").unwrap();
        fs::write(
            config.patcher_dir.join("ResList.xml"),
            super::super::pxml::encode(
                super::super::ledger::serialize(&local).as_bytes(),
                &config.app_id,
            )
            .unwrap(),
        )
        .unwrap();
        save_selection(
            &config,
            &bundle,
            &["pakchunk101".into(), "pakchunk102".into()],
        )
        .unwrap();
        remove(&config, &bundle, "pakchunk101").unwrap();
        assert!(!config.install_root.join("voice/chinese.pak").exists());
        assert!(!config.install_root.join("voice/old-chinese.pak").exists());
        assert!(config.install_root.join("base.pak").is_file());
        assert!(config.install_root.join("voice/japanese.pak").is_file());
        let local = super::super::check::load_local_ledger(&config)
            .unwrap()
            .unwrap();
        assert_eq!(local.version, "1.0");
        assert_eq!(local.base_versions.len(), 1);
        assert_eq!(local.base_versions[0].tag, "pakchunk102");
        let resources = catalog(&config, &bundle).unwrap();
        assert_eq!(resources[0].state, ResourceState::Absent);
        assert!(!resources[0].selected);
        assert!(resources[1].selected);
        assert!(
            !selected_tags(&config, &bundle, &OptionalSelection::Configured)
                .unwrap()
                .contains("pakchunk101")
        );
        remove(&config, &bundle, "pakchunk101").unwrap();
        save_selection(&config, &bundle, &["pakchunk101".into()]).unwrap();
        assert!(
            selected_tags(&config, &bundle, &OptionalSelection::Configured)
                .unwrap()
                .contains("pakchunk101")
        );
    }

    #[test]
    fn rejects_unknown_and_shared_files_before_deleting_anything() {
        let dir = TempDir::new("remove-shared-voice");
        let (config, mut bundle) = fixture(&dir);
        assert!(remove(&config, &bundle, "unknown").is_err());
        bundle.manifest.base_versions[0].res.push(ResEntry {
            filename: "BASE.PAK".into(),
            ..Default::default()
        });
        assert!(remove(&config, &bundle, "pakchunk101").is_err());
        assert!(config.install_root.join("voice/chinese.pak").is_file());
        assert!(config.install_root.join("base.pak").is_file());
    }

    #[test]
    fn rejects_symlinked_voice_and_corrupt_ledger_without_deletion() {
        let dir = TempDir::new("remove-invalid-voice");
        let (config, bundle) = fixture(&dir);
        fs::remove_file(config.install_root.join("voice/chinese.pak")).unwrap();
        std::os::unix::fs::symlink(
            config.install_root.join("base.pak"),
            config.install_root.join("voice/chinese.pak"),
        )
        .unwrap();
        assert!(remove(&config, &bundle, "pakchunk101").is_err());
        assert!(config.install_root.join("base.pak").is_file());
        fs::remove_file(config.install_root.join("voice/chinese.pak")).unwrap();
        fs::write(config.install_root.join("voice/chinese.pak"), b"data").unwrap();
        fs::write(config.patcher_dir.join("ResList.xml"), b"invalid").unwrap();
        assert!(remove(&config, &bundle, "pakchunk101").is_err());
        assert!(config.install_root.join("voice/chinese.pak").is_file());
    }

    #[test]
    fn names_voice_pak_chunks_and_keeps_other_tags() {
        assert_eq!(display_name("pakchunk101"), "中文语音");
        assert_eq!(display_name("pakchunk102"), "日语语音");
        assert_eq!(display_name("pakchunk103"), "英语语音");
        assert_eq!(display_name("pakchunk104"), "韩语语音");
        assert_eq!(display_name("pakchunk0"), "pakchunk0");
    }
}
