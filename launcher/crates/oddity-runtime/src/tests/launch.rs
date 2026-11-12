//! 启动会话契约测试: 票据委托、放弃语义、注册表查询与界面载荷形状.

use super::TEST_PACKAGE;
use crate::{
    GamePackage, GameProcess, LaunchError, LaunchFailure, LaunchFailureKind, LaunchMode,
    LaunchPlan, LaunchProgress, LaunchProvider, LaunchResult, LaunchStage, PackageDescriptor,
    ProcessExit, ProtonRef, Registry, RunnerSpec, SessionControl, SessionEnd, SessionHost,
    SessionSample, SessionStatus, SessionTarget, SessionTicket, SpawnRequest, StartedFacts,
};
use serde_json::json;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 测试宿主按调用顺序记录的回调.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostCall {
    Plan,
    Prepare,
    Spawn {
        program: String,
        stderr_null: bool,
        private_umask: bool,
        log: bool,
    },
    Started(StartedFacts),
    Stage(LaunchStage),
    Sample(SessionSample),
    GameExited(Option<i32>),
    End(SessionEnd),
    BindControl,
    Abandon,
}

/// 记录回调的假宿主.
#[derive(Default)]
struct RecordingHost {
    calls: Mutex<Vec<HostCall>>,
    cancelled: AtomicBool,
    control: Mutex<Option<Arc<dyn SessionControl>>>,
}

impl RecordingHost {
    fn record(&self, call: HostCall) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<HostCall> {
        self.calls.lock().unwrap().clone()
    }

    fn bound_control(&self) -> Arc<dyn SessionControl> {
        self.control
            .lock()
            .unwrap()
            .clone()
            .expect("已绑定控制入口")
    }
}

impl SessionHost for RecordingHost {
    fn plan(&self, _id: &str) -> Result<LaunchPlan, LaunchError> {
        self.record(HostCall::Plan);
        Ok(sample_plan())
    }

    fn prepare(
        &self,
        _id: &str,
        _command: &mut Command,
        log: &mut dyn Write,
    ) -> Result<(), LaunchError> {
        self.record(HostCall::Prepare);
        log.write_all(b"prepared")
            .map_err(|error| LaunchError::new(LaunchFailureKind::Backend, error.to_string()))
    }

    fn spawn(&self, _id: &str, request: SpawnRequest) -> Result<Arc<dyn GameProcess>, LaunchError> {
        self.record(HostCall::Spawn {
            program: request.command.get_program().to_string_lossy().into_owned(),
            stderr_null: request.stderr_null,
            private_umask: request.private_umask,
            log: request.log.is_some(),
        });
        Ok(Arc::new(FakeProcess))
    }

    fn started(&self, _id: &str, facts: StartedFacts) -> Result<(), LaunchError> {
        self.record(HostCall::Started(facts));
        Ok(())
    }

    fn stage(&self, _id: &str, stage: LaunchStage) {
        self.record(HostCall::Stage(stage));
    }

    fn sample(&self, _id: &str, sample: SessionSample) {
        self.record(HostCall::Sample(sample));
    }

    fn game_exited(&self, _id: &str, exit_code: Option<i32>) {
        self.record(HostCall::GameExited(exit_code));
    }

    fn end(&self, _id: &str, end: SessionEnd) {
        self.record(HostCall::End(end));
    }

    fn cancelled(&self, _id: &str) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn bind_control(&self, _id: &str, control: Arc<dyn SessionControl>) {
        *self.control.lock().unwrap() = Some(control);
        self.record(HostCall::BindControl);
    }

    fn abandon(&self, _id: &str) {
        self.record(HostCall::Abandon);
    }
}

/// 可观察的假控制入口.
struct FakeControl {
    cancelled: AtomicBool,
    closed: AtomicBool,
    alive: AtomicBool,
}

impl FakeControl {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            alive: AtomicBool::new(true),
        }
    }
}

impl SessionControl for FakeControl {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn close(&self) -> Result<(), String> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

/// 固定结局的假进程.
struct FakeProcess;

impl GameProcess for FakeProcess {
    fn pid(&self) -> u32 {
        4321
    }

    fn pgid(&self) -> i32 {
        4321
    }

    fn try_wait(&self) -> Result<Option<ProcessExit>, String> {
        Ok(Some(ProcessExit::Exited(0)))
    }

    fn wait(&self) -> Result<ProcessExit, String> {
        Ok(ProcessExit::Signaled)
    }
}

fn sample_plan() -> LaunchPlan {
    LaunchPlan {
        target: SessionTarget {
            game: "test".into(),
            distribution: "cn".into(),
            mode: LaunchMode::Background,
        },
        install_root: PathBuf::from("/games/test"),
        launcher_root: PathBuf::from("/games/test/launcher"),
        game_exe: Some(PathBuf::from("/games/test/launcher/Game.exe")),
        game_version: "1.0".into(),
        launcher_version: "2.0".into(),
        prefix: PathBuf::from("/prefix"),
        proton: ProtonRef {
            root: PathBuf::from("/proton"),
            name: "GE-Proton".into(),
            identity: "identity".into(),
        },
        umu: PathBuf::from("/usr/bin/umu-run"),
        log_dir: PathBuf::from("/logs"),
        helper_dir: PathBuf::from("/helpers"),
        runner: RunnerSpec {
            program: PathBuf::from("/usr/bin/umu-run"),
            prefix_args: vec![OsString::from("--verb")],
        },
        gamescope: false,
        account_id: None,
    }
}

static TEST_PROVIDER: TestProvider = TestProvider;

/// 经过票据走完整流程的假启动实现.
struct TestProvider;

impl LaunchProvider for TestProvider {
    fn launch(&self, ticket: SessionTicket) -> Result<LaunchResult, LaunchError> {
        let plan = ticket.plan()?;
        ticket.stage(LaunchStage::Running);
        ticket.end(SessionEnd {
            game_started: true,
            exit_code: None,
            failure: None,
            forced: false,
        });
        Ok(LaunchResult {
            mode: plan.target.mode.as_str().into(),
            pid: 4321,
            exe: plan
                .game_exe
                .as_ref()
                .expect("计划里有游戏可执行文件")
                .display()
                .to_string(),
            prefix: plan.prefix.display().to_string(),
            log: plan.log_dir.join("launch.log").display().to_string(),
            started_at: 1_700_000_000_000,
        })
    }
}

/// 声明启动实现的假游戏包.
struct ProviderPackage;

impl GamePackage for ProviderPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "test",
            name: "测试包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn launch_provider(&self) -> Option<&'static dyn LaunchProvider> {
        Some(&TEST_PROVIDER)
    }
}

#[test]
fn packages_without_a_launch_provider_stay_unregistered() {
    let mut registry = Registry::new();
    registry.install(&TEST_PACKAGE);

    assert!(registry.launch_provider("test").is_none());
    assert!(registry.launch_provider("missing").is_none());
}

#[test]
fn registry_finds_the_launch_provider_by_game_id() {
    let mut registry = Registry::new();
    registry.install(&ProviderPackage);
    let provider = registry
        .launch_provider("test")
        .expect("测试包应提供启动实现");

    let host = Arc::new(RecordingHost::default());
    let result = provider
        .launch(SessionTicket::new("session-1", host.clone()))
        .expect("启动应成功");

    assert_eq!(result.mode, "background");
    assert_eq!(result.pid, 4321);
    assert_eq!(result.exe, "/games/test/launcher/Game.exe");
    assert_eq!(result.prefix, "/prefix");
    assert_eq!(result.log, "/logs/launch.log");
    assert_eq!(result.started_at, 1_700_000_000_000);
    assert_eq!(
        host.calls(),
        [
            HostCall::Plan,
            HostCall::Stage(LaunchStage::Running),
            HostCall::End(SessionEnd {
                game_started: true,
                exit_code: None,
                failure: None,
                forced: false,
            }),
        ]
    );
}

#[test]
fn ticket_exposes_the_plan_and_abandons_an_unfinished_session() {
    let host = Arc::new(RecordingHost::default());
    let ticket = SessionTicket::new("session-1", host.clone());

    assert_eq!(ticket.id(), "session-1");
    assert_eq!(ticket.plan().unwrap(), sample_plan());

    drop(ticket);
    assert_eq!(host.calls(), [HostCall::Plan, HostCall::Abandon]);
}

#[test]
fn ticket_delegates_prepare_spawn_and_started() {
    let host = Arc::new(RecordingHost::default());
    let ticket = SessionTicket::new("session-1", host.clone());

    let mut command = Command::new("umu-run");
    command.arg("Game.exe");
    let mut log = Vec::new();
    ticket.prepare(&mut command, &mut log).expect("准备应成功");
    assert_eq!(log, b"prepared");

    let process = ticket
        .spawn(SpawnRequest {
            command: Command::new("umu-run"),
            log: None,
            stderr_null: true,
            private_umask: true,
        })
        .expect("拉起应成功");
    assert_eq!(process.pid(), 4321);
    assert_eq!(process.pgid(), 4321);
    assert_eq!(process.try_wait().unwrap(), Some(ProcessExit::Exited(0)));
    assert_eq!(process.wait().unwrap(), ProcessExit::Signaled);

    let facts = StartedFacts {
        exe: "Game.exe".into(),
        log: "/logs/launch.log".into(),
    };
    ticket.started(facts.clone()).expect("启动事实应被接受");
    ticket.end(SessionEnd {
        game_started: true,
        exit_code: Some(0),
        failure: None,
        forced: false,
    });

    assert_eq!(
        host.calls(),
        [
            HostCall::Prepare,
            HostCall::Spawn {
                program: "umu-run".into(),
                stderr_null: true,
                private_umask: true,
                log: false,
            },
            HostCall::Started(facts),
            HostCall::End(SessionEnd {
                game_started: true,
                exit_code: Some(0),
                failure: None,
                forced: false,
            }),
        ]
    );
}

#[test]
fn ticket_reports_samples_stages_and_control_bindings() {
    let host = Arc::new(RecordingHost::default());
    let ticket = SessionTicket::new("session-1", host.clone());
    let control = Arc::new(FakeControl::new());

    ticket.stage(LaunchStage::Helper);
    ticket.sample(SessionSample {
        game_running: true,
        game_pids: vec![7, 8],
    });
    ticket.game_exited(Some(3));
    let bound: Arc<dyn SessionControl> = control.clone();
    ticket.bind_control(bound);

    assert!(!ticket.cancelled());
    host.cancelled.store(true, Ordering::Release);
    assert!(ticket.cancelled());

    let bound = host.bound_control();
    assert!(bound.is_alive());
    bound.cancel();
    assert!(control.cancelled.load(Ordering::Acquire));
    assert_eq!(bound.close(), Ok(()));
    assert!(control.closed.load(Ordering::Acquire));

    let failure = LaunchFailure {
        kind: LaunchFailureKind::EarlyExit,
        detail: Some("退出码 3".into()),
    };
    ticket.end(SessionEnd {
        game_started: true,
        exit_code: Some(3),
        failure: Some(failure.clone()),
        forced: true,
    });

    assert_eq!(
        host.calls(),
        [
            HostCall::Stage(LaunchStage::Helper),
            HostCall::Sample(SessionSample {
                game_running: true,
                game_pids: vec![7, 8],
            }),
            HostCall::GameExited(Some(3)),
            HostCall::BindControl,
            HostCall::End(SessionEnd {
                game_started: true,
                exit_code: Some(3),
                failure: Some(failure),
                forced: true,
            }),
        ]
    );
}

#[test]
fn launch_errors_carry_the_failure_code_and_user_message() {
    let error = LaunchError::new(LaunchFailureKind::Cancelled, "启动已暂停。");

    assert_eq!(error.code, LaunchFailureKind::Cancelled);
    assert_eq!(error.message, "启动已暂停。");
    assert_eq!(error.to_string(), "启动已暂停。");
}

#[test]
fn launch_modes_keep_the_settings_names() {
    for (mode, name) in [
        (LaunchMode::Official, "official"),
        (LaunchMode::Background, "background"),
        (LaunchMode::Direct, "direct"),
    ] {
        assert_eq!(mode.as_str(), name);
    }
    let target = SessionTarget {
        game: "test".into(),
        distribution: "cn".into(),
        mode: LaunchMode::Direct,
    };
    assert_eq!(target.mode.as_str(), "direct");
}

// 以下载荷形状来自界面现有消费方式, 迁移到本模块后 JSON 必须逐字段不变.

#[test]
fn launch_result_keeps_the_ui_payload() {
    let result = LaunchResult {
        mode: "official".into(),
        pid: 4321,
        exe: "/games/nte/HTGame.exe".into(),
        prefix: "/prefix".into(),
        log: "/logs/launch.log".into(),
        started_at: 1_700_000_000_000,
    };

    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        json!({
            "mode": "official",
            "pid": 4321,
            "exe": "/games/nte/HTGame.exe",
            "prefix": "/prefix",
            "log": "/logs/launch.log",
            "startedAt": 1_700_000_000_000u64,
        })
    );
}

#[test]
fn launch_progress_keeps_the_ui_payload() {
    for (stage, name) in [
        (LaunchStage::Preparing, "preparing"),
        (LaunchStage::Helper, "helper"),
        (LaunchStage::Resources, "resources"),
        (LaunchStage::Game, "game"),
        (LaunchStage::Running, "running"),
    ] {
        let progress = LaunchProgress {
            stage,
            started_at: 42,
            cancelling: false,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            json!({
                "stage": name,
                "startedAt": 42,
                "cancelling": false,
            })
        );
    }

    let cancelling = LaunchProgress {
        stage: LaunchStage::Resources,
        started_at: 7,
        cancelling: true,
    };
    assert_eq!(
        serde_json::to_value(&cancelling).unwrap(),
        json!({
            "stage": "resources",
            "startedAt": 7,
            "cancelling": true,
        })
    );
}

#[test]
fn session_status_keeps_the_ui_payload() {
    let idle = SessionStatus {
        running: false,
        closing: false,
        pid: None,
        started_at: None,
        exe: None,
        log: None,
        launch: None,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&idle).unwrap(),
        json!({
            "running": false,
            "closing": false,
            "pid": null,
            "startedAt": null,
            "exe": null,
            "log": null,
        })
    );

    let running = SessionStatus {
        running: true,
        closing: false,
        pid: Some(4321),
        started_at: Some(1_700_000_000_000),
        exe: Some("/games/nte/HTGame.exe".into()),
        log: Some("/logs/launch.log".into()),
        launch: None,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&running).unwrap(),
        json!({
            "running": true,
            "closing": false,
            "pid": 4321,
            "startedAt": 1_700_000_000_000u64,
            "exe": "/games/nte/HTGame.exe",
            "log": "/logs/launch.log",
        })
    );

    let starting = SessionStatus {
        running: false,
        closing: false,
        pid: None,
        started_at: None,
        exe: None,
        log: None,
        launch: Some(LaunchProgress {
            stage: LaunchStage::Preparing,
            started_at: 42,
            cancelling: false,
        }),
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&starting).unwrap(),
        json!({
            "running": false,
            "closing": false,
            "pid": null,
            "startedAt": null,
            "exe": null,
            "log": null,
            "launch": {
                "stage": "preparing",
                "startedAt": 42,
                "cancelling": false,
            },
        })
    );

    let closing = SessionStatus {
        running: true,
        closing: true,
        pid: Some(9),
        started_at: Some(1),
        exe: Some("Game.exe".into()),
        log: Some("/logs/launch.log".into()),
        launch: Some(LaunchProgress {
            stage: LaunchStage::Running,
            started_at: 1,
            cancelling: true,
        }),
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&closing).unwrap(),
        json!({
            "running": true,
            "closing": true,
            "pid": 9,
            "startedAt": 1,
            "exe": "Game.exe",
            "log": "/logs/launch.log",
            "launch": {
                "stage": "running",
                "startedAt": 1,
                "cancelling": true,
            },
        })
    );

    let failed = SessionStatus {
        running: false,
        closing: false,
        pid: None,
        started_at: None,
        exe: None,
        log: None,
        launch: None,
        error: Some("启动失败：退出码 1".into()),
    };
    let value = serde_json::to_value(&failed).unwrap();
    assert_eq!(
        value,
        json!({
            "running": false,
            "closing": false,
            "pid": null,
            "startedAt": null,
            "exe": null,
            "log": null,
            "error": "启动失败：退出码 1",
        })
    );
    assert!(value.get("launch").is_none());
}
#[test]
fn launch_progress_stages_only_advance_and_freeze_while_cancelling() {
    let mut progress = LaunchProgress::new(42);
    assert_eq!(progress.stage, LaunchStage::Preparing);
    assert!(!progress.advance(LaunchStage::Preparing));
    assert!(progress.advance(LaunchStage::Helper));
    assert!(!progress.advance(LaunchStage::Preparing));
    progress.cancelling = true;
    assert!(!progress.advance(LaunchStage::Running));
    assert_eq!(progress.stage, LaunchStage::Helper);
}

#[test]
fn launch_progress_serializes_for_the_ui() {
    let progress = LaunchProgress::new(42);
    assert_eq!(
        serde_json::to_value(progress).unwrap(),
        json!({
            "stage": "preparing",
            "startedAt": 42,
            "cancelling": false,
        })
    );
    let mut running = LaunchProgress::new(7);
    running.advance(LaunchStage::Running);
    assert_eq!(
        serde_json::to_value(&running).unwrap()["stage"],
        json!("running")
    );
}
