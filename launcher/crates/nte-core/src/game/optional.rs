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
    pub selected: bool,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    tags: Vec<String>,
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
    let mut seen = BTreeSet::new();
    let mut resources = Vec::new();
    for base in &bundle.manifest.base_versions {
        if base.tag.is_empty() || base.tag == bundle.manifest.tag || !seen.insert(&base.tag) {
            return Err(Error::Parse("可选资源清单的 tag 为空或重复".into()));
        }
        let mut count = 0;
        let mut present = 0;
        let mut size = 0;
        for entry in entries(base) {
            let relative = crate::paths::relative_path(entry.path())?;
            count += 1;
            size += entry.size();
            if config.install_root.join(relative).is_file() {
                present += 1;
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
            // 未取得可靠名称映射的 tag 保留官方标识.
            name: base.tag.clone(),
            version: base.version.clone(),
            size,
            state,
            selected: present > 0 || selection.tags.contains(&base.tag),
        });
    }
    resources.sort_by(|a, b| a.tag.cmp(&b.tag));
    Ok(resources)
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
