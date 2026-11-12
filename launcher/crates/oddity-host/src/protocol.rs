//! 命令参数在此反序列化一次, 业务层只接收具体类型.
use crate::identity::GameId;
use crate::service::ServiceHandle;
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: String,
    /// 命令所属游戏; 旧调用没有该字段时使用设置里的当前游戏.
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub args: Value,
}

/// 显式渠道入口校验: 值域由当前游戏包的声明决定.
fn ensure_known_channel(
    app: &ServiceHandle,
    channel: &crate::channel::ChannelId,
) -> Result<(), Value> {
    let game = app
        .state
        .settings
        .lock()
        .map_err(|error| json!(error.to_string()))?
        .active_game
        .clone();
    crate::channel::resolve(&app.state.packages, game.as_str(), Some(channel))
        .map(|_| ())
        .map_err(|error| json!(error.to_string()))
}

/// 当前游戏校验: 非发现类命令要求请求的游戏与当前游戏一致.
fn ensure_current_game(app: &ServiceHandle, game: &GameId) -> Result<(), Value> {
    let current = app
        .state
        .settings
        .lock()
        .map_err(|error| json!(error.to_string()))?
        .active_game
        .clone();
    if game != &current {
        return Err(json!(format!(
            "请求的游戏是《{}》, 当前游戏是《{}》. 请刷新界面后重试.",
            crate::install::display_name(&app.state.packages, game),
            crate::install::display_name(&app.state.packages, &current)
        )));
    }
    Ok(())
}

pub async fn dispatch(
    app: ServiceHandle,
    command: String,
    game: Option<String>,
    args: Value,
) -> Result<Value, Value> {
    let game = match game.as_deref() {
        Some(value) => {
            let known = app.state.packages.selectable_games();
            GameId::parse_known(value, &known).map_err(|error| json!(error))?
        }
        // 旧调用没有 game 字段时沿用设置里的当前游戏.
        None => app
            .state
            .settings
            .lock()
            .map_err(|error| json!(error.to_string()))?
            .active_game
            .clone(),
    };
    // 命令组接管后, 组命令在路由时校验当前游戏, 宿主 arm 仍在 match 前校验;
    // 装配校验保证同一条命令只有一个来源.
    let packages = std::sync::Arc::clone(&app.state.packages);
    let group = packages.group_for(game.as_str(), &command);
    if group.is_none() && oddity_contract::requires_current_game(&command) {
        ensure_current_game(&app, &game)?;
    }
    match command.as_str() {
        "runtime_setup_info" => {
            serde_json::to_value(crate::runtime_setup_commands::info(&app).map_err(|e| json!(e))?)
                .map_err(|e| json!(e.to_string()))
        }
        "runtime_setup_status" => {
            serde_json::to_value(crate::runtime_setup_commands::status(&app).map_err(|e| json!(e))?)
                .map_err(|e| json!(e.to_string()))
        }
        "runtime_setup_dismiss" => {
            crate::runtime_setup_commands::dismiss().map_err(|e| json!(e))?;
            Ok(Value::Null)
        }
        "runtime_setup_plan" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                all_users: bool,
                proton: Option<String>,
                create_environment: bool,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            crate::service::spawn_blocking(move || {
                crate::runtime_setup_commands::plan(
                    &app,
                    args.all_users,
                    args.proton,
                    args.create_environment,
                )
            })
            .await
            .map_err(|e| json!(e.to_string()))?
            .map_err(|e| json!(e))
            .and_then(|p| serde_json::to_value(p).map_err(|e| json!(e.to_string())))
        }
        "runtime_setup_apply" => {
            crate::runtime_setup_commands::apply(app)
                .await
                .map_err(|e| json!(e))?;
            Ok(Value::Null)
        }
        "runtime_setup_environment" => serde_json::to_value(
            crate::runtime_setup_commands::ensure_environment(app)
                .await
                .map_err(|e| json!(e))?,
        )
        .map_err(|e| json!(e.to_string())),
        "environment_create" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                name: String,
                proton_path: String,
                source_id: Option<String>,
                distribution: crate::channel::ChannelId,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            ensure_known_channel(&app, &args.distribution)?;
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
                distribution: crate::channel::ChannelId,
                target: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            ensure_known_channel(&app, &args.distribution)?;
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
                distribution: crate::channel::ChannelId,
                target: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            ensure_known_channel(&app, &args.distribution)?;
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
                config: crate::game_mode::gpu::Config,
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
                config: crate::game_mode::tuning::Config,
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
                profile: crate::gamepad::Profile,
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
                profile: crate::gamepad::Profile,
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
            crate::service::spawn_blocking(move || crate::bootstrap::bootstrap(&app))
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
        "update_settings_section" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                owner: String,
                patch: Value,
            }
            let args: Args = serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            serde_json::to_value(
                crate::service::spawn_blocking(move || {
                    crate::settings::update_settings_section(&app, args.owner, args.patch)
                })
                .await
                .map_err(|e| json!(e.to_string()))?
                .map_err(|e| json!(e))?,
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
                crate::file_commands::open_url(args.url)
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
                crate::file_commands::open_directory(args.path)
                    .await
                    .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
            )
            .map_err(|e| json!(e.to_string()))
        }
        "launch_official" => serde_json::to_value(
            crate::session::launch_official(app.clone(), &app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "audio_status" => {
            serde_json::to_value(app.audio.status()).map_err(|e| json!(e.to_string()))
        }
        "game_mode_status" => serde_json::to_value(
            crate::session::game_mode_status(&app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "game_status" => serde_json::to_value(crate::session::game_status(&app.state))
            .map_err(|e| json!(e.to_string())),
        "take_launch_error" => serde_json::to_value(crate::session::take_launch_error(&app.state))
            .map_err(|e| json!(e.to_string())),
        "cancel_launch" => serde_json::to_value(
            crate::session::cancel_launch(app.clone(), &app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "close_game" => serde_json::to_value(
            crate::session::close_game(app.clone(), &app.state)
                .await
                .map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null))?,
        )
        .map_err(|e| json!(e.to_string())),
        "performance_overview"
        | "performance_recording"
        | "performance_pin"
        | "performance_delete"
        | "performance_report"
        | "performance_export" => {
            let args: crate::performance_commands::Args =
                serde_json::from_value(args).map_err(|e| json!(e.to_string()))?;
            crate::performance_commands::command(
                app.clone(),
                command.trim_start_matches("performance_").to_string(),
                args,
            )
            .await
            .map_err(|e| json!(e))
        }
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
            let request = crate::cli::parse(&argv, &app.state.packages).map_err(|e| json!(e))?;
            if let Some(request) = request {
                crate::external_launch::handle_external_launch(
                    app,
                    request,
                    if args.cold {
                        crate::external_launch::LaunchOrigin::ColdStart
                    } else {
                        crate::external_launch::LaunchOrigin::Forwarded
                    },
                )
                .await;
            }
            Ok(Value::Null)
        }
        _ => match group {
            Some(group) => dispatch_to_group(group, app, game, &command, args).await,
            None if packages.claims(&command)
                || oddity_contract::command(&command)
                    .is_some_and(|declared| declared.owner.is_some()) =>
            {
                Err(unavailable_command(&app, &game, &command))
            }
            None => Err(json!("无法识别的宿主命令")),
        },
    }
}

/// 命令存在, 但不属于当前游戏时的错误.
///
/// 已装配的游戏报"不提供"; 当前游戏未装配时, 若命令所属包也未装配就报该包,
/// 否则报当前游戏. 包归属来自契约, 包被移出装配表后仍能落到这里.
fn unavailable_command(app: &ServiceHandle, game: &GameId, command: &str) -> Value {
    if app.state.packages.package(game.as_str()).is_some() {
        return json!(format!(
            "《{}》不提供命令 {command}.",
            crate::install::display_name(&app.state.packages, game)
        ));
    }
    let owner = oddity_contract::command(command).and_then(|declared| declared.owner);
    match owner {
        Some(owner) if app.state.packages.package(owner).is_none() => {
            json!(format!("游戏包 {owner} 未装配, 不处理命令 {command}."))
        }
        _ => json!(format!("当前游戏《{game}》未装配, 不处理命令 {command}.")),
    }
}

/// 把命令交给命令组处理.
///
/// 组命令与宿主 arm 走同一条当前游戏校验; 装配校验保证同一条命令只有一个来源.
async fn dispatch_to_group(
    group: &oddity_runtime::CommandGroup,
    app: ServiceHandle,
    game: GameId,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    if oddity_contract::requires_current_game(command) {
        ensure_current_game(&app, &game)?;
    }
    let host = crate::host::HostHandle(app);
    group.dispatch(&host, command, args).await
}

/// 宿主 dispatch 覆盖的命令 id.
///
/// 契约里没有包归属的非窗口命令由宿主处理; 包命令由命令组接管,
/// 装配校验据此判断哪条命令该由谁认领.
pub fn dispatch_command_ids() -> Vec<String> {
    oddity_contract::COMMANDS
        .iter()
        .filter(|command| {
            command.audience != oddity_contract::Audience::Window && command.owner.is_none()
        })
        .map(|command| command.id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 扫描 dispatch 源码里的 match arm, 核对契约派生的宿主命令都有实际分支.
    ///
    /// 契约的 owner 只说明命令该由谁处理, 不能证明 dispatch 里真的写了 arm.
    fn scanned_dispatch_command_ids() -> Vec<String> {
        const SOURCE: &str = include_str!("protocol.rs");
        let marker = "match command.as_str() {";
        let start = SOURCE.find(marker).expect("dispatch 缺少命令 match") + marker.len();
        let fallback = "无法识别的宿主命令";
        let end = SOURCE[start..]
            .find(fallback)
            .map(|offset| start + offset)
            .expect("dispatch 缺少兜底分支");
        top_level_strings(&SOURCE[start..end])
    }

    /// 跳过字符串和注释, 只在括号深度为 0 时记录字面量;
    /// 这样 arm 体里的提示文案不会被当成命令.
    fn top_level_strings(body: &str) -> Vec<String> {
        let bytes = body.as_bytes();
        let mut strings = Vec::new();
        let mut depth = 0usize;
        let mut index = 0usize;
        while index < bytes.len() {
            match bytes[index] {
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b'"' => {
                    index += 1;
                    let mut value = Vec::new();
                    while index < bytes.len() && bytes[index] != b'"' {
                        if bytes[index] == b'\\' {
                            value.push(bytes[index + 1]);
                            index += 2;
                        } else {
                            value.push(bytes[index]);
                            index += 1;
                        }
                    }
                    index += 1;
                    if depth == 0 {
                        strings.push(String::from_utf8(value).expect("命令 id 必须是 UTF-8"));
                    }
                }
                b'{' | b'(' | b'[' => {
                    depth += 1;
                    index += 1;
                }
                b'}' | b')' | b']' => {
                    depth = depth.saturating_sub(1);
                    index += 1;
                }
                _ => index += 1,
            }
        }
        strings
    }

    #[test]
    fn dispatch_scan_skips_comments_and_handles_escaped_quotes() {
        let body = r#"// 注释里的 "not_a_command"
"game_status" => handle(),
"escaped\"quote" => handle(),
"#;
        assert_eq!(
            top_level_strings(body),
            vec!["game_status".to_string(), "escaped\"quote".to_string()]
        );
    }

    #[test]
    fn contract_ownerless_commands_match_the_dispatch_arms() {
        let mut arms = scanned_dispatch_command_ids();
        let unique: std::collections::BTreeSet<&str> = arms.iter().map(String::as_str).collect();
        assert_eq!(arms.len(), unique.len(), "dispatch 里有重复 arm: {arms:?}");
        let mut declared = dispatch_command_ids();
        arms.sort_unstable();
        declared.sort_unstable();
        assert_eq!(arms, declared);
    }

    #[test]
    fn host_arms_still_work_with_an_unassembled_active_game() {
        let app = crate::service::test_service();
        let status =
            crate::service::block_on(dispatch(app.clone(), "game_status".into(), None, json!({})))
                .unwrap();
        assert_eq!(status["running"], false);

        // 旧设置里可能有未装配的 activeGame; 省略 game 时仍按当前设置处理.
        app.state.settings.lock().unwrap().active_game = GameId::from("hsr");
        assert!(crate::service::block_on(dispatch(
            app.clone(),
            "game_status".into(),
            None,
            json!({}),
        ))
        .is_ok());
        let error = crate::service::block_on(dispatch(
            app,
            "game_status".into(),
            Some("nte".into()),
            json!({}),
        ))
        .unwrap_err();
        assert!(
            error.as_str().unwrap().contains("当前游戏是《hsr》"),
            "{error}"
        );
    }

    #[test]
    fn explicit_game_still_requires_a_selectable_package() {
        let app = crate::service::test_service();
        assert!(crate::service::block_on(dispatch(
            app.clone(),
            "game_status".into(),
            Some("nte".into()),
            json!({}),
        ))
        .is_ok());

        // 未装配的 id 即使与当前设置一致也不能显式请求.
        let error = crate::service::block_on(dispatch(
            app,
            "game_status".into(),
            Some("hsr".into()),
            json!({}),
        ))
        .unwrap_err();
        assert!(
            error
                .as_str()
                .unwrap()
                .contains("无法识别游戏: hsr. 可选值: nte."),
            "{error}"
        );
    }

    /// 带命令组的服务: nte 与 second 可显式请求, fixture 只能经 activeGame 到达.
    fn group_service() -> (crate::service::ServiceHandle, crate::tests::TempDir) {
        struct GroupPackage;

        impl oddity_runtime::GamePackage for GroupPackage {
            fn descriptor(&self) -> oddity_runtime::PackageDescriptor {
                oddity_runtime::PackageDescriptor {
                    id: "fixture",
                    name: "测试游戏",
                }
            }

            fn register(&self, registry: &mut oddity_runtime::Registry) {
                registry.register(oddity_runtime::CommandGroup::for_game(
                    "nte",
                    "account",
                    &["attendance_status", "attendance_run"],
                    route as oddity_runtime::CommandDispatch,
                ));
                registry.register(oddity_runtime::CommandGroup::for_game(
                    "fixture",
                    "fixture",
                    &["fixture_info"],
                    route as oddity_runtime::CommandDispatch,
                ));
            }
        }

        /// 只提供安装域、不注册命令的包, 用来让第二个游戏成为显式请求目标.
        struct SecondPackage;

        static SECOND_LOCATOR: SecondLocator = SecondLocator;

        impl oddity_runtime::GamePackage for SecondPackage {
            fn descriptor(&self) -> oddity_runtime::PackageDescriptor {
                oddity_runtime::PackageDescriptor {
                    id: "second",
                    name: "第二游戏",
                }
            }

            fn register(&self, _registry: &mut oddity_runtime::Registry) {}

            fn install_locator(&self) -> Option<&'static dyn oddity_runtime::InstallLocator> {
                Some(&SECOND_LOCATOR)
            }
        }

        /// 第二个游戏的安装域; 本用例只关心它是否可显式请求.
        struct SecondLocator;

        impl oddity_runtime::InstallLocator for SecondLocator {
            fn launcher_root(
                &self,
                _query: &oddity_runtime::InstallQuery,
            ) -> Option<std::path::PathBuf> {
                None
            }

            fn launcher_dir(&self, _query: &oddity_runtime::InstallQuery) -> Option<&'static str> {
                None
            }

            fn selected_install_root(
                &self,
                _query: &oddity_runtime::InstallQuery,
            ) -> std::path::PathBuf {
                std::path::PathBuf::new()
            }

            fn effective_install_root(
                &self,
                _query: &oddity_runtime::InstallQuery,
                _launcher_root: Option<&std::path::Path>,
            ) -> std::path::PathBuf {
                std::path::PathBuf::new()
            }

            fn game_exe(&self, _launcher_root: &std::path::Path) -> Option<std::path::PathBuf> {
                None
            }

            fn game_version(&self, _launcher_root: &std::path::Path) -> String {
                String::new()
            }

            fn launcher_info(
                &self,
                _launcher_root: &std::path::Path,
            ) -> Option<oddity_runtime::LauncherInfo> {
                None
            }

            fn game_exe_hint(&self) -> String {
                String::new()
            }
        }

        let (events, _events_out) = std::sync::mpsc::sync_channel(64);
        let (input, _jobs) = std::sync::mpsc::sync_channel(64);
        let directory = crate::tests::TempDir::new("protocol-groups");
        let mut registry = oddity_runtime::Registry::new();
        crate::testing::install(&mut registry);
        registry.install(&GroupPackage);
        registry.install(&SecondPackage);
        let service = crate::service::Service::new(crate::service::ServiceConfig {
            settings: crate::settings::Settings::default(),
            data_dir: directory.path().to_path_buf(),
            registry: std::sync::Arc::new(registry),
            events,
            input,
        });
        (service, directory)
    }

    #[test]
    fn group_command_of_another_game_is_unavailable() {
        let (app, _directory) = group_service();
        app.state.settings.lock().unwrap().active_game = GameId::from("fixture");

        for command in ["attendance_status", "attendance_run"] {
            let error =
                crate::service::block_on(dispatch(app.clone(), command.into(), None, json!({})))
                    .unwrap_err();
            assert_eq!(error, json!(format!("《测试游戏》不提供命令 {command}.")));
        }
    }

    #[test]
    fn unassembled_active_game_cannot_reach_group_commands() {
        let (app, _directory) = group_service();
        app.state.settings.lock().unwrap().active_game = GameId::from("hsr");

        let error =
            crate::service::block_on(dispatch(app, "attendance_status".into(), None, json!({})))
                .unwrap_err();
        assert_eq!(
            error,
            json!("当前游戏《hsr》未装配, 不处理命令 attendance_status.")
        );
    }

    #[test]
    fn other_game_request_reports_unavailable_unless_the_command_needs_the_current_game() {
        let (app, _directory) = group_service();
        app.state.settings.lock().unwrap().active_game = GameId::from("nte");

        // 不要求当前游戏的组命令: 请求的游戏没有该命令时直接报"不提供".
        let error = crate::service::block_on(dispatch(
            app.clone(),
            "attendance_status".into(),
            Some("second".into()),
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("《第二游戏》不提供命令 attendance_status."));

        // 要求当前游戏的组命令: 先校验请求游戏与当前游戏一致.
        let error = crate::service::block_on(dispatch(
            app,
            "attendance_run".into(),
            Some("second".into()),
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(
            error,
            json!("请求的游戏是《第二游戏》, 当前游戏是《异环》. 请刷新界面后重试.")
        );
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
        assert!(message.contains("可选值: nte."), "{message}");
    }

    fn ui_command_ids() -> Vec<&'static str> {
        oddity_contract::COMMANDS
            .iter()
            .filter(|command| {
                matches!(
                    command.audience,
                    oddity_contract::Audience::Renderer | oddity_contract::Audience::Window
                )
            })
            .map(|command| command.id)
            .collect()
    }

    fn interface_keys<'a>(source: &'a str, header: &str, end: Option<&str>) -> Vec<&'a str> {
        let start = source.find(header).expect("生成物缺少接口") + header.len();
        let body = match end {
            Some(end) => &source[start..source.find(end).expect("生成物缺少下一个接口")],
            None => &source[start..],
        };
        body.lines()
            .filter_map(|line| {
                let line = line.strip_prefix("  ")?;
                let (name, _) = line.split_once(": ")?;
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
                valid.then_some(name)
            })
            .collect()
    }

    fn route<'a>(
        _: &'a dyn oddity_runtime::HostContext,
        command: &'a str,
        payload: Value,
    ) -> oddity_runtime::DispatchFuture<'a> {
        Box::pin(async move {
            if command == "check_update" {
                Err(payload)
            } else {
                Ok(payload)
            }
        })
    }

    #[test]
    fn group_dispatch_checks_the_current_game_and_propagates_outcomes() {
        let app = crate::service::test_service();
        let mut registry = oddity_runtime::Registry::new();
        registry.register(oddity_runtime::CommandGroup::for_game(
            "nte",
            "test",
            &["fetch_bulletins", "check_update"],
            route as oddity_runtime::CommandDispatch,
        ));
        let group = registry
            .group_for("nte", "fetch_bulletins")
            .expect("测试组应认领抓取命令");
        let payload = json!({"ok": true});
        let routed = crate::service::block_on(dispatch_to_group(
            group,
            app.clone(),
            GameId::from("nte"),
            "fetch_bulletins",
            payload.clone(),
        ))
        .unwrap();
        assert_eq!(routed, payload);

        let failure = crate::service::block_on(dispatch_to_group(
            group,
            app.clone(),
            GameId::from("nte"),
            "check_update",
            json!("group failed"),
        ))
        .unwrap_err();
        assert_eq!(failure, json!("group failed"));

        app.state.settings.lock().unwrap().active_game = GameId::from("hsr");
        let error = crate::service::block_on(dispatch_to_group(
            group,
            app,
            GameId::from("nte"),
            "fetch_bulletins",
            json!({}),
        ))
        .unwrap_err();
        assert!(
            error.as_str().unwrap().contains("当前游戏是《hsr》"),
            "{error}"
        );
    }

    #[test]
    fn host_and_window_commands_keep_their_audience() {
        for id in ["host_state", "external_launch"] {
            let command = oddity_contract::command(id).expect("宿主命令缺少声明");
            assert_eq!(
                command.audience,
                oddity_contract::Audience::Host,
                "{id} 只应由 Electron 主进程调用"
            );
        }
        for id in [
            "win_minimize",
            "win_toggle_maximize",
            "win_close",
            "win_hide",
        ] {
            let command = oddity_contract::command(id).expect("窗口命令缺少声明");
            assert_eq!(
                command.audience,
                oddity_contract::Audience::Window,
                "{id} 只在 Electron 窗口层处理"
            );
            assert!(!command.current_game, "{id} 不应要求当前游戏");
        }
    }

    #[test]
    fn generated_whitelist_matches_ui_commands() {
        let expected = ui_command_ids();
        let whitelist = include_str!("../../../ui/electron/commands.ts");
        let generated: Vec<&str> = whitelist
            .lines()
            .filter_map(|line| {
                let name = line.trim().strip_suffix(": true,")?;
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
                valid.then_some(name)
            })
            .collect();
        assert_eq!(generated, expected, "Electron 白名单与契约声明不一致");
    }

    #[test]
    fn generated_command_ids_match_ui_commands() {
        let expected: Vec<String> = ui_command_ids()
            .into_iter()
            .map(|id| format!("\"{id}\","))
            .collect();
        let ids = include_str!("../../../ui/src/core/generated/command-ids.ts");
        let generated: Vec<&str> = ids
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('"') && line.ends_with("\","))
            .collect();
        assert_eq!(generated, expected, "演示命令键与契约声明不一致");
    }

    #[test]
    fn generated_types_cover_every_ui_command() {
        let source = include_str!("../../../ui/src/types/generated/commands.ts");
        let mut expected = ui_command_ids();
        let mut commands = interface_keys(
            source,
            "export interface Commands {",
            Some("export interface CommandArgs {"),
        );
        let mut args = interface_keys(source, "export interface CommandArgs {", None);
        expected.sort_unstable();
        commands.sort_unstable();
        args.sort_unstable();
        assert_eq!(commands, expected, "Commands 与契约声明不一致");
        assert_eq!(args, expected, "CommandArgs 与契约声明不一致");
    }

    #[test]
    fn bootstrap_discovers_the_current_game_without_validation() {
        assert!(!oddity_contract::requires_current_game("bootstrap"));
        assert!(!oddity_contract::requires_current_game("external_launch"));
        assert!(oddity_contract::requires_current_game("game_status"));
        assert!(oddity_contract::requires_current_game("take_launch_error"));
    }
}
