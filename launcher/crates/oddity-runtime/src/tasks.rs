//! 宿主任务注册表: 登记提交过的任务, 让宿主按 id 取消并回收已结束的任务.
//!
//! 句柄仍由提交方持有; 注册表只保存克隆, 不改变任务的所有权与等待方式.

use crate::Task;
use std::sync::{Mutex, MutexGuard};

/// 宿主登记的后台任务.
///
/// 每次访问都顺带回收已结束的任务, 因此表中只剩仍在运行或等待执行的任务.
#[derive(Default)]
pub struct TaskRegistry {
    tasks: Mutex<Vec<Task>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个任务并返回原句柄.
    pub fn track(&self, task: Task) -> Task {
        self.live().push(task.clone());
        task
    }

    /// 按任务 id 请求取消; 没有这个任务或已经结束时返回 false.
    pub fn cancel(&self, id: &str) -> bool {
        let tasks = self.live();
        let Some(task) = tasks.iter().find(|tracked| tracked.id() == id) else {
            return false;
        };
        task.cancel();
        true
    }

    /// 请求取消全部登记任务; 已结束的任务先被回收.
    pub fn cancel_all(&self) {
        for task in self.live().iter() {
            task.cancel();
        }
    }

    /// 仍在登记的未结束任务数.
    pub fn len(&self) -> usize {
        self.live().len()
    }

    /// 是否没有未结束的任务.
    pub fn is_empty(&self) -> bool {
        self.live().is_empty()
    }

    /// 回收已结束的任务后交出登记表.
    fn live(&self) -> MutexGuard<'_, Vec<Task>> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|error| error.into_inner());
        tasks.retain(|tracked| !tracked.is_finished());
        tasks
    }
}
