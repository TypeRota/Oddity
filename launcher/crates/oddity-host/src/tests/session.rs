//! 会话注册表的 game-state 事件序列与锁序.

use crate::channel::ChannelId;
use crate::identity::GameId;
use crate::session::{self, Registry, Session};
use crate::settings::Settings;
use crate::AppState;
use oddity_runtime::{LaunchProgress, LaunchStage};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

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
        monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
        control: crate::session::ControlSlot::default(),
    }
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
