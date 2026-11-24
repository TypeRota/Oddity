//! 减法构建的进程冒烟: 不装配《异环》时服务仍能启动并处理请求.
//!
//! 只在装配 fixture、不装配 NTE 时构建, 见 Cargo.toml.

#![cfg(all(feature = "game-fixture", not(feature = "game-nte")))]

mod support;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use support::Session;

/// 析构时删除的临时用户目录.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("oddity-subtraction-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时目录");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 写一份只指定当前游戏的设置, 其余字段走默认值.
fn write_settings(directory: &Path, active_game: &str) {
    let config = directory.join("config/oddity");
    std::fs::create_dir_all(&config).expect("创建设置目录");
    std::fs::write(
        config.join("settings.json"),
        format!(r#"{{"activeGame":"{active_game}","games":{{"{active_game}":{{}}}}}}"#),
    )
    .expect("写入设置");
}

#[test]
fn missing_nte_assembly_starts_and_reports_its_commands_unavailable() {
    let directory = TempDir::new("default-game");
    let mut session = Session::start(directory.path());

    let bootstrap = session.call("bootstrap", json!({}));
    assert_eq!(bootstrap["result"]["settings"]["activeGame"], "nte");
    assert_eq!(bootstrap["result"]["ok"], json!(false), "{bootstrap}");

    for (command, expected) in [
        (
            "attendance_status",
            "游戏包 nte 未装配, 不处理命令 attendance_status.",
        ),
        (
            "fixture_info",
            "当前游戏《nte》未装配, 不处理命令 fixture_info.",
        ),
    ] {
        let reply = session.call(command, json!({}));
        assert_eq!(reply["error"], json!(expected), "{reply}");
    }

    session.disconnect();
}

#[test]
fn fixture_package_routes_while_nte_commands_are_unavailable() {
    let directory = TempDir::new("fixture-game");
    write_settings(directory.path(), "fixture");
    let mut session = Session::start(directory.path());

    let bootstrap = session.call("bootstrap", json!({}));
    assert_eq!(bootstrap["result"]["settings"]["activeGame"], "fixture");
    assert_eq!(bootstrap["result"]["ok"], json!(false), "{bootstrap}");

    let info = session.call("fixture_info", json!({}));
    assert_eq!(info["result"]["game"], "fixture", "{info}");
    assert_eq!(info["result"]["name"], "测试游戏", "{info}");

    let attendance = session.call("attendance_status", json!({}));
    assert_eq!(
        attendance["error"],
        json!("《测试游戏》不提供命令 attendance_status."),
        "{attendance}"
    );

    let status = session.call("game_status", json!({}));
    assert_eq!(status["result"]["running"], json!(false), "{status}");

    session.disconnect();
}

#[test]
fn fixture_settings_round_trip_and_migrate_versions() {
    let directory = TempDir::new("fixture-settings");
    let config = directory.path().join("config/oddity");
    std::fs::create_dir_all(&config).expect("创建设置目录");
    std::fs::write(
        config.join("settings.json"),
        r#"{
            "activeGame": "fixture",
            "games": {"fixture": {}},
            "sections": {
                "game:fixture": {
                    "version": 1,
                    "values": {"displayName": "旧昵称"}
                }
            }
        }"#,
    )
    .expect("写入设置");
    let mut session = Session::start(directory.path());

    let bootstrap = session.call("bootstrap", json!({}));
    let schema = bootstrap["result"]["settingsSchemas"]
        .as_array()
        .expect("设置 schema 是数组")
        .iter()
        .find(|schema| schema["owner"] == "game:fixture")
        .expect("fixture 应声明设置 section");
    assert_eq!(schema["version"], json!(2));
    let field = |key: &str| {
        schema["fields"]
            .as_array()
            .expect("字段是数组")
            .iter()
            .find(|field| field["key"] == key)
            .expect("字段应存在")
    };
    assert_eq!(field("nickname")["value"], json!("旧昵称"));
    assert_eq!(field("showGreeting")["value"], json!(true));
    assert_eq!(field("maxRecords")["default"], json!(5));

    // 启动读取时迁移旧版本并立即写回, 不必等到下一次设置修改.
    let migrated: Value = serde_json::from_str(
        &std::fs::read_to_string(config.join("settings.json")).expect("读取设置"),
    )
    .expect("设置是 JSON");
    assert_eq!(migrated["sections"]["game:fixture"]["version"], json!(2));
    assert_eq!(
        migrated["sections"]["game:fixture"]["values"]["nickname"],
        json!("旧昵称")
    );

    let updated = session.call(
        "update_settings_section",
        json!({
            "owner": "game:fixture",
            "patch": {"nickname": "新昵称", "maxRecords": 9}
        }),
    );
    assert_eq!(
        updated["result"]["owner"],
        json!("game:fixture"),
        "{updated}"
    );
    assert_eq!(updated["result"]["fields"]["nickname"], json!("新昵称"));
    assert_eq!(updated["result"]["fields"]["maxRecords"], json!(9));

    let stored: Value = serde_json::from_str(
        &std::fs::read_to_string(config.join("settings.json")).expect("读取设置"),
    )
    .expect("设置是 JSON");
    assert_eq!(stored["sections"]["game:fixture"]["version"], json!(2));
    assert_eq!(
        stored["sections"]["game:fixture"]["values"]["nickname"],
        json!("新昵称")
    );
    assert_eq!(
        stored["sections"]["game:fixture"]["values"]["showGreeting"],
        json!(true)
    );
    assert!(stored["sections"]["game:fixture"]["values"]
        .get("displayName")
        .is_none());

    let rejected = session.call(
        "update_settings_section",
        json!({
            "owner": "game:fixture",
            "patch": {"maxRecords": 21}
        }),
    );
    assert!(
        rejected["error"]
            .as_str()
            .unwrap_or_default()
            .contains("必须在 1 到 20 之间"),
        "{rejected}"
    );

    session.disconnect();
}
