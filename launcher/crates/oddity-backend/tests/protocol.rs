#![cfg(feature = "game-nte")]

//! 会话服务的协议用例: 命令, 错误, 事件与空闲退出.

mod support;

use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;

use support::Session;

/// 带日志与假 systemctl 的临时根目录.
fn fixture() -> (std::path::PathBuf, std::path::PathBuf) {
    let directory = std::env::temp_dir().join(format!("oddity-ipc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("data/oddity/logs")).unwrap();
    std::fs::write(
        directory.join("data/oddity/logs/test.log"),
        "协议导出测试\n",
    )
    .unwrap();
    (directory.clone(), directory.join("commands/systemctl"))
}

/// 发一条命令并等待回复.
fn call(session: &mut Session, command: &str, args: Value) -> Value {
    session.call(command, args)
}

#[test]
fn service_roundtrip_errors_events_and_idle_shutdown() {
    let (directory, systemctl) = fixture();
    std::fs::create_dir_all(directory.join("commands")).unwrap();
    std::fs::write(&systemctl, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut session = Session::start(&directory);
    let bootstrap = call(&mut session, "bootstrap", json!({}));
    assert!(bootstrap["result"]["settings"].is_object(), "{bootstrap}");
    let installed = call(
        &mut session,
        "update_settings_section",
        json!({
            "owner": "game:nte",
            "patch": {
                "installRoots": {
                    "cn": directory.join("installation-cn"),
                    "global": directory.join("installation-global"),
                },
            },
        }),
    );
    assert!(installed.get("result").is_some(), "{installed}");
    assert!(call(
        &mut session,
        "integrity_acknowledge",
        json!({"notice":"success"})
    )
    .get("result")
    .is_some());
    for command in [
        "attendance_login",
        "attendance_send_code",
        "attendance_login_sms",
        "attendance_configure",
        "attendance_logout",
        "game_download_start",
        "environment_create",
        "environment_action",
        "environment_import",
        "environment_orphan_delete",
        "browse_files",
        "game_mode_gpu_apply",
        "game_mode_tuning_apply",
        "gamepad_feedback",
        "gamepad_profile_validate",
        "gamepad_profile_load",
        "gamepad_profile_save",
        "save_log_export",
        "discard_log_export",
        "check_update",
        "fetch_bulletins",
        "update_settings_section",
        "integrity_acknowledge",
        "open_url",
        "open_directory",
        "playtime_export_poster",
        "environment_snapshot_compare",
        "environment_snapshot_save_reference",
        "host_state",
        "external_launch",
        "migration_abandon",
    ] {
        let reply = call(&mut session, command, json!(false));
        assert!(reply["error"].is_string(), "{command}: {reply}");
    }
    for command in [
        "launcher_background",
        "launcher_background_archive",
        "game_download_status",
        "game_download_cancel",
        "game_mode_gpu_info",
        "game_mode_tuning_info",
        "gamepad_poll",
        "gamepad_release_idle",
        "game_mode_status",
        "game_status",
        "take_launch_error",
        "cancel_launch",
        "close_game",
        "playtime_history",
        "environment_snapshot_overview",
        "environment_snapshot_clear_history",
        "environment_snapshot_delete_reference",
        "environment_audit",
        "attendance_status",
        "migration_retry",
    ] {
        let reply = call(&mut session, command, json!({}));
        assert!(reply.get("result").is_some(), "{command}: {reply}");
    }
    let audit = call(&mut session, "environment_audit", json!({}));
    assert!(audit["result"]["environments"].is_array(), "{audit}");
    assert!(audit["result"]["orphans"].is_array(), "{audit}");
    let orphan = "f".repeat(32);
    let orphan_path = directory
        .join("data/oddity/logs/environments")
        .join(&orphan);
    std::fs::create_dir_all(&orphan_path).unwrap();
    let removed = call(
        &mut session,
        "environment_orphan_delete",
        json!({"kind":"logs","name":orphan}),
    );
    assert!(removed.get("result").is_some(), "{removed}");
    assert!(!orphan_path.exists());
    for (command, args) in [
        ("unregistered", json!({})),
        (
            "external_launch",
            json!({"argv":["--launch","--region","invalid"],"cold":false}),
        ),
        (
            "environment_create",
            json!({"name":"test","protonPath":"/missing/proton","sourceId":null}),
        ),
        ("environment_action", json!({"action":"invalid","id":null})),
        (
            "environment_import",
            json!({"id":"missing","name":"导入","protonPath":"/missing/proton"}),
        ),
        (
            "environment_orphan_delete",
            json!({"kind":"other","name":"missing"}),
        ),
        (
            "game_download_start",
            json!({"mode":"invalid","full":false}),
        ),
        (
            "game_mode_gpu_apply",
            json!({"config":oddity_host::game_mode::gpu::Config::default()}),
        ),
        (
            "game_mode_tuning_apply",
            json!({"config":oddity_host::game_mode::tuning::Config::default()}),
        ),
        ("open_url", json!({"url":"file:///etc/passwd"})),
        ("open_directory", json!({"path":directory.join("missing")})),
        ("environment_snapshot_compare", json!({"target":"missing"})),
        (
            "playtime_export_poster",
            json!({"destination":directory.join("poster.png"),"data":"invalid"}),
        ),
    ] {
        let reply = call(&mut session, command, args);
        assert!(reply.get("error").is_some(), "{command}: {reply}");
    }
    let reference = call(
        &mut session,
        "environment_snapshot_save_reference",
        json!({"label":"测试参考","attemptId":null}),
    );
    assert_eq!(reference["result"]["label"], "测试参考", "{reference}");
    let listing = call(&mut session, "browse_files", json!({"path":directory}));
    assert!(listing["result"]["entries"].is_array(), "{listing}");
    let update = call(&mut session, "check_update", json!({"full":false}));
    assert_eq!(update["result"]["installed"], false, "{update}");
    let feedback = call(
        &mut session,
        "gamepad_feedback",
        json!({"deviceId":0,"effect":"stop"}),
    );
    assert!(feedback.get("result").is_some(), "{feedback}");
    let profile = oddity_host::gamepad::Profile::default();
    let profile_path = directory.join("profile.json");
    for (command, args) in [
        ("gamepad_profile_validate", json!({"profile":profile})),
        (
            "gamepad_profile_save",
            json!({"path":profile_path,"profile":profile}),
        ),
        ("gamepad_profile_load", json!({"path":profile_path})),
    ] {
        let reply = call(&mut session, command, args);
        assert!(reply.get("result").is_some(), "{command}: {reply}");
    }
    let prepared = call(&mut session, "prepare_log_export", json!({}));
    let saved = call(
        &mut session,
        "save_log_export",
        json!({"id":prepared["result"]["id"],"destination":directory}),
    );
    assert!(saved["result"].is_string(), "{saved}");
    assert!(std::path::Path::new(saved["result"].as_str().unwrap()).is_file());
    let prepared = call(&mut session, "prepare_log_export", json!({}));
    assert!(call(
        &mut session,
        "discard_log_export",
        json!({"id":prepared["result"]["id"]})
    )
    .get("result")
    .is_some());
    let host = call(
        &mut session,
        "update_settings_section",
        json!({"owner":"host","patch":{"theme":"dark"}}),
    );
    assert_eq!(host["result"]["fields"]["theme"], "dark", "{host}");
    assert!(host["result"]["fields"].get("games").is_none(), "{host}");
    let game = call(
        &mut session,
        "update_settings_section",
        json!({"owner":"game:nte","patch":{"distribution":"global","launchMode":"official"}}),
    );
    assert_eq!(game["result"]["fields"]["distribution"], "global", "{game}");
    assert_eq!(game["result"]["fields"]["launchMode"], "official", "{game}");
    let saved = call(&mut session, "bootstrap", json!({}));
    assert_eq!(saved["result"]["settings"]["theme"], "dark", "{saved}");
    assert_eq!(
        saved["result"]["settings"]["games"]["nte"]["launchMode"], "official",
        "{saved}"
    );
    for (command, args) in [
        ("attendance_refresh_game_records", json!({})),
        (
            "attendance_login",
            json!({"phone":"","password":"","rememberPassword":false}),
        ),
        ("attendance_send_code", json!({"phone":""})),
        (
            "attendance_login_sms",
            json!({"phone":"","code":"","deviceId":""}),
        ),
        ("attendance_run", json!({})),
    ] {
        let reply = call(&mut session, command, args);
        assert_eq!(reply["error"]["kind"], "service", "{reply}");
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("仅支持国服"));
    }
    let logout = call(
        &mut session,
        "attendance_logout",
        json!({"accountId":"missing"}),
    );
    assert!(logout.get("error").is_some(), "{logout}");
    let keyring = call(&mut session, "attendance_keyring_check", json!({}));
    assert!(keyring["error"].is_object(), "{keyring}");
    let preferences = call(
        &mut session,
        "attendance_configure",
        json!({"preferences":oddity_game_nte::attendance::Preferences::default()}),
    );
    assert!(preferences.get("result").is_some(), "{preferences}");
    let launch = call(&mut session, "launch_official", json!({}));
    assert!(launch["error"].is_string(), "{launch}");
    assert!(call(
        &mut session,
        "host_state",
        json!({"focused":false,"visible":false})
    )
    .get("result")
    .is_some());
    // 新调用带 game, 旧调用不带时仍按《异环》处理.
    let unknown = session.call_with_events("game_status", json!({}), |_| {});
    assert!(
        unknown.get("result").is_some() || unknown.get("error").is_some(),
        "{unknown}"
    );
    let unknown_game = session_game(&mut session, Some("genshin"));
    assert!(
        unknown_game["error"]
            .as_str()
            .unwrap()
            .contains("无法识别游戏: genshin"),
        "{unknown_game}"
    );
    let matching = session_game(&mut session, Some("nte"));
    assert!(matching.get("result").is_some(), "{matching}");
    let legacy = session_game(&mut session, None);
    assert!(legacy.get("result").is_some(), "{legacy}");
    session.disconnect();
}

/// 带或不带游戏 id 的请求; 旧调用不带 game 时按当前游戏处理.
fn session_game(session: &mut Session, game: Option<&str>) -> Value {
    let mut request = json!({"id": 9001, "command": "game_status", "args": {}});
    if let Some(game) = game {
        request["game"] = json!(game);
    }
    session.call_value(request)
}
