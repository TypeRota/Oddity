use super::EnvGuard;
use crate::channel::ChannelId;
use crate::now_millis;
use crate::session;
use crate::settings::Settings;
use crate::AppState;
use oddity_runtime::{LaunchProgress, LaunchStage};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

mod playtime_export {
    use crate::playtime_commands::playtime_export_poster;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::fs;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\noddity";

    fn export(path: &std::path::Path, data: String) -> Result<String, String> {
        crate::service::block_on(playtime_export_poster(path.display().to_string(), data))
    }

    #[test]
    fn replaces_existing_poster_and_leaves_no_temporary_files() {
        let dir = super::super::TempDir::new("playtime-export-replace");
        let path = dir.write("poster.png", b"old image");
        assert_eq!(
            export(&path, STANDARD.encode(PNG)).unwrap(),
            path.display().to_string()
        );
        assert_eq!(fs::read(path).unwrap(), PNG);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn normalizes_extensions_and_preserves_uppercase_png() {
        let dir = super::super::TempDir::new("playtime-export-extensions");
        for (requested, expected) in [
            ("poster", "poster.png"),
            ("poster.jpg", "poster.png"),
            ("poster.PNG", "poster.PNG"),
        ] {
            let path = dir.join(expected);
            assert_eq!(
                export(&dir.join(requested), STANDARD.encode(PNG)).unwrap(),
                path.display().to_string()
            );
            assert_eq!(fs::read(path).unwrap(), PNG);
        }
    }

    #[test]
    fn invalid_data_cannot_overwrite_an_existing_poster() {
        let dir = super::super::TempDir::new("playtime-export-invalid");
        let path = dir.write("poster.png", PNG);
        for data in [
            String::new(),
            "invalid base64!".into(),
            STANDARD.encode(b"not png"),
            STANDARD.encode(&PNG[..7]),
        ] {
            assert!(export(&path, data).is_err());
            assert_eq!(fs::read(&path).unwrap(), PNG);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn rejects_directory_created_by_extension_normalization() {
        let dir = super::super::TempDir::new("playtime-export-normalized-dir");
        let occupied = dir.join("poster.png");
        fs::create_dir(&occupied).unwrap();
        let error = export(&dir.join("poster.jpg"), STANDARD.encode(PNG)).unwrap_err();
        assert!(error.contains("文件夹"));
        assert!(occupied.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_a_file_as_the_parent_directory() {
        let dir = super::super::TempDir::new("playtime-export-parent-file");
        let parent = dir.write("parent", b"keep");
        let error = export(&parent.join("poster.png"), STANDARD.encode(PNG)).unwrap_err();
        assert!(error.contains("文件夹不存在"));
        assert_eq!(fs::read(parent).unwrap(), b"keep");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn enforces_the_decoded_size_limit() {
        let dir = super::super::TempDir::new("playtime-export-size");
        let path = dir.join("poster.png");
        let limit = 20 * 1024 * 1024;
        let mut bytes = vec![0; limit];
        bytes[..8].copy_from_slice(&PNG[..8]);
        export(&path, STANDARD.encode(&bytes)).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), limit as u64);
        bytes.push(0);
        let error = export(&path, STANDARD.encode(&bytes)).unwrap_err();
        assert!(error.contains("大小上限"));
        assert_eq!(fs::metadata(path).unwrap().len(), limit as u64);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn game_mode_status_reports_capabilities_and_releases_the_finished_session() {
    use crate::game_mode::{Effect, Options, Session};

    let mut env = EnvGuard::new();
    let dir = super::TempDir::new("game-mode-status");
    env.set(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/nonexistent/oddity-test-bus",
    );
    env.set("PATH", dir.path());
    super::write_script(dir.path(), "mangohud", "#!/bin/sh\nexit 0\n");
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry::default()),
        packages: crate::testing::packages(),
    };
    let idle = crate::service::block_on(crate::session::game_mode_status(&app.state)).unwrap();
    assert!(idle.session.is_none());
    assert_eq!(idle.capabilities.len(), 5);
    assert!(!idle.capabilities[0].available);
    assert!(idle.capabilities[3].available);

    let mut command = Command::new("true");
    let mode = Arc::new(Mutex::new(Session::start(
        &Options {
            overlay: true,
            ..Default::default()
        },
        &mut command,
    )));
    assert!(command
        .get_envs()
        .any(|(key, value)| key == "MANGOHUD" && value == Some(std::ffi::OsStr::new("1"))));
    let session = session::Session {
        performance: std::sync::Arc::new(std::sync::Mutex::new(None)),
        id: 1,
        pid: 0,
        pgid: 0,
        exe: "game.exe".into(),
        started_at: 0,
        log: String::new(),
        mode: "background".into(),
        distribution: ChannelId::new("cn"),
        game: crate::identity::GameId::from("nte"),
        closing: false,
        exit_notified: false,
        game_alive: Arc::new(AtomicBool::new(false)),
        monitor_done: Arc::new(AtomicBool::new(false)),
        finalized: Arc::new(AtomicBool::new(false)),
        _install_lock: None,
        _prefix_lock: None,
        game_mode: Some(mode.clone()),
        snapshot: None,
        snapshot_success_after_ms: crate::snapshots::SUCCESS_AFTER_MS,
        launcher_root: None,
        monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    };
    app.state.game.lock().unwrap().session = Some(session.clone());
    let running = crate::service::block_on(crate::session::game_mode_status(&app.state)).unwrap();
    assert_eq!(running.session.unwrap()[3].state, Effect::Requested);
    mode.lock().unwrap().release();
    let finished = crate::service::block_on(crate::session::game_mode_status(&app.state)).unwrap();
    assert_eq!(finished.session.unwrap()[3].state, Effect::Finished);
}

#[test]
fn now_millis_is_a_wall_clock_timestamp() {
    assert!(now_millis() > 1_600_000_000_000);
}

#[test]
fn game_status_reflects_the_session() {
    let idle = crate::session::idle_status();
    assert!(!idle.running);
    assert!(!idle.closing);
    assert!(idle.pid.is_none());
    assert!(idle.started_at.is_none());
    assert!(idle.exe.is_none());
    assert!(idle.log.is_none());

    assert!(!crate::session::idle_status().running);

    let session = session::Session {
        performance: std::sync::Arc::new(std::sync::Mutex::new(None)),
        id: 7,
        pid: 1234,
        pgid: 1234,
        exe: "/game/HTGame.exe".to_string(),
        started_at: 99,
        log: "/tmp/launch.log".to_string(),
        mode: "official".into(),
        distribution: ChannelId::new("global"),
        game: crate::identity::GameId::from("nte"),
        closing: true,
        exit_notified: false,
        game_alive: Arc::new(AtomicBool::new(false)),
        monitor_done: Arc::new(AtomicBool::new(false)),
        finalized: Arc::new(AtomicBool::new(false)),
        _install_lock: None,
        _prefix_lock: None,
        game_mode: None,
        snapshot: None,
        snapshot_success_after_ms: crate::snapshots::SUCCESS_AFTER_MS,
        launcher_root: None,
        monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    };
    let status = session.status();
    assert!(status.running);
    assert!(status.closing);
    assert_eq!(status.pid, Some(1234));
    assert_eq!(status.started_at, Some(99));
    assert_eq!(status.exe.as_deref(), Some("/game/HTGame.exe"));
    assert_eq!(status.log.as_deref(), Some("/tmp/launch.log"));
}

fn test_game_session(closing: bool) -> session::Session {
    session::Session {
        performance: std::sync::Arc::new(std::sync::Mutex::new(None)),
        id: 11,
        pid: 4321,
        pgid: 4321,
        exe: "/game/HTGame.exe".to_string(),
        started_at: 99,
        log: "/tmp/launch.log".to_string(),
        mode: "background".into(),
        distribution: ChannelId::new("cn"),
        game: crate::identity::GameId::from("nte"),
        closing,
        exit_notified: false,
        game_alive: Arc::new(AtomicBool::new(false)),
        monitor_done: Arc::new(AtomicBool::new(false)),
        finalized: Arc::new(AtomicBool::new(false)),
        _install_lock: None,
        _prefix_lock: None,
        game_mode: None,
        snapshot: None,
        snapshot_success_after_ms: crate::snapshots::SUCCESS_AFTER_MS,
        launcher_root: None,
        monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    }
}

#[test]
fn game_status_exposes_launch_progress() {
    assert!(crate::session::idle_status().launch.is_none());

    let tracker = session::Registry {
        launch: Some(LaunchProgress::new(123)),
        ..Default::default()
    };
    let status = tracker.status();
    let launch = status.launch.as_ref().expect("launch progress");
    assert_eq!(launch.stage, LaunchStage::Preparing);
    assert!(!launch.cancelling);
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["launch"]["stage"], "preparing");
    assert_eq!(json["launch"]["startedAt"], 123);
    assert!(serde_json::to_value(crate::session::idle_status())
        .unwrap()
        .get("launch")
        .is_none());
}

#[test]
fn launch_cancel_action_prefers_the_live_session() {
    let mut tracker = session::Registry {
        starting: true,
        launch: Some(LaunchProgress::new(9)),
        launch_cancel: Some(Arc::new(AtomicBool::new(false))),
        ..Default::default()
    };
    let token = Arc::clone(tracker.launch_cancel.as_ref().unwrap());
    assert_eq!(
        crate::session::launch_cancel_action(&mut tracker),
        crate::session::LaunchCancelAction::Wait
    );
    assert!(token.load(Ordering::Acquire));
    assert!(tracker.launch.as_ref().unwrap().cancelling);

    let mut idle = session::Registry::default();
    assert_eq!(
        crate::session::launch_cancel_action(&mut idle),
        crate::session::LaunchCancelAction::Idle
    );
    idle.starting = true;
    assert_eq!(
        crate::session::launch_cancel_action(&mut idle),
        crate::session::LaunchCancelAction::Idle
    );

    let mut closing = session::Registry {
        session: Some(test_game_session(false)),
        launch: Some(LaunchProgress::new(9)),
        ..Default::default()
    };
    assert_eq!(
        crate::session::launch_cancel_action(&mut closing),
        crate::session::LaunchCancelAction::Close
    );
    assert!(closing.launch.as_ref().unwrap().cancelling);
    closing.session.as_mut().unwrap().closing = true;
    assert_eq!(
        crate::session::launch_cancel_action(&mut closing),
        crate::session::LaunchCancelAction::AlreadyClosing
    );
}

#[test]
fn cancel_launch_wait_marks_progress_and_returns_status() {
    let mut app = crate::service::test_service();
    let cancel = Arc::new(AtomicBool::new(false));
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            starting: true,
            launch: Some(LaunchProgress::new(5)),
            launch_cancel: Some(Arc::clone(&cancel)),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let status =
        crate::service::block_on(crate::session::cancel_launch(app.clone(), &app.state)).unwrap();
    assert!(cancel.load(Ordering::Acquire));
    assert!(!status.running);
    let launch = status.launch.expect("launch progress");
    assert_eq!(launch.stage, LaunchStage::Preparing);
    assert!(launch.cancelling);
}

#[test]
fn cancel_launch_idle_clears_stale_progress() {
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            launch: Some(LaunchProgress::new(5)),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let status =
        crate::service::block_on(crate::session::cancel_launch(app.clone(), &app.state)).unwrap();
    assert!(!status.running);
    assert!(status.launch.is_none());
    assert!(app.state.game.lock().unwrap().launch.is_none());
}

#[test]
fn cancel_launch_already_closing_returns_live_status() {
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            session: Some(test_game_session(true)),
            launch: Some(LaunchProgress::new(5)),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let status =
        crate::service::block_on(crate::session::cancel_launch(app.clone(), &app.state)).unwrap();
    assert!(status.running);
    assert!(status.closing);
    assert!(status.launch.is_some());
}

#[test]
fn cancel_launch_dead_session_returns_idle() {
    let mut session = test_game_session(false);
    session.pgid = 1_999_999_999;
    session.pid = 1_999_999_999;
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            session: Some(session),
            launch: Some(LaunchProgress::new(5)),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let status =
        crate::service::block_on(crate::session::cancel_launch(app.clone(), &app.state)).unwrap();
    assert!(!status.running);
    assert!(status.launch.is_none());
}

#[test]
fn close_game_rejects_starting_and_closing_sessions() {
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            starting: true,
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let starting =
        crate::service::block_on(crate::session::close_game(app.clone(), &app.state)).unwrap_err();
    assert!(starting.contains("正在启动"));

    app.state.game.lock().unwrap().starting = false;
    app.state.game.lock().unwrap().session = Some(test_game_session(true));
    let closing =
        crate::service::block_on(crate::session::close_game(app.clone(), &app.state)).unwrap_err();
    assert!(closing.contains("正在关闭"));
}

#[test]
fn close_game_dead_session_returns_idle_and_clears_progress() {
    let mut session = test_game_session(false);
    session.pgid = 1_999_999_999;
    session.pid = 1_999_999_999;
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            session: Some(session),
            launch: Some(LaunchProgress::new(5)),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let status =
        crate::service::block_on(crate::session::close_game(app.clone(), &app.state)).unwrap();
    assert!(!status.running);
    assert!(app.state.game.lock().unwrap().launch.is_none());
}

#[test]
fn close_game_without_session_returns_idle() {
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry::default()),
        packages: crate::testing::packages(),
    };
    let status =
        crate::service::block_on(crate::session::close_game(app.clone(), &app.state)).unwrap();
    assert!(!status.running);
}

#[test]
fn game_status_clears_launch_for_a_dead_session() {
    let mut session = test_game_session(false);
    session.pgid = 1_999_999_999;
    session.pid = 1_999_999_999;
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(session::Registry {
            session: Some(session),
            launch: Some(LaunchProgress::new(5)),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let status = crate::session::game_status(&app.state);
    assert!(!status.running);
    assert!(status.launch.is_none());
}

#[test]
fn begin_launch_creates_progress_only_for_background() {
    let mut background = session::Registry::default();
    let mut background_settings = Settings::default();
    background_settings.current_mut().launch_mode = "background".into();
    let cancel = crate::session::begin_launch(&mut background, &background_settings);
    assert!(background.starting);
    assert!(background.launch.is_some());
    assert!(background.launch_cancel.is_some());
    assert!(!cancel.load(Ordering::Acquire));

    let mut official = session::Registry::default();
    let mut official_settings = Settings::default();
    official_settings.current_mut().launch_mode = "official".into();
    crate::session::begin_launch(&mut official, &official_settings);
    assert!(official.starting);
    assert!(official.launch.is_none());
    assert!(official.launch_cancel.is_none());
}

#[test]
fn reset_launch_clears_all_start_flags() {
    let mut tracker = session::Registry {
        starting: true,
        launch: Some(LaunchProgress::new(1)),
        launch_cancel: Some(Arc::new(AtomicBool::new(false))),
        ..Default::default()
    };
    crate::session::reset_launch(&mut tracker);
    assert!(!tracker.starting);
    assert!(tracker.launch.is_none());
    assert!(tracker.launch_cancel.is_none());
}

#[test]
fn advance_launch_is_scoped_to_the_current_session() {
    let mut tracker = session::Registry {
        session: Some(test_game_session(false)),
        launch: Some(LaunchProgress::new(1)),
        ..Default::default()
    };
    assert!(crate::session::advance_launch(
        &mut tracker,
        11,
        LaunchStage::Helper
    ));
    assert!(!crate::session::advance_launch(
        &mut tracker,
        12,
        LaunchStage::Resources
    ));
    assert!(!crate::session::advance_launch(
        &mut tracker,
        11,
        LaunchStage::Preparing
    ));
    tracker.launch.as_mut().unwrap().cancelling = true;
    assert!(!crate::session::advance_launch(
        &mut tracker,
        11,
        LaunchStage::Resources
    ));
    tracker.session = None;
    assert!(!crate::session::advance_launch(
        &mut tracker,
        11,
        LaunchStage::Resources
    ));
}

#[test]
fn clear_launch_removes_progress_and_token() {
    let mut tracker = session::Registry {
        launch: Some(LaunchProgress::new(1)),
        launch_cancel: Some(Arc::new(AtomicBool::new(false))),
        ..Default::default()
    };
    tracker.clear_launch();
    assert!(tracker.launch.is_none());
    assert!(tracker.launch_cancel.is_none());
}

#[test]
fn finish_game_close_clears_success_or_restores_failure() {
    let mut success = session::Registry {
        session: Some(test_game_session(false)),
        launch: Some(LaunchProgress::new(1)),
        launch_cancel: Some(Arc::new(AtomicBool::new(false))),
        last_error: Some("old".into()),
        ..Default::default()
    };
    crate::session::finish_game_close(&mut success, 11, true);
    assert!(success.session.is_none());
    assert!(success.launch.is_none());
    assert!(success.launch_cancel.is_none());
    assert!(success.last_error.is_none());

    let mut failure = session::Registry {
        session: Some(test_game_session(true)),
        launch: Some(LaunchProgress {
            cancelling: true,
            ..LaunchProgress::new(1)
        }),
        ..Default::default()
    };
    crate::session::finish_game_close(&mut failure, 11, false);
    assert!(!failure.session.as_ref().unwrap().closing);
    assert!(!failure.launch.as_ref().unwrap().cancelling);
    crate::session::finish_game_close(&mut failure, 12, true);
    assert!(failure.session.is_some());
}

fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    condition()
}

#[test]
fn monitor_fills_the_game_version_after_a_successful_snapshot() {
    use oddity_runtime::SessionSample;

    let dir = super::TempDir::new("monitor-snapshot-version");
    let app = crate::service::test_service();
    let inputs = crate::snapshot_commands::current_inputs(
        &crate::testing::packages(),
        &crate::settings::Settings::default(),
    );
    let id = app
        .snapshots
        .begin(crate::snapshots::capture(inputs))
        .expect("记录启动快照");
    dir.write_text(
        "launcher/UserData/Config/Config.ini",
        "[Game]\nVersion_=1.4.20\n",
    );

    let mut game_session = test_game_session(false);
    game_session.snapshot = Some(id.clone());
    game_session.launcher_root = Some(dir.join("launcher"));
    game_session.snapshot_success_after_ms = 0;
    {
        let mut state = game_session.monitor_state.lock().unwrap();
        state.snapshot_running_at = Some(0);
        state.start_retry_at = std::time::Instant::now() + std::time::Duration::from_secs(60);
    }
    crate::session::monitor::apply_sample(
        &app,
        &game_session,
        &SessionSample {
            game_running: true,
            game_pids: Vec::new(),
        },
    );

    let overview = app.snapshots.overview().expect("快照概览");
    let latest = overview.latest.as_ref().expect("最新快照");
    assert_eq!(latest.status, crate::snapshots::Status::Success);
    assert_eq!(latest.id, id);
}

#[test]
fn sampling_starts_and_stops_playtime_for_the_game_process() {
    use crate::playtime_commands::Playtime;
    use crate::session::monitor::MonitorState;
    use oddity_runtime::SessionSample;

    let _env = EnvGuard::new();
    let dir = super::TempDir::new("playtime-process");
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().playtime = Playtime::load(dir.path());
    let mut child = Command::new("bash")
        .arg("-c")
        .arg("exec -a HTGame.exe sleep 30")
        .process_group(0)
        .spawn()
        .expect("spawn game process");
    let pid = child.id();
    let pgid = i32::try_from(pid).expect("pid fits i32");
    let session = crate::session::Session {
        performance: std::sync::Arc::new(std::sync::Mutex::new(None)),
        id: 1,
        pid,
        pgid,
        exe: "HTGame.exe".into(),
        started_at: 0,
        log: String::new(),
        mode: "background".into(),
        distribution: ChannelId::new("cn"),
        game: crate::identity::GameId::from("nte"),
        closing: false,
        exit_notified: false,
        game_alive: Arc::new(AtomicBool::new(true)),
        monitor_done: Arc::new(AtomicBool::new(false)),
        finalized: Arc::new(AtomicBool::new(false)),
        _install_lock: None,
        _prefix_lock: None,
        game_mode: None,
        snapshot: None,
        snapshot_success_after_ms: crate::snapshots::SUCCESS_AFTER_MS,
        launcher_root: None,
        monitor_state: Arc::new(Mutex::new(MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    };
    app.state.game.lock().unwrap().session = Some(session.clone());
    let sample = SessionSample {
        game_running: true,
        game_pids: vec![pid],
    };
    session::monitor::apply_sample(&app, &session, &sample);
    assert!(
        wait_until(
            || app.playtime.history().unwrap().active.is_some(),
            Duration::from_secs(3)
        ),
        "游戏进程出现后应开始计时"
    );

    child.kill().expect("kill game");
    let stopped = SessionSample {
        game_running: false,
        game_pids: Vec::new(),
    };
    session::monitor::apply_sample(&app, &session, &stopped);
    assert!(
        wait_until(
            || app.playtime.history().unwrap().active.is_none(),
            Duration::from_secs(3)
        ),
        "游戏进程退出后应结束计时"
    );
    assert!(!session.game_alive.load(Ordering::Acquire));
    let _ = child.wait();
}

#[test]
fn close_game_falls_back_when_the_session_has_no_control() {
    let app = crate::service::test_service();
    let mut child = Command::new("/bin/sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();
    let pgid = i32::try_from(pid).expect("pid fits i32");
    let mut session = test_game_session(false);
    session.pid = pid;
    session.pgid = pgid;
    session.game_alive = Arc::new(AtomicBool::new(false));
    app.state.game.lock().unwrap().session = Some(session);
    // 监控线程在真实会话里负责回收组长, 这里用等待线程等价代劳.
    let waiter = thread::spawn(move || child.wait());
    let status = crate::service::block_on(crate::session::close_game(app.clone(), &app.state))
        .expect("close game");
    assert!(!status.running);
    let _ = waiter.join();
    assert!(app.state.game.lock().unwrap().session.is_none());
}
