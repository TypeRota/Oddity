//! 下载域与宿主设置/启动的集成用例.
//!
//! 包内已经覆盖下载命令与状态; 这里只保留需要宿主设置的互斥语义.

use crate::channel::ChannelId;
use crate::testing::{reset, serial, set_package_busy};

#[test]
fn read_only_checks_do_not_block_launch_or_settings() {
    // 保存设置会按 HOME/XDG 写盘; 与其它设置用例共用环境锁, 避免写进别人的临时主目录.
    let _lock = serial();
    let mut env = crate::tests::EnvGuard::new();
    let home = crate::tests::TempDir::new("download-settings");
    env.set("HOME", home.join("home"));
    env.set("XDG_CONFIG_HOME", home.join("config"));
    env.set("XDG_DATA_HOME", home.join("data"));
    reset();
    let app = crate::service::test_service();
    // 只读资源检查: 正在跑, 但不写盘, 不阻塞启动与设置保存.
    set_package_busy(true, false);
    let settings = app.state.settings.lock().unwrap().clone();
    let mut next = settings.clone();
    *next.current_mut() = crate::tests::settings_with_game(
        ChannelId::new("cn"),
        Some(std::path::PathBuf::from("/tmp/install")),
        None,
    )
    .current()
    .clone();
    let section = crate::settings::update_settings_section(
        &app,
        "game:nte".into(),
        serde_json::json!({
            "installRoots": {"cn": "/tmp/install"},
        }),
    );
    assert!(section.is_ok(), "{section:?}");
    let mut registry = crate::session::Registry::default();
    assert!(crate::session::prepare_launch(&crate::testing::packages(), &mut registry).is_ok());
    reset();
}

#[test]
fn automatic_check_uses_detection_without_persisting_it() {
    let _lock = serial();
    reset();
    let mut env = crate::tests::EnvGuard::new();
    let dir = crate::tests::TempDir::new("detect-without-persist");
    dir.write(
        "NTELauncher/Config/Config.ini",
        "[General]
GameID=1289
[VERSION]
Version=1.0
",
    );
    env.set("HOME", dir.join("home"));
    env.set("XDG_CONFIG_HOME", dir.join("config"));
    env.set("XDG_DATA_HOME", dir.join("data"));
    env.set("NTE_ROOT", dir.path());
    let settings = crate::settings::Settings::default();
    let detected = crate::install::launcher_root(&crate::testing::packages(), &settings);
    assert!(detected.is_some(), "应检测到测试安装");
    let install = crate::install::effective_install_root(
        &crate::testing::packages(),
        &settings,
        detected.as_deref(),
    );
    assert_eq!(install, dir.path());
    // 端口探测只作为本轮任务的输入, 不写回设置文件.
    assert!(settings.current().install_root_for_key("cn").is_none());
    assert!(crate::settings::load()
        .current()
        .install_root_for_key("cn")
        .is_none());
    reset();
}
