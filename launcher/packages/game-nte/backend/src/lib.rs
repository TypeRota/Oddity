//! 《异环》游戏包后端.
//!
//! 命令组按领域逐步迁入; 当前公告、官方背景、安装探测、塔吉多签到与
//! 游戏账号由本包处理, 其余命令仍由宿主 dispatch 处理.

use oddity_runtime::{
    Capability, CapabilityDescriptor, CapabilityId, CapabilityRequirement, CapabilityRevision,
    ChannelDescriptor, CommandGroup, DispatchFuture, GamePackage, HeadlessCli, HostContext,
    InstallLocator, LaunchProvider, PackageDescriptor, PackageRuntime, Registry, Scope,
};
use serde_json::Value;

pub mod attendance;
pub mod auth;
mod background;
mod bulletin;
pub mod cli;
mod install;
pub mod launch;
mod runtime;
pub mod update;

#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests;

/// 游戏包 id; 契约与设置里的 game 字段都使用这个值.
pub(crate) const PACKAGE_ID: &str = oddity_contract::PACKAGE_NTE;

/// 官方客户端账号能力: 短信, 登录, 账号列表与直接启动的一次性凭证.
const ACCOUNT_GAME: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.account.game"),
    revision: CapabilityRevision(1),
    scope: Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT),
};

/// 塔吉多社区账号与角色面板能力.
const PLAYER_PROFILE: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.profile.player"),
    revision: CapabilityRevision(1),
    scope: Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT),
};

/// 本包提供的能力 sheet.
static CAPABILITIES: &[Capability] = &[
    Capability::new(ACCOUNT_GAME, auth::availability),
    Capability::new(PLAYER_PROFILE, attendance::availability),
];

/// 本包依赖自己提供的能力; 删掉 provider 即装配失败.
static REQUIREMENTS: &[CapabilityRequirement] = &[
    CapabilityRequirement::required(ACCOUNT_GAME),
    CapabilityRequirement::required(PLAYER_PROFILE),
];

/// 异环的发行渠道; 国服是未选择时的默认值.
static CHANNELS: &[ChannelDescriptor] = &[
    ChannelDescriptor {
        id: "cn",
        label: "国服",
        is_default: true,
        install_root_key: "cn",
    },
    ChannelDescriptor {
        id: "global",
        label: "国际服",
        is_default: false,
        install_root_key: "global",
    },
];

/// 异环游戏包.
pub struct NtePackage;

/// 装配表直接引用的唯一实例.
pub static NTE_PACKAGE: NtePackage = NtePackage;

impl GamePackage for NtePackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: PACKAGE_ID,
            name: "异环",
        }
    }

    fn register(&self, registry: &mut Registry) {
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "bulletin",
            bulletin::COMMANDS,
            bulletin_dispatch,
        ));
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "background",
            background::COMMANDS,
            background_dispatch,
        ));
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "attendance",
            attendance::COMMANDS,
            attendance_dispatch,
        ));
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "auth",
            auth::COMMANDS,
            auth_dispatch,
        ));
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "update",
            update::COMMANDS,
            update_dispatch,
        ));
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "download",
            update::commands::COMMANDS,
            download_dispatch,
        ));
    }

    fn capabilities(&self) -> &'static [Capability] {
        CAPABILITIES
    }

    fn requires(&self) -> &'static [CapabilityRequirement] {
        REQUIREMENTS
    }

    fn channels(&self) -> &'static [ChannelDescriptor] {
        CHANNELS
    }

    fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
        Some(install::locator())
    }

    fn launch_provider(&self) -> Option<&'static dyn LaunchProvider> {
        Some(launch::provider())
    }

    fn runtime(&self) -> Option<&'static dyn PackageRuntime> {
        Some(runtime::runtime())
    }

    fn headless_cli(&self) -> Option<&'static dyn HeadlessCli> {
        Some(cli::headless_cli())
    }
}

/// 公告域分发: 失败载荷沿用宿主的 JSON 形状, 由域内按命令解析参数.
fn bulletin_dispatch<'a>(
    _host: &'a dyn HostContext,
    command: &'a str,
    args: Value,
) -> DispatchFuture<'a> {
    Box::pin(bulletin::command(command, args))
}

/// 背景域分发: 宿主负责定位官方启动器, 参数与结果仍是宿主载荷.
fn background_dispatch<'a>(
    host: &'a dyn HostContext,
    command: &'a str,
    args: Value,
) -> DispatchFuture<'a> {
    Box::pin(background::command(host, command, args))
}

/// 签到域分发: 区服来自宿主只读设置快照, 密钥环与网络由平台能力提供.
fn attendance_dispatch<'a>(
    host: &'a dyn HostContext,
    command: &'a str,
    args: Value,
) -> DispatchFuture<'a> {
    Box::pin(attendance::command(host, command, args))
}

/// 账号域分发: 国服能力门与官方启动器根目录由宿主提供.
fn auth_dispatch<'a>(
    host: &'a dyn HostContext,
    command: &'a str,
    args: Value,
) -> DispatchFuture<'a> {
    Box::pin(auth::command(host, command, args))
}

/// 更新域分发: 只读设置快照, 活动状态与官方启动器根目录由宿主提供.
fn update_dispatch<'a>(
    host: &'a dyn HostContext,
    command: &'a str,
    args: Value,
) -> DispatchFuture<'a> {
    Box::pin(update::command(host, command, args))
}

/// 下载命令组分发: 状态常驻包内, 后台任务经绑定的宿主句柄发事件.
fn download_dispatch<'a>(
    host: &'a dyn HostContext,
    command: &'a str,
    args: Value,
) -> DispatchFuture<'a> {
    Box::pin(update::commands::dispatch(host, command, args))
}
