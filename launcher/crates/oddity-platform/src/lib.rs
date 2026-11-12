//! Oddity 平台原子能力: 与游戏无关的 HTTP, 文件, 哈希, 归档, 差分, INI 解析,
//! 目录树, Wine 路径改写, 音频控制, 网络计费, 系统时钟, 进程, 锁, 密钥环与桌面通知.
//!
//! 只依赖操作系统与通用库, 不依赖任何游戏或宿主 crate.

pub mod archive;
pub mod audio;
pub mod clock;
pub mod diff;
pub mod disk;
pub mod error;
pub mod fs_tree;
pub mod hash;
pub mod http;
pub mod ini;
pub mod install_lock;
pub mod io;
pub mod keyring;
pub mod network;
pub mod notifications;
pub mod paths;
pub mod process;
pub mod throttle;
pub mod version;
pub mod wine;

#[cfg(test)]
mod testing;

pub use error::{Error, Result};

pub const USER_AGENT: &str = "Oddity/0.1 (+linux)";
