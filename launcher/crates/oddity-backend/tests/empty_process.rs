//! 空装配表下的进程冒烟: 服务可启动, bootstrap 返回空态, 包命令不可用.
//!
//! 只在没有任何游戏包 feature 时构建, 见 Cargo.toml.

#![cfg(not(any(feature = "game-nte", feature = "game-fixture")))]

mod support;

use serde_json::json;

use support::Session;

#[test]
fn empty_assembly_service_starts_reports_empty_state_and_exits_cleanly() {
    let directory = std::env::temp_dir().join(format!("oddity-empty-ipc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let mut session = Session::start(&directory);

    let reply = session.call("bootstrap", json!({}));
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

    let reply = session.call("fixture_info", json!({}));
    assert_eq!(
        reply["error"],
        json!("游戏包 fixture 未装配, 不处理命令 fixture_info."),
        "{reply}"
    );

    // 断连后服务没有会话也没有任务, 自行退出.
    session.disconnect();
}
