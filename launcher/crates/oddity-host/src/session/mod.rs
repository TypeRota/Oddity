//! 本轮游戏会话: 注册表, 状态投影与启动命令入口.
//!
//! 宿主只保留一份会话真相. 注册表持有启动进度, 取消令牌, 唯一会话记录与最近
//! 一次冷启动错误; 预留会话后把票据交给启动实现, 宿主能力见 host 子模块,
//! 管理器观察与收尾见 monitor 子模块.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use oddity_runtime::{
    EventSink, LaunchError, LaunchFailureKind, LaunchMode, LaunchProgress, LaunchResult,
    LaunchStage, SessionControl, SessionStatus, SessionTicket,
};
use serde::Serialize;

use crate::service::ServiceHandle;
use crate::settings::Settings;
use crate::AppState;

pub mod host;
pub(crate) mod monitor;
pub(crate) mod process;

/// 准备阶段被取消时的提示.
pub const LAUNCH_CANCELLED: &str = "启动已暂停。";

/// 游戏进程结束后是否按设置显示主界面.
pub(crate) fn should_show_launcher_after_exit(app: &ServiceHandle) -> bool {
    app.state
        .settings
        .lock()
        .map(|settings| settings.show_launcher_after_game_exit)
        .unwrap_or(false)
}

/// 启动设置快照; 外部启动的临时覆盖不写回设置.
pub fn launch_settings(state: &crate::AppState) -> Result<Settings, String> {
    state
        .settings
        .lock()
        .map_err(|error| error.to_string())
        .map(|settings| settings.clone())
}

pub fn show_main(app: &ServiceHandle) {
    let _ = app.emit("host-show", ());
}

/// 会话绑定的关闭与取消入口.
#[derive(Clone, Default)]
pub(crate) struct ControlSlot(pub(crate) Arc<Mutex<Option<Arc<dyn SessionControl>>>>);

impl std::fmt::Debug for ControlSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ControlSlot")
    }
}

/// 宿主会话状态: 唯一的启动/运行真相.
#[derive(Default)]
pub struct Registry {
    pub environment_busy: bool,
    pub migration_busy: bool,
    pub session: Option<Session>,
    pub starting: bool,
    pub next_id: u64,
    pub last_error: Option<String>,
    pub launch: Option<LaunchProgress>,
    pub launch_cancel: Option<Arc<AtomicBool>>,
}

impl Registry {
    /// 对外的状态投影, 也是 game-state 事件的载荷.
    pub fn status(&self) -> SessionStatus {
        let mut status = self
            .session
            .as_ref()
            .map_or_else(idle_status, Session::status);
        status.error = self.last_error.clone();
        status.launch = self.launch.clone();
        status
    }

    pub(crate) fn clear_launch(&mut self) {
        self.launch = None;
        self.launch_cancel = None;
    }

    pub(crate) fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    pub(crate) fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }
}

/// 一轮运行中的游戏会话; 会话即安装与 prefix 租约的持有者.
#[derive(Debug, Clone)]
pub struct Session {
    pub performance: Arc<Mutex<Option<crate::performance::Recording>>>,
    pub id: u64,
    pub pid: u32,
    pub pgid: i32,
    pub exe: String,
    pub started_at: u64,
    pub log: String,
    pub mode: String,
    pub distribution: crate::channel::ChannelId,
    pub game: crate::identity::GameId,
    pub closing: bool,
    pub exit_notified: bool,
    pub game_alive: Arc<AtomicBool>,
    pub monitor_done: Arc<AtomicBool>,
    pub finalized: Arc<AtomicBool>,
    pub _install_lock: Option<Arc<oddity_platform::install_lock::InstallLock>>,
    pub _prefix_lock: Option<Arc<crate::environment::PrefixLock>>,
    pub game_mode: Option<Arc<Mutex<crate::game_mode::Session>>>,
    pub snapshot: Option<String>,
    pub snapshot_success_after_ms: u64,
    pub launcher_root: Option<PathBuf>,
    pub(crate) monitor_state: Arc<Mutex<monitor::MonitorState>>,
    pub(crate) control: ControlSlot,
}

impl Session {
    pub(crate) fn status(&self) -> SessionStatus {
        SessionStatus {
            running: true,
            closing: self.closing,
            pid: Some(self.pid),
            started_at: Some(self.started_at),
            exe: Some(self.exe.clone()),
            log: Some(self.log.clone()),
            launch: None,
            error: None,
        }
    }

    pub(crate) fn control(&self) -> Option<Arc<dyn SessionControl>> {
        self.control
            .0
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
    }
}

/// 没有会话时的状态投影.
pub(crate) fn idle_status() -> SessionStatus {
    SessionStatus {
        running: false,
        closing: false,
        pid: None,
        started_at: None,
        exe: None,
        log: None,
        launch: None,
        error: None,
    }
}

/// 会话是否还活着.
///
/// 绑定控制入口后以启动实现为准 (后台会话要等清理完成); 没有控制入口时
/// 退回进程组与最近一次采样的游戏进程判断.
pub fn session_alive(session: &Session) -> bool {
    session.control().map_or_else(
        || {
            oddity_platform::process::group_alive(session.pgid)
                || session.game_alive.load(Ordering::Acquire)
        },
        |control| control.is_alive(),
    )
}

/// 发 game-state; 调用方必须已经释放注册表锁.
pub fn emit_game_status(app: &ServiceHandle, state: &AppState) {
    let status = state
        .game
        .lock()
        .map(|registry| registry.status())
        .unwrap_or_else(|_| idle_status());
    let _ = app.emit("game-state", status);
}

/// 监控线程只更新本轮会话的阶段, 防止迟到的旧回调覆盖新启动.
pub(crate) fn advance_launch(registry: &mut Registry, session_id: u64, stage: LaunchStage) -> bool {
    if registry.session.as_ref().map(|session| session.id) != Some(session_id) {
        return false;
    }
    registry
        .launch
        .as_mut()
        .is_some_and(|launch| launch.advance(stage))
}

/// 启动实现的阶段汇报: 只在进度推进时发事件.
pub(crate) fn publish_launch_stage(app: &ServiceHandle, session_id: u64, stage: LaunchStage) {
    let state = &app.state;
    let changed = match state.game.lock() {
        Ok(mut registry) => advance_launch(&mut registry, session_id, stage),
        Err(_) => false,
    };
    if changed {
        emit_game_status(app, state);
    }
}

/// 设置里的启动模式.
pub(crate) fn launch_mode(settings: &Settings) -> Result<LaunchMode, LaunchError> {
    match settings.current().launch_mode.as_str() {
        "official" => Ok(LaunchMode::Official),
        "background" => Ok(LaunchMode::Background),
        "direct" => Ok(LaunchMode::Direct),
        other => Err(LaunchError::new(
            LaunchFailureKind::Backend,
            format!("未知的启动模式：{other}"),
        )),
    }
}

/// 初始化本轮启动的进度与取消令牌, 只有后台模式需要进度.
pub fn begin_launch(registry: &mut Registry, settings: &Settings) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    registry.starting = true;
    let background = settings.current().launch_mode == "background";
    registry.launch = background.then(|| LaunchProgress::new(crate::now_millis()));
    registry.launch_cancel = background.then(|| Arc::clone(&cancel));
    cancel
}

pub(crate) fn reset_launch(registry: &mut Registry) {
    registry.starting = false;
    registry.clear_launch();
}

pub(crate) async fn launch_official(
    app: ServiceHandle,
    state: &AppState,
) -> Result<LaunchResult, String> {
    let settings = crate::session::launch_settings(state)?;
    start_launch(app, settings).await
}

pub fn prepare_launch(
    packages: &oddity_runtime::Registry,
    registry: &mut Registry,
) -> Result<(), String> {
    if packages.blocks_changes() {
        return Err("游戏资源任务正在进行，暂时无法启动游戏".into());
    }
    if registry.migration_busy || crate::migration_commands::repair_pending() {
        return Err("安装目录迁移尚未完成, 请先完成绑定".into());
    }
    if registry.environment_busy {
        return Err("运行环境任务正在进行, 请完成后再启动游戏".into());
    }
    if registry.starting {
        return Err("游戏正在启动，请稍候。".to_string());
    }
    if let Some(session) = registry.session.as_ref() {
        if session.closing {
            return Err("游戏正在关闭，请稍候。".to_string());
        }
        if session_alive(session) {
            return Err("游戏已在运行。".to_string());
        }
    }
    // 已结束的会话允许重新启动.
    registry.session = None;
    registry.last_error = None;
    registry.starting = true;
    Ok(())
}

/// 启动会话的公共入口. settings 可以带外部启动的临时模式覆盖, 不会写回设置.
pub async fn start_launch(app: ServiceHandle, settings: Settings) -> Result<LaunchResult, String> {
    let provider = app
        .state
        .packages
        .launch_provider(settings.active_game.as_str())
        .ok_or_else(|| {
            format!(
                "《{}》尚未支持启动.",
                crate::install::display_name(&app.state.packages, &settings.active_game)
            )
        })?;
    let target = oddity_runtime::SessionTarget {
        game: settings.active_game.as_str().to_string(),
        distribution: crate::channel::effective_id(
            &app.state.packages,
            settings.active_game.as_str(),
            settings.current().distribution.as_ref(),
        ),
        mode: launch_mode(&settings).map_err(|error| error.message)?,
    };
    provider.check(&target).map_err(|error| error.message)?;
    let (cancel, id) = {
        let state = &app.state;
        let mut registry = state.game.lock().map_err(|e| e.to_string())?;
        prepare_launch(&state.packages, &mut registry)?;
        let cancel = begin_launch(&mut registry, &settings);
        registry.next_id = registry.next_id.wrapping_add(1);
        (cancel, registry.next_id)
    };
    emit_game_status(&app, &app.state);

    let host = host::HostSession::new(app.clone(), settings.clone(), id, Arc::clone(&cancel));
    let ticket = SessionTicket::new(
        id.to_string(),
        Arc::clone(&host) as Arc<dyn oddity_runtime::SessionHost>,
    );
    let result = match crate::service::spawn_blocking(move || provider.launch(ticket)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            if let Ok(mut registry) = app.state.game.lock() {
                reset_launch(&mut registry);
            }
            emit_game_status(&app, &app.state);
            return Err(error.message);
        }
        Err(error) => {
            if let Ok(mut registry) = app.state.game.lock() {
                reset_launch(&mut registry);
            }
            emit_game_status(&app, &app.state);
            return Err(format!("启动任务失败：{error}"));
        }
    };
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GameModeStatus {
    pub(crate) capabilities: Vec<crate::game_mode::Status>,
    pub(crate) session: Option<Vec<crate::game_mode::Status>>,
}

pub(crate) async fn game_mode_status(state: &AppState) -> Result<GameModeStatus, String> {
    let mode = state
        .game
        .lock()
        .map_err(|error| error.to_string())?
        .session()
        .and_then(|session| session.game_mode.clone());
    crate::service::spawn_blocking(move || {
        let session = mode
            .map(|mode| {
                mode.lock()
                    .map(|mode| mode.status())
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
        Ok(GameModeStatus {
            capabilities: crate::game_mode::probe(),
            session,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn game_status(state: &AppState) -> SessionStatus {
    let registry = match state.game.lock() {
        Ok(registry) => registry,
        Err(_) => return idle_status(),
    };
    let mut status = registry.status();
    if registry
        .session()
        .is_some_and(|session| !session_alive(session))
    {
        status.running = false;
        status.closing = false;
        status.launch = None;
    }
    status
}

/// 冷启动失败且桌面通知不可用时, 前端初始化后取走错误并提示.
pub(crate) fn take_launch_error(state: &AppState) -> Option<String> {
    state
        .game
        .lock()
        .ok()
        .and_then(|mut registry| registry.take_error())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchCancelAction {
    Close,
    Wait,
    AlreadyClosing,
    Idle,
}

pub(crate) fn launch_cancel_action(registry: &mut Registry) -> LaunchCancelAction {
    if let Some(session) = registry.session.as_mut() {
        if let Some(launch) = registry.launch.as_mut() {
            launch.cancelling = true;
        }
        return if session.closing {
            LaunchCancelAction::AlreadyClosing
        } else {
            LaunchCancelAction::Close
        };
    }
    if registry.starting && registry.launch_cancel.is_some() {
        if let Some(cancel) = registry.launch_cancel.as_ref() {
            cancel.store(true, Ordering::Release);
        }
        if let Some(launch) = registry.launch.as_mut() {
            launch.cancelling = true;
        }
        return LaunchCancelAction::Wait;
    }
    LaunchCancelAction::Idle
}

pub(crate) async fn cancel_launch(
    app: ServiceHandle,
    state: &AppState,
) -> Result<SessionStatus, String> {
    let action = {
        let mut registry = state.game.lock().map_err(|error| error.to_string())?;
        launch_cancel_action(&mut registry)
    };
    match action {
        LaunchCancelAction::Close => close_game(app, state).await,
        LaunchCancelAction::Wait | LaunchCancelAction::AlreadyClosing => {
            let status = state
                .game
                .lock()
                .map(|registry| registry.status())
                .unwrap_or_else(|_| idle_status());
            let _ = app.emit("game-state", status.clone());
            Ok(status)
        }
        LaunchCancelAction::Idle => {
            if let Ok(mut registry) = state.game.lock() {
                registry.clear_launch();
            }
            Ok(idle_status())
        }
    }
}

pub(crate) fn finish_game_close(registry: &mut Registry, session_id: u64, success: bool) {
    if registry.session.as_ref().map(|session| session.id) != Some(session_id) {
        return;
    }
    if success {
        registry.last_error = None;
        registry.session = None;
        registry.clear_launch();
    } else if let Some(session) = registry.session.as_mut() {
        session.closing = false;
        if let Some(launch) = registry.launch.as_mut() {
            launch.cancelling = false;
        }
    }
}

pub async fn close_game(app: ServiceHandle, state: &AppState) -> Result<SessionStatus, String> {
    let (session, control, pgid, game_alive) = {
        let mut registry = state.game.lock().map_err(|e| e.to_string())?;
        if registry.starting {
            return Err("游戏正在启动，请稍候。".to_string());
        }
        match registry.session.as_mut() {
            Some(session) if session.closing => {
                return Err("游戏正在关闭，请稍候。".to_string());
            }
            Some(session) => {
                if !session_alive(session) {
                    let expired = session.clone();
                    registry.session = None;
                    registry.clear_launch();
                    drop(registry);
                    monitor::finalize_game_session(
                        &app,
                        &expired,
                        Some(crate::snapshots::Failure::early_exit(None)),
                    );
                    let status = idle_status();
                    let _ = app.emit("game-state", status.clone());
                    return Ok(status);
                }
                session.closing = true;
                (
                    session.clone(),
                    session.control(),
                    session.pgid,
                    Arc::clone(&session.game_alive),
                )
            }
            None => return Ok(idle_status()),
        }
    };

    emit_game_status(&app, state);

    let result = match crate::service::spawn_blocking(move || {
        if let Some(control) = control {
            return control.close();
        }
        oddity_platform::process::terminate_group(pgid)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while game_alive.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if game_alive.load(Ordering::Acquire) {
            Err("游戏进程仍在运行, 请退出游戏后重试".into())
        } else {
            Ok(())
        }
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(format!("关闭任务失败：{error}")),
    };

    if result.is_ok() {
        monitor::finalize_game_session(
            &app,
            &session,
            Some(crate::snapshots::Failure::early_exit(None)),
        );
    }

    {
        let mut registry = state.game.lock().map_err(|e| e.to_string())?;
        finish_game_close(&mut registry, session.id, result.is_ok());
    }

    let status = state
        .game
        .lock()
        .map(|registry| registry.status())
        .unwrap_or_else(|_| idle_status());
    let _ = app.emit("game-state", status.clone());
    result?;
    Ok(status)
}
