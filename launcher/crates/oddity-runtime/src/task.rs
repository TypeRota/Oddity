//! 宿主后台任务: 提交, 进度, 取消与完成结果.
//!
//! 包把阻塞工作交给宿主执行器, 拿到可取消的 [`Task`] 句柄. 任务体持有
//! [`TaskContext`], 进度经宿主绑定的 [`ProgressSink`] 写进契约声明的
//! `task-progress` 事件, 完成结果区分成功载荷, 失败载荷与取消.

use crate::Cancellation;
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

/// 任务进度: 已完成量与总量, 单位由任务自己定.
///
/// 阶段与说明是可选的补充信息; 速率与剩余字节由接收方按时间戳和总量
/// 自行推导, 不进载荷.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskProgress {
    pub done: u64,
    pub total: u64,
    /// 阶段标识, 例如 download、verify; 同一次任务内可以变化.
    pub stage: Option<&'static str>,
    /// 当前阶段的简短说明.
    pub detail: Option<&'static str>,
}

impl TaskProgress {
    pub const fn new(done: u64, total: u64) -> Self {
        Self {
            done,
            total,
            stage: None,
            detail: None,
        }
    }

    /// 补充阶段标识.
    pub const fn with_stage(mut self, stage: &'static str) -> Self {
        self.stage = Some(stage);
        self
    }

    /// 补充阶段说明.
    pub const fn with_detail(mut self, detail: &'static str) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// 进度事件的发送口.
///
/// 宿主提交任务时把它绑定到 `task-progress` 事件; 包只交出载荷形状,
/// 不接触事件名, 也不另开通道.
pub type ProgressSink = Arc<dyn Fn(Value) -> Result<(), String> + Send + Sync>;

/// 进度事件的载荷; 任务 id 让接收方把事件关联到提交方.
///
/// 阶段与说明只在任务提供时出现, 旧消费方忽略即可.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload<'a> {
    task: &'a str,
    done: u64,
    total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

/// 任务结局.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskOutcome {
    /// 正常结束, 载荷由包定义.
    Done(Value),
    /// 任务体报告的失败, 载荷是包内失败类型的序列化结果.
    Failed(Value),
    /// 任务读到取消请求后自行停下.
    Cancelled,
}

/// 交给宿主执行器的阻塞任务体.
pub type TaskJob = Box<dyn FnOnce(TaskContext) -> TaskOutcome + Send + 'static>;

/// 任务体持有的能力: 汇报进度, 观察取消.
#[derive(Clone)]
pub struct TaskContext {
    id: Arc<str>,
    cancellation: Cancellation,
    progress: ProgressSink,
}

impl TaskContext {
    /// 任务 id, 与进度事件里的 `task` 字段一致.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 取消令牌; 任务体在安全点轮询它.
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    /// 是否已经请求取消.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// 发送一条进度事件; 宿主输出已关闭时返回错误, 任务可以放弃进度.
    pub fn progress(&self, progress: TaskProgress) -> Result<(), String> {
        (self.progress)(
            serde_json::to_value(ProgressPayload {
                task: &self.id,
                done: progress.done,
                total: progress.total,
                stage: progress.stage,
                detail: progress.detail,
            })
            .map_err(|error| error.to_string())?,
        )
    }
}

/// 完成状态: 结果只写一次, 等待方由唤醒通知.
#[derive(Default)]
struct Completion {
    outcome: Option<TaskOutcome>,
    waker: Option<Waker>,
}

fn lock(state: &Mutex<Completion>) -> MutexGuard<'_, Completion> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

/// 宿主后台任务的句柄.
///
/// 取消是协作式的, [`Self::cancel`] 只设置令牌; 丢弃句柄不会取消任务.
/// 克隆出的句柄共享取消状态与完成结果, 宿主可以留一份用于取消, 另一份
/// 交给等待方.
#[derive(Clone)]
pub struct Task {
    context: TaskContext,
    completion: Arc<Mutex<Completion>>,
}

impl Task {
    /// 宿主提交任务时先建句柄, 再用 [`Self::runner`] 把任务体交给执行器.
    pub fn new(progress: ProgressSink) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = format!("task-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        Self {
            context: TaskContext {
                id: id.into(),
                cancellation: Cancellation::new(),
                progress,
            },
            completion: Arc::new(Mutex::new(Completion::default())),
        }
    }

    /// 任务 id, 与进度事件里的 `task` 字段一致.
    pub fn id(&self) -> &str {
        self.context.id()
    }

    /// 请求任务在下一个安全点退出.
    pub fn cancel(&self) {
        self.context.cancellation.cancel();
    }

    /// 是否已经请求取消.
    pub fn is_cancelled(&self) -> bool {
        self.context.is_cancelled()
    }

    /// 已结束任务的结局; 仍在运行时返回 None.
    ///
    /// 只读窥视, 不消费句柄, 等待方仍可照常取回结果.
    pub fn outcome(&self) -> Option<TaskOutcome> {
        lock(&self.completion).outcome.clone()
    }

    /// 任务是否已经结束.
    pub fn is_finished(&self) -> bool {
        self.outcome().is_some()
    }

    /// 包装任务体; 返回的闭包必须在宿主执行器上运行, 句柄才会拿到结果.
    pub fn runner<F>(&self, body: F) -> impl FnOnce() + Send + 'static
    where
        F: FnOnce(TaskContext) -> TaskOutcome + Send + 'static,
    {
        let context = self.context.clone();
        let completion = Arc::clone(&self.completion);
        move || {
            // 任务体 panic 时也要落结局, 否则等待方永远等不到结果.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(context)))
                .unwrap_or_else(|_| TaskOutcome::Failed(Value::String("任务异常退出".into())));
            let waker = {
                let mut state = lock(&completion);
                state.outcome = Some(outcome);
                state.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    /// 等待任务结束; 消费的句柄不再用于取消.
    pub fn wait(self) -> TaskWait {
        TaskWait {
            completion: self.completion,
        }
    }
}

/// 等待任务结束的结果.
pub struct TaskWait {
    completion: Arc<Mutex<Completion>>,
}

impl Future for TaskWait {
    type Output = TaskOutcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<TaskOutcome> {
        let mut state = lock(&self.completion);
        match &state.outcome {
            Some(outcome) => Poll::Ready(outcome.clone()),
            None => {
                state.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}
