//! 启动域测试: 命令构造, presence 调优, helper 交接与监视线程.
//!
//! 宿主由假 SessionHost/SessionTicket 替代, 进程用脚本夹具运行, 覆盖
//! 三种模式的命令与环境, 早退与后台清理路径.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oddity_runtime::{
    LaunchFailureKind, LaunchMode, LaunchPlan, ProtonRef, RunnerSpec, SessionHost, SessionTarget,
    SessionTicket,
};

use crate::launch::recipe::NTE_LAUNCH;
use crate::launch::testing::{
    expect_launch_error, is_private, wait_until, Call, LaunchFixture, TestHost,
};
use crate::testing::TempDir;
use crate::update::region::{Region, LAYOUT};

fn env_value<'a>(dump: &'a str, key: &str) -> Option<&'a str> {
    dump.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
}

#[test]
fn official_launch_builds_the_launcher_command_and_presence() {
    let fixture = LaunchFixture::new("official-launch");
    fixture.dir.write(
        "install/NTELauncher/UserData/Config/Config.ini",
        "[Setting]\r\nautoLogin=1\r\n[User]\r\nuser_t92=deadbeef\r\n",
    );

    let (result, host) = fixture.run(LaunchMode::Official).expect("launch");
    assert_eq!(result.mode, "official");
    assert_eq!(result.pid, host.pgid().expect("spawned process") as u32);
    assert!(result.exe.ends_with("NTEGame.exe"));
    assert_eq!(result.prefix, fixture.prefix.display().to_string());
    assert!(result.started_at > 1_600_000_000_000);
    assert!(Path::new(&result.log).is_file());
    assert!(result.log.ends_with("nte-app-launch.log"));

    let dump = fixture.dump();
    assert_eq!(
        env_value(&dump, "WINEPREFIX"),
        Some(fixture.prefix.display().to_string().as_str())
    );
    assert_eq!(
        env_value(&dump, "PROTONPATH"),
        Some(fixture.proton.display().to_string().as_str())
    );
    assert_eq!(env_value(&dump, "GAMEID"), Some("0"));
    assert_eq!(env_value(&dump, "LauncherStarted"), Some("1"));
    assert_eq!(env_value(&dump, "pgp_scale_value"), Some("1"));
    assert_eq!(env_value(&dump, "UMU_LOG"), Some("info"));
    assert_eq!(env_value(&dump, "PROTON_LOG"), Some("0"));
    assert_eq!(env_value(&dump, "PROTON_VERB"), Some("run"));
    assert_eq!(env_value(&dump, "PROTON_ENABLE_WAYLAND"), Some("0"));
    assert_eq!(env_value(&dump, "PROTON_USE_WAYLAND"), Some("0"));
    assert_eq!(
        dump.lines()
            .find_map(|line| line.strip_prefix("cwd: "))
            .expect("umu cwd"),
        fixture.install.display().to_string()
    );
    let args = dump
        .lines()
        .find_map(|line| line.strip_prefix("args: "))
        .expect("umu args");
    assert!(args.contains("NTEGame.exe"), "{args}");
    assert!(args.ends_with("/launcher /directly"), "{args}");

    // 官方启动器被改成"随游戏退出"的薄壳.
    let presence = std::fs::read_to_string(
        fixture
            .dir
            .join("install/NTELauncher/UserData/Config/Config.ini"),
    )
    .expect("read presence config");
    assert!(presence.contains("user_t92=deadbeef"), "{presence}");
    assert!(presence.contains("autoUpdate=1"), "{presence}");
    assert!(presence.contains("quitWithGame=1"), "{presence}");
    assert!(presence.contains("NeedUpdate=0"), "{presence}");
    let backup = fixture
        .launcher
        .join("UserData/Config/Config.ini.nte-presence.bak");
    assert!(backup.is_file());
    assert!(is_private(&backup));

    // Wine 托盘图标被隐藏.
    let wine = std::fs::read_to_string(&fixture.wine_log).expect("read wine log");
    assert!(wine.contains("NoTrayItemsDisplay"), "{wine}");

    fixture.stop(&host);
}

/// 代理变量是进程级全局状态: 在测试进程里设置会波及同二进制并发的 HTTP 用例.
/// 父用例只把变量注入运行本夹具的子进程, 不污染测试进程.
const PROXY_KEYS: [&str; 5] = [
    "HTTP_PROXY",
    "https_proxy",
    "aLl_PrOxY",
    "NO_PROXY",
    "proxy",
];
const PROXY_URL: &str = "http://proxy.invalid:8080";
const PROXY_FIXTURE: &str = "launch::tests::launch_proxy_environment_fixture";

#[test]
fn launch_keeps_launcher_proxy_settings_out_of_the_game() {
    let executable = std::env::current_exe().expect("test executable");
    let mut command = std::process::Command::new(executable);
    command
        .args([
            "--exact",
            PROXY_FIXTURE,
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .envs(PROXY_KEYS.map(|key| (key, PROXY_URL)));
    let output = command
        .output()
        .expect("run proxy fixture in a child process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "proxy fixture failed: {}\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    // 过滤条件写错时子进程会跑 0 个用例并正常退出.
    assert!(
        stdout.contains("1 passed"),
        "proxy fixture did not run:\n{stdout}"
    );
}

#[test]
#[ignore = "由 launch_keeps_launcher_proxy_settings_out_of_the_game 在注入代理环境的子进程中运行"]
fn launch_proxy_environment_fixture() {
    let fixture = LaunchFixture::new("launch-proxy");
    let (_, host) = fixture.run(LaunchMode::Official).expect("launch");
    let dump = fixture.dump();
    for key in PROXY_KEYS {
        assert_eq!(env_value(&dump, key), None);
        assert_eq!(
            std::env::var(key).ok().as_deref(),
            Some(PROXY_URL),
            "{key} 应由父用例注入子进程"
        );
    }
    fixture.stop(&host);
}

#[test]
fn cancellation_stops_the_launch_before_spawning() {
    let fixture = LaunchFixture::new("launch-cancel");
    let host = TestHost::new(fixture.plan(LaunchMode::Background));
    host.cancel();
    let ticket = SessionTicket::new("1", Arc::clone(&host) as Arc<dyn SessionHost>);
    let error = crate::launch::provider()
        .launch(ticket)
        .expect_err("cancelled launch");
    assert_eq!(error.message, "启动已暂停。");
    assert!(!fixture.dump.exists());
}

#[test]
fn background_launch_configures_the_native_session() {
    let mut fixture = LaunchFixture::new("launch-background");
    fixture.env.set("WAYLAND_DISPLAY", "wayland-test");
    fixture.dir.write(
        "install/NTELauncher/UserData/Config/Config.ini",
        "[Setting]\nautoLogin=1\n",
    );

    let (result, host) = fixture.run(LaunchMode::Background).expect("launch");
    assert_eq!(result.mode, "background");
    assert!(result.exe.ends_with("NTEGame.exe"));
    let dump = fixture.dump();
    assert_eq!(env_value(&dump, "PROTON_ENABLE_WAYLAND"), Some("1"));
    assert_eq!(env_value(&dump, "PROTON_USE_WAYLAND"), Some("1"));
    assert_eq!(
        env_value(&dump, "NTE_BRIDGE_EXE"),
        Some(oddity_platform::paths::windows_path(&fixture.launcher.join("NTEGame.exe")).as_str())
    );
    assert_eq!(
        env_value(&dump, "NTE_BRIDGE_CWD"),
        Some(oddity_platform::paths::windows_path(&fixture.launcher).as_str())
    );
    assert_eq!(
        env_value(&dump, "NTE_BRIDGE_ARGS"),
        Some("/launcher /directly")
    );
    assert!(env_value(&dump, "NTE_HELPER_LOG")
        .expect("helper log path")
        .ends_with("helper-window.log"));
    assert_eq!(env_value(&dump, "PROTON_VERB"), Some("run"));
    assert_eq!(
        env_value(&dump, "NTE_SESSION_CLICK"),
        Some("1280,720,1120,636")
    );
    assert!(env_value(&dump, "NTE_SESSION_HELPER")
        .unwrap()
        .ends_with("nte-launch-helper.exe"));
    for key in [
        "NTE_SESSION_OUTPUT",
        "NTE_SESSION_SIGNAL",
        "NTE_SESSION_CANCEL",
    ] {
        assert!(env_value(&dump, key).unwrap().starts_with("Z:"), "{key}");
    }
    // 会话控制器驱动官方正常启动流程.
    assert!(env_value(&dump, "NTE_GAME_EXE").is_none());
    let args = dump
        .lines()
        .find_map(|line| line.strip_prefix("args: "))
        .expect("umu args");
    assert!(args.contains("nte-session-helper.exe"), "{args}");

    fixture.stop(&host);
}

#[test]
fn background_launch_reports_a_missing_official_launcher() {
    let fixture = LaunchFixture::new("launch-background-missing");
    std::fs::remove_file(fixture.launcher.join("NTEGame.exe")).unwrap();
    let error = expect_launch_error(fixture.run(LaunchMode::Background));
    assert!(error.contains("官方启动器不存在"), "{error}");
}

#[test]
fn unwritable_runtime_config_fails_before_spawning() {
    let fixture = LaunchFixture::new("launch-readonly-config");
    let bin = fixture
        .dir
        .join("install/Client/WindowsNoEditor/HT/Binaries/Win64");
    let config = bin.join("GameLauncher.config");
    std::fs::write(&config, "E:/WindowsLauncher/WPGameClientSDK_64.dll").expect("Windows SDK path");
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o444))
        .expect("readonly config");

    let error = expect_launch_error(fixture.run(LaunchMode::Background));
    assert!(error.contains("游戏运行配置不可写"), "{error}");
    assert!(error.contains("GameLauncher.config"), "{error}");
    assert!(!fixture.dump.exists());
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644))
        .expect("restore config permissions");

    let cache = bin.join("patchersdk_serverlist");
    std::fs::create_dir(&cache).expect("server cache");
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o555))
        .expect("readonly server cache");
    let error = expect_launch_error(fixture.run(LaunchMode::Background));
    assert!(error.contains("patchersdk_serverlist"), "{error}");
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755))
        .expect("restore server cache permissions");

    NTE_LAUNCH
        .check_runtime_config_writable(&bin.join("HTGame.exe"))
        .expect("writable runtime config");
}

#[test]
fn direct_launch_requires_the_bundled_sdk_components() {
    let fixture = LaunchFixture::new("launch-direct");
    let error = expect_launch_error(fixture.run(LaunchMode::Direct));
    assert!(error.contains("缺少直接启动所需的官方SDK组件"), "{error}");
    assert!(error.contains("SDK组件"), "{error}");
    assert!(!fixture.dump.exists());
}

#[test]
fn skip_presence_tuning_leaves_the_official_config_alone() {
    let mut fixture = LaunchFixture::new("launch-no-presence");
    let config = fixture.launcher.join("UserData/Config/Config.ini");
    std::fs::create_dir_all(config.parent().expect("config parent")).expect("create config dir");
    std::fs::write(&config, "[Setting]\nautoLogin=1\n").expect("write config");
    fixture.env.set("NTE_DISABLE_PRESENCE_TUNING", "1");

    let (_, host) = fixture.run(LaunchMode::Background).expect("launch");
    assert_eq!(
        std::fs::read_to_string(&config).expect("read config"),
        "[Setting]\nautoLogin=1\n"
    );
    assert!(!fixture.wine_log.exists());
    fixture.stop(&host);
}

#[test]
fn presence_tuning_failure_is_logged_but_does_not_block_the_launch() {
    let fixture = LaunchFixture::new("launch-presence-readonly");
    let config_dir = fixture.launcher.join("UserData/Config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("Config.ini"), "x=1\n").unwrap();
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (result, host) = fixture.run(LaunchMode::Official).expect("launch continues");
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let text = std::fs::read_to_string(&result.log).unwrap();
    assert!(text.contains("presence settings not applied"), "{text}");
    fixture.stop(&host);
}

#[test]
fn immediate_exit_reports_the_log_tail() {
    let fixture = LaunchFixture::new("launch-exit");
    // 假的包装程序立刻失败: 应当把日志尾部带回错误信息.
    let failing = |mode| {
        let mut plan = fixture.plan(mode);
        plan.runner = RunnerSpec {
            program: PathBuf::from("/bin/sh"),
            prefix_args: vec![
                "-c".into(),
                "echo 'wine: cannot find L\"C:\\\\windows\"' >&2; exit 1".into(),
            ],
        };
        fixture.run_plan(plan)
    };

    let error = expect_launch_error(failing(LaunchMode::Official));
    assert!(error.contains("官方启动器启动失败（退出码 1）"), "{error}");
    assert!(error.contains("cannot find"), "{error}");

    let (_, host) = failing(LaunchMode::Background).expect("background monitor owns cleanup");
    let end = host
        .wait_end(Duration::from_secs(10))
        .expect("background session ends");
    assert!(wait_until(
        || host.calls().iter().any(|call| matches!(call, Call::End(_))),
        Duration::from_secs(1)
    ));
    let failure = end.failure.expect("早退错误应带到终态");
    assert_eq!(failure.kind, LaunchFailureKind::EarlyExit);
    let detail = failure.detail.expect("failure detail");
    assert!(detail.contains("后台包装进程"), "{detail}");
}

#[test]
fn global_handoff_uses_the_global_client() {
    for mode in [LaunchMode::Official, LaunchMode::Background] {
        let fixture = LaunchFixture::new("launch-global");
        std::fs::rename(
            fixture.launcher.join("NTEGame.exe"),
            fixture.launcher.join("NTEGlobalGame.exe"),
        )
        .unwrap();
        let (result, host) = fixture
            .run_plan(fixture.plan_for(mode, "global"))
            .expect("global handoff");
        assert!(result.exe.ends_with("NTEGlobalGame.exe"));
        let dump = fixture.dump();
        if mode == LaunchMode::Background {
            assert_eq!(
                env_value(&dump, "NTE_BRIDGE_EXE"),
                Some(
                    oddity_platform::paths::windows_path(
                        &fixture.launcher.join("NTEGlobalGame.exe")
                    )
                    .as_str()
                )
            );
            // 国际服沿用同一套点击坐标, 仅验证交接参数.
            assert_eq!(
                env_value(&dump, "NTE_SESSION_CLICK"),
                Some("1280,720,1120,636")
            );
        } else {
            assert!(
                dump.contains("NTEGlobalGame.exe /launcher /directly"),
                "{dump}"
            );
        }
        fixture.stop(&host);
    }
}

/// 真机后台启动: 需要显式提供安装目录, prefix 与 Proton.
#[test]
#[ignore = "需要国服安装、Proton 和图形环境; 读取 ODDITY_LIVE_CN_ROOT/ODDITY_LIVE_PREFIX/ODDITY_LIVE_PROTON"]
fn live_cn_background_launch() {
    live_background_launch("cn", "ODDITY_LIVE_CN_ROOT");
}

#[test]
#[ignore = "需要国际服安装、Proton 和图形环境; 读取 ODDITY_LIVE_GLOBAL_ROOT/ODDITY_LIVE_PREFIX/ODDITY_LIVE_PROTON"]
fn live_global_background_launch() {
    live_background_launch("global", "ODDITY_LIVE_GLOBAL_ROOT");
}

fn live_background_launch(distribution: &str, root_key: &str) {
    let read_path = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("missing {key}"));
    let region = Region::parse(distribution).expect("live region");
    let install_root = PathBuf::from(read_path(root_key));
    let prefix = PathBuf::from(read_path("ODDITY_LIVE_PREFIX"));
    let proton = PathBuf::from(read_path("ODDITY_LIVE_PROTON"));
    let umu = oddity_platform::process::find_executable("umu-run")
        .unwrap_or_else(|| PathBuf::from("umu-run"));
    let scratch = TempDir::new("live-background");
    let log_dir = scratch.join("logs");
    std::fs::create_dir_all(&log_dir).expect("create log dir");
    let plan = LaunchPlan {
        target: SessionTarget {
            game: "nte".into(),
            distribution: distribution.into(),
            mode: LaunchMode::Background,
        },
        install_root: install_root.clone(),
        launcher_root: install_root.join(region.profile().launcher_dir),
        game_exe: Some(install_root.join(LAYOUT.game_exe)),
        game_version: String::new(),
        launcher_version: String::new(),
        prefix,
        proton: ProtonRef {
            root: proton,
            name: "live".into(),
            identity: String::new(),
        },
        umu: umu.clone(),
        log_dir,
        helper_dir: scratch.join("helpers"),
        runner: RunnerSpec {
            program: umu,
            prefix_args: Vec::new(),
        },
        gamescope: false,
        account_id: None,
    };
    let host = TestHost::new(plan);
    let ticket = SessionTicket::new("live", Arc::clone(&host) as Arc<dyn SessionHost>);
    let result = crate::launch::provider()
        .launch(ticket)
        .expect("live background launch");
    assert_eq!(result.mode, "background");
    let started = wait_until(
        || {
            host.calls()
                .iter()
                .any(|call| matches!(call, Call::Sample(sample) if sample.game_running))
        },
        Duration::from_secs(150),
    );
    let control = host.control().expect("bound control");
    let close = control.close();
    let end = host.wait_end(Duration::from_secs(60));
    assert!(close.is_ok(), "close: {close:?}");
    assert!(started, "official game did not start");
    assert!(
        end.as_ref().is_some_and(|end| end.game_started),
        "end: {end:?}"
    );
}

#[test]
fn launch_check_only_gates_direct_starts_to_the_cn_region() {
    let provider = crate::launch::provider();
    let target = |distribution: &str, mode: LaunchMode| SessionTarget {
        game: "nte".into(),
        distribution: distribution.into(),
        mode,
    };

    let error = provider
        .check(&target("global", LaunchMode::Direct))
        .expect_err("国际服不支持直启");
    assert!(error.message.contains("仅支持《异环》国服"), "{error}");
    assert_eq!(error.code, LaunchFailureKind::Backend);

    assert_eq!(provider.check(&target("cn", LaunchMode::Direct)), Ok(()));
    for mode in [LaunchMode::Official, LaunchMode::Background] {
        assert_eq!(provider.check(&target("global", mode)), Ok(()));
    }
}
