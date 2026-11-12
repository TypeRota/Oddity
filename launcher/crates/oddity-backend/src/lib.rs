//! 后端装配的共享入口: `oddity-backend` 与 `nte-cli` 使用同一份装配表.
//!
//! 常驻服务的启动顺序, 会话与全部命令实现都在 oddity-host; 这里只导出
//! 装配模块, 供两个二进制按同一份包列表构造注册表.

// lib target 只使用 [`assembly::build`]; 启动校验由二进制目标使用同一模块.
#[allow(dead_code)]
pub mod assembly;
