//! 减法构建的进程冒烟: 不装配《异环》时宿主仍能启动并处理请求.
//!
//! 只在装配 fixture、不装配 NTE 时构建, 见 Cargo.toml.

#![cfg(all(feature = "game-fixture", not(feature = "game-nte")))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

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

fn spawn(directory: &Path) -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let backend =
        std::env::var("CARGO_BIN_EXE_oddity-backend").expect("CARGO_BIN_EXE_oddity-backend");
    let mut child = Command::new(backend)
        .arg("--ipc")
        .env("HOME", directory)
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("XDG_DATA_HOME", directory.join("data"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/oddity-subtraction-bus",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动后端");
    let input = child.stdin.take().expect("打开 stdin");
    let output = BufReader::new(child.stdout.take().expect("打开 stdout"));
    (child, input, output)
}

/// 读一行回复; 中间出现的服务事件直接跳过.
fn read_reply(lines: &mut impl Iterator<Item = std::io::Result<String>>, id: u64) -> Value {
    loop {
        let line = lines
            .next()
            .expect("宿主提前关闭 stdout")
            .expect("读取宿主回复");
        let value: Value = serde_json::from_str(&line).expect("宿主回复是 JSON");
        if value.get("id") == Some(&json!(id)) {
            return value;
        }
        assert!(value.get("event").is_some(), "{value}");
    }
}

fn call(
    input: &mut ChildStdin,
    lines: &mut impl Iterator<Item = std::io::Result<String>>,
    id: u64,
    command: &str,
) -> Value {
    call_with_args(input, lines, id, command, json!({}))
}

fn call_with_args(
    input: &mut ChildStdin,
    lines: &mut impl Iterator<Item = std::io::Result<String>>,
    id: u64,
    command: &str,
    args: Value,
) -> Value {
    writeln!(
        input,
        "{}",
        json!({"id": id, "command": command, "args": args})
    )
    .expect("写入请求");
    read_reply(lines, id)
}

/// 关闭 stdin 后宿主应正常退出.
fn wait_for_exit(mut child: Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("等待后端") {
            assert!(status.success(), "{status:?}");
            return;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("stdin 关闭后宿主未退出");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn missing_nte_assembly_starts_and_reports_its_commands_unavailable() {
    let directory = TempDir::new("default-game");
    let (child, mut input, output) = spawn(directory.path());
    let mut lines = output.lines();

    let bootstrap = call(&mut input, &mut lines, 1, "bootstrap");
    assert_eq!(bootstrap["result"]["settings"]["activeGame"], "nte");
    assert_eq!(bootstrap["result"]["ok"], json!(false), "{bootstrap}");

    for (id, command, expected) in [
        (
            2,
            "attendance_status",
            "游戏包 nte 未装配, 不处理命令 attendance_status.",
        ),
        (
            3,
            "fixture_info",
            "当前游戏《nte》未装配, 不处理命令 fixture_info.",
        ),
    ] {
        let reply = call(&mut input, &mut lines, id, command);
        assert_eq!(reply["error"], json!(expected), "{reply}");
    }

    drop(input);
    wait_for_exit(child);
}

#[test]
fn fixture_package_routes_while_nte_commands_are_unavailable() {
    let directory = TempDir::new("fixture-game");
    write_settings(directory.path(), "fixture");
    let (child, mut input, output) = spawn(directory.path());
    let mut lines = output.lines();

    let bootstrap = call(&mut input, &mut lines, 1, "bootstrap");
    assert_eq!(bootstrap["result"]["settings"]["activeGame"], "fixture");
    assert_eq!(bootstrap["result"]["ok"], json!(false), "{bootstrap}");

    let info = call(&mut input, &mut lines, 2, "fixture_info");
    assert_eq!(info["result"]["game"], "fixture", "{info}");
    assert_eq!(info["result"]["name"], "测试游戏", "{info}");

    let attendance = call(&mut input, &mut lines, 3, "attendance_status");
    assert_eq!(
        attendance["error"],
        json!("《测试游戏》不提供命令 attendance_status."),
        "{attendance}"
    );

    let status = call(&mut input, &mut lines, 4, "game_status");
    assert_eq!(status["result"]["running"], json!(false), "{status}");

    drop(input);
    wait_for_exit(child);
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
    let (child, mut input, output) = spawn(directory.path());
    let mut lines = output.lines();

    let bootstrap = call(&mut input, &mut lines, 1, "bootstrap");
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

    let updated = call_with_args(
        &mut input,
        &mut lines,
        2,
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

    let rejected = call_with_args(
        &mut input,
        &mut lines,
        3,
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

    drop(input);
    wait_for_exit(child);
}
