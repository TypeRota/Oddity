use nte_core::game_auth::{self, AccountSummary, Client, Vault};
use nte_core::{distribution::Distribution, game_profile::GameId, settings::Settings};

pub fn require_supported(settings: &Settings) -> Result<(), String> {
    if settings.active_game != GameId::Nte || settings.current().distribution != Distribution::Cn {
        return Err("直接启动和游戏账号登录目前仅支持《异环》国服, 请使用后台或前台启动.".into());
    }
    Ok(())
}
pub async fn accounts(state: &crate::AppState) -> game_auth::Result<Vec<AccountSummary>> {
    let settings = crate::launch_settings(state).map_err(|message| game_auth::Failure {
        kind: "service",
        message,
    })?;
    require_supported(&settings).map_err(|message| game_auth::Failure {
        kind: "unsupported",
        message,
    })?;
    crate::service::spawn_blocking(Vault::summaries)
        .await
        .map_err(|_| game_auth::Failure {
            kind: "service",
            message: "读取游戏账号任务异常.".into(),
        })?
}
fn launcher_root(state: &crate::AppState) -> game_auth::Result<std::path::PathBuf> {
    let settings = crate::launch_settings(state).map_err(|message| game_auth::Failure {
        kind: "service",
        message,
    })?;
    require_supported(&settings).map_err(|message| game_auth::Failure {
        kind: "unsupported",
        message,
    })?;
    crate::detect_launcher_root(&settings).ok_or_else(|| game_auth::Failure {
        kind: "unsupported",
        message: "请先安装官方组件和游戏资源.".into(),
    })
}
pub async fn send_code(state: &crate::AppState, phone: String) -> game_auth::Result<String> {
    let root = launcher_root(state)?;
    crate::service::spawn_blocking(move || Client::load(&root)?.send_code(phone.trim()))
        .await
        .map_err(|_| game_auth::Failure {
            kind: "service",
            message: "获取游戏验证码任务异常.".into(),
        })?
}
pub async fn login(
    state: &crate::AppState,
    phone: String,
    secret: String,
    password: bool,
    device_id: String,
) -> game_auth::Result<AccountSummary> {
    let secret = zeroize::Zeroizing::new(secret);
    let root = launcher_root(state)?;
    crate::service::spawn_blocking(move || {
        Client::load(&root)?.login(phone.trim(), &secret, password, device_id)
    })
    .await
    .map_err(|_| game_auth::Failure {
        kind: "service",
        message: "游戏账号认证任务异常.".into(),
    })?
}
pub async fn remove(state: &crate::AppState, id: String) -> game_auth::Result<()> {
    let settings = crate::launch_settings(state).map_err(|message| game_auth::Failure {
        kind: "service",
        message,
    })?;
    require_supported(&settings).map_err(|message| game_auth::Failure {
        kind: "unsupported",
        message,
    })?;
    crate::service::spawn_blocking(move || Vault::delete(&id))
        .await
        .map_err(|_| game_auth::Failure {
            kind: "service",
            message: "删除游戏账号任务异常.".into(),
        })?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_distribution_stops_before_account_or_network_access() {
        let mut app = crate::service::test_service();
        std::sync::Arc::get_mut(&mut app)
            .unwrap()
            .state
            .settings
            .get_mut()
            .unwrap()
            .current_mut()
            .distribution = Distribution::Global;
        crate::service::block_on(async {
            for error in [
                accounts(&app.state).await.err(),
                send_code(&app.state, String::new()).await.err(),
                login(
                    &app.state,
                    String::new(),
                    String::new(),
                    false,
                    String::new(),
                )
                .await
                .err(),
                remove(&app.state, String::new()).await.err(),
            ] {
                assert_eq!(error.unwrap().kind, "unsupported");
            }
        });
    }
}
