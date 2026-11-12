//! Bootstrap 读模型: 启动器/游戏安装, 运行时与 prefix 的当前状态.
//!
//! 字段全部来自设置与包的安装域, 宿主不保存游戏目录规则.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::service::Service;
use crate::settings::Settings;
use crate::{environment_commands, paths};
use oddity_runtime::{ActivitySnapshot, Availability, Registry};

/// 已校验的 Proton 安装; `PROTONPATH` 需要目录而不是 `proton` 可执行文件.
#[derive(Debug, Clone)]
pub(crate) struct ProtonInstall {
    pub(crate) name: String,
    pub(crate) root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckItem {
    pub(crate) label: String,
    pub(crate) ok: bool,
    pub(crate) detail: String,
}

/// 能力在当前上下文下的状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CapabilityState {
    Available,
    Unsupported,
    Unavailable,
}

/// bootstrap 投影的一条能力事实.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilityFact {
    pub(crate) id: &'static str,
    pub(crate) revision: u16,
    pub(crate) state: CapabilityState,
    /// 不可用时的稳定原因码; 其它状态为 None.
    pub(crate) code: Option<&'static str>,
    /// 不可用时的包内文案; 其它状态为 None.
    pub(crate) message: Option<&'static str>,
}

/// 当前游戏声明的一条渠道及其选择状态.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelView {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) default: bool,
    pub(crate) active: bool,
    pub(crate) install_root_key: &'static str,
}

/// 一个设置字段的声明与当前值.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsFieldView {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) description: &'static str,
    pub(crate) order: u16,
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) default: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) options: Option<&'static [&'static str]>,
    pub(crate) value: Value,
}

/// 一个 owner 的设置 section 及其字段.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsSchemaView {
    pub(crate) owner: &'static str,
    pub(crate) version: u32,
    pub(crate) fields: Vec<SettingsFieldView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub(crate) ok: bool,
    pub(crate) error: Option<String>,
    pub(crate) install_root: String,
    pub(crate) launcher_root: String,
    pub(crate) prefix: String,
    pub(crate) prefix_exists: bool,
    pub(crate) proton_name: String,
    pub(crate) proton_path: String,
    pub(crate) umu_available: bool,
    pub(crate) launcher_version: String,
    pub(crate) launcher_build: String,
    pub(crate) game_version: String,
    pub(crate) game_exe: String,
    pub(crate) game_exists: bool,
    pub(crate) sdk_path: String,
    pub(crate) checks: Vec<CheckItem>,
    /// 当前游戏提供的能力及其上下文可用性.
    pub(crate) capabilities: Vec<CapabilityFact>,
    /// 当前游戏声明的发行渠道.
    pub(crate) channels: Vec<ChannelView>,
    /// 已装配包声明的设置 section 与当前值.
    pub(crate) settings_schemas: Vec<SettingsSchemaView>,
    pub(crate) runtime_installations: Vec<environment_commands::Installation>,
    pub(crate) settings: Settings,
    pub(crate) integrity: crate::integrity::Status,
    pub(crate) app_version: String,
    pub(crate) os: String,
}

pub(crate) fn proton_install(path: &Path) -> Option<ProtonInstall> {
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    // umu-launcher 启动 Proton 前会校验这两个文件.
    if !root.join("proton").is_file() || !root.join("toolmanifest.vdf").is_file() {
        return None;
    }
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "custom Proton".to_string());
    Some(ProtonInstall { name, root })
}

pub(crate) fn detect_proton(settings: &Settings) -> Option<ProtonInstall> {
    environment_commands::proton_path(settings).and_then(proton_install)
}

pub(crate) fn bootstrap_impl(registry: &Registry, settings: &Settings) -> Bootstrap {
    bootstrap_impl_with_activity(registry, settings, &ActivitySnapshot::default())
}

fn bootstrap_impl_with_activity(
    registry: &Registry,
    settings: &Settings,
    activity: &ActivitySnapshot,
) -> Bootstrap {
    let mut error: Option<String> = None;
    let game = &settings.active_game;
    let launcher_root = crate::install::launcher_root(registry, settings);
    let install_root =
        crate::install::effective_install_root(registry, settings, launcher_root.as_deref());
    let prefix = match environment_commands::prefix(registry, settings) {
        Ok(prefix) => prefix,
        Err(message) => {
            error = Some(message);
            PathBuf::new()
        }
    };
    let prefix_exists = !prefix.as_os_str().is_empty() && prefix.join("drive_c").is_dir();

    // 启动器版本知识属于游戏包, 宿主只消费安装域给出的结果.
    let mut launcher_version = String::new();
    let mut launcher_build = String::new();
    if let Some(root) = &launcher_root {
        match crate::install::launcher_info(registry, game, root) {
            Some(info) => {
                launcher_version = info.version;
                launcher_build = info.build;
            }
            None => error = Some(format!("读取启动器配置失败：{}", root.display())),
        }
    } else if crate::install::configured_install_root(registry, settings).is_some() {
        error = Some(format!(
            "所选安装目录尚未安装启动器文件: {}. 可在游戏更新页开始安装.",
            install_root.display()
        ));
    } else {
        error = Some("尚未找到所选游戏地区的游戏安装. 可在游戏更新页开始安装.".to_string());
    }

    let game_version = launcher_root
        .as_ref()
        .map(|p| crate::install::game_version(registry, game, p))
        .unwrap_or_default();
    let game_exe = launcher_root
        .as_ref()
        .and_then(|p| crate::install::game_exe(registry, game, p))
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let game_exists = !game_exe.is_empty() && Path::new(&game_exe).is_file();
    let sdk_path = launcher_root
        .as_ref()
        .and_then(|p| crate::install::sdk_dir(registry, game, p))
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if let Err(message) = environment_commands::verify_proton(settings) {
        error.get_or_insert(message);
    }
    let proton = detect_proton(settings);
    let umu = crate::runtime_setup::umu(&paths::data_dir());
    let official_exe = launcher_root
        .as_ref()
        .and_then(|root| crate::install::official_client_exe(registry, game, root));
    let official_ready = official_exe.as_ref().is_some_and(|path| path.is_file());

    let mut checks = Vec::new();
    checks.push(CheckItem {
        label: "启动器配置".into(),
        ok: launcher_root.is_some(),
        detail: launcher_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "未找到".into()),
    });
    checks.push(CheckItem {
        label: "游戏可执行文件".into(),
        ok: game_exists,
        detail: if game_exists {
            game_exe.clone()
        } else {
            crate::install::game_exe_hint(registry, game)
        },
    });
    checks.push(CheckItem {
        label: "umu-launcher".into(),
        ok: umu.is_some(),
        detail: umu
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "PATH 中未找到 umu-run".into()),
    });
    checks.push(CheckItem {
        label: "Proton".into(),
        ok: proton.is_some(),
        detail: proton
            .as_ref()
            .map(|install| install.name.clone())
            .unwrap_or_else(|| "未找到 Proton".into()),
    });
    checks.push(CheckItem {
        label: "Wine prefix".into(),
        ok: prefix_exists,
        detail: prefix.display().to_string(),
    });
    checks.push(CheckItem {
        label: "官方启动器".into(),
        ok: official_ready,
        detail: official_exe
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未找到".into()),
    });

    Bootstrap {
        ok: error.is_none() && launcher_root.is_some(),
        error,
        install_root: install_root.display().to_string(),
        launcher_root: launcher_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        prefix: prefix.display().to_string(),
        prefix_exists,
        proton_name: proton
            .as_ref()
            .map(|install| install.name.clone())
            .unwrap_or_default(),
        proton_path: proton
            .as_ref()
            .map(|install| install.root.display().to_string())
            .unwrap_or_default(),
        umu_available: umu.is_some(),
        launcher_version,
        launcher_build,
        game_version,
        game_exe,
        game_exists,
        sdk_path,
        checks,
        capabilities: capability_facts(registry, settings, activity),
        channels: channel_facts(registry, settings),
        settings_schemas: settings_schema_facts(registry, settings),
        runtime_installations: environment_commands::installations(registry, settings),
        settings: settings.clone(),
        integrity: crate::integrity::Status::checking(env!("CARGO_PKG_VERSION"), 0),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: format!(
            "{} / {} / {}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
        ),
    }
}

/// 当前游戏声明的渠道; 没有声明的游戏返回空表.
fn channel_facts(registry: &Registry, settings: &Settings) -> Vec<ChannelView> {
    let active = crate::channel::effective_id(
        registry,
        settings.active_game.as_str(),
        settings.current().distribution.as_ref(),
    );
    crate::channel::channels(registry, settings.active_game.as_str())
        .iter()
        .map(|channel| ChannelView {
            id: channel.id,
            label: channel.label,
            default: channel.is_default,
            active: channel.id == active,
            install_root_key: channel.install_root_key,
        })
        .collect()
}

/// 已装配包声明的设置 section; 值缺失时使用声明里的默认值.
fn settings_schema_facts(registry: &Registry, settings: &Settings) -> Vec<SettingsSchemaView> {
    let schemas = crate::settings::SettingsSchemas::from_registry(registry);
    schemas
        .schemas()
        .iter()
        .map(|schema| {
            let values = crate::settings::section_values(settings, schema)
                .unwrap_or_else(|_| schema.defaults());
            let fields = schema
                .ordered_fields()
                .into_iter()
                .map(|field| {
                    let default = field.default_value();
                    let value = values
                        .get(field.key)
                        .cloned()
                        .unwrap_or_else(|| default.clone());
                    let mut view = SettingsFieldView {
                        key: field.key,
                        label: field.label,
                        description: field.description,
                        order: field.order,
                        kind: field.kind.type_name(),
                        default,
                        min: None,
                        max: None,
                        max_chars: None,
                        options: None,
                        value,
                    };
                    match field.kind {
                        oddity_runtime::SettingsFieldKind::Integer { min, max, .. } => {
                            view.min = Some(min);
                            view.max = Some(max);
                        }
                        oddity_runtime::SettingsFieldKind::Text { max_chars, .. } => {
                            view.max_chars = Some(max_chars);
                        }
                        oddity_runtime::SettingsFieldKind::Select { options, .. } => {
                            view.options = Some(options);
                        }
                        oddity_runtime::SettingsFieldKind::Boolean { .. } => {}
                    }
                    view
                })
                .collect();
            SettingsSchemaView {
                owner: schema.owner,
                version: schema.version,
                fields,
            }
        })
        .collect()
}

/// 当前游戏的能力 sheet 按上下文求值; 没有能力声明的游戏返回空表.
fn capability_facts(
    registry: &Registry,
    settings: &Settings,
    activity: &ActivitySnapshot,
) -> Vec<CapabilityFact> {
    let Some(capabilities) = registry.capabilities(settings.active_game.as_str()) else {
        return Vec::new();
    };
    let context = crate::host::build_capability_context(registry, settings, activity);
    capabilities
        .iter()
        .map(|capability| {
            let descriptor = capability.descriptor;
            let (state, code, message) = match capability.availability(&context) {
                Availability::Available => (CapabilityState::Available, None, None),
                Availability::Unsupported => (CapabilityState::Unsupported, None, None),
                Availability::Unavailable { code, message } => {
                    (CapabilityState::Unavailable, Some(code), Some(message))
                }
            };
            CapabilityFact {
                id: descriptor.id.0,
                revision: descriptor.revision.0,
                state,
                code,
                message,
            }
        })
        .collect()
}

pub(crate) fn bootstrap(app: &Service) -> Bootstrap {
    let settings = app
        .state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    let activity = crate::host::activity_snapshot(app).unwrap_or_default();
    let mut bootstrap = bootstrap_impl_with_activity(&app.state.packages, &settings, &activity);
    bootstrap.integrity = app.integrity.status();
    bootstrap
}
