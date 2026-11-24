//! 宿主观察者: 采样驱动管理器, 退出与终态收尾.
//!
//! 采样频率与顺序沿用原监视线程: 每 500ms 一轮, 游戏退出后立即释放界面,
//! 安装保护保留到终态收尾.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use oddity_runtime::{EventSink, LaunchFailure, SessionEnd, SessionHang, SessionSample};

use crate::playtime_commands;
use crate::service::ServiceHandle;
use crate::session::Session;

/// 每轮采样需要跨次数保留的本地状态, 等价原监视线程的局部变量.
#[derive(Debug)]
pub(crate) struct MonitorState {
    pub(crate) last_sample: Instant,
    pub(crate) snapshot_running_at: Option<u64>,
    pub(crate) snapshot_success: bool,
    pub(crate) playtime_started: bool,
    pub(crate) start_retry_at: Instant,
    pub(crate) last_touch: Instant,
    /// 下一次扫描崩溃现场的时间; 扫描比采样稀疏.
    pub(crate) next_hang_scan: Instant,
    /// 本轮会话是否已经上报过现场; 每轮只提示一次.
    pub(crate) hang_reported: bool,
    /// 会话起点已经存在的崩溃现场; 基线之外的目录才属于本轮会话.
    pub(crate) hang_baseline: BTreeSet<PathBuf>,
    /// 上下文还没读完的现场; 补读结束之前一直保留.
    pub(crate) hang_site: Option<PathBuf>,
    /// 下一次补读现场上下文的时间.
    pub(crate) next_hang_read: Instant,
    /// 错误说明的补读截止时间; 崩溃类型才是必填字段.
    pub(crate) hang_enrich_until: Option<Instant>,
}

impl Default for MonitorState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            last_sample: now - Duration::from_secs(1),
            snapshot_running_at: None,
            snapshot_success: false,
            playtime_started: false,
            start_retry_at: now,
            last_touch: now,
            next_hang_scan: now,
            hang_reported: false,
            hang_baseline: BTreeSet::new(),
            hang_site: None,
            next_hang_read: now,
            hang_enrich_until: None,
        }
    }
}

impl MonitorState {
    /// 以给定 prefix 的当前现场作为基线.
    pub(crate) fn for_prefix(prefix: &Path) -> Self {
        Self {
            hang_baseline: crate::session::hang::baseline(prefix),
            ..Self::default()
        }
    }
}

/// 应用一轮采样: 音频 PID, 性能硬件采样, game_mode 进程表, 快照与游玩时长.
pub(crate) fn apply_sample(app: &ServiceHandle, session: &Session, sample: &SessionSample) {
    let mut state = match session.monitor_state.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    let game_running = sample.game_running;
    let running = game_running;
    let mut audio_pids = sample.game_pids.clone();
    audio_pids.sort_unstable();
    audio_pids.dedup();
    app.audio.processes(audio_pids.clone());
    if game_running && state.last_sample.elapsed() >= Duration::from_secs(1) {
        if let Ok(mut recording) = session.performance.lock() {
            if let Some(recording) = recording.as_mut() {
                if let Err(error) = recording.sample(&audio_pids) {
                    eprintln!("性能硬件采样失败: {error:#}");
                }
            }
        }
        state.last_sample = Instant::now();
    }
    if let Some(mode) = &session.game_mode {
        if let Ok(mut mode) = mode.lock() {
            if mode.needs_game_processes() {
                mode.update_game_processes(&audio_pids);
            }
        }
    }
    // 崩溃现场出现时进程仍会常驻, 只能靠扫描 prefix 发现.
    let detected = if game_running && !session.closing {
        detect_hang(session, &mut state)
    } else {
        None
    };
    if detected.is_some() {
        state.hang_reported = true;
        // 卡死后不再续期游玩记录, 由收尾动作结束本轮计时.
        state.playtime_started = false;
    }
    // 上下文可能很晚才落盘, 摘要补齐前持续按扫描间隔补读.
    let retry = if detected.is_none() && game_running && !session.closing {
        retry_hang(app, session, &mut state)
    } else {
        None
    };
    // 卡死后不再推进快照, 避免把已经挂起的会话记成成功启动.
    if game_running && !state.hang_reported {
        if state.snapshot_running_at.is_none() {
            let now = crate::now_millis();
            if let Some(id) = session.snapshot.as_deref() {
                app.snapshots.mark_running(id, now);
            }
            state.snapshot_running_at = Some(now);
        }
        if let (Some(start), false) = (state.snapshot_running_at, state.snapshot_success) {
            let now = crate::now_millis();
            if now.saturating_sub(start) >= session.snapshot_success_after_ms {
                if let Some(id) = session.snapshot.as_deref() {
                    let version = session
                        .launcher_root
                        .as_ref()
                        .map(|root| {
                            crate::install::game_version(&app.state.packages, &session.game, root)
                        })
                        .filter(|version| !version.is_empty());
                    state.snapshot_success = app.snapshots.mark_success(
                        id,
                        now,
                        version,
                        session.snapshot_success_after_ms,
                    );
                } else {
                    state.snapshot_success = true;
                }
            }
        }
    } else if !game_running {
        state.snapshot_running_at = None;
    }
    if game_running
        && !state.playtime_started
        && !state.hang_reported
        && Instant::now() >= state.start_retry_at
    {
        let started = match app.playtime.start(
            playtime_commands::now_millis(),
            &session.mode,
            session.distribution.as_str(),
        ) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("开始记录游玩时长失败: {error}");
                state.start_retry_at = Instant::now() + Duration::from_secs(30);
                false
            }
        };
        if started {
            state.playtime_started = true;
            state.last_touch = Instant::now();
            // 会话创建晚于 running, 通知前端补读记录。
            let _ = app.emit("playtime-started", ());
        }
    } else if state.playtime_started && !game_running {
        if let Err(error) = app.playtime.finish(playtime_commands::now_millis()) {
            eprintln!("结束游玩记录失败: {error}");
        }
        state.playtime_started = false;
    } else if state.playtime_started && state.last_touch.elapsed() >= Duration::from_secs(30) {
        if let Err(error) = app.playtime.touch(playtime_commands::now_millis()) {
            eprintln!("更新游玩记录心跳失败: {error}");
        }
        state.last_touch = Instant::now();
    }
    session.game_alive.store(running, Ordering::Release);
    // 上报要取注册表锁并发事件; 先在采样状态之外完成, 避免与注册表锁交叉.
    drop(state);
    if let Some(hang) = detected {
        report_hang(app, session, hang);
    }
    if let Some(fill) = retry {
        refresh_hang(app, session, fill);
    }
}

/// 按固定间隔扫描本轮会话新增的崩溃现场.
fn detect_hang(session: &Session, state: &mut MonitorState) -> Option<SessionHang> {
    if state.hang_reported {
        return None;
    }
    let now = Instant::now();
    if now < state.next_hang_scan {
        return None;
    }
    state.next_hang_scan = now + crate::session::hang::SCAN_INTERVAL;
    let hit = crate::session::hang::scan(&session.prefix, &state.hang_baseline)?;
    // 上下文可能还没写完: 保留现场补读崩溃类型与错误说明.
    state.hang_site = Some(hit.site);
    state.next_hang_read = now + crate::session::hang::SCAN_INTERVAL;
    state.hang_enrich_until = None;
    Some(hit.hang)
}

/// 补读保留的现场; 读到新内容时返回该现场的原因与说明.
///
/// UE 可能在现场目录出现很久之后才落盘上下文, 所以补读一直持续到会话收尾,
/// 只在还没读到崩溃类型时按扫描间隔读取.
fn retry_hang(
    app: &ServiceHandle,
    session: &Session,
    state: &mut MonitorState,
) -> Option<(String, String)> {
    let site = state.hang_site.clone()?;
    let now = Instant::now();
    if now < state.next_hang_read {
        return None;
    }
    state.next_hang_read = now + crate::session::hang::SCAN_INTERVAL;
    // 摘要是注册表里的唯一副本: 崩溃类型到手后只再补读一段错误说明.
    if let Some(hang) = crate::session::hang_of(&app.state, session.id) {
        if !hang.crash_type.is_empty() {
            if !hang.error_message.is_empty() {
                state.hang_site = None;
                state.hang_enrich_until = None;
                return None;
            }
            // 第一次发现崩溃类型时才开补读窗口, 到期就不再读盘.
            let until = *state
                .hang_enrich_until
                .get_or_insert_with(|| now + crate::session::hang::ENRICH_WINDOW);
            if now >= until {
                state.hang_site = None;
                state.hang_enrich_until = None;
                return None;
            }
        }
    }
    let (crash_type, error_message) = crate::session::hang::context(&site);
    if crash_type.is_empty() && error_message.is_empty() {
        return None;
    }
    Some((crash_type, error_message))
}

/// 补读结果是否需要通知界面: 读到崩溃类型, 或补齐了错误说明.
fn filled(before: &SessionHang, fill: &(String, String)) -> bool {
    (before.crash_type.is_empty() && !fill.0.is_empty())
        || (before.error_message.is_empty() && !fill.1.is_empty())
}

/// 按会话 id 合并崩溃摘要; 返回本次补读是否填入了新内容.
///
/// 摘要是注册表里的唯一副本, 补读只填还没读到的字段.
fn merge_hang(app: &ServiceHandle, session: &Session, fill: &(String, String)) -> Option<bool> {
    let state = &app.state;
    let mut tracker = match state.game.lock() {
        Ok(tracker) => tracker,
        Err(_) => return None,
    };
    // 迟到的采样不能把现场记到下一轮会话上.
    if tracker.session.as_ref().map(|current| current.id) != Some(session.id) {
        return None;
    }
    let hang = tracker.hang.as_mut()?;
    let changed = filled(hang, fill);
    if hang.crash_type.is_empty() {
        hang.crash_type = fill.0.clone();
    }
    if hang.error_message.is_empty() {
        hang.error_message = fill.1.clone();
    }
    Some(changed)
}

/// 登记本轮会话的崩溃摘要, 摘要从此由注册表独占.
fn record_hang(app: &ServiceHandle, session: &Session, hang: SessionHang) -> bool {
    let state = &app.state;
    let mut tracker = match state.game.lock() {
        Ok(tracker) => tracker,
        Err(_) => return false,
    };
    if tracker.session.as_ref().map(|current| current.id) != Some(session.id) {
        return false;
    }
    tracker.hang = Some(hang);
    true
}

/// 上报崩溃现场: 收敛游玩记录, 记录摘要, 通知界面并把窗口带回前台.
///
/// 卡死时游戏窗口占着前台, 玩家看不到提示, 所以这里不受设置影响.
fn report_hang(app: &ServiceHandle, session: &Session, hang: SessionHang) {
    if !record_hang(app, session, hang) {
        return;
    }
    if let Err(error) = app.playtime.finish(playtime_commands::now_millis()) {
        eprintln!("结束游玩记录失败: {error}");
    }
    crate::session::emit_game_status(app, &app.state);
    crate::session::show_main(app);
}

/// 摘要补全: 填到新内容时补发一次状态, 不重复提示.
///
/// 上下文还没写完时先把读到的字段记下, 后续读到缺失字段再通知界面.
fn refresh_hang(app: &ServiceHandle, session: &Session, fill: (String, String)) {
    if merge_hang(app, session, &fill) != Some(true) {
        return;
    }
    crate::session::emit_game_status(app, &app.state);
}

/// 游戏进程消失: 立即回窗口, 安装保护保留到后台清理结束.
pub(crate) fn finish_game_early(app: &ServiceHandle, session: &Session) {
    if let Some(mode) = &session.game_mode {
        if let Ok(mut mode) = mode.lock() {
            mode.release();
        }
    }
    if let Some(id) = session.snapshot.as_deref() {
        app.snapshots.finish(id, None);
    }
    app.audio.processes(Vec::new());
    if let Ok(mut state) = session.monitor_state.lock() {
        state.playtime_started = false;
    }
    finish_performance(app, session);
    let cleared = {
        let state = &app.state;
        let mut tracker = match state.game.lock() {
            Ok(tracker) => tracker,
            Err(_) => return,
        };
        if let Some(current) = tracker
            .session
            .as_mut()
            .filter(|current| current.id == session.id && !current.exit_notified)
        {
            current.exit_notified = true;
            Some(tracker.status())
        } else {
            None
        }
    };
    if let Some(status) = cleared {
        let _ = app.emit("game-state", status);
        if crate::session::should_show_launcher_after_exit(app) {
            crate::session::show_main(app);
        }
    }
}

fn finish_performance(app: &ServiceHandle, session: &Session) {
    let recording = session.performance.lock().ok().and_then(|mut r| r.take());
    if let Some(recording) = recording {
        let app = app.clone();
        thread::spawn(move || {
            if let Err(error) = app.performance.finish(&recording) {
                eprintln!("性能报告保存失败: {error}");
            }
            let _ = app.emit("performance-updated", ());
        });
    }
}

/// 会话只收尾一次: 停止监控, 完成快照、游玩记录和游戏模式清理.
pub(crate) fn finalize_game_session(
    app: &ServiceHandle,
    session: &Session,
    failure: Option<crate::snapshots::Failure>,
) {
    if session.finalized.swap(true, Ordering::AcqRel) {
        return;
    }
    session.monitor_done.store(true, Ordering::Release);
    finish_performance(app, session);
    app.audio.processes(Vec::new());
    if let Some(id) = session.snapshot.as_deref() {
        app.snapshots.finish(id, failure);
    }
    if let Err(error) = app.playtime.finish(playtime_commands::now_millis()) {
        eprintln!("结束游玩记录失败: {error}");
    }
    if let Some(mode) = &session.game_mode {
        if let Ok(mut mode) = mode.lock() {
            mode.release();
        }
    }
}

/// 会话终态: 收尾, 记录错误, 发终态事件并按设置回窗口.
pub(crate) fn finish_session(app: &ServiceHandle, session: &Session, end: &SessionEnd) {
    // 收尾前再读一次现场, UE 可能在最后一轮采样之后才写完上下文.
    fill_hang_before_exit(app, session);
    let mut failure = crate::snapshots::Failure::early_exit(end.exit_code);
    if let Some(detail) = end
        .failure
        .as_ref()
        .and_then(|failure| failure.detail.clone())
    {
        failure.detail = Some(detail);
    }
    finalize_game_session(app, session, Some(failure));
    let (status, exit_notified) = {
        let state = &app.state;
        let mut tracker = match state.game.lock() {
            Ok(tracker) => tracker,
            Err(_) => return,
        };
        if tracker.session.as_ref().map(|current| current.id) != Some(session.id) {
            return;
        }
        let exit_notified = tracker
            .session
            .as_ref()
            .is_some_and(|session| session.exit_notified);
        tracker.last_error = last_error(end);
        // 清理尚未完成时保留会话, 关闭命令可以重试.
        if session.control().is_some_and(|control| control.is_alive()) {
            if let Some(current) = tracker.session.as_mut() {
                current.closing = false;
            }
        } else {
            tracker.session = None;
        }
        tracker.clear_launch();
        (tracker.status(), exit_notified)
    };
    let finished = !status.running;
    let _ = app.emit("game-state", status);
    if finished && !exit_notified && crate::session::should_show_launcher_after_exit(app) {
        crate::session::show_main(app);
    }
}

/// 收尾兜底: 对保留的现场再读一次, 摘要补齐时只更新注册表.
///
/// 终态事件随后发出, 这里不再单独补发, 也不会重复提示.
fn fill_hang_before_exit(app: &ServiceHandle, session: &Session) {
    let Some(site) = session
        .monitor_state
        .lock()
        .ok()
        .and_then(|state| state.hang_site.clone())
    else {
        return;
    };
    let (crash_type, error_message) = crate::session::hang::context(&site);
    if crash_type.is_empty() && error_message.is_empty() {
        return;
    }
    // 终态事件随后发出, 这里只更新摘要.
    if merge_hang(app, session, &(crash_type, error_message)).is_some() {
        if let Ok(mut state) = session.monitor_state.lock() {
            state.hang_site = None;
        }
    }
}

fn failure_message(kind: oddity_runtime::LaunchFailureKind) -> &'static str {
    use oddity_runtime::LaunchFailureKind;
    match kind {
        LaunchFailureKind::Spawn => "启动失败",
        LaunchFailureKind::EarlyExit => "游戏提前退出",
        LaunchFailureKind::Timeout => "启动超时",
        LaunchFailureKind::Cancelled => "启动已取消",
        LaunchFailureKind::Backend => "启动实现失败",
    }
}

fn last_error(end: &SessionEnd) -> Option<String> {
    let failure = end.failure.as_ref()?;
    if failure.kind == oddity_runtime::LaunchFailureKind::Cancelled {
        return None;
    }
    failure
        .detail
        .clone()
        .or_else(|| Some(failure_message(failure.kind).to_string()))
}

/// 编排线程异常退出: 兜底收尾并释放租约, 让注册表回到空闲.
pub(crate) fn abandon_session(app: &ServiceHandle, session: &Session) {
    let end = SessionEnd {
        game_started: false,
        exit_code: None,
        failure: Some(LaunchFailure {
            kind: oddity_runtime::LaunchFailureKind::Backend,
            detail: Some("启动实现异常退出".into()),
        }),
        forced: true,
    };
    finish_session(app, session, &end);
}
