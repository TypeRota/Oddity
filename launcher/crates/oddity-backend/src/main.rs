//! Oddity 后端入口: 参数判定, 装配校验与包的无头入口.
//!
//! 常驻服务的启动顺序, 会话与全部命令实现都在 oddity-host; 这里只装配包.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

mod assembly;

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let registry = assembly::build();
    for runtime in registry.runtimes() {
        if let Some(code) = runtime.headless_entry(&args) {
            std::process::exit(code);
        }
    }
    if oddity_host::cli::help_requested(&args) {
        println!("{}", oddity_host::cli::USAGE);
        return;
    }
    if !args.iter().any(|arg| arg == "--ipc") {
        eprintln!("请通过 oddity 启动图形界面.");
        std::process::exit(2);
    }
    if let Err(error) = assembly::check(&registry) {
        eprintln!("命令装配校验失败: {error}");
        std::process::exit(1);
    }
    if let Err(error) = assembly::check_capabilities(&registry) {
        eprintln!("能力装配校验失败: {error}");
        std::process::exit(1);
    }
    let compatible = args.iter().any(|arg| arg == "--recovery-compatible");
    oddity_host::service::run(Arc::new(registry), compatible);
}
