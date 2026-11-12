//! 外部启动入口: 从命令行转发来的启动请求与失败兜底.
//!
//! 冷启动与转发共用同一条路径; 失败先尝试桌面通知, 通知不可用时回到主界面.

use crate::service::ServiceHandle;
use crate::{bootstrap, channel, cli, environment_commands, install, session};
use oddity_runtime::EventSink;
use oddity_runtime::LaunchResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchOrigin {
    ColdStart,
    Forwarded,
}

pub(crate) async fn start_external_launch(
    app: &ServiceHandle,
    request: &cli::LaunchRequest,
) -> Result<LaunchResult, String> {
    if install::regions(&app.state.packages, &request.game).is_empty() {
        return Err(format!(
            "《{}》尚未支持启动.",
            install::display_name(&app.state.packages, &request.game)
        ));
    }
    let selected_app = app.clone();
    let game = request.game.clone();
    let region = request.region.clone();
    let environment = request.environment.clone();
    let selection = crate::service::spawn_blocking(move || {
        let selection = environment_commands::select_for_launch(
            &selected_app.state,
            &game,
            region,
            environment.as_deref(),
        )?;
        if selection.changed {
            let _ = selected_app.emit("settings-changed", bootstrap::bootstrap(&selected_app));
        }
        Ok::<_, String>(selection)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut settings = selection.settings;
    settings.current_mut().launch_mode = request.mode.as_str().to_string();
    let result = session::start_launch(app.clone(), settings).await?;
    let _ = app.emit("host-hide", ());
    Ok(result)
}

pub(crate) async fn handle_external_launch(
    app: ServiceHandle,
    request: cli::LaunchRequest,
    origin: LaunchOrigin,
) {
    let summary = launch_failure_summary(&app, &request);
    if let Err(error) = start_external_launch(&app, &request).await {
        eprintln!("外部启动失败: {error}");
        let body = notification_body(&error);
        let notified = crate::service::spawn_blocking(move || {
            oddity_platform::notifications::send(&summary, &body)
        })
        .await
        .map(|result| result.is_ok())
        .unwrap_or(false);
        if notified {
            if origin == LaunchOrigin::ColdStart {
                let _ = app.emit("host-exit", 1);
            }
            return;
        }
        // 通知服务不可用时退回主界面提示, 并保留错误供前端初始化时读取.
        if let Ok(mut tracker) = app.state.game.lock() {
            tracker.last_error = Some(error);
        }
        session::show_main(&app);
        // 冷启动时前端可能还没订阅事件, 由初始化阶段调用 take_launch_error 兜底.
        let state = &app.state;
        session::emit_game_status(&app, state);
    }
}

pub(crate) fn launch_failure_summary(app: &ServiceHandle, request: &cli::LaunchRequest) -> String {
    let registry = &app.state.packages;
    let distribution = request.region.clone().or_else(|| {
        let state = &app.state;
        let settings = state.settings.lock().ok()?;
        if let Some(value) = request.environment.as_deref() {
            if let Some(environment) = settings
                .runtime_environments
                .iter()
                .find(|entry| entry.id == value || entry.name == value)
                .filter(|entry| entry.game == request.game)
            {
                return Some(environment.distribution.clone());
            }
        }
        settings.game(request.game.clone()).distribution.clone()
    });
    let name = distribution
        .map(|distribution| channel::label(registry, request.game.as_str(), &distribution))
        .unwrap_or_else(|| install::display_name(registry, &request.game).to_string());
    format!("{name}启动失败")
}

pub(crate) fn notification_body(message: &str) -> String {
    const LIMIT: usize = 800;
    let mut body: String = message.chars().take(LIMIT).collect();
    if message.chars().count() > LIMIT {
        body.push('…');
    }
    body
}
