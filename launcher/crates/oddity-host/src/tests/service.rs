//! 会话服务与界面连接的生命周期.
//!
//! 界面只是服务的一个客户端: 断开连接不能结束运行中的会话, 重新连上要读回同一份
//! 权威状态.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::service::{self, EventBus, Service, ServiceConfig, ServiceHandle};
use crate::settings::Settings;

/// 构造一个使用连接总线的服务; 返回服务与它的事件登记表.
fn socket_service() -> (ServiceHandle, Arc<crate::ipc::Clients>) {
    let clients = Arc::new(crate::ipc::Clients::default());
    let (input, _jobs) = std::sync::mpsc::sync_channel(64);
    let service = Service::new(ServiceConfig {
        settings: Settings::default(),
        data_dir: std::env::temp_dir()
            .join(format!("oddity-service-connections-{}", std::process::id())),
        registry: crate::testing::packages(),
        events: EventBus::Clients(Arc::clone(&clients)),
        input,
    });
    (service, clients)
}

/// 只带标识的空会话; 不涉及真实进程.
fn idle_session(id: u64) -> crate::session::Session {
    crate::session::Session {
        performance: Arc::new(std::sync::Mutex::new(None)),
        id,
        pid: 1234,
        pgid: 1234,
        exe: "/game/HTGame.exe".into(),
        started_at: 1,
        log: String::new(),
        mode: "background".into(),
        distribution: crate::channel::ChannelId::new("cn"),
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
        snapshot_success_after_ms: 0,
        launcher_root: None,
        prefix: std::path::PathBuf::new(),
        monitor_state: Arc::new(std::sync::Mutex::new(
            crate::session::monitor::MonitorState::default(),
        )),
        control: crate::session::ControlSlot::default(),
    }
}

#[test]
fn connections_are_counted_per_client() {
    let (app, clients) = socket_service();
    assert_eq!(app.connections(), 0);

    let (first, _first_messages) = clients.register_for_test();
    assert_eq!(app.connections(), 1);
    let (second, _second_messages) = clients.register_for_test();
    assert_eq!(app.connections(), 2);

    clients.unregister_for_test(first);
    assert_eq!(app.connections(), 1);
    clients.unregister_for_test(second);
    assert_eq!(app.connections(), 0);
}

#[test]
fn a_running_session_keeps_the_service_alive_without_clients() {
    let (app, _clients) = socket_service();
    app.state.game.lock().unwrap().session = Some(idle_session(7));

    assert!(
        service::service_busy(&app, 0),
        "没有界面时运行中的会话仍要留住服务"
    );
}

#[test]
fn a_disconnected_interface_leaves_no_reason_to_keep_the_service() {
    let (app, _clients) = socket_service();
    assert!(
        !service::service_busy(&app, 0),
        "既没有连接也没有会话时服务应退出"
    );
    assert!(service::service_busy(&app, 1), "有界面连接时服务不能退出");
}

#[test]
fn a_pending_launch_keeps_the_service_alive() {
    let (app, _clients) = socket_service();
    app.state.game.lock().unwrap().starting = true;
    assert!(service::service_busy(&app, 0));

    {
        let mut game = app.state.game.lock().unwrap();
        game.starting = false;
        game.environment_busy = true;
    }
    assert!(
        service::service_busy(&app, 0),
        "运行环境任务进行时服务不能退出"
    );
}

#[test]
fn session_status_survives_a_reconnect() {
    // 同一份服务状态被第二个"界面"读到: 连接只是视图, 不是状态本身.
    let (app, _clients) = socket_service();
    app.state.game.lock().unwrap().session = Some(idle_session(9));

    let status = service::block_on(async {
        crate::protocol::dispatch(
            app.clone(),
            "game_status".into(),
            None,
            serde_json::json!({}),
        )
        .await
    })
    .expect("读取会话状态");
    assert_eq!(status["running"], serde_json::json!(true));
    assert_eq!(status["pid"], serde_json::json!(1234));
}

#[test]
fn closing_the_interface_does_not_clear_the_session() {
    let (app, clients) = socket_service();
    app.state.game.lock().unwrap().session = Some(idle_session(11));
    let (id, _messages) = clients.register_for_test();
    assert_eq!(app.connections(), 1);

    // 界面退出: 只注销连接, 注册表不动.
    clients.unregister_for_test(id);
    assert_eq!(app.connections(), 0);
    let running = app
        .state
        .game
        .lock()
        .unwrap()
        .session
        .as_ref()
        .map(|session| session.id);
    assert_eq!(running, Some(11), "界面退出不应结束会话");
}
