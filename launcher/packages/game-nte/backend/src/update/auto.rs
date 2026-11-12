//! 资源自动下载调度.
//!
//! 只在启动器运行期间工作: 发现更新后按设置快照里的时段与计费网络策略启动
//! 包内下载任务. 关闭启动器后的无人值守不在本模块范围内.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oddity_platform::clock::local_minute;
use oddity_platform::network::{metered, Metered};
use oddity_runtime::{
    HostContext, MeteredPolicy, SettingsSnapshot, UpdatePreferences, UpdateWindow,
};

use super::state::DownloadStatus;
use super::{busy, cancel_download, download_status, set_auto_waiting_reason, start_download};
use crate::install;
use crate::update::region::{self, Region};

/// 调度线程的检查周期.
#[cfg(not(test))]
const TICK: Duration = Duration::from_secs(15);
#[cfg(test)]
const TICK: Duration = Duration::from_millis(10);
/// 两次资源检查之间的最小间隔; 启动时的静默检查也计入这个时间.
const CHECK_INTERVAL_MS: u64 = 30 * 60 * 1000;
/// 用户手动暂停自动下载后, 这段时间内不再自动重启.
const CANCEL_SNOOZE_MS: u64 = 60 * 60 * 1000;
/// 任务失败后的重试间隔.
const FAILURE_BACKOFF_MS: u64 = 10 * 60 * 1000;
/// 计费状态缓存时间.
const METERED_CACHE_MS: u64 = 60 * 1000;
/// 下载过程中的状态采样间隔.
#[cfg(not(test))]
const MONITOR_INTERVAL: Duration = Duration::from_secs(5);
#[cfg(test)]
const MONITOR_INTERVAL: Duration = Duration::from_millis(10);

pub fn start(host: Arc<dyn HostContext>, stop: Arc<AtomicBool>) {
    start_with_stop(host, stop);
}

fn start_with_stop(host: Arc<dyn HostContext>, stop: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new()
        .name("oddity-auto-update".into())
        .spawn(move || run(host, &stop));
}

#[cfg(test)]
fn start_for_test(host: Arc<dyn HostContext>, stop: Arc<AtomicBool>) {
    start_with_stop(host, stop);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn settings(host: &Arc<dyn HostContext>) -> Option<SettingsSnapshot> {
    host.settings().ok()
}

fn game_busy(host: &Arc<dyn HostContext>) -> bool {
    match host.activity() {
        Ok(activity) => {
            activity.environment_busy || activity.game_starting || activity.game_running
        }
        Err(_) => true,
    }
}

/// 与下载任务一致: 未配置目录时使用环境探测结果.
fn install_root(settings: &SettingsSnapshot) -> PathBuf {
    let selected = install::selected_install_root_for(settings);
    if install::configured_install_root_for(settings).is_some() {
        return selected;
    }
    install::detect_launcher_root_for(settings)
        .and_then(|root| root.parent().map(|parent| parent.to_path_buf()))
        .unwrap_or(selected)
}

fn installed(settings: &SettingsSnapshot) -> bool {
    let launcher_root = install_root(settings).join(launcher_dir(settings));
    region::LAYOUT
        .patcher_sdk_path(&launcher_root)
        .join("ResList.xml")
        .is_file()
}

fn launcher_dir(settings: &SettingsSnapshot) -> &'static str {
    Region::parse(&settings.distribution)
        .map(|region| region.profile().launcher_dir)
        .unwrap_or("")
}

fn snapshot() -> Option<DownloadStatus> {
    Some(download_status())
}

/// 查询计费状态; 相同结果在一分钟内复用.
fn current_metered(
    options: &UpdatePreferences,
    cache: &mut Option<(u64, Metered)>,
    now: u64,
) -> Metered {
    if options.metered == MeteredPolicy::Allow {
        return Metered::No;
    }
    if let Some((at, value)) = *cache {
        if now.saturating_sub(at) < METERED_CACHE_MS {
            return value;
        }
    }
    let value = metered();
    *cache = Some((now, value));
    value
}

fn wait_idle(timeout: Duration) {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        match snapshot() {
            Some(status) if busy(&status.phase) => {
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => return,
        }
    }
}

fn run(host: Arc<dyn HostContext>, stop: &AtomicBool) {
    let mut snooze_until = 0u64;
    let mut metered_cache: Option<(u64, Metered)> = None;
    let mut last_failure = 0u64;
    let mut announced = false;

    loop {
        std::thread::sleep(TICK);
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(settings) = settings(&host) else {
            continue;
        };
        let options = &settings.update;
        if !options.auto_download {
            announced = false;
            // 关闭自动下载后清掉残留的等待提示.
            if let Some(status) = snapshot() {
                if !status.auto_waiting.is_empty() {
                    set_auto_waiting_reason("");
                }
            }
            continue;
        }
        if !announced {
            eprintln!("自动下载: 已启用, 等待检查周期");
            announced = true;
        }
        let now = now_ms();
        if snooze_until > now {
            continue;
        }
        if last_failure > 0 && now.saturating_sub(last_failure) < FAILURE_BACKOFF_MS {
            continue;
        }

        let Some(status) = snapshot() else {
            continue;
        };

        if busy(&status.phase) {
            monitor(&host, &status, options, &mut metered_cache);
            continue;
        }

        // 手动或自动下载被暂停后都先冷却一段时间; 因时段或计费网络
        // 暂停的任务带有等待原因, 条件恢复后可以继续.
        if status.phase == "cancelled" && status.mode == "apply" && status.auto_waiting.is_empty() {
            if snooze_until == 0 {
                snooze_until = now_ms().saturating_add(CANCEL_SNOOZE_MS);
                continue;
            }
            // 冷却结束, 允许重新检查; 新一轮取消会重新计时.
            snooze_until = 0;
        }

        let root = install_root(&settings).display().to_string();
        let same_target =
            status.distribution == settings.distribution && status.install_root == root;
        // 手动预下载的结果保留一个检查周期, 便于查看完成状态或继续下载.
        if same_target
            && status.mode == "preload"
            && status.checked_at > 0
            && now.saturating_sub(status.checked_at) < CHECK_INTERVAL_MS
        {
            continue;
        }
        let recent = matches!(
            status.phase.as_str(),
            "ready" | "needs-install" | "needs-update" | "needs-verify"
        ) && same_target
            && status.checked_at > 0
            && now.saturating_sub(status.checked_at) < CHECK_INTERVAL_MS;

        if game_busy(&host) {
            // 最近一次检查已确认就绪时不再提示等待下载.
            let waiting = if recent && status.phase == "ready" {
                ""
            } else {
                "game"
            };
            set_auto_waiting_reason(waiting);
            continue;
        }
        if !installed(&settings) {
            // 首次安装必须由用户发起; 目录未就绪时保持静默.
            set_auto_waiting_reason("");
            continue;
        }

        // 检查不写入游戏文件, 时段外也照常进行, 以便提前告知有待下载的更新.
        let status = if recent {
            status
        } else {
            eprintln!("自动下载: 开始检查资源更新");
            if let Err(error) = start_download("check", false, "auto") {
                // 已有任务在进行, 下一轮再试.
                eprintln!("自动下载: 检查未启动: {error}");
                continue;
            }
            wait_idle(Duration::from_secs(10 * 60));
            let Some(status) = snapshot() else {
                continue;
            };
            if status.phase == "failed" {
                last_failure = now_ms();
                continue;
            }
            if status.phase == "cancelled" {
                continue;
            }
            last_failure = 0;
            status
        };

        if status.phase == "ready" {
            set_auto_waiting_reason("");
            continue;
        }
        if !matches!(
            status.phase.as_str(),
            "needs-install" | "needs-update" | "needs-verify"
        ) {
            continue;
        }

        if options
            .window
            .as_ref()
            .is_some_and(|window| !window_contains(window, local_minute()))
        {
            set_auto_waiting_reason("window");
            continue;
        }
        let metered = current_metered(options, &mut metered_cache, now);
        if metered.is_metered() && options.metered != MeteredPolicy::Allow {
            set_auto_waiting_reason("metered");
            continue;
        }
        if status
            .disk_available_bytes
            .is_some_and(|available| status.disk_required_bytes > available)
        {
            set_auto_waiting_reason("disk");
            continue;
        }
        set_auto_waiting_reason("");

        snooze_until = 0;
        eprintln!("自动下载: 发现待下载资源, 开始下载");
        let started = match start_download("apply", false, "auto") {
            Ok(status) => status,
            Err(error) => {
                eprintln!("自动下载: 无法开始下载: {error}");
                continue;
            }
        };
        let finished = monitor_until_idle(&host, started, &mut metered_cache);
        match finished.phase.as_str() {
            "ready" => {
                last_failure = 0;
                set_auto_waiting_reason("");
            }
            "failed" => last_failure = now_ms(),
            "cancelled" if finished.auto_waiting.is_empty() => {
                snooze_until = now_ms().saturating_add(CANCEL_SNOOZE_MS);
            }
            _ => {}
        }
    }
}

/// 自动任务执行期间持续检查时段与计费网络, 违反策略时暂停.
fn monitor_until_idle(
    host: &Arc<dyn HostContext>,
    mut status: DownloadStatus,
    metered_cache: &mut Option<(u64, Metered)>,
) -> DownloadStatus {
    loop {
        if !busy(&status.phase) {
            return status;
        }
        std::thread::sleep(MONITOR_INTERVAL);
        // 下载期间设置可能被修改, 每次都读取最新值.
        if let Some(settings) = settings(host) {
            monitor(host, &status, &settings.update, metered_cache);
        }
        match snapshot() {
            Some(next) => status = next,
            None => return status,
        }
    }
}

fn monitor(
    _host: &Arc<dyn HostContext>,
    status: &DownloadStatus,
    options: &UpdatePreferences,
    metered_cache: &mut Option<(u64, Metered)>,
) {
    // 检查阶段很短, 只有真正写入文件的下载才需要按策略暂停.
    if status.origin != "auto" || status.mode != "apply" {
        return;
    }
    if options
        .window
        .as_ref()
        .is_some_and(|window| !window_contains(window, local_minute()))
    {
        cancel_download();
        set_auto_waiting_reason("window");
        return;
    }
    let metered = current_metered(options, metered_cache, now_ms());
    if metered.is_metered() && options.metered != MeteredPolicy::Allow {
        cancel_download();
        set_auto_waiting_reason("metered");
    }
}

fn parse_minutes(value: &str) -> Option<u16> {
    let value = value.trim();
    let (hour, minute) = value.split_once(':')?;
    if hour.is_empty()
        || hour.len() > 2
        || minute.len() != 2
        || !hour.bytes().all(|byte| byte.is_ascii_digit())
        || !minute.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let hour: u16 = hour.parse().ok()?;
    let minute: u16 = minute.parse().ok()?;
    if hour >= 24 || minute >= 60 {
        return None;
    }
    Some(hour * 60 + minute)
}

/// 与设置里的时间窗口语义一致: 支持跨零点, 无效窗口不阻塞自动下载.
fn window_contains(window: &UpdateWindow, minute: u16) -> bool {
    let (Some(start), Some(end)) = (parse_minutes(&window.start), parse_minutes(&window.end))
    else {
        return true;
    };
    if start == end {
        return true;
    }
    if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::state;
    use crate::update::testing::{reset, serial, FakeHost};
    use oddity_runtime::ActivitySnapshot;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicU64;
    use std::time::Instant;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "oddity-scheduler-{}-{}-{}",
                std::process::id(),
                tag,
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn installed_root(dir: &TempDir) -> PathBuf {
        let root = dir.path().join("install");
        let ledger = root.join("NTELauncher/UserData/Patcher/PatcherSDK/ResList.xml");
        fs::create_dir_all(ledger.parent().expect("ledger parent")).expect("create ledger dir");
        fs::write(ledger, b"ledger").expect("write ledger");
        root
    }

    fn test_settings(root: &Path) -> SettingsSnapshot {
        SettingsSnapshot {
            game: "nte".into(),
            distribution: "cn".into(),
            language: "zh-CN".into(),
            reduce_motion: false,
            theme: "light".into(),
            install_root: Some(root.display().to_string()),
            preload_root: None,
            update: UpdatePreferences {
                auto_download: true,
                metered: MeteredPolicy::Allow,
                ..Default::default()
            },
        }
    }

    fn test_host(settings: SettingsSnapshot) -> Arc<FakeHost> {
        let host = FakeHost::new();
        host.set_settings(settings);
        host
    }

    fn set_status(_host: &Arc<FakeHost>, update: impl FnOnce(&mut DownloadStatus)) {
        let mut status = crate::update::snapshot();
        update(&mut status);
        state::set_status(status);
    }

    fn ready_status(host: &Arc<FakeHost>, root: &Path) {
        let root = root.display().to_string();
        set_status(host, |status| {
            status.phase = "ready".into();
            status.mode = "check".into();
            status.origin = "manual".into();
            status.auto_waiting.clear();
            status.distribution = "cn".into();
            status.install_root = root.clone();
            status.checked_at = now_ms();
        });
    }

    fn download_status() -> DownloadStatus {
        crate::update::snapshot()
    }

    fn wait_until(mut check: impl FnMut() -> bool, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if check() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        check()
    }

    fn window_outside_now() -> UpdateWindow {
        let minute = local_minute();
        let start = (minute + 60) % (24 * 60);
        let end = (start + 60) % (24 * 60);
        UpdateWindow {
            start: format!("{:02}:{:02}", start / 60, start % 60),
            end: format!("{:02}:{:02}", end / 60, end % 60),
        }
    }

    fn start(host: &Arc<FakeHost>) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        start_for_test(host.clone(), Arc::clone(&stop));
        stop
    }

    fn halt(stop: &AtomicBool) {
        stop.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn manual_preload_results_survive_automatic_update_ticks() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("preload-result");
        let root = installed_root(&dir);
        let host = test_host(test_settings(&root));
        ready_status(&host, &root);
        set_status(&host, |status| {
            status.mode = "preload".into();
            status.phase = "preloaded".into();
        });
        let stop = start(&host);
        for phase in ["preloaded", "preload-unavailable", "cancelled", "failed"] {
            set_status(&host, |status| status.phase = phase.into());
            std::thread::sleep(Duration::from_millis(50));
            let status = download_status();
            assert_eq!(status.mode, "preload");
            assert_eq!(status.phase, phase);
            assert!(status.auto_waiting.is_empty());
        }
        halt(&stop);
    }

    #[test]
    fn disabled_auto_download_does_nothing() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("disabled");
        let root = installed_root(&dir);
        let mut settings = test_settings(&root);
        settings.update.auto_download = false;
        let host = test_host(settings);
        ready_status(&host, &root);
        let stop = start(&host);
        std::thread::sleep(Duration::from_millis(80));
        assert!(download_status().auto_waiting.is_empty());
        halt(&stop);
    }

    #[test]
    fn waits_for_game_and_clears_when_ready() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("game");
        let root = installed_root(&dir);
        let host = test_host(test_settings(&root));
        let root_text = root.display().to_string();
        set_status(&host, |status| {
            status.phase = "idle".into();
            status.mode = "check".into();
            status.distribution = "cn".into();
            status.install_root = root_text;
            status.checked_at = 0;
            status.auto_waiting.clear();
        });
        host.set_activity(ActivitySnapshot {
            game_starting: true,
            ..Default::default()
        });
        let stop = start(&host);
        assert!(wait_until(
            || download_status().auto_waiting == "game",
            Duration::from_secs(2)
        ));

        ready_status(&host, &root);
        assert!(wait_until(
            || download_status().auto_waiting.is_empty(),
            Duration::from_secs(2)
        ));

        host.set_activity(ActivitySnapshot::default());
        assert!(download_status().auto_waiting.is_empty());
        halt(&stop);
    }

    #[test]
    fn waits_for_window_then_disk() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("gates");
        let root = installed_root(&dir);
        let mut settings = test_settings(&root);
        settings.update.window = Some(window_outside_now());
        let host = test_host(settings.clone());
        let root_text = root.display().to_string();
        set_status(&host, |status| {
            status.phase = "needs-update".into();
            status.mode = "check".into();
            status.origin = "manual".into();
            status.auto_waiting.clear();
            status.distribution = "cn".into();
            status.install_root = root_text.clone();
            status.checked_at = now_ms();
            status.disk_available_bytes = Some(u64::MAX);
            status.disk_required_bytes = 0;
        });
        let stop = start(&host);
        assert!(wait_until(
            || download_status().auto_waiting == "window",
            Duration::from_secs(2)
        ));
        set_status(&host, |status| {
            status.disk_available_bytes = Some(0);
            status.disk_required_bytes = 1;
        });
        settings.update.window = None;
        host.set_settings(settings);
        assert!(wait_until(
            || download_status().auto_waiting == "disk",
            Duration::from_secs(2)
        ));
        halt(&stop);
    }

    #[test]
    fn monitors_auto_download_and_pauses_at_window_end() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("monitor");
        let root = installed_root(&dir);
        let mut settings = test_settings(&root);
        settings.update.window = Some(window_outside_now());
        let host = test_host(settings);
        let root_text = root.display().to_string();
        set_status(&host, |status| {
            status.phase = "game".into();
            status.mode = "apply".into();
            status.origin = "auto".into();
            status.distribution = "cn".into();
            status.install_root = root_text.clone();
            status.checked_at = now_ms();
        });
        let stop = start(&host);
        assert!(wait_until(
            || download_status().auto_waiting == "window",
            Duration::from_secs(2)
        ));
        halt(&stop);
    }

    #[test]
    fn cancelled_apply_starts_a_snooze() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("snooze");
        let root = installed_root(&dir);
        let host = test_host(test_settings(&root));
        let root_text = root.display().to_string();
        set_status(&host, |status| {
            status.phase = "cancelled".into();
            status.mode = "apply".into();
            status.origin = "auto".into();
            status.auto_waiting.clear();
            status.distribution = "cn".into();
            status.install_root = root_text.clone();
            status.checked_at = now_ms();
        });
        let stop = start(&host);
        std::thread::sleep(Duration::from_millis(80));
        let status = download_status();
        assert_eq!(status.phase, "cancelled");
        assert_eq!(status.mode, "apply");
        halt(&stop);
    }

    #[test]
    fn missing_installation_stays_silent() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("missing");
        let root = dir.path().join("install");
        let host = test_host(test_settings(&root));
        ready_status(&host, &root);
        let stop = start(&host);
        std::thread::sleep(Duration::from_millis(80));
        assert!(download_status().auto_waiting.is_empty());
        halt(&stop);
    }

    #[test]
    fn wait_idle_returns_after_the_task_finishes() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("idle");
        let root = installed_root(&dir);
        let host = test_host(test_settings(&root));
        set_status(&host, |status| status.phase = "checking".into());
        let helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            let mut status = crate::update::snapshot();
            status.phase = "ready".into();
            state::set_status(status);
        });
        let started = Instant::now();
        wait_idle(Duration::from_secs(2));
        assert!(started.elapsed() >= Duration::from_millis(80));
        helper.join().expect("join idle helper");
    }

    #[test]
    fn helper_decisions_are_stable() {
        let _serial = serial();
        reset();
        let dir = TempDir::new("helpers");
        let root = installed_root(&dir);
        let settings = test_settings(&root);
        assert_eq!(install_root(&settings), root);
        assert!(installed(&settings));
        let mut cache = None;
        assert_eq!(
            current_metered(&settings.update, &mut cache, 0),
            Metered::No
        );
        assert!(now_ms() > 0);
    }
}
