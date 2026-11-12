//! 后台任务能力测试.

use super::{block_on, TestHost};
use crate::{CommandGroup, DispatchFuture, HostContext, Registry, TaskOutcome, TaskProgress};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn task_result_is_available_through_the_handle() {
    let host = TestHost::new();
    let task = host.submit(Box::new(|_| TaskOutcome::Done(json!({"accounts": 2}))));
    assert_eq!(
        block_on(task.wait()),
        TaskOutcome::Done(json!({"accounts": 2}))
    );
}

#[test]
fn failed_tasks_keep_their_own_payload() {
    let host = TestHost::new();
    let failure = json!({"kind": "busy", "message": "账号或同类任务正在处理, 请稍后重试"});
    let expected = failure.clone();
    let task = host.submit(Box::new(move |_| TaskOutcome::Failed(failure)));
    assert_eq!(block_on(task.wait()), TaskOutcome::Failed(expected));
}

#[test]
fn cancelled_tasks_are_distinct_from_failures() {
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

    // 取消句柄与等待句柄可以是同一任务的克隆.
    let control = task.clone();
    control.cancel();
    assert!(task.is_cancelled());
    assert_eq!(block_on(task.wait()), TaskOutcome::Cancelled);
}

#[test]
fn outcome_is_available_without_consuming_the_handle() {
    let host = TestHost::new();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let task = host.submit(Box::new(move |_| {
        started_tx.send(()).unwrap();
        finish_rx.recv().unwrap();
        TaskOutcome::Done(json!({"ok": true}))
    }));
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("任务应已启动");
    assert!(!task.is_finished());
    assert_eq!(task.outcome(), None);

    // 等待句柄取回结果后, 同一任务的克隆仍能读结局.
    let control = task.clone();
    finish_tx.send(()).unwrap();
    assert_eq!(
        block_on(task.wait()),
        TaskOutcome::Done(json!({"ok": true}))
    );
    assert!(control.is_finished());
    assert_eq!(
        control.outcome(),
        Some(TaskOutcome::Done(json!({"ok": true})))
    );
}

#[test]
fn progress_carries_optional_stage_and_detail() {
    let host = TestHost::new();
    let task = host.submit(Box::new(|context| {
        context
            .progress(
                TaskProgress::new(1, 3)
                    .with_stage("download")
                    .with_detail("资源包"),
            )
            .unwrap();
        context
            .progress(TaskProgress::new(2, 3).with_stage("download"))
            .unwrap();
        context.progress(TaskProgress::new(3, 3)).unwrap();
        TaskOutcome::Done(json!(true))
    }));
    let id = task.id().to_string();
    assert_eq!(block_on(task.wait()), TaskOutcome::Done(json!(true)));

    let progress: Vec<Value> = host
        .events
        .recorded()
        .into_iter()
        .filter(|(event, _)| *event == "task-progress")
        .map(|(_, payload)| payload)
        .collect();
    assert_eq!(
        progress,
        [
            json!({"task": id, "done": 1, "total": 3, "stage": "download", "detail": "资源包"}),
            json!({"task": id, "done": 2, "total": 3, "stage": "download"}),
            json!({"task": id, "done": 3, "total": 3}),
        ]
    );
}

#[test]
fn progress_is_reported_through_the_event_outlet() {
    let host = TestHost::new();
    let task = host.submit(Box::new(|context| {
        context.progress(TaskProgress::new(1, 2)).unwrap();
        context.progress(TaskProgress::new(2, 2)).unwrap();
        TaskOutcome::Done(json!(true))
    }));
    let id = task.id().to_string();
    assert_eq!(block_on(task.wait()), TaskOutcome::Done(json!(true)));

    let progress: Vec<Value> = host
        .events
        .recorded()
        .into_iter()
        .filter(|(event, _)| *event == "task-progress")
        .map(|(_, payload)| payload)
        .collect();
    assert_eq!(
        progress,
        [
            json!({"task": id, "done": 1, "total": 2}),
            json!({"task": id, "done": 2, "total": 2}),
        ]
    );
}

/// 求和的测试组: 命令提交任务, 等任务结局后按命令载荷返回.
fn sum<'a>(host: &'a dyn HostContext, _command: &'a str, payload: Value) -> DispatchFuture<'a> {
    Box::pin(async move {
        let total = payload["total"].as_u64().unwrap_or(0);
        let task = host.submit(Box::new(move |context| {
            let mut sum = 0;
            for step in 1..=total {
                sum += step;
                let _ = context.progress(TaskProgress::new(step, total));
            }
            TaskOutcome::Done(json!({"sum": sum}))
        }));
        match task.wait().await {
            TaskOutcome::Done(value) => Ok(value),
            TaskOutcome::Failed(value) => Err(value),
            TaskOutcome::Cancelled => Err(json!("任务已取消")),
        }
    })
}

#[test]
fn registry_routes_a_command_that_submits_a_task() {
    let host = TestHost::new();
    let mut registry = Registry::new();
    registry.register(CommandGroup::for_game(
        "test",
        "task",
        &["game_status"],
        sum,
    ));

    let group = registry
        .group_for("test", "game_status")
        .expect("命令组应已登记");
    let outcome = block_on(group.dispatch(&host, "game_status", json!({"total": 3})));
    assert_eq!(outcome, Ok(json!({"sum": 6})));
    assert_eq!(
        host.events
            .recorded()
            .iter()
            .filter(|(event, _)| *event == "task-progress")
            .count(),
        3
    );
}
