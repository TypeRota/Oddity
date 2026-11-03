use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use nte_core::config::LauncherConfig;
use nte_core::game;
use nte_core::game::check::CheckOptions;
use nte_core::game::download::ApplyOptions;
use nte_core::game::optional::OptionalSelection;
use nte_core::manifest::Manifest;
use nte_core::{update, updates, version};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().cloned() else {
        return usage();
    };
    if command == "recovery" {
        return recovery(&args[1..]);
    }
    if command == "game-mode-gpu-apply" && args.len() == 1 {
        return nte_core::game_mode::gpu::apply_system_json(std::io::stdin());
    }
    if command == "game-mode-tuning-apply" && args.len() == 1 {
        return nte_core::game_mode::tuning::apply_system_json(std::io::stdin());
    }
    if command == "keyring-check" {
        nte_core::attendance::check_keyring()?;
        println!("Keyring 存取与清理检查通过");
        return Ok(());
    }
    if command == "attendance" {
        let dir = flag_value(&args, "--config")
            .map(PathBuf::from)
            .unwrap_or_else(nte_core::attendance::default_dir);
        let status = nte_core::attendance::Service::new(dir).run(None)?;
        if status.accounts.is_empty() {
            anyhow::bail!("请先登录塔吉多账号");
        }
        let mut failed = false;
        for account in status.accounts {
            if let Some(error) = &account.attention {
                eprintln!("{}: {}", account.masked_phone, error);
                failed = true;
            }
            if account.signed_today {
                println!("{}: 今日已签到", account.masked_phone);
            } else {
                failed = true;
                println!("{}: 今日未签到", account.masked_phone);
            }
            if let Some(report) = &account.last_report {
                for step in &report.steps {
                    println!("{}: {}: {}", account.masked_phone, step.label, step.message);
                }
            }
        }
        if failed {
            anyhow::bail!("签到未全部成功");
        }
        return Ok(());
    }
    let root = flag_value(&args, "--root")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing --root <launcher install dir>"))?;
    let config = LauncherConfig::load(&root)
        .with_context(|| format!("load {}/Config/Config.ini", root.display()))?;

    match command.as_str() {
        "check" => check(&config, args.contains(&"--quick".to_string())),
        "apply" => apply(&config, flag_value(&args, "--only")),
        "doctor" => doctor(&config),
        "game-resources" => game_resources(&config, &args),
        "game-check" => game_check(&config, &args),
        "game-apply" => game_apply(&config, &args),
        "game-preload" => game_download(&config, &args, true),
        _ => usage(),
    }
}

fn check(config: &LauncherConfig, quick: bool) -> Result<()> {
    println!("[local]  version={} build={}", config.version, config.build);
    let remote = version::fetch(&config.version_url).context("fetch Version.ini")?;
    println!("[remote] version={} build={}", remote.version, remote.build);
    let url = remote
        .file_list_url
        .clone()
        .ok_or_else(|| anyhow!("remote Version.ini has no FileListURL"))?;
    let manifest = Manifest::from_url(&url).context("fetch AllFiles.xml")?;
    println!(
        "[manifest] product={:?} files={} base={:?}",
        manifest.product_version,
        manifest.files.len(),
        manifest.base_url
    );
    let report = update::check(&config.root, &manifest, quick)?;
    println!(
        "[check] ok={} missing={} changed={} bytes={}",
        report.ok,
        report.missing.len(),
        report.changed.len(),
        report.total_missing_bytes()
    );
    for f in report.missing.iter().take(20) {
        println!("  missing {}", f.path);
    }
    for f in report.changed.iter().take(20) {
        println!("  changed {}", f.path);
    }
    Ok(())
}

fn apply(config: &LauncherConfig, only: Option<String>) -> Result<()> {
    let root = config
        .root
        .parent()
        .ok_or_else(|| anyhow!("启动器目录没有安装根目录"))?;
    let _install_lock = nte_core::install_lock::InstallLock::acquire(root)?;
    let remote = version::fetch(&config.version_url).context("fetch Version.ini")?;
    let url = remote
        .file_list_url
        .clone()
        .ok_or_else(|| anyhow!("remote Version.ini has no FileListURL"))?;
    let base_dir = url
        .rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or(url.clone());
    let manifest = Manifest::from_url(&url).context("fetch AllFiles.xml")?;
    let report = update::check(&config.root, &manifest, false)?;
    let mut todo: Vec<_> = report.missing.into_iter().chain(report.changed).collect();
    if let Some(filter) = only {
        todo.retain(|f| f.path.contains(&filter));
    }
    println!("[apply] files={}", todo.len());
    let mut failed = 0;
    for f in &todo {
        match update::apply_file(&config.root, &base_dir, f) {
            Ok(n) => println!("  ok {} <- {} B", f.path, n),
            Err(e) => {
                failed += 1;
                println!("  FAIL {}: {e}", f.path);
            }
        }
    }
    if failed > 0 {
        anyhow::bail!("{failed} launcher update file(s) failed");
    }
    Ok(())
}

fn game_config(config: &LauncherConfig) -> Result<game::GameUpdateConfig> {
    game::GameUpdateConfig::load(&config.root, config.game_id).with_context(|| {
        format!(
            "load PatcherConfig under {}",
            config.root.join("ResFilesM").display()
        )
    })
}

fn optional_selection(args: &[String]) -> Result<OptionalSelection> {
    let mut tags = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        if argument == "--optional" {
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| anyhow!("--optional requires a resource tag"))?;
            tags.push(value.clone());
        }
    }
    if args.iter().any(|arg| arg == "--include-optional") {
        if !tags.is_empty() {
            anyhow::bail!("--include-optional cannot be combined with --optional");
        }
        Ok(OptionalSelection::All)
    } else if tags.is_empty() {
        Ok(OptionalSelection::Configured)
    } else {
        Ok(OptionalSelection::Tags(tags))
    }
}

fn game_resources(launcher: &LauncherConfig, args: &[String]) -> Result<()> {
    let mut config = game_config(launcher)?;
    let bundle = if args.iter().any(|arg| arg == "--preload") {
        let Some(remote) = game::check::load_preload_remote(&config)? else {
            println!("[resources] no resource preload available");
            return Ok(());
        };
        config = remote.config;
        remote.bundle
    } else {
        game::check::load_remote(&config)?
    };
    let selection = optional_selection(args)?;
    let clear = args.iter().any(|arg| arg == "--clear-selection");
    let tags = match selection {
        OptionalSelection::Configured if clear => Some(Vec::new()),
        OptionalSelection::Configured => None,
        OptionalSelection::Tags(tags) if !clear => Some(tags),
        OptionalSelection::All if !clear => Some(
            game::optional::catalog(&config, &bundle)?
                .into_iter()
                .map(|resource| resource.tag)
                .collect(),
        ),
        _ => anyhow::bail!(
            "--clear-selection cannot be combined with an optional resource selection"
        ),
    };
    if let Some(tags) = tags {
        let _lock = nte_core::install_lock::InstallLock::acquire(&config.install_root)?;
        game::optional::save_selection(&config, &bundle, &tags)?;
    }
    println!(
        "[resources] version={} branch={}",
        bundle.config.res_version, config.branch
    );
    for resource in game::optional::catalog(&config, &bundle)? {
        println!(
            "{} {} version={} bytes={} state={}",
            if resource.selected { "[x]" } else { "[ ]" },
            resource.tag,
            resource.version,
            resource.size,
            serde_json::to_string(&resource.state)?
        );
    }
    Ok(())
}

fn game_check(launcher: &LauncherConfig, args: &[String]) -> Result<()> {
    let cfg = game_config(launcher)?;
    let opts = CheckOptions {
        full: args.iter().any(|a| a == "--full"),
        optional: optional_selection(args)?,
    };
    let plan = game::check::check(&cfg, opts.clone()).context("check game resources")?;
    println!(
        "[game] local={} remote={} hash={} branch={} mode={}",
        if plan.local_version.is_empty() {
            "?"
        } else {
            &plan.local_version
        },
        plan.remote_version,
        plan.remote_hash,
        cfg.branch,
        if opts.full { "full" } else { "quick" }
    );
    println!(
        "[plan] checked={} update={} unverified={} download_bytes={}",
        plan.checked,
        plan.items.len(),
        plan.unverified.len(),
        plan.total_bytes()
    );
    for item in &plan.items {
        let patch = if item.patches.is_empty() {
            String::new()
        } else {
            let steps = item
                .patches
                .iter()
                .map(|patch| format!("{}.{}", patch.md5, patch.size))
                .collect::<Vec<_>>()
                .join("+");
            format!(" patch={steps}")
        };
        println!(
            "  {:<9} {:>12}  {}{}",
            item.reason.as_str(),
            item.size,
            item.path,
            patch
        );
    }
    for item in plan.unverified.iter().take(40) {
        println!(
            "  {:<9} {:>12}  {}  (use --full to hash)",
            item.reason.as_str(),
            item.size,
            item.path
        );
    }
    if plan.unverified.len() > 40 {
        println!("  ... {} more unverified", plan.unverified.len() - 40);
    }
    if plan.items.is_empty() {
        if plan.unverified.is_empty() {
            println!("[plan] game resources are up to date");
        } else {
            println!(
                "[plan] no updates; {} file(s) unverified (use --full to hash)",
                plan.unverified.len()
            );
        }
    }
    Ok(())
}

fn game_apply(launcher: &LauncherConfig, args: &[String]) -> Result<()> {
    game_download(launcher, args, false)
}

fn game_download(launcher: &LauncherConfig, args: &[String], preload: bool) -> Result<()> {
    let mut cfg = game_config(launcher)?;
    let _install_lock = if args.iter().any(|arg| arg == "--dry-run") {
        None
    } else {
        Some(nte_core::install_lock::InstallLock::acquire(
            &cfg.install_root,
        )?)
    };
    let future_bundle = if preload {
        let Some(remote) =
            game::check::load_preload_remote(&cfg).context("discover resource preload")?
        else {
            println!("[preload] no resource preload available");
            return Ok(());
        };
        cfg = remote.config;
        Some(remote.bundle)
    } else {
        None
    };
    let selection = optional_selection(args)?;
    let check = |full| {
        let opts = CheckOptions {
            full,
            optional: selection.clone(),
        };
        match &future_bundle {
            Some(bundle) => game::check::check_bundle(&cfg, bundle, opts),
            None => game::check::check(&cfg, opts),
        }
    };
    let mut plan = check(args.iter().any(|a| a == "--full")).context("check game resources")?;
    if !plan.unverified.is_empty() {
        plan = check(true).context("verify game resources")?;
    }
    let opts = ApplyOptions {
        dry_run: args.iter().any(|a| a == "--dry-run"),
        only: flag_value(args, "--only"),
        force: args.iter().any(|a| a == "--force"),
        update_ledger: !preload,
        probe_cdns: !updates::metered().is_metered(),
        preload,
    };
    let operation = if preload { "preload" } else { "apply" };
    let selected: Vec<_> = plan
        .items
        .iter()
        .filter(|i| match &opts.only {
            Some(f) => i.path.contains(f),
            None => true,
        })
        .collect();
    let selected_bytes: u64 = selected.iter().map(|i| i.download_size).sum();
    if selected.is_empty() && !plan.items.is_empty() {
        println!("[{operation}] no files match the selected filter");
        return Ok(());
    }
    if opts.dry_run {
        println!(
            "[{operation}] dry-run: {} files, {} bytes",
            selected.len(),
            selected_bytes
        );
        for item in &selected {
            println!("  {} ({} B)", item.path, item.download_size);
        }
        return Ok(());
    }
    println!(
        "[{operation}] files={} bytes={} branch={}",
        selected.len(),
        selected_bytes,
        cfg.branch
    );
    let report = game::download::apply_plan(&cfg, &plan, &opts);
    for r in &report.results {
        if r.ok {
            if r.skipped {
                println!("  skip {}", r.path);
            } else {
                println!("  ok   {} ({} B downloaded)", r.path, r.bytes);
            }
        } else {
            println!(
                "  FAIL {}: {}",
                r.path,
                r.error.as_deref().unwrap_or("unknown")
            );
        }
    }
    println!(
        "[{operation}] ok={} failed={} skipped={} downloaded={} B",
        report.results.iter().filter(|r| r.ok).count(),
        report.failed,
        report.skipped,
        report.downloaded_bytes
    );
    if let Some(e) = &report.ledger_error {
        return Err(anyhow!("ledger update failed: {e}"));
    }
    if report.failed > 0 {
        return Err(anyhow!("{} game update file(s) failed", report.failed));
    }
    if preload && opts.only.is_none() && !report.cancelled {
        game::preload::mark_complete(
            &cfg,
            &cfg.branch,
            &plan.remote_version,
            plan.items.len(),
            plan.total_bytes(),
        )
        .context("write preload completion marker")?;
    }
    Ok(())
}

fn recovery(args: &[String]) -> Result<()> {
    use nte_core::{recovery, settings::UserPaths};
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

fn doctor(config: &LauncherConfig) -> Result<()> {
    println!("root       = {}", config.root.display());
    println!("launcher   = {}", config.launcher);
    println!("patcher    = {}", config.patcher);
    println!("client     = {}", config.client);
    println!("game id    = {}", config.game_id);
    println!("version    = {} ({})", config.version, config.build);
    println!("version url= {}", config.version_url);
    println!("branch     = {}", config.branch);
    println!("game dir   = {}", config.game_dir().display());
    println!("ignored    = {:?}", config.patcher_ignored_sections);
    println!("mutex      = {}", config.mutex_name);
    Ok(())
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn usage() -> Result<()> {
    println!("nte-cli recovery [diagnose|export <目录>|reset-interface --confirm [--reset-invalid]|stop --confirm|start-compatible]");
    println!("nte-cli attendance [--config <签到配置目录>]\nnte-cli keyring-check");
    println!("nte-cli <check|apply|doctor|game-resources|game-check|game-apply|game-preload> --root <launcher install dir> [--quick] [--full] [--optional <tag> ... | --include-optional] [--preload] [--clear-selection] [--only <substr>] [--dry-run] [--force]");
    Ok(())
}

#[cfg(test)]
mod tests;
