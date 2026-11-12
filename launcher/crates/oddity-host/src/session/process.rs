//! 宿主拉起的进程句柄: 回收线程缓存结局, 包只读结果.

use std::process::Child;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use oddity_runtime::{GameProcess, ProcessExit};

/// 宿主已经拉起并接管的进程.
///
/// 回收线程立即 wait 并缓存结局; try_wait/wait 只读共用的缓存, 可重复调用,
/// 即使包线程异常退出也不会留下僵尸.
pub(crate) struct SpawnedProcess {
    pid: u32,
    pgid: i32,
    state: Arc<ExitState>,
}

#[derive(Default)]
struct ExitState {
    exit: Mutex<Option<ProcessExit>>,
    ready: Condvar,
}

impl SpawnedProcess {
    /// 接管已拉起的进程; 立即由宿主回收线程 wait.
    pub(crate) fn adopt(mut child: Child) -> Self {
        let pid = child.id();
        let pgid = i32::try_from(pid).unwrap_or(0);
        let state = Arc::new(ExitState::default());
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            let exit = match child.wait() {
                Ok(status) => match status.code() {
                    Some(code) => ProcessExit::Exited(code),
                    None => ProcessExit::Signaled,
                },
                Err(error) => {
                    eprintln!("回收游戏进程失败: {error}");
                    ProcessExit::Signaled
                }
            };
            if let Ok(mut slot) = shared.exit.lock() {
                *slot = Some(exit);
            }
            shared.ready.notify_all();
        });
        Self { pid, pgid, state }
    }
}

impl GameProcess for SpawnedProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn pgid(&self) -> i32 {
        self.pgid
    }

    fn try_wait(&self) -> Result<Option<ProcessExit>, String> {
        self.state
            .exit
            .lock()
            .map(|exit| *exit)
            .map_err(|_| "进程结局不可用".to_string())
    }

    fn wait(&self) -> Result<ProcessExit, String> {
        let mut exit = self
            .state
            .exit
            .lock()
            .map_err(|_| "进程结局不可用".to_string())?;
        while exit.is_none() {
            exit = self
                .state
                .ready
                .wait(exit)
                .map_err(|_| "进程结局不可用".to_string())?;
        }
        exit.ok_or_else(|| "进程结局不可用".to_string())
    }
}
