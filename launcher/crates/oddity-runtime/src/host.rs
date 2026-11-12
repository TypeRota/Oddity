//! 宿主能力: 游戏包只能通过这里读取设置、定位官方启动器、发送事件与提交后台任务.
//!
//! 能力接口保持对象安全, 宿主以 trait object 传给包; 包不接触宿主的
//! 具体类型, 也不持有可写设置. 任务提交走 [`HostContext::submit`], 句柄可
//! 取消并取回结果, 也可以按 id 让宿主统一取消; 进度经同一事件出口发送.

use crate::{CapabilityContext, Task, TaskJob};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 宿主设置的只读快照.
///
/// 只带游戏包真正需要的少量字段; 包拿到的是拷贝, 不能写回设置.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSnapshot {
    /// 当前游戏 id.
    pub game: String,
    /// 当前发行渠道, 也就是区服, 例如 cn、global.
    pub distribution: String,
    /// 界面语言.
    pub language: String,
    /// 是否减少动画.
    pub reduce_motion: bool,
    /// 主题名.
    pub theme: String,
    /// 当前发行渠道配置的安装根目录; 未配置时 None.
    pub install_root: Option<String>,
    /// 自定义预下载缓存目录.
    pub preload_root: Option<String>,
    /// 更新下载的只读策略快照.
    pub update: UpdatePreferences,
}

/// 更新下载的只读策略快照.
///
/// 与设置里的更新选项字段一致, 但不依赖设置模块; 宿主负责映射.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdatePreferences {
    /// 发现更新后自动下载.
    pub auto_download: bool,
    /// 计费网络下的处理方式.
    pub metered: MeteredPolicy,
    /// 下载限速, 单位为字节每秒; 0 表示不限速.
    pub limit_bytes_per_second: u64,
    /// 允许自动下载的本地时间窗口.
    pub window: Option<UpdateWindow>,
}

/// 宿主当前的活动状态.
///
/// 包用它判断能否开始会与游戏抢资源的操作; 读到的只是当下的一帧,
/// 需要在真正动手前再取一次.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshot {
    /// 游戏正在启动.
    pub game_starting: bool,
    /// 游戏进程仍在运行.
    pub game_running: bool,
    /// 游戏正在退出.
    pub game_closing: bool,
    /// 运行环境安装或修复进行中.
    pub environment_busy: bool,
    /// 安装目录迁移进行中.
    pub migration_busy: bool,
    /// 有等待修复的迁移记录.
    pub migration_repair_pending: bool,
}

/// 计费网络下的自动下载策略.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MeteredPolicy {
    /// 等待用户确认.
    #[default]
    Ask,
    /// 暂停, 等切换到非计费网络后再继续.
    Pause,
    /// 照常下载.
    Allow,
}

/// 允许自动下载的本地时间窗口, 格式为 HH:MM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateWindow {
    pub start: String,
    pub end: String,
}

/// 后台任务的取消令牌.
///
/// 克隆出的令牌共享同一个取消状态; 任务体在安全点轮询
/// [`Self::is_cancelled`], 由任务自己决定在哪里停下.
#[derive(Debug, Clone, Default)]
pub struct Cancellation {
    cancelled: Arc<AtomicBool>,
}

impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求取消; 已经完成的任务不受影响.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// 是否已经请求取消.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// 宿主提供给游戏包的能力.
///
/// 宿主以 &dyn HostContext 传入; 包只能读取设置, 发送契约声明的事件,
/// 或把自己的阻塞工作交给宿主执行器.
pub trait HostContext: Send + Sync + 'static {
    /// 读取当前设置快照.
    fn settings(&self) -> Result<SettingsSnapshot, String>;

    /// 读取宿主当前的活动状态.
    ///
    /// 宿主状态不可读时返回错误, 包按自己的策略决定是否继续.
    fn activity(&self) -> Result<ActivitySnapshot, String>;

    /// 当前游戏官方启动器的根目录.
    ///
    /// 宿主把探测交给当前游戏的 [`crate::InstallLocator`]; 未安装或该
    /// 游戏没有官方启动器时返回 None.
    fn official_launcher_root(&self) -> Option<PathBuf>;

    /// 发送契约里声明的事件; 未声明的事件返回错误.
    fn emit(&self, id: &'static str, payload: Value) -> Result<(), String>;

    /// 在宿主后台执行器上运行阻塞任务.
    ///
    /// 返回的句柄可取消, 也可等到任务结局; 任务体的进度经宿主事件出口
    /// 发送, 不新增旁路通道.
    fn submit(&self, job: TaskJob) -> Task;

    /// 按任务 id 请求取消.
    ///
    /// 任务不在宿主登记中或已经结束时返回 false.
    fn cancel_task(&self, id: &str) -> bool;

    /// 能力求值上下文; 默认实现读设置, 活动状态与官方启动器探测.
    fn capability_context(&self) -> Result<CapabilityContext, String> {
        let settings = self.settings()?;
        let activity = self.activity()?;
        Ok(CapabilityContext {
            game: settings.game,
            distribution: settings.distribution,
            // 设置快照不包含账号.
            account_id: None,
            session_running: activity.game_starting
                || activity.game_running
                || activity.game_closing,
            launcher_present: self.official_launcher_root().is_some(),
        })
    }
}
