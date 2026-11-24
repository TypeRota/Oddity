//! Sunshine 应用注册: 直接改写应用列表, 不经过管理接口.

use std::sync::Mutex;

use crate::cross_screen::{self, Applications, Client};
use crate::settings::Settings;
use crate::AppState;

fn managed_state(settings: Settings, game: crate::session::Registry) -> AppState {
    AppState {
        settings: Mutex::new(settings),
        game: Mutex::new(game),
        packages: crate::testing::packages(),
    }
}

fn applications(names: &[&str]) -> Applications {
    Applications {
        apps: names
            .iter()
            .map(|name| serde_json::json!({ "name": name }))
            .collect(),
        rest: serde_json::Map::new(),
    }
}

#[test]
fn applications_are_keyed_by_name_so_user_order_and_other_fields_survive() {
    let mut apps = applications(&["Desktop", "Oddity - 异环"]);
    apps.apps[1] = serde_json::json!({"name": "Oddity - 异环", "cmd": "old"});
    cross_screen::upsert(
        &mut apps,
        "Oddity - 异环",
        serde_json::json!({"name": "Oddity - 异环", "cmd": "new"}),
    );
    assert_eq!(apps.apps.len(), 2);
    assert_eq!(apps.apps[0]["name"], "Desktop");
    assert_eq!(apps.apps[1]["cmd"], "new");

    cross_screen::upsert(
        &mut apps,
        "Oddity - 其他",
        serde_json::json!({"name": "Oddity - 其他"}),
    );
    assert_eq!(apps.apps.len(), 3);

    assert!(cross_screen::remove(&mut apps, "Oddity - 异环"));
    assert!(!cross_screen::remove(&mut apps, "Oddity - 异环"));
    assert_eq!(apps.apps.len(), 2);
}

#[test]
fn entries_carry_the_prepare_and_restore_pair() {
    let entry = cross_screen::app_entry("Oddity - 异环", "'/opt/Oddity/oddity'", "nte");
    assert_eq!(entry["cmd"], "'/opt/Oddity/oddity' run --game 'nte'");
    assert_eq!(
        entry["prep-cmd"][0]["do"],
        "'/opt/Oddity/oddity' prepare --game 'nte'"
    );
    assert_eq!(entry["prep-cmd"][0]["undo"], "'/opt/Oddity/oddity' restore");
    // 主命令必须等游戏退出, 否则会话一结束就会恢复显示.
    assert_eq!(entry["auto-detach"], false);
    assert_eq!(entry["wait-all"], false);
}

#[test]
fn an_existing_entry_with_a_different_command_is_a_conflict() {
    let mut apps = applications(&["Oddity - 异环"]);
    // 没有命令的同名条目是我们自己留下的, 可以直接接管.
    assert!(!cross_screen::conflict(&apps, "Oddity - 异环", "new"));
    apps.apps[0] = serde_json::json!({"name": "Oddity - 异环", "cmd": "old"});
    assert!(!cross_screen::conflict(&apps, "Oddity - 异环", "old"));
    assert!(cross_screen::conflict(&apps, "Oddity - 异环", "new"));
    assert!(!cross_screen::conflict(&apps, "Oddity - 其他", "new"));
}

#[test]
fn supported_requires_kde_on_wayland() {
    assert!(cross_screen::desktop_supported("KDE", true));
    assert!(cross_screen::desktop_supported("ubuntu:KDE", true));
    assert!(!cross_screen::desktop_supported("KDE", false));
    assert!(!cross_screen::desktop_supported("GNOME", true));
    assert!(!cross_screen::desktop_supported("", true));
}

#[test]
fn sunshine_config_is_read_as_key_value_lines() {
    let mut env = crate::tests::EnvGuard::new();
    std::env::remove_var("CONFIGURATION_DIRECTORY");
    let dir = crate::tests::TempDir::new("cross-screen-config");
    let sunshine = dir.path().join("sunshine");
    std::fs::create_dir_all(&sunshine).unwrap();
    std::fs::write(
        sunshine.join("sunshine.conf"),
        "# 注释\ncapture = kwin\noutput_name = DP-3\n\nfile_apps = /tmp/apps.json\n",
    )
    .unwrap();
    env.set("XDG_CONFIG_HOME", dir.path());
    assert_eq!(cross_screen::config_value("capture").unwrap(), "kwin");
    assert_eq!(cross_screen::config_value("output_name").unwrap(), "DP-3");
    assert_eq!(cross_screen::config_value("missing").unwrap(), "");
    assert_eq!(
        cross_screen::apps_path().unwrap(),
        std::path::PathBuf::from("/tmp/apps.json")
    );
}

#[test]
fn a_missing_config_directory_reads_as_empty() {
    let mut env = crate::tests::EnvGuard::new();
    std::env::remove_var("CONFIGURATION_DIRECTORY");
    let dir = crate::tests::TempDir::new("cross-screen-empty");
    env.set("XDG_CONFIG_HOME", dir.path());
    assert_eq!(cross_screen::config_value("capture").unwrap(), "");
    assert_eq!(
        cross_screen::apps_path().unwrap(),
        dir.path().join("sunshine/apps.json")
    );
}

#[test]
fn applications_round_trip_through_the_apps_file() {
    let dir = crate::tests::TempDir::new("cross-screen-apps");
    let path = dir.path().join("apps.json");
    std::fs::write(
        &path,
        r#"{"env":{"PATH":"/bin"},"apps":[{"name":"Desktop"}]}"#,
    )
    .unwrap();
    let mut apps = cross_screen::read_apps(&path).unwrap();
    assert_eq!(apps.rest["env"]["PATH"], "/bin");
    cross_screen::upsert(
        &mut apps,
        "Oddity - 异环",
        serde_json::json!({"name":"Oddity - 异环"}),
    );
    cross_screen::write_apps(&path, &apps).unwrap();
    let reread = cross_screen::read_apps(&path).unwrap();
    assert_eq!(reread.apps.len(), 2);
    // env 等其它字段必须原样保留, 否则 Sunshine 会丢掉全局环境.
    assert_eq!(reread.rest["env"]["PATH"], "/bin");
    assert!(!dir.path().join("apps.json.tmp").exists());
    assert!(cross_screen::read_apps(&dir.path().join("missing.json")).is_err());
}

#[test]
fn configuring_requires_an_idle_session() {
    let mut app = crate::service::test_service();
    std::sync::Arc::get_mut(&mut app).unwrap().state = managed_state(
        Settings::default(),
        crate::session::Registry {
            starting: true,
            ..Default::default()
        },
    );
    assert_eq!(
        cross_screen::configure(&app, true).unwrap_err().to_string(),
        "请先结束游戏再更改跨屏游玩设置"
    );
    assert_eq!(
        cross_screen::configure(&app, false)
            .unwrap_err()
            .to_string(),
        "请先结束游戏再更改跨屏游玩设置"
    );
}

#[test]
fn client_requests_are_bounded_and_bound_to_the_launch_copy() {
    let client = Client {
        width: 1920,
        height: 1080,
        fps: 60,
        output_width: 2560,
        output_height: 1440,
    };
    client.validate().unwrap();
    let settings = Settings::default();
    let mut launch = settings.clone();
    client.apply(&mut launch);
    assert_eq!(launch.current().game_mode.gamescope.game_width, 1920);
    assert_eq!(launch.current().game_mode.gamescope.output_width, 2560);
    assert_eq!(launch.current().game_mode.gamescope.fps, 60);
    assert!(launch.current().game_mode.inhibit);
    assert!(!settings.current().game_mode.gamescope.enabled);

    for width in [0, 319, 7681] {
        let oversized = Client { width, ..client };
        assert!(oversized.validate().is_err());
    }
    for fps in [0, 241] {
        let wrong = Client { fps, ..client };
        assert!(wrong.validate().is_err());
    }
}

#[test]
fn a_client_request_is_rejected_when_framing_is_missing() {
    // Unix 域套接字路径有 108 字节上限; TMPDIR 可能很长 (CI 会指到构建目录).
    let Ok(dir) = tempfile::Builder::new()
        .prefix("oddity-sock")
        .tempdir_in("/tmp")
    else {
        return;
    };
    let path = dir.path().join("s.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        use std::io::Write;
        let _ = socket.write_all(b"{}\n");
    });
    assert!(cross_screen::connect(&path).is_err());
}

/// Sunshine 配置目录、应用列表写入与协议入口的联动.
#[test]
fn configuring_writes_and_removes_the_sunshine_entry() {
    use crate::service::test_service;
    let mut env = crate::tests::EnvGuard::new();
    std::env::remove_var("CONFIGURATION_DIRECTORY");
    std::env::remove_var("APPIMAGE");
    let dir = crate::tests::TempDir::new("cross-screen-configure");
    let sunshine = dir.path().join("sunshine");
    std::fs::create_dir_all(&sunshine).unwrap();
    std::fs::write(
        sunshine.join("sunshine.conf"),
        "capture = kwin\noutput_name = DP-1\n",
    )
    .unwrap();
    std::fs::write(
        sunshine.join("apps.json"),
        r#"{"apps":[{"name":"Desktop"}],"env":{"PATH":"/bin"}}"#,
    )
    .unwrap();
    env.set("XDG_CONFIG_HOME", dir.path());
    env.set("XDG_DATA_HOME", dir.path());
    // 只有 KDE Wayland 才继续往下走; 不能依赖运行测试的机器恰好是 KDE.
    env.set("XDG_CURRENT_DESKTOP", "KDE");
    env.set("WAYLAND_DISPLAY", "wayland-test");
    // 串流组件不在 PATH 上, 唯一的失败点留给组件检查.
    env.set("PATH", dir.path());

    let mut app: crate::service::ServiceHandle = test_service();
    let settings = Settings {
        active_game: crate::identity::GameId::from("nte"),
        ..Default::default()
    };
    std::sync::Arc::get_mut(&mut app).unwrap().state = managed_state(settings, Default::default());

    // 组件缺失时给出可执行的提示, 且不写入任何条目.
    let error = cross_screen::configure(&app, true).unwrap_err().to_string();
    assert_eq!(error, "请先安装串流组件");
    assert_eq!(
        cross_screen::read_apps(&sunshine.join("apps.json"))
            .unwrap()
            .apps
            .len(),
        1
    );

    // 关闭是幂等的: 没有登记过也返回当前状态.
    let status = cross_screen::configure(&app, false).unwrap();
    assert!(!status.enabled);
    assert!(
        status.supported
            == cross_screen::desktop_supported(
                &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
                std::env::var_os("WAYLAND_DISPLAY").is_some()
            )
    );
}

/// 串流请求: 没有握手就拒绝启动, 不会退回普通启动.
#[test]
fn an_external_launch_with_a_broken_stream_socket_is_rejected() {
    use crate::cli::{LaunchMode, LaunchRequest};
    use crate::identity::GameId;
    let app = crate::service::test_service();
    let dir = crate::tests::TempDir::new("cross-screen-external");
    let missing = dir.path().join("absent.sock");
    let request = LaunchRequest {
        game: GameId::from("nte"),
        region: None,
        mode: LaunchMode::Background,
        environment: None,
        cross_screen_socket: Some(missing),
    };
    let error = crate::service::block_on(crate::external_launch::start_external_launch(
        &app, &request,
    ))
    .unwrap_err();
    assert!(!error.is_empty());
    // 启动状态不能被半途失败的串流请求占住.
    assert!(!app.state.game.lock().unwrap().starting);
}

#[test]
fn entry_arguments_are_strict() {
    let args = |items: &[&str]| {
        items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
    };
    assert!(matches!(
        cross_screen::parse_entry(&args(&["restore"])).unwrap(),
        cross_screen::Entry::Restore
    ));
    for (input, mode, expected_game) in [
        (vec!["run", "--game", "nte"], "run", "nte"),
        (vec!["prepare", "--game", "nte"], "prepare", "nte"),
        (vec!["run", "--game", "a-b1"], "run", "a-b1"),
    ] {
        match cross_screen::parse_entry(&args(&input)).unwrap() {
            cross_screen::Entry::Stream { mode: got, game } => {
                assert_eq!(got, mode);
                assert_eq!(game, expected_game);
            }
            cross_screen::Entry::Restore => panic!("应为串流子命令"),
        }
    }
    for invalid in [
        vec![],
        vec!["restore", "extra"],
        vec!["run", "--game"],
        vec!["run", "nte", "nte"],
        vec!["launch", "--game", "nte"],
        vec!["run", "--game", ""],
        vec!["run", "--game", "有 空格"],
        vec!["run", "--game", "../etc"],
    ] {
        assert!(
            cross_screen::parse_entry(&args(&invalid)).is_err(),
            "{invalid:?}"
        );
    }
}

#[test]
fn client_environment_is_read_with_actionable_errors() {
    let env = |values: Vec<(&'static str, &'static str)>| {
        move |key: &str| {
            values
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    };
    let client = cross_screen::Client::from_env(env(vec![
        ("SUNSHINE_CLIENT_WIDTH", "1280"),
        ("SUNSHINE_CLIENT_HEIGHT", "720"),
        ("SUNSHINE_CLIENT_FPS", "60"),
    ]))
    .unwrap();
    assert_eq!((client.width, client.height, client.fps), (1280, 720, 60));
    assert_eq!((client.output_width, client.output_height), (1920, 1080));

    let missing = cross_screen::Client::from_env(env(vec![
        ("SUNSHINE_CLIENT_HEIGHT", "720"),
        ("SUNSHINE_CLIENT_FPS", "60"),
    ]))
    .unwrap_err()
    .to_string();
    assert!(missing.contains("SUNSHINE_CLIENT_WIDTH"), "{missing}");
    assert!(missing.contains("Moonlight"), "{missing}");

    let invalid = cross_screen::Client::from_env(env(vec![
        ("SUNSHINE_CLIENT_WIDTH", "wide"),
        ("SUNSHINE_CLIENT_HEIGHT", "720"),
        ("SUNSHINE_CLIENT_FPS", "60"),
    ]))
    .unwrap_err()
    .to_string();
    assert!(invalid.contains("不是有效的数值"), "{invalid}");

    // 超出支持范围的请求同样被拒绝, 不会交给 Gamescope.
    assert!(cross_screen::Client::from_env(env(vec![
        ("SUNSHINE_CLIENT_WIDTH", "20000"),
        ("SUNSHINE_CLIENT_HEIGHT", "720"),
        ("SUNSHINE_CLIENT_FPS", "60"),
    ]))
    .is_err());
}

#[test]
fn install_commands_follow_the_distribution() {
    let arch = cross_screen::install_args("ID=cachyos\nID_LIKE=arch\n").unwrap();
    assert_eq!(arch[0], "/usr/bin/pacman");
    assert!(arch.contains(&"sunshine"));
    assert!(arch.contains(&"gamescope"));

    let fedora = cross_screen::install_args("ID=fedora\n").unwrap();
    assert_eq!(fedora[0], "/usr/bin/dnf");

    let error = cross_screen::install_args("ID=debian\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("暂不支持"), "{error}");
}

/// 条件齐全时写入条目, 关闭时移除, 并保留 Sunshine 的其它字段.
#[test]
fn a_complete_setup_registers_and_removes_the_entry() {
    use crate::channel::ChannelId;
    use crate::environment::{ProtonPin, RuntimeEnvironment};

    let _lock = crate::testing::serial();
    let mut env = crate::tests::EnvGuard::new();
    std::env::remove_var("CONFIGURATION_DIRECTORY");
    let dir = crate::tests::TempDir::new("cross-screen-full");
    let sunshine = dir.path().join("sunshine");
    std::fs::create_dir_all(&sunshine).unwrap();
    std::fs::write(
        sunshine.join("sunshine.conf"),
        "capture = kwin\noutput_name = DP-1\n",
    )
    .unwrap();
    std::fs::write(
        sunshine.join("apps.json"),
        r#"{"apps":[{"name":"Desktop"}],"env":{"PATH":"/bin"}}"#,
    )
    .unwrap();
    env.set("XDG_CONFIG_HOME", dir.path());
    env.set("XDG_DATA_HOME", dir.path());

    // 串流组件都在 PATH 上; KScreen 返回一块可切换的主屏.
    let bin = dir.path().join("bin");
    for name in ["sunshine", "gamescope", "systemctl"] {
        crate::tests::write_script(&bin, name, "#!/bin/sh\nexit 0\n");
    }
    crate::tests::write_script(
        &bin,
        "kscreen-doctor",
        "#!/bin/sh\nprintf '%s' '{\"outputs\":[{\"name\":\"DP-1\",\"connected\":true,\"enabled\":true,\"priority\":1,\"currentModeId\":\"1\",\"modes\":[{\"id\":\"1\",\"size\":{\"width\":1920,\"height\":1080},\"refreshRate\":60}],\"pos\":{\"x\":0,\"y\":0},\"scale\":1.0}]}'\nexit 0\n",
    );
    env.set("PATH", &bin);
    env.set("WAYLAND_DISPLAY", "wayland-test");
    env.set("XDG_CURRENT_DESKTOP", "KDE");
    env.set("APPIMAGE", "/opt/Oddity/Oddity.AppImage");

    // 测试安装域按 <安装根>/cn/Launcher 布局定位官方目录.
    let install_root = dir.path().join("games/cn");
    let launcher = install_root.join("NTELauncher");
    std::fs::create_dir_all(launcher.join("Config")).unwrap();
    std::fs::write(
        launcher.join("Config/Config.ini"),
        "[General]\nLauncher=Launcher.exe\nGameID=0\n",
    )
    .unwrap();

    let app = crate::service::test_service();
    {
        let mut settings = app.state.settings.lock().unwrap();
        settings.active_game = crate::identity::GameId::from("nte");
        let game = settings.current_mut();
        game.distribution = Some(ChannelId::new("cn"));
        game.install_roots
            .insert("cn".into(), install_root.display().to_string());
        let environment = RuntimeEnvironment {
            id: "a".repeat(32),
            name: "Env".into(),
            proton: ProtonPin {
                path: std::path::PathBuf::from("/tmp/proton"),
                identity: "identity".into(),
            },
            install_root: install_root.clone(),
            game: crate::identity::GameId::from("nte"),
            created_at: 0,
        };
        settings.runtime_environments = vec![environment.clone()];
        settings
            .current_mut()
            .set_active_environment("cn", Some(&environment.id));
    }

    let status = cross_screen::configure(&app, true).unwrap();
    assert!(status.enabled, "{}", status.message);
    let apps = cross_screen::read_apps(&sunshine.join("apps.json")).unwrap();
    assert_eq!(apps.apps.len(), 2);
    let entry = &apps.apps[1];
    assert_eq!(entry["name"], "Oddity - 异环");
    assert_eq!(
        entry["cmd"],
        "'/opt/Oddity/Oddity.AppImage' run --game 'nte'"
    );
    // Sunshine 的全局环境必须原样保留.
    assert_eq!(apps.rest["env"]["PATH"], "/bin");
    assert!(cross_screen::load("nte").unwrap().is_some());

    let status = cross_screen::configure(&app, false).unwrap();
    assert!(!status.enabled);
    let apps = cross_screen::read_apps(&sunshine.join("apps.json")).unwrap();
    assert_eq!(apps.apps.len(), 1);
    assert_eq!(apps.apps[0]["name"], "Desktop");
    assert!(cross_screen::load("nte").unwrap().is_none());
    crate::testing::reset();
}

/// 同名但命令不同的条目不会被覆盖.
#[test]
fn an_unknown_entry_with_the_same_name_is_not_replaced() {
    use crate::channel::ChannelId;
    use crate::environment::{ProtonPin, RuntimeEnvironment};

    let _lock = crate::testing::serial();
    let mut env = crate::tests::EnvGuard::new();
    std::env::remove_var("CONFIGURATION_DIRECTORY");
    let dir = crate::tests::TempDir::new("cross-screen-conflict");
    let sunshine = dir.path().join("sunshine");
    std::fs::create_dir_all(&sunshine).unwrap();
    std::fs::write(
        sunshine.join("sunshine.conf"),
        "capture = kwin\noutput_name = DP-1\n",
    )
    .unwrap();
    std::fs::write(
        sunshine.join("apps.json"),
        r#"{"apps":[{"name":"Oddity - 异环","cmd":"other"}]}"#,
    )
    .unwrap();
    env.set("XDG_CONFIG_HOME", dir.path());
    env.set("XDG_DATA_HOME", dir.path());

    let bin = dir.path().join("bin");
    for name in ["sunshine", "gamescope", "systemctl"] {
        crate::tests::write_script(&bin, name, "#!/bin/sh\nexit 0\n");
    }
    crate::tests::write_script(
        &bin,
        "kscreen-doctor",
        "#!/bin/sh\nprintf '%s' '{\"outputs\":[{\"name\":\"DP-1\",\"connected\":true,\"enabled\":true,\"priority\":1,\"currentModeId\":\"1\",\"modes\":[{\"id\":\"1\",\"size\":{\"width\":1920,\"height\":1080},\"refreshRate\":60}],\"pos\":{\"x\":0,\"y\":0},\"scale\":1.0}]}'\nexit 0\n",
    );
    env.set("PATH", &bin);
    env.set("WAYLAND_DISPLAY", "wayland-test");
    env.set("XDG_CURRENT_DESKTOP", "KDE");
    env.set("APPIMAGE", "/opt/Oddity/Oddity.AppImage");

    let install_root = dir.path().join("games/cn");
    let launcher = install_root.join("NTELauncher");
    std::fs::create_dir_all(launcher.join("Config")).unwrap();
    std::fs::write(
        launcher.join("Config/Config.ini"),
        "[General]\nLauncher=Launcher.exe\nGameID=0\n",
    )
    .unwrap();

    let app = crate::service::test_service();
    {
        let mut settings = app.state.settings.lock().unwrap();
        settings.active_game = crate::identity::GameId::from("nte");
        let game = settings.current_mut();
        game.distribution = Some(ChannelId::new("cn"));
        game.install_roots
            .insert("cn".into(), install_root.display().to_string());
        let environment = RuntimeEnvironment {
            id: "b".repeat(32),
            name: "Env".into(),
            proton: ProtonPin {
                path: std::path::PathBuf::from("/tmp/proton"),
                identity: "identity".into(),
            },
            install_root: install_root.clone(),
            game: crate::identity::GameId::from("nte"),
            created_at: 0,
        };
        settings.runtime_environments = vec![environment.clone()];
        settings
            .current_mut()
            .set_active_environment("cn", Some(&environment.id));
    }

    let error = cross_screen::configure(&app, true).unwrap_err().to_string();
    assert!(error.contains("同名"), "{error}");
    assert_eq!(
        cross_screen::read_apps(&sunshine.join("apps.json"))
            .unwrap()
            .apps[0]["cmd"],
        "other"
    );
    crate::testing::reset();
}

/// 握手必须收到启动器的两条回执, 否则按失败处理.
#[test]
fn handshake_requires_the_launcher_to_report_both_phases() {
    use std::io::{BufRead, Write};

    let client = cross_screen::Client {
        width: 1280,
        height: 720,
        fps: 60,
        output_width: 1920,
        output_height: 1080,
    };
    // 启动器先读请求, 再回 started / finished.
    let (host, peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let received = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(peer.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut writer = peer;
        writer.write_all(b"started\n").unwrap();
        writer.write_all(b"finished\n").unwrap();
        line
    });
    cross_screen::handshake(host, &client).unwrap();
    let request = received.join().unwrap();
    assert!(request.contains("\"width\":1280"), "{request}");

    // 只回 started 时必须报会话中断.
    let (host, peer) = std::os::unix::net::UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(peer.try_clone().unwrap());
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        let mut writer = peer;
        let _ = writer.write_all(b"started\n");
    });
    let error = cross_screen::handshake(host, &client)
        .unwrap_err()
        .to_string();
    assert!(error.contains("会话连接已中断"), "{error}");

    // 启动器没有报告开始就断开时给出启动失败.
    let (host, peer) = std::os::unix::net::UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(peer.try_clone().unwrap());
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        let mut writer = peer;
        let _ = writer.write_all(b"failed\n");
    });
    let error = cross_screen::handshake(host, &client)
        .unwrap_err()
        .to_string();
    assert!(error.contains("未能启动游戏"), "{error}");
}

/// CONFIGURATION_DIRECTORY 优先于 XDG 配置目录.
#[test]
fn the_service_configuration_directory_wins() {
    let mut env = crate::tests::EnvGuard::new();
    let dir = crate::tests::TempDir::new("cross-screen-service-dir");
    let service = dir.path().join("service");
    std::fs::create_dir_all(service.join("sunshine")).unwrap();
    std::fs::write(service.join("sunshine/sunshine.conf"), "capture = kwin\n").unwrap();
    env.set("CONFIGURATION_DIRECTORY", service.as_os_str());
    env.set("XDG_CONFIG_HOME", dir.path().join("ignored"));
    assert_eq!(
        cross_screen::apps_path().unwrap(),
        service.join("sunshine/apps.json")
    );
}

/// status 把不支持的环境说清楚, 不暴露底层错误.
#[test]
fn status_explains_an_unsupported_desktop() {
    let mut env = crate::tests::EnvGuard::new();
    env.set("XDG_CURRENT_DESKTOP", "GNOME");
    env.set("WAYLAND_DISPLAY", "wayland-test");
    let app = crate::service::test_service();
    let status = cross_screen::status(&app).unwrap();
    assert!(!status.enabled);
    assert!(!status.supported);
    assert_eq!(status.message, "当前仅支持 KDE Plasma Wayland");
}

/// 登记后显示校验失败时 status 降级为未开启并给出原因.
#[test]
fn status_clears_enabled_when_the_capture_screen_changed() {
    let mut env = crate::tests::EnvGuard::new();
    std::env::remove_var("CONFIGURATION_DIRECTORY");
    let dir = crate::tests::TempDir::new("cross-screen-status");
    let sunshine = dir.path().join("sunshine");
    std::fs::create_dir_all(&sunshine).unwrap();
    std::fs::write(
        sunshine.join("sunshine.conf"),
        "capture = kwin\noutput_name = HDMI-9\n",
    )
    .unwrap();
    std::fs::write(
        sunshine.join("apps.json"),
        r#"{"apps":[{"name":"Oddity - 异环"}]}"#,
    )
    .unwrap();
    env.set("XDG_CONFIG_HOME", dir.path());
    env.set("XDG_DATA_HOME", dir.path());
    let bin = dir.path().join("bin");
    for name in ["sunshine", "gamescope"] {
        crate::tests::write_script(&bin, name, "#!/bin/sh\nexit 0\n");
    }
    crate::tests::write_script(
        &bin,
        "kscreen-doctor",
        "#!/bin/sh\nprintf '%s' '{\"outputs\":[{\"name\":\"DP-1\",\"connected\":true,\"enabled\":true,\"priority\":1,\"currentModeId\":\"1\",\"modes\":[{\"id\":\"1\",\"size\":{\"width\":1920,\"height\":1080},\"refreshRate\":60}],\"pos\":{\"x\":0,\"y\":0},\"scale\":1.0}]}'\nexit 0\n",
    );
    env.set("PATH", &bin);
    env.set("XDG_CURRENT_DESKTOP", "KDE");
    env.set("WAYLAND_DISPLAY", "wayland-test");

    // 先写入一条登记记录, 但配置里的捕获屏幕并不存在.
    std::fs::create_dir_all(dir.path().join("oddity")).unwrap();
    std::fs::write(
        dir.path().join("oddity/sunshine-nte.json"),
        r#"{"game":"nte","name":"Oddity - 异环"}"#,
    )
    .unwrap();
    let app = crate::service::test_service();
    let status = cross_screen::status(&app).unwrap();
    assert!(!status.enabled);
    assert!(status.message.contains("无法确定"), "{}", status.message);
}
