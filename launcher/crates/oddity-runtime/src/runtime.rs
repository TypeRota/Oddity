//! 包的进程级长驻域: 生命周期钩子, 变更门, 无头入口.
//!
//! 宿主只通过本 trait 接触包的进程内状态; 下载调度, 签到触发与门闩的
//! 内部结构都留在包内. 没有长驻任务的包不实现本 trait, [`crate::GamePackage::runtime`]
//! 返回 None, 宿主据此跳过全部钩子.

use crate::HostContext;
use std::ffi::OsString;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 包的长驻域.
///
/// 宿主在启动序列里调用 [`Self::start`], 关闭序列里调用 [`Self::stop`];
/// 两者都不得阻塞宿主. 其余方法供宿主在设置保存, 启动前置检查与打开启动器
/// 时按需调用.
pub trait PackageRuntime: Send + Sync + 'static {
    /// 绑定宿主能力并启动长驻任务.
    ///
    /// `stop` 由宿主创建并共享; 包保存克隆, 在 [`Self::stop`] 里置位.
    fn start(&self, host: Arc<dyn HostContext>, stop: Arc<AtomicBool>);

    /// 关闭: 置停止位并取消自身任务, 不阻塞宿主关闭路径.
    fn stop(&self) {}

    /// 该域是否正在写盘; 宿主用它做跨域互斥.
    fn busy(&self) -> bool {
        false
    }

    /// 该域是否阻塞设置保存与启动.
    ///
    /// 只读检查不阻塞; 只有会写盘的任务才返回 true.
    fn blocks_changes(&self) -> bool {
        false
    }

    /// 设置保存与启动前的变更门; 调用方 drop 释放.
    ///
    /// 默认返回不阻塞的空门禁.
    fn gate(&self) -> Result<Box<dyn PackageGate>, String> {
        Ok(Box::new(OpenGate))
    }

    /// 启动器打开后的领域触发 (签到等); 包自己判断是否执行.
    fn on_launch(&self, _host: &dyn HostContext) {}

    /// 无头入口: 识别 argv 并执行, 返回进程退出码; 不识别返回 None.
    fn headless_entry(&self, _args: &[OsString]) -> Option<i32> {
        None
    }
}

/// [`PackageRuntime::gate`] 返回的门闩; 持有时该域与设置/启动互斥.
///
/// 门闩只在持它的线程内使用, 不跨线程传递.
pub trait PackageGate {
    /// 持有期间该域是否与设置/启动互斥.
    fn blocks_changes(&self) -> bool;
}

/// 不阻塞任何操作的默认门闩.
struct OpenGate;

impl PackageGate for OpenGate {
    fn blocks_changes(&self) -> bool {
        false
    }
}
