use crate::environment;
use crate::environment_commands::Installation;
use crate::runtime_setup::{self, Plan, Progress, Receipt};
use crate::service::ServiceHandle;
use crate::settings::Settings;
use oddity_runtime::{ChannelDescriptor, EventSink, Registry};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Default)]
pub struct Setup {
    plan: Option<Plan>,
    confirmation: Option<Confirmation>,
    status: Status,
}

/// 向导确认时锁定的名称、地区与安装目录.
pub struct Confirmation {
    name: String,
    channel: String,
    install_root: PathBuf,
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
    ready: bool,
    needs_setup: bool,
    installed: bool,
    channel: String,
    install_root: String,
    suggested_name: String,
    environment: Option<crate::environment::RuntimeEnvironment>,
    umu: Option<PathBuf>,
    proton: Option<PathBuf>,
    proton_directory: PathBuf,
    system_umu: bool,
}

/// 当前渠道对应的安装状态.
fn installation_for(
    registry: &Registry,
    settings: &Settings,
    channel: &ChannelDescriptor,
) -> Option<Installation> {
    crate::environment_commands::installations(registry, settings)
        .into_iter()
        .find(|target| target.distribution.as_str() == channel.id)
}

pub fn info(app: &ServiceHandle) -> Result<Info, String> {
    let data = crate::paths::data_dir();
    let receipt = Receipt::load(&data).map_err(|error| format!("{error:#}"))?;
    let settings = app
        .state
        .settings
        .lock()
        .map_err(|error| error.to_string())?;
    let umu = runtime_setup::umu(&data);
    let proton = crate::bootstrap::detect_proton(&app.state.packages, &settings)
        .map(|installed| installed.root)
        .or(receipt.proton);
    let channel = crate::channel::effective(
        &app.state.packages,
        settings.active_game.as_str(),
        settings.current().distribution.as_ref(),
    );
    let target =
        channel.and_then(|channel| installation_for(&app.state.packages, &settings, channel));
    let environment = crate::environment_commands::selected(&app.state.packages, &settings)
        .ok()
        .cloned();
    let ready = environment.is_some()
        && crate::environment_commands::prefix(&app.state.packages, &settings)
            .is_ok_and(|prefix| prefix.join("system.reg").is_file())
        && crate::environment_commands::verify_proton(&app.state.packages, &settings).is_ok();
    let display = crate::install::display_name(&app.state.packages, &settings.active_game);
    // 默认名按游戏区分, 序号在全部环境中递增保证唯一.
    let base = display.to_string();
    Ok(Info {
        ready,
        needs_setup: !ready || umu.is_none() || proton.is_none(),
        installed: target.as_ref().is_some_and(|target| target.installed),
        channel: channel.map_or_else(String::new, |channel| channel.label.to_string()),
        install_root: target
            .as_ref()
            .map(|target| target.install_root.display().to_string())
            .unwrap_or_default(),
        suggested_name: crate::environment_commands::suggested_name(&settings, &base),
        environment,
        umu,
        proton,
        proton_directory: runtime_setup::steam_tools(&crate::paths::home_dir()),
        system_umu: runtime_setup::system_umu().is_some(),
    })
}

pub fn dismiss() -> Result<(), String> {
    let data = crate::paths::data_dir();
    let mut receipt = Receipt::load(&data).map_err(|error| format!("{error:#}"))?;
    receipt.seen = true;
    receipt.save(&data).map_err(|error| format!("{error:#}"))
}

pub fn plan(
    app: &ServiceHandle,
    all_users: bool,
    proton: Option<String>,
    environment_name: Option<String>,
) -> Result<Plan, String> {
    let mut setup = app
        .runtime_setup
        .lock()
        .map_err(|error| error.to_string())?;
    if setup.status.busy {
        return Err("启动环境正在配置".into());
    }
    setup.plan = None;
    setup.confirmation = None;
    let (preferred, confirmation) = {
        let settings = app
            .state
            .settings
            .lock()
            .map_err(|error| error.to_string())?;
        let preferred = crate::bootstrap::detect_proton(&app.state.packages, &settings)
            .map(|installed| installed.root);
        let confirmation = environment_name
            .map(|name| -> Result<Confirmation, String> {
                let name = environment::validate_name(&name)
                    .map_err(|error| format!("{error:#}"))?
                    .to_string();
                if settings
                    .runtime_environments
                    .iter()
                    .any(|environment| environment.name == name)
                {
                    return Err(format!("运行环境名称「{name}」已存在, 请换一个名称"));
                }
                let channel = crate::channel::resolve(
                    &app.state.packages,
                    settings.active_game.as_str(),
                    settings.current().distribution.as_ref(),
                )
                .map_err(|error| error.to_string())?;
                let target = installation_for(&app.state.packages, &settings, channel)
                    .ok_or("当前渠道不可用")?;
                if !target.installed {
                    return Err(crate::environment_commands::missing_install_error(
                        channel,
                        &target.install_root,
                    ));
                }
                Ok(Confirmation {
                    name,
                    channel: channel.id.to_string(),
                    install_root: target.install_root,
                })
            })
            .transpose()?;
        (preferred, confirmation)
    };
    let proton =
        proton
            .map(PathBuf::from)
            .or(preferred)
            .or(Receipt::load(&crate::paths::data_dir())
                .map_err(|error| format!("{error:#}"))?
                .proton);
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let plan = runtime_setup::plan(
        &crate::paths::data_dir(),
        &crate::paths::home_dir(),
        all_users,
        proton.as_deref(),
        &os,
    )
    .map_err(|error| format!("{error:#}"))?;
    setup.plan = Some(plan.clone());
    setup.confirmation = confirmation;
    setup.status = Status::default();
    Ok(plan)
}

pub fn status(app: &ServiceHandle) -> Result<Status, String> {
    Ok(app
        .runtime_setup
        .lock()
        .map_err(|error| error.to_string())?
        .status
        .clone())
}

pub async fn apply(app: ServiceHandle) -> Result<(), String> {
    let (plan, confirmation) = {
        let mut setup = app
            .runtime_setup
            .lock()
            .map_err(|error| error.to_string())?;
        if setup.status.busy {
            return Err("启动环境正在配置".into());
        }
        let plan = setup.plan.take().ok_or("请先查看安装计划")?;
        let confirmation = setup.confirmation.take();
        setup.status = Status {
            busy: true,
            ..Default::default()
        };
        (plan, confirmation)
    };
    let worker = app.clone();
    let result = crate::service::spawn_blocking(move || {
        install_components(&worker, &plan)?;
        if let Some(confirmation) = &confirmation {
            create_confirmed_environment(&worker, &plan, confirmation)?;
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    {
        let mut setup = app
            .runtime_setup
            .lock()
            .map_err(|error| error.to_string())?;
        setup.status.busy = false;
        setup.status.done = result.is_ok();
        setup.status.error = result.as_ref().err().cloned();
    }
    result
}

/// 只安装启动组件, 不接触运行环境.
fn install_components(worker: &ServiceHandle, plan: &Plan) -> Result<(), String> {
    let _task = crate::environment_commands::Task::begin(&worker.state)?;
    runtime_setup::install(&crate::paths::data_dir(), plan, |progress| {
        if let Ok(mut setup) = worker.runtime_setup.lock() {
            setup.status.progress = progress;
            let _ = worker.emit("runtime-setup-state", setup.status.clone());
        }
    })
    .map_err(|error| format!("{error:#}"))?;
    Ok(())
}

/// 按确认时的快照创建并切换运行环境, 然后初始化前缀.
fn create_confirmed_environment(
    worker: &ServiceHandle,
    plan: &Plan,
    confirmation: &Confirmation,
) -> Result<(), String> {
    let state = &worker.state;
    {
        let settings = state
            .settings
            .lock()
            .map_err(|error| error.to_string())?
            .clone();
        let channel = crate::channel::resolve(
            &state.packages,
            settings.active_game.as_str(),
            settings.current().distribution.as_ref(),
        )
        .map_err(|error| error.to_string())?;
        let current =
            installation_for(&state.packages, &settings, channel).ok_or("当前渠道不可用")?;
        if channel.id != confirmation.channel
            || !current.installed
            || !oddity_platform::fs_tree::same_path(
                &current.install_root,
                &confirmation.install_root,
            )
        {
            return Err("游戏地区或安装目录已变化, 请重新确认运行环境".into());
        }
    }
    let created = crate::environment_commands::create_impl(
        state,
        None,
        &confirmation.name,
        &plan.proton.to_string_lossy(),
    )?;
    let id = created
        .settings
        .runtime_environments
        .last()
        .ok_or("创建运行环境失败: 设置中未登记新环境")?
        .id
        .clone();
    crate::environment_commands::action_impl(state, "switch", Some(id))?;
    prepare_prefix(worker)?;
    Ok(())
}

/// 对齐前缀并按需初始化 Windows 注册表.
fn prepare_prefix(worker: &ServiceHandle) -> Result<crate::bootstrap::Bootstrap, String> {
    let state = &worker.state;
    let _task = crate::environment_commands::Task::begin(state)?;
    let settings = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let selection = crate::environment_commands::resolve(&state.packages, &settings)?;
    selection
        .environment
        .proton
        .verify()
        .map_err(|error| format!("{error:#}"))?;
    let (prefix, _install_lock, _prefix_lock) =
        crate::environment_commands::lock_and_align(state, &settings, &selection)?;
    if !prefix.join("system.reg").is_file() {
        let umu = runtime_setup::umu(&crate::paths::data_dir()).ok_or("未找到 umu-run")?;
        let proton = crate::environment_commands::proton_path(&state.packages, &settings)
            .ok_or("未选择 Proton")?;
        std::fs::create_dir_all(crate::paths::data_dir().join("logs"))
            .map_err(|error| error.to_string())?;
        runtime_setup::initialize(
            &umu,
            proton,
            &prefix,
            &crate::paths::data_dir().join("logs/runtime-setup.log"),
        )
        .map_err(|error| format!("{error:#}"))?;
    }
    Ok(crate::bootstrap::bootstrap(worker))
}

/// 启动前确认当前选择的环境可用; 不创建或切换环境.
pub async fn ensure_environment(app: ServiceHandle) -> Result<crate::bootstrap::Bootstrap, String> {
    let worker = app.clone();
    let mut bootstrap = crate::service::spawn_blocking(move || {
        let settings = worker
            .state
            .settings
            .lock()
            .map_err(|error| error.to_string())?
            .clone();
        if crate::environment_commands::selected(&worker.state.packages, &settings).is_err() {
            return Err("请先在设置中创建并选择 Proton 运行环境".into());
        }
        let channel = crate::channel::resolve(
            &worker.state.packages,
            settings.active_game.as_str(),
            settings.current().distribution.as_ref(),
        )
        .map_err(|error| error.to_string())?;
        let target =
            installation_for(&worker.state.packages, &settings, channel).ok_or("当前渠道不可用")?;
        if !target.installed {
            return Err(crate::environment_commands::missing_install_error(
                channel,
                &target.install_root,
            ));
        }
        prepare_prefix(&worker)
    })
    .await
    .map_err(|error| error.to_string())??;
    bootstrap.integrity = app.integrity.status();
    Ok(bootstrap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fake_proton, write_script, EnvGuard, TempDir};
    use std::sync::MutexGuard;

    // 组件安装与运行环境操作都读写进程环境, 用例之间必须串行.
    struct Fixture {
        directory: TempDir,
        app: ServiceHandle,
        proton: PathBuf,
        _env: EnvGuard,
        _serial: MutexGuard<'static, ()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            crate::testing::reset();
        }
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let _serial = crate::testing::serial();
            let mut env = EnvGuard::new();
            let directory = TempDir::new(tag);
            env.set("HOME", directory.join("home"));
            env.set("XDG_CONFIG_HOME", directory.join("config"));
            env.set("XDG_DATA_HOME", directory.join("data"));
            env.remove("NTE_ROOT");
            env.remove("NTE_INSTALL");
            std::fs::create_dir_all(directory.join("bin")).unwrap();
            // 假 umu 同时响应版本检查与 wineboot 初始化.
            write_script(
                &directory.join("bin"),
                "umu-run",
                "#!/bin/sh\ncase \"$1\" in\n  --version) exit 0 ;;\n  wineboot) printf registry > \"$WINEPREFIX/system.reg\" ;;\nesac\n",
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
            let proton = fake_proton(directory.path(), "Proton");
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
            Self {
                directory,
                app,
                proton,
                _env: env,
                _serial,
            }
        }

        fn proton_path(&self) -> String {
            self.proton.display().to_string()
        }

        fn install_root(&self) -> PathBuf {
            self.directory.join("install")
        }

        fn settings(&self) -> crate::settings::Settings {
            self.app.state.settings.lock().unwrap().clone()
        }

        fn prefix_of(&self, name: &str) -> PathBuf {
            let settings = self.settings();
            let environment = settings
                .runtime_environments
                .iter()
                .find(|environment| environment.name == name)
                .unwrap();
            environment.prefix(&crate::paths::data_dir()).unwrap()
        }
    }

    #[test]
    fn running_environment_requires_an_existing_selection_and_install() {
        let fixture = Fixture::new("runtime-environment-required");
        let missing = fixture.directory.join("missing-install");
        fixture
            .app
            .state
            .settings
            .lock()
            .unwrap()
            .current_mut()
            .install_roots
            .insert("cn".into(), missing.to_string_lossy().into());
        let error = crate::service::block_on(ensure_environment(fixture.app.clone())).unwrap_err();
        assert_eq!(error, "请先在设置中创建并选择 Proton 运行环境");
        assert!(fixture.settings().runtime_environments.is_empty());
        assert!(!crate::paths::data_dir().join("environments").exists());
        assert!(!missing.exists());
    }

    #[test]
    fn planning_without_environment_only_prepares_components() {
        let fixture = Fixture::new("runtime-components");
        let plan = plan(&fixture.app, false, Some(fixture.proton_path()), None).unwrap();
        assert!(!plan.install_umu);
        assert!(!plan.install_proton);
        assert!(fixture
            .app
            .runtime_setup
            .lock()
            .unwrap()
            .confirmation
            .is_none());
        crate::service::block_on(apply(fixture.app.clone())).unwrap();
        assert!(fixture.settings().runtime_environments.is_empty());
        assert!(fixture
            .settings()
            .current()
            .active_environment("cn")
            .is_none());
        assert!(!crate::paths::data_dir().join("environments").exists());
        let receipt = Receipt::load(&crate::paths::data_dir()).unwrap();
        assert!(receipt.seen);
        assert_eq!(receipt.proton.as_deref(), Some(fixture.proton.as_path()));
        let status = status(&fixture.app).unwrap();
        assert!(!status.busy);
        assert!(status.done);
        assert!(status.error.is_none());
        assert!(!fixture.app.state.game.lock().unwrap().environment_busy);
        let info = info(&fixture.app).unwrap();
        assert!(!info.ready);
        assert!(info.needs_setup);
        assert!(info.installed);
        assert_eq!(info.channel, "国服");
        assert_eq!(
            info.install_root,
            fixture.install_root().display().to_string()
        );
        assert_eq!(info.suggested_name, "异环-1");
    }

    #[test]
    fn confirmed_environment_is_created_selected_and_initialized() {
        let fixture = Fixture::new("runtime-confirmed");
        let name = "我的环境";
        plan(
            &fixture.app,
            false,
            Some(fixture.proton_path()),
            Some(name.into()),
        )
        .unwrap();
        assert!(fixture
            .app
            .runtime_setup
            .lock()
            .unwrap()
            .confirmation
            .is_some());
        crate::service::block_on(apply(fixture.app.clone())).unwrap();
        let settings = fixture.settings();
        assert_eq!(settings.runtime_environments.len(), 1);
        let environment = &settings.runtime_environments[0];
        assert_eq!(environment.name, name);
        assert_eq!(
            settings.current().active_environment("cn"),
            Some(environment.id.as_str())
        );
        let prefix = environment.prefix(&crate::paths::data_dir()).unwrap();
        assert!(prefix.join("system.reg").is_file());
        assert!(fixture
            .app
            .runtime_setup
            .lock()
            .unwrap()
            .confirmation
            .is_none());
        let info = info(&fixture.app).unwrap();
        assert!(info.ready);
        assert!(!info.needs_setup);
        assert_eq!(
            info.environment
                .as_ref()
                .map(|environment| environment.id.as_str()),
            Some(environment.id.as_str())
        );
        assert_eq!(info.suggested_name, "异环-1");
    }

    #[test]
    fn confirmed_environment_rejects_uninstalled_channel() {
        let fixture = Fixture::new("runtime-uninstalled");
        let missing = fixture.directory.join("missing-install");
        fixture
            .app
            .state
            .settings
            .lock()
            .unwrap()
            .current_mut()
            .install_roots
            .insert("cn".into(), missing.to_string_lossy().into());
        let error = plan(
            &fixture.app,
            false,
            Some(fixture.proton_path()),
            Some("环境".into()),
        )
        .unwrap_err();
        assert!(
            error.contains("安装目录未找到官方启动器或游戏文件"),
            "{error}"
        );
        assert!(error.contains("国服"), "{error}");
        assert!(!missing.exists());
        assert!(fixture.app.runtime_setup.lock().unwrap().plan.is_none());
        assert!(fixture
            .app
            .runtime_setup
            .lock()
            .unwrap()
            .confirmation
            .is_none());
    }

    #[test]
    fn duplicate_environment_name_is_rejected_before_the_plan_is_written() {
        let fixture = Fixture::new("runtime-duplicate");
        let name = "重复名称";
        plan(
            &fixture.app,
            false,
            Some(fixture.proton_path()),
            Some(name.into()),
        )
        .unwrap();
        crate::service::block_on(apply(fixture.app.clone())).unwrap();
        let error = plan(
            &fixture.app,
            false,
            Some(fixture.proton_path()),
            Some(name.into()),
        )
        .unwrap_err();
        assert_eq!(error, "运行环境名称「重复名称」已存在, 请换一个名称");
        assert_eq!(fixture.settings().runtime_environments.len(), 1);
        {
            let setup = fixture.app.runtime_setup.lock().unwrap();
            assert!(setup.plan.is_none());
            assert!(setup.confirmation.is_none());
        }
        assert!(!fixture.app.state.game.lock().unwrap().environment_busy);

        // 计划生成后名称也可能被其他操作占用, 创建时再次校验.
        let another = "另一个名称";
        plan(
            &fixture.app,
            false,
            Some(fixture.proton_path()),
            Some(another.into()),
        )
        .unwrap();
        crate::environment_commands::create_impl(
            &fixture.app.state,
            None,
            another,
            &fixture.proton_path(),
        )
        .unwrap();
        let error = crate::service::block_on(apply(fixture.app.clone())).unwrap_err();
        assert_eq!(error, "运行环境名称「另一个名称」已存在, 请换一个名称");
        assert_eq!(fixture.settings().runtime_environments.len(), 2);
        let status = status(&fixture.app).unwrap();
        assert!(!status.busy);
        assert!(!status.done);
        assert!(status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("另一个名称")));
        assert!(fixture.app.runtime_setup.lock().unwrap().plan.is_none());
        assert!(!fixture.app.state.game.lock().unwrap().environment_busy);
        assert_eq!(
            crate::service::block_on(apply(fixture.app.clone())).unwrap_err(),
            "请先查看安装计划"
        );
    }

    #[test]
    fn plan_falls_back_from_a_stale_receipt_and_validates_manual_proton() {
        let fixture = Fixture::new("runtime-stale-receipt");
        let stale = fixture.directory.join("old-steam/Proton");
        let default_target = runtime_setup::steam_tools(&crate::paths::home_dir())
            .join(runtime_setup::PROTON_VERSION);
        for incomplete in [false, true] {
            if incomplete {
                fixture.directory.write("old-steam/Proton/proton", "proton");
            }
            Receipt {
                seen: true,
                proton: Some(stale.clone()),
            }
            .save(&crate::paths::data_dir())
            .unwrap();
            assert!(info(&fixture.app).unwrap().proton.is_none());
            let automatic = plan(&fixture.app, false, None, None).unwrap();
            assert!(automatic.install_proton);
            assert_eq!(automatic.proton, default_target);

            // 手工 Proton 必须是指向完整 Proton 的绝对目录.
            for manual in [stale.display().to_string(), "relative/proton".to_string()] {
                let error = plan(&fixture.app, false, Some(manual), None).unwrap_err();
                assert!(error.contains("Proton 绝对路径"), "{error}");
            }
        }
        fake_proton(fixture.directory.path(), "old-steam/Proton");
        assert_eq!(plan(&fixture.app, false, None, None).unwrap().proton, stale);
    }

    #[test]
    fn runtime_environment_reuses_or_repairs_the_selection() {
        let fixture = Fixture::new("runtime-repair");
        let name = "修复环境";
        plan(
            &fixture.app,
            false,
            Some(fixture.proton_path()),
            Some(name.into()),
        )
        .unwrap();
        crate::service::block_on(apply(fixture.app.clone())).unwrap();
        let prefix = fixture.prefix_of(name);
        std::fs::remove_file(prefix.join("system.reg")).unwrap();
        let repaired = crate::service::block_on(ensure_environment(fixture.app.clone())).unwrap();
        assert_eq!(repaired.settings.runtime_environments.len(), 1);
        assert!(prefix.join("system.reg").is_file());
        let reused = crate::service::block_on(ensure_environment(fixture.app.clone())).unwrap();
        assert_eq!(
            reused.settings.runtime_environments,
            repaired.settings.runtime_environments
        );
        assert!(prefix.join("system.reg").is_file());
        assert_eq!(
            reused
                .settings
                .current()
                .active_environment("cn")
                .map(str::to_string),
            Some(repaired.settings.runtime_environments[0].id.clone())
        );
        std::fs::remove_file(
            fixture
                .directory
                .join(format!("install/{}", crate::testing::GAME_EXE)),
        )
        .unwrap();
        let error = crate::service::block_on(ensure_environment(fixture.app.clone())).unwrap_err();
        assert!(
            error.contains("安装目录未找到官方启动器或游戏文件"),
            "{error}"
        );
        assert!(error.contains("国服"), "{error}");
    }
}
