//! 无头 CLI 缝: 命令表收集, 按游戏查找与 typed 调用.

use super::TestPackage;
use crate::{
    GamePackage, HeadlessCli, HeadlessCommand, HeadlessRequest, PackageDescriptor, Registry,
    ValidationIssue,
};
use std::path::Path;

const COMMANDS: &[HeadlessCommand] = &[
    HeadlessCommand {
        name: "attendance",
        needs_root: false,
    },
    HeadlessCommand {
        name: "doctor",
        needs_root: true,
    },
];

const USAGE: &[&str] = &["attendance", "doctor --root <dir>"];

struct FakeCli;

static FAKE_CLI: FakeCli = FakeCli;

impl HeadlessCli for FakeCli {
    fn commands(&self) -> &'static [HeadlessCommand] {
        COMMANDS
    }

    fn usage(&self) -> &'static [&'static str] {
        USAGE
    }

    fn run(&self, request: &HeadlessRequest<'_>) -> Result<(), String> {
        match request.command {
            "doctor" if request.root.is_some() => Ok(()),
            command => Err(format!("未认领: {command}")),
        }
    }
}

/// 带无头 CLI 的测试包.
struct CliPackage(&'static str);

impl GamePackage for CliPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: self.0,
            name: "CLI 测试包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn headless_cli(&self) -> Option<&'static dyn HeadlessCli> {
        Some(&FAKE_CLI)
    }
}

#[test]
fn registry_collects_headless_clis_in_assembly_order() {
    let mut registry = Registry::new();
    registry.install(&CliPackage("first"));
    let plain = TestPackage {
        id: "test",
        commands: &[],
    };
    registry.install(&plain);
    registry.install(&CliPackage("second"));

    let entries: Vec<(&str, *const dyn HeadlessCli)> = registry
        .headless_clis()
        .map(|(game, cli)| (game, cli as *const dyn HeadlessCli))
        .collect();
    assert_eq!(
        entries,
        [
            (
                "first",
                &FAKE_CLI as *const FakeCli as *const dyn HeadlessCli
            ),
            (
                "second",
                &FAKE_CLI as *const FakeCli as *const dyn HeadlessCli
            ),
        ]
    );

    assert!(registry.headless_cli("first").is_some());
    assert!(registry.headless_cli("second").is_some());
    assert!(registry.headless_cli("test").is_none());
    assert!(registry.headless_cli("missing").is_none());
}

#[test]
fn package_without_a_cli_exposes_nothing() {
    let mut registry = Registry::new();
    let plain = TestPackage {
        id: "test",
        commands: &[],
    };
    registry.install(&plain);

    assert_eq!(registry.headless_clis().count(), 0);
    assert!(registry.headless_cli("test").is_none());
}

#[test]
fn duplicate_headless_commands_are_reported() {
    let mut registry = Registry::new();
    registry.install(&CliPackage("first"));
    registry.install(&CliPackage("second"));

    let issues = registry
        .validate(&[])
        .expect_err("两个包声明同一无头命令时应装配失败");
    let duplicates: Vec<&ValidationIssue> = issues
        .iter()
        .filter(|issue| matches!(issue, ValidationIssue::DuplicateHeadlessCommand { .. }))
        .collect();
    assert_eq!(duplicates.len(), 2);
    assert_eq!(
        duplicates[0].to_string(),
        "无头命令 attendance 被多个包声明: first, second"
    );
}

#[test]
fn headless_request_reaches_the_package() {
    let mut registry = Registry::new();
    registry.install(&CliPackage("game"));
    let cli = registry.headless_cli("game").expect("CLI 应已登记");

    assert_eq!(cli.commands(), COMMANDS);
    assert_eq!(cli.usage(), USAGE);

    let root = Path::new("/games");
    assert_eq!(
        cli.run(&HeadlessRequest {
            command: "doctor",
            root: Some(root),
            args: &[],
        }),
        Ok(())
    );
    assert_eq!(
        cli.run(&HeadlessRequest {
            command: "attendance",
            root: None,
            args: &[],
        }),
        Err("未认领: attendance".to_string())
    );
}
