#![cfg(feature = "game-nte")]

//! 宿主会话与《异环》启动实现的集成用例.
//!
//! 启动配方与进程拉起属于游戏包, 会话注册表, 租约, 快照与收尾属于宿主;
//! 这组用例只在装配了真实包的 crate 里运行.

#![allow(clippy::type_complexity)]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oddity_host::channel::ChannelId;
use oddity_host::paths::data_dir;
use oddity_host::service::{Service, ServiceConfig, ServiceHandle};
use oddity_host::session::host::HostSession;
use oddity_host::settings::Settings;
use oddity_host::{environment_commands, performance_commands, snapshot_commands};
use oddity_platform::paths::windows_path;
use oddity_platform::process;
use oddity_runtime::{GamePackage, GameProcess, LaunchResult, SessionHost, SessionTicket};

mod support;

use support::{
    fake_proton, managed_environment, settings_with_game, write_script, EnvGuard, TempDir,
};

thread_local! {
    /// 服务句柄只借用路径, 临时目录跟着用例线程释放.
    static KEEPALIVE: std::cell::RefCell<Vec<TempDir>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// 装配了《异环》包的宿主服务.
fn test_service() -> ServiceHandle {
    test_service_with_output().0
}

/// 同上, 保留事件接收端用来断言输出.
fn test_service_with_output() -> (ServiceHandle, std::sync::mpsc::Receiver<serde_json::Value>) {
    let (events, receiver) = std::sync::mpsc::sync_channel(256);
    let (input, jobs) = std::sync::mpsc::sync_channel(256);
    drop(jobs);
    let directory = TempDir::new("service");
    let data_dir = directory.path().to_path_buf();
    KEEPALIVE.with(|slot| slot.borrow_mut().push(directory));
    let mut registry = oddity_runtime::Registry::new();
    registry.install(&oddity_game_nte::NTE_PACKAGE);
    let service = Service::new(ServiceConfig {
        settings: Settings::default(),
        data_dir,
        registry: std::sync::Arc::new(registry),
        events,
        input,
    });
    (service, receiver)
}

fn wait_until_install_lock_released(install: &std::path::Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if oddity_platform::install_lock::InstallLock::acquire(install).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

/// 一次已经拉起的会话; 进程与租约都挂在宿主注册表上.
struct StartedSession {
    result: LaunchResult,
    process: Arc<dyn GameProcess>,
    pgid: i32,
    performance: Option<Arc<Mutex<Option<oddity_host::performance::Recording>>>>,
    app: ServiceHandle,
}

#[test]
fn gamescope_preserves_the_umu_handoff_and_environment() {
    let mut fixture = LaunchFixture::new("gamescope-launch");
    fixture.env.set("WAYLAND_DISPLAY", "wayland-test");
    fixture.env.set("PROTON_ENABLE_WAYLAND", "1");
    fixture.env.set("PROTON_USE_WAYLAND", "1");
    let options_log = fixture.dir.join("gamescope-args");
    write_script(&fixture.dir.join("bin"), "gamescope", &format!(
        "#!/bin/sh\nwhile [ \"$1\" != -- ]; do printf '%s\\n' \"$1\" >> '{}'; shift; done\nshift\nexec \"$@\"\n", options_log.display()
    ));
    let mut settings = fixture.settings("official");
    settings.current_mut().game_mode.gamescope = oddity_host::game_mode::gamescope::Options {
        enabled: true,
        game_width: 1280,
        game_height: 720,
        fps: 60,
        ..Default::default()
    };
    let invocation = launch(&settings).unwrap();
    let dump = fixture.dump();
    assert!(dump.contains("NTEGame.exe") && dump.contains("WINEPREFIX="));
    assert!(env_value(&dump, "PULSE_PROP")
        .unwrap()
        .contains("oddity-managed-game"));
    assert_eq!(env_value(&dump, "PROTON_ENABLE_WAYLAND"), Some("0"));
    assert_eq!(env_value(&dump, "PROTON_USE_WAYLAND"), Some("0"));
    let options = std::fs::read_to_string(options_log).unwrap();
    assert!(options.contains("-w\n1280\n-h\n720\n-r\n60\n"));
    fixture.stop(invocation);
}

/// 一次完整的启动环境：安装目录、prefix、Proton、假 umu-run。
struct LaunchFixture {
    dir: TempDir,
    launcher: PathBuf,
    prefix: PathBuf,
    proton: PathBuf,
    profile: oddity_host::environment::RuntimeEnvironment,
    dump: PathBuf,
    path: String,
    env: EnvGuard,
}

impl LaunchFixture {
    fn new(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let mut env = EnvGuard::new();
        env.set("HOME", dir.join("home"));
        env.set("XDG_DATA_HOME", dir.join("data"));
        std::fs::create_dir_all(dir.join("home")).expect("create fake home");
        env.remove("WINEPREFIX");
        env.remove("PROTONPATH");
        env.remove("WAYLAND_DISPLAY");
        env.remove("NTE_ROOT");
        env.remove("NTE_INSTALL");
        env.remove("STEAM_COMPAT_CLIENT_INSTALL_PATH");
        env.remove("NTE_DISABLE_PRESENCE_TUNING");

        let launcher = dir.join("install/NTELauncher");
        std::fs::create_dir_all(&launcher).expect("create launcher dir");
        dir.write_text(
            "install/NTELauncher/Config/Config.ini",
            "[VERSION]\nVersion=1.4.16\n",
        );
        dir.write(
            "install/Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
            b"MZ",
        );
        dir.write("install/NTELauncher/NTEGame.exe", b"MZ");

        let proton = fake_proton(&dir.join("tools"), "GE-Proton9-1");
        let profile = managed_environment(&proton, &dir.join("install"), ChannelId::new("cn"));
        let prefix = profile.prefix(&data_dir()).unwrap();
        std::fs::create_dir_all(proton.join("files/bin")).expect("create wine dir");
        let wine_log = dir.join("wine.log");
        write_script(
            &proton.join("files/bin"),
            "wine",
            &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", wine_log.display()),
        );

        let dump = dir.join("umu-dump.txt");
        let script = format!(
            "#!/bin/sh\n{{ echo \"args: $@\"; echo \"cwd: $(pwd)\"; env; }} > {dump}.tmp\nmv {dump}.tmp {dump}\nsleep 10\n",
            dump = dump.display()
        );
        let bin = dir.join("bin");
        write_script(&bin, "umu-run", &script);
        // 假脚本自己也要能找到 env、mv 这些系统命令.
        let inherited = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{inherited}", bin.display());
        env.set("PATH", &path);

        Self {
            dir,
            launcher,
            prefix,
            proton,
            profile,
            dump,
            path,
            env,
        }
    }

    fn settings(&self, mode: &str) -> Settings {
        let mut settings =
            settings_with_game(ChannelId::new("cn"), Some(self.launcher.clone()), None);
        settings.runtime_environments = vec![self.profile.clone()];
        settings.current_mut().active_environment = Some(self.profile.id.clone());
        settings.current_mut().launch_mode = mode.to_string();
        settings
    }

    /// 等假 umu-run 把参数与环境变量写出来.
    fn dump(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.dump.is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::read_to_string(&self.dump).expect("read umu dump")
    }

    /// 和应用的监控线程一样：先有人回收组长，进程组才可能真正消失。
    fn stop(&self, invocation: StartedSession) {
        let process = invocation.process;
        let app = invocation.app;
        let waiter = thread::spawn(move || process.wait());
        process::terminate_group(invocation.pgid).expect("terminate launched group");
        let _ = waiter.join();
        // 等监视线程完成终态收尾, 租约随会话记录释放后再让测试断言.
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.state.game.lock().unwrap().session.is_some() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// 用宿主会话与注册表里的启动实现拉起一次.
fn test_launch_in(
    app: ServiceHandle,
    settings: &Settings,
    cancel: Arc<AtomicBool>,
) -> Result<StartedSession, String> {
    let host = HostSession::new(app.clone(), settings.clone(), 1, cancel);
    let provider = oddity_game_nte::NTE_PACKAGE
        .launch_provider()
        .expect("当前游戏应提供启动实现");
    let ticket = SessionTicket::new("1", Arc::clone(&host) as Arc<dyn SessionHost>);
    let result = provider.launch(ticket).map_err(|error| error.message)?;
    let process = host.process().ok_or_else(|| "启动进程缺失".to_string())?;
    let session = host.session().ok_or_else(|| "会话记录缺失".to_string())?;
    let performance = session.performance.lock().ok().and_then(|recording| {
        recording
            .is_some()
            .then(|| Arc::clone(&session.performance))
    });
    Ok(StartedSession {
        result,
        pgid: process.pgid(),
        process,
        performance,
        app,
    })
}

fn launch(settings: &Settings) -> Result<StartedSession, String> {
    test_launch_in(test_service(), settings, Arc::new(AtomicBool::new(false)))
}

/// 期待启动失败；万一真的起来了，先收掉进程再报错。
fn expect_launch_error(result: Result<StartedSession, String>) -> String {
    match result {
        Ok(invocation) => {
            let _ = process::terminate_group(invocation.pgid);
            panic!("expected the launch to fail");
        }
        Err(error) => error,
    }
}

/// 从 env 输出里取一个变量的值。
fn env_value<'a>(dump: &'a str, key: &str) -> Option<&'a str> {
    dump.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
}

#[test]
fn managed_environment_launch_uses_its_prefix_proton_and_log_directory() {
    let mut fixture = LaunchFixture::new("launch-managed-environment");
    fixture.env.set("NTE_DISABLE_PRESENCE_TUNING", "1");
    let target = fake_proton(fixture.dir.path(), "GE-Proton-New");
    let profile = oddity_host::environment::create(
        &data_dir(),
        Some((&fixture.prefix, &fixture.proton)),
        &target,
        &fixture.dir.join("install"),
        oddity_host::environment::EnvironmentTarget {
            game: oddity_host::identity::GameId::from("nte"),
            distribution: ChannelId::new("cn"),
            link_name: Some("Program Files (x86)/Neverness To Everness"),
        },
        "新版测试",
    )
    .unwrap();
    let mut settings = fixture.settings("official");
    settings.current_mut().active_environment = Some(profile.id.clone());
    settings.runtime_environments.push(profile.clone());
    let invocation = launch(&settings).unwrap();
    let prefix = profile.prefix(&data_dir()).unwrap();
    let dump = fixture.dump();
    assert_eq!(env_value(&dump, "WINEPREFIX"), prefix.to_str());
    assert_eq!(env_value(&dump, "PROTONPATH"), target.to_str());
    assert!(invocation
        .result
        .log
        .contains(&format!("logs/environments/{}", profile.id)));
    assert!(oddity_host::environment::lock_prefix(&prefix).is_err());
    fixture.stop(invocation);
    assert!(oddity_host::environment::lock_prefix(&prefix).is_ok());
    fixture.dir.write("GE-Proton-New/proton", "changed version");
    assert!(expect_launch_error(launch(&settings)).contains("Proton 已变化"));
    settings.current_mut().active_environment = None;
    assert!(
        environment_commands::prefix(&support::packages(), &settings)
            .unwrap_err()
            .contains("请先")
    );
}

#[test]
fn launch_reports_a_configured_empty_root_as_not_installed() {
    let fixture = LaunchFixture::new("launch-empty-root");
    let mut settings = fixture.settings("official");
    settings.current_mut().install_roots.insert(
        "cn".into(),
        fixture.dir.join("empty-install").display().to_string(),
    );

    let error = expect_launch_error(launch(&settings));
    assert!(error.contains("尚未安装启动器文件"), "{error}");
    assert!(!fixture.dump.exists());
}

#[test]
fn global_launch_handoff_uses_global_client_for_official_and_background_modes() {
    for mode in ["official", "background"] {
        let fixture = LaunchFixture::new("launch-global");
        let global = fixture.dir.join("install/NTEGlobal");
        std::fs::rename(&fixture.launcher, &global).unwrap();
        std::fs::rename(global.join("NTEGame.exe"), global.join("NTEGlobalGame.exe")).unwrap();
        fixture.dir.write_text(
            "install/NTEGlobal/Config/Config.ini",
            "[General]\nGameID=3000001\n",
        );
        let mut settings = fixture.settings(mode);
        settings.current_mut().distribution = Some(ChannelId::new("global"));
        settings
            .current_mut()
            .install_roots
            .insert("global".into(), global.display().to_string());
        settings.runtime_environments[0].distribution = ChannelId::new("global");
        let invocation = launch(&settings).expect("global handoff");
        assert!(invocation.result.exe.ends_with("NTEGlobalGame.exe"));
        let dump = fixture.dump();
        if mode == "background" {
            assert_eq!(
                env_value(&dump, "NTE_BRIDGE_EXE"),
                Some(windows_path(&global.join("NTEGlobalGame.exe")).as_str())
            );
        } else {
            assert!(
                dump.contains("NTEGlobalGame.exe /launcher /directly"),
                "{dump}"
            );
        }
        fixture.stop(invocation);
    }
}

#[test]
fn launch_requires_the_expected_tools_and_files() {
    let mut fixture = LaunchFixture::new("launch-errors");
    let settings = fixture.settings("official");

    // PATH 里没有 umu-run
    fixture.env.set("PATH", fixture.dir.join("empty-bin"));
    let error = expect_launch_error(launch(&settings));
    assert!(error.contains("PATH 中未找到 umu-run"), "{error}");

    let path = fixture.path.clone();
    fixture.env.set("PATH", path);

    let mut without_proton = settings.clone();
    without_proton.runtime_environments[0].proton.path = fixture.dir.join("missing");
    assert!(expect_launch_error(launch(&without_proton)).contains("Proton"));
    let mut unselected = settings.clone();
    unselected.current_mut().active_environment = None;
    assert!(expect_launch_error(launch(&unselected)).contains("请先"));

    // 官方启动器目录里没有 NTEGame.exe
    std::fs::create_dir_all(fixture.dir.join("bare-install/NTELauncher")).expect("create dir");
    fixture
        .dir
        .write_text("bare-install/NTELauncher/Config/Config.ini", "[VERSION]\n");
    let mut bare = settings.clone();
    bare.current_mut().install_roots.insert(
        "cn".into(),
        fixture
            .dir
            .join("bare-install/NTELauncher")
            .display()
            .to_string(),
    );
    bare.runtime_environments[0].install_root = fixture.dir.join("bare-install");
    let error = expect_launch_error(launch(&bare));
    assert!(error.contains("官方启动器不存在"), "{error}");

    // 直接模式找不到游戏可执行文件
    let mut no_game = bare.clone();
    no_game.current_mut().launch_mode = "direct".to_string();
    let error = expect_launch_error(launch(&no_game));
    assert!(error.contains("未找到 HTGame.exe"), "{error}");
}

#[test]
fn launch_records_an_environment_snapshot() {
    let fixture = LaunchFixture::new("snapshot-launch");
    let invocation = launch(&fixture.settings("official")).unwrap();
    let dump = fixture.dump();
    assert!(dump.contains("NTEGame.exe"), "{dump}");

    let overview = invocation.app.snapshots.overview().unwrap();
    let latest = overview.latest.expect("snapshot");
    assert_eq!(latest.environment.as_deref(), Some("测试环境"));
    assert_eq!(latest.proton.as_deref(), Some("GE-Proton9-1"));
    assert_eq!(latest.mode, "official");
    assert_eq!(latest.status, oddity_host::snapshots::Status::Starting);
    let log = std::fs::read_to_string(&invocation.result.log).unwrap();
    assert!(log.contains("oddity="), "{log}");
    fixture.stop(invocation);
}

#[test]
fn snapshot_inputs_follow_the_managed_environment() {
    let fixture = LaunchFixture::new("snapshot-inputs");
    let config = fixture.launcher.join("UserData/Config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("Config.ini"), "[Game]\nVersion_=1.4.16\n").unwrap();

    let inputs =
        snapshot_commands::current_inputs(&support::packages(), &fixture.settings("direct"));
    assert_eq!(inputs.distribution, ChannelId::new("cn"));
    assert_eq!(
        inputs.environment_id.as_deref(),
        Some(fixture.profile.id.as_str())
    );
    assert_eq!(inputs.environment_name.as_deref(), Some("测试环境"));
    assert_eq!(inputs.proton_name.as_deref(), Some("GE-Proton9-1"));
    assert!(inputs.proton_ok);
    assert_eq!(inputs.prefix.as_deref(), Some(fixture.prefix.as_path()));
    assert!(inputs.prefix_ok);
    assert_eq!(inputs.game_version.as_deref(), Some("1.4.16"));
    assert_eq!(inputs.launcher_version.as_deref(), Some("1.4.16"));
    assert_eq!(
        inputs.launcher_root.as_deref(),
        Some(fixture.launcher.as_path())
    );
    assert_eq!(inputs.launch_mode, "direct");
    assert!(!inputs.gamescope.enabled);
    assert!(inputs.umu.is_some());
}

#[test]
fn immediate_exit_marks_the_snapshot_failed() {
    let fixture = LaunchFixture::new("snapshot-failure");
    write_script(&fixture.dir.join("bin"), "umu-run", "#!/bin/sh\nexit 3\n");
    let app = test_service();
    let error = test_launch_in(
        oddity_host::service::ServiceHandle::clone(&app),
        &fixture.settings("official"),
        Arc::new(AtomicBool::new(false)),
    )
    .err()
    .expect("early exit fails the launch");
    assert!(error.contains("退出码 3"), "{error}");

    let latest = app.snapshots.overview().unwrap().latest.expect("snapshot");
    assert_eq!(latest.status, oddity_host::snapshots::Status::Failed);
    let failure = latest.failure.expect("failure");
    assert_eq!(
        failure.reason,
        oddity_host::snapshots::FailureReason::EarlyExit
    );
    assert_eq!(failure.exit_code, Some(3));
}

/// 装配宿主服务, 并接上真实数据目录里的性能记录.
fn service_with_performance() -> ServiceHandle {
    let mut app = test_service();
    let performance = performance_commands::Performance::load(&data_dir());
    std::sync::Arc::get_mut(&mut app).unwrap().performance = std::sync::Arc::new(performance);
    app
}

#[test]
fn performance_sampling_is_attached_only_while_recording_is_on() {
    let mut fixture = LaunchFixture::new("performance-launch");
    fixture.env.set("MANGOHUD_CONFIG", "read_cfg,fps_limit=75");
    let mut store = oddity_host::performance::Store::load(data_dir().join("performance")).unwrap();
    // 开关默认关闭, 启动不挂采样.
    let app = service_with_performance();
    let invocation = test_launch_in(
        oddity_host::service::ServiceHandle::clone(&app),
        &fixture.settings("official"),
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .expect("launch without recording");
    assert!(invocation.performance.is_none());
    fixture.stop(invocation);

    store.set_recording(true).unwrap();
    let app = service_with_performance();
    let _ = std::fs::remove_file(&fixture.dump);
    let invocation = test_launch_in(
        oddity_host::service::ServiceHandle::clone(&app),
        &fixture.settings("official"),
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .expect("performance launch");
    assert!(invocation.performance.is_some());
    let dump = fixture.dump();
    let config = env_value(&dump, "MANGOHUD_CONFIG").unwrap();
    assert!(config.contains("fps_limit=75"));
    assert!(config.contains("log_interval=0"));
    assert!(config.contains("upload_logs=0"));
    assert_eq!(env_value(&dump, "MANGOHUD"), Some("1"));
    assert_eq!(
        std::env::var("MANGOHUD_CONFIG").unwrap(),
        "read_cfg,fps_limit=75"
    );
    fixture.stop(invocation);
}

/// 一次完整启动: 注册表登记会话, 监视线程接管, 关闭后回到空闲.
#[test]
fn start_launch_registers_the_session_and_close_game_finalizes_it() {
    let fixture = LaunchFixture::new("session-lifecycle");
    let settings = fixture.settings("official");
    let mut app = test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = oddity_host::AppState {
        settings: Mutex::new(settings.clone()),
        game: Mutex::new(oddity_host::session::Registry::default()),
        packages: support::packages(),
    };
    let result =
        oddity_host::service::block_on(oddity_host::session::start_launch(app.clone(), settings))
            .expect("launch starts");
    assert!(result.pid > 0);
    assert_eq!(result.mode, "official");
    let running = oddity_host::session::game_status(&app.state);
    assert!(running.running);
    assert!(!running.closing);
    assert!(app.state.game.lock().unwrap().session.is_some());
    // 会话持有安装租约, 关闭后才能再次领取.
    assert!(
        oddity_platform::install_lock::InstallLock::acquire(&fixture.dir.join("install")).is_err()
    );

    let closed =
        oddity_host::service::block_on(oddity_host::session::close_game(app.clone(), &app.state))
            .expect("close game");
    assert!(!closed.running);
    // 监控线程完成收尾后注册表清空, 会话可以再次启动.
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.state.game.lock().unwrap().session.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(app.state.game.lock().unwrap().session.is_none());
    assert!(!oddity_host::session::game_status(&app.state).running);
    // 监视线程交出终态后才释放安装租约.
    let released = wait_until_install_lock_released(&fixture.dir.join("install"));
    assert!(released, "关闭后安装租约应释放");
}

/// 伪 provider + 脚本化进程驱动完整 game-state 序列, 宿主回调不持注册表锁.
#[test]
fn pseudo_provider_drives_the_session_state_sequence() {
    struct PseudoProvider;

    impl oddity_runtime::LaunchProvider for PseudoProvider {
        fn launch(
            &self,
            ticket: oddity_runtime::SessionTicket,
        ) -> Result<oddity_runtime::LaunchResult, oddity_runtime::LaunchError> {
            let plan = ticket.plan()?;
            let mut command = std::process::Command::new("/bin/sh");
            command
                .arg("-c")
                .arg("exec -a HTGame.exe /bin/sleep 0.2")
                .current_dir(&plan.install_root);
            let process = ticket.spawn(oddity_runtime::SpawnRequest {
                command,
                log: None,
                stderr_null: true,
                private_umask: false,
            })?;
            let result = oddity_runtime::LaunchResult {
                mode: "official".into(),
                pid: process.pid(),
                exe: plan.launcher_root.join("NTEGame.exe").display().to_string(),
                prefix: plan.prefix.display().to_string(),
                log: plan
                    .log_dir
                    .join("nte-app-launch.log")
                    .display()
                    .to_string(),
                started_at: 1,
            };
            ticket.started(oddity_runtime::StartedFacts {
                exe: result.exe.clone(),
                log: result.log.clone(),
            })?;
            std::thread::sleep(Duration::from_millis(120));
            ticket.sample(oddity_runtime::SessionSample {
                game_running: true,
                game_pids: vec![process.pid()],
            });
            ticket.game_exited(None);
            std::thread::sleep(Duration::from_millis(50));
            ticket.end(oddity_runtime::SessionEnd {
                game_started: true,
                exit_code: Some(0),
                failure: None,
                forced: false,
            });
            Ok(result)
        }
    }

    let fixture = LaunchFixture::new("session-pseudo-provider");
    let settings = fixture.settings("background");
    let (app, events) = test_service_with_output();
    // 事件回调里读注册表: 宿主发出事件时不应持锁, 否则这里会卡住.
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = app.clone();
    app.subscribe("game-state", move |_| {
        let _ = sender.send(reader.state.game.lock().unwrap().status());
    });
    let cancel = {
        let mut registry = app.state.game.lock().unwrap();
        oddity_host::session::prepare_launch(&app.state.packages, &mut registry).unwrap();
        oddity_host::session::begin_launch(&mut registry, &settings);
        registry.next_id = 1;
        std::sync::Arc::new(AtomicBool::new(false))
    };
    oddity_host::session::emit_game_status(&app, &app.state);
    let host: std::sync::Arc<dyn oddity_runtime::SessionHost> =
        oddity_host::session::host::HostSession::new(app.clone(), settings.clone(), 1, cancel);
    let ticket = oddity_runtime::SessionTicket::new("1", host);
    <PseudoProvider as oddity_runtime::LaunchProvider>::launch(&PseudoProvider, ticket)
        .expect("pseudo launch");

    let mut payloads = Vec::new();
    while let Ok(message) = events.try_recv() {
        if message["event"] == "game-state" {
            payloads.push(message["payload"].clone());
        }
    }
    assert_eq!(payloads.len(), 4, "{payloads:?}");
    assert_eq!(payloads[0]["running"], serde_json::json!(false));
    assert_eq!(
        payloads[0]["launch"]["stage"],
        serde_json::json!("preparing")
    );
    assert_eq!(payloads[1]["running"], serde_json::json!(true));
    assert_eq!(payloads[1]["launch"]["stage"], serde_json::json!("helper"));
    assert_eq!(payloads[1]["exe"], serde_json::json!(payloads[1]["exe"]));
    assert_eq!(payloads[2]["running"], serde_json::json!(true));
    assert_eq!(payloads[3]["running"], serde_json::json!(false));
    assert!(payloads[3].get("launch").is_none());
    assert!(receiver.try_recv().is_ok(), "事件回调读取注册表不应持有锁");
    assert!(app.state.game.lock().unwrap().session.is_none());
}

/// 启动实现交出失败终态: 注册表记录错误并回到空闲.
#[test]
fn provider_end_failure_records_the_launch_error() {
    struct FailingProvider;

    impl oddity_runtime::LaunchProvider for FailingProvider {
        fn launch(
            &self,
            ticket: oddity_runtime::SessionTicket,
        ) -> Result<LaunchResult, oddity_runtime::LaunchError> {
            let plan = ticket.plan()?;
            let mut command = std::process::Command::new("/bin/true");
            command.current_dir(&plan.install_root);
            let process = ticket.spawn(oddity_runtime::SpawnRequest {
                command,
                log: None,
                stderr_null: true,
                private_umask: false,
            })?;
            ticket.started(oddity_runtime::StartedFacts {
                exe: "early.exe".into(),
                log: plan.log_dir.join("early.log").display().to_string(),
            })?;
            ticket.end(oddity_runtime::SessionEnd {
                game_started: false,
                exit_code: Some(1),
                failure: Some(oddity_runtime::LaunchFailure {
                    kind: oddity_runtime::LaunchFailureKind::EarlyExit,
                    detail: Some("后台包装进程在官方会话启动前退出.".into()),
                }),
                forced: false,
            });
            Ok(LaunchResult {
                mode: "background".into(),
                pid: process.pid(),
                exe: "early.exe".into(),
                prefix: plan.prefix.display().to_string(),
                log: plan.log_dir.join("early.log").display().to_string(),
                started_at: 0,
            })
        }
    }

    let fixture = LaunchFixture::new("session-end-failure");
    let settings = fixture.settings("background");
    let (app, _events) = test_service_with_output();
    let cancel = {
        let mut registry = app.state.game.lock().unwrap();
        oddity_host::session::prepare_launch(&app.state.packages, &mut registry).unwrap();
        oddity_host::session::begin_launch(&mut registry, &settings);
        registry.next_id = 1;
        std::sync::Arc::new(AtomicBool::new(false))
    };
    oddity_host::session::emit_game_status(&app, &app.state);
    let session_host =
        oddity_host::session::host::HostSession::new(app.clone(), settings.clone(), 1, cancel);
    let ticket = SessionTicket::new("1", Arc::clone(&session_host) as Arc<dyn SessionHost>);
    <FailingProvider as oddity_runtime::LaunchProvider>::launch(&FailingProvider, ticket)
        .expect("failing provider returns the launch result");
    let registry = app.state.game.lock().unwrap();
    assert!(registry.session.is_none());
    assert_eq!(
        registry.last_error.as_deref(),
        Some("后台包装进程在官方会话启动前退出.")
    );
}

/// 设置里出现未知启动模式时直接拒绝, 不进入编排.
#[test]
fn launch_rejects_an_unknown_mode() {
    let fixture = LaunchFixture::new("launch-unknown-mode");
    let mut settings = fixture.settings("official");
    settings.current_mut().launch_mode = "weird".into();
    let error = expect_launch_error(launch(&settings));
    assert!(error.contains("未知的启动模式"), "{error}");
}

/// 启动实现拉起进程后异常返回 (未交出终态): 宿主经 abandon 终止回收并收尾.
#[test]
fn provider_abandon_cleans_up_after_an_abnormal_return() {
    struct AbandoningProvider;

    impl oddity_runtime::LaunchProvider for AbandoningProvider {
        fn launch(
            &self,
            ticket: oddity_runtime::SessionTicket,
        ) -> Result<oddity_runtime::LaunchResult, oddity_runtime::LaunchError> {
            let plan = ticket.plan()?;
            let mut command = std::process::Command::new("/bin/sleep");
            command.arg("30").current_dir(&plan.install_root);
            let process = ticket.spawn(oddity_runtime::SpawnRequest {
                command,
                log: None,
                stderr_null: true,
                private_umask: false,
            })?;
            ticket.started(oddity_runtime::StartedFacts {
                exe: "abandon.exe".into(),
                log: plan.log_dir.join("abandon.log").display().to_string(),
            })?;
            // 拉起后直接异常返回: 票据未 end, 由宿主兜底.
            Err(oddity_runtime::LaunchError::new(
                oddity_runtime::LaunchFailureKind::Backend,
                "编排线程异常退出",
            ))
            .map(|()| oddity_runtime::LaunchResult {
                mode: "official".into(),
                pid: process.pid(),
                exe: String::new(),
                prefix: String::new(),
                log: String::new(),
                started_at: 0,
            })
        }
    }

    let fixture = LaunchFixture::new("session-abandon");
    let settings = fixture.settings("official");
    let (app, _events) = test_service_with_output();
    let cancel = {
        let mut registry = app.state.game.lock().unwrap();
        oddity_host::session::prepare_launch(&app.state.packages, &mut registry).unwrap();
        oddity_host::session::begin_launch(&mut registry, &settings);
        registry.next_id = 1;
        std::sync::Arc::new(AtomicBool::new(false))
    };
    oddity_host::session::emit_game_status(&app, &app.state);
    let host: std::sync::Arc<dyn oddity_runtime::SessionHost> =
        oddity_host::session::host::HostSession::new(app.clone(), settings.clone(), 1, cancel);
    let ticket = oddity_runtime::SessionTicket::new("1", host);
    let error =
        <AbandoningProvider as oddity_runtime::LaunchProvider>::launch(&AbandoningProvider, ticket)
            .expect_err("abnormal provider returns an error");
    assert_eq!(error.message, "编排线程异常退出");
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.state.game.lock().unwrap().session.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(app.state.game.lock().unwrap().session.is_none());
}
