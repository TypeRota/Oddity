//! 宿主任务注册表测试.

use super::block_on;
use crate::{Task, TaskOutcome, TaskRegistry};
use serde_json::json;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn task() -> Task {
    Task::new(Arc::new(|_| Ok(())))
}

#[test]
fn tracks_a_running_task_and_cancels_it_by_id() {
    let registry = TaskRegistry::new();
    assert!(registry.is_empty());

    let task = task();
    let tracked = registry.track(task.clone());
    assert_eq!(tracked.id(), task.id());
    assert_eq!(registry.len(), 1);

    let (started_tx, started_rx) = mpsc::channel();
    std::thread::spawn(task.runner(move |context| {
        started_tx.send(()).unwrap();
        while !context.is_cancelled() {
            std::thread::yield_now();
        }
        TaskOutcome::Cancelled
    }));
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("任务应已启动");

    assert!(registry.cancel(tracked.id()));
    assert_eq!(block_on(tracked.wait()), TaskOutcome::Cancelled);
    assert_eq!(registry.len(), 0);
}

#[test]
fn finished_tasks_are_reaped_on_access() {
    let registry = TaskRegistry::new();
    let task = task();
    std::thread::spawn(task.runner(|_| TaskOutcome::Done(json!(1))));
    let tracked = registry.track(task);

    assert_eq!(
        block_on(tracked.clone().wait()),
        TaskOutcome::Done(json!(1))
    );
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn cancelling_an_unknown_or_finished_task_is_rejected() {
    let registry = TaskRegistry::new();
    assert!(!registry.cancel("task-404"));

    let task = task();
    std::thread::spawn(task.runner(|_| TaskOutcome::Cancelled));
    let tracked = registry.track(task);
    assert_eq!(block_on(tracked.clone().wait()), TaskOutcome::Cancelled);
    assert!(!registry.cancel(tracked.id()));
}

#[test]
fn cancel_all_stops_every_tracked_task() {
    let registry = TaskRegistry::new();
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let task = task();
        let (started_tx, started_rx) = mpsc::channel();
        let tracked = registry.track(task.clone());
        std::thread::spawn(task.runner(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancelled() {
                std::thread::yield_now();
            }
            TaskOutcome::Cancelled
        }));
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("任务应已启动");
        tasks.push(tracked);
    }
    assert_eq!(registry.len(), 2);

    registry.cancel_all();
    for tracked in tasks {
        assert_eq!(block_on(tracked.wait()), TaskOutcome::Cancelled);
    }
    assert!(registry.is_empty());
}
