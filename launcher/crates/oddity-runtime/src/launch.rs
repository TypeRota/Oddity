//! 启动会话契约: 宿主解析启动计划, 游戏包提供启动实现.
//!
//! 启动入口命令、会话注册表、安装与 prefix 租约、事件与进程回收兜底属于
//! 宿主; 启动配方、三种模式的命令构造与进程监视属于游戏包. 宿主按目标预留
//! 会话后把 [`SessionTicket`] 交给当前游戏的 [`LaunchProvider`], 票据把会话
//! id 与宿主能力绑在一起, 包只能在这次会话内读取计划、拉起进程与汇报事实,
//! 无法在会话之外调用宿主. 票据没有经过 [`SessionTicket::end`] 就被丢弃时
//! 视为放弃, 宿主据此兜底清理编排线程异常退出的会话.

use serde::Serialize;
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

/// 启动模式.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaunchMode {
    /// 官方启动器前台启动.
    Official,
    /// 后台静默启动.
    Background,
    /// 直启游戏并交接 SDK.
    Direct,
}

impl LaunchMode {
    /// 设置与界面使用的模式名; 与 LaunchResult 的 mode 字段取值一致.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Background => "background",
            Self::Direct => "direct",
        }
    }
}

/// 一次启动的目标.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTarget {
    /// 游戏 id, 与设置里的当前游戏一致.
    pub game: String,
    /// 发行渠道, 例如 cn、global.
    pub distribution: String,
    /// 本次启动的模式; 外部启动的临时覆盖不写回设置.
    pub mode: LaunchMode,
}

/// 已选定的 Proton 版本.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtonRef {
    /// Proton 安装目录, 也就是 PROTONPATH 的取值.
    pub root: PathBuf,
    /// 展示名.
    pub name: String,
    /// 版本身份, 用于快照比对与环境固定校验.
    pub identity: String,
}

/// 包装程序与显示、网络策略的已解析结果.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSpec {
    /// 包装程序, 例如 gamescope 或 umu-run.
    pub program: PathBuf,
    /// 传给包装程序、位于游戏程序之前的参数.
    pub prefix_args: Vec<OsString>,
}

/// 宿主解析完成后交给包的启动计划.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// 启动目标.
    pub target: SessionTarget,
    /// 游戏安装根目录.
    pub install_root: PathBuf,
    /// 官方启动器目录.
    pub launcher_root: PathBuf,
    /// 当前地区游戏可执行文件; direct 模式使用, 未安装时为 None.
    pub game_exe: Option<PathBuf>,
    /// 游戏版本.
    pub game_version: String,
    /// 官方启动器版本.
    pub launcher_version: String,
    /// Wine prefix.
    pub prefix: PathBuf,
    /// 已选定的 Proton.
    pub proton: ProtonRef,
    /// umu-run 路径.
    pub umu: PathBuf,
    /// 启动日志与环境专属日志目录.
    pub log_dir: PathBuf,
    /// 宿主数据目录下的 helpers.
    pub helper_dir: PathBuf,
    /// 包装与显示、网络策略.
    pub runner: RunnerSpec,
    /// 是否由 gamescope 接管.
    pub gamescope: bool,
    /// direct 模式取凭据用的账号 id.
    pub account_id: Option<String>,
}

/// 一次启动的结果; 字段与序列化和界面现有载荷一致.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResult {
    /// 启动模式名, 取值见 [`LaunchMode::as_str`].
    pub mode: String,
    /// 包装进程 id.
    pub pid: u32,
    /// 实际拉起的可执行文件.
    pub exe: String,
    /// Wine prefix.
    pub prefix: String,
    /// 启动日志路径.
    pub log: String,
    /// 启动时间, Unix 毫秒.
    pub started_at: u64,
}

/// 启动的可视化阶段; 前端展示到 Game 为止, Running 用于收窗.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchStage {
    /// 目标与环境准备.
    Preparing,
    /// helper 安装与交接.
    Helper,
    /// 资源校验与更新.
    Resources,
    /// 游戏进程拉起.
    Game,
    /// 游戏已进入运行.
    Running,
}

/// 后台启动的进度; 字段与序列化和界面现有载荷一致.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProgress {
    /// 当前阶段.
    pub stage: LaunchStage,
    /// 进度开始时间, Unix 毫秒.
    pub started_at: u64,
    /// 是否已请求取消.
    pub cancelling: bool,
}

impl LaunchProgress {
    /// 以准备阶段开始一轮进度.
    pub const fn new(started_at: u64) -> Self {
        Self {
            stage: LaunchStage::Preparing,
            started_at,
            cancelling: false,
        }
    }

    /// 阶段只向前推进; 取消期间冻结进度, 避免清单继续跳动.
    pub fn advance(&mut self, stage: LaunchStage) -> bool {
        if self.cancelling || stage <= self.stage {
            return false;
        }
        self.stage = stage;
        true
    }
}

/// 会话状态, 也是 game-state 事件的载荷.
///
/// 字段与序列化和宿主现有状态投影一致; launch 与 error 没有值时不出现,
/// 与界面现有消费方式保持一致.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    /// 会话是否在运行.
    pub running: bool,
    /// 是否正在关闭.
    pub closing: bool,
    /// 包装进程 id.
    pub pid: Option<u32>,
    /// 启动时间, Unix 毫秒.
    pub started_at: Option<u64>,
    /// 实际拉起的可执行文件.
    pub exe: Option<String>,
    /// 启动日志路径.
    pub log: Option<String>,
    /// 准备阶段的进度; 只有后台模式提供.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchProgress>,
    /// 冷启动错误, 由界面取走并提示.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 一轮会话采样的事实.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSample {
    /// 游戏主进程是否仍在.
    pub game_running: bool,
    /// 识别到的游戏进程 id.
    pub game_pids: Vec<u32>,
}

/// 启动失败的分类.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchFailureKind {
    /// 进程拉起失败.
    Spawn,
    /// 拉起后立即退出.
    EarlyExit,
    /// 等待启动完成超时.
    Timeout,
    /// 用户取消.
    Cancelled,
    /// 后端准备或宿主侧失败.
    Backend,
}

/// 一次启动失败.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchFailure {
    /// 失败分类.
    pub kind: LaunchFailureKind,
    /// 给用户看的补充信息, 例如启动日志尾部.
    pub detail: Option<String>,
}

/// 会话结束的事实.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnd {
    /// 游戏是否真的进入过运行.
    pub game_started: bool,
    /// 包装进程退出码; 被信号终止时为 None.
    pub exit_code: Option<i32>,
    /// 启动失败的原因; 正常结束为 None.
    pub failure: Option<LaunchFailure>,
    /// 是否由用户关闭或取消.
    pub forced: bool,
}

/// 进程的结局.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessExit {
    /// 正常退出, 携带退出码.
    Exited(i32),
    /// 被信号终止.
    Signaled,
}

/// 宿主已经拉起的进程句柄.
///
/// 回收由宿主线程完成, 包只读缓存的结果; 即使包线程异常退出, 宿主仍持有
/// 进程组记录, 在会话放弃时负责终止并回收.
pub trait GameProcess: Send + Sync {
    /// 包装进程 id.
    fn pid(&self) -> u32;

    /// 包装进程所在进程组.
    fn pgid(&self) -> i32;

    /// 非阻塞读取结局; 结果内部缓存, 可重复调用.
    fn try_wait(&self) -> Result<Option<ProcessExit>, String>;

    /// 阻塞到进程结束.
    fn wait(&self) -> Result<ProcessExit, String>;
}

/// 包构造好的拉起请求; 进程组、日志与 umask 由宿主补齐.
pub struct SpawnRequest {
    /// 包构造的命令.
    pub command: Command,
    /// stdout 与 stderr 的重定向目标.
    pub log: Option<File>,
    /// 丢弃 stderr, 用于只关心退出码的辅助进程.
    pub stderr_null: bool,
    /// 使用私有 umask, 避免 helper 释放出的文件权限过宽.
    pub private_umask: bool,
}

/// 宿主为当前会话实现的能力, 由 [`SessionTicket`] 委托.
///
/// 实现方在调用包的回调前必须先释放注册表锁: 包的收尾会阻塞并回调
/// [`SessionHost::end`], 持锁回调会形成环.
pub trait SessionHost: Send + Sync + 'static {
    /// 取本次会话的启动计划; 宿主管安装与 prefix 租约、运行环境校验.
    fn plan(&self, id: &str) -> Result<LaunchPlan, LaunchError>;

    /// 让宿主接入 audio、game_mode、performance、snapshot 等准备, 并把协商
    /// 内容写进启动日志.
    fn prepare(
        &self,
        id: &str,
        command: &mut Command,
        log: &mut dyn Write,
    ) -> Result<(), LaunchError>;

    /// 拉起进程组; 宿主随后接管回收, 并在会话放弃时终止.
    fn spawn(&self, id: &str, request: SpawnRequest) -> Result<Arc<dyn GameProcess>, LaunchError>;

    /// 记录已确认的启动事实.
    fn started(&self, id: &str, facts: StartedFacts) -> Result<(), LaunchError>;

    /// 汇报启动阶段.
    fn stage(&self, id: &str, stage: LaunchStage);

    /// 汇报一轮采样.
    fn sample(&self, id: &str, sample: SessionSample);

    /// 游戏进程消失; 宿主据此立即回到窗口.
    fn game_exited(&self, id: &str, exit_code: Option<i32>);

    /// 会话终态; 宿主发终态事件并释放租约.
    fn end(&self, id: &str, end: SessionEnd);

    /// 是否已经请求取消.
    fn cancelled(&self, id: &str) -> bool;

    /// 绑定关闭与取消入口.
    fn bind_control(&self, id: &str, control: Arc<dyn SessionControl>);

    /// 编排线程没有交出终态; 宿主兜底终止并回收.
    fn abandon(&self, id: &str);
}

/// 启动成功后确认的事实.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedFacts {
    /// 实际拉起的可执行文件.
    pub exe: String,
    /// 启动日志路径.
    pub log: String,
}

/// 宿主绑定到会话的关闭与取消入口.
pub trait SessionControl: Send + Sync {
    /// 请求取消; 准备与运行阶段都有效.
    fn cancel(&self);

    /// 阻塞到本轮收尾完成或失败, 可以重试.
    ///
    /// 收尾完成会回调 [`SessionHost::end`], 调用方必须先释放注册表锁.
    fn close(&self) -> Result<(), String>;

    /// 会话是否还活着; 等价现有 session_alive.
    fn is_alive(&self) -> bool;
}

/// 包持有的会话凭据.
///
/// 由宿主在预留会话后创建; 包拿不到 [`SessionHost`], 只能在本次会话内经由
/// 票据调用宿主能力.
pub struct SessionTicket {
    id: String,
    host: Arc<dyn SessionHost>,
    ended: bool,
}

impl SessionTicket {
    /// 宿主预留会话后创建; 游戏包不直接构造.
    pub fn new(id: impl Into<String>, host: Arc<dyn SessionHost>) -> Self {
        Self {
            id: id.into(),
            host,
            ended: false,
        }
    }

    /// 会话 id, 与宿主注册表里的记录一致.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 取启动计划.
    pub fn plan(&self) -> Result<LaunchPlan, LaunchError> {
        self.host.plan(&self.id)
    }

    /// 让宿主接入准备内容并写启动日志.
    pub fn prepare(&self, command: &mut Command, log: &mut dyn Write) -> Result<(), LaunchError> {
        self.host.prepare(&self.id, command, log)
    }

    /// 拉起进程组.
    pub fn spawn(&self, request: SpawnRequest) -> Result<Arc<dyn GameProcess>, LaunchError> {
        self.host.spawn(&self.id, request)
    }

    /// 记录启动事实.
    pub fn started(&self, facts: StartedFacts) -> Result<(), LaunchError> {
        self.host.started(&self.id, facts)
    }

    /// 汇报启动阶段.
    pub fn stage(&self, stage: LaunchStage) {
        self.host.stage(&self.id, stage);
    }

    /// 汇报一轮采样.
    pub fn sample(&self, sample: SessionSample) {
        self.host.sample(&self.id, sample);
    }

    /// 游戏进程消失.
    pub fn game_exited(&self, exit_code: Option<i32>) {
        self.host.game_exited(&self.id, exit_code);
    }

    /// 交出会话终态; 消费票据, 之后不会再触发放弃.
    pub fn end(mut self, end: SessionEnd) {
        self.ended = true;
        self.host.end(&self.id, end);
    }

    /// 是否已经请求取消.
    pub fn cancelled(&self) -> bool {
        self.host.cancelled(&self.id)
    }

    /// 绑定关闭与取消入口.
    pub fn bind_control(&self, control: Arc<dyn SessionControl>) {
        self.host.bind_control(&self.id, control);
    }
}

impl Drop for SessionTicket {
    fn drop(&mut self) {
        if !self.ended {
            self.host.abandon(&self.id);
        }
    }
}

/// 启动错误.
///
/// 命令层把 message 作为字符串返回给界面; code 只用于宿主日志、快照失败
/// 分类与测试断言.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchError {
    /// 错误分类, 与会话结束的失败分类一致.
    pub code: LaunchFailureKind,
    /// 给用户看的错误说明.
    pub message: String,
}

impl LaunchError {
    /// 按分类与说明构造错误.
    pub fn new(code: LaunchFailureKind, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LaunchError {}

/// 游戏包提供的启动实现.
///
/// 宿主已按目标预留会话并分配 [`SessionTicket`]; 返回时游戏要么已进入运行
/// 并移交监视线程, 要么返回错误且会话已由宿主收尾 (预留之前的失败由宿主
/// 处理).
pub trait LaunchProvider: Send + Sync + 'static {
    /// 启动前置检查.
    ///
    /// 宿主在预留会话前调用; 包在这里拒绝不支持的区服或模式. 默认放行.
    fn check(&self, _target: &SessionTarget) -> Result<(), LaunchError> {
        Ok(())
    }

    /// 执行一次启动.
    fn launch(&self, ticket: SessionTicket) -> Result<LaunchResult, LaunchError>;
}
