//! 测试用游戏包后端.
//!
//! 包身份与命令都是假的, 真正验证的是装配、契约声明、命令组路由、能力与
//! 设置 schema: 宿主只按声明工作, 不需要认识 fixture.

use oddity_runtime::{
    Availability, Capability, CapabilityContext, CapabilityDescriptor, CapabilityId,
    CapabilityRequirement, CapabilityRevision, CommandGroup, DispatchFuture, GamePackage,
    HostContext, PackageDescriptor, Registry, Scope, SettingsSchema,
};
use serde_json::Value;

mod fixture;
mod settings;

#[cfg(test)]
mod tests;

/// 包 id; 命令组与契约里的 game 字段都使用这个值.
const PACKAGE_ID: &str = oddity_contract::PACKAGE_FIXTURE;

/// fixture 游戏包.
pub struct FixturePackage;

/// fixture 设置分区依赖的能力; 只在本游戏激活时提供.
const SETTINGS_CAPABILITY: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.fixture.settings"),
    revision: CapabilityRevision(1),
    scope: Scope::GAME,
};

static CAPABILITIES: &[Capability] = &[Capability::new(SETTINGS_CAPABILITY, settings_availability)];

static REQUIREMENTS: &[CapabilityRequirement] =
    &[CapabilityRequirement::required(SETTINGS_CAPABILITY)];

/// 能力只在 fixture 游戏上下文内可用.
fn settings_availability(context: &CapabilityContext) -> Availability {
    if context.game == PACKAGE_ID {
        Availability::Available
    } else {
        Availability::Unsupported
    }
}

/// 装配表直接引用的唯一实例.
pub static FIXTURE_PACKAGE: FixturePackage = FixturePackage;

impl GamePackage for FixturePackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: PACKAGE_ID,
            name: "测试游戏",
        }
    }

    fn register(&self, registry: &mut Registry) {
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "fixture",
            fixture::PROBE_COMMANDS,
            dispatch,
        ));
        registry.register(CommandGroup::for_game(
            PACKAGE_ID,
            "fixture-info",
            fixture::INFO_COMMANDS,
            dispatch,
        ));
    }

    fn capabilities(&self) -> &'static [Capability] {
        CAPABILITIES
    }

    fn requires(&self) -> &'static [CapabilityRequirement] {
        REQUIREMENTS
    }

    fn settings(&self) -> &'static [SettingsSchema] {
        settings::SCHEMAS
    }
}

/// 两个命令组共用同一套 JSON 载荷分发.
fn dispatch<'a>(_host: &'a dyn HostContext, command: &'a str, args: Value) -> DispatchFuture<'a> {
    Box::pin(fixture::command(command, args))
}
