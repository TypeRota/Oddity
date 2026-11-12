//! 游戏资源下载命令: 目录查询, 可选资源增删, 检查/下载/预下载与状态.
//!
//! 下载状态常驻包内 (state.rs). 后台任务经宿主的 `submit` 运行, 并通过
//! 启动时绑定的 `Arc<dyn HostContext>` 发 `game-download-state`; 命令组本身
//! 不持有宿主实现, 也不关心宿主的具体类型.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use oddity_platform::disk;
use oddity_platform::install_lock::InstallLock;
use oddity_platform::network::metered;
use oddity_platform::throttle::RateLimiter;
use oddity_runtime::{
    ActivitySnapshot, HostContext, SettingsSnapshot, Task, TaskContext, TaskOutcome,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::check::CheckOptions;
use super::download::ApplyOptions;
use super::launcher::{self as launcher_update, LauncherConfig, LauncherPlan};
use super::optional::OptionalSelection;
use super::plan::{StageTimings, Throughput};
use super::region::{self, Region};
use super::state::{
    self, busy, launcher_plan_full, now_ms, publish, publish_bytes, publish_cancelled,
    refresh_disk_available, writes_launcher_files, DownloadManager, DownloadStatus,
    ResourceCatalog, ResourceGroup, ResourceTask,
};
use crate::install;
use crate::update as game;

/// 命令使用的宿主上下文: 绑定的宿主句柄 + 命令开始时的设置快照.
#[derive(Clone)]
pub(crate) struct Ctx {
    host: Arc<dyn HostContext>,
    settings: Settings,
}

/// 包装设置快照; 保留旧代码的 current()/active_game 访问形态.
#[derive(Clone, Debug)]
struct Settings(SettingsSnapshot);

impl Settings {
    fn current(&self) -> &SettingsSnapshot {
        &self.0
    }

    fn current_mut(&mut self) -> &mut SettingsSnapshot {
        &mut self.0
    }

    fn active_game(&self) -> &str {
        &self.0.game
    }

    fn distribution(&self) -> &str {
        &self.0.distribution
    }

    fn distribution_label(&self) -> &'static str {
        Region::parse(&self.0.distribution)
            .map(|region| region.label())
            .unwrap_or("未知地区")
    }

    fn region(&self) -> Region {
        Region::parse(&self.0.distribution).unwrap_or_default()
    }
}

impl Ctx {
    /// 由已绑定的宿主能力构造; 未绑定时给命令层返回明确错误.
    pub(crate) fn new() -> Result<Self, String> {
        let host = state::host()?;
        let settings = Settings(host.settings()?);
        Ok(Self { host, settings })
    }

    fn settings(&self) -> &Settings {
        &self.settings
    }

    fn emit_download_state(&self, payload: impl serde::Serialize) -> Result<(), String> {
        let payload = serde_json::to_value(payload).map_err(|error| error.to_string())?;
        self.host.emit("game-download-state", payload)
    }

    fn activity(&self) -> Result<ActivitySnapshot, String> {
        self.host.activity()
    }

    fn downloads(&self) -> &'static DownloadManager {
        state::manager()
    }

    /// 把阻塞任务交给宿主执行器; 返回句柄用于取消与回收.
    /// 把不需要返回值的阻塞任务交给宿主执行器.
    fn submit_detached(&self, job: impl FnOnce(&Ctx) + Send + 'static) -> Task {
        let ctx = self.clone();
        self.host.submit(Box::new(move |_task: TaskContext| {
            job(&ctx);
            TaskOutcome::Done(Value::Null)
        }))
    }

    fn selected_install_root(&self) -> PathBuf {
        install::selected_install_root_for(&self.settings.0)
    }

    fn configured_install_root(&self) -> Option<PathBuf> {
        install::configured_install_root_for(&self.settings.0)
    }

    fn detect_launcher_root(&self) -> Option<PathBuf> {
        self.host
            .official_launcher_root()
            .or_else(|| install::detect_launcher_root_for(&self.settings.0))
    }

    fn current_launcher_dir(&self) -> &'static str {
        install::launcher_dir_for(&self.settings.0)
    }

    fn game_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        install::game_exe_for(launcher_root)
    }
}

fn preload_root_for(settings: &Settings, install_root: &Path) -> PathBuf {
    let configured = settings
        .current()
        .preload_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    match configured {
        Some(path) if path.is_absolute() => path,
        Some(path) => install_root.join(path),
        None => region::LAYOUT
            .patcher_sdk_path(&install_root.join(launcher_dir_for(settings)))
            .join("preload"),
    }
}

/// 快照对应的官方启动器目录名; 只覆盖《异环》.
fn launcher_dir_for(settings: &Settings) -> &'static str {
    Region::parse(&settings.0.distribution)
        .map(|region| region.profile().launcher_dir)
        .unwrap_or("")
}
fn apply_preload_root(config: &mut game::GameUpdateConfig, settings: &Settings) {
    config.preload_root = Some(preload_root_for(settings, &config.install_root));
}

fn launcher_root_for(app: &Ctx) -> PathBuf {
    let settings = app.settings();
    let selected = install::selected_install_root_for(&settings.0);
    let launcher_dir = launcher_dir_for(settings);
    if install::configured_install_root_for(&settings.0).is_none() {
        app.detect_launcher_root()
            .unwrap_or_else(|| selected.join(launcher_dir))
    } else {
        selected.join(launcher_dir)
    }
}

fn load_launcher_config(
    settings: &Settings,
    root: &Path,
) -> oddity_platform::Result<LauncherConfig> {
    let config = LauncherConfig::load(root)?;
    if settings.active_game() != crate::PACKAGE_ID {
        return Err(oddity_platform::Error::Unsupported(format!(
            "当前游戏《{}》没有启动器更新布局",
            settings.active_game()
        )));
    }
    Ok(config)
}
fn load_catalog_context(
    app: &Ctx,
) -> Result<(Settings, game::GameUpdateConfig, game::check::RemoteBundle), String> {
    let settings = app.settings().clone();
    let root = launcher_root_for(app);
    let launcher = load_launcher_config(&settings, &root)
        .map_err(|error| format!("读取现有启动器配置失败: {error}"))?;
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
    let mut preload_resource_bytes = BTreeMap::new();
    if preload_cached_bytes > 0 && preload_available && preload_cached_version == preload_version {
        // 清单不可用时保留整版本清理, 不显示无法确认归属的逐语种入口.
        if let Ok(Some(future)) = game::check::load_preload_remote(config) {
            for base in &future.bundle.manifest.base_versions {
                if let Ok(bytes) =
                    game::preload::optional_cached_bytes(&future.config, &future.bundle, &base.tag)
                {
                    if bytes > 0 {
                        preload_resource_bytes.insert(base.tag.clone(), bytes);
                    }
                }
            }
        }
    }
    Ok(ResourceCatalog {
        distribution: settings.distribution().into(),
        install_root: config.install_root.display().to_string(),
        version: bundle.config.res_version.clone(),
        preload_version,
        preload_available,
        preload_completed,
        preload_cached: preload_cached_bytes > 0,
        preload_cached_version,
        preload_cached_bytes,
        preload_resource_bytes,
        resources: game::optional::catalog(config, bundle)
            .map_err(|error| format!("读取可选资源状态失败: {error}"))?,
    })
}

/// 列举当前版本的目录, 同时给出公告中的预下载版本. 所选 tag 在核心边界校验.
pub(crate) fn game_resource_catalog(app: &Ctx) -> Result<ResourceCatalog, String> {
    let (settings, config, bundle) = load_catalog_context(app)?;
    catalog_payload(&settings, &config, &bundle)
}

/// 资源删除与下载共用安装锁, 删除后丢弃旧的检查结果.
pub(crate) fn game_resource_remove(
    app: &Ctx,
    tag: String,
    preload: bool,
) -> Result<ResourceCatalog, String> {
    let (settings, config, bundle) = load_catalog_context(app)?;
    let future = if preload {
        Some(
            game::check::load_preload_remote(&config)
                .map_err(|error| format!("读取预下载资源清单失败: {error}"))?
                .ok_or("当前没有可删除的语音预下载资源")?,
        )
    } else {
        None
    };
    let resources = game::optional::catalog(&config, &bundle).map_err(|error| error.to_string())?;
    let _install_lock =
        InstallLock::acquire(&config.install_root).map_err(|error| error.to_string())?;
    let mut inner = app
        .downloads()
        .inner
        .lock()
        .map_err(|error| error.to_string())?;
    if busy(&inner.status.phase) {
        return Err("游戏资源任务正在进行, 暂时无法删除语音资源".into());
    }
    let activity = app.activity()?;
    if activity.game_starting || activity.game_running {
        return Err("游戏正在运行, 暂时无法删除语音资源".into());
    }
    if let Some(future) = future {
        let removed = game::preload::remove_optional(&future.config, &future.bundle, &tag)
            .map_err(|error| format!("删除语音预下载失败: {error}"))?;
        if removed == 0 {
            return Err("该语种没有可删除的预下载缓存".into());
        }
        let tags = resources
            .into_iter()
            .filter(|resource| resource.selected && resource.tag != tag)
            .map(|resource| resource.tag)
            .collect::<Vec<_>>();
        game::optional::save_selection(&config, &bundle, &tags)
            .map_err(|error| error.to_string())?;
    } else {
        game::optional::remove(&config, &bundle, &tag)
            .map_err(|error| format!("卸载语音资源失败: {error}"))?;
    }
    inner.status = DownloadStatus::default();
    let status = inner.status.clone();
    drop(inner);
    let _ = app.emit_download_state(status);
    catalog_payload(&settings, &config, &bundle)
}

/// 每个语种独立规划和执行, 同一安装目录的语种任务共享限速与安装锁.
pub(crate) fn game_resource_download(app: &Ctx, tag: String) -> Result<DownloadStatus, String> {
    let (settings, config, bundle) = load_catalog_context(app)?;
    let mut inner = app
        .downloads()
        .inner
        .lock()
        .map_err(|error| error.to_string())?;
    let activity = app.activity()?;
    if activity.environment_busy || activity.migration_busy || activity.migration_repair_pending {
        return Err("运行环境或安装目录任务正在进行, 暂时无法下载语音资源".into());
    }
    if activity.game_starting || activity.game_running {
        return Err("游戏正在运行, 请退出游戏后下载语音资源".into());
    }
    let joining = inner.resource_group.is_some();
    if busy(&inner.status.phase) && !joining {
        return Err("游戏资源任务正在进行".into());
    }
    if joining
        && (inner.status.install_root != config.install_root.display().to_string()
            || inner.status.distribution != settings.distribution()
            || inner.status.remote_version != bundle.config.res_version)
    {
        return Err("请等待当前安装目录的语音任务结束".into());
    }
    if joining
        && inner
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        return Err("语音资源任务正在停止".into());
    }
    if inner
        .status
        .resource_tasks
        .get(&tag)
        .is_some_and(ResourceTask::running)
    {
        return Err("该语音资源正在下载".into());
    }
    let install_lock = match &inner.resource_group {
        Some(group) => group.install_lock.clone(),
        None => {
            Arc::new(InstallLock::acquire(&config.install_root).map_err(|error| error.to_string())?)
        }
    };
    let resources = game::optional::catalog(&config, &bundle).map_err(|error| error.to_string())?;
    if !resources.iter().any(|resource| resource.tag == tag) {
        return Err(format!("未知的可选资源: {tag}"));
    }
    let tags = resources
        .into_iter()
        .filter(|resource| resource.selected || resource.tag == tag)
        .map(|resource| resource.tag)
        .collect::<Vec<_>>();
    game::optional::save_selection(&config, &bundle, &tags).map_err(|error| error.to_string())?;
    if !joining {
        let cancel = Arc::new(AtomicBool::new(false));
        inner.cancel = Some(cancel);
        inner.resource_group = Some(ResourceGroup {
            install_lock: install_lock.clone(),
            limiter: Arc::new(RateLimiter::with_connections(
                settings.current().update.limit_bytes_per_second,
                config.max_threads.clamp(1, 8),
            )),
            reserved: BTreeMap::new(),
        });
        inner.status = DownloadStatus {
            mode: "resources".into(),
            phase: "game".into(),
            distribution: settings.distribution().into(),
            install_root: config.install_root.display().to_string(),
            remote_version: bundle.config.res_version.clone(),
            installed: config.patcher_dir.join("ResList.xml").is_file(),
            ..Default::default()
        };
    }
    let group = inner.resource_group.as_mut().expect("语音任务组已建立");
    group.reserved.insert(tag.clone(), 0);
    let limiter = group.limiter.clone();
    let cancel = inner
        .cancel
        .as_ref()
        .expect("语音任务取消标识已建立")
        .clone();
    inner.status.resource_tasks.insert(
        tag.clone(),
        ResourceTask {
            phase: "checking".into(),
            downloaded_bytes: 0,
            total_bytes: 0,
            error: String::new(),
        },
    );
    inner.status.optional_tags = inner.status.resource_tasks.keys().cloned().collect();
    inner.status.phase = "game".into();
    inner.status.error.clear();
    let status = inner.status.clone();
    let _ = app.emit_download_state(status.clone());
    drop(inner);
    app.submit_detached(move |app| {
        let result = (|| -> Result<(), String> {
            let mut plan =
                game::optional::plan(&config, &bundle, &tag, &|| cancel.load(Ordering::Relaxed))
                    .map_err(|error| format!("检查语音资源失败: {error}"))?;
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            // 每个语种可使用完整并发度, 实际 HTTP 连接受任务组共享预算约束.
            plan.max_threads = config.max_threads;
            {
                let mut inner = app
                    .downloads()
                    .inner
                    .lock()
                    .map_err(|error| error.to_string())?;
                let available = disk::available_bytes(&config.install_root)
                    .map_err(|error| error.to_string())?;
                let group = inner.resource_group.as_mut().expect("运行中的语音任务组");
                let reserved = group
                    .reserved
                    .values()
                    .copied()
                    .fold(0u64, u64::saturating_add);
                let required = plan.required_disk_bytes();
                if required.saturating_add(reserved) > available {
                    return Err("磁盘空间不足, 无法同时下载所选语音资源".into());
                }
                group.reserved.insert(tag.clone(), required);
                inner.status.disk_available_bytes = Some(available);
                inner.status.disk_required_bytes = required.saturating_add(reserved);
                inner.status.total_files += plan.items.len();
                let task = inner
                    .status
                    .resource_tasks
                    .get_mut(&tag)
                    .expect("语音任务已登记");
                task.phase = "downloading".into();
                task.total_bytes = plan.total_bytes();
                inner.status.total_bytes = inner
                    .status
                    .resource_tasks
                    .values()
                    .map(|task| task.total_bytes)
                    .sum();
                let _ = app.emit_download_state(inner.status.clone());
            }
            let report = game::download::apply_plan_observed_shared(
                &config,
                &plan,
                &ApplyOptions {
                    update_ledger: false,
                    probe_cdns: false,
                    ..Default::default()
                },
                &limiter,
                Some(&tag),
                &mut |_| {},
                &mut |_, bytes| {
                    let Ok(mut inner) = app.downloads().inner.lock() else {
                        return;
                    };
                    if let Some(task) = inner.status.resource_tasks.get_mut(&tag) {
                        task.downloaded_bytes += bytes;
                    }
                    inner.status.downloaded_bytes = inner
                        .status
                        .resource_tasks
                        .values()
                        .map(|task| task.downloaded_bytes)
                        .sum();
                    let now = Instant::now();
                    if inner
                        .last_progress
                        .is_some_and(|last| now.duration_since(last) < Duration::from_millis(150))
                    {
                        return;
                    }
                    inner.last_progress = Some(now);
                    let _ = app.emit_download_state(inner.status.clone());
                },
                &mut |path, stage| {
                    publish(|status| {
                        status.current_file = path.into();
                        status.stage = stage.as_str().into();
                    })
                },
                &mut |result| {
                    if result.ok {
                        publish(|status| status.completed_files += 1);
                    }
                },
                &|| cancel.load(Ordering::Relaxed),
            );
            let successful = report
                .results
                .iter()
                .filter(|result| result.ok && !result.skipped)
                .map(|result| result.path.clone())
                .chain(plan.verified.iter().map(|item| item.path().to_string()))
                .collect::<Vec<_>>();
            {
                let _ledger = app
                    .downloads()
                    .resource_ledger
                    .lock()
                    .map_err(|error| error.to_string())?;
                game::ledger::update_after_apply(&config, &plan, &successful, false)
                    .map_err(|error| format!("写入语音资源记录失败: {error}"))?;
            }
            if report.failed > 0 || report.ledger_error.is_some() {
                let error = report
                    .ledger_error
                    .or_else(|| report.results.iter().find_map(|item| item.error.clone()))
                    .unwrap_or_default();
                return Err(format!("语音资源下载失败: {error}"));
            }
            Ok(())
        })();
        // 发布终态前释放本工作线程的锁引用; 最后一个任务同时释放任务组.
        drop(install_lock);
        if let Ok(mut inner) = app.downloads().inner.lock() {
            let cancelled = cancel.load(Ordering::Relaxed);
            if let Some(task) = inner.status.resource_tasks.get_mut(&tag) {
                task.phase = if cancelled {
                    "cancelled"
                } else if result.is_err() {
                    "failed"
                } else {
                    "ready"
                }
                .into();
                task.error = result.err().filter(|_| !cancelled).unwrap_or_default();
            }
            if let Some(group) = &mut inner.resource_group {
                group.reserved.remove(&tag);
            }
            inner.status.disk_required_bytes = inner
                .resource_group
                .as_ref()
                .map_or(0, |group| group.reserved.values().sum());
            if !inner
                .status
                .resource_tasks
                .values()
                .any(ResourceTask::running)
            {
                inner.resource_group = None;
                inner.cancel = None;
                inner.status.phase = if cancelled {
                    "cancelled"
                } else if inner
                    .status
                    .resource_tasks
                    .values()
                    .any(|task| task.phase == "failed")
                {
                    "failed"
                } else {
                    "ready"
                }
                .into();
                inner.status.error = inner
                    .status
                    .resource_tasks
                    .values()
                    .find(|task| task.phase == "failed")
                    .map(|task| task.error.clone())
                    .unwrap_or_default();
                inner.status.current_file.clear();
                inner.status.stage.clear();
            }
            let _ = app.emit_download_state(inner.status.clone());
        }
    });
    Ok(status)
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

/// 检查期间进度事件的最小间隔; 清单条目多时避免频繁刷新界面.
const CHECK_PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

fn check_game(
    app: &Ctx,
    settings: &Settings,
    root: &Path,
    full: bool,
    cancel: &AtomicBool,
) -> Result<game::UpdatePlan, String> {
    let config = load_launcher_config(settings, root)
        .map_err(|error| format!("读取启动器配置失败: {error}"))?;
    let mut game_config = game::GameUpdateConfig::load(root, config.game_id)
        .map_err(|error| format!("读取游戏资源配置失败: {error}"))?;
    apply_preload_root(&mut game_config, settings);
    // 锁只用于复制估计, 资源检查可能持续数秒, 不能持锁等待。
    let throughput = app
        .downloads()
        .inner
        .lock()
        .map(|inner| current_throughput(inner.throughput.as_ref(), &game_config.app_id))
        .unwrap_or_else(|_| Throughput::for_app_id(&game_config.app_id));
    // 限速会拖长实际下载, 规划必须按受限后的速度估算, 否则可能误判整包更快。
    let throughput =
        cap_download_throughput(throughput, settings.current().update.limit_bytes_per_second);
    // 回调可能远密于界面刷新间隔, 交给单独的发布线程保留最新值;
    // 开始哈希大文件时即使落在节流窗口内, 也会在间隔结束后补发.
    let (progress_tx, progress_rx) = mpsc::channel();
    let publisher = {
        std::thread::Builder::new()
            .name("oddity-check-progress".into())
            .spawn(move || publish_check_progress(&progress_rx))
            .map_err(|error| format!("启动检查进度线程失败: {error}"))?
    };
    let mut on_progress = move |progress: game::check::CheckProgress| {
        let _ = progress_tx.send(progress);
    };
    let result = game::check::check_cancellable_observed(
        &game_config,
        CheckOptions {
            full,
            optional: OptionalSelection::Configured,
        },
        Some(&throughput),
        &mut on_progress,
        &|| cancel.load(Ordering::Relaxed),
    );
    // 先结束检查再等发布线程收尾, 保证进度先于计划状态落库.
    drop(on_progress);
    let _ = publisher.join();
    let plan = result.map_err(|error| format!("检查游戏资源失败: {error}"))?;
    publish_game_plan(settings, root, &plan);
    Ok(plan)
}

/// 按固定间隔发布检查进度, 但始终补发最新值.
///
/// 只在回调里丢弃节流窗口内的事件会让界面停留在上一个文件, 而开始
/// 哈希大文件后可能很久没有新回调; 这里保留最新值, 在间隔结束后补发.
fn publish_check_progress(rx: &mpsc::Receiver<game::check::CheckProgress>) {
    let Ok(first) = rx.recv() else {
        return;
    };
    publish_check_progress_state(&first);
    let mut last = Instant::now();
    let mut pending: Option<game::check::CheckProgress> = None;
    loop {
        let wait = CHECK_PROGRESS_INTERVAL.saturating_sub(last.elapsed());
        match rx.recv_timeout(wait) {
            Ok(progress) => pending = Some(progress),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(progress) = pending.take() {
                    publish_check_progress_state(&progress);
                    last = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last.elapsed() >= CHECK_PROGRESS_INTERVAL {
            if let Some(progress) = pending.take() {
                publish_check_progress_state(&progress);
                last = Instant::now();
            }
        }
    }
    if let Some(progress) = pending {
        publish_check_progress_state(&progress);
    }
}

fn publish_check_progress_state(progress: &game::check::CheckProgress) {
    publish(|status| {
        status.total_files = progress.total;
        status.completed_files = progress.checked;
        status.current_file = progress.current.clone();
        status.current_file_bytes = 0;
        status.stage = "check".into();
    });
}

fn publish_game_plan(settings: &Settings, root: &Path, plan: &game::UpdatePlan) {
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
    publish(|status| {
        status.distribution = settings.distribution().into();
        status.remote_version = plan.remote_version.clone();
        status.optional_tags = plan
            .optional_versions
            .iter()
            .map(|base| base.tag.clone())
            .collect();
        status.unverified_files = plan.unverified.len();
        status.pending_files = plan.items.len();
        status.total_bytes = total_bytes;
        // 检查进度已经给出完成状态, 阶段切换前不要改写计数, 避免进度回跳.
        if status.stage != "check" {
            status.total_files = plan.items.len();
            status.completed_files = 0;
            status.current_file.clear();
            status.current_file_bytes = 0;
            status.stage.clear();
        }
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
    app: &Ctx,
    settings: &Settings,
    mode: &str,
    full: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let install_root = app.selected_install_root();
    let _install_lock = if mode != "check" {
        Some(InstallLock::acquire(&install_root).map_err(|error| error.to_string())?)
    } else {
        None
    };
    if mode == "preload" {
        return run_preload_task(app, settings, full, cancel);
    }
    let launcher_plan = launcher_update::plan_launcher_cancellable(
        settings.region(),
        &install_root,
        launcher_plan_full(mode, full),
        &|| cancel.load(Ordering::Relaxed),
    )
    .map_err(|error| format!("检查启动器文件失败: {error}"))?;
    // 启动器文件总是完整校验; 游戏资源沿用请求或上一轮保留的校验深度.
    run_plan(app, settings, mode, full, cancel, launcher_plan)
}

fn run_preload_task(
    app: &Ctx,
    settings: &Settings,
    full: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let root = app.selected_install_root().join(app.current_launcher_dir());
    let launcher = load_launcher_config(settings, &root)
        .map_err(|error| format!("读取现有启动器配置失败: {error}"))?;
    let mut config = game::GameUpdateConfig::load(&root, launcher.game_id)
        .map_err(|error| format!("读取游戏资源配置失败: {error}"))?;
    apply_preload_root(&mut config, settings);
    let preload = game::check::load_preload_remote(&config)
        .map_err(|error| format!("检查资源预下载失败: {error}"))?;
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled();
        return Ok(());
    }
    let Some(preload) = preload else {
        publish(|status| {
            status.phase = "preload-unavailable".into();
            status.checked_at = now_ms();
        });
        return Ok(());
    };
    // 正式版本已经追平公告后不再执行预下载.
    if oddity_platform::version::compare(
        &preload.bundle.config.res_version,
        &preload.current_version,
    ) != std::cmp::Ordering::Greater
    {
        publish(|status| {
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
    publish(|status| status.verify = completed);
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
        publish(|status| status.full = true);
        plan = game::check::check_bundle_cancellable(
            &preload.config,
            &preload.bundle,
            options,
            &|| cancel.load(Ordering::Relaxed),
        )
        .map_err(|error| format!("校验预下载基础资源失败: {error}"))?;
    }
    publish_game_plan(settings, &root, &plan);
    apply_game_plan(app, settings, cancel, &root, &preload.config, &plan, true)
}

/// 启动器文件大多是几十到几百 KiB 的 ZIP, 逐个下载会被握手和首字节
/// 延迟拖慢; 这里使用与游戏资源一致的固定并发。
const LAUNCHER_WORKERS: usize = 5;

fn install_launcher_files(
    plan: &LauncherPlan,
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
            scope.spawn(move || loop {
                if cancel.load(Ordering::Relaxed) || stop_ref.load(Ordering::Relaxed) {
                    return;
                }
                let index = next_ref.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(index) else {
                    return;
                };
                publish(|status| {
                    status.current_file = file.path.clone();
                    status.current_file_bytes = file.zip_size.unwrap_or(file.size);
                });
                let mut progress = |bytes| publish_bytes(bytes);
                let outcome = launcher_update::apply_file_resumable_limited(
                    root,
                    zip_bases,
                    file,
                    limiter,
                    &mut progress,
                    &|| cancel.load(Ordering::Relaxed) || stop_ref.load(Ordering::Relaxed),
                );
                match outcome {
                    Ok(_) => publish(|status| {
                        status.completed_files += 1;
                        status.pending_files = status.pending_files.saturating_sub(1);
                        if status.current_file == file.path {
                            status.current_file.clear();
                            status.current_file_bytes = 0;
                        }
                    }),
                    Err(oddity_platform::Error::Cancelled) => return,
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
        publish_cancelled();
        return Ok(());
    }
    Ok(())
}

fn run_plan(
    app: &Ctx,
    settings: &Settings,
    mode: &str,
    full: bool,
    cancel: &AtomicBool,
    launcher_plan: LauncherPlan,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled();
        return Ok(());
    }
    publish(|status| {
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
        publish(|status| status.phase = "needs-install".into());
        return Ok(());
    }
    if writes_launcher_files(mode) && !launcher_plan.files.is_empty() {
        publish(|status| status.phase = "launcher".into());
        install_launcher_files(
            &launcher_plan,
            settings.current().update.limit_bytes_per_second,
            cancel,
        )?;
    }
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled();
        return Ok(());
    }
    publish(|status| {
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
        publish(|status| status.full = true);
        plan = check_game(app, settings, &launcher_plan.root, true, cancel)?;
    }
    if cancel.load(Ordering::Relaxed) {
        publish_cancelled();
        return Ok(());
    }
    if plan.items.is_empty() && app.game_exe(&launcher_plan.root).is_none() {
        return Err("资源清单未发现待下载文件，但本地缺少游戏可执行文件".into());
    }
    if mode == "check" || mode == "launcher" {
        publish(|status| {
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
    let launcher_config = load_launcher_config(settings, &launcher_plan.root)
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
pub(crate) fn game_preload_delete(app: &Ctx) -> Result<ResourceCatalog, String> {
    let (settings, config, bundle) = load_catalog_context(app)?;
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
            .downloads()
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行, 暂时无法删除资源预下载".into());
        }
    }
    let _install_lock =
        InstallLock::acquire(&config.install_root).map_err(|error| error.to_string())?;
    {
        let inner = app
            .downloads()
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        if busy(&inner.status.phase) {
            return Err("游戏资源任务正在进行, 暂时无法删除资源预下载".into());
        }
    }
    game::preload::delete(&config, &target.0, &target.1)
        .map_err(|error| format!("删除资源预下载失败: {error}"))?;
    publish(|status| {
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
    app: &Ctx,
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
        publish(|status| {
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
    publish(|status| {
        status.phase = "game".into();
        status.completed_files = 0;
        status.downloaded_bytes = 0;
        status.pending_files = plan.items.len();
        status.total_files = plan.items.len();
        status.stage = "download".into();
    });
    let mut on_file = |item: &game::UpdateItem| {
        publish(|status| {
            status.current_file = item.path.clone();
            status.current_file_bytes = item.size;
            status.stage = "download".into();
        });
    };
    let mut on_bytes = |_: &str, bytes: u64| {
        publish_bytes(bytes);
    };
    let mut on_stage = |path: &str, stage: game::download::ApplyStage| {
        publish(|status| {
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
        publish(|status| {
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
        probe_cdns: !metered().is_metered(),
        preload,
        ..ApplyOptions::default()
    };
    let report = game::download::apply_plan_observed_full(
        game_config,
        plan,
        &apply_options,
        settings.current().update.limit_bytes_per_second,
        &mut on_file,
        &mut on_bytes,
        &mut on_stage,
        &mut on_result,
        &|| cancel.load(Ordering::Relaxed),
    );
    // 取消或失败的任务样本不完整, 只在校验与安装都完成时校准。
    if !report.cancelled && report.failed == 0 && report.ledger_error.is_none() {
        // 锁只覆盖估计更新, 不跨后续发布与错误判断。
        if let Ok(mut inner) = app.downloads().inner.lock() {
            let next = next_throughput(
                inner.throughput.as_ref(),
                &game_config.app_id,
                &report.timings,
            );
            inner.throughput = next;
        }
    }
    if report.cancelled {
        publish_cancelled();
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
    if !preload && app.game_exe(root).is_none() {
        return Err("资源下载已结束，但未找到游戏可执行文件".into());
    }
    if preload {
        mark_preload_complete(game_config, plan)?;
    }
    publish(|status| {
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

pub(crate) fn game_download_status() -> DownloadStatus {
    state::snapshot()
}
fn prepare_task(
    app: &Ctx,
    manager: &DownloadManager,
    mode: &str,
    full: bool,
    origin: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(DownloadStatus, Settings, bool), String> {
    if !["check", "apply", "launcher", "preload"].contains(&mode) {
        return Err("无效的游戏资源操作".into());
    }
    let activity = app.activity()?;
    let mut settings = app.settings().clone();
    let mut inner = manager.inner.lock().map_err(|error| error.to_string())?;
    if busy(&inner.status.phase) {
        return Err("游戏资源任务正在进行".into());
    }
    // 还没有更新布局的游戏不能继续规划下载, 否则会按 NTE 的布局写入错误目录.
    if settings.active_game() != crate::PACKAGE_ID {
        return Err(format!(
            "当前游戏尚未支持{}地区, 暂时无法检查或更新游戏资源.",
            settings.distribution_label()
        ));
    }
    // 未配置目录时沿用环境探测结果, 只用于本轮任务.
    if app.configured_install_root().is_none() {
        if let Some(root) = app
            .detect_launcher_root()
            .and_then(|root| root.parent().map(Path::to_path_buf))
        {
            settings.current_mut().install_root = Some(root.display().to_string());
        }
    }
    if activity.environment_busy {
        return Err("运行环境任务正在进行, 暂时无法检查或更新资源".into());
    }
    if mode != "check" && (activity.game_starting || activity.game_running) {
        return Err(if mode == "preload" {
            "游戏正在运行, 请退出游戏后开始资源预下载"
        } else {
            "游戏正在运行, 无法修改游戏文件"
        }
        .into());
    }
    let install_root = install::selected_install_root_for(&settings.0);
    let disk_available_bytes = if mode == "preload" {
        disk::available_bytes(&preload_root_for(&settings, &install_root)).ok()
    } else {
        disk::available_bytes(&install_root).ok()
    };
    let launcher_root = install_root.join(launcher_dir_for(&settings));
    let installed = region::LAYOUT
        .patcher_sdk_path(&launcher_root)
        .join("ResList.xml")
        .is_file();
    let install_root = install_root.display().to_string();
    let full = full
        || (inner.status.full
            && (inner.status.mode == "preload") == (mode == "preload")
            && inner.status.phase != "ready"
            && inner.status.distribution == settings.distribution()
            && inner.status.install_root == install_root);
    inner.cancel = Some(cancel.clone());
    inner.last_progress = None;
    inner.preload_root = Some(preload_root_for(&settings, Path::new(&install_root)));
    inner.status = DownloadStatus {
        phase: "checking".into(),
        mode: mode.into(),
        origin: origin.into(),
        distribution: settings.distribution().into(),
        install_root,
        installed,
        full,
        disk_available_bytes,
        ..DownloadStatus::default()
    };
    Ok((inner.status.clone(), settings, full))
}

/// 启动一轮资源检查或下载, 命令与自动调度共用.
pub(crate) fn start_task(
    app: &Ctx,
    mode: &str,
    full: bool,
    origin: &str,
) -> Result<DownloadStatus, String> {
    let activity = app.activity()?;
    if activity.migration_busy || activity.migration_repair_pending {
        return Err("安装目录迁移正在进行".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (status, settings, full) = prepare_task(app, app.downloads(), mode, full, origin, &cancel)?;
    let mode = mode.to_string();
    let _ = app.emit_download_state(status.clone());
    let task = app.submit_detached({
        let settings = settings.clone();
        let cancel = cancel.clone();
        move |app: &Ctx| {
            let result = run_task(app, &settings, &mode, full, &cancel);
            if let Err(error) = result {
                let cancelled = cancel.load(Ordering::Relaxed);
                if !cancelled {
                    eprintln!("资源任务失败: {error}");
                }
                state::publish_inner(|inner| {
                    let preload_root = inner.preload_root.clone();
                    refresh_disk_available(&mut inner.status, preload_root.as_deref());
                    inner.status.phase = if cancelled { "cancelled" } else { "failed" }.into();
                    inner.status.error = if cancelled { String::new() } else { error };
                    if inner.status.mode == "preload" {
                        inner.status.checked_at = now_ms();
                    }
                    inner.status.stage.clear();
                });
            }
            if let Ok(mut inner) = app.downloads().inner.lock() {
                if inner
                    .cancel
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &cancel))
                {
                    inner.cancel = None;
                }
            }
        }
    });
    state::set_task_id(Some(task.id().to_string()));
    Ok(status)
}
pub(crate) fn game_download_start(
    app: &Ctx,
    mode: String,
    full: bool,
) -> Result<DownloadStatus, String> {
    start_task(app, &mode, full, "manual")
}

/// 记录自动下载的等待原因, 供界面解释为什么暂未开始.
/// 记录自动下载的等待原因, 供界面解释为什么暂未开始.
pub(crate) fn set_auto_waiting(reason: &str) {
    let changed = {
        let Ok(mut inner) = state::manager().inner.lock() else {
            return;
        };
        if inner.status.auto_waiting == reason {
            return;
        }
        inner.status.auto_waiting = reason.to_string();
        inner.status.clone()
    };
    state::emit_status(&changed);
}
/// 请求取消当前任务, 自动调度与宿主关闭共用.
pub fn cancel_all() {
    state::cancel_current();
}

/// 取消命令(按命令组协议返回取消后的状态).
pub(crate) fn game_download_cancel(_app: &Ctx) -> DownloadStatus {
    state::cancel_current();
    state::snapshot()
}

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

/// 更新域下载命令组声明的命令 id.
pub const COMMANDS: &[&str] = &[
    "game_resource_catalog",
    "game_resource_remove",
    "game_resource_download",
    "game_preload_delete",
    "game_download_status",
    "game_download_start",
    "game_download_cancel",
];

/// 包内命令分发: 参数解析后交给下载实现, 阻塞工作移出异步线程.
pub async fn dispatch(
    _host: &dyn HostContext,
    command: &str,
    args: Value,
) -> std::result::Result<Value, Value> {
    let ctx = Ctx::new().map_err(|error| json!(error))?;
    match command {
        "game_resource_catalog" => blocking(move || game_resource_catalog(&ctx)).await,
        "game_preload_delete" => blocking(move || game_preload_delete(&ctx)).await,
        "game_resource_download" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                tag: String,
            }
            let Args { tag } =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            blocking(move || game_resource_download(&ctx, tag)).await
        }
        "game_resource_remove" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                tag: String,
                preload: bool,
            }
            let Args { tag, preload } =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            blocking(move || game_resource_remove(&ctx, tag, preload)).await
        }
        "game_download_status" => {
            serde_json::to_value(game_download_status()).map_err(|error| json!(error.to_string()))
        }
        "game_download_start" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                mode: String,
                full: bool,
            }
            let Args { mode, full } =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            blocking(move || game_download_start(&ctx, mode, full)).await
        }
        "game_download_cancel" => serde_json::to_value(game_download_cancel(&ctx))
            .map_err(|error| json!(error.to_string())),
        _ => Err(json!("无法识别的更新下载命令")),
    }
}

async fn blocking<T: Send + 'static + serde::Serialize>(
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> std::result::Result<Value, Value> {
    let result = tokio::task::spawn_blocking(job)
        .await
        .map_err(|error| json!(error.to_string()))?;
    let value = result.map_err(|error| json!(error))?;
    serde_json::to_value(value).map_err(|error| json!(error.to_string()))
}

/// 启动一轮下载/检查; 宿主需先 bind_host.
pub fn start_download(mode: &str, full: bool, origin: &str) -> Result<DownloadStatus, String> {
    let app = Ctx::new()?;
    start_task(&app, mode, full, origin)
}

/// 取消当前下载任务.
pub fn cancel_download() {
    state::cancel_current();
}

/// 当前下载状态快照.
pub fn download_status() -> DownloadStatus {
    game_download_status()
}

/// 记录自动下载的等待原因.
pub fn set_auto_waiting_reason(reason: &str) {
    set_auto_waiting(reason);
}

#[cfg(test)]
mod testing_tests {
    use super::*;
    use crate::update::testing::{bind, reset, serial, FakeHost};
    use oddity_runtime::ActivitySnapshot;

    fn host() -> std::sync::Arc<FakeHost> {
        let host = FakeHost::new();
        host.set_launcher_root(Some(PathBuf::from("/missing/NTELauncher")));
        bind(&host);
        host
    }

    #[test]
    fn commands_require_a_bound_host() {
        let _lock = serial();
        reset();
        let error = game_download_status();
        assert_eq!(error.phase, "idle");
        let value = block_on(dispatch(
            &*FakeHost::new(),
            "game_download_cancel",
            Value::Null,
        ));
        assert!(value.is_err());
        let failure = value.unwrap_err();
        assert!(failure
            .as_str()
            .unwrap_or_default()
            .contains("尚未绑定宿主"));
    }

    #[test]
    fn cancel_and_status_read_the_package_state() {
        let _lock = serial();
        reset();
        let host = host();
        block_on(dispatch(&*host, "game_download_cancel", Value::Null)).expect("取消命令");
        assert_eq!(game_download_status().phase, "idle");
        set_auto_waiting_reason("window");
        assert_eq!(game_download_status().auto_waiting, "window");
        let events = host.download_events();
        assert!(!events.is_empty());
        assert_eq!(events.last().unwrap()["autoWaiting"], "window");
        reset();
    }

    #[test]
    fn catalog_requires_a_launcher_root() {
        let _lock = serial();
        reset();
        let host = host();
        let error = block_on(dispatch(&*host, "game_resource_catalog", Value::Null))
            .expect_err("缺少启动器配置应失败");
        assert!(
            error
                .as_str()
                .unwrap_or_default()
                .contains("读取现有启动器配置失败"),
            "{error}"
        );
        reset();
    }

    #[test]
    fn migration_activity_blocks_starting_a_download() {
        let _lock = serial();
        reset();
        let host = host();
        host.set_activity(ActivitySnapshot {
            migration_busy: true,
            ..Default::default()
        });
        let error = start_download("check", false, "manual").expect_err("迁移期间拒绝启动");
        assert!(error.contains("安装目录迁移正在进行"), "{error}");
        reset();
    }

    #[test]
    fn settings_gate_shares_the_state_lock_with_downloads() {
        let _lock = serial();
        reset();
        let (sender, receiver) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let gate = state::settings_gate().expect("取得设置门闩");
            sender.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(120));
            drop(gate);
        });
        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("门闩已取得");
        // 门闩持有期间再次获取会阻塞, 释放后成功.
        let started = Instant::now();
        let gate = state::settings_gate().expect("释放后取得门闩");
        assert!(started.elapsed() >= Duration::from_millis(80));
        drop(gate);
        holder.join().unwrap();
        reset();
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("构建测试运行时")
            .block_on(future)
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod ported_tests;
