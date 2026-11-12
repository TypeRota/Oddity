use crate::{
    Capability, CapabilityRequirement, HeadlessCli, InstallLocator, LaunchProvider, PackageRuntime,
    Registry, SettingsSchema,
};
use serde::Serialize;

/// 游戏包的静态身份.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PackageDescriptor {
    /// 包 id, 与协议里的 game 字段一致.
    pub id: &'static str,
    /// 展示名.
    pub name: &'static str,
}

/// 游戏包声明的发行渠道.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDescriptor {
    /// 渠道 id, 与协议参数, 设置值和 CLI `--region` 一致.
    pub id: &'static str,
    /// 展示名, 用于选择器, 错误文案与历史记录.
    pub label: &'static str,
    /// 未选择渠道时使用的默认值; 同一游戏恰好一个.
    pub is_default: bool,
    /// 安装根键; 共享同一安装根的渠道取相同值.
    pub install_root_key: &'static str,
}

/// 可装载的游戏包.
///
/// 包不依赖具体宿主; 需要宿主能力时从 `&dyn HostContext` 读取, 不持有
/// 宿主实现.
pub trait GamePackage: Send + Sync + 'static {
    /// 包的静态声明.
    fn descriptor(&self) -> PackageDescriptor;

    /// 把包内命令组注册进宿主注册表.
    fn register(&self, registry: &mut Registry);

    /// 本包提供的能力 sheet; 不声明能力的包返回空.
    fn capabilities(&self) -> &'static [Capability] {
        &[]
    }

    /// 本包对能力的依赖; 不依赖其它能力的包返回空.
    fn requires(&self) -> &'static [CapabilityRequirement] {
        &[]
    }

    /// 本包声明的发行渠道; 没有渠道概念时返回空.
    fn channels(&self) -> &'static [ChannelDescriptor] {
        &[]
    }

    /// 本包提供的设置 section schema; 没有声明设置时返回空.
    fn settings(&self) -> &'static [SettingsSchema] {
        &[]
    }

    /// 安装位置探测能力; 没有安装位置知识或尚未实现的游戏返回 None.
    fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
        None
    }

    /// 启动实现; 没有启动配方或尚未实现的游戏返回 None.
    fn launch_provider(&self) -> Option<&'static dyn LaunchProvider> {
        None
    }

    /// 包的进程级长驻域; 没有长驻任务的游戏返回 None.
    fn runtime(&self) -> Option<&'static dyn PackageRuntime> {
        None
    }

    /// 包导出的无头 CLI; 只有提供 `nte-cli` 命令的游戏返回 Some.
    ///
    /// 与 [`Self::runtime`] 的 `headless_entry` 分开: 这里只服务 CLI 的命令表
    /// 与 typed 调用, 不参与 backend 启动.
    fn headless_cli(&self) -> Option<&'static dyn HeadlessCli> {
        None
    }
}
