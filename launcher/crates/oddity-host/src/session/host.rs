//! 宿主的会话能力实现: 解析计划, 拉起进程, 驱动管理器与兜底收尾.
//!
//! 注册表预留会话后把 SessionTicket 交给启动实现; 这里实现宿主一侧能力.
//! 所有回调都先释放注册表锁, 避免启动实现的收尾回调形成锁环.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use oddity_runtime::{
    GameProcess, LaunchError, LaunchFailureKind, LaunchPlan, LaunchStage, ProtonRef, RunnerSpec,
    SessionControl, SessionEnd, SessionHost, SessionSample, SessionTarget, SpawnRequest,
    StartedFacts,
};

use crate::service::ServiceHandle;
use crate::session::{self, process::SpawnedProcess, Session};
use crate::{environment_commands, snapshot_commands};

/// 一次预留会话的宿主侧状态.
pub struct HostSession {
    app: ServiceHandle,
    pub(crate) settings: crate::settings::Settings,
    pub(crate) id: u64,
    pub(crate) cancel: Arc<AtomicBool>,
    inner: Mutex<Reserved>,
}

/// plan 与 spawn 之间需要跨回调保留的宿主资源.
#[derive(Default)]
struct Reserved {
    plan: Option<LaunchPlan>,
    install_lock: Option<Arc<oddity_platform::install_lock::InstallLock>>,
    prefix_lock: Option<Arc<crate::environment::PrefixLock>>,
    snapshot: Option<String>,
    performance: Option<crate::performance::Recording>,
    game_mode: Option<Arc<Mutex<crate::game_mode::Session>>>,
    process: Option<Arc<dyn GameProcess>>,
    control: Option<Arc<dyn SessionControl>>,
}

impl HostSession {
    pub fn new(
        app: ServiceHandle,
        settings: crate::settings::Settings,
        id: u64,
        cancel: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            app,
            settings,
            id,
            cancel,
            inner: Mutex::new(Reserved::default()),
        })
    }

    /// 已经拉起的进程句柄.
    #[cfg(any(test, feature = "testing"))]
    pub fn process(&self) -> Option<Arc<dyn GameProcess>> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.process.clone())
    }

    pub fn session(&self) -> Option<Session> {
        self.app.state.game.lock().ok().and_then(|registry| {
            registry
                .session
                .as_ref()
                .filter(|session| session.id == self.id)
                .cloned()
        })
    }

    fn resolve_plan(&self) -> Result<LaunchPlan, LaunchError> {
        let mut inner = self.inner.lock().map_err(|_| backend("会话状态不可用"))?;
        if let Some(plan) = inner.plan.clone() {
            return Ok(plan);
        }
        let settings = &self.settings;
        let mode = session::launch_mode(settings)?;
        if self.cancel.load(Ordering::Acquire) {
            return Err(backend(crate::session::LAUNCH_CANCELLED));
        }
        let packages = &self.app.state.packages;
        let game = &settings.active_game;
        let name = crate::install::display_name(packages, game);
        let launcher_root = crate::install::launcher_root(packages, settings).ok_or_else(|| {
            backend(
                if crate::install::configured_install_root(packages, settings).is_some() {
                    format!(
                        "所选安装目录尚未安装启动器文件: {}. 可在游戏更新页开始安装.",
                        crate::install::selected_install_root(packages, settings).display()
                    )
                } else {
                    format!("尚未找到《{name}》启动器安装目录")
                },
            )
        })?;
        let install_dir =
            crate::install::effective_install_root(packages, settings, Some(&launcher_root));
        let install_lock = Arc::new(
            oddity_platform::install_lock::InstallLock::acquire(&install_dir).map_err(backend)?,
        );
        let channel = crate::channel::effective(
            packages,
            game.as_str(),
            settings.current().distribution.as_ref(),
        )
        .ok_or_else(|| backend(format!("《{name}》没有可用的发行渠道.")))?;
        let game_exe = crate::install::game_exe(packages, game, &launcher_root);
        let prefix = environment_commands::prefix(packages, settings).map_err(backend)?;
        environment_commands::verify_proton(settings).map_err(backend)?;
        let umu = crate::runtime_setup::umu(&crate::paths::data_dir())
            .ok_or_else(|| backend("PATH 中未找到 umu-run"))?;
        let proton = crate::bootstrap::detect_proton(settings)
            .ok_or_else(|| backend("未找到可用的 Proton（需要 proton 与 toolmanifest.vdf）"))?;
        if !prefix.join("drive_c").is_dir() {
            return Err(backend(format!(
                "Wine prefix 尚未初始化：{}",
                prefix.display()
            )));
        }
        let prefix_lock = Arc::new(
            crate::environment::lock_prefix(&prefix)
                .map_err(|error| backend(format!("{error:#}")))?,
        );
        let log_dir = environment_commands::log_dir(settings).map_err(backend)?;
        std::fs::create_dir_all(&log_dir)
            .map_err(|error| backend(format!("create log dir: {error}")))?;
        let runner = self.runner(&umu)?;
        let identity = environment_commands::selected(settings)
            .map(|environment| environment.proton.identity.clone())
            .unwrap_or_default();
        let plan = LaunchPlan {
            target: SessionTarget {
                game: settings.active_game.as_str().to_string(),
                distribution: channel.id.to_string(),
                mode,
            },
            install_root: install_dir,
            launcher_root: launcher_root.clone(),
            game_exe,
            game_version: crate::install::game_version(packages, game, &launcher_root),
            launcher_version: crate::install::launcher_info(packages, game, &launcher_root)
                .map(|info| info.version)
                .unwrap_or_default(),
            prefix,
            proton: ProtonRef {
                root: proton.root.clone(),
                name: proton.name.clone(),
                identity,
            },
            umu,
            log_dir,
            helper_dir: crate::paths::data_dir().join("helpers"),
            runner,
            gamescope: settings.current().game_mode.gamescope.enabled,
            account_id: settings.current().game_account_id.clone(),
        };
        inner.plan = Some(plan.clone());
        inner.install_lock = Some(install_lock);
        inner.prefix_lock = Some(prefix_lock);
        Ok(plan)
    }

    fn runner(&self, umu: &std::path::Path) -> Result<RunnerSpec, LaunchError> {
        let command = self
            .settings
            .current()
            .game_mode
            .gamescope
            .command(umu, self.settings.current().game_mode.overlay)
            .map_err(|error| backend(error.to_string()))?;
        Ok(RunnerSpec {
            program: std::path::PathBuf::from(command.get_program()),
            prefix_args: command
                .get_args()
                .map(|argument| argument.to_os_string())
                .collect(),
        })
    }

    fn release(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.install_lock = None;
            inner.prefix_lock = None;
            inner.process = None;
            inner.control = None;
        }
    }
}

pub(crate) fn backend(message: impl std::fmt::Display) -> LaunchError {
    LaunchError::new(LaunchFailureKind::Backend, message.to_string())
}

impl SessionHost for HostSession {
    fn plan(&self, _id: &str) -> Result<LaunchPlan, LaunchError> {
        self.resolve_plan()
    }

    fn prepare(
        &self,
        _id: &str,
        command: &mut std::process::Command,
        log: &mut dyn Write,
    ) -> Result<(), LaunchError> {
        oddity_platform::audio::configure(command).map_err(backend)?;
        let plan = self.resolve_plan()?;
        let mut inner = self.inner.lock().map_err(|_| backend("会话状态不可用"))?;
        let game_mode = Arc::new(Mutex::new(crate::game_mode::Session::start(
            &self.settings.current().game_mode,
            command,
        )));
        inner.game_mode = Some(game_mode);
        match self.app.performance.prepare(&self.settings, &plan, command) {
            Ok(recording) => inner.performance = recording,
            Err(error) => eprintln!("{error}"),
        }
        let snapshot = snapshot_facts(&self.settings, &plan, &self.app.state.packages);
        match self.app.snapshots.begin(snapshot.clone()) {
            Ok(id) => {
                writeln!(log, "{}", snapshot.log_line()).ok();
                inner.snapshot = Some(id);
            }
            Err(error) => eprintln!("记录环境快照失败: {error}"),
        }
        Ok(())
    }

    fn spawn(&self, _id: &str, request: SpawnRequest) -> Result<Arc<dyn GameProcess>, LaunchError> {
        let mut command = request.command;
        command.process_group(0);
        if let Some(log) = request.log {
            command.stdout(std::process::Stdio::from(log.try_clone().map_err(backend)?));
            command.stderr(std::process::Stdio::from(log));
        } else if request.stderr_null {
            command.stderr(std::process::Stdio::null());
        }
        let child = command.spawn().map_err(|error| {
            LaunchError::new(LaunchFailureKind::Spawn, format!("启动游戏失败：{error}"))
        })?;
        let process: Arc<dyn GameProcess> = Arc::new(SpawnedProcess::adopt(child));
        let mut inner = self.inner.lock().map_err(|_| backend("会话状态不可用"))?;
        inner.process = Some(Arc::clone(&process));
        Ok(process)
    }

    fn started(&self, _id: &str, facts: StartedFacts) -> Result<(), LaunchError> {
        let mut inner = self.inner.lock().map_err(|_| backend("会话状态不可用"))?;
        let plan = inner.plan.clone().ok_or_else(|| backend("启动计划缺失"))?;
        let process = inner
            .process
            .clone()
            .ok_or_else(|| backend("游戏进程缺失"))?;
        let cancelled = self.cancel.load(Ordering::Acquire);
        let distribution = crate::channel::ChannelId::new(plan.target.distribution.clone());
        let snapshot_success_after_ms =
            crate::snapshots::success_after_ms(self.settings.snapshot_success_minutes);
        let session = Session {
            performance: Arc::new(Mutex::new(inner.performance.take())),
            id: self.id,
            pid: process.pid(),
            pgid: process.pgid(),
            exe: facts.exe,
            started_at: crate::now_millis(),
            log: facts.log,
            mode: plan.target.mode.as_str().to_string(),
            distribution,
            game: self.settings.active_game.clone(),
            closing: cancelled,
            exit_notified: false,
            game_alive: Arc::new(AtomicBool::new(true)),
            monitor_done: Arc::new(AtomicBool::new(false)),
            finalized: Arc::new(AtomicBool::new(false)),
            _install_lock: inner.install_lock.clone(),
            _prefix_lock: inner.prefix_lock.clone(),
            game_mode: inner.game_mode.clone(),
            snapshot: inner.snapshot.clone(),
            snapshot_success_after_ms,
            launcher_root: Some(plan.launcher_root.clone()),
            monitor_state: Arc::new(Mutex::new(session::monitor::MonitorState::default())),
            control: session::ControlSlot(Arc::new(Mutex::new(inner.control.clone()))),
        };
        {
            let state = &self.app.state;
            let mut registry = state.game.lock().map_err(|_| backend("会话状态不可用"))?;
            registry.starting = false;
            registry.launch_cancel = None;
            if let Some(progress) = registry.launch.as_mut() {
                progress.advance(LaunchStage::Helper);
                if cancelled {
                    progress.cancelling = true;
                }
            }
            registry.session = Some(session);
        }
        session::emit_game_status(&self.app, &self.app.state);
        Ok(())
    }

    fn stage(&self, _id: &str, stage: LaunchStage) {
        session::publish_launch_stage(&self.app, self.id, stage);
    }

    fn sample(&self, _id: &str, sample: SessionSample) {
        if let Some(session) = self.session() {
            session::monitor::apply_sample(&self.app, &session, &sample);
        }
    }

    fn game_exited(&self, _id: &str, _exit_code: Option<i32>) {
        if let Some(session) = self.session() {
            session::monitor::finish_game_early(&self.app, &session);
        }
    }

    fn end(&self, _id: &str, end: SessionEnd) {
        let Some(session) = self.session() else {
            return;
        };
        // 后台清理失败时保留会话, 由 close_game 重试并完成统一收尾.
        if end.failure.is_some() && session.control().is_some_and(|control| control.is_alive()) {
            {
                let state = &self.app.state;
                if let Ok(mut registry) = state.game.lock() {
                    if registry.session.as_ref().map(|current| current.id) == Some(session.id) {
                        if let Some(current) = registry.session.as_mut() {
                            current.closing = false;
                        }
                        if let Some(launch) = registry.launch.as_mut() {
                            launch.cancelling = false;
                        }
                    }
                }
                session::emit_game_status(&self.app, state);
            }
            self.release();
            return;
        }
        session::monitor::finish_session(&self.app, &session, &end);
        self.release();
    }

    fn cancelled(&self, _id: &str) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    fn bind_control(&self, _id: &str, control: Arc<dyn SessionControl>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.control = Some(control.clone());
        }
        if let Ok(mut registry) = self.app.state.game.lock() {
            if let Some(session) = registry
                .session
                .as_mut()
                .filter(|session| session.id == self.id)
            {
                if let Ok(mut slot) = session.control.0.lock() {
                    *slot = Some(control);
                }
            }
        }
    }

    fn abandon(&self, _id: &str) {
        let (process, snapshot) = {
            let inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(_) => return,
            };
            (inner.process.clone(), inner.snapshot.clone())
        };
        let mut exit_code = None;
        if let Some(process) = process {
            if process.try_wait().ok().flatten().is_none() {
                let _ = oddity_platform::process::terminate_group(process.pgid());
            }
            if let Ok(exit) = process.wait() {
                exit_code = match exit {
                    oddity_runtime::ProcessExit::Exited(code) => Some(code),
                    oddity_runtime::ProcessExit::Signaled => None,
                };
            }
        }
        if let Some(id) = snapshot {
            self.app
                .snapshots
                .finish(&id, Some(crate::snapshots::Failure::early_exit(exit_code)));
        }
        if let Some(session) = self.session() {
            session::monitor::abandon_session(&self.app, &session);
        }
        self.release();
    }
}

/// 启动前采集的环境快照; 探测可能调用 glxinfo, 只在开始快照时执行.
pub(crate) fn snapshot_facts(
    settings: &crate::settings::Settings,
    plan: &LaunchPlan,
    registry: &oddity_runtime::Registry,
) -> crate::snapshots::Snapshot {
    let mut inputs = snapshot_commands::current_inputs(registry, settings);
    inputs.install_root = Some(plan.install_root.clone());
    inputs.launcher_root = Some(plan.launcher_root.clone());
    inputs.prefix = Some(plan.prefix.clone());
    inputs.prefix_ok = true;
    inputs.proton_name = Some(plan.proton.name.clone());
    inputs.proton_ok = true;
    inputs.umu = Some(plan.umu.clone());
    crate::snapshots::capture(inputs)
}
