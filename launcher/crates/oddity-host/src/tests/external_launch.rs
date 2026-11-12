//! 外部启动入口与启动前状态检查.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::channel::ChannelId;
use crate::environment::{ProtonPin, RuntimeEnvironment};
use crate::session::{prepare_launch, Registry, Session};
use crate::settings::Settings;
use crate::{cli, AppState};

fn managed_state(settings: Settings, game: Registry) -> AppState {
    AppState {
        settings: Mutex::new(settings),
        game: Mutex::new(game),
        packages: crate::testing::packages(),
    }
}

fn session(closing: bool, alive: bool) -> Session {
    Session {
        performance: std::sync::Arc::new(std::sync::Mutex::new(None)),
        id: 1,
        pid: 0,
        pgid: 0,
        exe: String::new(),
        started_at: 0,
        log: String::new(),
        mode: "background".into(),
        distribution: ChannelId::new("cn"),
        game: crate::identity::GameId::from("nte"),
        closing,
        exit_notified: false,
        game_alive: Arc::new(AtomicBool::new(alive)),
        monitor_done: Arc::new(AtomicBool::new(false)),
        finalized: Arc::new(AtomicBool::new(false)),
        _install_lock: None,
        _prefix_lock: None,
        game_mode: None,
        snapshot: None,
        snapshot_success_after_ms: 0,
        launcher_root: None,
        monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    }
}

fn global_environment() -> RuntimeEnvironment {
    RuntimeEnvironment {
        id: "0".repeat(32),
        name: "Global Env".into(),
        proton: ProtonPin {
            path: PathBuf::from("/tmp/proton"),
            identity: "identity".into(),
        },
        install_root: PathBuf::from("/tmp/install"),
        game: crate::identity::GameId::from("nte"),
        distribution: ChannelId::new("global"),
        created_at: 0,
    }
}

#[test]
fn prepare_launch_rejects_busy_downloads() {
    let _lock = crate::testing::serial();
    crate::testing::reset();
    crate::testing::set_package_busy(true, true);
    let mut tracker = Registry::default();
    assert_eq!(
        prepare_launch(&crate::testing::packages(), &mut tracker).unwrap_err(),
        "游戏资源任务正在进行，暂时无法启动游戏"
    );
    crate::testing::reset();
}

#[test]
fn prepare_launch_rejects_environment_tasks_and_starting_sessions() {
    let _download_state = crate::testing::serial();
    crate::testing::reset();
    let mut environment = Registry {
        environment_busy: true,
        ..Default::default()
    };
    assert_eq!(
        prepare_launch(&crate::testing::packages(), &mut environment).unwrap_err(),
        "运行环境任务正在进行, 请完成后再启动游戏"
    );

    let mut starting = Registry {
        starting: true,
        ..Default::default()
    };
    assert_eq!(
        prepare_launch(&crate::testing::packages(), &mut starting).unwrap_err(),
        "游戏正在启动，请稍候。"
    );
}

#[test]
fn prepare_launch_rejects_closing_and_running_sessions() {
    let _download_state = crate::testing::serial();
    crate::testing::reset();
    let mut closing = Registry {
        session: Some(session(true, false)),
        ..Default::default()
    };
    assert_eq!(
        prepare_launch(&crate::testing::packages(), &mut closing).unwrap_err(),
        "游戏正在关闭，请稍候。"
    );

    let mut running = Registry {
        session: Some(session(false, true)),
        ..Default::default()
    };
    assert_eq!(
        prepare_launch(&crate::testing::packages(), &mut running).unwrap_err(),
        "游戏已在运行。"
    );
}

#[test]
fn prepare_launch_clears_finished_sessions() {
    let _download_state = crate::testing::serial();
    crate::testing::reset();
    let mut tracker = Registry {
        session: Some(session(false, false)),
        last_error: Some("旧错误".into()),
        ..Default::default()
    };
    prepare_launch(&crate::testing::packages(), &mut tracker).unwrap();
    assert!(tracker.starting);
    assert!(tracker.session.is_none());
    assert!(tracker.last_error.is_none());
}

#[test]
fn launch_summary_uses_region_environment_or_current_distribution() {
    let mut app = crate::service::test_service();
    let mut settings = crate::tests::settings_with_game(ChannelId::new("cn"), None, None);
    settings.runtime_environments = vec![global_environment()];
    std::sync::Arc::get_mut(&mut app).unwrap().state = managed_state(settings, Registry::default());

    let region = cli::LaunchRequest {
        game: crate::identity::GameId::from("nte"),
        region: Some(ChannelId::new("global")),
        mode: cli::LaunchMode::Background,
        environment: None,
    };
    assert_eq!(
        crate::external_launch::launch_failure_summary(&app, &region),
        "国际服启动失败"
    );

    let environment = cli::LaunchRequest {
        game: crate::identity::GameId::from("nte"),
        region: None,
        mode: cli::LaunchMode::Background,
        environment: Some("Global Env".into()),
    };
    assert_eq!(
        crate::external_launch::launch_failure_summary(&app, &environment),
        "国际服启动失败"
    );

    let current = cli::LaunchRequest {
        game: crate::identity::GameId::from("nte"),
        region: None,
        mode: cli::LaunchMode::Background,
        environment: None,
    };
    assert_eq!(
        crate::external_launch::launch_failure_summary(&app, &current),
        "国服启动失败"
    );

    let unknown = cli::LaunchRequest {
        game: crate::identity::GameId::from("nte"),
        region: None,
        mode: cli::LaunchMode::Background,
        environment: Some("missing".into()),
    };
    assert_eq!(
        crate::external_launch::launch_failure_summary(&app, &unknown),
        "国服启动失败"
    );

    let other_game = cli::LaunchRequest {
        game: crate::identity::GameId::from("hsr"),
        region: None,
        mode: cli::LaunchMode::Background,
        environment: None,
    };
    assert_eq!(
        crate::external_launch::launch_failure_summary(&app, &other_game),
        "hsr启动失败"
    );
}

#[test]
fn external_launch_rejects_games_without_a_launch_profile() {
    let app = crate::service::test_service();
    let request = cli::LaunchRequest {
        game: crate::identity::GameId::from("hsr"),
        region: None,
        mode: cli::LaunchMode::Background,
        environment: None,
    };
    let error = crate::service::block_on(crate::external_launch::start_external_launch(
        &app, &request,
    ))
    .unwrap_err();
    assert_eq!(error, "《hsr》尚未支持启动.");
}

#[test]
fn take_launch_error_returns_and_clears_the_pending_error() {
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = managed_state(
        Settings::default(),
        Registry {
            last_error: Some("启动失败".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        crate::session::take_launch_error(&app.state),
        Some("启动失败".into())
    );
    assert_eq!(crate::session::take_launch_error(&app.state), None);
}

#[test]
fn launch_settings_clones_the_current_settings() {
    let mut app = crate::service::test_service();
    let mut settings = crate::tests::settings_with_game(ChannelId::new("global"), None, None);
    settings.current_mut().launch_mode = "direct".into();
    std::sync::Arc::get_mut(&mut app).unwrap().state = managed_state(settings, Registry::default());

    let settings = crate::session::launch_settings(&app.state).unwrap();
    assert_eq!(
        settings.current().distribution.as_ref(),
        Some(&ChannelId::new("global"))
    );
    assert_eq!(settings.current().launch_mode, "direct");
}

#[test]
fn launch_mode_names_are_stable() {
    assert_eq!(cli::LaunchMode::Official.as_str(), "official");
    assert_eq!(cli::LaunchMode::Background.as_str(), "background");
    assert_eq!(cli::LaunchMode::Direct.as_str(), "direct");
}
