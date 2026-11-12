//! 下载域的 UI 载荷与进程内长驻状态.
//!
//! 状态由包持有; 宿主只通过公开查询与门闩 API 读取或持锁, 不再保存第二份
//! `Service.downloads`. 后台任务需要继续发事件, 因此宿主在启动时绑定一个
//! `Arc<dyn HostContext>`; 这是单实例语义 (同进程只有一个下载域状态),
//! 多游戏实例需要运行时提供按包状态槽, 属未解问题.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Instant;

use oddity_platform::install_lock::InstallLock;
use oddity_platform::throttle::RateLimiter;
use oddity_runtime::HostContext;
use serde::Serialize;

use crate::update::optional::OptionalResource;
use crate::update::plan::Throughput;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub phase: String,
    pub mode: String,
    /// 本轮任务的触发来源: manual 或 auto.
    pub origin: String,
    /// 自动下载当前等待的原因, 空字符串表示没有等待.
    pub auto_waiting: String,
    pub distribution: String,
    pub install_root: String,
    /// 官方本地清单存在时视为已安装，用于区分安装与更新。
    pub installed: bool,
    pub full: bool,
    pub unverified_files: usize,
    pub current_file: String,
    pub current_file_bytes: u64,
    pub completed_files: usize,
    pub total_files: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub patch_files: usize,
    pub patch_bytes: u64,
    pub full_files: usize,
    pub full_bytes: u64,
    pub stage: String,
    pub remote_version: String,
    /// 本次计划包含的可选资源 tag.
    pub optional_tags: Vec<String>,
    /// 当前预下载任务是在校验已完成的缓存.
    pub verify: bool,
    pub pending_files: usize,
    /// 最近一次资源检查完成的时间 (Unix 毫秒), 0 表示尚未检查.
    pub checked_at: u64,
    /// 本轮写入目标所在文件系统的可用字节, 读取失败时为空.
    pub disk_available_bytes: Option<u64>,
    /// 当前计划预计额外占用的字节.
    pub disk_required_bytes: u64,
    pub error: String,
    pub resource_tasks: BTreeMap<String, ResourceTask>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTask {
    pub(crate) phase: String,
    pub(crate) downloaded_bytes: u64,
    pub(crate) total_bytes: u64,
    pub(crate) error: String,
}

impl ResourceTask {
    pub(crate) fn running(&self) -> bool {
        matches!(self.phase.as_str(), "checking" | "downloading")
    }
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
            resource_tasks: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCatalog {
    pub(crate) distribution: String,
    pub(crate) install_root: String,
    /// 当前正式资源版本.
    pub(crate) version: String,
    /// 公告中的未来版本, 空字符串表示当前没有资源预下载.
    pub(crate) preload_version: String,
    /// 公告版本仍晚于当前正式版本, 可以执行预下载.
    pub(crate) preload_available: bool,
    /// 公告版本的预下载缓存是否已完成.
    pub(crate) preload_completed: bool,
    /// 缓存根下是否存在可删除的预下载缓存.
    pub(crate) preload_cached: bool,
    /// 删除按钮对应的缓存版本.
    pub(crate) preload_cached_version: String,
    /// 缓存目录当前占用的字节数.
    pub(crate) preload_cached_bytes: u64,
    /// 可按语种清理的未来版本缓存大小.
    pub(crate) preload_resource_bytes: BTreeMap<String, u64>,
    pub(crate) resources: Vec<OptionalResource>,
}

#[derive(Default)]
pub(crate) struct DownloadInner {
    pub(crate) status: DownloadStatus,
    pub(crate) cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) last_progress: Option<Instant>,
    /// 最近一次成功应用任务的实测吞吐, 含 app_id, 用于下次检查估算完成时间.
    pub(crate) throughput: Option<(String, Throughput)>,
    pub(crate) resource_group: Option<ResourceGroup>,
    /// 预下载缓存根, 用于取消时刷新磁盘余量.
    pub(crate) preload_root: Option<std::path::PathBuf>,
    /// 宿主登记的任务 id; 取消时同时请求宿主取消.
    pub(crate) task_id: Option<String>,
}

pub(crate) struct ResourceGroup {
    pub(crate) install_lock: Arc<InstallLock>,
    pub(crate) limiter: Arc<RateLimiter>,
    pub(crate) reserved: BTreeMap<String, u64>,
}

#[derive(Default)]
pub struct DownloadManager {
    pub(crate) inner: Mutex<DownloadInner>,
    pub(crate) resource_ledger: Mutex<()>,
}

pub fn busy(phase: &str) -> bool {
    matches!(phase, "checking" | "launcher" | "game")
}

pub(crate) fn writes_launcher_files(mode: &str) -> bool {
    mode == "apply" || mode == "launcher"
}

/// 启动器更新是写操作, 单靠大小比对会漏掉同尺寸的内容变更.
pub(crate) fn launcher_plan_full(mode: &str, requested: bool) -> bool {
    requested || mode == "launcher"
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

static MANAGER: OnceLock<DownloadManager> = OnceLock::new();

/// 下载域唯一的进程内状态.
pub fn manager() -> &'static DownloadManager {
    MANAGER.get_or_init(DownloadManager::default)
}

static HOST: RwLock<Option<Arc<dyn HostContext>>> = RwLock::new(None);

/// 宿主启动时绑定能力句柄; 重复绑定覆盖旧句柄.
pub fn bind_host(host: Arc<dyn HostContext>) {
    if let Ok(mut slot) = HOST.write() {
        *slot = Some(host);
    }
}

/// 解除绑定; 仅供测试与宿主关闭路径使用.
pub fn unbind_host() {
    if let Ok(mut slot) = HOST.write() {
        *slot = None;
    }
}

/// 绑定的宿主能力; 未绑定时命令返回明确错误.
pub(crate) fn host() -> Result<Arc<dyn HostContext>, String> {
    HOST.read()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "更新域尚未绑定宿主".to_string())
}

/// 发一条下载状态事件; 宿主输出关闭时直接丢弃.
pub(crate) fn emit_status(status: &DownloadStatus) {
    let Ok(host) = host() else {
        return;
    };
    if let Ok(payload) = serde_json::to_value(status) {
        let _ = host.emit("game-download-state", payload);
    }
}

/// 当前下载状态快照.
pub fn snapshot() -> DownloadStatus {
    manager()
        .inner
        .lock()
        .map(|inner| inner.status.clone())
        .unwrap_or_default()
}

/// 持锁修改状态并发布事件; 事件在持锁时发送, 避免并行下载时事件乱序.
pub(crate) fn publish(change: impl FnOnce(&mut DownloadStatus)) {
    let Ok(mut inner) = manager().inner.lock() else {
        return;
    };
    change(&mut inner.status);
    let status = inner.status.clone();
    emit_status(&status);
}

/// 累加下载进度, 按 150ms 节流发布.
pub(crate) fn publish_bytes(bytes: u64) {
    let Ok(mut inner) = manager().inner.lock() else {
        return;
    };
    inner.status.downloaded_bytes += bytes;
    let now = Instant::now();
    if inner
        .last_progress
        .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(150))
    {
        return;
    }
    inner.last_progress = Some(now);
    let status = inner.status.clone();
    emit_status(&status);
}

/// 取消时刷新目标文件系统的可用空间.
///
/// 由持有 inner 锁的调用方传入预下载根, 避免在已持锁时二次加锁.
pub(crate) fn refresh_disk_available(status: &mut DownloadStatus, preload_root: Option<&Path>) {
    let root = Path::new(&status.install_root);
    let path = if status.mode == "preload" {
        preload_root
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf())
    } else {
        root.to_path_buf()
    };
    status.disk_available_bytes = oddity_platform::disk::available_bytes(&path).ok();
}

/// 持锁修改整个下载内部状态并发布事件; 闭包内可安全读取 preload_root 等字段.
pub(crate) fn publish_inner(change: impl FnOnce(&mut DownloadInner)) {
    let Ok(mut inner) = manager().inner.lock() else {
        return;
    };
    change(&mut inner);
    let status = inner.status.clone();
    drop(inner);
    emit_status(&status);
}

pub(crate) fn publish_cancelled() {
    publish_inner(|inner| {
        let preload_root = inner.preload_root.clone();
        refresh_disk_available(&mut inner.status, preload_root.as_deref());
        inner.status.phase = "cancelled".into();
        if inner.status.mode == "preload" {
            inner.status.checked_at = now_ms();
        }
    });
}

/// 宿主设置保存使用的门闩: 与下载启动/进度共用同一把状态锁.
pub struct SettingsGate {
    _guard: MutexGuard<'static, DownloadInner>,
}

/// 获取设置保存门闩; 下载任务运行期间会阻塞到状态锁可用.
pub fn settings_gate() -> Result<SettingsGate, String> {
    manager()
        .inner
        .lock()
        .map(|guard| SettingsGate { _guard: guard })
        .map_err(|error| error.to_string())
}

impl SettingsGate {
    /// 持门闩时的下载状态快照; 避免调用方在已持锁时再调 snapshot() 自锁.
    pub fn status(&self) -> DownloadStatus {
        self._guard.status.clone()
    }
}

/// 当前是否处于会阻塞设置/启动/环境变更的写操作.
pub fn blocks_changes(status: &DownloadStatus) -> bool {
    busy(&status.phase) && status.mode != "check"
}

/// 测试注入: 覆盖当前状态 (只改字段, 不发事件).
#[cfg(test)]
pub fn set_status(status: DownloadStatus) {
    if let Ok(mut inner) = manager().inner.lock() {
        inner.status = status;
    }
}

/// 是否有任务仍在运行.
pub fn is_busy() -> bool {
    manager()
        .inner
        .lock()
        .map(|inner| busy(&inner.status.phase))
        .unwrap_or(false)
}

/// 请求取消当前任务并同步宿主任务登记.
pub(crate) fn cancel_current() {
    let mut task_id = None;
    if let Ok(inner) = manager().inner.lock() {
        if let Some(cancel) = &inner.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        task_id = inner.task_id.clone();
    }
    if let Some(id) = task_id {
        if let Ok(host) = host() {
            host.cancel_task(&id);
        }
    }
}

/// 记录本轮任务在宿主登记里的 id.
pub(crate) fn set_task_id(id: Option<String>) {
    if let Ok(mut inner) = manager().inner.lock() {
        inner.task_id = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .expect("payload 是对象")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn download_status_payload_matches_the_ui_fields() {
        let value = serde_json::to_value(DownloadStatus::default()).unwrap();
        assert_eq!(
            keys(&value),
            vec![
                "autoWaiting",
                "checkedAt",
                "completedFiles",
                "currentFile",
                "currentFileBytes",
                "diskAvailableBytes",
                "diskRequiredBytes",
                "distribution",
                "downloadedBytes",
                "error",
                "full",
                "fullBytes",
                "fullFiles",
                "installRoot",
                "installed",
                "mode",
                "optionalTags",
                "origin",
                "patchBytes",
                "patchFiles",
                "pendingFiles",
                "phase",
                "remoteVersion",
                "resourceTasks",
                "stage",
                "totalBytes",
                "totalFiles",
                "unverifiedFiles",
                "verify",
            ]
        );
        assert_eq!(value["mode"], json!("check"));
        assert_eq!(value["phase"], json!("idle"));
        assert_eq!(value["origin"], json!("manual"));
        assert_eq!(value["diskAvailableBytes"], json!(null));
        assert_eq!(value["resourceTasks"], json!({}));
    }

    #[test]
    fn resource_catalog_payload_matches_the_ui_fields() {
        let catalog = ResourceCatalog {
            distribution: "cn".into(),
            install_root: "/games/nte".into(),
            version: "1.4.20".into(),
            preload_version: String::new(),
            preload_available: false,
            preload_completed: false,
            preload_cached: false,
            preload_cached_version: String::new(),
            preload_cached_bytes: 0,
            preload_resource_bytes: BTreeMap::new(),
            resources: Vec::new(),
        };
        let value = serde_json::to_value(catalog).unwrap();
        assert_eq!(
            keys(&value),
            vec![
                "distribution",
                "installRoot",
                "preloadAvailable",
                "preloadCached",
                "preloadCachedBytes",
                "preloadCachedVersion",
                "preloadCompleted",
                "preloadResourceBytes",
                "preloadVersion",
                "resources",
                "version",
            ]
        );
        assert_eq!(value["distribution"], json!("cn"));
        assert_eq!(value["preloadResourceBytes"], json!({}));
    }
}
