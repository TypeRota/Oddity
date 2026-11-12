//! NTE 启动域: 启动配方, 三种模式的命令构造, presence 调优与进程监视.
//!
//! 宿主只保留会话注册表与事件; 配方, 命令构造, helper 交接与游戏进程识别
//! 都属于游戏包. 启动实现经票据读取计划并汇报启动事实.

pub mod background;
pub mod direct;
pub mod helper;
mod presence;
mod provider;
mod recipe;
pub mod watch;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub(crate) use provider::provider;
