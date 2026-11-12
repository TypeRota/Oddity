use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod protocol;

pub use self::protocol::{click_profile, ClickProfile, FreshLog, SessionStage, SessionStatus};
use oddity_platform::process;
use oddity_runtime::{GameProcess, LaunchStage, ProcessExit};

const STATUS_LIMIT: u64 = 4 * 1024 * 1024;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
struct Timeouts {
    helper: Duration,
    resources: Duration,
    game: Duration,
    cleanup_grace: Duration,
    cleanup_total: Duration,
    poll: Duration,
    reap: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            helper: Duration::from_secs(60),
            resources: Duration::from_secs(180),
            game: Duration::from_secs(90),
            cleanup_grace: Duration::from_secs(12),
            cleanup_total: Duration::from_secs(20),
            poll: Duration::from_millis(200),
            reap: Duration::from_secs(2),
        }
    }
}

pub struct NativeSession {
    status: PathBuf,
    signal: PathBuf,
    cancel: PathBuf,
    native_log: PathBuf,
    fresh_log: FreshLog,
    profile: ClickProfile,
    gamescope: bool,
    token: String,
    processes: process::ProcessSet,
    closing: AtomicBool,
    finished: AtomicBool,
    cleanup: Mutex<()>,
    timeouts: Timeouts,
}

impl std::fmt::Debug for NativeSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeSession")
            .field("status", &self.status)
            .field("profile", &self.profile)
            .field("closing", &self.closing)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl NativeSession {
    pub fn prepare(
        log_dir: &Path,
        native_log: &Path,
        profile: ClickProfile,
        gamescope: bool,
    ) -> Result<Arc<Self>, String> {
        let fresh_log = FreshLog::capture(native_log)
            .map_err(|error| format!("读取官方启动器日志快照失败: {error}"))?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(log_dir)
            .map_err(|error| format!("创建后台会话日志目录失败: {error}"))?;
        fs::set_permissions(log_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("设置后台会话目录权限失败: {error}"))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        let directory = log_dir.join(format!(
            "native-session-{}-{stamp:x}-{sequence}",
            std::process::id()
        ));
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|error| format!("创建本轮后台会话目录失败: {error}"))?;
        let directory = directory
            .canonicalize()
            .map_err(|error| format!("解析后台会话目录失败: {error}"))?;
        let status = directory.join("status.log");
        let token = oddity_platform::paths::windows_path(&status);
        let processes = process::ProcessSet::new("NTE_SESSION_OUTPUT", &token);
        Ok(Arc::new(Self {
            status,
            signal: directory.join("go.signal"),
            cancel: directory.join("cancel.signal"),
            native_log: native_log.to_path_buf(),
            fresh_log,
            profile,
            gamescope,
            token,
            processes,
            closing: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            cleanup: Mutex::new(()),
            timeouts: Timeouts::default(),
        }))
    }

    pub fn configure(&self, command: &mut Command, helper: &Path) {
        command
            .env(
                "NTE_SESSION_HELPER",
                oddity_platform::paths::windows_path(helper),
            )
            .env("NTE_SESSION_OUTPUT", &self.token)
            .env(
                "NTE_SESSION_SIGNAL",
                oddity_platform::paths::windows_path(&self.signal),
            )
            .env(
                "NTE_SESSION_CANCEL",
                oddity_platform::paths::windows_path(&self.cancel),
            )
            .env("NTE_SESSION_CLICK", self.profile.argument());
        // 仅影响本轮子进程, 保证 Wine 新建的状态文件也只有当前用户可读.
        unsafe {
            command.pre_exec(|| {
                libc::umask(0o077);
                Ok(())
            });
        }
    }

    pub fn status_path(&self) -> &Path {
        &self.status
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }

    pub fn monitor(
        &self,
        process: &dyn GameProcess,
        on_stage: impl Fn(LaunchStage),
    ) -> Result<(), String> {
        let mut result = self.drive(process, &on_stage);
        // 状态读取失败不能结束已经启动的游戏, 关闭按钮仍可显式结束本轮会话.
        while !self.closing.load(Ordering::Acquire) {
            match self.game_alive() {
                Ok(true) => {
                    let _ = process.try_wait();
                    thread::sleep(self.timeouts.poll);
                }
                Ok(false) => break,
                Err(error) => {
                    result = Err(error);
                    break;
                }
            }
        }
        let cleanup = self.cleanup(self.closing.load(Ordering::Acquire));
        let reap = self.reap(process);
        cleanup?;
        reap?;
        if self.closing.load(Ordering::Acquire) {
            Ok(())
        } else {
            result
        }
    }

    pub fn close(&self) -> Result<(), String> {
        self.closing.store(true, Ordering::Release);
        self.cleanup(true)
    }

    fn read_status(&self) -> Result<SessionStatus, String> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.status)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(SessionStatus::default());
            }
            Err(error) => return Err(format!("读取后台会话状态失败: {error}")),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > STATUS_LIMIT {
            return Err("后台会话状态文件超过读取上限.".into());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置后台会话状态权限失败: {error}"))?;
        let mut bytes = Vec::new();
        file.take(STATUS_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("读取后台会话状态失败: {error}"))?;
        if bytes.len() as u64 > STATUS_LIMIT {
            return Err("后台会话状态文件超过读取上限.".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| "后台会话状态格式无效.".to_string())?;
        Ok(SessionStatus::parse(text))
    }

    fn drive(
        &self,
        process: &dyn GameProcess,
        on_stage: &impl Fn(LaunchStage),
    ) -> Result<(), String> {
        enum Phase {
            Helper,
            Resources,
            Game,
            Running,
        }
        let mut phase = Phase::Helper;
        let mut deadline = Instant::now() + self.timeouts.helper;
        let mut game_exit_seen = None;
        let mut reported_stage = None;
        let mut report = |stage: LaunchStage| {
            if reported_stage == Some(stage) {
                return;
            }
            reported_stage = Some(stage);
            on_stage(stage);
        };
        report(LaunchStage::Helper);
        loop {
            if self.closing.load(Ordering::Acquire) || self.finished() {
                return Ok(());
            }
            let status = self.read_status()?;
            if status.game_started {
                report(LaunchStage::Running);
            }
            if status.stage == SessionStage::OfficialStartClickSent {
                report(LaunchStage::Game);
            }
            // 按钮消息可能超时而游戏已经启动, 中途的 ERR 由控制器完成观察后再收束.
            if status.cleanup_incomplete {
                return Err(status.error().unwrap_or("官方后台清理未完成.").into());
            }
            if status.finished() {
                if let Some(error) = status.error() {
                    return Err(error.into());
                }
                return if status.game_started && status.game_exited {
                    Ok(())
                } else {
                    Err("官方后台会话已结束, 但未确认游戏正常启动及退出.".into())
                };
            }
            let child_exit = process
                .try_wait()
                .map_err(|error| format!("读取后台包装进程状态失败: {error}"))?;
            if status.game_started {
                phase = Phase::Running;
            }
            match phase {
                Phase::Helper if status.stage != SessionStage::Starting => {
                    phase = Phase::Resources;
                    deadline = Instant::now() + self.timeouts.resources;
                    report(LaunchStage::Resources);
                }
                Phase::Resources => {
                    if self
                        .fresh_log
                        .is_ready(&self.native_log)
                        .map_err(|error| format!("检查本轮官方资源初始化记录失败: {error}"))?
                    {
                        create_signal(&self.signal, false)?;
                        report(LaunchStage::Game);
                        phase = Phase::Game;
                        deadline = Instant::now() + self.timeouts.game;
                    }
                }
                Phase::Running => {
                    if status.game_exited {
                        let seen = game_exit_seen.get_or_insert_with(Instant::now);
                        if seen.elapsed() > self.timeouts.cleanup_total {
                            return Err("游戏已退出, 官方后台会话未按时完成清理.".into());
                        }
                    } else if child_exit.is_some()
                        && !self.game_alive()?
                        && !self
                            .owned_processes()?
                            .iter()
                            .any(|process| process.kind().is_native())
                    {
                        // 关闭 Gamescope 窗口会同时结束助手, 此时以包装进程和本轮进程的退出确认结束.
                        if self.gamescope
                            && child_exit.is_some_and(|exit| exit == ProcessExit::Exited(0))
                            && status.error().is_none()
                        {
                            return Ok(());
                        }
                        return Err("官方后台会话提前退出, 未确认游戏退出状态.".into());
                    }
                }
                _ => {}
            }
            if !matches!(phase, Phase::Running) && Instant::now() >= deadline {
                return Err(match phase {
                    Phase::Helper => "官方后台未能在 60 秒内就绪.",
                    Phase::Resources => "官方后台未能在 180 秒内完成资源初始化.",
                    Phase::Game => "官方后台未能在 90 秒内启动游戏.",
                    Phase::Running => unreachable!(),
                }
                .into());
            }
            if child_exit.is_some()
                && status.stage == SessionStage::Starting
                && self.owned_processes()?.is_empty()
            {
                return Err("后台包装进程在官方会话启动前退出.".into());
            }
            thread::sleep(self.timeouts.poll);
        }
    }

    pub fn game_pids(&self) -> Result<Vec<u32>, String> {
        Ok(self
            .owned_processes()?
            .iter()
            .filter(|process| *process.kind() == ProcessKind::Game)
            .map(|process| process.pid())
            .collect())
    }

    pub fn game_alive(&self) -> Result<bool, String> {
        Ok(self
            .owned_processes()?
            .iter()
            .any(|process| *process.kind() == ProcessKind::Game))
    }

    fn cleanup(&self, allow_game: bool) -> Result<(), String> {
        let _lock = self
            .cleanup
            .lock()
            .map_err(|_| "后台会话清理锁不可用.".to_string())?;
        if self.finished() {
            return Ok(());
        }
        if !allow_game && self.game_alive()? {
            return Err("本轮游戏仍在运行, 已保留官方后台会话.".into());
        }
        create_signal(&self.cancel, true)?;
        let started = Instant::now();
        while started.elapsed() < self.timeouts.cleanup_grace {
            let processes = self.owned_processes()?;
            if processes.is_empty() {
                self.finished.store(true, Ordering::Release);
                return Ok(());
            }
            if !allow_game
                && processes
                    .iter()
                    .any(|process| *process.kind() == ProcessKind::Game)
            {
                return Err("本轮游戏仍在运行, 已保留官方后台会话.".into());
            }
            if self.read_status().is_ok_and(|status| status.finished()) {
                break;
            }
            thread::sleep(self.timeouts.poll);
        }
        let term_deadline = Instant::now() + Duration::from_secs(2);
        while started.elapsed() < self.timeouts.cleanup_total {
            let processes = self.owned_processes()?;
            if processes.is_empty() {
                self.finished.store(true, Ordering::Release);
                return Ok(());
            }
            if !allow_game
                && processes
                    .iter()
                    .any(|process| *process.kind() == ProcessKind::Game)
            {
                return Err("本轮游戏仍在运行, 已保留官方后台会话.".into());
            }
            let signal = if Instant::now() < term_deadline {
                libc::SIGTERM
            } else {
                libc::SIGKILL
            };
            for process in processes {
                // 平台句柄固定进程身份, 发信号前再次确认会话标记及映像范围.
                if process.matches(process_kind)? {
                    process.signal(signal)?;
                }
            }
            thread::sleep(self.timeouts.poll);
        }
        Err("本轮后台进程未能在 20 秒内完成清理, 请稍后重试.".into())
    }

    fn reap(&self, process: &dyn GameProcess) -> Result<(), String> {
        let deadline = Instant::now() + self.timeouts.reap;
        loop {
            if process
                .try_wait()
                .map_err(|error| format!("回收后台包装进程失败: {error}"))?
                .is_some()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("本轮后台包装进程尚未退出, 无法确认回收完成.".into());
            }
            thread::sleep(self.timeouts.poll);
        }
    }

    fn owned_processes(&self) -> Result<Vec<process::ScopedProcess<ProcessKind>>, String> {
        self.processes.scoped(process_kind)
    }
}

fn create_signal(path: &Path, allow_existing: bool) -> Result<(), String> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(error) if allow_existing && error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("检查后台会话信号失败: {error}"))?;
            if metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() } {
                Ok(())
            } else {
                Err("后台会话信号路径无效.".into())
            }
        }
        Err(error) => Err(format!("写入后台会话信号失败: {error}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessKind {
    Wrapper,
    Native,
    Game,
}

impl ProcessKind {
    fn is_native(self) -> bool {
        self != Self::Wrapper
    }
}

/// 会话进程的类别; 具体映像名属于 NTE 启动配方, 仍留在宿主.
fn process_kind(process: &process::ProcessInfo) -> Option<ProcessKind> {
    let name = command_name(process.cmdline.first()?);
    let kind = match name.as_str() {
        "htgame.exe" => Some(ProcessKind::Game),
        "nte-session-helper.exe"
        | "nte-launch-helper.exe"
        | "ntegame.exe"
        | "ntebrowser.exe"
        | "nteglobalgame.exe"
        | "nteglobalbrowser.exe" => Some(ProcessKind::Native),
        "umu" | "umu-run" | "umu-launcher" | "umu-shim" | "umu.exe" | "proton" => {
            Some(ProcessKind::Wrapper)
        }
        name if name.starts_with("python") => {
            let script = command_name(process.cmdline.get(1)?);
            matches!(script.as_str(), "umu-run" | "proton").then_some(ProcessKind::Wrapper)
        }
        _ => None,
    }?;
    Some(kind)
}

fn command_name(value: &str) -> String {
    value
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    /// 回收式进程句柄: 测试里把假的包装进程交给会话监视.
    struct TestProcess {
        pid: u32,
        pgid: i32,
        child: Mutex<std::process::Child>,
    }

    impl TestProcess {
        fn adopt(child: std::process::Child) -> Self {
            let pid = child.id();
            Self {
                pid,
                pgid: i32::try_from(pid).unwrap_or(0),
                child: Mutex::new(child),
            }
        }
    }

    impl GameProcess for TestProcess {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn pgid(&self) -> i32 {
            self.pgid
        }

        fn try_wait(&self) -> Result<Option<ProcessExit>, String> {
            let mut child = self
                .child
                .lock()
                .map_err(|_| "child unavailable".to_string())?;
            child
                .try_wait()
                .map(|status| {
                    status.map(|status| match status.code() {
                        Some(code) => ProcessExit::Exited(code),
                        None => ProcessExit::Signaled,
                    })
                })
                .map_err(|error| error.to_string())
        }

        fn wait(&self) -> Result<ProcessExit, String> {
            let mut child = self
                .child
                .lock()
                .map_err(|_| "child unavailable".to_string())?;
            let status = child.wait().map_err(|error| error.to_string())?;
            Ok(match status.code() {
                Some(code) => ProcessExit::Exited(code),
                None => ProcessExit::Signaled,
            })
        }
    }
    use std::process::Child;

    fn session(directory: &TempDir) -> Arc<NativeSession> {
        let mut session = NativeSession::prepare(
            &directory.join("logs"),
            &directory.join("NTEGame.log"),
            ClickProfile {
                width: 1280,
                height: 720,
                x: 1120,
                y: 636,
            },
            false,
        )
        .unwrap();
        std::sync::Arc::get_mut(&mut session).unwrap().timeouts = Timeouts {
            helper: Duration::from_secs(3),
            resources: Duration::from_secs(3),
            game: Duration::from_secs(3),
            cleanup_grace: Duration::from_millis(200),
            cleanup_total: Duration::from_secs(3),
            poll: Duration::from_millis(20),
            reap: Duration::from_secs(1),
        };
        session
    }

    fn fake_wrapper(directory: &TempDir, session: &NativeSession, body: &str) -> Child {
        let script = directory.join("umu-run");
        fs::write(
            &script,
            format!(
                r#"import os,time,subprocess
def unix(name): return os.environ[name][2:].replace('\\', '/')
status=unix('NTE_SESSION_OUTPUT')
go=unix('NTE_SESSION_SIGNAL')
cancel=unix('NTE_SESSION_CANCEL')
def record(text):
    with open(status,'a') as output: output.write(text+'\n')
{body}
"#
            ),
        )
        .unwrap();
        let mut command = Command::new("/usr/bin/python3");
        command.arg(script);
        session.configure(&mut command, Path::new("/tmp/nte-launch-helper.exe"));
        command.spawn().unwrap()
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < deadline, "condition timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn close_failure_keeps_the_session_available_for_retry() {
        let directory = TempDir::new("native-session-close-retry");
        let background = session(&directory);
        // 取消信号路径被目录占用时清理失败, 会话必须保留以便重试.
        fs::create_dir(&background.cancel).unwrap();

        assert!(background.close().is_err());
        assert!(!background.finished());
        assert!(background.closing());

        fs::remove_dir(&background.cancel).unwrap();
        background.close().unwrap();
        assert!(background.finished());
    }

    #[test]
    fn prepare_keeps_control_files_private_and_configures_the_session() {
        let directory = TempDir::new("native-session-paths");
        let session = session(&directory);
        assert_eq!(
            fs::metadata(session.status.parent().unwrap())
                .unwrap()
                .mode()
                & 0o777,
            0o700
        );
        assert!(!session.status.exists());
        create_signal(&session.signal, false).unwrap();
        assert!(create_signal(&session.signal, false).is_err());
        assert_eq!(fs::metadata(&session.signal).unwrap().mode() & 0o777, 0o600);
        let mut command = Command::new("/bin/true");
        session.configure(&mut command, Path::new("/tmp/nte-launch-helper.exe"));
        let environment: std::collections::HashMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment[std::ffi::OsStr::new("NTE_SESSION_OUTPUT")],
            Some(std::ffi::OsStr::new(&session.token))
        );
        assert_eq!(
            environment[std::ffi::OsStr::new("NTE_SESSION_CLICK")],
            Some(std::ffi::OsStr::new("1280,720,1120,636"))
        );
    }

    #[test]
    fn monitor_waits_for_fresh_resources_and_reaps_its_wrapper() {
        let directory = TempDir::new("native-session-monitor");
        let native_log = directory.join("NTEGame.log");
        fs::write(
            &native_log,
            "[old] [GameClientAgent::_setLocalResChecked] 0->1\n",
        )
        .unwrap();
        let session = session(&directory);
        let body = format!(
            r#"record('HELPER_STARTED windows_pid=8')
time.sleep(0.15)
assert not os.path.exists(go)
with open({:?},'a') as output: output.write('[new] [GameClientAgent::_setLocalResChecked] 0->1\n')
while not os.path.exists(go): time.sleep(0.01)
record('OFFICIAL_START_CLICK_SENT')
record('GAME_STARTED windows_pid=12')
record('GAME_EXITED code=0')
record('SESSION_EXITED code=0')
time.sleep(60)
"#,
            native_log
        );
        let child = fake_wrapper(&directory, &session, &body);
        let process = Arc::new(TestProcess::adopt(child));
        session.monitor(process.as_ref(), |_| {}).unwrap();
        assert!(session.finished());
        assert!(process.try_wait().unwrap().is_some());
        assert!(session.signal.exists() && session.cancel.exists());
        assert_eq!(fs::metadata(&session.status).unwrap().mode() & 0o777, 0o600);
        session.close().unwrap();
    }

    #[test]
    fn monitor_reports_launch_stages_in_order() {
        let directory = TempDir::new("native-session-stages");
        let native_log = directory.join("NTEGame.log");
        let session = session(&directory);
        let stages = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&stages);
        let body = format!(
            r#"record('HELPER_STARTED windows_pid=8')
with open({:?},'w') as output: output.write('[new] [GameClientAgent::_setLocalResChecked] 0->1\n')
while not os.path.exists(go): time.sleep(0.01)
record('OFFICIAL_START_CLICK_SENT')
record('GAME_STARTED windows_pid=12')
record('GAME_EXITED code=0')
record('SESSION_EXITED code=0')
time.sleep(60)
"#,
            native_log
        );
        let child = fake_wrapper(&directory, &session, &body);
        let process = Arc::new(TestProcess::adopt(child));
        session
            .monitor(process.as_ref(), |stage| {
                collected.lock().unwrap().push(stage)
            })
            .unwrap();
        assert_eq!(
            *stages.lock().unwrap(),
            vec![
                LaunchStage::Helper,
                LaunchStage::Resources,
                LaunchStage::Game,
                LaunchStage::Running,
            ]
        );
        session.close().unwrap();
    }

    #[test]
    fn gamescope_window_close_confirms_exit_without_a_final_helper_record() {
        let directory = TempDir::new("native-session-gamescope-close");
        let mut session = session(&directory);
        std::sync::Arc::get_mut(&mut session).unwrap().gamescope = true;
        let game_image = directory.join("HTGame.exe");
        let native_image = directory.join("NTEGame.exe");
        for image in [&game_image, &native_image] {
            std::os::unix::fs::symlink("/bin/sleep", image).unwrap();
        }
        let closed = directory.join("window-closed");
        let body = format!(
            r#"native=subprocess.Popen([{native_image:?},'60'])
game=subprocess.Popen([{game_image:?},'60'])
record('HELPER_STARTED windows_pid=8')
record('GAME_STARTED windows_pid=12')
while not os.path.exists({closed:?}): time.sleep(0.01)
for process in [game,native]:
    process.terminate()
    process.wait()
"#
        );
        let child = fake_wrapper(&directory, &session, &body);
        let process = Arc::new(TestProcess::adopt(child));
        wait_until(|| session.game_alive().unwrap());
        let monitored = Arc::clone(&session);
        let monitored_process = Arc::clone(&process);
        let monitor = thread::spawn(move || {
            let result = monitored.monitor(monitored_process.as_ref(), |_| {});
            assert!(monitored_process.try_wait().unwrap().is_some());
            result
        });
        thread::sleep(Duration::from_millis(100));
        assert!(!session.finished());
        assert!(!session.cancel.exists());
        fs::write(closed, "").unwrap();
        monitor.join().unwrap().unwrap();
        assert!(session.finished());
        assert!(session.owned_processes().unwrap().is_empty());
        assert!(!session.read_status().unwrap().game_exited);
    }

    #[test]
    fn gamescope_exit_does_not_hide_startup_errors_crashes_or_other_wrapper_failures() {
        for (gamescope, events, exit) in [
            (false, "GAME_STARTED windows_pid=12", "raise SystemExit(0)"),
            (true, "HELPER_STARTED windows_pid=8", "raise SystemExit(0)"),
            (true, "GAME_STARTED windows_pid=12", "raise SystemExit(1)"),
            (
                true,
                "GAME_STARTED windows_pid=12",
                "os.kill(os.getpid(),9)",
            ),
            (
                true,
                "GAME_STARTED windows_pid=12\\nERR helper failed",
                "raise SystemExit(0)",
            ),
        ] {
            let directory = TempDir::new("native-session-gamescope-failure");
            let mut session = session(&directory);
            std::sync::Arc::get_mut(&mut session).unwrap().gamescope = gamescope;
            let child = fake_wrapper(
                &directory,
                &session,
                &format!("record('{events}')\n{exit}\n"),
            );
            let process = Arc::new(TestProcess::adopt(child));
            assert!(
                session.monitor(process.as_ref(), |_| {}).is_err(),
                "{gamescope}: {events}, {exit}"
            );
            assert!(session.finished());
            assert!(process.try_wait().unwrap().is_some());
        }
    }

    #[test]
    fn close_does_not_touch_other_tokens_or_unrecognized_images() {
        let directory = TempDir::new("native-session-scope");
        let other_directory = TempDir::new("native-session-other");
        let other = session(&other_directory);
        let session = session(&directory);
        let image = directory.join("NTEGlobalGame.exe");
        std::os::unix::fs::symlink("/bin/sleep", &image).unwrap();
        let mut own = Command::new(&image)
            .arg("60")
            .env("NTE_SESSION_OUTPUT", &session.token)
            .spawn()
            .unwrap();
        let mut unrelated = Command::new(&image)
            .arg("60")
            .env("NTE_SESSION_OUTPUT", &other.token)
            .spawn()
            .unwrap();
        let mut unknown = Command::new("/bin/sleep")
            .arg("60")
            .env("NTE_SESSION_OUTPUT", &session.token)
            .spawn()
            .unwrap();
        wait_until(|| {
            session
                .owned_processes()
                .unwrap()
                .iter()
                .any(|process| process.pid() == own.id() && *process.kind() == ProcessKind::Native)
        });
        session.close().unwrap();
        own.wait().unwrap();
        assert!(unrelated.try_wait().unwrap().is_none());
        assert!(unknown.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unknown.kill().unwrap();
        unrelated.wait().unwrap();
        unknown.wait().unwrap();
    }

    #[test]
    fn monitor_error_preserves_the_game_until_an_explicit_concurrent_close() {
        let directory = TempDir::new("native-session-close");
        let session = session(&directory);
        let game_image = directory.join("HTGame.exe");
        std::os::unix::fs::symlink("/bin/sleep", &game_image).unwrap();
        let body = format!(
            r#"game=subprocess.Popen([{game_image:?},'60'])
record('HELPER_STARTED windows_pid=8')
record('GAME_STARTED windows_pid=12')
record('ERR synthetic status read failure')
while not os.path.exists(cancel): time.sleep(0.01)
game.terminate()
game.wait()
record('GAME_EXITED code=0')
record('SESSION_EXITED code=0')
"#
        );
        let child = fake_wrapper(&directory, &session, &body);
        let process = Arc::new(TestProcess::adopt(child));
        wait_until(|| session.game_alive().unwrap());
        let game_pids = session.game_pids().unwrap();
        assert_eq!(game_pids.len(), 1);
        assert_ne!(game_pids[0], process.pid());
        let monitored = Arc::clone(&session);
        let monitored_process = Arc::clone(&process);
        let monitor = thread::spawn(move || monitored.monitor(monitored_process.as_ref(), |_| {}));
        thread::sleep(Duration::from_millis(200));
        assert!(!session.cancel.exists());
        assert!(session.game_alive().unwrap());
        session.close().unwrap();
        monitor.join().unwrap().unwrap();
        assert!(session.finished());
        assert!(!session.game_alive().unwrap());
        assert!(session.game_pids().unwrap().is_empty());
    }

    #[test]
    fn immediate_wrapper_failure_still_cancels_and_cleans_its_native_child() {
        let directory = TempDir::new("native-session-immediate-exit");
        let session = session(&directory);
        let native_image = directory.join("NTEGame.exe");
        std::os::unix::fs::symlink("/bin/sleep", &native_image).unwrap();
        let body = format!(
            r#"native=subprocess.Popen([{native_image:?},'60'])
record('HELPER_STARTED windows_pid=8')
record('ERR synthetic immediate wrapper failure')
record('SESSION_EXITED code=7')
raise SystemExit(7)
"#
        );
        let child = fake_wrapper(&directory, &session, &body);
        let process = Arc::new(TestProcess::adopt(child));
        assert!(session.monitor(process.as_ref(), |_| {}).is_err());
        assert!(session.finished());
        assert!(session.cancel.exists());
        assert!(session.owned_processes().unwrap().is_empty());
        assert!(process.try_wait().unwrap().is_some());
    }

    #[test]
    fn nonterminal_error_does_not_cancel_a_pending_official_game() {
        let directory = TempDir::new("native-session-pending-game");
        let session = session(&directory);
        let body = format!(
            r#"record('HELPER_STARTED windows_pid=8')
with open({:?},'w') as output: output.write('[new] [GameClientAgent::_setLocalResChecked] 0->1\n')
while not os.path.exists(go): time.sleep(0.01)
record('OFFICIAL_START_CLICK_SENT')
record('ERR synthetic click message timeout')
time.sleep(0.15)
assert not os.path.exists(cancel)
record('GAME_STARTED windows_pid=12')
record('GAME_EXITED code=0')
record('SESSION_EXITED code=1')
time.sleep(60)
"#,
            session.native_log
        );
        let child = fake_wrapper(&directory, &session, &body);
        let process = Arc::new(TestProcess::adopt(child));
        assert!(session.monitor(process.as_ref(), |_| {}).is_err());
        let status = session.read_status().unwrap();
        assert!(status.game_started && status.game_exited && status.finished());
        assert!(session.finished());
        assert!(process.try_wait().unwrap().is_some());
    }

    #[test]
    fn oversized_status_is_rejected_and_token_matching_is_exact() {
        let directory = TempDir::new("native-session-limits");
        let session = session(&directory);
        let file = fs::File::create(&session.status).unwrap();
        file.set_len(STATUS_LIMIT + 1).unwrap();
        assert!(session.read_status().is_err());
        let environment = directory.join("environ");
        fs::write(
            &environment,
            format!("OTHER=value\0NTE_SESSION_OUTPUT={}extra\0", session.token),
        )
        .unwrap();
        assert!(!process::environment_matches(
            &environment,
            "NTE_SESSION_OUTPUT",
            &session.token
        ));
        fs::write(
            &environment,
            format!(
                "OTHER=value\0NTE_SESSION_OUTPUT={}\0OTHER2=value\0",
                session.token
            ),
        )
        .unwrap();
        assert!(process::environment_matches(
            &environment,
            "NTE_SESSION_OUTPUT",
            &session.token
        ));
    }
}
