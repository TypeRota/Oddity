//! fixture 包的装载、契约与分发测试.

use super::{FixturePackage, FIXTURE_PACKAGE};
use oddity_contract::{Audience, COMMANDS};
use oddity_runtime::{
    ActivitySnapshot, GamePackage, HostContext, Registry, SettingsSnapshot, Task, TaskJob,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;

struct TestHost;

impl HostContext for TestHost {
    fn settings(&self) -> Result<SettingsSnapshot, String> {
        Err("测试宿主不提供设置".into())
    }

    fn activity(&self) -> Result<ActivitySnapshot, String> {
        Ok(ActivitySnapshot::default())
    }

    fn official_launcher_root(&self) -> Option<PathBuf> {
        None
    }

    fn emit(&self, _id: &'static str, _payload: Value) -> Result<(), String> {
        Ok(())
    }

    fn submit(&self, _job: TaskJob) -> Task {
        Task::new(Arc::new(|_| Ok(())))
    }

    fn cancel_task(&self, _id: &str) -> bool {
        // 测试宿主不执行任务, 没有可取消的登记.
        false
    }
}

fn loaded() -> Registry {
    let mut registry = Registry::new();
    registry.install(&FIXTURE_PACKAGE);
    registry
}

fn backend_commands() -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|command| command.audience != Audience::Window)
        .map(|command| command.id)
        .collect()
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("构建测试运行时")
        .block_on(future)
}

#[test]
fn package_declares_its_capability_and_settings_section() {
    let registry = loaded();
    assert_eq!(registry.capabilities("fixture").unwrap().len(), 1);
    registry
        .validate_capabilities()
        .expect("fixture 的能力与依赖应装配通过");

    let schema = registry
        .settings_schema("game:fixture")
        .expect("fixture 应声明设置 section");
    assert_eq!(schema.owner, "game:fixture");
    assert_eq!(schema.version, 2);
    assert_eq!(
        schema
            .ordered_fields()
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>(),
        ["nickname", "showGreeting", "greetingStyle", "maxRecords"]
    );
}

#[test]
fn settings_defaults_validate_and_migrate_from_version_one() {
    let schema = crate::settings::SCHEMAS[0];
    let mut values = schema.defaults();
    assert_eq!(values["nickname"], "旅行者");
    assert_eq!(values["maxRecords"], 5);
    schema.validate_values(&values).expect("默认设置应通过校验");

    values.insert("maxRecords".into(), json!(21));
    assert!(schema
        .validate_values(&values)
        .unwrap_err()
        .contains("必须在 1 到 20 之间"));
    values.insert("maxRecords".into(), json!(5));
    values.insert("nickname".into(), json!("  "));
    assert!(schema
        .validate_values(&values)
        .unwrap_err()
        .contains("测试昵称不能为空"));

    let mut old: Map<String, Value> = [("displayName".to_string(), json!("旧昵称"))]
        .into_iter()
        .collect();
    schema.migrate_from(1, &mut old).expect("版本 1 应迁移");
    schema.fill_defaults(&mut old);
    schema.validate_values(&old).expect("迁移后应通过校验");
    assert_eq!(old["nickname"], "旧昵称");
    assert!(!old.contains_key("displayName"));
}

#[test]
fn package_loads_with_its_descriptor_and_two_command_groups() {
    let registry = loaded();
    let descriptor = <FixturePackage as GamePackage>::descriptor(&FIXTURE_PACKAGE);
    assert_eq!(descriptor.id, "fixture");
    assert_eq!(descriptor.name, "测试游戏");
    assert_eq!(registry.packages(), [descriptor]);

    let groups: Vec<(Option<&str>, &str, Vec<&str>)> = registry
        .groups()
        .iter()
        .map(|group| (group.game(), group.domain(), group.commands().to_vec()))
        .collect();
    assert_eq!(
        groups,
        [
            (
                Some("fixture"),
                "fixture",
                vec!["fixture_echo", "fixture_fail"]
            ),
            (Some("fixture"), "fixture-info", vec!["fixture_info"]),
        ]
    );
}

#[test]
fn fixture_package_owns_its_two_groups() {
    let registry = loaded();
    assert_eq!(registry.groups().len(), 2);
    assert!(
        registry
            .groups()
            .iter()
            .all(|group| group.game() == Some("fixture")),
        "fixture 的命令组只属于 fixture"
    );
    assert!(registry.group_for("fixture", "fixture_echo").is_some());
    assert!(registry.group_for("nte", "fixture_echo").is_none());
}

#[test]
fn package_commands_are_declared_for_the_renderer() {
    for id in ["fixture_echo", "fixture_info", "fixture_fail"] {
        let declared =
            oddity_contract::command(id).unwrap_or_else(|| panic!("契约缺少 fixture 命令: {id}"));
        assert_eq!(declared.audience, Audience::Renderer, "{id}");
        assert!(!declared.current_game, "{id}");
    }
}

#[test]
fn echo_returns_the_message_and_its_character_count() {
    let registry = loaded();
    let group = registry
        .group_for("fixture", "fixture_echo")
        .expect("fixture 组应接管回显");
    let result = block_on(group.dispatch(&TestHost, "fixture_echo", json!({"message": "你好"})))
        .expect("回显应成功");
    assert_eq!(result, json!({"message": "你好", "length": 2}));
}

#[test]
fn info_returns_fixed_package_data() {
    let registry = loaded();
    let group = registry
        .group_for("fixture", "fixture_info")
        .expect("信息组应接管查询");
    let result =
        block_on(group.dispatch(&TestHost, "fixture_info", json!({}))).expect("固定数据应成功");
    assert_eq!(
        result,
        json!({
            "game": "fixture",
            "name": "测试游戏",
            "capabilities": ["info", "echo", "failure-injection"],
        })
    );
}

#[test]
fn failure_injection_returns_a_structured_error() {
    let registry = loaded();
    let group = registry
        .group_for("fixture", "fixture_fail")
        .expect("fixture 组应接管失败注入");
    let error = block_on(group.dispatch(&TestHost, "fixture_fail", json!({"message": "注入失败"})))
        .expect_err("失败注入必须返回错误");
    assert_eq!(
        error,
        json!({"code": "fixture_failure", "message": "注入失败"})
    );
}

#[test]
fn malformed_and_unknown_commands_are_rejected() {
    let registry = loaded();
    let group = registry
        .group_for("fixture", "fixture_echo")
        .expect("fixture 组应接管回显");

    let missing = block_on(group.dispatch(&TestHost, "fixture_echo", json!({}))).unwrap_err();
    assert!(
        missing
            .as_str()
            .unwrap()
            .contains("missing field `message`"),
        "{missing}"
    );

    let extra = block_on(group.dispatch(
        &TestHost,
        "fixture_echo",
        json!({"message": "你好", "extra": true}),
    ))
    .unwrap_err();
    assert!(
        extra.as_str().unwrap().contains("unknown field `extra`"),
        "{extra}"
    );

    let unknown = block_on(group.dispatch(&TestHost, "unknown", json!({}))).unwrap_err();
    assert_eq!(unknown, json!("无法识别的 fixture 命令"));
}

#[test]
fn package_leaves_host_commands_to_the_host_and_validates() {
    let registry = loaded();
    let claimed: Vec<&str> = registry
        .groups()
        .iter()
        .flat_map(|group| group.commands().iter().copied())
        .collect();
    assert_eq!(claimed, ["fixture_echo", "fixture_fail", "fixture_info"]);

    let host: Vec<&str> = backend_commands()
        .into_iter()
        .filter(|command| !claimed.contains(command))
        .collect();
    registry
        .validate(&host)
        .expect("组与宿主覆盖全部后端命令时装配校验应通过");

    let error = registry
        .validate(&backend_commands())
        .expect_err("宿主重复声明组命令时装配校验应失败");
    let message = error
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(message.contains("fixture_echo"), "{message}");
    assert!(
        message.contains("命令 fixture_echo 同时由宿主与命令组 fixture 声明"),
        "{message}"
    );
}
