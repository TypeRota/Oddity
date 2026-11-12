//! 更新域: 官方启动器自更新.
//!
//! 命令组声明 `check_update`; 清单, 版本, 文件安装与规划都在
//! [`launcher`] 里, 区服与布局常量在 [`region`]. 宿主只提供只读设置快照,
//! 活动状态与官方启动器根目录, 返回值与搬入前的宿主实现逐字段一致.

pub mod auto;
pub mod check;
pub mod commands;
pub mod config;
pub mod download;
pub mod launcher;
pub mod ledger;
pub mod manifest;
pub mod optional;
pub mod paths;
pub mod plan;
pub mod preload;
pub mod pxml;
pub mod region;
pub mod state;

#[cfg(test)]
pub mod testing;

pub use check::{CheckOptions, UpdateItem, UpdateKind, UpdatePlan};
pub use config::GameUpdateConfig;
pub use download::{apply_plan, ApplyReport, ApplyResult};
pub use manifest::{DiffManifest, GameManifest, LocalManifest, RemoteConfig};

pub use commands::{cancel_download, download_status, set_auto_waiting_reason, start_download};
pub use state::{
    bind_host, blocks_changes, busy, is_busy, settings_gate, snapshot, DownloadStatus,
    ResourceCatalog, ResourceTask,
};

use std::path::{Path, PathBuf};
use std::time::Instant;

use oddity_runtime::{HostContext, InstallQuery, SettingsSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use launcher::{version_ini, LauncherConfig, Manifest};

/// 更新域注册到宿主命令组里的命令 id.
pub(crate) const COMMANDS: &[&str] = &["check_update"];

/// 返回给渲染层的自检结果; 字段与 UI 手抄的 TS 类型逐项一致.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResult {
    installed: bool,
    distribution: String,
    install_root: String,
    remote_version: String,
    local_version: String,
    product_version: String,
    files: usize,
    ok: usize,
    missing: usize,
    changed: usize,
    bytes: u64,
    duration_ms: u128,
    missing_preview: Vec<String>,
    changed_preview: Vec<String>,
}

impl CheckResult {
    fn not_installed(distribution: String, install_root: String) -> Self {
        Self {
            installed: false,
            distribution,
            install_root,
            remote_version: String::new(),
            local_version: String::new(),
            product_version: String::new(),
            files: 0,
            ok: 0,
            missing: 0,
            changed: 0,
            bytes: 0,
            duration_ms: 0,
            missing_preview: Vec::new(),
            changed_preview: Vec::new(),
        }
    }
}

/// 分发更新域命令; 参数与结果都是宿主载荷.
pub(crate) async fn command(
    host: &dyn HostContext,
    command: &str,
    args: Value,
) -> std::result::Result<Value, Value> {
    match command {
        "check_update" => check_update(host, args).await,
        _ => Err(json!("无法识别的更新命令")),
    }
}

async fn check_update(host: &dyn HostContext, args: Value) -> std::result::Result<Value, Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        full: bool,
    }
    let Args { full } = serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;

    let activity = host.activity().map_err(|error| json!(error))?;
    if activity.migration_busy || activity.migration_repair_pending {
        return Err(json!("安装目录迁移正在进行"));
    }
    let settings = host.settings().map_err(|error| json!(error))?;
    // 非本包的游戏没有安装布局; 宿主此前也没有对应安装域, 结果保持未安装.
    if settings.game != crate::PACKAGE_ID {
        return to_payload(CheckResult::not_installed(
            settings.distribution,
            String::new(),
        ));
    }
    let launcher_root = host.official_launcher_root();
    let install_root = effective_install_root(&settings, launcher_root.as_deref());
    let Some(launcher_root) = launcher_root else {
        return to_payload(CheckResult::not_installed(
            settings.distribution,
            install_root.display().to_string(),
        ));
    };
    let distribution = settings.distribution.clone();
    let result = tokio::task::spawn_blocking(move || {
        check_impl(&launcher_root, &install_root, &distribution, full)
    })
    .await
    .map_err(|error| json!(format!("校验任务失败：{error}")))?
    .map_err(|error| json!(error))?;
    to_payload(result)
}

fn to_payload(result: CheckResult) -> std::result::Result<Value, Value> {
    serde_json::to_value(result).map_err(|error| json!(error.to_string()))
}

/// 当前设置下的有效安装根目录.
///
/// 与宿主安装域同源: 配置了目录时以它为准, 否则取已探测启动器根目录的
/// 父目录, 两者都没有时退回该地区的默认安装位置.
fn effective_install_root(settings: &SettingsSnapshot, launcher_root: Option<&Path>) -> PathBuf {
    crate::install::effective_install_root_for(&install_query(settings), launcher_root)
}

fn install_query(settings: &SettingsSnapshot) -> InstallQuery {
    InstallQuery {
        distribution: settings.distribution.clone(),
        configured_install_root: settings.install_root.clone().map(PathBuf::from),
    }
}

fn check_impl(
    launcher_root: &Path,
    install_root: &Path,
    distribution: &str,
    full: bool,
) -> Result<CheckResult, String> {
    let cfg = LauncherConfig::load(launcher_root)
        .map_err(|error| format!("读取启动器配置失败：{error}"))?;
    let started = Instant::now();
    let remote = version_ini::fetch(&cfg.version_url)
        .map_err(|error| format!("拉取 Version.ini 失败：{error}"))?;
    let file_list_url = remote
        .file_list_url
        .clone()
        .ok_or_else(|| "远端 Version.ini 缺少 FileListURL".to_string())?;
    let manifest = Manifest::from_url(&file_list_url)
        .map_err(|error| format!("拉取 AllFiles.xml 失败：{error}"))?;
    let report = launcher::check(&cfg.root, &manifest, !full)
        .map_err(|error| format!("本地校验失败：{error}"))?;
    let duration_ms = started.elapsed().as_millis();
    let missing_preview = report
        .missing
        .iter()
        .take(12)
        .map(|file| file.path.clone())
        .collect();
    let changed_preview = report
        .changed
        .iter()
        .take(12)
        .map(|file| file.path.clone())
        .collect();
    Ok(CheckResult {
        installed: true,
        distribution: distribution.to_string(),
        install_root: install_root.display().to_string(),
        remote_version: remote.version,
        local_version: cfg.version,
        product_version: manifest.product_version.unwrap_or_default(),
        files: manifest.files.len(),
        ok: report.ok,
        missing: report.missing.len(),
        changed: report.changed.len(),
        bytes: report.total_missing_bytes(),
        duration_ms,
        missing_preview,
        changed_preview,
    })
}

#[cfg(test)]
mod tests;
