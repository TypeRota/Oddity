#![cfg(feature = "game-nte")]

use std::process::Command;

/// systemd 单元用 `--taygedo-attendance` 启动同一可执行文件; 没有账号时退出 1.
#[test]
fn headless_attendance_exits_when_no_account_is_logged_in() {
    let directory = std::env::temp_dir().join(format!("oddity-attendance-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let backend =
        std::env::var("CARGO_BIN_EXE_oddity-backend").expect("CARGO_BIN_EXE_oddity-backend");
    let output = Command::new(&backend)
        .arg("--taygedo-attendance")
        .arg("--taygedo-config")
        .arg(&directory)
        .env("HOME", &directory)
        .env("XDG_CONFIG_HOME", &directory)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/oddity-attendance-bus",
        )
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("请先登录塔吉多账号"), "{stderr}");
    std::fs::remove_dir_all(&directory).unwrap();
}
