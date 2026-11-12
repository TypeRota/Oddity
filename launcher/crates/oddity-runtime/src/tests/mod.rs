//! oddity-runtime 单元测试.

use crate::{
    ActivitySnapshot, ChannelDescriptor, CommandDispatch, CommandGroup, DispatchFuture, EventSink,
    GamePackage, HostContext, InstallLocator, InstallQuery, LauncherInfo, MeteredPolicy,
    PackageDescriptor, Registry, SettingsSnapshot, Task, TaskJob, TaskRegistry, UpdatePreferences,
    UpdateWindow, ValidationIssue,
};
use oddity_contract::{Audience, COMMANDS};
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct TestHost {
    settings: SettingsSnapshot,
    activity: ActivitySnapshot,
    events: Arc<RecordingSink>,
    tasks: Arc<TaskRegistry>,
}

impl TestHost {
    fn new() -> Self {
        Self {
            settings: SettingsSnapshot {
                game: "test".into(),
                distribution: "cn".into(),
                language: "zh-CN".into(),
                reduce_motion: false,
                theme: "light".into(),
                install_root: Some("/games/test".into()),
                preload_root: Some("/games/test/preload".into()),
                update: UpdatePreferences {
                    auto_download: true,
                    metered: MeteredPolicy::Pause,
                    limit_bytes_per_second: 4096,
                    window: Some(UpdateWindow {
                        start: "02:00".into(),
                        end: "06:00".into(),
                    }),
                },
            },
            activity: ActivitySnapshot {
                game_running: true,
                migration_repair_pending: true,
                ..ActivitySnapshot::default()
            },
            events: Arc::default(),
            tasks: Arc::default(),
        }
    }
}

impl HostContext for TestHost {
    fn settings(&self) -> Result<SettingsSnapshot, String> {
        Ok(self.settings.clone())
    }

    fn activity(&self) -> Result<ActivitySnapshot, String> {
        Ok(self.activity.clone())
    }

    fn official_launcher_root(&self) -> Option<PathBuf> {
        None
    }

    fn emit(&self, id: &'static str, payload: Value) -> Result<(), String> {
        self.events.record(id, payload);
        Ok(())
    }

    fn submit(&self, job: TaskJob) -> Task {
        let sink = self.events.clone();
        let task = Task::new(Arc::new(move |payload| sink.emit("task-progress", payload)));
        let task = self.tasks.track(task);
        std::thread::spawn(task.runner(job));
        task
    }

    fn cancel_task(&self, id: &str) -> bool {
        self.tasks.cancel(id)
    }
}

fn echo<'a>(host: &'a dyn HostContext, _command: &'a str, payload: Value) -> DispatchFuture<'a> {
    let snapshot = host.settings();
    Box::pin(async move {
        let snapshot = snapshot.map_err(|error| json!(error))?;
        Ok(json!({"echo": payload, "game": snapshot.game}))
    })
}

fn fail<'a>(_host: &'a dyn HostContext, _command: &'a str, payload: Value) -> DispatchFuture<'a> {
    Box::pin(async move { Err(payload) })
}

/// 没有命令、只用于检查包级钩子的测试包.
const TEST_PACKAGE: TestPackage = TestPackage {
    id: "test",
    commands: &[],
};

/// 测试包: 按包 id 拥有一个命令组.
struct TestPackage {
    id: &'static str,
    commands: &'static [&'static str],
}

impl GamePackage for TestPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: self.id,
            name: "测试包",
        }
    }

    fn register(&self, registry: &mut Registry) {
        registry.register(CommandGroup::for_game(
            self.id,
            "test-domain",
            self.commands,
            echo as CommandDispatch,
        ));
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => {}
        }
    }
}

/// 契约里由宿主处理的命令: 没有包归属的非窗口命令.
fn host_commands() -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|command| command.audience != Audience::Window && command.owner.is_none())
        .map(|command| command.id)
        .collect()
}

/// 宿主覆盖除指定命令外的全部宿主命令.
fn host_without(command: &str) -> Vec<&'static str> {
    host_commands()
        .into_iter()
        .filter(|candidate| *candidate != command)
        .collect()
}

#[test]
fn install_records_the_package_and_its_groups() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "test",
        commands: &["game_status"],
    });

    assert_eq!(registry.packages().len(), 1);
    assert_eq!(registry.packages()[0].id, "test");
    assert_eq!(registry.packages()[0].name, "测试包");
    assert_eq!(
        registry.package("test").map(|package| package.name),
        Some("测试包")
    );
    assert_eq!(registry.groups().len(), 1);
    assert_eq!(
        registry
            .group_for("test", "game_status")
            .map(CommandGroup::domain),
        Some("test-domain")
    );
}

#[test]
fn lookup_finds_the_owning_group() {
    let mut registry = Registry::new();
    registry.register(CommandGroup::for_game(
        "test",
        "player",
        &["game_status", "bootstrap"],
        echo,
    ));

    let group = registry
        .group_for("test", "bootstrap")
        .expect("命令组应已登记");
    assert_eq!(group.game(), Some("test"));
    assert_eq!(group.domain(), "player");
    assert_eq!(group.commands(), ["game_status", "bootstrap"]);
    assert!(group.handles("game_status"));
    assert!(!group.handles("launch_official"));
    assert!(registry.group_for("test", "not_a_command").is_none());
    assert!(registry.group_for("other", "bootstrap").is_none());
    assert_eq!(Registry::default().groups().len(), 0);
}

#[test]
fn group_resolution_prefers_the_exact_game_then_any_game() {
    let mut registry = Registry::new();
    registry.register(CommandGroup::shared("shared-player", &["bootstrap"], echo));
    registry.register(CommandGroup::for_game(
        "test",
        "test-player",
        &["bootstrap"],
        echo,
    ));

    assert_eq!(
        registry
            .group_for("test", "bootstrap")
            .map(CommandGroup::domain),
        Some("test-player")
    );
    assert_eq!(
        registry
            .group_for("other", "bootstrap")
            .map(CommandGroup::domain),
        Some("shared-player")
    );
    assert!(registry.group_for("other", "game_status").is_none());
    assert!(registry.claims("bootstrap"));
    assert!(!registry.claims("game_status"));
}

#[test]
fn dispatch_runs_the_group_handler() {
    let host = TestHost::new();

    let success = CommandGroup::for_game("test", "player", &["game_status"], echo);
    assert_eq!(
        block_on(success.dispatch(&host, "game_status", json!(1))),
        Ok(json!({"echo": 1, "game": "test"}))
    );

    let failure = CommandGroup::shared("player", &["game_status"], fail);
    assert_eq!(
        block_on(failure.dispatch(&host, "game_status", json!(7))),
        Err(json!(7))
    );
}

#[test]
fn full_host_coverage_leaves_nothing_unclaimed() {
    let registry = Registry::new();
    assert_eq!(registry.validate(&host_commands()), Ok(()));
}

#[test]
fn missing_host_coverage_is_reported() {
    let mut host = host_commands();
    let missing = host.pop().expect("契约里应有宿主命令");
    let registry = Registry::new();
    assert_eq!(
        registry.validate(&host).unwrap_err(),
        [ValidationIssue::Unclaimed { command: missing }]
    );
}

#[test]
fn commands_of_unassembled_packages_are_exempt_from_unclaimed() {
    // NTE 不在装配表里, 它的命令跟着缺席; fixture 的命令由自己的组认领.
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "fixture",
        commands: &["fixture_echo", "fixture_info", "fixture_fail"],
    });
    assert_eq!(registry.validate(&host_commands()), Ok(()));
}

#[test]
fn commands_of_an_assembled_package_still_need_a_claim() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "nte",
        commands: &[],
    });
    let expected: Vec<ValidationIssue> = COMMANDS
        .iter()
        .filter(|command| command.audience != Audience::Window && command.owner == Some("nte"))
        .map(|command| ValidationIssue::Unclaimed {
            command: command.id,
        })
        .collect();
    assert_eq!(registry.validate(&host_commands()).unwrap_err(), expected);
}

/// 构造一条只用于 owner 校验的契约命令.
fn command_with_owner(audience: Audience, owner: Option<&'static str>) -> oddity_contract::Command {
    oddity_contract::Command {
        id: "test_command",
        audience,
        current_game: false,
        owner,
        args: oddity_contract::TsType::UNDEFINED,
        result: oddity_contract::TsType::NULL,
    }
}

#[test]
fn misspelled_command_owner_is_rejected() {
    let command = command_with_owner(Audience::Renderer, Some("nte-typo"));
    assert_eq!(
        crate::registry::owner_issue(&command),
        Some(ValidationIssue::InvalidOwner {
            command: "test_command",
            owner: "nte-typo",
        })
    );
}

#[test]
fn empty_command_owner_is_rejected() {
    let command = command_with_owner(Audience::Renderer, Some(""));
    assert_eq!(
        crate::registry::owner_issue(&command),
        Some(ValidationIssue::InvalidOwner {
            command: "test_command",
            owner: "",
        })
    );
}

#[test]
fn window_commands_cannot_declare_an_owner() {
    let command = command_with_owner(Audience::Window, Some("nte"));
    assert_eq!(
        crate::registry::owner_issue(&command),
        Some(ValidationIssue::WindowOwner {
            command: "test_command",
            owner: "nte",
        })
    );
}

#[test]
fn a_package_group_can_claim_its_own_commands() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "fixture",
        commands: &["fixture_echo", "fixture_info", "fixture_fail"],
    });
    assert_eq!(registry.validate(&host_commands()), Ok(()));
}

#[test]
fn host_and_group_conflict_is_reported() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "fixture",
        commands: &["fixture_echo", "fixture_info", "fixture_fail"],
    });
    let mut host = host_commands();
    host.push("fixture_echo");
    let error = registry.validate(&host).unwrap_err();
    assert_eq!(
        error,
        [ValidationIssue::Duplicate {
            command: "fixture_echo",
            game: None,
            sources: vec!["host", "test-domain"],
        }]
    );
    assert!(error[0]
        .to_string()
        .contains("同时由宿主与命令组 test-domain"));
}

#[test]
fn two_shared_groups_cannot_claim_the_same_command() {
    let mut registry = Registry::new();
    registry.register(CommandGroup::shared("first", &["game_status"], echo));
    registry.register(CommandGroup::shared("second", &["game_status"], echo));
    assert_eq!(
        registry.validate(&host_without("game_status")).unwrap_err(),
        [ValidationIssue::Duplicate {
            command: "game_status",
            game: None,
            sources: vec!["first", "second"],
        }]
    );
}

#[test]
fn same_command_in_one_game_is_duplicate() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "fixture",
        commands: &["fixture_echo", "fixture_info", "fixture_fail"],
    });
    registry.register(CommandGroup::for_game(
        "fixture",
        "extra",
        &["fixture_echo"],
        echo,
    ));
    assert_eq!(
        registry.validate(&host_commands()).unwrap_err(),
        [ValidationIssue::Duplicate {
            command: "fixture_echo",
            game: Some("fixture"),
            sources: vec!["test-domain", "extra"],
        }]
    );
}

#[test]
fn a_group_cannot_claim_another_packages_command() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "test",
        commands: &[],
    });
    registry.register(CommandGroup::for_game(
        "test",
        "test-extra",
        &["attendance_status"],
        echo,
    ));
    assert_eq!(
        registry.validate(&host_commands()).unwrap_err(),
        [ValidationIssue::OwnerMismatch {
            command: "attendance_status",
            group: "test-extra",
            game: Some("test"),
            owner: Some("nte"),
        }]
    );
}

#[test]
fn a_group_bound_to_an_unassembled_game_is_rejected() {
    let mut registry = Registry::new();
    registry.register(CommandGroup::for_game("ghost", "ghost", &[], echo));
    assert_eq!(
        registry.validate(&host_commands()).unwrap_err(),
        [ValidationIssue::UnknownGroupGame {
            group: "ghost",
            game: "ghost",
        }]
    );
}

/// 没有任何安装知识的安装域, 只为满足"有安装域"的装配条件.
struct EmptyLocator;

static EMPTY_LOCATOR: EmptyLocator = EmptyLocator;

impl InstallLocator for EmptyLocator {
    fn launcher_root(&self, _query: &InstallQuery) -> Option<PathBuf> {
        None
    }

    fn launcher_dir(&self, _query: &InstallQuery) -> Option<&'static str> {
        None
    }

    fn selected_install_root(&self, _query: &InstallQuery) -> PathBuf {
        PathBuf::new()
    }

    fn effective_install_root(
        &self,
        _query: &InstallQuery,
        _launcher_root: Option<&Path>,
    ) -> PathBuf {
        PathBuf::new()
    }

    fn game_exe(&self, _launcher_root: &Path) -> Option<PathBuf> {
        None
    }

    fn game_version(&self, _launcher_root: &Path) -> String {
        String::new()
    }

    fn launcher_info(&self, _launcher_root: &Path) -> Option<LauncherInfo> {
        None
    }

    fn game_exe_hint(&self) -> String {
        String::new()
    }
}

/// 可注入渠道与安装域的测试包.
struct ChannelPackage {
    channels: &'static [ChannelDescriptor],
    locator: bool,
}

impl GamePackage for ChannelPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "channels",
            name: "渠道测试包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn channels(&self) -> &'static [ChannelDescriptor] {
        self.channels
    }

    fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
        self.locator.then_some(&EMPTY_LOCATOR)
    }
}

fn channel_registry(channels: &'static [ChannelDescriptor]) -> Registry {
    let mut registry = Registry::new();
    registry.install(&ChannelPackage {
        channels,
        locator: false,
    });
    registry
}

#[test]
fn package_with_an_install_domain_must_declare_a_channel() {
    let mut registry = Registry::new();
    registry.install(&ChannelPackage {
        channels: &[],
        locator: true,
    });
    let issues = registry.validate(&host_commands()).unwrap_err();
    assert_eq!(
        issues,
        [ValidationIssue::Channel {
            game: "channels",
            reason: "有安装域但没有声明渠道".into(),
        }]
    );
    assert!(issues[0].to_string().contains("游戏 channels"));
}

#[test]
fn channel_declarations_are_validated_at_assembly() {
    static INVALID: &[ChannelDescriptor] = &[
        ChannelDescriptor {
            id: "CN",
            label: "国服",
            is_default: false,
            install_root_key: "cn",
        },
        ChannelDescriptor {
            id: "cn",
            label: "",
            is_default: false,
            install_root_key: "bad key",
        },
    ];
    let issues = channel_registry(INVALID)
        .validate(&host_commands())
        .unwrap_err();
    assert!(issues.iter().any(|issue| matches!(
        issue,
        ValidationIssue::Channel { reason, .. } if reason.contains("渠道 id CN")
    )));
    assert!(issues.iter().any(|issue| matches!(
        issue,
        ValidationIssue::Channel { reason, .. } if reason.contains("展示名为空")
    )));
    assert!(issues.iter().any(|issue| matches!(
        issue,
        ValidationIssue::Channel { reason, .. } if reason.contains("默认渠道数量为 0")
    )));
}

#[test]
fn duplicate_channel_ids_are_rejected() {
    static DUPLICATE: &[ChannelDescriptor] = &[
        ChannelDescriptor {
            id: "cn",
            label: "官服",
            is_default: true,
            install_root_key: "cn",
        },
        ChannelDescriptor {
            id: "cn",
            label: "B服",
            is_default: false,
            install_root_key: "cn_bilibili",
        },
    ];
    let issues = channel_registry(DUPLICATE)
        .validate(&host_commands())
        .unwrap_err();
    assert_eq!(
        issues,
        [ValidationIssue::Channel {
            game: "channels",
            reason: "渠道 id 重复: cn".into(),
        }]
    );
}

#[test]
fn shared_install_root_keys_are_allowed() {
    static CHANNELS: &[ChannelDescriptor] = &[
        ChannelDescriptor {
            id: "cn",
            label: "官服",
            is_default: true,
            install_root_key: "cn",
        },
        ChannelDescriptor {
            id: "cn_bilibili",
            label: "B服",
            is_default: false,
            install_root_key: "cn",
        },
    ];
    assert_eq!(
        channel_registry(CHANNELS).validate(&host_commands()),
        Ok(())
    );
}

#[test]
fn duplicate_package_id_is_rejected() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "same",
        commands: &[],
    });
    registry.install(&TestPackage {
        id: "same",
        commands: &[],
    });
    assert_eq!(
        registry.validate(&host_commands()).unwrap_err(),
        [ValidationIssue::DuplicatePackage { id: "same" }]
    );
}

#[test]
fn group_claims_outside_the_contract_are_rejected() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "test",
        commands: &["not_a_command"],
    });
    assert_eq!(
        registry.validate(&host_commands()).unwrap_err(),
        [ValidationIssue::InvalidClaim {
            command: "not_a_command",
            group: "test-domain",
        }]
    );
}

#[test]
fn window_commands_belong_to_the_window_layer() {
    let mut registry = Registry::new();
    registry.install(&TestPackage {
        id: "test",
        commands: &["win_close"],
    });
    assert_eq!(
        registry.validate(&host_commands()).unwrap_err(),
        [ValidationIssue::InvalidClaim {
            command: "win_close",
            group: "test-domain",
        }]
    );
}

#[test]
fn issues_explain_themselves() {
    let unclaimed = ValidationIssue::Unclaimed {
        command: "game_status",
    };
    assert!(unclaimed.to_string().contains("game_status"));

    let duplicate = ValidationIssue::Duplicate {
        command: "game_status",
        game: Some("test"),
        sources: vec!["first", "second"],
    };
    assert!(duplicate.to_string().contains("在游戏 test 下"));
    assert!(duplicate.to_string().contains("first, second"));

    let host_conflict = ValidationIssue::Duplicate {
        command: "game_status",
        game: None,
        sources: vec!["host", "player"],
    };
    assert!(host_conflict
        .to_string()
        .contains("同时由宿主与命令组 player"));

    let invalid = ValidationIssue::InvalidClaim {
        command: "win_close",
        group: "player",
    };
    assert!(invalid.to_string().contains("win_close"));

    let unknown = ValidationIssue::UnknownGroupGame {
        group: "player",
        game: "ghost",
    };
    assert!(unknown.to_string().contains("未装配的游戏 ghost"));

    let package = ValidationIssue::DuplicatePackage { id: "test" };
    assert!(package.to_string().contains("游戏包 id 重复: test"));

    let missing_owner = ValidationIssue::InvalidOwner {
        command: "game_status",
        owner: "nte-typo",
    };
    assert!(missing_owner.to_string().contains("nte-typo"));

    let empty_owner = ValidationIssue::InvalidOwner {
        command: "game_status",
        owner: "",
    };
    assert!(empty_owner.to_string().contains("包归属为空"));

    let window_owner = ValidationIssue::WindowOwner {
        command: "win_close",
        owner: "nte",
    };
    assert!(window_owner.to_string().contains("窗口命令 win_close"));

    let mismatch = ValidationIssue::OwnerMismatch {
        command: "attendance_status",
        group: "fixture",
        game: Some("fixture"),
        owner: Some("nte"),
    };
    assert!(mismatch.to_string().contains("包归属是 nte"));

    let shared_mismatch = ValidationIssue::OwnerMismatch {
        command: "attendance_status",
        group: "shared",
        game: None,
        owner: None,
    };
    assert!(shared_mismatch.to_string().contains("通用命令组 shared"));
}

mod capability;
mod headless;
mod host;
mod install;
mod launch;
mod runtime;
mod settings;
mod task;
mod tasks;

/// 记录事件的测试出口.
#[derive(Default)]
struct RecordingSink(Mutex<Vec<(&'static str, Value)>>);

impl RecordingSink {
    fn record(&self, id: &'static str, payload: Value) {
        self.0.lock().unwrap().push((id, payload));
    }

    fn recorded(&self) -> Vec<(&'static str, Value)> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for RecordingSink {
    fn deliver(
        &self,
        event: &'static oddity_contract::Event,
        payload: Value,
    ) -> Result<(), String> {
        self.record(event.id, payload);
        Ok(())
    }
}

/// 输出已关闭的测试出口.
struct ClosedSink;

impl EventSink for ClosedSink {
    fn deliver(&self, _: &'static oddity_contract::Event, _: Value) -> Result<(), String> {
        Err("输出已关闭".into())
    }
}

#[test]
fn emit_delivers_the_serialized_payload() {
    let sink = RecordingSink::default();
    sink.emit("game-state", json!({"running": true}))
        .expect("已声明的事件应能发送");
    sink.emit("playtime-started", Value::Null)
        .expect("空载荷应能发送");
    assert_eq!(
        sink.recorded(),
        [
            ("game-state", json!({"running": true})),
            ("playtime-started", Value::Null),
        ]
    );
}

#[test]
fn emit_rejects_events_outside_the_contract() {
    let sink = RecordingSink::default();
    let error = sink.emit("not-an-event", Value::Null).unwrap_err();
    assert!(
        error.contains("事件未在契约里声明: not-an-event"),
        "{error}"
    );
    assert!(sink.recorded().is_empty());
}

#[test]
fn emit_surfaces_a_closed_outlet() {
    let error = ClosedSink.emit("host-show", Value::Null).unwrap_err();
    assert_eq!(error, "输出已关闭");
}
