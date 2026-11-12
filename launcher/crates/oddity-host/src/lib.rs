//! 宿主内核与全部宿主域: 服务循环, 命令分发, 会话, 安装适配与各领域实现.
//!
//! 只依赖平台原子能力与运行时契约, 不依赖任何游戏包. 游戏包经装配表由
//! bin crate 注入 [`service::ServiceConfig`]; 宿主按字符串 id 从
//! [`oddity_runtime::Registry`] 取用安装域, 启动实现与长驻域.

pub mod channel;
pub mod cli;
pub mod protocol;
pub mod service;

pub(crate) mod bootstrap;
pub(crate) mod host;

pub(crate) mod audio_commands;
pub mod environment_commands;
pub(crate) mod external_launch;
pub(crate) mod file_commands;
pub(crate) mod game_mode_commands;
pub(crate) mod gamepad_commands;
pub(crate) mod install;
pub(crate) mod integrity_commands;
pub(crate) mod log_commands;
pub(crate) mod migration_commands;
pub mod paths;
pub mod performance_commands;
pub(crate) mod playtime_commands;
pub(crate) mod runtime_setup_commands;
pub mod session;
pub mod snapshot_commands;

pub mod environment;
pub mod game_mode;
pub mod gamepad;
pub mod identity;
pub mod integrity;
pub mod logs;
pub mod performance;
pub mod playtime;
pub mod recovery;
pub mod runtime_setup;
pub mod settings;
pub mod snapshots;
pub mod updates;

pub use service::AppState;

/// 当前 Unix 毫秒; 会话与时序记录共用.
pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;
