use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use oddity_backend::assembly;
use oddity_host::identity::GameId;
use oddity_runtime::{HeadlessCli, HeadlessRequest, Registry};

fn main() -> Result<()> {
    run(std::env::args().skip(1).collect())
}

/// 解析命令行并执行; 入口只把进程参数转进来, 便于按命令测试.
fn run(args: Vec<String>) -> Result<()> {
    let Some(command) = args.first().cloned() else {
        return usage(&registry());
    };
    if command == "recovery" {
        return recovery(&args[1..]);
    }
    if command == "game-mode-gpu-apply" && args.len() == 1 {
        return oddity_host::game_mode::gpu::apply_system_json(std::io::stdin());
    }
    if command == "game-mode-tuning-apply" && args.len() == 1 {
        return oddity_host::game_mode::tuning::apply_system_json(std::io::stdin());
    }
    let registry = registry();
    // 不依赖安装目录的包命令 (签到与密钥环自检) 先执行, 其余命令再解析公共参数.
    if let Some(cli) = standalone_cli(&registry, &command) {
        return dispatch(cli, &command, None, &args[1..]);
    }
    let root = flag_value(&args, "--root")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing --root <launcher install dir>"))?;
    let game = selectable_game(&args, &registry)?;
    game_command(&registry, &game, &command, &root, &args[1..])
}

/// 按共享装配表构建注册表; 与 backend 使用同一份包列表.
fn registry() -> Registry {
    assembly::build()
}

/// 不需要启动器目录的包命令, 由包自己声明.
fn standalone_cli(registry: &Registry, command: &str) -> Option<&'static dyn HeadlessCli> {
    registry.headless_clis().map(|(_, cli)| cli).find(|cli| {
        cli.commands()
            .iter()
            .any(|candidate| candidate.name == command && !candidate.needs_root)
    })
}

/// `--game` 只接受已装配且提供安装域的游戏; 省略时用默认 id.
fn selectable_game(args: &[String], registry: &Registry) -> Result<GameId> {
    let known = registry.selectable_games();
    let requested = flag_value(args, "--game").unwrap_or_else(|| GameId::default().to_string());
    GameId::parse_known(&requested, &known).map_err(|error| anyhow!(error))
}

/// 更新命令由包实现; 游戏 id 与包 id 不一致时说明这个包没有无头入口.
fn game_command(
    registry: &Registry,
    game: &GameId,
    command: &str,
    root: &Path,
    args: &[String],
) -> Result<()> {
    // 未知命令先回落 usage; 只有包声明的更新命令才继续检查游戏入口.
    let known = registry.headless_clis().any(|(_, cli)| {
        cli.commands()
            .iter()
            .any(|candidate| candidate.name == command && candidate.needs_root)
    });
    if !known {
        return usage(registry);
    }
    let Some(cli) = registry.headless_cli(game.as_str()) else {
        return Err(anyhow!("《{game}》没有无头更新命令"));
    };
    if !cli
        .commands()
        .iter()
        .any(|candidate| candidate.name == command && candidate.needs_root)
    {
        return usage(registry);
    }
    dispatch(cli, command, Some(root), args)
}

/// 把命令交给包执行; 失败信息按终端输出约定转成 anyhow 错误.
fn dispatch(
    cli: &'static dyn HeadlessCli,
    command: &str,
    root: Option<&Path>,
    args: &[String],
) -> Result<()> {
    cli.run(&HeadlessRequest {
        command,
        root,
        args,
    })
    .map_err(|message| anyhow!(message))
}

fn recovery(args: &[String]) -> Result<()> {
    use oddity_host::{recovery, settings::UserPaths};
    let paths = UserPaths::discover();
    match args.first().map(String::as_str).unwrap_or("diagnose") {
        "diagnose" if args.len() <= 1 => print!("{}", recovery::diagnose(&paths)),
        "export" if args.len() == 2 => println!("{}", recovery::export_diagnostics(&paths, std::path::Path::new(&args[1]))?.display()),
        "reset-interface" if args == ["reset-interface", "--confirm"] || args == ["reset-interface", "--confirm", "--reset-invalid"] => {
            let backup = recovery::reset_interface(&paths, args.iter().any(|a| a == "--reset-invalid"))?;
            println!("界面设置已恢复. 备份: {}", backup.map_or_else(|| "原配置不存在".into(), |p| p.display().to_string()));
        }
        "stop" if args == ["stop", "--confirm"] => {
            recovery::stop_launcher(&paths)?;
            println!("启动器已退出");
        }
        "start-compatible" if args.len() == 1 => { println!("已启动兼容模式, PID={}", recovery::start_compatible(&paths)?.id()); }
        _ => anyhow::bail!("用法: nte-cli recovery [diagnose|export <目录>|reset-interface --confirm [--reset-invalid]|stop --confirm|start-compatible]\nstop 会终止启动器, 可能中断正在进行的任务. 重建损坏配置后需重新选择安装目录和 Proton, 原配置保存在备份中."),
    }
    Ok(())
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn usage(registry: &Registry) -> Result<()> {
    println!("nte-cli recovery [diagnose|export <目录>|reset-interface --confirm [--reset-invalid]|stop --confirm|start-compatible]");
    for (_, cli) in registry.headless_clis() {
        for line in cli.usage() {
            println!("nte-cli {line}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
