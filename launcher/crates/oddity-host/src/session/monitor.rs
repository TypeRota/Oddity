//! 宿主观察者: 采样驱动管理器, 退出与终态收尾.
//!
//! 采样频率与顺序沿用原监视线程: 每 500ms 一轮, 游戏退出后立即释放界面,
//! 安装保护保留到终态收尾.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use oddity_runtime::{EventSink, LaunchFailure, SessionEnd, SessionSample};

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
    if game_running {
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
    } else {
        state.snapshot_running_at = None;
    }
    if game_running && !state.playtime_started && Instant::now() >= state.start_retry_at {
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
