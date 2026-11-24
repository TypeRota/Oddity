#![cfg(feature = "game-nte")]

//! 会话服务独立于界面: 界面断开不结束服务, 重新连接读到同一份权威状态.

mod support;

use std::time::Duration;

use serde_json::json;

use support::Session;

/// 进程是否还活着; 僵尸进程等待回收, 不算存活.
fn alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let state = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|state| state.chars().next());
    state.is_some_and(|state| state != 'Z')
}

/// 为用例准备一个隔离的用户目录.
fn directory(tag: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oddity-service-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn service_outlives_the_interface_and_keeps_its_state() {
    let root = directory("lifecycle");
    let mut first = Session::start(&root);
    let pid = first.pid();
    let bootstrap = first.call("bootstrap", json!({}));
    assert_eq!(bootstrap["result"]["settings"]["activeGame"], "nte");

    // 界面断开后服务仍在: 它是常驻的会话服务, 不是界面的子进程.
    first.close_connection();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid), "界面断开后服务不应立刻退出");

    // 第二个界面重新连上, 读到的是同一个服务实例.
    let mut second = Session::connect(&root);
    let status = second.call("game_status", json!({}));
    assert_eq!(status["result"]["running"], json!(false), "{status}");
    let bootstrap = second.call("bootstrap", json!({}));
    assert!(bootstrap["result"]["settings"].is_object(), "{bootstrap}");

    // 没有人连接且没有会话时, 服务自行退出.
    second.disconnect();
    // 第一个客户端只剩连接已断的句柄, 这里直接放掉, 由服务自己收尾.
    drop(first);
    wait_gone(pid);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_second_interface_joins_the_running_service() {
    let root = directory("single");
    let mut first = Session::start(&root);
    let pid = first.pid();

    let mut second = Session::connect(&root);
    assert_eq!(second.pid(), 0, "后连的界面不拉起新进程");
    assert_eq!(
        second.call("game_status", json!({}))["result"]["running"],
        json!(false)
    );

    // 还有界面连着, 服务不能退出.
    first.close_connection();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid), "仍有界面连接时服务不应退出");

    // 最后一个界面离开后服务才自行退出.
    second.disconnect();
    wait_gone(pid);
    drop(first);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn termination_signal_leaves_no_socket_behind() {
    let root = directory("signal");
    let session = Session::start(&root);
    let pid = session.pid();
    let entry = root.join("data/oddity/ipc-path");
    // 入口文件记录真实套接字路径; 真实套接字在短路径下.
    let socket = std::path::PathBuf::from(
        std::fs::read_to_string(&entry)
            .expect("入口文件应存在")
            .trim(),
    );
    assert!(entry.exists() && socket.exists());

    // 模拟玩家或系统结束服务: 走 SIGTERM, 与正常退出共用一条收尾路径.
    let status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("发送终止信号");
    assert!(status.success(), "发送 SIGTERM 失败");
    wait_gone(pid);
    assert!(!entry.exists(), "收到终止信号后入口文件必须清理");
    assert!(!socket.exists(), "收到终止信号后套接字必须清理");
    let _ = std::fs::remove_dir_all(&root);
}

/// 等待进程消失.
fn wait_gone(pid: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while alive(pid) {
        assert!(std::time::Instant::now() < deadline, "服务在超时后仍未退出");
        std::thread::sleep(Duration::from_millis(20));
    }
}
