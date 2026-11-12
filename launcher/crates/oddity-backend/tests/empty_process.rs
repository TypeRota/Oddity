//! 空装配表下的进程冒烟: 宿主可启动, bootstrap 返回空态, 包命令不可用.
//!
//! 只在没有任何游戏包 feature 时构建, 见 Cargo.toml.

#![cfg(not(any(feature = "game-nte", feature = "game-fixture")))]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

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

#[test]
fn empty_assembly_binary_starts_reports_empty_state_and_exits_cleanly() {
    let directory = std::env::temp_dir().join(format!("oddity-empty-ipc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let backend =
        std::env::var("CARGO_BIN_EXE_oddity-backend").expect("CARGO_BIN_EXE_oddity-backend");
    let mut child = Command::new(&backend)
        .arg("--ipc")
        .env("HOME", &directory)
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("XDG_DATA_HOME", directory.join("data"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/oddity-empty-bus",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    writeln!(
        input,
        "{}",
        json!({"id": 1, "command": "bootstrap", "args": {}})
    )
    .unwrap();
    let reply = read_reply(&mut lines, 1);
    assert_eq!(reply["result"]["ok"], json!(false), "{reply}");
    assert_eq!(reply["result"]["launcherRoot"], json!(""), "{reply}");
    assert_eq!(reply["result"]["gameExists"], json!(false), "{reply}");
    let installations = reply["result"]["runtimeInstallations"]
        .as_array()
        .expect("安装清单是数组");
    assert!(
        installations
            .iter()
            .all(|installation| installation["installed"] == json!(false)),
        "{reply}"
    );

    writeln!(
        input,
        "{}",
        json!({"id": 2, "command": "fixture_info", "args": {}})
    )
    .unwrap();
    let reply = read_reply(&mut lines, 2);
    assert_eq!(
        reply["error"],
        json!("游戏包 fixture 未装配, 不处理命令 fixture_info."),
        "{reply}"
    );

    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{status:?}");
            break;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("stdin 关闭后空装配宿主未退出");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::remove_dir_all(&directory).unwrap();
}
