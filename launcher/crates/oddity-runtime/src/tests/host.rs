//! 宿主能力接口测试.

use super::TestHost;
use crate::{Cancellation, HostContext, TaskOutcome};
use serde_json::json;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn settings_are_exposed_as_a_read_only_snapshot() {
    let mut host = TestHost::new();
    host.settings.language = "ja-JP".into();
    let host: &dyn HostContext = &host;

    let snapshot = host.settings().expect("测试宿主总能读设置");
    assert_eq!(snapshot.game, "test");
    assert_eq!(snapshot.distribution, "cn");
    assert_eq!(snapshot.language, "ja-JP");
    assert!(!snapshot.reduce_motion);
    assert_eq!(snapshot.theme, "light");
    assert_eq!(snapshot.install_root.as_deref(), Some("/games/test"));
    assert_eq!(
        snapshot.preload_root.as_deref(),
        Some("/games/test/preload")
    );
    assert!(snapshot.update.auto_download);
    assert_eq!(snapshot.update.metered, crate::MeteredPolicy::Pause);
    assert_eq!(snapshot.update.limit_bytes_per_second, 4096);
    assert_eq!(
        snapshot.update.window,
        Some(crate::UpdateWindow {
            start: "02:00".into(),
            end: "06:00".into(),
        })
    );

    // 快照是拷贝; 修改它不会写回宿主设置.
    let mut copy = snapshot;
    copy.language = "zh-CN".into();
    copy.update.auto_download = false;
    assert_eq!(host.settings().unwrap().language, "ja-JP");
    assert!(host.settings().unwrap().update.auto_download);
}

#[test]
fn activity_reports_the_hosts_current_state() {
    let activity = TestHost::new().activity().expect("测试宿主总能读活动状态");
    assert!(!activity.game_starting);
    assert!(activity.game_running);
    assert!(!activity.game_closing);
    assert!(!activity.environment_busy);
    assert!(!activity.migration_busy);
    assert!(activity.migration_repair_pending);

    // 载荷字段用 camelCase, 与契约里其它事件载荷一致.
    assert_eq!(
        serde_json::to_value(&activity).expect("活动状态可序列化"),
        json!({
            "gameStarting": false,
            "gameRunning": true,
            "gameClosing": false,
            "environmentBusy": false,
            "migrationBusy": false,
            "migrationRepairPending": true,
        })
    );
}

#[test]
fn launcher_root_is_absent_when_the_game_is_not_installed() {
    assert_eq!(TestHost::new().official_launcher_root(), None);
}

#[test]
fn events_carry_declared_ids_and_payloads() {
    let host = TestHost::new();
    host.emit("game-download-state", json!({"phase": "ready"}))
        .unwrap();
    assert_eq!(
        host.events.recorded(),
        [("game-download-state", json!({"phase": "ready"}))]
    );
}

#[test]
fn submitted_jobs_run_with_their_cancellation_token() {
    let host = TestHost::new();
    let (done_tx, done_rx) = mpsc::channel();
    let task = host.submit(Box::new(move |context| {
        while !context.is_cancelled() {
            std::thread::yield_now();
        }
        done_tx.send(()).unwrap();
        TaskOutcome::Cancelled
    }));

    assert!(!task.is_cancelled());
    task.cancel();
    assert!(task.is_cancelled());
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("任务应观察到取消");
    assert_eq!(super::block_on(task.wait()), TaskOutcome::Cancelled);
}

#[test]
fn cancel_task_reaches_the_host_task_registry() {
    let host = TestHost::new();
    let (started_tx, started_rx) = mpsc::channel();
    let task = host.submit(Box::new(move |context| {
        started_tx.send(()).unwrap();
        while !context.is_cancelled() {
            std::thread::yield_now();
        }
        TaskOutcome::Cancelled
    }));
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("任务应已启动");

    let id = task.id().to_string();
    assert!(host.cancel_task(&id));
    assert_eq!(super::block_on(task.wait()), TaskOutcome::Cancelled);

    // 已结束的任务会被回收, 再取消就找不到了.
    assert!(!host.cancel_task(&id));
    assert!(!host.cancel_task("task-missing"));
}

#[test]
fn cancellation_tokens_share_state_through_clones() {
    let token = Cancellation::new();
    let clone = token.clone();
    assert!(!clone.is_cancelled());
    token.cancel();
    assert!(clone.is_cancelled());
    assert!(token.is_cancelled());
    assert!(!Cancellation::new().is_cancelled());
}
