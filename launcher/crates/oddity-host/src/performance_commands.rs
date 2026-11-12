//! 性能记录命令, 与游戏启动复用已有的游戏状态锁.
use std::path::Path;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::environment_commands;
use crate::performance::{Facts, Recording, Store};
use crate::service::ServiceHandle;
use crate::settings::Settings;
use oddity_runtime::LaunchPlan;

pub struct Performance {
    store: Mutex<Result<Store, String>>,
}

impl Performance {
    pub fn load(data: &Path) -> Self {
        Self {
            store: Mutex::new(Store::load(data.join("performance")).map_err(|e| format!("{e:#}"))),
        }
    }

    pub fn prepare(
        &self,
        settings: &Settings,
        plan: &LaunchPlan,
        command: &mut std::process::Command,
    ) -> Result<Option<Recording>, String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        let store = store.as_mut().map_err(|e| e.clone())?;
        if !store.recording_enabled() {
            return Ok(None);
        }
        let environment = environment_commands::selected(settings)?;
        let version = (!plan.game_version.is_empty()).then(|| plan.game_version.clone());
        // 进程名取实际启动计划里的可执行文件, 不再读取宿主内置的安装布局.
        let game_exe = plan
            .game_exe
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().to_string());
        let display = command
            .get_envs()
            .find(|(key, _)| *key == "PROTON_ENABLE_WAYLAND")
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().to_string());
        let facts = Facts {
            game_version: version,
            game_executable: game_exe.ok_or("当前游戏没有可采样的进程名称")?,
            launch_mode: settings.current().launch_mode.clone(),
            display_backend: if display.as_deref() == Some("1") {
                "Wayland"
            } else {
                "XWayland/X11"
            }
            .into(),
            game_mode: settings.current().game_mode.clone(),
            vkd3d_config: std::env::var("VKD3D_CONFIG").ok(),
            proton_identity: environment.proton.identity.clone(),
            kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .ok()
                .map(|s| s.trim().to_string()),
            cpu: std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("model name")
                        .and_then(|v| v.split_once(':'))
                        .map(|(_, v)| v.trim().to_string())
                })
            }),
            gpu: None,
        };
        store
            .prepare(&settings.active_game, environment, facts, command)
            .map_err(|error| format!("无法开始性能记录: {error:#}"))
    }

    pub fn finish(&self, recording: &Recording) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        store
            .as_mut()
            .map_err(|e| e.clone())?
            .finish(recording)
            .map_err(|e| format!("{e:#}"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Args {
    a: Option<String>,
    b: Option<String>,
    id: Option<String>,
    destination: Option<String>,
    pinned: Option<bool>,
    enabled: Option<bool>,
}

pub async fn command(app: ServiceHandle, operation: String, args: Args) -> Result<Value, String> {
    crate::service::spawn_blocking(move || {
        let settings = crate::session::launch_settings(&app.state)?;
        let mut store = app.performance.store.lock().map_err(|e| e.to_string())?;
        let store = store.as_mut().map_err(|e| e.clone())?;
        let record_id = |id: Option<String>| id.ok_or_else(|| "缺少记录 ID".to_string());
        let ensure_mine = |store: &Store, id: &str| -> Result<(), String> {
            let record = store.record(id).map_err(|e| format!("{e:#}"))?;
            if record.game != settings.active_game {
                return Err("性能记录不属于当前游戏".into());
            }
            Ok(())
        };
        match operation.as_str() {
            "overview" => {
                let mut value = json!(store.summary(&settings.active_game));
                // 会话退出后后台仍在解析, 界面不能再显示正在记录.
                let recording =
                    app.state
                        .game
                        .lock()
                        .map_err(|e| e.to_string())?
                        .session
                        .as_ref()
                        .filter(|session| crate::session::session_alive(session))
                        .and_then(|session| {
                            session.performance.lock().ok().and_then(|recording| {
                                recording.as_ref().map(|r| r.id().to_string())
                            })
                        });
                value["recording"] = json!(recording);
                Ok(value)
            }
            "recording" => {
                store
                    .set_recording(args.enabled.ok_or("缺少采样开关")?)
                    .map_err(|e| format!("{e:#}"))?;
                Ok(Value::Null)
            }
            "report" => {
                let (a, b) = pair(&args)?;
                ensure_mine(store, &a)?;
                ensure_mine(store, &b)?;
                Ok(json!(store.svg(&a, &b).map_err(|e| format!("{e:#}"))?))
            }
            "export" => {
                let destination = args.destination.clone().ok_or("缺少保存路径")?;
                let (a, b) = pair(&args)?;
                ensure_mine(store, &a)?;
                ensure_mine(store, &b)?;
                Ok(json!(store
                    .export(&a, &b, Path::new(&destination))
                    .map_err(|e| format!("{e:#}"))?
                    .display()
                    .to_string()))
            }
            "pin" | "delete" => {
                {
                    let game = app.state.game.lock().map_err(|e| e.to_string())?;
                    if game.starting
                        || game.environment_busy
                        || game
                            .session
                            .as_ref()
                            .is_some_and(crate::session::session_alive)
                    {
                        return Err("请先结束游戏和运行环境任务".into());
                    }
                }
                let id = record_id(args.id)?;
                ensure_mine(store, &id)?;
                if operation == "pin" {
                    store
                        .pin(&id, args.pinned.ok_or("缺少置顶状态")?)
                        .map_err(|e| format!("{e:#}"))?;
                } else {
                    store.delete(&id).map_err(|e| format!("{e:#}"))?;
                }
                Ok(Value::Null)
            }
            _ => Err("未知性能记录操作".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 报告的差值方向由调用方选择: A 为基准, B 为对比.
fn pair(args: &Args) -> Result<(String, String), String> {
    Ok((
        args.a.clone().ok_or("缺少基准记录 ID")?,
        args.b.clone().ok_or("缺少对比记录 ID")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelId;
    use crate::environment::{ProtonPin, RuntimeEnvironment};
    use crate::identity::GameId;
    use crate::service::{block_on, test_service};
    use crate::tests::EnvGuard;

    fn service() -> ServiceHandle {
        let app = test_service();
        let make = |id: &str| RuntimeEnvironment {
            id: id.repeat(32),
            name: format!("环境 {id}"),
            proton: ProtonPin {
                path: format!("/proton/GE-Proton-{id}").into(),
                identity: id.into(),
            },
            install_root: "/game".into(),
            game: GameId::from("nte"),
            distribution: ChannelId::new("cn"),
            created_at: 1,
        };
        let mut settings = app.state.settings.lock().unwrap();
        settings.runtime_environments = vec![make("a")];
        settings.current_mut().active_environment = Some("a".repeat(32));
        drop(settings);
        app
    }

    fn call(app: &ServiceHandle, name: &str, args: Value) -> Result<Value, String> {
        block_on(crate::protocol::dispatch(
            app.clone(),
            name.into(),
            Some("nte".into()),
            args,
        ))
        .map_err(|error| error.to_string())
    }

    #[test]
    fn toggles_recording_and_reports_the_new_state_through_the_protocol() {
        let _env = EnvGuard::new();
        let app = service();
        let overview = call(&app, "performance_overview", json!({})).unwrap();
        assert_eq!(overview["recordingEnabled"], false);
        assert!(overview["records"].as_array().unwrap().is_empty());
        assert!(overview["recording"].is_null());

        call(&app, "performance_recording", json!({"enabled": true})).unwrap();
        let overview = call(&app, "performance_overview", json!({})).unwrap();
        assert_eq!(overview["recordingEnabled"], true);

        call(&app, "performance_recording", json!({"enabled": false})).unwrap();
        assert_eq!(
            call(&app, "performance_overview", json!({})).unwrap()["recordingEnabled"],
            false
        );
        assert!(call(&app, "performance_recording", json!({})).is_err());
    }

    #[test]
    fn refuses_records_of_another_game_and_unknown_operations() {
        let _env = EnvGuard::new();
        let app = service();
        call(&app, "performance_recording", json!({"enabled": true})).unwrap();
        let error = call(
            &app,
            "performance_pin",
            json!({"id":"missing","pinned":true}),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("性能记录不存在"));
        let args = |value: Value| serde_json::from_value::<Args>(value).unwrap();
        assert!(
            block_on(command(app.clone(), "history".into(), args(json!({}))))
                .unwrap_err()
                .contains("未知性能记录操作")
        );
        assert!(
            block_on(command(app.clone(), "recording".into(), args(json!({}))))
                .unwrap_err()
                .contains("缺少采样开关")
        );
        assert!(block_on(crate::protocol::dispatch(
            app,
            "performance_overview".into(),
            Some("hsr".into()),
            json!({})
        ))
        .is_err());
    }

    #[test]
    fn does_not_pin_or_delete_while_a_launch_is_in_progress() {
        let _env = EnvGuard::new();
        let app = service();
        app.state.game.lock().unwrap().starting = true;
        let error = call(&app, "performance_delete", json!({"id":"x"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("请先结束游戏"));
        assert!(
            call(&app, "performance_pin", json!({"id":"x","pinned":true}))
                .unwrap_err()
                .to_string()
                .contains("请先结束游戏")
        );
    }

    #[test]
    fn reports_require_two_existing_records() {
        let _env = EnvGuard::new();
        let app = service();
        let error = call(
            &app,
            "performance_report",
            json!({"a": "missing", "b": "other"}),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("性能记录不存在"));
        assert!(call(&app, "performance_report", json!({"a": "missing"}))
            .unwrap_err()
            .to_string()
            .contains("缺少对比记录"));
        assert!(call(&app, "performance_export", json!({"a":"x","b":"y"}))
            .unwrap_err()
            .to_string()
            .contains("缺少保存路径"));
    }
}
