//! 《异环》的进程级长驻域: 下载调度, 设置门闩, 签到触发与无头入口.
//!
//! 宿主只看到 [`PackageRuntime`] 的方法; 下载状态机, 自动调度线程与签到
//! 业务都留在包内.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use oddity_runtime::{HostContext, PackageGate, PackageRuntime};

use crate::update::{self, state::SettingsGate};

/// 装配表引用的唯一长驻域实例.
static RUNTIME: NteRuntime = NteRuntime {
    stop: Mutex::new(None),
};

/// 交给宿主的长驻域.
pub(crate) fn runtime() -> &'static dyn PackageRuntime {
    &RUNTIME
}

struct NteRuntime {
    /// 宿主创建的共享停止位; 启动时保存, 关闭时置位.
    stop: Mutex<Option<Arc<AtomicBool>>>,
}

impl PackageRuntime for NteRuntime {
    fn start(&self, host: Arc<dyn HostContext>, stop: Arc<AtomicBool>) {
        update::bind_host(Arc::clone(&host));
        if let Ok(mut slot) = self.stop.lock() {
            *slot = Some(Arc::clone(&stop));
        }
        update::auto::start(host, stop);
    }

    fn stop(&self) {
        let stop = self.stop.lock().ok().and_then(|mut slot| slot.take());
        if let Some(stop) = stop {
            stop.store(true, Ordering::Relaxed);
        }
        // 与关闭顺序一致: 调度线程停止后再取消当前下载.
        update::cancel_download();
    }

    fn busy(&self) -> bool {
        update::is_busy()
    }

    fn blocks_changes(&self) -> bool {
        update::blocks_changes(&update::snapshot())
    }

    fn gate(&self) -> Result<Box<dyn PackageGate>, String> {
        let gate = update::settings_gate()?;
        Ok(Box::new(UpdateGate {
            blocks: update::blocks_changes(&gate.status()),
            _gate: gate,
        }))
    }

    fn on_launch(&self, host: &dyn HostContext) {
        crate::attendance::lifecycle::on_launch(host);
    }

    fn headless_entry(&self, args: &[OsString]) -> Option<i32> {
        crate::attendance::lifecycle::headless_entry(args)
    }
}

/// 持有下载状态锁的设置门闩.
struct UpdateGate {
    blocks: bool,
    _gate: SettingsGate,
}

impl PackageGate for UpdateGate {
    fn blocks_changes(&self) -> bool {
        self.blocks
    }
}
