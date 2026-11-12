//! 能力声明: 稳定 id, 精确 revision, 覆盖维度与纯可用性函数.
//!
//! 能力职责:
//! `oddity.account.game` 是官方客户端账号 (OneSDK/Laohu 令牌, 用于直接启动),
//! `oddity.profile.player` 是塔吉多社区账号与角色面板 (独立 vault).
//! 两者的登录态, 凭据位置与失效条件都不同, 不能合并成一个账号能力.
//!
//! `Capability` 持有声明与可用性函数, 不持有生命周期资源; 已发起的任务与会话
//! 由 `PackageRuntime` 管理.

use std::fmt;

/// 能力 id: `oddity.<family>.<name>`, 三段, 全 ASCII 小写.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityId(pub &'static str);

impl CapabilityId {
    /// 装配期校验用; 不满足时 `validate_capabilities` 报 `InvalidId`.
    pub const fn is_well_formed(self) -> bool {
        const PREFIX: &[u8] = b"oddity.";
        let bytes = self.0.as_bytes();
        if bytes.len() <= PREFIX.len() {
            return false;
        }
        let mut index = 0;
        while index < PREFIX.len() {
            if bytes[index] != PREFIX[index] {
                return false;
            }
            index += 1;
        }
        let mut dots = 0;
        let mut segment_len = 0;
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'.' {
                if segment_len == 0 {
                    return false;
                }
                dots += 1;
                segment_len = 0;
            } else if byte.is_ascii_lowercase() {
                segment_len += 1;
            } else {
                return false;
            }
            index += 1;
        }
        segment_len != 0 && dots == 2
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// 精确版本; 只做相等断言, 不做区间.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapabilityRevision(pub u16);

impl fmt::Display for CapabilityRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// 能力声明覆盖的上下文维度.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope(u8);

impl Scope {
    pub const NONE: Self = Self(0);
    pub const GAME: Self = Self(1 << 0);
    pub const DISTRIBUTION: Self = Self(1 << 1);
    pub const ACCOUNT: Self = Self(1 << 2);
    pub const SESSION: Self = Self(1 << 3);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// self 是否覆盖 other.
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return formatter.write_str("none");
        }
        let mut first = true;
        for (scope, name) in [
            (Self::GAME, "game"),
            (Self::DISTRIBUTION, "distribution"),
            (Self::ACCOUNT, "account"),
            (Self::SESSION, "session"),
        ] {
            if !self.contains(scope) {
                continue;
            }
            if !first {
                formatter.write_str("|")?;
            }
            first = false;
            formatter.write_str(name)?;
        }
        Ok(())
    }
}

/// provider 的静态声明.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub revision: CapabilityRevision,
    pub scope: Scope,
}

/// 依赖是必需还是可选.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Necessity {
    Required,
    Optional,
}

/// 消费方的静态依赖声明.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityRequirement {
    pub id: CapabilityId,
    pub revision: CapabilityRevision,
    pub necessity: Necessity,
    pub scope: Scope,
}

impl CapabilityRequirement {
    /// 必需依赖; scope 取 provider 声明的全集.
    pub const fn required(descriptor: CapabilityDescriptor) -> Self {
        Self {
            id: descriptor.id,
            revision: descriptor.revision,
            necessity: Necessity::Required,
            scope: descriptor.scope,
        }
    }

    /// 可选依赖; 缺失时消费者自行降级.
    pub const fn optional(descriptor: CapabilityDescriptor) -> Self {
        Self {
            id: descriptor.id,
            revision: descriptor.revision,
            necessity: Necessity::Optional,
            scope: descriptor.scope,
        }
    }
}

/// 宿主/包在边界构造的只读快照; 只在装配与命令边界构造, 不长期持有.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityContext {
    pub game: String,
    pub distribution: String,
    pub account_id: Option<String>,
    pub session_running: bool,
    pub launcher_present: bool,
}

/// 可用性三态.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// 当前上下文下提供.
    Available,
    /// 该游戏/区服不提供这个能力; UI 不挂载, 不显示占位.
    Unsupported,
    /// 提供, 但此刻不可进入; 保留入口并显示 code 对应的包内文案.
    Unavailable {
        code: &'static str,
        message: &'static str,
    },
}

/// 命令边界的稳定错误; 文案由 provider 或调用方提供.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityFailure {
    pub code: &'static str,
    pub reason: Option<&'static str>,
    pub message: &'static str,
}

impl Availability {
    /// Available -> Ok; Unsupported -> 调用方文案;
    /// Unavailable -> provider 的 code, reason 与文案.
    pub fn into_gate(self, unsupported: &'static str) -> Result<(), CapabilityFailure> {
        match self {
            Self::Available => Ok(()),
            Self::Unsupported => Err(CapabilityFailure {
                code: "unsupported",
                reason: None,
                message: unsupported,
            }),
            Self::Unavailable { code, message } => Err(CapabilityFailure {
                code: "capability_unavailable",
                reason: Some(code),
                message,
            }),
        }
    }
}

/// 能力 = 声明 + 一个纯可用性函数; 不持有消费者资源, 没有 start/stop.
pub struct Capability {
    pub descriptor: CapabilityDescriptor,
    /// 必须是纯函数: 不读文件, 不读密钥环, 不查网络, 不读全局.
    pub availability: fn(&CapabilityContext) -> Availability,
}

impl Capability {
    pub const fn new(
        descriptor: CapabilityDescriptor,
        availability: fn(&CapabilityContext) -> Availability,
    ) -> Self {
        Self {
            descriptor,
            availability,
        }
    }

    pub fn availability(&self, context: &CapabilityContext) -> Availability {
        (self.availability)(context)
    }
}
