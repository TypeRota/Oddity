//! 账号域测试: 加解密夹具, 会话帧与命令组路由.

use super::*;
use crate::auth::testing::{self, PHONE};
use crate::testing::{block_on, HttpServer, TestHost};
use serde_json::json;

#[test]
fn password_cipher_matches_independent_utf8_pkcs7_vector() {
    let key = b"0123456789abcdef0123456789abcdef";
    let encoded = encrypt(" test-密码 ".as_bytes(), key).unwrap();
    assert_eq!(encoded, "ZjR+NH5ynbxFpEw2YA0sIw==");
    assert_eq!(
        decrypt(encoded.as_bytes(), key).unwrap().as_slice(),
        " test-密码 ".as_bytes()
    );
    assert!(decrypt(b"invalid", key).is_err());
    assert!(decrypt(b"AAAAAAAAAAAAAAAAAAAAAA==", key).is_err());
}

#[test]
fn session_packet_matches_sdk_accepted_fixture() {
    let session = Session {
        uid: "synthetic-master-uid".into(),
        token: Zeroizing::new("synthetic-master-token".into()),
        extra_one: serde_json::json!({"fixture":"synthetic","userIp":"192.0.2.1","version":1}),
        record: Record {
            version: 1,
            account_id: "a".repeat(32),
            phone: "13800138000".into(),
            channel_uid: "synthetic-channel-uid".into(),
            channel_token: "synthetic-channel-token".into(),
            device: Device::new("b".repeat(32)).unwrap(),
        },
    };
    assert_eq!(
        session.packet().unwrap().as_slice(),
        include_bytes!("../../tests/fixtures/game-session.bin")
    );
    assert_eq!(session.record.summary().masked_phone, "138****8000");
    let mut oversize = session;
    oversize.token = Zeroizing::new("x".repeat(128));
    assert!(oversize.packet().is_err());
}

#[test]
fn session_packet_rejects_empty_or_unterminated_fields() {
    let mut session = testing::session();
    session.record = testing::record(&"a".repeat(32), "channel-1");
    session.extra_one = serde_json::json!({"userIp": "x".repeat(100)});
    assert_eq!(session.packet().unwrap_err().kind, "protocol");

    session.extra_one = serde_json::json!({"userIp": "192.0.2.1\0tail"});
    assert_eq!(session.packet().unwrap_err().kind, "protocol");

    session.extra_one = serde_json::json!({"version": 1});
    session.uid = String::new();
    assert_eq!(session.packet().unwrap_err().kind, "protocol");
}

#[test]
fn query_reply_preserves_real_restrictions_and_signed_error_code() {
    let body = br#"{"code":12001,"message":"restricted"}"#;
    let reply = query_reply(1, 7, -1, "校验失败", body).unwrap();
    assert_eq!(
        u32::from_le_bytes(reply[24..28].try_into().unwrap()),
        u32::MAX
    );
    assert_eq!(&reply[4 + 0x41c..4 + 0x41c + body.len()], body);
    assert!(query_reply(1, 7, 0, "", b"bad\0json").is_err());
    assert!(query_reply(1, 7, 0, "", &vec![b'a'; 65536]).is_err());
}

#[test]
fn account_summary_masks_only_mainland_numbers() {
    let mut record = testing::record(&"a".repeat(32), "channel-1");
    let summary = record.summary();
    assert_eq!(summary.account_id, record.account_id);
    assert_eq!(summary.masked_phone, "138****8000");
    record.phone = "not-a-phone".into();
    assert_eq!(record.summary().masked_phone, "游戏账号");
    let error = Device::new("z".repeat(32)).err().unwrap();
    assert_eq!(error.message, "设备编号无效, 请重新获取验证码.");
    assert_eq!(error.to_string(), error.message);
}

#[test]
fn cn_gate_rejects_other_games_and_regions_before_account_access() {
    for host in [
        TestHost::with_settings("nte", "global"),
        TestHost::with_settings("hsr", "cn"),
    ] {
        for (name, args) in [
            ("game_accounts", json!({})),
            ("game_send_code", json!({"phone": PHONE})),
            (
                "game_login",
                json!({"phone": PHONE, "secret": "123456", "password": false, "deviceId": ""}),
            ),
            ("game_account_remove", json!({"accountId": "a".repeat(32)})),
        ] {
            let error = block_on(super::command(&host, name, args)).unwrap_err();
            assert_eq!(error["kind"], json!("unsupported"), "{name}");
            assert_eq!(error["code"], json!("unsupported"), "{name}");
            assert!(error.get("reason").is_none(), "{name}: {error}");
            assert_eq!(
                error["message"],
                json!("直接启动和游戏账号登录目前仅支持《异环》国服, 请使用后台或前台启动."),
                "{name}"
            );
        }
    }
}

#[test]
fn settings_failures_and_missing_launcher_are_reported_before_network() {
    let error = block_on(super::command(
        &TestHost::failing_settings(),
        "game_accounts",
        json!({}),
    ))
    .unwrap_err();
    assert_eq!(error["kind"], json!("service"));
    assert_eq!(error["message"], json!("测试设置不可读"));
    assert!(error.get("code").is_none(), "{error}");
    assert!(error.get("reason").is_none(), "{error}");

    let host = TestHost::new(None);
    for (name, args) in [
        ("game_send_code", json!({"phone": PHONE})),
        (
            "game_login",
            json!({"phone": PHONE, "secret": "123456", "password": false, "deviceId": ""}),
        ),
    ] {
        let error = block_on(super::command(&host, name, args)).unwrap_err();
        assert_eq!(error["kind"], json!("unsupported"), "{name}");
        assert_eq!(error["code"], json!("capability_unavailable"), "{name}");
        assert_eq!(error["reason"], json!("launcher_missing"), "{name}");
        assert_eq!(
            error["message"],
            json!("请先安装官方组件和游戏资源."),
            "{name}"
        );
    }
}

#[test]
fn invalid_arguments_keep_the_host_error_payload() {
    let host = TestHost::with_settings("nte", "cn");
    for (name, args) in [
        ("game_send_code", json!({})),
        ("game_send_code", json!({"phone": PHONE, "extra": 1})),
        (
            "game_login",
            json!({"phone": PHONE, "secret": "123456", "password": true}),
        ),
        (
            "game_login",
            json!({"phone": PHONE, "secret": "123456", "password": true, "deviceId": "", "extra": 1}),
        ),
        ("game_account_remove", json!({"accountId": 1})),
        (
            "game_account_remove",
            json!({"accountId": "a".repeat(32), "extra": 1}),
        ),
    ] {
        let error = block_on(super::command(&host, name, args)).unwrap_err();
        assert_eq!(error, json!("游戏账号请求参数无效"), "{name}");
    }

    let error = block_on(super::command(&host, "unknown", json!({}))).unwrap_err();
    assert_eq!(error, json!("无法识别的游戏账号命令"));
}

#[test]
fn accounts_return_masked_summaries_from_the_injected_vault() {
    let server = HttpServer::start();
    let (_store, vault) = testing::vault();
    let mut saved = testing::record(&"a".repeat(32), "channel-1");
    vault.save(&mut saved).unwrap();
    let value = block_on(dispatch(
        &TestHost::with_settings("nte", "cn"),
        "game_accounts",
        json!({}),
        Deps {
            vault,
            endpoints: testing::endpoints(&server),
        },
    ))
    .unwrap();
    assert_eq!(
        value,
        json!([{"accountId": "a".repeat(32), "maskedPhone": "138****8000"}])
    );
}

#[test]
fn accounts_report_keyring_failures_as_the_host_payload() {
    let server = HttpServer::start();
    let (store, vault) = testing::vault();
    store.set_failing(true);
    let error = block_on(dispatch(
        &TestHost::with_settings("nte", "cn"),
        "game_accounts",
        json!({}),
        Deps {
            vault,
            endpoints: testing::endpoints(&server),
        },
    ))
    .unwrap_err();
    assert_eq!(
        error,
        json!({
            "kind": "keyring",
            "message": "无法访问游戏账号密钥环, 请确认桌面密钥环已启动并解锁."
        })
    );
}

#[test]
fn remove_deletes_the_selected_account() {
    let server = HttpServer::start();
    let (_store, vault) = testing::vault();
    let mut saved = testing::record(&"a".repeat(32), "channel-1");
    vault.save(&mut saved).unwrap();
    let value = block_on(dispatch(
        &TestHost::with_settings("nte", "cn"),
        "game_account_remove",
        json!({"accountId": saved.account_id}),
        Deps {
            vault: vault.clone(),
            endpoints: testing::endpoints(&server),
        },
    ))
    .unwrap();
    assert_eq!(value, Value::Null);
    assert!(vault.records().unwrap().is_empty());
}

#[test]
fn send_code_routes_to_the_injected_services() {
    let server = HttpServer::start();
    server.route("/m/newApi/sendPhoneCaptchaWithOutLogin", r#"{"code":0}"#);
    let dir = testing::config_dir("auth-command-code");
    let host = TestHost::new(Some(dir.path().to_path_buf()));
    let (_store, vault) = testing::vault();
    let value = block_on(dispatch(
        &host,
        "game_send_code",
        json!({"phone": format!(" {PHONE} ")}),
        Deps {
            vault,
            endpoints: testing::endpoints(&server),
        },
    ))
    .unwrap();
    assert_eq!(value.as_str().unwrap().len(), 32);
    assert_eq!(server.hits("/m/newApi/sendPhoneCaptchaWithOutLogin"), 1);

    let error = block_on(dispatch(
        &host,
        "game_send_code",
        json!({"phone": "123"}),
        Deps {
            vault: testing::vault().1,
            endpoints: testing::endpoints(&server),
        },
    ))
    .unwrap_err();
    assert_eq!(error["kind"], json!("invalid_input"));
    assert_eq!(error["message"], json!("请输入11位中国内地手机号码."));
}

#[test]
fn login_routes_and_returns_the_account_summary() {
    let server = HttpServer::start();
    server.route(
        "/openApi/secureLogin",
        r#"{"code":0,"result":{"userId":"user-1","token":"channel-token"}}"#,
    );
    server.route(
        "/sdkapi/v2/user/login",
        testing::sealed(
            testing::APP_KEY,
            r#"{"code":0,"uid":"master-uid","token":"master-token","extraOne":{"userIp":"192.0.2.1"}}"#,
        ),
    );
    let dir = testing::config_dir("auth-command-login");
    let host = TestHost::new(Some(dir.path().to_path_buf()));
    let (_store, vault) = testing::vault();
    let value = block_on(dispatch(
        &host,
        "game_login",
        json!({"phone": PHONE, "secret": "password-value", "password": true, "deviceId": ""}),
        Deps {
            vault: vault.clone(),
            endpoints: testing::endpoints(&server),
        },
    ))
    .unwrap();
    assert_eq!(
        value,
        json!({"accountId": vault.records().unwrap()[0].account_id.clone(), "maskedPhone": "138****8000"})
    );
    assert_eq!(server.hits("/openApi/secureLogin"), 1);
    assert_eq!(server.hits("/sdkapi/v2/user/login"), 1);
}
