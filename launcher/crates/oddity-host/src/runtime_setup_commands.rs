use crate::runtime_setup::{self, Plan, Progress, Receipt};
use crate::service::ServiceHandle;
use oddity_runtime::EventSink;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Default)]
pub struct Setup {
    plan: Option<Plan>,
    create_environment: bool,
    status: Status,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub busy: bool,
    pub progress: Progress,
    pub error: Option<String>,
    pub done: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    needs_setup: bool,
    environment: Option<crate::environment::RuntimeEnvironment>,
    umu: Option<PathBuf>,
    proton: Option<PathBuf>,
    proton_directory: PathBuf,
    system_umu: bool,
}
pub fn info(app: &ServiceHandle) -> Result<Info, String> {
    let data = crate::paths::data_dir();
    let receipt = Receipt::load(&data).map_err(|e| format!("{e:#}"))?;
    let settings = app.state.settings.lock().map_err(|e| e.to_string())?;
    let umu = runtime_setup::umu(&data);
    let proton = crate::bootstrap::detect_proton(&settings)
        .map(|p| p.root)
        .or(receipt.proton);
    let environment = crate::environment_commands::selected(&settings)
        .ok()
        .filter(|_| {
            crate::environment_commands::prefix(&app.state.packages, &settings)
                .is_ok_and(|prefix| prefix.join("system.reg").is_file())
                && crate::environment_commands::verify_proton(&settings).is_ok()
        })
        .cloned();
    Ok(Info {
        needs_setup: environment.is_none() || umu.is_none() || proton.is_none(),
        environment,
        umu,
        proton,
        proton_directory: runtime_setup::steam_tools(&crate::paths::home_dir()),
        system_umu: runtime_setup::system_umu().is_some(),
    })
}
pub fn dismiss() -> Result<(), String> {
    let data = crate::paths::data_dir();
    let mut receipt = Receipt::load(&data).map_err(|e| format!("{e:#}"))?;
    receipt.seen = true;
    receipt.save(&data).map_err(|e| format!("{e:#}"))
}
pub fn plan(
    app: &ServiceHandle,
    all_users: bool,
    proton: Option<String>,
    create_environment: bool,
) -> Result<Plan, String> {
    let mut setup = app.runtime_setup.lock().map_err(|e| e.to_string())?;
    if setup.status.busy {
        return Err("启动环境正在配置".into());
    }
    setup.plan = None;
    let preferred = {
        let settings = app.state.settings.lock().map_err(|e| e.to_string())?;
        crate::bootstrap::detect_proton(&settings).map(|installed| installed.root)
    };
    let proton =
        proton
            .map(PathBuf::from)
            .or(preferred)
            .or(Receipt::load(&crate::paths::data_dir())
                .map_err(|e| format!("{e:#}"))?
                .proton);
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let plan = runtime_setup::plan(
        &crate::paths::data_dir(),
        &crate::paths::home_dir(),
        all_users,
        proton.as_deref(),
        &os,
    )
    .map_err(|e| format!("{e:#}"))?;
    setup.plan = Some(plan.clone());
    setup.create_environment = create_environment;
    setup.status = Status::default();
    Ok(plan)
}
pub fn status(app: &ServiceHandle) -> Result<Status, String> {
    Ok(app
        .runtime_setup
        .lock()
        .map_err(|e| e.to_string())?
        .status
        .clone())
}
pub async fn apply(app: ServiceHandle) -> Result<(), String> {
    let (plan, create_environment) = {
        let mut setup = app.runtime_setup.lock().map_err(|e| e.to_string())?;
        if setup.status.busy {
            return Err("启动环境正在配置".into());
        }
        let plan = setup.plan.take().ok_or("请先查看安装计划")?;
        setup.status = Status {
            busy: true,
            ..Default::default()
        };
        (plan, setup.create_environment)
    };
    let worker = app.clone();
    let result = crate::service::spawn_blocking(move || {
        let _task = crate::environment_commands::Task::begin(&worker.state)?;
        runtime_setup::install(&crate::paths::data_dir(), &plan, |progress| {
            if let Ok(mut setup) = worker.runtime_setup.lock() {
                setup.status.progress = progress;
                let _ = worker.emit("runtime-setup-state", setup.status.clone());
            }
        })
        .map_err(|e| format!("{e:#}"))?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let result = match result {
        Ok(()) => prepare_environment(app.clone(), create_environment)
            .await
            .map(|_| ()),
        Err(error) => Err(error),
    };
    {
        let mut setup = app.runtime_setup.lock().map_err(|e| e.to_string())?;
        setup.status.busy = false;
        setup.status.done = result.is_ok();
        setup.status.error = result.as_ref().err().cloned();
    }
    result
}

pub async fn ensure_environment(app: ServiceHandle) -> Result<crate::bootstrap::Bootstrap, String> {
    prepare_environment(app, false).await
}

async fn prepare_environment(
    app: ServiceHandle,
    create_environment: bool,
) -> Result<crate::bootstrap::Bootstrap, String> {
    let worker = app.clone();
    let mut bootstrap =
        crate::service::spawn_blocking(move || -> Result<crate::bootstrap::Bootstrap, String> {
            let settings = worker
                .state
                .settings
                .lock()
                .map_err(|e| e.to_string())?
                .clone();
            let receipt = Receipt::load(&crate::paths::data_dir()).map_err(|e| format!("{e:#}"))?;
            let channel = crate::channel::ChannelId::new(crate::channel::effective_id(
                &worker.state.packages,
                settings.active_game.as_str(),
                settings.current().distribution.as_ref(),
            ));
            let installed =
                crate::environment_commands::installations(&worker.state.packages, &settings)
                    .into_iter()
                    .find(|i| i.distribution == channel);
            let target = installed.ok_or("当前渠道不可用")?;
            if !create_environment
                && crate::environment_commands::selected(&settings).is_ok_and(|entry| {
                    entry.matches(&settings.active_game, &target.install_root, &channel)
                })
            {
                crate::environment_commands::verify_proton(&settings)?;
            } else if let Some(existing) = settings.runtime_environments.iter().find(|entry| {
                !create_environment
                    && entry.matches(&settings.active_game, &target.install_root, &channel)
            }) {
                crate::environment_commands::action_impl(
                    &worker.state,
                    "switch",
                    Some(existing.id.clone()),
                )?;
            } else {
                let proton = receipt.proton.ok_or("请先自动配置或手动创建运行环境")?;
                let label = crate::channel::effective(
                    &worker.state.packages,
                    settings.active_game.as_str(),
                    settings.current().distribution.as_ref(),
                )
                .map_or("默认渠道", |channel| channel.label);
                let created = crate::environment_commands::create_impl(
                    &worker.state,
                    None,
                    &format!(
                        "{}-{}-{}",
                        crate::install::display_name(&worker.state.packages, &settings.active_game),
                        label,
                        settings.runtime_environments.len() + 1
                    ),
                    &proton.to_string_lossy(),
                    channel,
                    false,
                )?;
                let id = created
                    .settings
                    .runtime_environments
                    .last()
                    .unwrap()
                    .id
                    .clone();
                crate::environment_commands::action_impl(&worker.state, "switch", Some(id))?;
            }
            let _task = crate::environment_commands::Task::begin(&worker.state)?;
            let settings = worker
                .state
                .settings
                .lock()
                .map_err(|e| e.to_string())?
                .clone();
            let prefix = crate::environment_commands::prefix(&worker.state.packages, &settings)?;
            crate::environment_commands::verify_proton(&settings)?;
            if !prefix.join("system.reg").is_file() {
                let umu = runtime_setup::umu(&crate::paths::data_dir()).ok_or("未找到 umu-run")?;
                let proton =
                    crate::environment_commands::proton_path(&settings).ok_or("未选择 Proton")?;
                std::fs::create_dir_all(crate::paths::data_dir().join("logs"))
                    .map_err(|e| e.to_string())?;
                runtime_setup::initialize(
                    &umu,
                    proton,
                    &prefix,
                    &crate::paths::data_dir().join("logs/runtime-setup.log"),
                )
                .map_err(|e| format!("{e:#}"))?;
            }
            Ok(crate::bootstrap::bootstrap_impl(
                &worker.state.packages,
                &settings,
            ))
        })
        .await
        .map_err(|e| e.to_string())??;
    bootstrap.integrity = app.integrity.status();
    Ok(bootstrap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fake_proton, write_script, EnvGuard, TempDir};

    #[test]
    fn automatic_environment_is_created_once_and_initializes_the_real_prefix() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("runtime-auto");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        env.remove("NTE_ROOT");
        env.remove("NTE_INSTALL");
        std::fs::create_dir_all(directory.join("bin")).unwrap();
        write_script(
            &directory.join("bin"),
            "umu-run",
            "#!/bin/sh\nprintf initialized > \"$WINEPREFIX/system.reg\"\n",
        );
        env.set("PATH", directory.join("bin"));
        directory.write_text(
            format!("install/{}/Config/Config.ini", crate::testing::LAUNCHER_DIR),
            "[VERSION]\nVersion=1.0\n",
        );
        directory.write(
            format!(
                "install/{}/{}",
                crate::testing::LAUNCHER_DIR,
                crate::testing::CLIENT_EXE
            ),
            "MZ",
        );
        directory.write(format!("install/{}", crate::testing::GAME_EXE), "MZ");
        fake_proton(directory.path(), "Proton");
        let app = crate::service::test_service();
        app.state
            .settings
            .lock()
            .unwrap()
            .current_mut()
            .install_roots
            .insert(
                "cn".into(),
                directory.join("install").to_string_lossy().into(),
            );
        Receipt {
            seen: true,
            proton: Some(directory.join("Proton")),
        }
        .save(&crate::paths::data_dir())
        .unwrap();
        assert!(info(&app).unwrap().needs_setup);
        let result = crate::service::block_on(ensure_environment(app.clone())).unwrap();
        assert!(!info(&app).unwrap().needs_setup);
        assert_eq!(result.settings.runtime_environments.len(), 1);
        let prefix =
            crate::environment_commands::prefix(&app.state.packages, &result.settings).unwrap();
        assert!(prefix.join("system.reg").is_file());
        let again = crate::service::block_on(ensure_environment(app.clone())).unwrap();
        assert_eq!(
            again.settings.runtime_environments,
            result.settings.runtime_environments
        );
        assert!(!app.state.game.lock().unwrap().environment_busy);
        std::fs::remove_file(prefix.join("system.reg")).unwrap();
        let incomplete = info(&app).unwrap();
        assert!(incomplete.needs_setup);
        assert!(incomplete.environment.is_none());
        let repaired = crate::service::block_on(ensure_environment(app.clone())).unwrap();
        assert_eq!(repaired.settings.runtime_environments.len(), 1);
        assert!(prefix.join("system.reg").is_file());
        let additional = crate::service::block_on(prepare_environment(app.clone(), true)).unwrap();
        assert_eq!(additional.settings.runtime_environments.len(), 2);
        assert_eq!(
            additional.settings.runtime_environments[0],
            result.settings.runtime_environments[0]
        );
        assert_ne!(
            additional.settings.current().active_environment,
            result.settings.current().active_environment
        );
        assert_eq!(
            additional.settings.current().stable_environment,
            result.settings.current().stable_environment
        );
        let reused = crate::service::block_on(ensure_environment(app.clone())).unwrap();
        assert_eq!(reused.settings.runtime_environments.len(), 2);
        assert_eq!(
            info(&app).unwrap().environment.unwrap().id,
            reused
                .settings
                .current()
                .active_environment
                .clone()
                .unwrap()
        );
        std::fs::remove_file(directory.join(format!("install/{}", crate::testing::GAME_EXE)))
            .unwrap();
        {
            let mut settings = app.state.settings.lock().unwrap();
            settings.runtime_environments.clear();
            settings.current_mut().active_environment = None;
            settings.current_mut().stable_environment = None;
        }
        let before_install_info = info(&app).unwrap();
        assert!(before_install_info.needs_setup);
        assert!(before_install_info.environment.is_none());
        let before_install = crate::service::block_on(ensure_environment(app.clone())).unwrap();
        assert_eq!(before_install.settings.runtime_environments.len(), 1);
        assert!(!before_install.game_exists);
        assert!(
            crate::environment_commands::prefix(&app.state.packages, &before_install.settings)
                .unwrap()
                .join("system.reg")
                .is_file()
        );
    }

    #[test]
    fn automatic_plan_ignores_stale_receipts_but_validates_manual_proton() {
        let mut env = EnvGuard::new();
        let directory = TempDir::new("runtime-stale-receipt");
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        std::fs::create_dir_all(directory.join("bin")).unwrap();
        write_script(&directory.join("bin"), "umu-run", "#!/bin/sh\nexit 0\n");
        env.set("PATH", directory.join("bin"));
        let app = crate::service::test_service();
        let proton = directory.join("old-steam/Proton");
        for incomplete in [false, true] {
            if incomplete {
                directory.write("old-steam/Proton/proton", "proton");
            }
            Receipt {
                seen: true,
                proton: Some(proton.clone()),
            }
            .save(&crate::paths::data_dir())
            .unwrap();
            assert!(info(&app).unwrap().proton.is_none());
            let automatic = plan(&app, false, None, false).unwrap();
            assert!(automatic.install_proton);
            assert_eq!(
                automatic.proton,
                runtime_setup::steam_tools(&crate::paths::home_dir())
                    .join(runtime_setup::PROTON_VERSION)
            );
            assert!(plan(&app, false, Some(proton.display().to_string()), false)
                .unwrap_err()
                .contains("Proton 绝对路径"));
        }
        fake_proton(directory.path(), "old-steam/Proton");
        assert_eq!(plan(&app, false, None, false).unwrap().proton, proton);
    }

    #[test]
    fn failed_setup_consumes_plan_and_releases_environment_task() {
        let _env = EnvGuard::new();
        let app = crate::service::test_service();
        app.state.game.lock().unwrap().starting = true;
        app.runtime_setup.lock().unwrap().plan = Some(Plan {
            umu: "/unused/umu".into(),
            proton: "/unused/proton".into(),
            install_umu: false,
            install_proton: false,
            system_command: None,
            umu_version: "test".into(),
            log: "/unused/log".into(),
        });
        assert!(crate::service::block_on(apply(app.clone())).is_err());
        let status = status(&app).unwrap();
        assert!(!status.busy && !status.done && status.error.is_some());
        assert!(app.runtime_setup.lock().unwrap().plan.is_none());
        assert!(!app.state.game.lock().unwrap().environment_busy);
        let error = crate::service::block_on(apply(app)).unwrap_err();
        assert!(error.contains("先查看安装计划"));
    }
}
