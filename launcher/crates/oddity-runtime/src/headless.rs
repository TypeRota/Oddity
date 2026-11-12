//! 游戏包的无头 CLI 表面: 命令表, 用法行与 typed 调用.
//!
//! 与 [`PackageRuntime::headless_entry`](crate::PackageRuntime::headless_entry)
//! 的分工: `headless_entry` 拦截 backend 进程和 systemd 单元的整段 argv,
//! 处理与启动器生命周期绑定的无头执行; [`HeadlessCli`] 只描述 `nte-cli`
//! 的命令与调用, 由 CLI 壳层按命令名分发.
//!
//! 命令语义留在包内: 壳层只负责识别命令属于谁, 解析公共参数并补齐用法行.

use std::path::Path;

/// 一条无头子命令.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadlessCommand {
    /// 命令名, 与 `nte-cli` 的第一个参数一致.
    pub name: &'static str,
    /// 是否由壳层先解析 `--root <启动器目录>` 与 `--game <游戏 id>`.
    ///
    /// 签到与密钥环自检不依赖安装目录, 直接执行.
    pub needs_root: bool,
}

/// 壳层解析好的无头调用.
#[derive(Debug, Clone, Copy)]
pub struct HeadlessRequest<'a> {
    /// 命令名.
    pub command: &'a str,
    /// 启动器安装目录; 只有 [`HeadlessCommand::needs_root`] 命令会给出.
    pub root: Option<&'a Path>,
    /// 命令名之后的原始参数, 由包按自己的命令语义解析.
    pub args: &'a [String],
}

/// 包导出的无头 CLI 入口.
///
/// 壳层按 [`Self::commands`] 判断命令认领关系, 按 [`Self::usage`] 生成用法,
/// 不在前端写包 id 分支. 失败信息按终端输出约定返回, 由壳层打印到 stderr.
pub trait HeadlessCli: Send + Sync + 'static {
    /// 本包支持的命令.
    fn commands(&self) -> &'static [HeadlessCommand];

    /// 用法行; 每行不含 `nte-cli ` 前缀, 由壳层补齐.
    fn usage(&self) -> &'static [&'static str];

    /// 执行一条命令.
    fn run(&self, request: &HeadlessRequest<'_>) -> Result<(), String>;
}
