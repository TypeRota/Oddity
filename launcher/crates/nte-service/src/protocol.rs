//! 命令参数在此反序列化一次, 业务层只接收具体类型.
use crate::service::ServiceHandle;
use nte_core::game_profile::GameId;
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: String,
    /// 命令所属游戏; 旧调用没有该字段时使用《异环》.
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub args: Value,
}

/// bootstrap 用于让界面发现当前游戏, 不能要求调用方先知道游戏.
fn requires_current_game(command: &str) -> bool {
    matches!(
        command,
        "attendance_refresh_game_records"
            | "attendance_login"
            | "attendance_send_code"
            | "attendance_login_sms"
            | "attendance_run"
            | "save_settings"
            | "check_update"
            | "fetch_bulletins"
            | "launcher_background"
            | "launch_official"
            | "cancel_launch"
            | "game_status"
            | "take_launch_error"
            | "close_game"
            | "environment_create"
            | "environment_action"
            | "migration_plan"
            | "migration_start"
            | "migration_status"
            | "migration_cancel"
            | "migration_cleanup"
            | "migration_abandon"
            | "migration_dismiss"
            | "migration_retry"
            | "environment_snapshot_overview"
            | "environment_snapshot_compare"
            | "environment_snapshot_save_reference"
            | "environment_snapshot_delete_reference"
            | "environment_snapshot_clear_history"
            | "game_mode_gpu_info"
            | "game_mode_gpu_apply"
            | "game_mode_tuning_info"
            | "game_mode_tuning_apply"
            | "game_mode_status"
            | "game_resource_catalog"
            | "game_preload_delete"
            | "game_resource_select"
            | "game_download_status"
            | "game_download_start"
            | "game_download_cancel"
    )
}

pub async fn dispatch(
    app: ServiceHandle,
    command: String,
    game: Option<String>,
    args: Value,
) -> Result<Value, Value> {
    let game = match game.as_deref() {
        Some(value) => value.parse::<GameId>().map_err(|error| json!(error))?,
        None => GameId::default(),
    };
    if requires_current_game(&command) {
        let current = app
            .state
            .settings
            .lock()
            .map_err(|error| json!(error.to_string()))?
            .active_game;
        if game != current {
            return Err(json!(format!(
                "请求的游戏是《{}》, 当前游戏是《{}》. 请刷新界面后重试.",
                game.profile().name,
                current.profile().name
            )));
        }
    }
    match command.as_str() {
        "attendance_status" => serde_json::to_value(
            crate::attendance_commands::attendance_status()
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "attendance_refresh_game_records" => serde_json::to_value(
            crate::attendance_commands::attendance_refresh_game_records(&app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "attendance_login" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
                password: String,
                remember_password: bool,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::attendance_commands::attendance_login(
                    &app.state,
                    args.phone,
                    args.password,
                    args.remember_password,
                )
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "attendance_send_code" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::attendance_commands::attendance_send_code(&app.state, args.phone)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "attendance_login_sms" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
                code: String,
                device_id: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::attendance_commands::attendance_login_sms(
                    &app.state,
                    args.phone,
                    args.code,
                    args.device_id,
                )
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "attendance_configure" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                preferences: nte_core::attendance::Preferences,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::attendance_commands::attendance_configure(args.preferences)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "attendance_run" => serde_json::to_value(
            crate::attendance_commands::attendance_run(&app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "attendance_logout" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                account_id: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::attendance_commands::attendance_logout(args.account_id)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "attendance_keyring_check" => serde_json::to_value(
            crate::attendance_commands::attendance_keyring_check()
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_resource_catalog" => serde_json::to_value(
            crate::service::spawn_blocking(move || {
                crate::download_commands::game_resource_catalog(app)
            })
            .await
            .map_err(|e| json!(e.to_string()))?
            .map_err(|e| json!(e))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_preload_delete" => serde_json::to_value(
            crate::service::spawn_blocking(move || {
                crate::download_commands::game_preload_delete(app)
            })
            .await
            .map_err(|e| json!(e.to_string()))?
            .map_err(|e| json!(e))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_resource_select" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                tags: Vec<String>,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::service::spawn_blocking(move || {
                    crate::download_commands::game_resource_select(app, args.tags)
                })
                .await
                .map_err(|e| json!(e.to_string()))?
                .map_err(|e| json!(e))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "game_download_status" => serde_json::to_value(
            crate::download_commands::game_download_status(&app.downloads),
        )
        .map_err(|e| json!(e.to_string())),
        "game_download_start" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                mode: String,
                full: bool,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::service::spawn_blocking(move || {
                    crate::download_commands::game_download_start(app, args.mode, args.full)
                })
                .await
                .map_err(|e| json!(e.to_string()))?
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "game_download_cancel" => serde_json::to_value(
            crate::download_commands::game_download_cancel(&app.downloads),
        )
        .map_err(|e| json!(e.to_string())),
        "environment_create" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                name: String,
                proton_path: String,
                source_id: Option<String>,
                distribution: nte_core::distribution::Distribution,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::environment_commands::environment_create(
                    app.clone(),
                    args.name,
                    args.proton_path,
                    args.source_id,
                    args.distribution,
                )
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "migration_plan" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                distribution: nte_core::distribution::Distribution,
                target: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::service::spawn_blocking(move || {
                    crate::migration_commands::plan(&app, args.distribution, &args.target)
                })
                .await
                .map_err(|e| json!(e.to_string()))?
                .map_err(|e| json!(e))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "migration_start" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                distribution: nte_core::distribution::Distribution,
                target: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::migration_commands::start(app, args.distribution, args.target)
                    .await
                    .map_err(|e| json!(e))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "migration_status" => serde_json::to_value(crate::migration_commands::current(&app))
            .map_err(|e| json!(e.to_string())),
        "migration_cancel" => serde_json::to_value(crate::migration_commands::cancel(&app))
            .map_err(|e| json!(e.to_string())),
        "migration_cleanup" => serde_json::to_value(
            crate::migration_commands::cleanup(app.clone()).map_err(|e| json!(e))?,
        )
        .map_err(|e| json!(e.to_string())),
        "migration_abandon" => serde_json::to_value(
            crate::service::spawn_blocking(move || crate::migration_commands::abandon(&app))
                .await
                .map_err(|e| json!(e.to_string()))?
                .map_err(|e| json!(e))?,
        )
        .map_err(|e| json!(e.to_string())),
        "migration_dismiss" => {
            serde_json::to_value(crate::migration_commands::dismiss(&app).map_err(|e| json!(e))?)
                .map_err(|e| json!(e.to_string()))
        }
        "migration_retry" => serde_json::to_value(
            crate::service::spawn_blocking(move || crate::migration_commands::retry(&app))
                .await
                .map_err(|e| json!(e.to_string()))?
                .map_err(|e| json!(e))?,
        )
        .map_err(|e| json!(e.to_string())),
        "environment_action" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                action: String,
                id: Option<String>,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::environment_commands::environment_action(app.clone(), args.action, args.id)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "browse_files" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                path: Option<String>,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::file_commands::browse_files(args.path)
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "game_mode_gpu_info" => serde_json::to_value(
            crate::game_mode_commands::game_mode_gpu_info()
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_mode_gpu_apply" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                config: nte_core::game_mode::gpu::Config,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::game_mode_commands::game_mode_gpu_apply(&app.state, args.config)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "game_mode_tuning_info" => serde_json::to_value(
            crate::game_mode_commands::game_mode_tuning_info()
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_mode_tuning_apply" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                config: nte_core::game_mode::tuning::Config,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::game_mode_commands::game_mode_tuning_apply(&app.state, args.config)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "gamepad_poll" => serde_json::to_value(
            crate::gamepad_commands::gamepad_poll(app.clone())
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "gamepad_feedback" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                device_id: u32,
                effect: crate::gamepad_commands::Feedback,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::gamepad_commands::gamepad_feedback(app.clone(), args.device_id, args.effect)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "gamepad_profile_validate" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                profile: nte_core::gamepad::Profile,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::gamepad_commands::gamepad_profile_validate(args.profile)
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "gamepad_profile_load" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                path: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::gamepad_commands::gamepad_profile_load(args.path)
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "gamepad_profile_save" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                path: String,
                profile: nte_core::gamepad::Profile,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::gamepad_commands::gamepad_profile_save(args.path, args.profile)
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "gamepad_release_idle" => serde_json::to_value(
            crate::gamepad_commands::gamepad_release_idle(app.clone())
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "prepare_log_export" => serde_json::to_value(
            crate::log_commands::prepare_log_export(&app.exports)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "save_log_export" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                id: String,
                destination: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::log_commands::save_log_export(&app.exports, args.id, args.destination)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "discard_log_export" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                id: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::log_commands::discard_log_export(&app.exports, args.id)
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "bootstrap" => serde_json::to_value(
            crate::service::spawn_blocking(move || crate::bootstrap(&app))
                .await
                .map_err(|e| json!(e.to_string()))?,
        )
        .map_err(|e| json!(e.to_string())),
        "integrity_acknowledge" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                notice: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            crate::integrity_commands::acknowledge(&app, &args.notice)
                .map_err(|error| json!(error))?;
            Ok(Value::Null)
        }
        "launcher_background" => serde_json::to_value(
            crate::service::spawn_blocking(move || crate::launcher_background(&app.state))
                .await
                .map_err(|e| json!(e.to_string()))?,
        )
        .map_err(|e| json!(e.to_string())),
        "check_update" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                full: bool,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::check_update(&app.state, args.full)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "fetch_bulletins" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                distribution: nte_core::distribution::Distribution,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::fetch_bulletins(args.distribution)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "save_settings" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                settings: crate::Settings,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::service::spawn_blocking(move || crate::save_settings(&app, args.settings))
                    .await
                    .map_err(|e| json!(e.to_string()))?
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "open_url" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                url: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::open_url(args.url)
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "open_directory" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                path: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::open_directory(args.path)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "launch_official" => serde_json::to_value(
            crate::launch_official(app.clone(), &app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_mode_status" => serde_json::to_value(
            crate::game_mode_status(&app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_status" => {
            serde_json::to_value(crate::game_status(&app.state)).map_err(|e| json!(e.to_string()))
        }
        "take_launch_error" => serde_json::to_value(crate::take_launch_error(&app.state))
            .map_err(|e| json!(e.to_string())),
        "cancel_launch" => serde_json::to_value(
            crate::cancel_launch(app.clone(), &app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "close_game" => serde_json::to_value(
            crate::close_game(app.clone(), &app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "playtime_history" => serde_json::to_value(
            crate::playtime_commands::playtime_history(&app.playtime)
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "playtime_export_poster" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                destination: String,
                data: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::playtime_commands::playtime_export_poster(args.destination, args.data)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "environment_snapshot_overview" => serde_json::to_value(
            crate::snapshot_commands::environment_snapshot_overview(&app.snapshots)
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "environment_snapshot_compare" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                target: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::snapshot_commands::environment_snapshot_compare(app.clone(), args.target)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "environment_snapshot_save_reference" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                label: Option<String>,
                attempt_id: Option<String>,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::snapshot_commands::environment_snapshot_save_reference(
                    app.clone(),
                    args.label,
                    args.attempt_id,
                )
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "environment_snapshot_delete_reference" => serde_json::to_value(
            crate::snapshot_commands::environment_snapshot_delete_reference(&app.snapshots)
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "environment_snapshot_clear_history" => serde_json::to_value(
            crate::snapshot_commands::environment_snapshot_clear_history(&app.snapshots)
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "host_state" => {
            #[derive(Deserialize)]
            struct Args {
                focused: bool,
                visible: bool,
            }
            let state: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            app.focused
                .store(state.focused, std::sync::atomic::Ordering::Release);
            app.visible
                .store(state.visible, std::sync::atomic::Ordering::Release);
            if !state.focused || !state.visible {
                app.input_task(crate::gamepad_commands::stop)
                    .map_err(|e| json!(e))?;
            }
            Ok(Value::Null)
        }
        "external_launch" => {
            #[derive(Deserialize)]
            struct Args {
                argv: Vec<String>,
                cold: bool,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            let argv = args
                .argv
                .into_iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>();
            let request = crate::cli::parse(&argv).map_err(|e| json!(e))?;
            if let Some(request) = request {
                crate::handle_external_launch(
                    app,
                    request,
                    if args.cold {
                        crate::LaunchOrigin::ColdStart
                    } else {
                        crate::LaunchOrigin::Forwarded
                    },
                )
                .await;
            }
            Ok(Value::Null)
        }
        _ => Err(json!("无法识别的宿主命令")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_context_defaults_to_nte_and_must_match_the_current_game() {
        let app = crate::service::test_service();
        let status =
            crate::service::block_on(dispatch(app.clone(), "game_status".into(), None, json!({})))
                .unwrap();
        assert_eq!(status["running"], false);
        assert!(crate::service::block_on(dispatch(
            app.clone(),
            "game_status".into(),
            Some("nte".into()),
            json!({}),
        ))
        .is_ok());

        app.state.settings.lock().unwrap().active_game = GameId::Hsr;
        let error = crate::service::block_on(dispatch(
            app.clone(),
            "game_status".into(),
            Some("nte".into()),
            json!({}),
        ))
        .unwrap_err();
        assert!(
            error
                .as_str()
                .unwrap()
                .contains("当前游戏是《崩坏:星穹铁道》"),
            "{error}"
        );
        assert!(crate::service::block_on(dispatch(
            app,
            "game_status".into(),
            Some("hsr".into()),
            json!({}),
        ))
        .is_ok());
    }

    #[test]
    fn unknown_game_is_rejected_with_a_clear_error() {
        let app = crate::service::test_service();
        let error = crate::service::block_on(dispatch(
            app,
            "game_status".into(),
            Some("genshin".into()),
            json!({}),
        ))
        .unwrap_err();
        let message = error.as_str().unwrap();
        assert!(message.contains("无法识别游戏: genshin"), "{message}");
        assert!(message.contains("nte, hsr"), "{message}");
    }

    #[test]
    fn bootstrap_discovers_the_current_game_without_validation() {
        assert!(!requires_current_game("bootstrap"));
        assert!(!requires_current_game("external_launch"));
        assert!(requires_current_game("game_status"));
        assert!(requires_current_game("take_launch_error"));
    }
}
