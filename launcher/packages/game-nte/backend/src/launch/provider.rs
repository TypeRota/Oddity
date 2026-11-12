//! 《异环》启动实现: 三种模式的命令构造, 进程拉起与监视线程.
//!
//! 宿主预留会话后交出票据, 这里只通过票据读取计划并汇报事实; 关闭与取消
//! 由宿主经绑定的控制入口调用. 监视线程沿用 std 线程, 采样驱动宿主管家.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oddity_platform::paths::windows_path;
use oddity_platform::process;
use oddity_runtime::{
    GameProcess, LaunchError, LaunchFailure, LaunchFailureKind, LaunchMode, LaunchProvider,
    LaunchResult, LaunchStage, ProcessExit, SessionControl, SessionEnd, SessionSample,
    SessionTarget, SessionTicket, SpawnRequest, StartedFacts,
};

use crate::auth::{Client, Vault};
use crate::launch::background::{self, NativeSession};
use crate::launch::direct::{self as direct_session, GameSdkPaths};
use crate::launch::helper;
use crate::launch::presence;
use crate::launch::recipe::{configure_display, configure_network, NTE_LAUNCH, OFFICIAL_ARGS};
use crate::launch::watch::GameProcessWatch;
use crate::update::region::{Region, LAYOUT};

/// 准备阶段被取消时的提示; 与宿主提前取消的文案一致.
const CANCELLED: &str = "启动已暂停。";

/// 装配表引用的唯一启动实现.
static PROVIDER: NteLaunchProvider = NteLaunchProvider;

/// 交给宿主的启动实现.
pub(crate) fn provider() -> &'static dyn LaunchProvider {
    &PROVIDER
}

struct NteLaunchProvider;

fn backend(message: impl std::fmt::Display) -> LaunchError {
    LaunchError::new(LaunchFailureKind::Backend, message.to_string())
}

impl LaunchProvider for NteLaunchProvider {
    fn check(&self, target: &SessionTarget) -> Result<(), LaunchError> {
        // 直启需要 OneSDK 凭据, 只有国服受支持; 其余模式不受区服限制.
        if target.mode == LaunchMode::Direct {
            crate::auth::require_supported(&target.game, &target.distribution).map_err(backend)?;
        }
        Ok(())
    }

    fn launch(&self, ticket: SessionTicket) -> Result<LaunchResult, LaunchError> {
        let plan = ticket.plan()?;
        let region = Region::parse(&plan.target.distribution)
            .ok_or_else(|| backend("《异环》不支持当前地区, 请先在设置中切换地区."))?;
        if let Some(game_exe) = &plan.game_exe {
            NTE_LAUNCH
                .check_runtime_config_writable(game_exe)
                .map_err(backend)?;
        }
        NTE_LAUNCH
            .ensure_install_link(&plan.prefix, &plan.install_root)
            .map_err(backend)?;

        let launcher_root = plan.launcher_root.clone();
        let official_exe = launcher_root.join(region.profile().client_exe);

        let (mode, exe, args, display_exe, helper_env, official_env) = match plan.target.mode {
            LaunchMode::Direct => {
                let game_exe = plan
                    .game_exe
                    .clone()
                    .ok_or_else(|| backend(crate::install::game_exe_hint()))?;
                if !game_exe.is_file() {
                    return Err(backend(format!(
                        "游戏可执行文件不存在：{}",
                        game_exe.display()
                    )));
                }
                let helper = helper::ensure_launch_helper(&plan.helper_dir).map_err(backend)?;
                let win_exe = windows_path(&game_exe);
                let win_cwd = windows_path(game_exe.parent().ok_or_else(|| {
                    backend(format!(
                        "{} 没有父目录",
                        game_exe.file_name().unwrap_or_default().to_string_lossy()
                    ))
                })?);
                (
                    LaunchMode::Direct,
                    helper,
                    Vec::new(),
                    game_exe,
                    Some((win_exe, win_cwd)),
                    None,
                )
            }
            LaunchMode::Background => {
                if !official_exe.is_file() {
                    return Err(backend(format!(
                        "官方启动器不存在：{}",
                        official_exe.display()
                    )));
                }
                let helper = helper::ensure_session_helper(&plan.helper_dir).map_err(backend)?;
                let win_exe = windows_path(&official_exe);
                let win_cwd = windows_path(
                    official_exe
                        .parent()
                        .ok_or_else(|| backend("官方启动器没有父目录"))?,
                );
                (
                    LaunchMode::Background,
                    helper,
                    Vec::new(),
                    official_exe,
                    None,
                    Some((win_exe, win_cwd)),
                )
            }
            LaunchMode::Official => {
                if !official_exe.is_file() {
                    return Err(backend(format!(
                        "官方启动器不存在：{}",
                        official_exe.display()
                    )));
                }
                (
                    LaunchMode::Official,
                    official_exe.clone(),
                    OFFICIAL_ARGS.map(String::from).to_vec(),
                    official_exe,
                    None,
                    None,
                )
            }
        };
        let mode_name = mode.as_str();

        let log_path = plan.log_dir.join("nte-app-launch.log");
        let patcher_log_dir = plan.log_dir.join("patcher-game");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| backend(format!("open {}: {error}", log_path.display())))?;
        let mut log_writer = log.try_clone().map_err(backend)?;
        writeln!(
            log_writer,
            "\n=== oddity launch {mode_name} ===\ntarget={} runner={} proton={} prefix={}\ninstall={}",
            display_exe.display(),
            exe.display(),
            plan.proton.name,
            plan.prefix.display(),
            plan.install_root.display()
        )
        .ok();

        let background = if mode == LaunchMode::Background {
            let native_log = display_exe
                .parent()
                .ok_or_else(|| backend("官方启动器没有父目录"))?
                .join("UserData/Log")
                .join(format!(
                    "{}.log",
                    region.profile().client_exe.trim_end_matches(".exe")
                ));
            Some(
                NativeSession::prepare(
                    &plan.log_dir,
                    &native_log,
                    background::click_profile(),
                    plan.gamescope,
                )
                .map_err(backend)?,
            )
        } else {
            None
        };

        if mode != LaunchMode::Direct && std::env::var_os("NTE_DISABLE_PRESENCE_TUNING").is_none() {
            if let Some(dir) = display_exe.parent() {
                match presence::prepare_launcher(dir) {
                    Ok(()) => {
                        writeln!(
                            log_writer,
                            "presence settings: autoUpdate=1 quitWithGame=1 \
                             showAfterGameQuit=0 closeAction=0 NeedUpdate=0"
                        )
                        .ok();
                    }
                    Err(error) => {
                        writeln!(log_writer, "presence settings not applied: {error}").ok();
                    }
                }
            }
            match presence::hide_wine_tray(&plan.proton.root, &plan.prefix) {
                Ok(()) => {
                    writeln!(
                        log_writer,
                        "wine tray: NoTrayItemsDisplay=1 (official/Wine item hidden)"
                    )
                    .ok();
                }
                Err(error) => {
                    writeln!(log_writer, "wine tray not hidden: {error}").ok();
                }
            }
        }
        if ticket.cancelled() {
            return Err(backend(CANCELLED));
        }

        let direct = if mode == LaunchMode::Direct {
            fs::create_dir_all(&patcher_log_dir)
                .map_err(|error| backend(format!("无法创建游戏资源日志目录: {error}")))?;
            for path in [
                launcher_root.join(NTE_LAUNCH.sdk_dll),
                display_exe.with_file_name(NTE_LAUNCH.one_sdk_dll),
            ] {
                if !path.is_file() {
                    return Err(backend("缺少直接启动所需的官方SDK组件, 请先检查游戏资源."));
                }
            }
            let client = Client::load(&launcher_root).map_err(backend)?;
            for path in [
                LAYOUT
                    .patcher_config_path(&launcher_root, region.profile().app_id)
                    .join("PatcherConfig.json"),
                LAYOUT.patcher_sdk_path(&launcher_root).join("ResList.xml"),
            ] {
                if !path.is_file() {
                    return Err(backend(
                        "缺少直接启动所需的资源初始化信息, 请先完成资源更新或使用后台启动.",
                    ));
                }
            }
            if fs::canonicalize(plan.prefix.join("dosdevices/z:"))
                .ok()
                .as_deref()
                != Some(Path::new("/"))
            {
                return Err(backend(
                    "直接启动需要运行环境将Z:映射到Linux根目录, 请检查运行环境配置.",
                ));
            }
            let record = Vault::new()
                .load(plan.account_id.as_deref())
                .map_err(backend)?;
            let session = client.exchange(record).map_err(backend)?;
            let host_exe = helper::ensure_sdk_host(&plan.helper_dir).map_err(backend)?;
            let mut host = Command::new(&plan.umu);
            host.arg(windows_path(&host_exe))
                .env("WINEPREFIX", &plan.prefix)
                .env("PROTONPATH", &plan.proton.root)
                .env("GAMEID", "0")
                .env("PROTON_VERB", "run")
                .env("PROTON_LOG", "0")
                .env("UMU_LOG", "error")
                .current_dir(&plan.install_root);
            configure_network(&mut host);
            configure_display(&mut host, wayland_available(), plan.gamescope);
            let (direct_session, direct_pid) =
                direct_session::DirectSession::start(client, session, &mut host, &log)
                    .map_err(backend)?;
            Some((direct_session, direct_pid))
        } else {
            None
        };

        let mut cmd = Command::new(&plan.runner.program);
        cmd.args(&plan.runner.prefix_args);
        if mode == LaunchMode::Direct {
            cmd.arg(windows_path(&exe));
        } else {
            cmd.arg(&exe);
        }
        cmd.args(&args);
        cmd.env("WINEPREFIX", &plan.prefix)
            .env("PROTONPATH", &plan.proton.root)
            .env("GAMEID", "0")
            .env("LauncherStarted", "1")
            .env("pgp_scale_value", "1")
            .env("UMU_LOG", "info")
            .env("PROTON_LOG", "0")
            .env("PROTON_VERB", "run");
        configure_network(&mut cmd);
        configure_display(&mut cmd, wayland_available(), plan.gamescope);
        for key in [
            "NTE_GAME_EXE",
            "NTE_GAME_CWD",
            "NTE_GAME_ARGS",
            "NTE_BRIDGE_EXE",
            "NTE_BRIDGE_CWD",
            "NTE_BRIDGE_ARGS",
            "NTE_HELPER_LOG",
            "NTE_SESSION_HELPER",
            "NTE_SESSION_OUTPUT",
            "NTE_SESSION_SIGNAL",
            "NTE_SESSION_CANCEL",
            "NTE_SESSION_CLICK",
        ] {
            cmd.env_remove(key);
        }
        if let Some(background) = &background {
            let helper = helper::ensure_launch_helper(&plan.helper_dir).map_err(backend)?;
            background.configure(&mut cmd, &helper);
            writeln!(
                log_writer,
                "session_status={}",
                background.status_path().display()
            )
            .ok();
        }
        let mut prepared_log = log_writer.try_clone().map_err(backend)?;
        ticket.prepare(&mut cmd, &mut prepared_log)?;
        if let Some((_session, pid)) = &direct {
            for (name, _) in std::env::vars_os() {
                let text = name.to_string_lossy();
                if ["Arc", "CoreGame", "anchor_", "_PATCHER_", "NTE_BRIDGE_"]
                    .iter()
                    .any(|prefix| text.starts_with(prefix))
                {
                    cmd.env_remove(name);
                }
            }
            for name in [
                "SDKFILEPATH",
                "gameRealOemId",
                "dxtools",
                "ODDITY_HANDOFF_NONCE",
            ] {
                cmd.env_remove(name);
            }
            let paths = sdk_paths(region, &launcher_root, &plan.install_root, &patcher_log_dir);
            direct_session::DirectSession::configure_game(&mut cmd, &paths, *pid);
        }
        if let Some((win_exe, win_cwd)) = &helper_env {
            cmd.env("NTE_GAME_EXE", win_exe)
                .env("NTE_GAME_CWD", win_cwd)
                .env("NTE_GAME_ARGS", NTE_LAUNCH.args_line());
        }
        if let Some((win_exe, win_cwd)) = &official_env {
            cmd.env("NTE_BRIDGE_EXE", win_exe)
                .env("NTE_BRIDGE_CWD", win_cwd)
                .env("NTE_BRIDGE_ARGS", OFFICIAL_ARGS.join(" "))
                .env(
                    "NTE_HELPER_LOG",
                    windows_path(&plan.log_dir.join("helper-window.log")),
                );
        }
        cmd.current_dir(&plan.install_root);

        let process = ticket.spawn(SpawnRequest {
            command: cmd,
            log: Some(log),
            stderr_null: false,
            private_umask: false,
        })?;

        // umu-run 在运行环境或 PROTONPATH 无效时立即退出; 保留这段窗口,
        // 让界面显示真实日志行而不是笼统的"进程已退出".
        let immediate_exit = if mode == LaunchMode::Background {
            None
        } else {
            poll_immediate_exit(process.as_ref())
        };
        if let Some(status) = immediate_exit {
            let code = match status {
                ProcessExit::Exited(code) => format!("退出码 {code}"),
                ProcessExit::Signaled => "被信号终止".to_string(),
            };
            let label = match mode {
                LaunchMode::Direct => "游戏启动",
                LaunchMode::Background => "后台启动",
                LaunchMode::Official => "官方启动器启动",
            };
            return Err(LaunchError::new(
                LaunchFailureKind::EarlyExit,
                format!("{label}失败（{code}）{}", launch_log_tail(&log_path)),
            ));
        }

        let result = LaunchResult {
            mode: mode_name.to_string(),
            pid: process.pid(),
            exe: display_exe.display().to_string(),
            prefix: plan.prefix.display().to_string(),
            log: log_path.display().to_string(),
            started_at: now_millis(),
        };
        // 监视线程接管后才报告游戏存活; 编排线程异常退出时宿主据此回收会话.
        let game_alive = Arc::new(AtomicBool::new(false));
        let control: Arc<dyn SessionControl> = Arc::new(ProviderControl {
            process: Arc::clone(&process),
            background: background.clone(),
            game_alive: Arc::clone(&game_alive),
        });
        ticket.bind_control(control);
        ticket.started(StartedFacts {
            exe: display_exe.display().to_string(),
            log: log_path.display().to_string(),
        })?;
        let cancelled = ticket.cancelled();
        let direct = direct.map(|(session, _)| session);
        spawn_monitor(ticket, process, background, direct, game_alive, cancelled);
        Ok(result)
    }
}

/// 直启模式需要交给游戏的 SDK 路径集合.
fn sdk_paths(
    region: Region,
    launcher_root: &Path,
    install_root: &Path,
    patcher_log: &Path,
) -> GameSdkPaths {
    GameSdkPaths {
        sdk_share_dir: launcher_root.join(NTE_LAUNCH.sdk_share_dir),
        patcher_config_path: LAYOUT.patcher_config_path(launcher_root, region.profile().app_id),
        patcher_userdata_path: launcher_root.join("UserData"),
        patcher_res_path: install_root.to_path_buf(),
        patcher_sdk_path: LAYOUT.patcher_sdk_path(launcher_root),
        patcher_log_path: patcher_log.to_path_buf(),
    }
}

fn wayland_available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 立即退出时直接展示的最后几行启动日志.
fn launch_log_tail(path: &Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let start = lines.len().saturating_sub(3);
    let mut tail = lines[start..].join(" | ");
    if tail.chars().count() > 400 {
        tail = tail.chars().take(400).collect::<String>() + "…";
    }
    format!("：{tail}")
}

fn poll_immediate_exit(process: &dyn GameProcess) -> Option<ProcessExit> {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match process.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// 绑定到会话的关闭入口.
struct ProviderControl {
    process: Arc<dyn GameProcess>,
    background: Option<Arc<NativeSession>>,
    game_alive: Arc<AtomicBool>,
}

impl SessionControl for ProviderControl {
    fn cancel(&self) {
        // 运行阶段的结束统一走 close; 准备阶段的取消由宿主在票据上置位.
    }

    fn close(&self) -> Result<(), String> {
        if let Some(background) = &self.background {
            return background.close();
        }
        process::terminate_group(self.process.pgid())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.game_alive.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if self.game_alive.load(Ordering::Acquire) {
            Err("游戏进程仍在运行, 请退出游戏后重试".into())
        } else {
            Ok(())
        }
    }

    fn is_alive(&self) -> bool {
        self.background.as_ref().map_or_else(
            || process::group_alive(self.process.pgid()) || self.game_alive.load(Ordering::Acquire),
            |background| !background.finished(),
        )
    }
}

/// 监视线程: 采样驱动宿主管理器, 游戏退出后交终态给宿主.
fn spawn_monitor(
    ticket: SessionTicket,
    process: Arc<dyn GameProcess>,
    background: Option<Arc<NativeSession>>,
    direct: Option<Arc<direct_session::DirectSession>>,
    game_alive: Arc<AtomicBool>,
    initial_cancelled: bool,
) {
    thread::spawn(move || {
        let mut watch = GameProcessWatch::new(process.pid());
        let mut seen = false;
        let mut exit_code = None;
        let mut sampled = Instant::now() - Duration::from_secs(1);
        let (stage_tx, stage_rx) = mpsc::channel::<LaunchStage>();
        let (result_tx, result_rx) = mpsc::channel::<Result<(), String>>();
        let driver: Option<thread::JoinHandle<()>> = if let Some(background) = background.clone() {
            let process = Arc::clone(&process);
            Some(thread::spawn(move || {
                let result = if initial_cancelled {
                    // 取消请求可能早于监控线程启动, 这里补上会话关闭信号.
                    background.close()
                } else {
                    background.monitor(process.as_ref(), move |stage| {
                        let _ = stage_tx.send(stage);
                    })
                };
                let _ = result_tx.send(result);
            }))
        } else {
            None
        };
        let mut driver_result = None;

        loop {
            let game_running = watch.alive();
            let background_running = background
                .as_ref()
                .is_some_and(|background| background.game_alive().unwrap_or(true));
            let running = game_running || background_running;
            game_alive.store(running, Ordering::Release);
            if running {
                seen = true;
            }
            if sampled.elapsed() >= Duration::from_millis(500) {
                let mut game_pids: Vec<u32> = watch.known.keys().copied().collect();
                if let Some(background) = &background {
                    if let Ok(pids) = background.game_pids() {
                        game_pids.extend(pids);
                    }
                }
                game_pids.sort_unstable();
                game_pids.dedup();
                ticket.sample(SessionSample {
                    game_running,
                    game_pids,
                });
                while let Ok(stage) = stage_rx.try_recv() {
                    ticket.stage(stage);
                }
                sampled = Instant::now();
            }
            // 后台会话的监视线程结束后立即收尾, 不等游戏进程被识别.
            if driver.is_some() {
                if let Ok(result) = result_rx.try_recv() {
                    driver_result = Some(result);
                    break;
                }
            }
            let process_exited = if driver.is_none() {
                if let Ok(Some(exit)) = process.try_wait() {
                    exit_code = match exit {
                        ProcessExit::Exited(code) => Some(code),
                        ProcessExit::Signaled => None,
                    };
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if seen && !running {
                game_alive.store(false, Ordering::Release);
                ticket.game_exited(None);
                break;
            }
            // 包装进程退出且进程组已清理: 即使没识别到游戏进程也要交终态.
            if process_exited && !process::group_alive(process.pgid()) {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }

        let result = match driver {
            Some(driver) => {
                let _ = driver.join();
                driver_result
                    .take()
                    .or_else(|| result_rx.try_recv().ok())
                    .unwrap_or_else(|| Err("监控线程异常退出".into()))
            }
            None => {
                // 包装进程已经退出; 顺带取回退出码.
                let exit = process.wait().ok();
                if exit_code.is_none() {
                    exit_code = exit.and_then(|exit| match exit {
                        ProcessExit::Exited(code) => Some(code),
                        ProcessExit::Signaled => None,
                    });
                }
                Ok(())
            }
        };
        while process::group_alive(process.pgid()) {
            thread::sleep(Duration::from_millis(120));
        }
        game_alive.store(false, Ordering::Release);
        if let Some(direct) = direct {
            direct.close();
        }
        let retry = result.is_err()
            && background
                .as_ref()
                .is_some_and(|background| background.closing() && !background.finished());
        let failure = result.err().map(|error| LaunchFailure {
            kind: if retry {
                LaunchFailureKind::Backend
            } else {
                LaunchFailureKind::EarlyExit
            },
            detail: Some(error),
        });
        ticket.end(SessionEnd {
            game_started: seen,
            exit_code,
            failure,
            forced: initial_cancelled,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn launch_log_tail_keeps_the_last_lines() {
        let dir = TempDir::new("log-tail");
        assert_eq!(launch_log_tail(&dir.join("missing.log")), "");

        dir.write("empty.log", "\n   \n");
        assert_eq!(launch_log_tail(&dir.join("empty.log")), "");

        dir.write("full.log", "first\n  second  \nthird\nfourth\n");
        assert_eq!(
            launch_log_tail(&dir.join("full.log")),
            "：second | third | fourth"
        );

        dir.write("long.log", format!("{}\n", "x".repeat(500)));
        let tail = launch_log_tail(&dir.join("long.log"));
        assert_eq!(tail.chars().count(), 402);
        assert!(tail.ends_with('…'), "{tail}");
    }

    #[test]
    fn sdk_paths_follow_the_launcher_layout_and_recipe() {
        let paths = sdk_paths(
            Region::Cn,
            Path::new("/launcher"),
            Path::new("/install"),
            Path::new("/logs/patcher-game"),
        );
        assert_eq!(paths.sdk_share_dir, Path::new("/launcher/WPGameClientSDK"));
        assert_eq!(
            paths.patcher_config_path,
            Path::new("/launcher/ResFilesM/1289/PatcherConfig")
        );
        assert_eq!(paths.patcher_userdata_path, Path::new("/launcher/UserData"));
        assert_eq!(paths.patcher_res_path, Path::new("/install"));
        assert_eq!(
            paths.patcher_sdk_path,
            Path::new("/launcher/UserData/Patcher/PatcherSDK")
        );
        assert_eq!(paths.patcher_log_path, Path::new("/logs/patcher-game"));

        let global = sdk_paths(
            Region::Global,
            Path::new("/launcher"),
            Path::new("/install"),
            Path::new("/logs/patcher-game"),
        );
        assert_eq!(
            global.patcher_config_path,
            Path::new("/launcher/ResFilesM/3000001/PatcherConfig")
        );
    }
}
