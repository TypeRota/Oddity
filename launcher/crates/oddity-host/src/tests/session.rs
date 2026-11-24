//! 会话注册表的 game-state 事件序列与锁序.

use crate::channel::ChannelId;
use crate::identity::GameId;
use crate::session::{self, Registry, Session};
use crate::settings::Settings;
use crate::testing::{crash_context, crash_site as crash_dir};
use crate::AppState;
use oddity_runtime::{LaunchProgress, LaunchStage, SessionSample};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

fn test_session(id: u64) -> Session {
    Session {
        performance: Arc::new(Mutex::new(None)),
        id,
        pid: 4321,
        pgid: 4321,
        exe: "/game/HTGame.exe".into(),
        started_at: 99,
        log: "/tmp/launch.log".into(),
        mode: "background".into(),
        distribution: ChannelId::new("cn"),
        game: GameId::from("nte"),
        closing: false,
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
        prefix: PathBuf::new(),
        monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    }
}

/// 在 prefix 里建一份崩溃现场; 给出内容时一并写入崩溃上下文.
fn crash_site(prefix: &Path, name: &str, context: Option<&str>) -> PathBuf {
    let site = crash_dir(prefix, "player", name);
    if let Some(context) = context {
        fs::write(site.join("CrashContext.runtime-xml"), context).expect("写入崩溃上下文");
    }
    site
}

/// 现场扫描只看本轮会话的 prefix, 基线在会话起点取样.
fn hang_session(id: u64, prefix: &Path) -> Session {
    let mut session = test_session(id);
    session.prefix = prefix.to_path_buf();
    session.monitor_state = Arc::new(Mutex::new(
        crate::session::monitor::MonitorState::for_prefix(prefix),
    ));
    session
}

fn sample(running: bool) -> SessionSample {
    SessionSample {
        game_running: running,
        game_pids: Vec::new(),
    }
}

/// 注册表里当前的现场摘要.
fn hang_site(app: &crate::service::ServiceHandle) -> Option<String> {
    app.state
        .game
        .lock()
        .unwrap()
        .status()
        .hang
        .map(|hang| hang.site)
}

fn game_state_events(events: &mpsc::Receiver<Value>) -> Vec<Value> {
    let mut payloads = Vec::new();
    while let Ok(message) = events.try_recv() {
        if message["event"] == "game-state" {
            payloads.push(message["payload"].clone());
        }
    }
    payloads
}

/// 一次后台启动的事件序列: 准备 -> 运行 -> 关闭 -> 空闲.
#[test]
fn game_state_event_sequence_is_stable() {
    let (app, events) = crate::service::test_service_with_output();
    {
        let mut registry = app.state.game.lock().unwrap();
        *registry = Registry {
            starting: true,
            launch: Some(LaunchProgress::new(123)),
            ..Default::default()
        };
    }
    session::emit_game_status(&app, &app.state);

    {
        let mut registry = app.state.game.lock().unwrap();
        registry.starting = false;
        registry.session = Some(test_session(1));
        if let Some(launch) = registry.launch.as_mut() {
            assert!(launch.advance(LaunchStage::Helper));
        }
    }
    session::emit_game_status(&app, &app.state);

    app.state
        .game
        .lock()
        .unwrap()
        .session
        .as_mut()
        .unwrap()
        .closing = true;
    session::emit_game_status(&app, &app.state);

    {
        let mut registry = app.state.game.lock().unwrap();
        session::finish_game_close(&mut registry, 1, true);
    }
    session::emit_game_status(&app, &app.state);

    let launch = json!({
        "stage": "preparing",
        "startedAt": 123,
        "cancelling": false,
    });
    let helper = json!({
        "stage": "helper",
        "startedAt": 123,
        "cancelling": false,
    });
    assert_eq!(
        game_state_events(&events),
        vec![
            json!({
                "running": false,
                "closing": false,
                "pid": null,
                "startedAt": null,
                "exe": null,
                "log": null,
                "launch": launch,
            }),
            json!({
                "running": true,
                "closing": false,
                "pid": 4321,
                "startedAt": 99,
                "exe": "/game/HTGame.exe",
                "log": "/tmp/launch.log",
                "launch": helper,
            }),
            json!({
                "running": true,
                "closing": true,
                "pid": 4321,
                "startedAt": 99,
                "exe": "/game/HTGame.exe",
                "log": "/tmp/launch.log",
                "launch": {
                    "stage": "helper",
                    "startedAt": 123,
                    "cancelling": false,
                },
            }),
            json!({
                "running": false,
                "closing": false,
                "pid": null,
                "startedAt": null,
                "exe": null,
                "log": null,
            }),
        ]
    );
}

/// 事件回调里读注册表不能死锁: 发事件前必须已经释放注册表锁.
#[test]
fn game_state_events_are_emitted_without_the_registry_lock() {
    let (app, _events) = crate::service::test_service_with_output();
    let (sender, receiver) = mpsc::channel();
    let reader = app.clone();
    app.subscribe("game-state", move |_| {
        // 回调同步执行在发事件线程上; 这里拿得到锁说明发事件时没有持锁.
        let _ = sender.send(reader.state.game.lock().unwrap().status());
    });
    let emitter = app.clone();
    let worker = thread::spawn(move || session::emit_game_status(&emitter, &emitter.state));
    receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("发送 game-state 时不应持有注册表锁");
    worker.join().expect("事件线程应正常结束");
}

/// 关闭命令在收尾与发事件时不持有注册表锁, 否则管理线程会卡死.
#[test]
fn close_game_emits_without_holding_the_registry_lock() {
    let mut app = crate::service::test_service();
    let mut dead = test_session(7);
    dead.pgid = 1_999_999_999;
    dead.pid = 1_999_999_999;
    std::sync::Arc::get_mut(&mut app).unwrap().state = AppState {
        settings: Mutex::new(Settings::default()),
        game: Mutex::new(Registry {
            session: Some(dead),
            ..Default::default()
        }),
        packages: crate::testing::packages(),
    };
    let (sender, receiver) = mpsc::channel();
    let reader = app.clone();
    app.subscribe("game-state", move |_| {
        let _ = sender.send(reader.state.game.lock().unwrap().status());
    });
    let status = crate::service::block_on(session::close_game(app.clone(), &app.state))
        .expect("dead session closes");
    assert!(!status.running);
    receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("关闭事件不应持有注册表锁");
}

/// 逐轮采样; 测试把现场扫描与补读间隔清零, 不等真实时间.
fn sample_round(app: &crate::service::ServiceHandle, session: &Session, running: bool) {
    {
        let mut state = session.monitor_state.lock().unwrap();
        state.next_hang_scan = Instant::now();
        state.next_hang_read = Instant::now();
    }
    crate::session::monitor::apply_sample(app, session, &sample(running));
}

/// 指定事件的全部载荷.
fn event_payloads(events: &mpsc::Receiver<Value>, name: &str) -> Vec<Value> {
    let mut payloads = Vec::new();
    while let Ok(message) = events.try_recv() {
        if message["event"] == name {
            payloads.push(message["payload"].clone());
        }
    }
    payloads
}

/// 会话开始前就存在的崩溃现场属于历史记录, 不提示.
#[test]
fn crash_sites_before_the_session_are_ignored() {
    let dir = super::TempDir::new("hang-baseline");
    let prefix = dir.join("prefix");
    crash_site(
        &prefix,
        "UECC-old_0000",
        Some(&crash_context("Hang", "旧现场")),
    );
    let (app, events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    sample_round(&app, &session, true);
    sample_round(&app, &session, true);

    assert_eq!(hang_site(&app), None);
    assert!(event_payloads(&events, "game-state")
        .iter()
        .all(|payload| payload.get("hang").is_none()));
    assert!(app.playtime.history().unwrap().active.is_some());
}

/// 新现场在下一轮采样即上报, 同时收敛游玩记录并回前台.
#[test]
fn a_new_hang_is_reported_and_stops_playtime() {
    let dir = super::TempDir::new("hang-new");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let (shown, host_shows) = mpsc::channel();
    app.subscribe("host-show", move |_| {
        let _ = shown.send(());
    });
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    sample_round(&app, &session, true);
    assert!(app.playtime.history().unwrap().active.is_some());

    crash_site(
        &prefix,
        "UECC-Windows-1A2B_0000",
        Some(&crash_context("Hang", "watchdog timeout")),
    );
    sample_round(&app, &session, true);

    let hang = app
        .state
        .game
        .lock()
        .unwrap()
        .status()
        .hang
        .expect("新现场应上报");
    assert_eq!(hang.site, "UECC-Windows-1A2B_0000");
    assert_eq!(hang.crash_type, "Hang");
    assert_eq!(hang.error_message, "watchdog timeout");
    assert!(hang.detected_at > 1_600_000_000_000);
    assert_eq!(app.playtime.history().unwrap().active, None);
    host_shows
        .recv_timeout(Duration::from_secs(2))
        .expect("卡死时必须把启动器带回前台");
    let payload = event_payloads(&events, "game-state")
        .into_iter()
        .find(|payload| payload.get("hang").is_some())
        .expect("game-state 应带上现场摘要");
    assert_eq!(payload["hang"]["crashType"], json!("Hang"));
    assert_eq!(payload["hang"]["site"], json!("UECC-Windows-1A2B_0000"));
    assert_eq!(payload["hang"]["detectedAt"], json!(hang.detected_at));
}

/// 同一轮会话只提示一次, 后续采样不再发事件也不再回前台.
#[test]
fn a_hang_is_reported_once_per_session() {
    let dir = super::TempDir::new("hang-once");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let (shown, host_shows) = mpsc::channel();
    app.subscribe("host-show", move |_| {
        let _ = shown.send(());
    });
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    crash_site(
        &prefix,
        "UECC-first_0000",
        Some(&crash_context("Hang", "第一次")),
    );
    sample_round(&app, &session, true);
    sample_round(&app, &session, true);
    // 第二份现场也不重复提示, 同一轮会话只认首次.
    crash_site(
        &prefix,
        "UECC-second_0000",
        Some(&crash_context("Crash", "第二次")),
    );
    sample_round(&app, &session, true);

    let payloads: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload.get("hang").is_some())
        .collect();
    assert_eq!(payloads.len(), 1, "每轮会话只上报一次现场");
    assert_eq!(payloads[0]["hang"]["site"], json!("UECC-first_0000"));
    // 卡死之后不再重新开始计时.
    assert_eq!(app.playtime.history().unwrap().active, None);
    host_shows
        .recv_timeout(Duration::from_secs(2))
        .expect("卡死时必须把启动器带回前台");
    assert!(
        host_shows.recv_timeout(Duration::from_millis(200)).is_err(),
        "重复采样不应再次回前台"
    );
    // 会话结束后摘要仍留在注册表里.
    let mut registry = app.state.game.lock().unwrap();
    session::finish_game_close(&mut registry, 1, true);
    assert!(registry.session.is_none());
    assert_eq!(
        registry.status().hang.map(|hang| hang.site),
        Some("UECC-first_0000".into())
    );
}

/// 非 Hang 的现场同样上报, 载荷保留原始类型.
#[test]
fn other_crash_types_are_reported_with_their_own_value() {
    let dir = super::TempDir::new("hang-crash-type");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    crash_site(
        &prefix,
        "UECC-Crash_0000",
        Some(&crash_context("Crash", "GPU device removed")),
    );
    sample_round(&app, &session, true);

    let payload = event_payloads(&events, "game-state")
        .into_iter()
        .find(|payload| payload.get("hang").is_some())
        .expect("崩溃现场也应上报");
    assert_eq!(payload["hang"]["crashType"], json!("Crash"));
    assert_eq!(payload["hang"]["errorMessage"], json!("GPU device removed"));
}

/// 现场内容读不出来时按目录名上报, 不 panic 也不带入无关内容.
#[test]
fn unreadable_contexts_are_reported_without_details() {
    let dir = super::TempDir::new("hang-unreadable");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    // 残缺 XML: 目录存在即说明 UE 写下了现场.
    crash_site(
        &prefix,
        "UECC-truncated_0000",
        Some("<FGenericCrashContext><RuntimeProperties><CrashType>Hang"),
    );
    sample_round(&app, &session, true);
    let payload = event_payloads(&events, "game-state")
        .into_iter()
        .find(|payload| payload.get("hang").is_some())
        .expect("残缺现场仍要上报");
    assert_eq!(payload["hang"]["site"], json!("UECC-truncated_0000"));

    // 符号链接的上下文不被跟随, 只有目录本身算现场.
    let dir2 = super::TempDir::new("hang-linked-context");
    let prefix2 = dir2.join("prefix");
    let (app2, events2) = crate::service::test_service_with_output();
    let session2 = hang_session(2, &prefix2);
    app2.state.game.lock().unwrap().session = Some(session2.clone());
    let secret = dir2.join("secret.txt");
    fs::write(&secret, crash_context("Hang", "不该被读到")).unwrap();
    let linked = crash_site(&prefix2, "UECC-linked_0000", None);
    std::os::unix::fs::symlink(&secret, linked.join("CrashContext.runtime-xml")).unwrap();
    sample_round(&app2, &session2, true);
    let payload = event_payloads(&events2, "game-state")
        .into_iter()
        .find(|payload| payload.get("hang").is_some())
        .expect("现场目录本身仍要上报");
    assert_eq!(payload["hang"]["crashType"], json!(""));
    assert_eq!(payload["hang"]["errorMessage"], json!(""));

    // 超过读取上限的上下文不解析.
    let dir3 = super::TempDir::new("hang-oversized-context");
    let prefix3 = dir3.join("prefix");
    let (app3, events3) = crate::service::test_service_with_output();
    let session3 = hang_session(3, &prefix3);
    app3.state.game.lock().unwrap().session = Some(session3.clone());
    let huge = "x".repeat(crate::logs::MAX_TEXT_BYTES as usize + 1);
    crash_site(
        &prefix3,
        "UECC-huge_0000",
        Some(&format!("<CrashType>Hang</CrashType>{huge}")),
    );
    sample_round(&app3, &session3, true);
    let payload = event_payloads(&events3, "game-state")
        .into_iter()
        .find(|payload| payload.get("hang").is_some())
        .expect("超大现场仍要上报");
    assert_eq!(payload["hang"]["crashType"], json!(""));
}

/// 迟到的旧采样不能把现场记到下一轮会话上.
#[test]
fn a_stale_sample_does_not_report_on_the_current_session() {
    let dir = super::TempDir::new("hang-stale");
    let prefix = dir.join("prefix");
    let (app, _events) = crate::service::test_service_with_output();
    let stale = hang_session(1, &prefix);
    let current = hang_session(2, &prefix);
    {
        let mut registry = app.state.game.lock().unwrap();
        registry.session = Some(current.clone());
        registry.next_id = 2;
    }
    crash_site(
        &prefix,
        "UECC-stale_0000",
        Some(&crash_context("Hang", "旧采样")),
    );

    sample_round(&app, &stale, true);

    let registry = app.state.game.lock().unwrap();
    assert!(registry.status().hang.is_none());
    assert_eq!(registry.session.as_ref().map(|session| session.id), Some(2));
}

/// 下一次启动才清空摘要, 让玩家来得及看到上一轮的现场.
#[test]
fn hang_survives_the_close_and_clears_on_the_next_launch() {
    let dir = super::TempDir::new("hang-prepare");
    let prefix = dir.join("prefix");
    let (app, _events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());
    crash_site(
        &prefix,
        "UECC-carried_0000",
        Some(&crash_context("Hang", "交接")),
    );
    sample_round(&app, &session, true);
    assert_eq!(hang_site(&app), Some("UECC-carried_0000".into()));

    {
        let mut registry = app.state.game.lock().unwrap();
        session::finish_game_close(&mut registry, 1, true);
        assert_eq!(
            registry.status().hang.map(|hang| hang.site),
            Some("UECC-carried_0000".into())
        );
        session::prepare_launch(&app.state.packages, &mut registry).expect("重新启动");
        assert!(registry.status().hang.is_none());
    }
    assert!(serde_json::to_value(session::game_status(&app.state))
        .unwrap()
        .get("hang")
        .is_none());
}

/// 会话已经退出时关闭游戏, 返回的状态仍要带出崩溃摘要.
#[test]
fn closing_a_dead_session_keeps_the_hang_summary() {
    let dir = super::TempDir::new("hang-dead-close");
    let prefix = dir.join("prefix");
    let (app, _events) = crate::service::test_service_with_output();
    let mut session = hang_session(1, &prefix);
    // 进程组与 pid 都不存在, 关闭时走"会话已退出"分支.
    session.pgid = 1_999_999_999;
    session.pid = 1_999_999_999;
    app.state.game.lock().unwrap().session = Some(session.clone());
    crash_site(
        &prefix,
        "UECC-dead_0000",
        Some(&crash_context("Hang", "已退出")),
    );
    sample_round(&app, &session, true);
    assert_eq!(hang_site(&app), Some("UECC-dead_0000".into()));
    // 采样之后进程才消失, 关闭时走"会话已退出"分支.
    session.game_alive.store(false, Ordering::Release);

    let status = crate::service::block_on(crate::session::close_game(app.clone(), &app.state))
        .expect("关闭已退出的会话");
    assert!(!status.running);
    let hang = status.hang.expect("关闭后仍要带出崩溃摘要");
    assert_eq!(hang.site, "UECC-dead_0000");
    assert_eq!(hang.crash_type, "Hang");

    // 没有会话时再查状态也要保留摘要.
    let idle = crate::service::block_on(crate::session::close_game(app.clone(), &app.state))
        .expect("再次关闭");
    assert!(idle.hang.is_some(), "空闲状态仍带出上一轮摘要");
}

/// 现场目录先出现时立即上报, 上下文写出后补全摘要并补发状态.
#[test]
fn a_hang_summary_is_filled_in_once_the_context_is_written() {
    let dir = super::TempDir::new("hang-refresh");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let (shown, host_shows) = mpsc::channel();
    app.subscribe("host-show", move |_| {
        let _ = shown.send(());
    });
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    // UE 先建目录, 约定时刻才写出上下文.
    let site = crash_site(&prefix, "UECC-late_0000", None);
    sample_round(&app, &session, true);

    let first = event_payloads(&events, "game-state")
        .into_iter()
        .find(|payload| payload.get("hang").is_some())
        .expect("目录出现即上报");
    assert_eq!(first["hang"]["crashType"], json!(""));
    assert_eq!(first["hang"]["errorMessage"], json!(""));
    host_shows
        .recv_timeout(Duration::from_secs(2))
        .expect("首次命中要回前台");

    fs::write(
        site.join("CrashContext.runtime-xml"),
        crash_context("Hang", "watchdog timeout"),
    )
    .unwrap();
    sample_round(&app, &session, true);

    let payloads: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload.get("hang").is_some())
        .collect();
    assert_eq!(payloads.len(), 1, "摘要补全只补发一次状态");
    assert_eq!(payloads[0]["hang"]["crashType"], json!("Hang"));
    assert_eq!(
        payloads[0]["hang"]["errorMessage"],
        json!("watchdog timeout")
    );
    assert_eq!(
        payloads[0]["hang"]["site"],
        json!("UECC-late_0000"),
        "补全后仍指向同一份现场"
    );
    assert!(
        host_shows.recv_timeout(Duration::from_millis(200)).is_err(),
        "补全摘要不应重复回前台"
    );
    // 之后不再补发.
    sample_round(&app, &session, true);
    assert!(event_payloads(&events, "game-state")
        .iter()
        .all(|payload| payload.get("hang").is_none()
            || payload["hang"]["crashType"] == json!("Hang")));
    let hang = app.state.game.lock().unwrap().status().hang.unwrap();
    assert_eq!(hang.crash_type, "Hang");
    assert_eq!(hang.error_message, "watchdog timeout");
}

/// 上下文分两次写完时只在齐全后补发一次状态.
#[test]
fn a_partial_context_is_refreshed_once_it_completes() {
    let dir = super::TempDir::new("hang-partial");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    let site = crash_site(&prefix, "UECC-partial_0000", None);
    let context = site.join("CrashContext.runtime-xml");
    sample_round(&app, &session, true);
    event_payloads(&events, "game-state");

    // 先只写出崩溃类型: 能说明现场就立即补发.
    fs::write(
        &context,
        "<FGenericCrashContext><CrashType>Hang</CrashType>",
    )
    .unwrap();
    sample_round(&app, &session, true);
    let first: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload.get("hang").is_some())
        .collect();
    assert_eq!(first.len(), 1, "读到崩溃类型即补发状态");
    assert_eq!(first[0]["hang"]["crashType"], json!("Hang"));
    assert_eq!(first[0]["hang"]["errorMessage"], json!(""));

    // 再补上错误信息: 同一份现场只在补齐时再补发一次.
    fs::write(&context, crash_context("Hang", "watchdog timeout")).unwrap();
    sample_round(&app, &session, true);
    sample_round(&app, &session, true);
    sample_round(&app, &session, true);
    let payloads: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload.get("hang").is_some())
        .collect();
    assert_eq!(payloads.len(), 1, "错误说明只补发一次");
    assert_eq!(payloads[0]["hang"]["crashType"], json!("Hang"));
    assert_eq!(
        payloads[0]["hang"]["errorMessage"],
        json!("watchdog timeout")
    );
    assert_eq!(hang_site(&app), Some("UECC-partial_0000".into()));
}

/// 只写了崩溃类型的现场也要展示, 不能因为缺错误说明一直等.
#[test]
fn a_context_without_an_error_message_is_still_shown() {
    let dir = super::TempDir::new("hang-no-message");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    // UE 的 Hang 现场不一定记录错误说明.
    crash_site(
        &prefix,
        "UECC-nomessage_0000",
        Some("<FGenericCrashContext><CrashType>Hang</CrashType></FGenericCrashContext>"),
    );
    sample_round(&app, &session, true);
    sample_round(&app, &session, true);
    let payloads: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload["hang"]["crashType"] == json!("Hang"))
        .collect();
    assert_eq!(payloads.len(), 1, "读到崩溃类型即上报, 不等错误说明");
    assert_eq!(payloads[0]["hang"]["errorMessage"], json!(""));

    // 有崩溃类型后不再重读现场, 不会反复补发状态.
    for _ in 0..4 {
        sample_round(&app, &session, true);
    }
    assert!(
        event_payloads(&events, "game-state").is_empty(),
        "摘要不再变化时不补发状态"
    );
    assert_eq!(hang_site(&app), Some("UECC-nomessage_0000".into()));
}

/// 上下文比补偿窗口更晚落盘时仍要补全, 不能因为窗口到期就永久放弃.
#[test]
fn a_context_written_after_the_retry_window_is_still_read() {
    let dir = super::TempDir::new("hang-late-context");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());

    // 只有目录, 上下文还没写: 先上报一份空摘要.
    let site = crash_site(&prefix, "UECC-late_0000", None);
    sample_round(&app, &session, true);
    let first: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload.get("hang").is_some())
        .collect();
    assert_eq!(first.len(), 1, "目录出现即上报");
    assert_eq!(first[0]["hang"]["crashType"], json!(""));

    // 越过最初的补读窗口后仍在重读: 多轮采样都在窗口之外.
    for _ in 0..8 {
        sample_round(&app, &session, true);
        assert!(
            event_payloads(&events, "game-state").is_empty(),
            "摘要未补齐时不打扰界面"
        );
    }

    // 很久之后才写出上下文, 仍要补全并补发一次.
    fs::write(
        site.join("CrashContext.runtime-xml"),
        crash_context("Hang", "晚到的现场"),
    )
    .unwrap();
    sample_round(&app, &session, true);
    sample_round(&app, &session, true);
    let payloads: Vec<Value> = event_payloads(&events, "game-state")
        .into_iter()
        .filter(|payload| payload.get("hang").is_some())
        .collect();
    assert_eq!(payloads.len(), 1, "补全只补发一次状态");
    assert_eq!(payloads[0]["hang"]["crashType"], json!("Hang"));
    assert_eq!(payloads[0]["hang"]["errorMessage"], json!("晚到的现场"));
}

/// 卡死后不再推进快照, 挂起的会话不会记成成功启动.
#[test]
fn a_hang_freezes_the_snapshot_success() {
    let dir = super::TempDir::new("hang-snapshot");
    let prefix = dir.join("prefix");
    let app = crate::service::test_service();
    let inputs = crate::snapshot_commands::current_inputs(
        &crate::testing::packages(),
        &crate::settings::Settings::default(),
    );
    let id = app
        .snapshots
        .begin(crate::snapshots::capture(inputs))
        .expect("记录启动快照");
    let mut session = hang_session(1, &prefix);
    session.snapshot = Some(id.clone());
    session.snapshot_success_after_ms = 60_000;
    app.state.game.lock().unwrap().session = Some(session.clone());

    // 先正常记到运行中.
    sample_round(&app, &session, true);
    let status = |app: &crate::service::ServiceHandle| {
        app.snapshots
            .overview()
            .expect("快照概览")
            .latest
            .expect("最新快照")
            .status
    };
    assert_eq!(status(&app), crate::snapshots::Status::Running);

    // 现场出现后, 即使运行时长已经到点也不记成功.
    crash_site(
        &prefix,
        "UECC-frozen_0000",
        Some(&crash_context("Hang", "freeze")),
    );
    sample_round(&app, &session, true);
    assert_eq!(hang_site(&app), Some("UECC-frozen_0000".into()));
    {
        let mut state = session.monitor_state.lock().unwrap();
        state.snapshot_running_at = Some(0);
    }
    sample_round(&app, &session, true);
    assert_eq!(status(&app), crate::snapshots::Status::Running);
    assert!(!matches!(status(&app), crate::snapshots::Status::Success));

    // 收尾按失败归档, 不会被后来的成功判定覆盖.
    crate::session::monitor::finalize_game_session(&app, &session, None);
    assert_eq!(status(&app), crate::snapshots::Status::Failed);
}

/// 正在关闭的会话不再扫描现场.
#[test]
fn a_closing_session_does_not_scan_for_hangs() {
    let dir = super::TempDir::new("hang-closing");
    let prefix = dir.join("prefix");
    let (app, events) = crate::service::test_service_with_output();
    let mut session = hang_session(1, &prefix);
    session.closing = true;
    app.state.game.lock().unwrap().session = Some(session.clone());
    crash_site(
        &prefix,
        "UECC-closing_0000",
        Some(&crash_context("Hang", "正在关闭")),
    );

    sample_round(&app, &session, true);

    assert!(hang_site(&app).is_none());
    assert!(event_payloads(&events, "game-state")
        .iter()
        .all(|payload| payload.get("hang").is_none()));
}

/// 游玩记录按发现时刻收束, 时长足够时写入历史.
#[test]
fn a_hang_closes_the_playtime_session() {
    let dir = super::TempDir::new("hang-playtime-history");
    let prefix = dir.join("prefix");
    let app = crate::service::test_service();
    let session = hang_session(1, &prefix);
    app.state.game.lock().unwrap().session = Some(session.clone());
    // 直接写入一段已经持续两分钟的游玩记录, 让收尾时能通过最短时长过滤.
    app.playtime
        .start(
            crate::playtime_commands::now_millis() - 120_000,
            "background",
            "cn",
        )
        .expect("开始记录游玩时长");
    session.monitor_state.lock().unwrap().playtime_started = true;

    crash_site(
        &prefix,
        "UECC-history_0000",
        Some(&crash_context("Hang", "history")),
    );
    sample_round(&app, &session, true);

    let history = app.playtime.history().expect("读取游玩记录");
    assert_eq!(history.active, None);
    assert_eq!(history.sessions.len(), 1);
    assert!(history.sessions[0].duration_ms() >= 60_000);
}
