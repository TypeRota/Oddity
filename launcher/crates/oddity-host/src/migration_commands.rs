//! 安装目录迁移: 预检、复制、校验、重绑定与清理.
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{self, ChannelId};
use crate::environment::RuntimeEnvironment;
use crate::identity::GameId;
use crate::settings::Settings;
use oddity_platform::{fs_tree, wine};
use oddity_runtime::{EventSink, Registry};

use crate::service::ServiceHandle;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationEnvironment {
    pub id: String,
    pub name: String,
    pub prefix: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub distribution: ChannelId,
    /// move: 同文件系统改名; copy: 跨文件系统复制; repair: 只重绑定.
    pub mode: String,
    pub source: String,
    pub target: String,
    pub same_filesystem: bool,
    pub files: u64,
    pub bytes: u64,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub environments: Vec<MigrationEnvironment>,
    /// 自定义预下载目录在新安装根目录下的路径; None 表示无需改写.
    pub preload_root: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatus {
    pub phase: String,
    pub distribution: Option<ChannelId>,
    pub mode: String,
    pub source: String,
    pub target: String,
    pub files: u64,
    pub total_files: u64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub current: String,
    pub error: String,
    pub cleanup_pending: bool,
    pub cleanup_source: String,
    pub can_cancel: bool,
}

impl MigrationStatus {
    fn idle() -> Self {
        Self {
            phase: "idle".into(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct MigrationManager {
    pub status: MigrationStatus,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for MigrationManager {
    fn default() -> Self {
        Self {
            status: MigrationStatus::idle(),
            cancel: None,
        }
    }
}

impl MigrationManager {
    pub fn blocking(&self) -> bool {
        matches!(
            self.status.phase.as_str(),
            "planning"
                | "moving"
                | "copying"
                | "verifying"
                | "committing"
                | "rebinding"
                | "cleanup"
                | "repair"
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    id: String,
    phase: String,
    source: String,
    target: String,
    staging: Option<String>,
    distribution: ChannelId,
    mode: String,
    next_settings: Settings,
    created_at: u64,
    #[serde(default)]
    source_dev: u64,
    #[serde(default)]
    source_ino: u64,
}

fn migrations_dir() -> PathBuf {
    crate::paths::data_dir().join("migrations")
}

fn journal_path() -> PathBuf {
    migrations_dir().join("current.json")
}

fn write_journal(journal: &Journal) -> Result<(), String> {
    let dir = migrations_dir();
    fs::create_dir_all(&dir).map_err(|error| format!("创建迁移目录失败: {error}"))?;
    let text = serde_json::to_vec_pretty(journal).map_err(|error| error.to_string())?;
    let temp = dir.join("current.json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|error| format!("写入迁移记录失败: {error}"))?;
    file.write_all(&text)
        .map_err(|error| format!("写入迁移记录失败: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("同步迁移记录失败: {error}"))?;
    fs::rename(&temp, journal_path()).map_err(|error| format!("提交迁移记录失败: {error}"))
}

fn read_journal() -> Option<Journal> {
    let bytes = fs::read(journal_path()).ok()?;
    let mut value: Value = serde_json::from_slice(&bytes).ok()?;
    // 旧日志内嵌的是旧形状设置; 与设置文件走同一条迁移, 避免崩溃恢复时丢安装根.
    if let Some(settings) = value.get_mut("nextSettings") {
        crate::settings::migrate_value(settings);
    }
    serde_json::from_value(value).ok()
}

fn remove_journal() {
    let _ = fs::remove_file(journal_path());
}

fn remove_journal_checked() -> Result<(), String> {
    match fs::remove_file(journal_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除迁移记录失败: {error}")),
    }
}

fn status(journal: &Journal, phase: &str) -> MigrationStatus {
    MigrationStatus {
        phase: phase.into(),
        distribution: Some(journal.distribution.clone()),
        mode: journal.mode.clone(),
        source: journal.source.clone(),
        target: journal.target.clone(),
        ..MigrationStatus::default()
    }
}

fn emit(app: &ServiceHandle, status: &MigrationStatus) {
    let _ = app.emit("migration-state", status.clone());
}

fn set_status(app: &ServiceHandle, next: MigrationStatus, cancel: Option<Arc<AtomicBool>>) {
    if let Ok(mut manager) = app.migration.lock() {
        manager.status = next.clone();
        if cancel.is_some() {
            manager.cancel = cancel;
        }
    }
    emit(app, &next);
}

pub fn current(app: &ServiceHandle) -> MigrationStatus {
    if let Ok(manager) = app.migration.lock() {
        if manager.status.phase != "idle" {
            return manager.status.clone();
        }
    }
    if let Some(journal) = read_journal() {
        if journal.phase == "done" {
            let mut status = status(&journal, "done");
            status.cleanup_pending = Path::new(&journal.source).exists();
            status.cleanup_source = if status.cleanup_pending {
                journal.source.clone()
            } else {
                String::new()
            };
            return status;
        }
        if journal.phase == "committed" {
            return status(&journal, "repair");
        }
    }
    MigrationStatus::idle()
}

fn expand(value: &str) -> PathBuf {
    crate::paths::expand_path(value.trim())
}

fn canonical_target(value: &str) -> Result<PathBuf, String> {
    let path = expand(value);
    if !path.is_absolute() {
        return Err("请选择绝对路径作为目标目录".into());
    }
    let name = path.file_name().ok_or_else(|| "目标目录无效".to_string())?;
    if name == "." || name == ".." {
        return Err("目标目录无效".into());
    }
    let parent = path.parent().ok_or_else(|| "目标目录无效".to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("目标目录不可用: {error}"))?;
    Ok(parent.join(name))
}

fn configured_root(registry: &Registry, settings: &Settings) -> Option<PathBuf> {
    crate::install::configured_install_root(registry, settings).map(expand)
}

fn target_settings(settings: &Settings, distribution: &ChannelId) -> Settings {
    let mut copy = settings.clone();
    copy.current_mut().distribution = Some(distribution.clone());
    copy
}

fn source_root(
    registry: &Registry,
    settings: &Settings,
    distribution: &ChannelId,
) -> Result<(PathBuf, bool), String> {
    let target = target_settings(settings, distribution);
    if let Some(launcher) = crate::install::launcher_root(registry, &target) {
        let root = crate::install::effective_install_root(registry, &target, Some(&launcher));
        return Ok((root, false));
    }
    let Some(configured) = configured_root(registry, &target) else {
        let label = channel::label(registry, settings.active_game.as_str(), distribution);
        return Err(format!("尚未找到{label}安装"));
    };
    Ok((configured, true))
}

fn target_install_root(
    registry: &Registry,
    game: &GameId,
    target: &Path,
    distribution: &ChannelId,
) -> Option<PathBuf> {
    let launcher = crate::install::normalize_launcher_root(registry, game, target, distribution)?;
    Some(launcher.parent().unwrap_or(target).to_path_buf())
}

/// 目标位置已有其他内容时, 使用区服默认目录名创建子目录.
fn resolve_move_target(
    registry: &Registry,
    game: &GameId,
    selected: &Path,
    distribution: &ChannelId,
) -> Result<PathBuf, String> {
    if channel::find(registry, game.as_str(), distribution).is_none() {
        return Err("当前游戏尚不支持该渠道".to_string());
    }
    let metadata = match fs::symlink_metadata(selected) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(selected.to_path_buf())
        }
        Err(error) => return Err(format!("读取目标目录失败: {error}")),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(selected.to_path_buf());
    }
    if fs::read_dir(selected)
        .map_err(|error| format!("读取目标目录失败: {error}"))?
        .next()
        .is_none()
    {
        return Ok(selected.to_path_buf());
    }
    let installed = target_install_root(registry, game, selected, distribution).is_some()
        || channel::channels(registry, game.as_str())
            .iter()
            .any(|channel| {
                crate::install::launcher_dir_for(registry, game, channel.id)
                    .is_some_and(|dir| selected.join(dir).join("Config/Config.ini").is_file())
            });
    if installed {
        return Err("目标目录已有游戏安装".into());
    }
    let name = crate::install::default_install_name(registry, game, distribution.as_str())
        .ok_or_else(|| "当前游戏尚不支持该渠道".to_string())?;
    if selected
        .file_name()
        .is_some_and(|value| value == std::ffi::OsStr::new(&name))
    {
        return Err("目标目录不是空目录".into());
    }
    Ok(selected.join(name))
}

fn ensure_target_free(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err("目标目录不能是符号链接".into()),
        Ok(meta) if meta.is_file() => Err("目标路径是文件".into()),
        Ok(meta) if meta.is_dir() => {
            let mut entries = fs::read_dir(target).map_err(|error| error.to_string())?;
            if entries.next().is_some() {
                return Err("目标目录不是空目录".into());
            }
            Ok(())
        }
        Ok(_) => Err("目标路径类型不受支持".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("读取目标目录失败: {error}")),
    }
}

fn ensure_target_not_forbidden(
    target: &Path,
    source: &Path,
    settings: &Settings,
    registry: &Registry,
    game: &GameId,
    distribution: &ChannelId,
) -> Result<(), String> {
    if target.starts_with(source) || source.starts_with(target) {
        return Err("目标目录不能位于源目录内, 也不能包含源目录".into());
    }
    let data = crate::paths::data_dir();
    let config = crate::paths::config_dir();
    for forbidden in [data.as_path(), config.as_path()] {
        if target.starts_with(forbidden) {
            return Err("目标目录不能放在 Oddity 数据目录内".into());
        }
    }
    for environment in &settings.runtime_environments {
        if let Ok(prefix) = environment.prefix(&crate::paths::data_dir()) {
            if target.starts_with(&prefix) || prefix.starts_with(target) {
                return Err("目标目录不能与运行环境 prefix 重叠".into());
            }
        }
    }
    let current_key = channel::find(registry, game.as_str(), distribution)
        .map(|channel| channel.install_root_key);
    for other in channel::channels(registry, game.as_str()) {
        if other.id == distribution.as_str()
            || current_key.is_some_and(|key| key == other.install_root_key)
        {
            continue;
        }
        if let Some(dir) = crate::install::launcher_dir_for(registry, game, other.id) {
            if target.join(dir).join("Config/Config.ini").is_file() {
                return Err("目标目录已有另一渠道的游戏".into());
            }
        }
    }
    Ok(())
}

fn matched_environments<'a>(settings: &'a Settings, source: &Path) -> Vec<&'a RuntimeEnvironment> {
    settings
        .runtime_environments
        .iter()
        .filter(|environment| {
            environment.game == settings.active_game
                && fs_tree::same_path(&environment.install_root, source)
        })
        .collect()
}

fn preload_update(settings: &Settings, source: &Path, target: &Path) -> Option<String> {
    let raw = settings.current().preload_root.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let path = expand(raw);
    if !path.is_absolute() {
        return None;
    }
    path.strip_prefix(source)
        .ok()
        .map(|relative| target.join(relative).display().to_string())
}

fn ensure_writable(path: &Path) -> Result<(), String> {
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| "路径包含空字节".to_string())?;
    if unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) } != 0 {
        return Err(format!("目标目录不可写: {}", path.to_string_lossy()));
    }
    Ok(())
}

fn probe_target(parent: &Path) -> Result<(), String> {
    let probe = parent.join(format!(
        ".oddity-migration-probe-{}-{}",
        std::process::id(),
        now()
    ));
    fs::create_dir(&probe).map_err(|error| format!("目标文件系统不可写: {error}"))?;
    let result = (|| -> Result<(), String> {
        let file = probe.join("file");
        fs::write(&file, b"probe").map_err(|error| format!("目标文件系统不可写: {error}"))?;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640))
            .map_err(|error| format!("目标文件系统不支持权限位: {error}"))?;
        symlink("file", probe.join("link"))
            .map_err(|error| format!("目标文件系统不支持符号链接: {error}"))?;
        let path = CString::new(file.as_os_str().as_bytes())
            .map_err(|_| "探测路径包含空字节".to_string())?;
        let name = CString::new("user.oddity.probe").expect("探测属性名合法");
        let value = b"1";
        if unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        } != 0
        {
            return Err("目标文件系统不支持扩展属性".into());
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&probe);
    result
}

pub fn repair_pending() -> bool {
    read_journal().is_some_and(|journal| journal.phase == "committed")
}

pub fn plan(
    app: &ServiceHandle,
    distribution: ChannelId,
    target: &str,
) -> Result<MigrationPlan, String> {
    let settings = app
        .state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let registry = app.state.packages.clone();
    let game = settings.active_game.clone();
    if channel::find(&registry, game.as_str(), &distribution).is_none() {
        return Err("当前游戏尚不支持该渠道".into());
    }
    let launcher_dir = crate::install::launcher_dir_for(&registry, &game, distribution.as_str())
        .ok_or_else(|| "当前游戏尚不支持该渠道".to_string())?;
    let (source, repair) = source_root(&registry, &settings, &distribution)?;
    let source = if source.file_name().is_some_and(|name| name == launcher_dir) {
        source.parent().unwrap_or(&source).to_path_buf()
    } else {
        source
    };
    let source = source.canonicalize().unwrap_or_else(|_| source.clone());
    if !fs_tree::mount_points_under(&source)
        .map_err(|error| format!("读取挂载点失败: {error}"))?
        .is_empty()
    {
        return Err("安装目录包含挂载点, 无法安全迁移".into());
    }
    let selected = canonical_target(target)?;
    let target = if repair {
        target_install_root(&registry, &game, &selected, &distribution).unwrap_or(selected)
    } else {
        resolve_move_target(&registry, &game, &selected, &distribution)?
    };
    if target == source {
        return Err("目标目录与当前安装目录相同".into());
    }
    if !fs_tree::valid_windows_root(&target) {
        return Err("目标路径包含 Windows 不接受的字符".into());
    }
    ensure_target_not_forbidden(&target, &source, &settings, &registry, &game, &distribution)?;
    let target_parent = target.parent().ok_or_else(|| "目标目录无效".to_string())?;
    ensure_writable(target_parent)?;
    let available = oddity_platform::disk::available_bytes(target_parent)
        .map_err(|error| format!("读取目标磁盘空间失败: {error}"))?;

    if repair {
        if target_install_root(&registry, &game, &target, &distribution).is_none() {
            return Err("旧安装目录不存在, 目标目录也不是有效的游戏安装".into());
        }
        let launcher =
            crate::install::normalize_launcher_root(&registry, &game, &target, &distribution)
                .ok_or_else(|| "目标目录不是有效的游戏安装".to_string())?;
        let install = launcher
            .parent()
            .ok_or_else(|| "目标目录不是有效的游戏安装".to_string())?
            .to_path_buf();
        ensure_writable(&install)?;
        if !crate::install::game_exe(&registry, &game, &launcher).is_some_and(|path| path.is_file())
        {
            return Err("目标目录缺少游戏可执行文件".into());
        }
        let environments = collect_environments(&settings, &source)?;
        return Ok(MigrationPlan {
            distribution: distribution.clone(),
            mode: "repair".into(),
            source: source.display().to_string(),
            target: install.display().to_string(),
            same_filesystem: true,
            files: 0,
            bytes: 0,
            required_bytes: 0,
            available_bytes: available,
            environments,
            preload_root: preload_update(&settings, &source, &install),
            warnings: vec!["旧安装目录不存在, 仅更新绑定".into()],
        });
    }

    if !crate::install::game_exe(&registry, &game, &source.join(launcher_dir))
        .is_some_and(|path| path.is_file())
    {
        return Err("当前安装缺少游戏可执行文件".into());
    }
    ensure_target_free(&target)?;
    if let Some(parent) = source.parent() {
        ensure_writable(parent)?;
    }
    let stats = fs_tree::scan(&source).map_err(|error| format!("读取安装目录失败: {error}"))?;
    if stats.hardlinks > 0 {
        return Err("当前安装包含硬链接, 暂不支持迁移".into());
    }
    if stats.special > 0 {
        return Err("当前安装包含特殊文件, 暂不支持迁移".into());
    }
    if stats.foreign_owners > 0 {
        return Err("当前安装包含不属于当前用户的文件".into());
    }
    let source_dev = fs::metadata(&source)
        .map_err(|error| format!("读取安装目录失败: {error}"))?
        .dev();
    let target_dev = fs::metadata(target_parent)
        .map_err(|error| format!("读取目标目录失败: {error}"))?
        .dev();
    let same_filesystem = source_dev == target_dev;
    if !same_filesystem {
        probe_target(target_parent)?;
    }
    let required = if same_filesystem {
        0
    } else {
        stats
            .bytes
            .saturating_add((stats.bytes / 100).max(64 * 1024 * 1024))
    };
    if required > available {
        return Err(format!(
            "目标磁盘空间不足: 需要 {} GiB, 可用 {} GiB",
            required.div_ceil(1024 * 1024 * 1024),
            available / (1024 * 1024 * 1024)
        ));
    }
    let environments = collect_environments(&settings, &source)?;
    Ok(MigrationPlan {
        distribution,
        mode: if same_filesystem {
            "move".into()
        } else {
            "copy".into()
        },
        source: source.display().to_string(),
        target: target.display().to_string(),
        same_filesystem,
        files: stats.files.saturating_add(stats.symlinks),
        bytes: stats.bytes,
        required_bytes: required,
        available_bytes: available,
        environments,
        preload_root: preload_update(&settings, &source, &target),
        warnings: Vec::new(),
    })
}

fn collect_environments(
    settings: &Settings,
    source: &Path,
) -> Result<Vec<MigrationEnvironment>, String> {
    let mut result = Vec::new();
    for environment in matched_environments(settings, source) {
        let prefix = environment
            .prefix(&crate::paths::data_dir())
            .map_err(|error| format!("运行环境 {} 无效: {error:#}", environment.name))?;
        result.push(MigrationEnvironment {
            id: environment.id.clone(),
            name: environment.name.clone(),
            prefix: prefix.display().to_string(),
        });
    }
    Ok(result)
}

pub async fn start(
    app: ServiceHandle,
    distribution: ChannelId,
    target: String,
) -> Result<MigrationStatus, String> {
    {
        let manager = app.migration.lock().map_err(|error| error.to_string())?;
        if manager.blocking() {
            return Err("安装目录迁移正在进行".into());
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let mut start_status = MigrationStatus {
        phase: "planning".into(),
        distribution: Some(distribution.clone()),
        target: target.clone(),
        can_cancel: true,
        ..MigrationStatus::default()
    };
    if let Ok(mut manager) = app.migration.lock() {
        manager.status = start_status.clone();
        manager.cancel = Some(cancel.clone());
    }
    if let Ok(mut game) = app.state.game.lock() {
        game.migration_busy = true;
    }
    emit(&app, &start_status);

    let plan_app = app.clone();
    let plan_target = target.clone();
    let plan =
        match crate::service::spawn_blocking(move || plan(&plan_app, distribution, &plan_target))
            .await
        {
            Ok(Ok(plan)) => plan,
            Ok(Err(error)) => {
                start_status.phase = "failed".into();
                start_status.error = error.clone();
                set_status(&app, start_status, None);
                if let Ok(mut game) = app.state.game.lock() {
                    game.migration_busy = false;
                }
                return Err(error);
            }
            Err(error) => {
                let error = error.to_string();
                start_status.phase = "failed".into();
                start_status.error = error.clone();
                set_status(&app, start_status, None);
                if let Ok(mut game) = app.state.game.lock() {
                    game.migration_busy = false;
                }
                return Err(error);
            }
        };
    start_status.source = plan.source.clone();
    start_status.mode = plan.mode.clone();
    start_status.total_files = plan.files;
    start_status.total_bytes = plan.bytes;
    start_status.phase = if plan.mode == "repair" {
        "rebinding".into()
    } else {
        "moving".into()
    };
    if cancel.load(Ordering::Relaxed) {
        start_status.phase = "cancelled".into();
        start_status.error = "任务已取消".into();
        start_status.can_cancel = false;
        set_status(&app, start_status.clone(), None);
        if let Ok(mut game) = app.state.game.lock() {
            game.migration_busy = false;
        }
        return Ok(start_status);
    }
    if let Ok(mut manager) = app.migration.lock() {
        manager.status = start_status.clone();
    }
    emit(&app, &start_status);

    if let Ok(mut game) = app.state.game.lock() {
        game.migration_busy = true;
    }
    let worker = app.clone();
    let cancel_state = cancel.clone();
    drop(crate::service::spawn_blocking(move || {
        let result = execute(&worker, plan, cancel);
        if let Err(error) = result {
            let mut next = current(&worker);
            if next.phase != "done" {
                let cancelled = cancel_state.load(Ordering::Relaxed) || error.contains("取消");
                next.phase = if cancelled {
                    "cancelled".into()
                } else if repair_pending() {
                    "repair".into()
                } else {
                    "failed".into()
                };
                next.error = error;
                next.can_cancel = false;
                set_status(&worker, next, None);
            }
        }
        if let Ok(mut game) = worker.state.game.lock() {
            game.migration_busy = false;
        }
    }));
    Ok(current(&app))
}

fn execute(
    app: &ServiceHandle,
    plan: MigrationPlan,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let source = PathBuf::from(&plan.source);
    let target = PathBuf::from(&plan.target);
    let distribution = plan.distribution.clone();
    let install_root_key = app
        .state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .active_game
        .clone();
    let install_root_key = channel::find(
        &app.state.packages,
        install_root_key.as_str(),
        &distribution,
    )
    .map_or_else(
        || distribution.as_str().to_string(),
        |channel| channel.install_root_key.to_string(),
    );
    let repair = plan.mode == "repair";
    let same_filesystem = plan.same_filesystem;
    let mut journal = Journal {
        id: format!("{}-{}", std::process::id(), now()),
        phase: "prepared".into(),
        source: plan.source.clone(),
        target: plan.target.clone(),
        staging: None,
        distribution: distribution.clone(),
        mode: plan.mode.clone(),
        next_settings: app
            .state
            .settings
            .lock()
            .map_err(|error| error.to_string())?
            .clone(),
        created_at: now(),
        source_dev: fs::metadata(&source).map(|meta| meta.dev()).unwrap_or(0),
        source_ino: fs::metadata(&source).map(|meta| meta.ino()).unwrap_or(0),
    };
    let lock_root = if repair {
        target.clone()
    } else {
        source.clone()
    };
    let _install_lock = if lock_root.exists() {
        Some(
            oddity_platform::install_lock::InstallLock::acquire(&lock_root)
                .map_err(|error| format!("安装目录正在被使用: {error}"))?,
        )
    } else {
        None
    };
    check_cancel(&cancel)?;
    let mut prefix_locks = Vec::new();
    for environment in matched_environments(
        &app.state
            .settings
            .lock()
            .map_err(|error| error.to_string())?
            .clone(),
        &lock_root,
    ) {
        let prefix = environment
            .prefix(&crate::paths::data_dir())
            .map_err(|error| format!("{error:#}"))?;
        prefix_locks
            .push(crate::environment::lock_prefix(&prefix).map_err(|error| format!("{error:#}"))?);
    }

    let mut next_settings = journal.next_settings.clone();
    update_settings(
        &mut next_settings,
        &install_root_key,
        &source,
        &target,
        &plan,
    );
    journal.next_settings = next_settings.clone();
    write_journal(&journal)?;

    check_cancel(&cancel)?;
    if !repair {
        if target.exists() {
            ensure_target_free(&target)?;
            fs::remove_dir(&target).map_err(|error| format!("清理空目标目录失败: {error}"))?;
        }
        if same_filesystem {
            if let Err(error) = fs::rename(&source, &target) {
                if error.raw_os_error() != Some(libc::EXDEV) {
                    return Err(format!("移动安装目录失败: {error}"));
                }
                copy_and_verify(app, &source, &target, &plan, &cancel, &mut journal)?;
            }
        } else {
            copy_and_verify(app, &source, &target, &plan, &cancel, &mut journal)?;
        }
        journal.phase = "moved".into();
        write_journal(&journal)?;
    }

    next_settings = app
        .state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    update_settings(
        &mut next_settings,
        &install_root_key,
        &source,
        &target,
        &plan,
    );
    journal.next_settings = next_settings.clone();

    if let Some(active) = next_settings.current().active_environment.as_deref() {
        let candidate = next_settings.clone();
        if let Err(error) = crate::environment_commands::prefix(&app.state.packages, &candidate) {
            rollback(&source, &target, repair);
            remove_journal();
            return Err(format!("迁移后运行环境不可用: {error}"));
        }
        if let Err(error) = crate::environment_commands::verify_proton(&candidate) {
            rollback(&source, &target, repair);
            remove_journal();
            return Err(format!("迁移后 Proton 校验失败: {error}"));
        }
        let _ = active;
    }

    if check_cancel(&cancel).is_err() {
        rollback(&source, &target, repair);
        remove_journal();
        return Err("任务已取消".into());
    }
    journal.phase = "committed".into();
    write_journal(&journal)?;
    if let Err(error) = crate::settings::persist(&next_settings) {
        rollback(&source, &target, repair);
        remove_journal();
        return Err(format!("保存迁移后的设置失败: {error}"));
    }
    if let Ok(mut settings) = app.state.settings.lock() {
        *settings = next_settings.clone();
    }

    set_progress(app, "rebinding", &plan, 0, 0, "");
    rebind(
        &app.state.packages,
        &next_settings,
        &distribution,
        &source,
        &target,
    )?;
    journal.phase = "done".into();
    write_journal(&journal)?;

    let cleanup_pending = source.exists() && !repair;
    let mut done = MigrationStatus {
        phase: "done".into(),
        distribution: Some(distribution.clone()),
        mode: plan.mode,
        source: source.display().to_string(),
        target: target.display().to_string(),
        files: plan.files,
        total_files: plan.files,
        bytes: plan.bytes,
        total_bytes: plan.bytes,
        current: String::new(),
        error: String::new(),
        cleanup_pending,
        cleanup_source: if cleanup_pending {
            source.display().to_string()
        } else {
            String::new()
        },
        can_cancel: false,
    };
    drop(prefix_locks);
    set_status(app, std::mem::take(&mut done), None);
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("任务已取消".into());
    }
    Ok(())
}

fn copy_and_verify(
    app: &ServiceHandle,
    source: &Path,
    target: &Path,
    plan: &MigrationPlan,
    cancel: &AtomicBool,
    journal: &mut Journal,
) -> Result<(), String> {
    let staging =
        target.with_file_name(format!(".oddity-migrate-{}-{}", std::process::id(), now()));
    journal.staging = Some(staging.display().to_string());
    journal.phase = "copying".into();
    write_journal(journal)?;
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| format!("清理旧临时目录失败: {error}"))?;
    }
    let stats = fs_tree::scan(source).map_err(|error| format!("{error}"))?;
    let progress_app = app.clone();
    let progress_plan = plan.clone();
    let mut last_emit = std::time::Instant::now();
    let result = fs_tree::copy_tree_to(source, &staging, target, &stats, cancel, &mut |progress| {
        if last_emit.elapsed().as_millis() >= 150 {
            last_emit = std::time::Instant::now();
            set_progress(
                &progress_app,
                "copying",
                &progress_plan,
                progress.files,
                progress.bytes,
                &progress.current,
            );
        }
    });
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("复制安装目录失败: {error}"));
    }
    journal.phase = "verifying".into();
    write_journal(journal)?;
    set_progress(app, "verifying", plan, 0, 0, "");
    let progress_app = app.clone();
    let progress_plan = plan.clone();
    let mut last_emit = std::time::Instant::now();
    if let Err(error) =
        fs_tree::verify_tree_to(source, &staging, target, &stats, cancel, &mut |progress| {
            if last_emit.elapsed().as_millis() >= 150 {
                last_emit = std::time::Instant::now();
                set_progress(
                    &progress_app,
                    "verifying",
                    &progress_plan,
                    progress.files,
                    progress.bytes,
                    &progress.current,
                );
            }
        })
    {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("校验复制结果失败: {error}"));
    }
    if let Err(error) = fs::rename(&staging, target) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("提交目标目录失败: {error}"));
    }
    if let Some(parent) = target.parent() {
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}

fn update_settings(
    settings: &mut Settings,
    install_root_key: &str,
    source: &Path,
    target: &Path,
    plan: &MigrationPlan,
) {
    let target_text = target.display().to_string();
    {
        let current = settings.current_mut();
        current
            .install_roots
            .insert(install_root_key.to_string(), target_text);
        if let Some(preload) = &plan.preload_root {
            current.preload_root = Some(preload.clone());
        }
    }
    for environment in &mut settings.runtime_environments {
        if environment.game == settings.active_game
            && fs_tree::same_path(&environment.install_root, source)
        {
            environment.install_root = target.to_path_buf();
        }
    }
}

fn rebind(
    registry: &Registry,
    settings: &Settings,
    distribution: &ChannelId,
    source: &Path,
    target: &Path,
) -> Result<(), String> {
    let game = &settings.active_game;
    // 链接名属于游戏包; 没有迁移链接的游戏只改注册表与官方配置.
    if let Some(link_name) = crate::install::prefix_link_name(registry, game) {
        for environment in matched_environments(settings, target) {
            let prefix = environment
                .prefix(&crate::paths::data_dir())
                .map_err(|error| format!("{error:#}"))?;
            wine::rewrite_prefix(&prefix, link_name, source, target)
                .map_err(|error| format!("更新 prefix 链接失败: {error}"))?;
            wine::rewrite_registry(&prefix, source, target)
                .map_err(|error| format!("更新注册表失败: {error}"))?;
        }
    }
    let launcher_dir = crate::install::launcher_dir_for(registry, game, distribution.as_str())
        .ok_or_else(|| "当前游戏尚不支持该地区".to_string())?;
    let game_config = crate::install::launcher_config_path(registry, game, target);
    wine::rewrite_official(target, launcher_dir, game_config.as_deref(), source, target)
        .map_err(|error| format!("更新官方配置失败: {error}"))?;
    Ok(())
}

fn rollback(source: &Path, target: &Path, repair: bool) {
    if repair {
        return;
    }
    if source.exists() {
        let _ = fs::remove_dir_all(target);
    } else if target.exists() {
        let _ = fs::rename(target, source);
    }
}

fn set_progress(
    app: &ServiceHandle,
    phase: &str,
    plan: &MigrationPlan,
    files: u64,
    bytes: u64,
    file: &str,
) {
    let mut status = current(app);
    status.phase = phase.into();
    status.distribution = Some(plan.distribution.clone());
    status.mode = plan.mode.clone();
    status.source = plan.source.clone();
    status.target = plan.target.clone();
    status.files = files;
    status.total_files = plan.files;
    status.bytes = bytes;
    status.total_bytes = plan.bytes;
    status.current = file.to_string();
    status.can_cancel = matches!(phase, "copying" | "verifying");
    set_status(app, status, None);
}

pub fn cancel(app: &ServiceHandle) -> MigrationStatus {
    if let Ok(manager) = app.migration.lock() {
        if let Some(cancel) = &manager.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    let mut status = current(app);
    status.can_cancel = false;
    status.error = "正在取消迁移".into();
    set_status(app, status.clone(), None);
    status
}

pub fn retry(app: &ServiceHandle) -> Result<MigrationStatus, String> {
    let _operation = app.migration_op.lock().map_err(|error| error.to_string())?;
    let Some(journal) = read_journal() else {
        return Ok(current(app));
    };
    if journal.phase != "committed" {
        return Ok(current(app));
    }
    if let Ok(mut game) = app.state.game.lock() {
        game.migration_busy = true;
    }
    if !Path::new(&journal.target).exists() {
        if let Ok(mut game) = app.state.game.lock() {
            game.migration_busy = false;
        }
        let mut failed = current(app);
        failed.phase = "repair".into();
        failed.error = "迁移目标目录不存在".into();
        set_status(app, failed.clone(), None);
        return Err(failed.error);
    }
    let mut status = current(app);
    status.phase = "rebinding".into();
    status.error.clear();
    set_status(app, status, None);
    if let Err(error) = crate::settings::persist(&journal.next_settings) {
        if let Ok(mut game) = app.state.game.lock() {
            game.migration_busy = false;
        }
        let mut failed = current(app);
        failed.phase = "repair".into();
        failed.error = format!("保存迁移设置失败: {error}");
        set_status(app, failed.clone(), None);
        return Err(failed.error);
    }
    if let Ok(mut settings) = app.state.settings.lock() {
        *settings = journal.next_settings.clone();
    }
    let result = rebind(
        &app.state.packages,
        &journal.next_settings,
        &journal.distribution,
        Path::new(&journal.source),
        Path::new(&journal.target),
    );
    if let Ok(mut game) = app.state.game.lock() {
        game.migration_busy = false;
    }
    match result {
        Ok(()) => {
            let mut journal = journal;
            journal.phase = "done".into();
            let _ = write_journal(&journal);
            let source_exists = Path::new(&journal.source).exists();
            let mut done = current(app);
            done.phase = "done".into();
            done.cleanup_pending = source_exists && journal.mode != "repair";
            done.cleanup_source = if done.cleanup_pending {
                journal.source.clone()
            } else {
                String::new()
            };
            done.error.clear();
            set_status(app, done.clone(), None);
            Ok(done)
        }
        Err(error) => {
            let mut failed = current(app);
            failed.phase = "repair".into();
            failed.error = format!("更新绑定失败: {error}");
            set_status(app, failed.clone(), None);
            Err(failed.error)
        }
    }
}

pub fn cleanup(app: ServiceHandle) -> Result<MigrationStatus, String> {
    {
        let manager = app.migration.lock().map_err(|error| error.to_string())?;
        if manager.blocking() {
            return Err("安装目录迁移正在进行".into());
        }
    }
    let Some(journal) = read_journal() else {
        return Ok(current(&app));
    };
    if journal.phase != "done" {
        return Err("上次迁移尚未完成".into());
    }
    let source = PathBuf::from(&journal.source);
    if fs::symlink_metadata(&source).is_err() {
        remove_journal();
        let mut done = current(&app);
        done.phase = "done".into();
        done.cleanup_pending = false;
        done.cleanup_source.clear();
        set_status(&app, done.clone(), None);
        return Ok(done);
    }
    let metadata =
        fs::symlink_metadata(&source).map_err(|error| format!("读取旧目录失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("旧目录不是普通目录, 请手动处理".into());
    }
    if journal.source_dev != 0
        && (metadata.dev() != journal.source_dev || metadata.ino() != journal.source_ino)
    {
        return Err("旧目录已被替换, 请手动处理".into());
    }
    if !fs_tree::mount_points_under(&source)
        .map_err(|error| format!("读取挂载点失败: {error}"))?
        .is_empty()
    {
        return Err("旧目录包含挂载点, 请手动处理".into());
    }
    let target = Path::new(&journal.target);
    let game = &journal.next_settings.active_game;
    let launcher = crate::install::normalize_launcher_root(
        &app.state.packages,
        game,
        target,
        &journal.distribution,
    )
    .ok_or_else(|| "目标安装不可用, 已保留旧目录".to_string())?;
    if !crate::install::game_exe(&app.state.packages, game, &launcher)
        .is_some_and(|path| path.is_file())
    {
        return Err("目标安装缺少游戏可执行文件, 已保留旧目录".into());
    }
    if let Ok(mut game) = app.state.game.lock() {
        game.migration_busy = true;
    }
    let mut status = current(&app);
    status.phase = "cleanup".into();
    status.cleanup_pending = true;
    status.can_cancel = false;
    set_status(&app, status, None);
    let worker = app.clone();
    drop(crate::service::spawn_blocking(move || {
        match fs_tree::remove_tree(&source) {
            Ok(()) => {
                remove_journal();
                let mut done = current(&worker);
                done.phase = "done".into();
                done.cleanup_pending = false;
                done.cleanup_source.clear();
                set_status(&worker, done, None);
            }
            Err(error) => {
                let mut failed = current(&worker);
                failed.phase = "done".into();
                failed.cleanup_pending = true;
                failed.error = format!("清理旧目录失败: {error}");
                set_status(&worker, failed, None);
            }
        }
        if let Ok(mut game) = worker.state.game.lock() {
            game.migration_busy = false;
        }
    }));
    Ok(current(&app))
}

/// 放弃无法完成的修复; 保留文件与当前设置, 只解除迁移阻塞.
pub fn abandon(app: &ServiceHandle) -> Result<MigrationStatus, String> {
    let _operation = app.migration_op.lock().map_err(|error| error.to_string())?;
    let manager = app.migration.lock().map_err(|error| error.to_string())?;
    let repairing = manager.status.phase == "repair";
    if manager.blocking() && !repairing {
        return Err("安装目录迁移正在进行".into());
    }
    drop(manager);
    let journal = read_journal();
    if !repairing && !journal.is_some_and(|journal| journal.phase == "committed") {
        return Err("当前没有待修复的迁移".into());
    }
    remove_journal_checked()?;
    if let Ok(mut game) = app.state.game.lock() {
        game.migration_busy = false;
    }
    let status = MigrationStatus::idle();
    set_status(app, status.clone(), None);
    Ok(status)
}

pub fn dismiss(app: &ServiceHandle) -> Result<MigrationStatus, String> {
    {
        let manager = app.migration.lock().map_err(|error| error.to_string())?;
        if manager.blocking() {
            return Err("安装目录迁移正在进行".into());
        }
    }
    remove_journal();
    let status = MigrationStatus::idle();
    set_status(app, status.clone(), None);
    Ok(status)
}

/// 启动时恢复未完成的迁移.
pub fn recover(registry: &Registry, data: &Path, _settings_path: &Path) {
    let Some(mut journal) = read_journal() else {
        return;
    };
    let source = PathBuf::from(&journal.source);
    let target = PathBuf::from(&journal.target);
    match journal.phase.as_str() {
        "done" => {}
        "committed" => {
            if !target.exists() {
                eprintln!("迁移目标目录不存在, 等待修复");
                return;
            }
            if !Path::new(&journal.target).exists()
                || crate::settings::persist(&journal.next_settings).is_err()
            {
                return;
            }
            if let Err(error) = rebind(
                registry,
                &journal.next_settings,
                &journal.distribution,
                &source,
                &target,
            ) {
                eprintln!("迁移重绑定未完成: {error}");
                return;
            }
            journal.phase = "done".into();
            let _ = write_journal(&journal);
        }
        _ => {
            if source.exists() {
                if let Some(staging) = &journal.staging {
                    let _ = fs::remove_dir_all(staging);
                }
                if matches!(journal.phase.as_str(), "moved" | "verifying")
                    && target.exists()
                    && target != source
                {
                    let _ = fs::remove_dir_all(&target);
                }
                remove_journal();
            } else if target.exists() {
                if !Path::new(&journal.target).exists()
                    || crate::settings::persist(&journal.next_settings).is_err()
                {
                    return;
                }
                if rebind(
                    registry,
                    &journal.next_settings,
                    &journal.distribution,
                    &source,
                    &target,
                )
                .is_err()
                {
                    return;
                }
                journal.phase = "done".into();
                let _ = write_journal(&journal);
            } else {
                remove_journal();
            }
        }
    }
    let _ = data;
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::ProtonPin;
    use crate::identity::GameId;
    use crate::tests::{EnvGuard, TempDir};
    use base64::Engine;
    use std::os::unix::fs::symlink;

    #[test]
    fn preload_paths_inside_the_install_are_rewritten() {
        let source = Path::new("/games/old");
        let target = Path::new("/games/new");
        let mut settings = Settings::default();
        settings.current_mut().preload_root = Some(format!(
            "/games/old/{}/preload",
            crate::testing::LAUNCHER_DIR
        ));
        assert_eq!(
            preload_update(&settings, source, target).as_deref(),
            Some(format!("/games/new/{}/preload", crate::testing::LAUNCHER_DIR).as_str())
        );
        settings.current_mut().preload_root = Some("/cache/preload".into());
        assert_eq!(preload_update(&settings, source, target), None);
        settings.current_mut().preload_root = Some("preload".into());
        assert_eq!(preload_update(&settings, source, target), None);
    }

    #[test]
    fn non_empty_target_gets_the_game_subfolder() {
        let directory = TempDir::new("migration-target");
        let parent = directory.join("games");
        directory.write("games/other.txt", b"other");
        let registry = crate::testing::packages();
        let game = GameId::from("nte");
        assert_eq!(
            resolve_move_target(&registry, &game, &parent, &ChannelId::new("cn")).unwrap(),
            parent.join(crate::testing::DEFAULT_INSTALL_NAME)
        );
        assert_eq!(
            resolve_move_target(&registry, &game, &parent, &ChannelId::new("global")).unwrap(),
            parent.join(crate::testing::DEFAULT_GLOBAL_INSTALL_NAME)
        );
        let empty = directory.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(
            resolve_move_target(&registry, &game, &empty, &ChannelId::new("cn")).unwrap(),
            empty
        );
    }

    #[test]
    fn non_empty_target_plan_uses_the_game_subfolder() {
        let directory = TempDir::new("migration-plan-target");
        let source = directory.join("source");
        directory.write(
            format!("source/{}/Config/Config.ini", crate::testing::LAUNCHER_DIR),
            b"[VERSION]
Version=1.0
",
        );
        directory.write(format!("source/{}", crate::testing::GAME_EXE), b"MZ");
        let parent = directory.join("games");
        directory.write("games/other.txt", b"other");
        let app = crate::service::test_service();
        let mut settings = Settings::default();
        settings
            .current_mut()
            .install_roots
            .insert("cn".into(), source.display().to_string());
        *app.state.settings.lock().unwrap() = settings;
        let plan = plan(&app, ChannelId::new("cn"), &parent.display().to_string()).unwrap();
        assert_eq!(
            Path::new(&plan.target),
            parent.join(crate::testing::DEFAULT_INSTALL_NAME)
        );
    }

    #[test]
    fn global_migration_updates_global_install_root_and_environments() {
        let mut settings = Settings::default();
        settings
            .current_mut()
            .install_roots
            .insert("cn".into(), "/games/cn".into());
        settings
            .current_mut()
            .install_roots
            .insert("global".into(), "/games/old-global".into());
        settings.current_mut().distribution = Some(ChannelId::new("global"));
        settings.runtime_environments = vec![RuntimeEnvironment {
            id: "a".repeat(32),
            name: "global".into(),
            proton: ProtonPin {
                path: PathBuf::from("/proton"),
                identity: String::new(),
            },
            install_root: PathBuf::from("/games/old-global"),
            game: GameId::from("nte"),
            distribution: ChannelId::new("global"),
            created_at: 1,
        }];
        let plan = MigrationPlan {
            distribution: ChannelId::new("global"),
            mode: "move".into(),
            source: "/games/old-global".into(),
            target: "/games/new-global".into(),
            same_filesystem: true,
            files: 0,
            bytes: 0,
            required_bytes: 0,
            available_bytes: 0,
            environments: Vec::new(),
            preload_root: None,
            warnings: Vec::new(),
        };
        update_settings(
            &mut settings,
            "global",
            Path::new("/games/old-global"),
            Path::new("/games/new-global"),
            &plan,
        );
        assert_eq!(
            settings.current().install_root_for_key("global"),
            Some("/games/new-global")
        );
        assert_eq!(
            settings.current().install_root_for_key("cn"),
            Some("/games/cn")
        );
        assert_eq!(
            settings.runtime_environments[0].install_root,
            PathBuf::from("/games/new-global")
        );
    }

    #[test]
    fn journal_round_trips_next_settings() {
        let dir = TempDir::new("migration-journal");
        let journal = Journal {
            id: "test".into(),
            phase: "prepared".into(),
            source: dir.join("old").display().to_string(),
            target: dir.join("new").display().to_string(),
            staging: None,
            distribution: ChannelId::new("cn"),
            mode: "move".into(),
            next_settings: Settings::default(),
            created_at: 1,
            source_dev: 0,
            source_ino: 0,
        };
        let path = dir.join("current.json");
        let text = serde_json::to_vec_pretty(&journal).unwrap();
        fs::write(&path, text).unwrap();
        let restored: Journal = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(restored.phase, "prepared");
        assert_eq!(restored.distribution, ChannelId::new("cn"));
    }

    #[test]
    fn interrupted_copy_keeps_the_source_and_removes_the_incomplete_target() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-recover");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("install");
        let target = directory.join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(source.join("game.pak"), b"source").unwrap();
        std::fs::write(target.join("game.pak"), b"partial").unwrap();
        let settings_path = directory.join("config/oddity/settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        let journal = Journal {
            id: "recover".into(),
            phase: "moved".into(),
            source: source.display().to_string(),
            target: target.display().to_string(),
            staging: None,
            distribution: ChannelId::new("cn"),
            mode: "copy".into(),
            next_settings: Settings::default(),
            created_at: 1,
            source_dev: 0,
            source_ino: 0,
        };
        write_journal(&journal).unwrap();
        recover(
            &crate::testing::packages(),
            &directory.join("data/oddity"),
            &settings_path,
        );
        assert_eq!(std::fs::read(source.join("game.pak")).unwrap(), b"source");
        assert!(!target.exists());
        assert!(read_journal().is_none());
    }

    #[test]
    fn migrates_a_small_install_and_rebinds_prefix_and_official_config() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-command");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));

        let source = directory.join("install");
        let target = directory.join("target");
        let id = "a".repeat(32);
        let proton = directory.join("proton");
        directory.write("proton/proton", b"runner");
        directory.write("proton/toolmanifest.vdf", b"manifest");
        directory.write("proton/version", b"test");
        let pin = ProtonPin::capture(&proton).unwrap();
        directory.write(
            format!("install/{}/Config/Config.ini", crate::testing::LAUNCHER_DIR),
            b"[VERSION]\nVersion=1.0\n",
        );
        directory.write(format!("install/{}", crate::testing::GAME_EXE), b"MZ");
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!(
            "{}\\{}\\secret",
            wine::wine_path(&source),
            crate::testing::LAUNCHER_DIR
        ));
        directory.write(
            format!(
                "install/{}/UserData/Config/Config.ini",
                crate::testing::LAUNCHER_DIR
            ),
            format!("[Game]\nACE={encoded}\n"),
        );
        let environment = directory.join(format!("data/oddity/environments/{id}"));
        let prefix = environment.join("prefix");
        std::fs::create_dir_all(prefix.join("drive_c/Program Files (x86)")).unwrap();
        std::fs::write(environment.join(".oddity-environment"), &id).unwrap();
        let link = prefix.join(crate::testing::PREFIX_LINK);
        symlink(&source, &link).unwrap();
        let old_wine = wine::wine_path(&source).replace('\\', "\\\\");
        std::fs::write(
            prefix.join("system.reg"),
            format!(
                "\"ImagePath\"=str(2):\"{old_wine}\\\\{}\\\\driver\\\\x.sys\"",
                crate::testing::LAUNCHER_DIR
            ),
        )
        .unwrap();

        let profile = RuntimeEnvironment {
            id: id.clone(),
            name: "test".into(),
            proton: pin,
            install_root: source.clone(),
            game: GameId::from("nte"),
            distribution: ChannelId::new("cn"),
            created_at: 1,
        };
        let mut settings = Settings::default();
        settings
            .current_mut()
            .install_roots
            .insert("cn".into(), source.display().to_string());
        settings.current_mut().active_environment = Some(id.clone());
        settings.current_mut().stable_environment = Some(id.clone());
        settings.runtime_environments = vec![profile];
        let app = crate::service::test_service();
        *app.state.settings.lock().unwrap() = settings;

        let plan = plan(&app, ChannelId::new("cn"), &target.display().to_string()).unwrap();
        assert_eq!(plan.mode, "move");
        assert_eq!(plan.environments.len(), 1);
        app.state.settings.lock().unwrap().theme = "dark".into();
        execute(&app, plan, Arc::new(AtomicBool::new(false))).unwrap();

        assert!(!source.exists());
        assert!(target.join(crate::testing::GAME_EXE).is_file());
        let stored = app.state.settings.lock().unwrap();
        assert_eq!(stored.theme, "dark");
        assert_eq!(
            stored.current().install_root_for_key("cn"),
            Some(target.display().to_string().as_str())
        );
        assert_eq!(stored.runtime_environments[0].install_root, target);
        drop(stored);
        assert_eq!(std::fs::read_link(link).unwrap(), target);
        let registry = std::fs::read_to_string(prefix.join("system.reg")).unwrap();
        assert!(registry.contains(&wine::wine_path(&target).replace('\\', "\\\\")));
        let official = std::fs::read_to_string(target.join(format!(
            "{}/UserData/Config/Config.ini",
            crate::testing::LAUNCHER_DIR
        )))
        .unwrap();
        let ace = official
            .lines()
            .find_map(|line| line.strip_prefix("ACE="))
            .unwrap();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(ace)
                .unwrap(),
        )
        .unwrap();
        assert!(decoded.contains(&wine::wine_path(&target)));
    }

    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..300 {
            if condition() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    fn journal_with(_directory: &TempDir, phase: &str, source: &Path, target: &Path) -> Journal {
        let mut next = Settings::default();
        next.current_mut()
            .install_roots
            .insert("cn".into(), target.display().to_string());
        Journal {
            id: format!("test-{phase}"),
            phase: phase.into(),
            source: source.display().to_string(),
            target: target.display().to_string(),
            staging: None,
            distribution: ChannelId::new("cn"),
            mode: "move".into(),
            next_settings: next,
            created_at: 1,
            source_dev: 0,
            source_ino: 0,
        }
    }

    fn install(directory: &TempDir, root: &str) {
        directory.write(
            format!("{root}/{}/Config/Config.ini", crate::testing::LAUNCHER_DIR),
            b"[VERSION]
Version=1.0
",
        );
        directory.write(format!("{root}/{}", crate::testing::GAME_EXE), b"MZ");
    }

    #[test]
    fn recover_committed_applies_settings_and_finishes() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-recover-committed");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        install(&directory, "target");
        write_journal(&journal_with(&directory, "committed", &source, &target)).unwrap();
        recover(
            &crate::testing::packages(),
            &directory.join("data/oddity"),
            &directory.join("config/oddity/settings.json"),
        );
        assert_eq!(read_journal().unwrap().phase, "done");
        assert!(directory.join("config/oddity/settings.json").is_file());
    }

    #[test]
    fn recover_committed_keeps_journal_when_target_is_missing() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-recover-missing-target");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        write_journal(&journal_with(&directory, "committed", &source, &target)).unwrap();
        recover(
            &crate::testing::packages(),
            &directory.join("data/oddity"),
            &directory.join("config/oddity/settings.json"),
        );
        assert_eq!(read_journal().unwrap().phase, "committed");
    }

    #[test]
    fn recover_moved_missing_source_rolls_forward() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-recover-roll-forward");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        install(&directory, "target");
        write_journal(&journal_with(&directory, "moved", &source, &target)).unwrap();
        recover(
            &crate::testing::packages(),
            &directory.join("data/oddity"),
            &directory.join("config/oddity/settings.json"),
        );
        assert_eq!(read_journal().unwrap().phase, "done");
        assert!(directory.join("config/oddity/settings.json").is_file());
    }

    #[test]
    fn recover_discards_staging_and_partial_target() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-recover-staging");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        let staging = directory.join("staging");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        let mut journal = journal_with(&directory, "verifying", &source, &target);
        journal.staging = Some(staging.display().to_string());
        write_journal(&journal).unwrap();
        recover(
            &crate::testing::packages(),
            &directory.join("data/oddity"),
            &directory.join("config/oddity/settings.json"),
        );
        assert!(source.exists());
        assert!(!target.exists());
        assert!(!staging.exists());
        assert!(read_journal().is_none());
    }

    #[test]
    fn cleanup_removes_done_source() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-cleanup");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        directory.write("old/file", b"old");
        install(&directory, "target");
        let journal = journal_with(&directory, "done", &source, &target);
        write_journal(&journal).unwrap();
        let app = crate::service::test_service();
        cleanup(app).unwrap();
        assert!(wait_until(|| !source.exists()));
        assert!(read_journal().is_none());
    }

    #[test]
    fn cleanup_without_source_finishes() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-cleanup-missing");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        write_journal(&journal_with(&directory, "done", &source, &target)).unwrap();
        let app = crate::service::test_service();
        let status = cleanup(app).unwrap();
        assert_eq!(status.phase, "done");
        assert!(!status.cleanup_pending);
        assert!(read_journal().is_none());
    }

    #[test]
    fn retry_handles_missing_target_and_committed_journal() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-retry");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        let app = crate::service::test_service();
        write_journal(&journal_with(&directory, "committed", &source, &target)).unwrap();
        assert!(retry(&app).is_err());
        assert_eq!(read_journal().unwrap().phase, "committed");
        install(&directory, "target");
        let status = retry(&app).unwrap();
        assert_eq!(status.phase, "done");
        assert_eq!(read_journal().unwrap().phase, "done");
    }

    #[test]
    fn cancel_flags_the_active_operation() {
        let app = crate::service::test_service();
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut manager = app.migration.lock().unwrap();
            manager.status = MigrationStatus {
                phase: "copying".into(),
                can_cancel: true,
                ..MigrationStatus::default()
            };
            manager.cancel = Some(flag.clone());
        }
        let status = cancel(&app);
        assert!(flag.load(Ordering::Relaxed));
        assert!(!status.can_cancel);
    }

    #[test]
    fn cleanup_refuses_when_the_new_install_is_missing() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-cleanup-target");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        directory.write("old/file", b"old");
        write_journal(&journal_with(&directory, "done", &source, &target)).unwrap();
        let app = crate::service::test_service();
        assert!(cleanup(app).is_err());
        assert!(source.exists());
        assert!(read_journal().is_some());
    }

    #[test]
    fn abandon_clears_a_committed_repair_without_touching_files() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-abandon");
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        directory.write("target/file", b"target");
        write_journal(&journal_with(&directory, "committed", &source, &target)).unwrap();
        let app = crate::service::test_service();
        {
            let mut manager = app.migration.lock().unwrap();
            manager.status.phase = "repair".into();
        }
        let status = abandon(&app).unwrap();
        assert_eq!(status.phase, "idle");
        assert!(read_journal().is_none());
        assert!(target.join("file").is_file());
    }

    #[test]
    fn abandon_refuses_while_retry_is_rebinding() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-abandon-rebinding");
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        write_journal(&journal_with(&directory, "committed", &source, &target)).unwrap();
        let app = crate::service::test_service();
        app.migration.lock().unwrap().status.phase = "rebinding".into();
        assert!(abandon(&app).is_err());
        assert!(read_journal().is_some());
    }

    #[test]
    fn dismiss_clears_a_finished_journal() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-dismiss");
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("old");
        let target = directory.join("target");
        write_journal(&journal_with(&directory, "done", &source, &target)).unwrap();
        let app = crate::service::test_service();
        let status = dismiss(&app).unwrap();
        assert_eq!(status.phase, "idle");
        assert!(read_journal().is_none());
    }

    #[test]
    fn plan_rejects_invalid_targets_and_unsupported_games() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("migration-plan-errors");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        let source = directory.join("source");
        install(&directory, "source");
        let app = crate::service::test_service();
        let mut settings = Settings::default();
        settings
            .current_mut()
            .install_roots
            .insert("cn".into(), source.display().to_string());
        *app.state.settings.lock().unwrap() = settings;
        assert!(plan(&app, ChannelId::new("cn"), "relative").is_err());
        assert!(plan(&app, ChannelId::new("cn"), &source.display().to_string()).is_err());
        assert!(plan(
            &app,
            ChannelId::new("cn"),
            &source.join("child").display().to_string()
        )
        .is_err());
        assert!(plan(
            &app,
            ChannelId::new("cn"),
            &crate::paths::data_dir().join("x").display().to_string()
        )
        .is_err());
        assert!(plan(
            &app,
            ChannelId::new("cn"),
            &directory.join("bad:name").display().to_string()
        )
        .is_err());
        let hsr = Settings {
            active_game: GameId::from("hsr"),
            ..Settings::default()
        };
        *app.state.settings.lock().unwrap() = hsr;
        assert!(plan(
            &app,
            ChannelId::new("cn"),
            &directory.join("hsr").display().to_string()
        )
        .is_err());
    }
}
