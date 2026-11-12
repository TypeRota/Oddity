//! 游戏包装载、命令组注册与事件出口的骨架.
//!
//! 宿主从静态装配表加载 [`GamePackage`], 游戏包把 [`CommandGroup`] 注册进
//! [`Registry`]. 运行时只负责收集与校验; 宿主能力经 [`HostContext`] 以
//! trait object 传入, 命令载荷固定为 JSON; 事件统一经 [`EventSink`] 发送,
//! 后台任务经 [`Task`] 提交并取回结局. 安装位置探测, 启动实现, 无头 CLI
//! 与进程级长驻域由包以静态接口提供, 宿主按游戏 id 从注册表取用.

mod capability;
mod events;
mod group;
mod headless;
mod host;
mod install;
mod launch;
mod package;
mod registry;
mod runtime;
mod settings;
mod task;
mod tasks;

pub use capability::{
    Availability, Capability, CapabilityContext, CapabilityDescriptor, CapabilityFailure,
    CapabilityId, CapabilityRequirement, CapabilityRevision, Necessity, Scope,
};
pub use events::EventSink;
pub use group::{CommandDispatch, CommandGroup, DispatchFuture, DispatchOutcome};
pub use headless::{HeadlessCli, HeadlessCommand, HeadlessRequest};
pub use host::{
    ActivitySnapshot, Cancellation, HostContext, MeteredPolicy, SettingsSnapshot,
    UpdatePreferences, UpdateWindow,
};
pub use install::{InstallLocator, InstallQuery, LauncherInfo};
pub use launch::{
    GameProcess, LaunchError, LaunchFailure, LaunchFailureKind, LaunchMode, LaunchPlan,
    LaunchProgress, LaunchProvider, LaunchResult, LaunchStage, ProcessExit, ProtonRef, RunnerSpec,
    SessionControl, SessionEnd, SessionHost, SessionSample, SessionStatus, SessionTarget,
    SessionTicket, SpawnRequest, StartedFacts,
};
pub use package::{ChannelDescriptor, GamePackage, PackageDescriptor};
pub use registry::{CapabilityIssue, Registry, ValidationIssue};
pub use runtime::{PackageGate, PackageRuntime};
pub use settings::{
    SettingsField, SettingsFieldKind, SettingsMigrate, SettingsSchema, SettingsValidate,
};
pub use task::{ProgressSink, Task, TaskContext, TaskJob, TaskOutcome, TaskProgress, TaskWait};
pub use tasks::TaskRegistry;

#[cfg(test)]
mod tests;
