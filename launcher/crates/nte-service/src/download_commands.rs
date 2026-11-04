use crate::service::ServiceHandle;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use nte_core::config::LauncherConfig;
use nte_core::game::check::CheckOptions;
use nte_core::game::download::ApplyOptions;
use nte_core::game::optional::OptionalSelection;
use nte_core::game::plan::{StageTimings, Throughput};
use nte_core::throttle::RateLimiter;
use nte_core::{disk, game, install, update, updates};

use crate::{game_exe_for, selected_install_root, session_alive, AppState, Settings};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub(crate) phase: String,
    pub(crate) mode: String,
    /// 本轮任务的触发来源: manual 或 auto.
    pub(crate) origin: String,
    /// 自动下载当前等待的原因, 空字符串表示没有等待.
    pub(crate) auto_waiting: String,
    pub(crate) distribution: String,
    pub(crate) install_root: String,
    /// 官方本地清单存在时视为已安装，用于区分安装与更新。
    pub(crate) installed: bool,
    full: bool,
    pub(crate) unverified_files: usize,
    current_file: String,
    current_file_bytes: u64,
    completed_files: usize,
    total_files: usize,
    downloaded_bytes: u64,
    total_bytes: u64,
    patch_files: usize,
    patch_bytes: u64,
    full_files: usize,
    full_bytes: u64,
    stage: String,
    remote_version: String,
    /// 本次计划包含的可选资源 tag.
    optional_tags: Vec<String>,
    /// 当前预下载任务是在校验已完成的缓存.
    verify: bool,
    pub(crate) pending_files: usize,
    /// 最近一次资源检查完成的时间 (Unix 毫秒), 0 表示尚未检查.
    pub(crate) checked_at: u64,
    /// 本轮写入目标所在文件系统的可用字节, 读取失败时为空.
    pub(crate) disk_available_bytes: Option<u64>,
    /// 当前计划预计额外占用的字节.
    pub(crate) disk_required_bytes: u64,
    error: String,
}

impl Default for DownloadStatus {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            mode: "check".into(),
            origin: "manual".into(),
            auto_waiting: String::new(),
            distribution: "cn".into(),
            install_root: String::new(),
            installed: false,
            full: false,
            unverified_files: 0,
            current_file: String::new(),
            current_file_bytes: 0,
            completed_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: 0,
            patch_files: 0,
            patch_bytes: 0,
            full_files: 0,
            full_bytes: 0,
            stage: String::new(),
            remote_version: String::new(),
            optional_tags: Vec::new(),
            verify: false,
            pending_files: 0,
            checked_at: 0,
            disk_available_bytes: None,
            disk_required_bytes: 0,
            error: String::new(),
        }
    }
}

fn preload_root_for(settings: &Settings, install_root: &Path) -> std::path::PathBuf {
    let configured = settings
        .current()
        .preload_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::expand_path);
    match configured {
        Some(path) if path.is_absolute() => path,
        Some(path) => install_root.join(path),
        None => install_root
            .join(crate::current_launcher_dir(settings))
            .join("UserData")
            .join("Patcher")
            .join("PatcherSDK")
            .join("preload"),
    }
}

fn apply_preload_root(config: &mut game::GameUpdateConfig, settings: &Settings) {
    config.preload_root = Some(preload_root_for(settings, &config.install_root));
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCatalog {
    distribution: String,
    install_root: String,
    /// 当前正式资源版本.
    version: String,
    /// 公告中的未来版本, 空字符串表示当前没有资源预下载.
    preload_version: String,
    /// 公告版本仍晚于当前正式版本, 可以执行预下载.
    preload_available: bool,
    /// 公告版本的预下载缓存是否已完成.
    preload_completed: bool,
    /// 缓存根下是否存在可删除的预下载缓存.
    preload_cached: bool,
    /// 删除按钮对应的缓存版本.
    preload_cached_version: String,
    /// 缓存目录当前占用的字节数.
    preload_cached_bytes: u64,
    resources: Vec<game::optional::OptionalResource>,
}

fn launcher_root_for(settings: &Settings) -> std::path::PathBuf {
    if crate::configured_install_root(settings).is_none() {
        crate::detect_launcher_root(settings).unwrap_or_else(|| {
            selected_install_root(settings).join(crate::current_launcher_dir(settings))
        })
    } else {
        selected_install_root(settings).join(crate::current_launcher_dir(settings))
    }
}

fn load_catalog_context(
    app: &ServiceHandle,
) -> Result<(Settings, game::GameUpdateConfig, game::check::RemoteBundle), String> {
    let settings = app
        .state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let root = launcher_root_for(&settings);
    let launcher =
        LauncherConfig::load(&root).map_err(|error| format!("读取现有启动器配置失败: {error}"))?;
    let mut config = game::GameUpdateConfig::load(&root, launcher.game_id)
        .map_err(|error| format!("读取游戏资源配置失败: {error}"))?;
    apply_preload_root(&mut config, &settings);
    let bundle = game::check::load_remote(&config)
        .map_err(|error| format!("读取可选资源目录失败: {error}"))?;
    Ok((settings, config, bundle))
}

fn preload_target(
    config: &game::GameUpdateConfig,
    bundle: &game::check::RemoteBundle,
) -> Option<(String, String)> {
    game::preload::target(config, &bundle.config).map(|target| (target.branch, target.version))
}

fn catalog_payload(
    settings: &Settings,
    config: &game::GameUpdateConfig,
    bundle: &game::check::RemoteBundle,
) -> Result<ResourceCatalog, String> {
    let target = preload_target(config, bundle);
    let announced_version = bundle
        .config
        .preload
        .as_ref()
        .map(|preload| preload.version.clone())
        .unwrap_or_default();
    let preload_version = target
        .as_ref()
        .map(|(_, version)| version.clone())
        .unwrap_or(announced_version);
    let preload_available = target.is_some();
    let preload_completed = target.as_ref().is_some_and(|(branch, version)| {
        game::preload::completion(config, branch, version).is_some()
    });
    let announced_cache = bundle.config.preload.as_ref().and_then(|preload| {
        let bytes = game::preload::cache_bytes(config, &preload.branch, &preload.version);
        (bytes > 0).then(|| (preload.version.clone(), bytes))
    });
    let cached = announced_cache.or_else(|| {
        game::preload::cached_version(config).map(|cached| (cached.version, cached.bytes))
    });
    let (preload_cached_version, preload_cached_bytes) = cached.unwrap_or((String::new(), 0));
    Ok(ResourceCatalog {
        distribution: settings.current().distribution.as_str().into(),
        install_root: config.install_root.display().to_string(),
        version: bundle.config.res_version.clone(),
        preload_version,
        preload_available,
        preload_completed,
        preload_cached: preload_cached_bytes > 0,
        preload_cached_version,
        preload_cached_bytes,
        resources: game::optional::catalog(config, bundle)
            .map_err(|error| format!("读取可选资源状态失败: {error}"))?,
    })
}

/// 列举当前版本的目录, 同时给出公告中的预下载版本. 所选 tag 在核心边界校验.
pub fn game_resource_catalog(app: ServiceHandle) -> Result<ResourceCatalog, String> {
    let (settings, config, bundle) = load_catalog_context(&app)?;
    catalog_payload(&settings, &config, &bundle)
}

pub fn game_resource_select(
    app: ServiceHandle,
    tags: Vec<String>,
) -> Result<ResourceCatalog, String> {
    let (settings, config, bundle) = load_catalog_context(&app)?;
    {
        let inner = app
            .downloads
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行, 暂时无法修改可选资源".into());
        }
    }
    let _install_lock = nte_core::install_lock::InstallLock::acquire(&config.install_root)
        .map_err(|error| error.to_string())?;
    {
        // 拿到安装锁后再确认一次, 避免与刚启动的任务交错.
        let inner = app
            .downloads
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行, 暂时无法修改可选资源".into());
        }
    }
    {
        let game = app.state.game.lock().map_err(|error| error.to_string())?;
        if game.starting || game.session.as_ref().is_some_and(session_alive) {
            return Err("游戏正在运行, 暂时无法修改可选资源".into());
        }
    }
    game::optional::save_selection(&config, &bundle, &tags)
        .map_err(|error| format!("保存可选资源失败: {error}"))?;
    catalog_payload(&settings, &config, &bundle)
}

#[derive(Default)]
pub(crate) struct DownloadInner {
    pub(crate) status: DownloadStatus,
    cancel: Option<Arc<AtomicBool>>,
    last_progress: Option<Instant>,
    /// 最近一次成功应用任务的实测吞吐, 含 app_id, 用于下次检查估算完成时间.
    throughput: Option<(String, Throughput)>,
}

#[derive(Default)]
pub struct DownloadManager {
    pub(crate) inner: Mutex<DownloadInner>,
}

pub(crate) fn busy(phase: &str) -> bool {
    matches!(phase, "checking" | "launcher" | "game")
}

pub(crate) fn writes_launcher_files(mode: &str) -> bool {
    mode == "apply" || mode == "launcher"
}

/// 启动器更新是写操作, 单靠大小比对会漏掉同尺寸的内容变更.
fn launcher_plan_full(mode: &str, requested: bool) -> bool {
    requested || mode == "launcher"
}

pub(crate) fn blocks_changes(status: &DownloadStatus) -> bool {
    busy(&status.phase) && status.mode != "check"
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn publish(app: &ServiceHandle, change: impl FnOnce(&mut DownloadStatus)) {
    let manager = &app.downloads;
    // 进度事件必须在持锁时发送, 否则多线程并行下载时事件可能乱序,
    // 界面会看到已完成的文件数倒退。
    let Ok(mut inner) = manager.inner.lock() else {
        return;
    };
    change(&mut inner.status);
    let status = inner.status.clone();
    let _ = app.emit("game-download-state", status);
}

fn publish_bytes(app: &ServiceHandle, bytes: u64) {
    let manager = &app.downloads;
    let Ok(mut inner) = manager.inner.lock() else {
        return;
    };
    inner.status.downloaded_bytes += bytes;
    let now = Instant::now();
    if inner
        .last_progress
        .is_some_and(|last| now.duration_since(last) < Duration::from_millis(150))
    {
        return;
    }
    inner.last_progress = Some(now);
    let status = inner.status.clone();
    let _ = app.emit("game-download-state", status);
}

fn refresh_disk_available(app: &ServiceHandle, status: &mut DownloadStatus) {
    let root = Path::new(&status.install_root);
    let path = if status.mode == "preload" {
        let Ok(settings) = app.state.settings.lock() else {
            status.disk_available_bytes = None;
            return;
        };
        preload_root_for(&settings, root)
    } else {
        root.to_path_buf()
    };
    status.disk_available_bytes = disk::available_bytes(&path).ok();
}

fn publish_cancelled(app: &ServiceHandle) {
    publish(app, |status| {
        refresh_disk_available(app, status);
        status.phase = "cancelled".into();
        if status.mode == "preload" {
            status.checked_at = now_ms();
        }
    });
}

/// 当前游戏可用的吞吐估计; 没有历史样本时退回到该游戏的默认值。
fn current_throughput(current: Option<&(String, Throughput)>, app_id: &str) -> Throughput {
    match current {
        Some((id, throughput)) if id == app_id => *throughput,
        _ => Throughput::for_app_id(app_id),
    }
}

/// 用本次任务的实测耗时校准估计; 切换游戏时从新游戏的默认值重新累积。
fn next_throughput(
    current: Option<&(String, Throughput)>,
    app_id: &str,
    timings: &StageTimings,
) -> Option<(String, Throughput)> {
    if timings.is_empty() {
        // 没有有效样本时保留已有估计, 不用默认值覆盖已校准的结果。
        return current.cloned();
    }
    let mut throughput = current_throughput(current, app_id);
    throughput.observe(timings);
    Some((app_id.to_string(), throughput))
}

/// 规划按用户限速封顶下载速度; 限速为 0 表示不限速, 保持原估计。
fn cap_download_throughput(mut throughput: Throughput, limit_bytes_per_second: u64) -> Throughput {
    if limit_bytes_per_second > 0 {
        throughput.download_bytes_per_second = throughput
            .download_bytes_per_second
            .min(limit_bytes_per_second as f64)
            .max(1.0);
    }
    throughput
}

fn check_game(
    app: &ServiceHandle,
    settings: &Settings,
    root: &Path,
    full: bool,
    cancel: &AtomicBool,
) -> Result<game::UpdatePlan, String> {
    let config =
        LauncherConfig::load(root).map_err(|error| format!("读取启动器配置失败: {error}"))?;
    let mut game_config = game::GameUpdateConfig::load(root, config.game_id)
        .map_err(|error| format!("读取游戏资源配置失败: {error}"))?;
    apply_preload_root(&mut game_config, settings);
    // 锁只用于复制估计, 资源检查可能持续数秒, 不能持锁等待。
    let throughput = app
        .downloads
        .inner
        .lock()
        .map(|inner| current_throughput(inner.throughput.as_ref(), &game_config.app_id))
        .unwrap_or_else(|_| Throughput::for_app_id(&game_config.app_id));
    // 限速会拖长实际下载, 规划必须按受限后的速度估算, 否则可能误判整包更快。
    let throughput = cap_download_throughput(
        throughput,
        settings.current().updates.limit_bytes_per_second,
    );
    let plan = game::check::check_cancellable_with_estimates(
        &game_config,
        CheckOptions {
            full,
            optional: OptionalSelection::Configured,
        },
        Some(&throughput),
        &|| cancel.load(Ordering::Relaxed),
    )
    .map_err(|error| format!("检查游戏资源失败: {error}"))?;
    publish_game_plan(app, settings, root, &plan);
    Ok(plan)
}

fn publish_game_plan(
    app: &ServiceHandle,
    settings: &Settings,
    root: &Path,
    plan: &game::UpdatePlan,
) {
    let patch_files = plan.items.iter().filter(|item| item.has_patch()).count();
    let patch_bytes: u64 = plan
        .items
        .iter()
        .filter(|item| item.has_patch())
        .map(|item| item.download_size)
        .sum();
    let total_bytes = plan.total_bytes();
    let disk_available_bytes = disk::available_bytes(root).ok();
    let preload_available_bytes = disk::available_bytes(&preload_root_for(
        settings,
        root.parent().expect("启动器目录属于安装目录"),
    ))
    .ok();
    let disk_required_bytes = plan.required_disk_bytes();
    publish(app, |status| {
        status.distribution = settings.current().distribution.as_str().into();
        status.remote_version = plan.remote_version.clone();
        status.optional_tags = plan
            .optional_versions
            .iter()
            .map(|base| base.tag.clone())
            .collect();
        status.unverified_files = plan.unverified.len();
        status.pending_files = plan.items.len();
        status.total_bytes = total_bytes;
        status.total_files = plan.items.len();
        status.patch_files = patch_files;
        status.patch_bytes = patch_bytes;
        status.full_files = plan.items.len().saturating_sub(patch_files);
        status.full_bytes = total_bytes.saturating_sub(patch_bytes);
        status.disk_available_bytes = if status.mode == "preload" {
            preload_available_bytes
        } else {
            disk_available_bytes
        };
        status.disk_required_bytes = if status.mode == "preload" {
            total_bytes
        } else {
            disk_required_bytes
        };
        status.checked_at = now_ms();
    });
}

fn run_task(
    app: &ServiceHandle,
    settings: &Settings,
    mode: &str,
    full: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let install_root = selected_install_root(settings);
    let _install_lock = if mode != "check" {
        Some(
            nte_core::install_lock::InstallLock::acquire(&install_root)
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    if mode == "preload" {
        return run_preload_task(app, settings, full, cancel);
    }
    let launcher_plan = install::plan_launcher_cancellable(
        settings.current().distribution,
        &install_root,
        launcher_plan_full(mode, full),
        &|| cancel.load(Ordering::Relaxed),
    )
    .map_err(|error| format!("检查启动器文件失败: {error}"))?;
    // 启动器文件总是完整校验; 游戏资源沿用请求或上一轮保留的校验深度.
    run_plan(app, settings, mode, full, cancel, launcher_plan)
}

fn run_preload_task(
    app: &ServiceHandle,
    settings: &Settings,
    full: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let root = selected_install_root(settings).join(crate::current_launcher_dir(settings));
    let launcher =
        LauncherConfig::load(&root).map_err(|error| format!("读取现有启动器配置失败: {error}"))?;
    let mut config = game::GameUpdateConfig::load(&root, launcher.game_id)
        .map_err(|error| format!("读取游戏资源配置失败: {error}"))?;
    apply_preload_root(&mut config, settings);
    let preload = game::check::load_preload_remote(&config)
        .map_err(|error| format!("检查资源预下载失败: {error}"))?;
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled(app);
        return Ok(());
    }
    let Some(preload) = preload else {
        publish(app, |status| {
            status.phase = "preload-unavailable".into();
            status.checked_at = now_ms();
        });
        return Ok(());
    };
    // 正式版本已经追平公告后不再执行预下载.
    if nte_core::version::compare(&preload.bundle.config.res_version, &preload.current_version)
        != std::cmp::Ordering::Greater
    {
        publish(app, |status| {
            status.phase = "preload-unavailable".into();
            status.checked_at = now_ms();
        });
        return Ok(());
    }
    let completed = game::preload::completion(
        &preload.config,
        &preload.config.branch,
        &preload.bundle.config.res_version,
    )
    .is_some();
    publish(app, |status| status.verify = completed);
    let mut options = CheckOptions {
        full,
        optional: OptionalSelection::Configured,
    };
    let mut plan = game::check::check_bundle_cancellable(
        &preload.config,
        &preload.bundle,
        options.clone(),
        &|| cancel.load(Ordering::Relaxed),
    )
    .map_err(|error| format!("规划资源预下载失败: {error}"))?;
    if !plan.unverified.is_empty() {
        options.full = true;
        publish(app, |status| status.full = true);
        plan = game::check::check_bundle_cancellable(
            &preload.config,
            &preload.bundle,
            options,
            &|| cancel.load(Ordering::Relaxed),
        )
        .map_err(|error| format!("校验预下载基础资源失败: {error}"))?;
    }
    publish_game_plan(app, settings, &root, &plan);
    apply_game_plan(app, settings, cancel, &root, &preload.config, &plan, true)
}

/// 启动器文件大多是几十到几百 KiB 的 ZIP, 逐个下载会被握手和首字节
/// 延迟拖慢; 这里使用与游戏资源一致的固定并发。
const LAUNCHER_WORKERS: usize = 5;

fn install_launcher_files(
    app: &ServiceHandle,
    plan: &install::LauncherPlan,
    limit_bytes_per_second: u64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let files = &plan.files;
    let root = &plan.root;
    let zip_bases = &plan.zip_bases;
    let limiter = RateLimiter::new(limit_bytes_per_second);
    let limiter = &limiter;
    let workers = LAUNCHER_WORKERS.min(files.len()).max(1);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let first_error = Mutex::new(None::<String>);
    let next_ref = &next;
    let stop_ref = &stop;
    let first_error_ref = &first_error;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let app = app.clone();
            scope.spawn(move || loop {
                if cancel.load(Ordering::Relaxed) || stop_ref.load(Ordering::Relaxed) {
                    return;
                }
                let index = next_ref.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(index) else {
                    return;
                };
                publish(&app, |status| {
                    status.current_file = file.path.clone();
                    status.current_file_bytes = file.zip_size.unwrap_or(file.size);
                });
                let mut progress = |bytes| publish_bytes(&app, bytes);
                let outcome = update::apply_file_resumable_limited(
                    root,
                    zip_bases,
                    file,
                    limiter,
                    &mut progress,
                    &|| cancel.load(Ordering::Relaxed) || stop_ref.load(Ordering::Relaxed),
                );
                match outcome {
                    Ok(_) => publish(&app, |status| {
                        status.completed_files += 1;
                        status.pending_files = status.pending_files.saturating_sub(1);
                        if status.current_file == file.path {
                            status.current_file.clear();
                            status.current_file_bytes = 0;
                        }
                    }),
                    Err(nte_core::Error::Cancelled) => return,
                    Err(error) => {
                        if let Ok(mut slot) = first_error_ref.lock() {
                            if slot.is_none() {
                                *slot = Some(format!("安装启动器文件 {} 失败: {error}", file.path));
                            }
                        }
                        stop_ref.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            });
        }
    });
    if let Some(error) = first_error.into_inner().unwrap_or(None) {
        return Err(error);
    }
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled(app);
        return Ok(());
    }
    Ok(())
}

fn run_plan(
    app: &ServiceHandle,
    settings: &Settings,
    mode: &str,
    full: bool,
    cancel: &AtomicBool,
    launcher_plan: install::LauncherPlan,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled(app);
        return Ok(());
    }
    publish(app, |status| {
        status.pending_files = launcher_plan.file_count();
        status.total_files = launcher_plan.file_count();
        status.total_bytes = launcher_plan.download_bytes;
        status.completed_files = 0;
        status.downloaded_bytes = 0;
        status.current_file.clear();
        status.current_file_bytes = 0;
        status.patch_files = 0;
        status.patch_bytes = 0;
        status.full_files = launcher_plan.file_count();
        status.full_bytes = launcher_plan.download_bytes;
        status.disk_required_bytes = launcher_plan.required_disk_bytes();
        status.stage = "download".into();
        status.error.clear();
    });
    if mode == "check" && !launcher_plan.files.is_empty() {
        publish(app, |status| status.phase = "needs-install".into());
        return Ok(());
    }
    if writes_launcher_files(mode) && !launcher_plan.files.is_empty() {
        publish(app, |status| status.phase = "launcher".into());
        install_launcher_files(
            app,
            &launcher_plan,
            settings.current().updates.limit_bytes_per_second,
            cancel,
        )?;
    }
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled(app);
        return Ok(());
    }
    publish(app, |status| {
        status.phase = "checking".into();
        status.current_file.clear();
        status.current_file_bytes = 0;
        status.completed_files = 0;
        status.total_files = 0;
        status.pending_files = 0;
        status.downloaded_bytes = 0;
        status.total_bytes = 0;
        status.patch_files = 0;
        status.patch_bytes = 0;
        status.full_files = 0;
        status.full_bytes = 0;
        status.disk_required_bytes = 0;
        status.stage.clear();
    });
    let mut plan = check_game(app, settings, &launcher_plan.root, full, cancel)?;
    // 应用任务必须先确认未知资源，不能把仅大小相符的大文件视为已校验。
    if mode == "apply" && !plan.unverified.is_empty() && !cancel.load(Ordering::Relaxed) {
        publish(app, |status| status.full = true);
        plan = check_game(app, settings, &launcher_plan.root, true, cancel)?;
    }
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled(app);
        return Ok(());
    }
    if plan.items.is_empty() && game_exe_for(&launcher_plan.root).is_none() {
        return Err("资源清单未发现待下载文件，但本地缺少 HTGame.exe".into());
    }
    if mode == "check" || mode == "launcher" {
        publish(app, |status| {
            status.phase = if !plan.unverified.is_empty() {
                "needs-verify"
            } else if plan.items.is_empty() {
                "ready"
            } else {
                "needs-update"
            }
            .into();
        });
        return Ok(());
    }
    let launcher_config = LauncherConfig::load(&launcher_plan.root)
        .map_err(|error| format!("读取启动器配置失败: {error}"))?;
    let mut game_config =
        game::GameUpdateConfig::load(&launcher_plan.root, launcher_config.game_id)
            .map_err(|error| format!("读取游戏资源配置失败: {error}"))?;
    apply_preload_root(&mut game_config, settings);
    apply_game_plan(
        app,
        settings,
        cancel,
        &launcher_plan.root,
        &game_config,
        &plan,
        false,
    )
}

/// 删除公告版本的预下载缓存; 正式游戏文件和安装记录不受影响.
pub fn game_preload_delete(app: ServiceHandle) -> Result<ResourceCatalog, String> {
    let (settings, config, bundle) = load_catalog_context(&app)?;
    let target = match bundle.config.preload.as_ref() {
        Some(preload)
            if game::preload::cache_bytes(&config, &preload.branch, &preload.version) > 0 =>
        {
            (preload.branch.clone(), preload.version.clone())
        }
        _ => {
            let cached = game::preload::cached_version(&config)
                .ok_or_else(|| "当前没有可删除的资源预下载".to_string())?;
            (cached.branch, cached.version)
        }
    };
    {
        let inner = app
            .downloads
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行, 暂时无法删除资源预下载".into());
        }
    }
    let _install_lock = nte_core::install_lock::InstallLock::acquire(&config.install_root)
        .map_err(|error| error.to_string())?;
    {
        let inner = app
            .downloads
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行, 暂时无法删除资源预下载".into());
        }
    }
    game::preload::delete(&config, &target.0, &target.1)
        .map_err(|error| format!("删除资源预下载失败: {error}"))?;
    publish(&app, |status| {
        if status.mode == "preload"
            && !busy(&status.phase)
            && matches!(
                status.phase.as_str(),
                "preloaded" | "preload-unavailable" | "cancelled" | "failed" | "idle"
            )
        {
            status.phase = "idle".into();
            status.full = false;
            status.pending_files = 0;
            status.total_files = 0;
            status.total_bytes = 0;
            status.downloaded_bytes = 0;
            status.completed_files = 0;
            status.patch_files = 0;
            status.patch_bytes = 0;
            status.full_files = 0;
            status.full_bytes = 0;
            status.remote_version.clear();
            status.current_file.clear();
            status.current_file_bytes = 0;
            status.stage.clear();
            status.optional_tags.clear();
            status.verify = false;
            status.checked_at = 0;
            status.error.clear();
        }
    });
    catalog_payload(&settings, &config, &bundle)
}

fn mark_preload_complete(
    config: &game::GameUpdateConfig,
    plan: &game::UpdatePlan,
) -> Result<(), String> {
    game::preload::mark_complete(
        config,
        &config.branch,
        &plan.remote_version,
        plan.items.len(),
        plan.total_bytes(),
    )
    .map_err(|error| format!("写入预下载完成记录失败: {error}"))
}

fn apply_game_plan(
    app: &ServiceHandle,
    settings: &Settings,
    cancel: &AtomicBool,
    root: &Path,
    game_config: &game::GameUpdateConfig,
    plan: &game::UpdatePlan,
    preload: bool,
) -> Result<(), String> {
    // 预下载没有待下载文件时只校验缓存, 不进入下载阶段.
    if preload && plan.items.is_empty() {
        mark_preload_complete(game_config, plan)?;
        publish(app, |status| {
            status.phase = "preloaded".into();
            status.completed_files = status.total_files;
            status.pending_files = 0;
            status.auto_waiting.clear();
            status.checked_at = now_ms();
            status.current_file.clear();
            status.current_file_bytes = 0;
            status.stage.clear();
        });
        return Ok(());
    }
    publish(app, |status| {
        status.phase = "game".into();
        status.completed_files = 0;
        status.downloaded_bytes = 0;
        status.pending_files = plan.items.len();
        status.total_files = plan.items.len();
        status.stage = "download".into();
    });
    let mut on_file = |item: &game::UpdateItem| {
        publish(app, |status| {
            status.current_file = item.path.clone();
            status.current_file_bytes = item.size;
            status.stage = "download".into();
        });
    };
    let mut on_bytes = |_: &str, bytes: u64| {
        publish_bytes(app, bytes);
    };
    let mut on_stage = |path: &str, stage: game::download::ApplyStage| {
        publish(app, |status| {
            status.current_file = path.into();
            status.current_file_bytes = plan
                .items
                .iter()
                .find(|item| item.path == path)
                .map_or(0, |item| item.size);
            status.stage = stage.as_str().into();
        });
    };
    let mut on_result = |result: &game::download::ApplyResult| {
        publish(app, |status| {
            if result.ok {
                status.completed_files += 1;
            }
            status.pending_files = status.pending_files.saturating_sub(1);
            if status.current_file == result.path {
                status.current_file.clear();
                status.current_file_bytes = 0;
                status.stage.clear();
            }
            if status.pending_files == 0 {
                status.stage.clear();
            }
        });
    };
    let apply_options = ApplyOptions {
        probe_cdns: !updates::metered().is_metered(),
        preload,
        ..ApplyOptions::default()
    };
    let report = game::download::apply_plan_observed_full(
        game_config,
        plan,
        &apply_options,
        settings.current().updates.limit_bytes_per_second,
        &mut on_file,
        &mut on_bytes,
        &mut on_stage,
        &mut on_result,
        &|| cancel.load(Ordering::Relaxed),
    );
    // 取消或失败的任务样本不完整, 只在校验与安装都完成时校准。
    if !report.cancelled && report.failed == 0 && report.ledger_error.is_none() {
        // 锁只覆盖估计更新, 不跨后续发布与错误判断。
        if let Ok(mut inner) = app.downloads.inner.lock() {
            let next = next_throughput(
                inner.throughput.as_ref(),
                &game_config.app_id,
                &report.timings,
            );
            inner.throughput = next;
        }
    }
    if report.cancelled {
        publish_cancelled(app);
        return Ok(());
    }
    if report.failed > 0 {
        let first = report
            .results
            .iter()
            .find_map(|item| item.error.as_ref())
            .cloned()
            .unwrap_or_default();
        return Err(format!("{} 个游戏文件下载失败: {first}", report.failed));
    }
    if let Some(error) = report.ledger_error {
        return Err(format!("写入游戏资源记录失败: {error}"));
    }
    if !preload && game_exe_for(root).is_none() {
        return Err("资源下载已结束，但未找到游戏可执行文件".into());
    }
    if preload {
        mark_preload_complete(game_config, plan)?;
    }
    publish(app, |status| {
        status.phase = if preload { "preloaded" } else { "ready" }.into();
        status.completed_files = status.total_files;
        status.pending_files = 0;
        status.auto_waiting.clear();
        status.checked_at = now_ms();
        status.current_file.clear();
        status.current_file_bytes = 0;
        status.stage.clear();
    });
    Ok(())
}

pub fn game_download_status(manager: &DownloadManager) -> DownloadStatus {
    manager
        .inner
        .lock()
        .map(|inner| inner.status.clone())
        .unwrap_or_default()
}

fn prepare_task(
    state: &AppState,
    manager: &DownloadManager,
    mode: &str,
    full: bool,
    origin: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(DownloadStatus, Settings, bool), String> {
    if !["check", "apply", "launcher", "preload"].contains(&mode) {
        return Err("无效的游戏资源操作".into());
    }
    let (status, settings, full) = {
        let mut inner = manager.inner.lock().map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行".into());
        }
        let mut settings = state
            .settings
            .lock()
            .map_err(|error| error.to_string())?
            .clone();
        // 还没有安装配方的游戏不能继续规划下载, 否则会按 NTE 的布局写入错误目录.
        if settings
            .active_game
            .profile()
            .region(settings.current().distribution)
            .is_none()
        {
            return Err(format!(
                "《{}》尚未支持{}地区, 暂时无法检查或更新游戏资源.",
                settings.active_game.profile().name,
                settings.current().distribution.label()
            ));
        }
        // 未配置目录时沿用环境探测结果, 只用于本轮任务。
        if crate::configured_install_root(&settings).is_none() {
            if let Some(root) = crate::detect_launcher_root(&settings)
                .and_then(|root| root.parent().map(Path::to_path_buf))
            {
                let root = Some(root.display().to_string());
                match settings.current().distribution {
                    nte_core::distribution::Distribution::Cn => {
                        settings.current_mut().install_root = root
                    }
                    nte_core::distribution::Distribution::Global => {
                        settings.current_mut().global_install_root = root
                    }
                }
            }
        }
        {
            let game = state.game.lock().map_err(|error| error.to_string())?;
            if game.environment_busy {
                return Err("运行环境任务正在进行, 暂时无法检查或更新资源".into());
            }
            if mode != "check"
                && (game.starting || game.session.as_ref().is_some_and(session_alive))
            {
                return Err(if mode == "preload" {
                    "游戏正在运行, 请退出游戏后开始资源预下载"
                } else {
                    "游戏正在运行, 无法修改游戏文件"
                }
                .into());
            }
        }
        let install_root = selected_install_root(&settings);
        let disk_available_bytes = if mode == "preload" {
            disk::available_bytes(&preload_root_for(&settings, &install_root)).ok()
        } else {
            disk::available_bytes(&install_root).ok()
        };
        let installed = install_root
            .join(crate::current_launcher_dir(&settings))
            .join("UserData/Patcher/PatcherSDK/ResList.xml")
            .is_file();
        let install_root = install_root.display().to_string();
        let full = full
            || (inner.status.full
                && (inner.status.mode == "preload") == (mode == "preload")
                && inner.status.phase != "ready"
                && inner.status.distribution == settings.current().distribution.as_str()
                && inner.status.install_root == install_root);
        inner.cancel = Some(cancel.clone());
        inner.last_progress = None;
        inner.status = DownloadStatus {
            phase: "checking".into(),
            mode: mode.into(),
            origin: origin.into(),
            distribution: settings.current().distribution.as_str().into(),
            install_root,
            installed,
            full,
            disk_available_bytes,
            ..DownloadStatus::default()
        };
        (inner.status.clone(), settings, full)
    };
    Ok((status, settings, full))
}

/// 启动一轮资源检查或下载, 命令与自动调度共用.
pub(crate) fn start_task(
    app: &ServiceHandle,
    mode: &str,
    full: bool,
    origin: &str,
) -> Result<DownloadStatus, String> {
    let state = &app.state;
    let manager = &app.downloads;
    let cancel = Arc::new(AtomicBool::new(false));
    let (status, settings, full) = prepare_task(state, manager, mode, full, origin, &cancel)?;
    let app = app.clone();
    let mode = mode.to_string();
    let _ = app.emit("game-download-state", status.clone());
    crate::service::spawn_blocking(move || {
        let result = run_task(&app, &settings, &mode, full, &cancel);
        if let Err(error) = result {
            let cancelled = cancel.load(Ordering::Relaxed);
            if !cancelled {
                eprintln!("资源任务失败: {error}");
            }
            publish(&app, |status| {
                refresh_disk_available(&app, status);
                status.phase = if cancelled { "cancelled" } else { "failed" }.into();
                status.error = if cancelled { String::new() } else { error };
                if status.mode == "preload" {
                    status.checked_at = now_ms();
                }
                status.stage.clear();
            });
        }
        if let Ok(mut inner) = app.downloads.inner.lock() {
            if inner
                .cancel
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &cancel))
            {
                inner.cancel = None;
            }
        }
    });
    Ok(status)
}

pub fn game_download_start(
    app: ServiceHandle,
    mode: String,
    full: bool,
) -> Result<DownloadStatus, String> {
    start_task(&app, &mode, full, "manual")
}

/// 记录自动下载的等待原因, 供界面解释为什么暂未开始.
pub(crate) fn set_auto_waiting(app: &ServiceHandle, reason: &str) {
    let manager = &app.downloads;
    let Ok(mut inner) = manager.inner.lock() else {
        return;
    };
    if inner.status.auto_waiting == reason {
        return;
    }
    inner.status.auto_waiting = reason.to_string();
    let status = inner.status.clone();
    let _ = app.emit("game-download-state", status);
}

/// 请求取消当前任务, 自动调度在暂停时使用.
pub(crate) fn cancel_task(app: &ServiceHandle) {
    let manager = &app.downloads;
    let guard = manager.inner.lock();
    if let Ok(inner) = guard {
        if let Some(cancel) = &inner.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// 当前下载状态的快照, 自动调度据此决定下一步.
pub(crate) fn snapshot(app: &ServiceHandle) -> Option<DownloadStatus> {
    let manager = &app.downloads;
    let inner = manager.inner.lock().ok()?;
    Some(inner.status.clone())
}

pub fn game_download_cancel(manager: &DownloadManager) -> DownloadStatus {
    if let Ok(inner) = manager.inner.lock() {
        if let Some(cancel) = &inner.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        inner.status.clone()
    } else {
        DownloadStatus::default()
    }
}

#[cfg(test)]
#[path = "tests/download.rs"]
mod tests;

#[cfg(test)]
mod throughput_tests {
    use super::*;

    fn download_sample(bytes: u64, nanos: u64) -> StageTimings {
        StageTimings {
            download_bytes: bytes,
            download_nanos: nanos,
            ..StageTimings::default()
        }
    }

    fn custom_throughput(download: f64) -> Throughput {
        Throughput {
            download_bytes_per_second: download,
            apply_bytes_per_second: 2.0,
            verify_bytes_per_second: 3.0,
        }
    }

    #[test]
    fn next_throughput_calibrates_same_app_id() {
        let current = ("1289".to_string(), custom_throughput(1_000.0));
        // 2 秒下载 40 MB, 速率远高于旧估计, 应明显上调。
        let (app_id, throughput) = next_throughput(
            Some(&current),
            "1289",
            &download_sample(40_000_000, 2_000_000_000),
        )
        .unwrap();
        assert_eq!(app_id, "1289");
        let expected = 0.3 * 20_000_000.0 + 0.7 * 1_000.0;
        assert!((throughput.download_bytes_per_second - expected).abs() < 1.0);
        // 样本里没有应用与校验数据, 这两项保持原值。
        assert_eq!(throughput.apply_bytes_per_second, 2.0);
        assert_eq!(throughput.verify_bytes_per_second, 3.0);
    }

    #[test]
    fn next_throughput_resets_for_different_app_id() {
        let current = ("1289".to_string(), custom_throughput(100_000_000.0));
        let sample = download_sample(40_000_000, 2_000_000_000);
        let (app_id, throughput) = next_throughput(Some(&current), "3000001", &sample).unwrap();
        assert_eq!(app_id, "3000001");
        // 切换游戏后从国际服默认值重新累积, 不沿用旧游戏的估计。
        let mut expected = Throughput::for_app_id("3000001");
        expected.observe(&sample);
        assert_eq!(throughput, expected);
    }

    #[test]
    fn next_throughput_keeps_estimate_without_samples() {
        let current = ("1289".to_string(), custom_throughput(1_000.0));
        assert_eq!(
            next_throughput(Some(&current), "1289", &StageTimings::default()),
            Some(current.clone())
        );
        assert_eq!(
            next_throughput(None, "1289", &StageTimings::default()),
            None
        );
    }

    #[test]
    fn current_throughput_uses_matching_estimate_or_default() {
        let current = ("3000001".to_string(), custom_throughput(1_000.0));
        assert_eq!(current_throughput(Some(&current), "3000001"), current.1);
        assert_eq!(
            current_throughput(Some(&current), "1289"),
            Throughput::for_app_id("1289")
        );
        assert_eq!(
            current_throughput(None, "1289"),
            Throughput::for_app_id("1289")
        );
    }

    #[test]
    fn cap_download_throughput_keeps_estimate_without_limit() {
        let throughput = custom_throughput(50_000_000.0);
        assert_eq!(cap_download_throughput(throughput, 0), throughput);
    }

    #[test]
    fn cap_download_throughput_clamps_to_user_limit() {
        let throughput = custom_throughput(50_000_000.0);
        let capped = cap_download_throughput(throughput, 2_000_000);
        assert_eq!(capped.download_bytes_per_second, 2_000_000.0);
        // 应用与校验吞吐不受下载限速影响。
        assert_eq!(
            capped.apply_bytes_per_second,
            throughput.apply_bytes_per_second
        );
        assert_eq!(
            capped.verify_bytes_per_second,
            throughput.verify_bytes_per_second
        );
    }
}
