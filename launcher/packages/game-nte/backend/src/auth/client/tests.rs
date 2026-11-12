use super::*;
use crate::auth::testing::{self, sealed, APP_KEY, PHONE};
use crate::testing::{HttpServer, TempDir};
use serde_json::json;
use std::net::TcpListener;

fn client(server: &HttpServer, tag: &str) -> (TempDir, Vault, Client) {
    let dir = testing::config_dir(tag);
    let (_store, vault) = testing::vault();
    let client = Client::open(dir.path(), vault.clone(), testing::endpoints(server))
        .expect("合成配置应加载");
    (dir, vault, client)
}

/// 关闭的本地端口, 用来触发连接失败且不依赖外网.
fn closed_base() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("绑定临时端口");
    let addr = listener.local_addr().expect("读取临时端口");
    drop(listener);
    format!("http://{addr}")
}

#[test]
fn production_client_reads_the_config_and_official_hosts() {
    let dir = testing::config_dir("auth-client-load");
    let client = Client::load(dir.path()).expect("合成配置应加载");
    assert_eq!(client.endpoints.user, "https://user.laohu.com");
    assert_eq!(client.endpoints.api, "https://apidev.laohu.com");
    assert_eq!(client.endpoints.gm, "https://gmapi.laohu.com");
    assert_eq!(
        client.endpoints.antifatigue,
        "https://antifatigue.laohu.com"
    );
    assert_eq!(
        Client::load(&dir.join("missing")).err().unwrap().kind,
        "protocol"
    );
}

#[test]
fn send_code_validates_the_number_before_touching_the_network() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-code-input");
    for phone in ["", "1380013800", "23800138000", "1380013800a"] {
        let error = client.send_code(phone).err().unwrap();
        assert_eq!(error.kind, "invalid_input");
        assert_eq!(error.message, "请输入11位中国内地手机号码.");
    }
    assert_eq!(server.targets().len(), 0);
}

#[test]
fn send_code_reports_keyring_failure_before_asking_for_a_code() {
    let server = HttpServer::start();
    let dir = testing::config_dir("auth-code-keyring");
    let (store, vault) = testing::vault();
    store.set_failing(true);
    let client =
        Client::open(dir.path(), vault, testing::endpoints(&server)).expect("合成配置应加载");
    let error = client.send_code(PHONE).err().unwrap();
    assert_eq!(error.kind, "keyring");
    assert_eq!(server.targets().len(), 0);
}

#[test]
fn send_code_returns_a_device_id() {
    let server = HttpServer::start();
    server.route("/m/newApi/sendPhoneCaptchaWithOutLogin", r#"{"code":0}"#);
    let (_dir, _vault, client) = client(&server, "auth-code-ok");
    let id = client.send_code(PHONE).unwrap();
    assert_eq!(id.len(), 32);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
    assert_eq!(server.hits("/m/newApi/sendPhoneCaptchaWithOutLogin"), 1);
}

#[test]
fn channel_errors_map_sms_required_business_and_protocol_failures() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-code-errors");
    let path = "/m/newApi/sendPhoneCaptchaWithOutLogin";
    for (body, kind, message) in [
        (
            r#"{"code":12001,"message":"为了保障您的账号安全，请使用短信动态码登陆"}"#,
            "sms_required",
            "为保障账号安全, 服务端要求使用短信验证码登录.",
        ),
        (
            r#"{"code":12001,"message":"其他"}"#,
            "authentication",
            "游戏账号认证未成功 (业务码: 12001). 请检查输入或改用短信登录.",
        ),
    ] {
        server.route(path, body);
        let error = client.send_code(PHONE).err().unwrap();
        assert_eq!(error.kind, kind, "{body}");
        assert_eq!(error.message, message, "{body}");
    }
    server.route(path, "not-json");
    assert_eq!(client.send_code(PHONE).err().unwrap().kind, "protocol");
}

#[test]
fn channel_network_failures_are_reported() {
    let dir = testing::config_dir("auth-code-network");
    let (_store, vault) = testing::vault();
    let client =
        Client::open(dir.path(), vault, testing::endpoints_at(&closed_base())).expect("配置应加载");
    let error = client.send_code(PHONE).err().unwrap();
    assert_eq!(error.kind, "network");
    assert_eq!(error.message, "游戏账号请求失败, 请检查网络后重试.");
}

#[test]
fn login_validates_inputs_before_the_network() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-login-input");
    for (phone, secret, password) in [
        ("", "123456", false),
        ("23800138000", "123456", false),
        (PHONE, "", false),
        (PHONE, "123", false),
        (PHONE, "123456789", false),
        (PHONE, "abcdef", false),
    ] {
        let error = client
            .login(phone, secret, password, String::new())
            .err()
            .unwrap();
        assert_eq!(error.kind, "invalid_input", "{phone} {secret}");
        assert_eq!(error.message, "请检查手机号及密码或短信验证码.");
    }
    assert_eq!(server.targets().len(), 0);
}

#[test]
fn login_rejects_a_malformed_device_id_before_authenticating() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-login-device");
    let error = client
        .login(PHONE, "123456", false, "bad".into())
        .err()
        .unwrap();
    assert_eq!(error.kind, "invalid_input");
    assert_eq!(error.message, "设备编号无效, 请重新获取验证码.");
    assert_eq!(server.targets().len(), 0);
}

fn exchange_body() -> Vec<u8> {
    sealed(
        APP_KEY,
        r#"{"code":0,"uid":"synthetic-uid","token":"master-token","extraOne":{"userIp":"192.0.2.1","version":1}}"#,
    )
}

#[test]
fn password_login_exchanges_the_master_session_and_saves_the_account() {
    let server = HttpServer::start();
    server.route(
        "/openApi/secureLogin",
        r#"{"code":0,"result":{"userId":9007199254740993,"token":"channel-token"}}"#,
    );
    server.route("/sdkapi/v2/user/login", exchange_body());
    let (_dir, vault, client) = client(&server, "auth-login-password");

    let summary = client
        .login(PHONE, "password-value", true, String::new())
        .unwrap();
    assert_eq!(summary.masked_phone, "138****8000");
    assert_eq!(server.hits("/openApi/secureLogin"), 1);
    assert_eq!(server.hits("/sdkapi/v2/user/login"), 1);

    let records = vault.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].channel_uid, "9007199254740993");
    assert_eq!(records[0].channel_token, "channel-token");
    assert_eq!(records[0].device.device_id.len(), 32);
    assert_eq!(records[0].phone, PHONE);
}

#[test]
fn sms_login_uses_the_captcha_route_and_keeps_the_provided_device() {
    let server = HttpServer::start();
    server.route(
        "/openApi/sms/new/login",
        r#"{"code":0,"result":{"userId":"user-1","token":"channel-token"}}"#,
    );
    server.route("/sdkapi/v2/user/login", exchange_body());
    let (_dir, vault, client) = client(&server, "auth-login-sms");

    let device = "c".repeat(32);
    client
        .login(PHONE, "123456", false, device.clone())
        .unwrap();
    assert_eq!(server.hits("/openApi/sms/new/login"), 1);
    assert_eq!(server.hits("/openApi/secureLogin"), 0);
    let records = vault.records().unwrap();
    assert_eq!(records[0].channel_uid, "user-1");
    assert_eq!(records[0].device.device_id, device);
}

#[test]
fn relogin_keeps_the_saved_account_identity_for_the_same_channel_user() {
    let server = HttpServer::start();
    server.route(
        "/openApi/secureLogin",
        r#"{"code":0,"result":{"userId":"user-1","token":"channel-token"}}"#,
    );
    server.route("/sdkapi/v2/user/login", exchange_body());
    let (_dir, vault, client) = client(&server, "auth-login-repeat");

    let mut existing = testing::record(&"a".repeat(32), "user-1");
    vault.save(&mut existing).unwrap();
    client
        .login(PHONE, "password-value", true, String::new())
        .unwrap();

    let records = vault.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].account_id, existing.account_id);
}

#[test]
fn login_reports_channel_and_exchange_failures() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-login-errors");
    server.route("/openApi/secureLogin", r#"{"code":1,"message":"拒绝"}"#);
    let error = client
        .login(PHONE, "password-value", true, String::new())
        .err()
        .unwrap();
    assert_eq!(error.kind, "authentication");
    assert_eq!(server.hits("/sdkapi/v2/user/login"), 0);

    server.route(
        "/openApi/secureLogin",
        r#"{"code":0,"result":{"userId":"user-1","token":"channel-token"}}"#,
    );
    server.route("/sdkapi/v2/user/login", b"");
    let error = client
        .login(PHONE, "password-value", true, String::new())
        .err()
        .unwrap();
    assert_eq!(error.kind, "protocol");
}

#[test]
fn exchange_reports_expired_sessions_and_protocol_failures() {
    let server = HttpServer::start();
    let dir = testing::config_dir("auth-exchange");
    let (_store, vault) = testing::vault();
    let client =
        Client::open(dir.path(), vault, testing::endpoints(&server)).expect("合成配置应加载");
    let record = testing::record(&"a".repeat(32), "channel-1");
    let cases = [
        (
            b"{\"code\":1}".to_vec(),
            "login_required",
            "游戏登录状态不可用, 请重新登录.",
        ),
        (
            sealed(APP_KEY, r#"{"code":1}"#),
            "login_required",
            "游戏登录状态已失效, 请重新登录.",
        ),
        (
            b"invalid".to_vec(),
            "protocol",
            "游戏账号协议或配置与当前支持版本不符.",
        ),
        (
            sealed(APP_KEY, r#"{"code":0,"token":"t","extraOne":{}}"#),
            "protocol",
            "游戏账号协议或配置与当前支持版本不符.",
        ),
        (
            sealed(APP_KEY, r#"{"code":0,"uid":"u","extraOne":{}}"#),
            "protocol",
            "游戏账号协议或配置与当前支持版本不符.",
        ),
        (
            sealed(APP_KEY, r#"{"code":0,"uid":"u","token":"","extraOne":{}}"#),
            "protocol",
            "游戏账号协议或配置与当前支持版本不符.",
        ),
        (
            sealed(APP_KEY, r#"{"code":0,"uid":"u","token":"t","extraOne":[]}"#),
            "protocol",
            "游戏账号协议或配置与当前支持版本不符.",
        ),
        (
            sealed(APP_KEY, r#"{"code":0,"uid":"","token":"t","extraOne":{}}"#),
            "protocol",
            "游戏账号协议或配置与当前支持版本不符.",
        ),
    ];
    for (body, kind, message) in cases {
        server.route("/sdkapi/v2/user/login", body.as_slice());
        let error = client.exchange(record.clone()).err().unwrap();
        assert_eq!(error.kind, kind, "{body:?}");
        assert_eq!(error.message, message, "{body:?}");
    }
}

#[test]
fn exchange_network_failures_are_reported() {
    let dir = testing::config_dir("auth-exchange-network");
    let (_store, vault) = testing::vault();
    let client =
        Client::open(dir.path(), vault, testing::endpoints_at(&closed_base())).expect("配置应加载");
    let error = client
        .exchange(testing::record(&"a".repeat(32), "channel-1"))
        .err()
        .unwrap();
    assert_eq!(error.kind, "network");
    assert_eq!(error.message, "交换游戏登录状态失败, 请检查网络后重试.");
}

#[test]
fn sdk_query_reads_roles_and_reports_business_codes() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-query");
    let session = testing::session();

    server.route(
        "/c/api/get/roles",
        r#"{"code":0,"message":"ok","result":[{"roleId":"r1"}]}"#,
    );
    let (code, message, body) = client.sdk_query(&session, 0, json!({})).unwrap();
    assert_eq!(code, 0);
    assert_eq!(message, "ok");
    assert_eq!(body, br#"[{"roleId":"r1"}]"#.as_slice());

    let (code, message, body) = client
        .sdk_query(
            &session,
            0,
            json!({"userId": "other", "token": "other-token", "serverId": "7"}),
        )
        .unwrap();
    assert_eq!((code, message.as_str(), body.len()), (0, "ok", 17));

    server.route(
        "/c/api/get/roles",
        r#"{"code":12001,"message":"restricted"}"#,
    );
    let (code, message, body) = client.sdk_query(&session, 0, json!({})).unwrap();
    assert_eq!(
        (code, message.as_str(), body.len()),
        (12001, "restricted", 0)
    );

    server.route("/c/api/get/roles", r#"{"code":0,"result":{}}"#);
    assert_eq!(
        client.sdk_query(&session, 0, json!({})).err().unwrap().kind,
        "protocol"
    );

    server.route("/c/api/get/roles", r#"{"code":4294967296}"#);
    assert_eq!(
        client.sdk_query(&session, 0, json!({})).err().unwrap().kind,
        "protocol"
    );

    assert_eq!(
        client
            .sdk_query(&session, 0, json!({"userId": 5}))
            .err()
            .unwrap()
            .kind,
        "protocol"
    );
}

#[test]
fn sdk_query_posts_restriction_checks_and_echoes_the_response() {
    let server = HttpServer::start();
    let (_dir, _vault, client) = client(&server, "auth-query-antifatigue");
    let session = testing::session();
    let body = br#"{"code":7,"message":"limited"}"#.as_slice();
    server.route("/sdkapi/preLogin", body);
    let (code, message, raw) = client.sdk_query(&session, 1, json!({})).unwrap();
    assert_eq!(code, 0);
    assert_eq!(message, "limited");
    assert_eq!(raw, body);
    assert_eq!(server.hits("/sdkapi/preLogin"), 1);

    let (code, message, raw) = client.sdk_query(&session, 2, json!({})).unwrap();
    assert_eq!(
        (code, message.as_str(), raw.len()),
        (-1, "Unsupported SDK query", 0)
    );

    server.route("/sdkapi/preLogin", "not-json");
    assert_eq!(
        client.sdk_query(&session, 1, json!({})).err().unwrap().kind,
        "protocol"
    );
}

#[test]
fn sdk_query_network_failures_are_reported() {
    let dir = testing::config_dir("auth-query-network");
    let (_store, vault) = testing::vault();
    let client =
        Client::open(dir.path(), vault, testing::endpoints_at(&closed_base())).expect("配置应加载");
    let error = client
        .sdk_query(&testing::session(), 0, json!({}))
        .err()
        .unwrap();
    assert_eq!(error.kind, "network");
    assert_eq!(error.message, "游戏登录前查询失败.");
}
