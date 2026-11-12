//! 包内启动测试夹具: 临时安装, prefix, Proton, 假包装程序与假宿主.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oddity_runtime::{
    GameProcess, LaunchError, LaunchMode, LaunchPlan, LaunchResult, LaunchStage, ProcessExit,
    ProtonRef, RunnerSpec, SessionControl, SessionEnd, SessionHost, SessionSample, SessionTarget,
    SpawnRequest, StartedFacts,
};

use crate::testing::{write_script, EnvGuard, TempDir};
use crate::update::region::LAYOUT;

/// 一次完整启动的夹具: 安装目录, prefix, Proton 与假包装程序.
pub(crate) struct LaunchFixture {
    pub(crate) dir: TempDir,
    pub(crate) launcher: PathBuf,
    pub(crate) install: PathBuf,
    pub(crate) prefix: PathBuf,
    pub(crate) proton: PathBuf,
    pub(crate) dump: PathBuf,
    pub(crate) wine_log: PathBuf,
    pub(crate) wrapper: PathBuf,
    pub(crate) env: EnvGuard,
}

impl LaunchFixture {
    pub(crate) fn new(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let mut env = EnvGuard::new();
        env.remove("NTE_DISABLE_PRESENCE_TUNING");
        env.remove("WAYLAND_DISPLAY");
        env.remove("WINEPREFIX");
        env.remove("PROTONPATH");
        env.remove("NTE_ROOT");
        env.remove("NTE_INSTALL");
        env.remove("STEAM_COMPAT_CLIENT_INSTALL_PATH");

        let install = dir.join("install");
        let launcher = install.join("NTELauncher");
        fs::create_dir_all(&launcher).expect("create launcher dir");
        dir.write(
            "install/NTELauncher/Config/Config.ini",
            "[VERSION]\nVersion=1.4.16\n",
        );
        dir.write(
            "install/Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
            b"MZ",
        );
        dir.write("install/NTELauncher/NTEGame.exe", b"MZ");
        fs::create_dir_all(dir.join("install/NTELauncher/UserData/Log"))
            .expect("create native log dir");
        fs::create_dir_all(dir.join("logs")).expect("create launch log dir");

        let prefix = dir.join("prefix");
        fs::create_dir_all(prefix.join("drive_c")).expect("create drive_c");
        fs::create_dir_all(prefix.join("dosdevices")).expect("create dosdevices");
        std::os::unix::fs::symlink("/", prefix.join("dosdevices/z:")).expect("link z:");

        let proton = dir.join("proton");
        fs::create_dir_all(proton.join("files/bin")).expect("create wine dir");
        let wine_log = dir.join("wine.log");
        write_script(
            &proton.join("files/bin"),
            "wine",
            &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", wine_log.display()),
        );

        let dump = dir.join("umu-dump.txt");
        let wrapper = dir.join("bin/umu-run");
        fs::create_dir_all(dir.join("bin")).expect("create bin dir");
        write_script(
            &dir.join("bin"),
            "umu-run",
            &format!(
                r#"import os, sys, time
path = {dump:?}
temporary = path + '.tmp'
with open(temporary, 'w') as output:
    output.write('args: ' + ' '.join(sys.argv[1:]) + '\n')
    output.write('cwd: ' + os.getcwd() + '\n')
    for key, value in os.environ.items():
        output.write('%s=%s\n' % (key, value))
# 先写临时文件再原子改名, 读取方看到文件时内容一定完整.
os.replace(temporary, path)
cancel = os.environ.get('NTE_SESSION_CANCEL')
while True:
    if cancel and os.path.exists(cancel[2:].replace('\\', '/')):
        break
    time.sleep(0.05)
"#,
                dump = dump.display().to_string()
            ),
        );
        // 假包装程序由 python 解释, 与后台会话的进程识别保持一致.
        let inherited = std::env::var("PATH").unwrap_or_default();
        env.set("PATH", format!("{}:{inherited}", dir.join("bin").display()));

        Self {
            dir,
            launcher,
            install,
            prefix,
            proton,
            dump,
            wine_log,
            wrapper,
            env,
        }
    }

    /// 启动计划; 模式之外的字段与宿主解析结果同形.
    pub(crate) fn plan(&self, mode: LaunchMode) -> LaunchPlan {
        self.plan_for(mode, "cn")
    }

    pub(crate) fn plan_for(&self, mode: LaunchMode, distribution: &str) -> LaunchPlan {
        LaunchPlan {
            target: SessionTarget {
                game: "nte".into(),
                distribution: distribution.into(),
                mode,
            },
            install_root: self.install.clone(),
            launcher_root: self.launcher.clone(),
            game_exe: Some(self.install.join(LAYOUT.game_exe)),
            game_version: "1.4.16".into(),
            launcher_version: "1.4.16".into(),
            prefix: self.prefix.clone(),
            proton: ProtonRef {
                root: self.proton.clone(),
                name: "GE-Proton9-1".into(),
                identity: "identity".into(),
            },
            umu: self.wrapper.clone(),
            log_dir: self.dir.join("logs"),
            helper_dir: self.dir.join("helpers"),
            runner: RunnerSpec {
                program: PathBuf::from("/usr/bin/python3"),
                prefix_args: vec![self.wrapper.clone().into_os_string()],
            },
            gamescope: false,
            account_id: None,
        }
    }

    /// 启动一次; 失败时返回用户可见错误.
    pub(crate) fn run(&self, mode: LaunchMode) -> Result<(LaunchResult, Arc<TestHost>), String> {
        self.run_plan(self.plan(mode))
    }

    pub(crate) fn run_plan(
        &self,
        plan: LaunchPlan,
    ) -> Result<(LaunchResult, Arc<TestHost>), String> {
        let host = TestHost::new(plan);
        let ticket =
            oddity_runtime::SessionTicket::new("1", Arc::clone(&host) as Arc<dyn SessionHost>);
        let result = crate::launch::provider()
            .launch(ticket)
            .map_err(|error| error.message)?;
        Ok((result, host))
    }

    /// 等假包装程序写出参数与环境.
    ///
    /// 包装程序先写临时文件再原子改名, 文件出现即内容完整, 不会读到半个 dump.
    pub(crate) fn dump(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.dump.is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        fs::read_to_string(&self.dump)
            .unwrap_or_else(|error| panic!("读取 umu dump {}: {error}", self.dump.display()))
    }

    /// 收掉本轮进程组, 让监视线程有机会交出终态.
    pub(crate) fn stop(&self, host: &Arc<TestHost>) {
        if let Some(control) = host.control() {
            let _ = control.close();
        }
        if let Some(pgid) = host.pgid() {
            let _ = oddity_platform::process::terminate_group(pgid);
        }
    }
}

/// 期待启动失败; 万一真的起来了, 先收掉进程再报错.
pub(crate) fn expect_launch_error(result: Result<(LaunchResult, Arc<TestHost>), String>) -> String {
    match result {
        Ok((_, host)) => {
            if let Some(control) = host.control() {
                let _ = control.close();
            }
            if let Some(pgid) = host.pgid() {
                let _ = oddity_platform::process::terminate_group(pgid);
            }
            panic!("expected the launch to fail");
        }
        Err(error) => error,
    }
}

/// 假宿主记录的回调.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Plan,
    Prepare,
    Spawn { program: String },
    Started(StartedFacts),
    Stage(LaunchStage),
    Sample(SessionSample),
    GameExited(Option<i32>),
    End(SessionEnd),
    BindControl,
    Abandon,
}

/// 按宿主契约记录回调并真正拉起进程的假宿主.
pub(crate) struct TestHost {
    plan: LaunchPlan,
    cancelled: AtomicBool,
    calls: Mutex<Vec<Call>>,
    control: Mutex<Option<Arc<dyn SessionControl>>>,
    process: Mutex<Option<Arc<dyn GameProcess>>>,
}

impl TestHost {
    pub(crate) fn new(plan: LaunchPlan) -> Arc<Self> {
        Arc::new(Self {
            plan,
            cancelled: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            control: Mutex::new(None),
            process: Mutex::new(None),
        })
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn control(&self) -> Option<Arc<dyn SessionControl>> {
        self.control.lock().unwrap().clone()
    }

    pub(crate) fn pgid(&self) -> Option<i32> {
        self.process
            .lock()
            .unwrap()
            .as_ref()
            .map(|process| process.pgid())
    }

    pub(crate) fn end(&self) -> Option<SessionEnd> {
        self.calls().into_iter().find_map(|call| match call {
            Call::End(end) => Some(end),
            _ => None,
        })
    }

    pub(crate) fn wait_end(&self, timeout: Duration) -> Option<SessionEnd> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(end) = self.end() {
                return Some(end);
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.end()
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

impl SessionHost for TestHost {
    fn plan(&self, _id: &str) -> Result<LaunchPlan, LaunchError> {
        self.record(Call::Plan);
        Ok(self.plan.clone())
    }

    fn prepare(
        &self,
        _id: &str,
        _command: &mut Command,
        log: &mut dyn std::io::Write,
    ) -> Result<(), LaunchError> {
        self.record(Call::Prepare);
        log.write_all(b"prepared\n").map_err(|error| {
            LaunchError::new(
                oddity_runtime::LaunchFailureKind::Backend,
                error.to_string(),
            )
        })
    }

    fn spawn(&self, _id: &str, request: SpawnRequest) -> Result<Arc<dyn GameProcess>, LaunchError> {
        self.record(Call::Spawn {
            program: request.command.get_program().to_string_lossy().into_owned(),
        });
        let mut command = request.command;
        command.process_group(0);
        if let Some(log) = request.log {
            command.stdout(std::process::Stdio::from(log.try_clone().map_err(
                |error| {
                    LaunchError::new(
                        oddity_runtime::LaunchFailureKind::Backend,
                        error.to_string(),
                    )
                },
            )?));
            command.stderr(std::process::Stdio::from(log));
        } else if request.stderr_null {
            command.stderr(std::process::Stdio::null());
        }
        let child = command.spawn().map_err(|error| {
            LaunchError::new(
                oddity_runtime::LaunchFailureKind::Spawn,
                format!("启动游戏失败：{error}"),
            )
        })?;
        let process: Arc<dyn GameProcess> = Arc::new(SpawnedProcess::adopt(child));
        *self.process.lock().unwrap() = Some(Arc::clone(&process));
        Ok(process)
    }

    fn started(&self, _id: &str, facts: StartedFacts) -> Result<(), LaunchError> {
        self.record(Call::Started(facts));
        Ok(())
    }

    fn stage(&self, _id: &str, stage: LaunchStage) {
        self.record(Call::Stage(stage));
    }

    fn sample(&self, _id: &str, sample: SessionSample) {
        self.record(Call::Sample(sample));
    }

    fn game_exited(&self, _id: &str, exit_code: Option<i32>) {
        self.record(Call::GameExited(exit_code));
    }

    fn end(&self, _id: &str, end: SessionEnd) {
        self.record(Call::End(end));
    }

    fn cancelled(&self, _id: &str) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn bind_control(&self, _id: &str, control: Arc<dyn SessionControl>) {
        *self.control.lock().unwrap() = Some(control);
        self.record(Call::BindControl);
    }

    fn abandon(&self, _id: &str) {
        self.record(Call::Abandon);
    }
}

/// 宿主已拉起并接管的进程; 回收线程缓存结局.
struct SpawnedProcess {
    pid: u32,
    pgid: i32,
    state: Arc<ExitState>,
}

#[derive(Default)]
struct ExitState {
    exit: Mutex<Option<ProcessExit>>,
    ready: Condvar,
}

impl SpawnedProcess {
    fn adopt(mut child: std::process::Child) -> Self {
        let pid = child.id();
        let pgid = i32::try_from(pid).unwrap_or(0);
        let state = Arc::new(ExitState::default());
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            let exit = match child.wait() {
                Ok(status) => match status.code() {
                    Some(code) => ProcessExit::Exited(code),
                    None => ProcessExit::Signaled,
                },
                Err(_) => ProcessExit::Signaled,
            };
            if let Ok(mut slot) = shared.exit.lock() {
                *slot = Some(exit);
            }
            shared.ready.notify_all();
        });
        Self { pid, pgid, state }
    }
}

impl GameProcess for SpawnedProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn pgid(&self) -> i32 {
        self.pgid
    }

    fn try_wait(&self) -> Result<Option<ProcessExit>, String> {
        self.state
            .exit
            .lock()
            .map(|exit| *exit)
            .map_err(|_| "进程结局不可用".to_string())
    }

    fn wait(&self) -> Result<ProcessExit, String> {
        let mut exit = self
            .state
            .exit
            .lock()
            .map_err(|_| "进程结局不可用".to_string())?;
        while exit.is_none() {
            exit = self
                .state
                .ready
                .wait(exit)
                .map_err(|_| "进程结局不可用".to_string())?;
        }
        exit.ok_or_else(|| "进程结局不可用".to_string())
    }
}

/// 等条件成立; CI 负载高时留足宽限.
pub(crate) fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    condition()
}

/// 断言路径存在且是私有文件.
pub(crate) fn is_private(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.permissions().mode() & 0o777 == 0o600)
        .unwrap_or(false)
}
