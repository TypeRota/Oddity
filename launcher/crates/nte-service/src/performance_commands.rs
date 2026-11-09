//! 性能比较命令, 与游戏启动复用已有的游戏状态锁.
use std::path::Path;
use std::sync::Mutex;

use nte_core::performance::{Facts, Recording, Store};
use serde_json::json;

use crate::service::ServiceHandle;
use crate::{environment_commands, Settings};

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
        command: &mut std::process::Command,
    ) -> Result<Option<Recording>, String> {
        let mut store = self.store.lock().map_err(|e| e.to_string())?;
        let store = store.as_mut().map_err(|e| e.clone())?;
        let environment = environment_commands::selected(settings)?;
        let launcher = crate::detect_launcher_root(settings);
        let version = launcher
            .as_ref()
            .map(|root| crate::game_version_for(root))
            .filter(|v| !v.is_empty());
        let game_exe = settings
            .active_game
            .profile()
            .layout()
            .and_then(|l| Path::new(l.game_exe).file_stem())
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
        match store.prepare(environment, facts, command) {
            Ok(recording) => Ok(recording),
            Err(error) => {
                let message = format!("无法开始性能采样: {error:#}");
                store.fail(message.clone()).map_err(|e| format!("{e:#}"))?;
                Err(message)
            }
        }
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

pub async fn command(
    app: ServiceHandle,
    operation: String,
    name: Option<String>,
    a: Option<String>,
    b: Option<String>,
    id: Option<String>,
    destination: Option<String>,
) -> Result<serde_json::Value, String> {
    crate::service::spawn_blocking(move || {
        let mut store = app.performance.store.lock().map_err(|e| e.to_string())?;
        let store = store.as_mut().map_err(|e| e.clone())?;
        if let Some(id) = &id {
            let settings = crate::launch_settings(&app.state)?;
            if store.comparison(id).map_err(|e| e.to_string())?.a.game != settings.active_game {
                return Err("比较不属于当前游戏".into());
            }
        }
        match operation.as_str() {
            "overview" => {
                let settings = crate::launch_settings(&app.state)?;
                let overview = store.summary(settings.active_game);
                let recording = app
                    .state
                    .game
                    .lock()
                    .map_err(|e| e.to_string())?
                    .session
                    .as_ref()
                    .and_then(|s| {
                        s.performance
                            .lock()
                            .ok()
                            .and_then(|r| r.as_ref().map(|r| r.comparison.clone()))
                    });
                let mut value = json!(overview);
                value["recording"] = json!(recording);
                Ok(value)
            }
            "report" => Ok(json!(store
                .report_svg(id.as_deref().ok_or("缺少比较 ID")?)
                .map_err(|e| format!("{e:#}"))?)),
            "export" => Ok(json!(store
                .export(
                    id.as_deref().ok_or("缺少比较 ID")?,
                    Path::new(&destination.ok_or("缺少保存路径")?)
                )
                .map_err(|e| format!("{e:#}"))?
                .display()
                .to_string())),
            "create" | "activate" => {
                let game = app.state.game.lock().map_err(|e| e.to_string())?;
                if game.starting
                    || game.environment_busy
                    || game.session.as_ref().is_some_and(crate::session_alive)
                {
                    return Err("请先结束游戏和运行环境任务".into());
                }
                let settings = crate::launch_settings(&app.state)?;
                if operation == "create" {
                    let find = |id: Option<String>| {
                        settings
                            .runtime_environments
                            .iter()
                            .find(|e| Some(&e.id) == id.as_ref() && e.game == settings.active_game)
                            .cloned()
                            .ok_or_else(|| "所选运行环境不存在".to_string())
                    };
                    if !nte_core::game_mode::probe()
                        .iter()
                        .any(|s| s.id == nte_core::game_mode::Feature::Overlay && s.available)
                    {
                        return Err("未找到 MangoHud, 请先安装后再创建性能比较".into());
                    }
                    let a = find(a)?;
                    let b = find(b)?;
                    if !a.proton.path.is_dir() || !b.proton.path.is_dir() {
                        return Err("所选 Proton 目录不存在".into());
                    }
                    store
                        .create(name.ok_or("缺少比较名称")?, a, b)
                        .map_err(|e| format!("{e:#}"))?;
                } else {
                    store.activate(id).map_err(|e| format!("{e:#}"))?;
                }
                Ok(serde_json::Value::Null)
            }
            _ => Err("未知性能比较操作".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{block_on, test_service};
    use crate::tests::{write_script, EnvGuard, TempDir};
    use nte_core::{
        distribution::Distribution,
        environment::{ProtonPin, RuntimeEnvironment},
        game_profile::GameId,
    };

    #[test]
    fn creates_pauses_and_reloads_a_comparison_through_the_game_scoped_protocol() {
        let mut env = EnvGuard::new();
        let dir = TempDir::new("performance-command");
        write_script(dir.path(), "mangohud", "#!/bin/sh\nexit 0\n");
        env.set("PATH", dir.path());
        let app = test_service();
        let make = |id: &str| RuntimeEnvironment {
            id: id.repeat(32),
            name: id.into(),
            proton: ProtonPin {
                path: dir.path().into(),
                identity: id.into(),
            },
            install_root: dir.path().into(),
            game: GameId::Nte,
            distribution: Distribution::Cn,
            created_at: 1,
        };
        app.state.settings.lock().unwrap().runtime_environments = vec![make("a"), make("b")];
        let call = |name: &str, args| {
            block_on(crate::protocol::dispatch(
                app.clone(),
                name.into(),
                Some("nte".into()),
                args,
            ))
        };
        call(
            "performance_create",
            json!({"name":"A/B", "a":"a".repeat(32), "b":"b".repeat(32)}),
        )
        .unwrap();
        let overview = call("performance_overview", json!({})).unwrap();
        let id = overview["active"].as_str().unwrap();
        assert_eq!(overview["comparisons"][0]["name"], "A/B");
        assert!(overview["recording"].is_null());
        call("performance_activate", json!({"id":null})).unwrap();
        assert!(call("performance_overview", json!({})).unwrap()["active"].is_null());
        call("performance_activate", json!({"id":id})).unwrap();
        assert!(call("performance_report", json!({"id":id}))
            .unwrap_err()
            .to_string()
            .contains("尚无性能报告"));
        assert!(block_on(crate::protocol::dispatch(
            app,
            "performance_overview".into(),
            Some("hsr".into()),
            json!({})
        ))
        .is_err());
    }

    #[test]
    fn does_not_change_sampling_while_a_launch_is_in_progress() {
        let _env = EnvGuard::new();
        let app = test_service();
        app.state.game.lock().unwrap().starting = true;
        let error = block_on(command(
            app,
            "activate".into(),
            None,
            None,
            None,
            None,
            None,
        ))
        .unwrap_err();
        assert!(error.contains("请先结束游戏"));
    }
}
