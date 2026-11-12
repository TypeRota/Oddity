//! 入口测试: 参数解析, `--game` 校验, 恢复入口与包转发.
//!
//! 更新命令的实现与输出在游戏包里, 这里只覆盖壳层.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "oddity-cli-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).expect("创建临时目录");
    path
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn flag_value_reads_the_following_argument() {
    let args: Vec<String> = ["--root", "/games", "--only", "a.bin", "--full"]
        .iter()
        .map(|value| value.to_string())
        .collect();
    assert_eq!(flag_value(&args, "--root").as_deref(), Some("/games"));
    assert_eq!(flag_value(&args, "--only").as_deref(), Some("a.bin"));
    assert_eq!(flag_value(&args, "--full"), None);
    assert_eq!(flag_value(&args, "--missing"), None);
    // 重复出现时取第一个
    let repeated: Vec<String> = ["--only", "a", "--only", "b"]
        .iter()
        .map(|value| value.to_string())
        .collect();
    assert_eq!(flag_value(&repeated, "--only").as_deref(), Some("a"));
    // 前缀相同不算命中
    assert_eq!(flag_value(&args, "--roo"), None);
}

#[test]
fn usage_always_succeeds() {
    usage(&registry()).expect("usage");
}

#[test]
fn standalone_commands_come_from_the_package_assembly() {
    let registry = registry();
    assert!(standalone_cli(&registry, "attendance").is_some());
    assert!(standalone_cli(&registry, "keyring-check").is_some());
    // 需要 --root 的更新命令不算独立命令.
    assert!(standalone_cli(&registry, "doctor").is_none());
    assert!(standalone_cli(&registry, "not-a-command").is_none());
}

#[test]
fn run_without_arguments_prints_usage() {
    run(Vec::new()).expect("usage");
}

#[test]
fn selectable_games_come_from_the_package_assembly() {
    assert_eq!(registry().selectable_games(), ["nte"]);
}

#[test]
fn selectable_game_uses_the_default_id_when_omitted() {
    assert_eq!(
        selectable_game(&[], &registry()).expect("默认游戏"),
        GameId::from("nte")
    );
    let args = vec!["--game".to_string(), "NTE".to_string()];
    assert_eq!(
        selectable_game(&args, &registry()).expect("显式游戏"),
        GameId::from("nte")
    );
}

#[test]
fn rejects_games_without_an_install_domain() {
    let registry = registry();
    for game in ["hsr", "genshin", "fixture"] {
        let args = vec!["--game".to_string(), game.to_string()];
        let error = selectable_game(&args, &registry)
            .expect_err("未装配的游戏应被拒绝")
            .to_string();
        assert_eq!(error, format!("无法识别游戏: {game}. 可选值: nte."));
    }
    let args = vec!["--game".to_string(), String::new()];
    assert!(selectable_game(&args, &registry)
        .expect_err("空 id 应被拒绝")
        .to_string()
        .contains("不能为空"));
}

#[test]
fn run_rejects_games_without_an_install_domain_before_loading_the_config() {
    let error = run(strings(&[
        "doctor",
        "--root",
        "/nonexistent",
        "--game",
        "hsr",
    ]))
    .expect_err("未装配的游戏应被拒绝");
    assert!(error.to_string().contains("无法识别游戏: hsr"), "{error}");
}

#[test]
fn run_dispatches_update_commands_to_the_package() {
    let error = run(strings(&[
        "doctor",
        "--root",
        "/nonexistent",
        "--game",
        "nte",
    ]))
    .expect_err("没有启动器配置应报错");
    assert!(
        format!("{error:#}").contains("Config/Config.ini"),
        "{error:#}"
    );
}

#[test]
fn update_commands_need_a_package_entry() {
    let error = game_command(
        &registry(),
        &GameId::from("fixture"),
        "doctor",
        Path::new("/nonexistent"),
        &[],
    )
    .expect_err("没有无头入口的包应报错");
    assert_eq!(error.to_string(), "《fixture》没有无头更新命令");
}

#[test]
fn unknown_update_commands_fall_back_to_usage_before_the_package_entry() {
    for game in ["nte", "fixture"] {
        game_command(
            &registry(),
            &GameId::from(game),
            "not-a-command",
            Path::new("/nonexistent"),
            &[],
        )
        .expect("未知命令应打印用法");
    }
}

#[test]
fn run_recovery_diagnose_reads_the_environment() {
    run(strings(&["recovery", "diagnose"])).expect("诊断只读, 应成功");
}

#[test]
fn run_recovery_requires_confirmation() {
    let error = run(strings(&["recovery", "stop"])).expect_err("未确认时应拒绝");
    assert!(
        error.to_string().contains("用法: nte-cli recovery"),
        "{error}"
    );
}

#[test]
fn run_attendance_reports_missing_login() {
    let dir = temp_dir("attendance");
    let args = vec![
        "attendance".to_string(),
        "--config".to_string(),
        dir.display().to_string(),
    ];
    let error = run(args).expect_err("没有账号时手动签到应失败");
    let _ = fs::remove_dir_all(&dir);
    // 具体文案由包内的签到测试断言, 这里只确认错误透传到命令行.
    assert!(error.to_string().contains("登录"), "{error}");
}
