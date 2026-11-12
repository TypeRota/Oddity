//! 无头入口的实际转发: 用法, 未知命令与 `--game` 校验.

use std::process::Command;

fn binary() -> String {
    std::env::var("CARGO_BIN_EXE_nte-cli").expect("CARGO_BIN_EXE_nte-cli")
}

#[test]
fn prints_usage_without_arguments() {
    let output = Command::new(binary()).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("nte-cli recovery"), "{text}");
    assert!(text.contains("nte-cli attendance"), "{text}");
    assert!(text.contains("game-preload"), "{text}");
    assert!(text.contains("--game <游戏 id>"), "{text}");
}

#[test]
fn unknown_commands_still_require_a_root() {
    let output = Command::new(binary())
        .arg("not-a-command")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("missing --root"));
}

#[test]
fn rejects_games_without_an_install_domain_before_loading_the_config() {
    let output = Command::new(binary())
        .args(["doctor", "--root", "/nonexistent", "--game", "hsr"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("无法识别游戏: hsr"), "{error}");
    assert!(error.contains("可选值: nte."), "{error}");
}

#[test]
fn accepted_game_reads_the_launcher_config() {
    let output = Command::new(binary())
        .args(["doctor", "--root", "/nonexistent", "--game", "nte"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Config/Config.ini"), "{error}");
}
