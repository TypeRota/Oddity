//! 国服游戏账号协议: 短信验证码, 密码/短信登录, 主账号交换与 SDK 查询.
//!
//! 请求主机可替换, 生产为官方地址; 测试注入回环服务, 网络能力统一来自
//! oddity-platform 的共享连接池, 状态码与重定向由协议自行判定.

use super::{
    config, decrypt, encrypt, failure, nonce, now, protocol, valid_phone, AccountSummary, Device,
    Parameters, Record, Result, Session, Vault,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use md5::{Digest, Md5};
use oddity_platform::http::HttpClient;
use oddity_platform::USER_AGENT;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::path::Path;
use std::time::Duration;
use zeroize::Zeroizing;

/// 单次协议请求时长; 与共享连接池的下载超时分开.
const TIMEOUT: Duration = Duration::from_secs(20);

/// 官方接口主机; 测试替换为回环服务.
#[derive(Clone)]
pub(super) struct Endpoints {
    pub(super) user: String,
    pub(super) api: String,
    pub(super) gm: String,
    pub(super) antifatigue: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            user: "https://user.laohu.com".into(),
            api: "https://apidev.laohu.com".into(),
            gm: "https://gmapi.laohu.com".into(),
            antifatigue: "https://antifatigue.laohu.com".into(),
        }
    }
}

pub struct Client {
    config: config::Config,
    vault: Vault,
    endpoints: Endpoints,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Id {
    Text(String),
    Number(u64),
}
impl Id {
    fn text(self) -> String {
        match self {
            Self::Text(s) => s,
            Self::Number(n) => n.to_string(),
        }
    }
}
#[derive(Deserialize)]
struct Reply {
    code: i64,
    #[serde(default)]
    message: String,
    #[serde(default)]
    result: Value,
}

/// 补全协议请求: 不跟随重定向, 状态码由业务码判断。
fn prepared<B>(request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    let config = request
        .config()
        .max_redirects(0)
        .http_status_as_error(false);
    // 单元测试只连接回环服务, 不继承宿主代理.
    #[cfg(test)]
    let config = config.proxy(None);
    config.build()
}

impl Client {
    pub fn load(root: &Path) -> Result<Self> {
        Self::open(root, Vault::new(), Endpoints::default())
    }
    pub(super) fn open(root: &Path, vault: Vault, endpoints: Endpoints) -> Result<Self> {
        Ok(Self {
            config: config::Config::load(root)?,
            vault,
            endpoints,
        })
    }
    fn channel(&self, route: &str, mut values: Parameters) -> Result<Value> {
        values.insert("t".into(), now().to_string());
        let mut canonical =
            Zeroizing::new(values.values().cloned().collect::<String>().into_bytes());
        canonical.extend_from_slice(&self.config.channel_key);
        values.insert(
            "sign".into(),
            Md5::digest(&canonical)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        );
        let request = prepared(HttpClient::shared().post_as(
            &format!("{}{route}", self.endpoints.user),
            TIMEOUT,
            USER_AGENT,
        ));
        let mut response = request
            .send_form(values.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .map_err(|_| failure("network", "游戏账号请求失败, 请检查网络后重试."))?;
        let raw = Zeroizing::new(
            response
                .body_mut()
                .with_config()
                .limit(1_048_576)
                .read_to_vec()
                .map_err(|_| failure("network", "无法读取游戏账号响应."))?,
        );
        let reply: Reply = serde_json::from_slice(&raw).map_err(|_| protocol())?;
        if reply.code != 0 {
            return Err(
                if reply.message == "为了保障您的账号安全，请使用短信动态码登陆"
                {
                    failure(
                        "sms_required",
                        "为保障账号安全, 服务端要求使用短信验证码登录.",
                    )
                } else {
                    failure(
                        "authentication",
                        format!(
                            "游戏账号认证未成功 (业务码: {}). 请检查输入或改用短信登录.",
                            reply.code
                        ),
                    )
                },
            );
        }
        Ok(reply.result)
    }
    pub fn send_code(&self, phone: &str) -> Result<String> {
        if !valid_phone(phone) {
            return Err(failure("invalid_input", "请输入11位中国内地手机号码."));
        }
        // 保存前检查密钥环, 避免发送验证码后才发现无法记住账号.
        self.vault.summaries()?;
        let id = nonce()?[..32].to_owned();
        self.channel(
            "/m/newApi/sendPhoneCaptchaWithOutLogin",
            Parameters::from([
                ("appId".into(), "10518".into()),
                ("channelId".into(), "9".into()),
                ("cellphone".into(), phone.into()),
                ("areaCodeId".into(), "1".into()),
                ("type".into(), "18".into()),
                ("mac".into(), format!("02{}", &id[..10]).to_uppercase()),
            ]),
        )?;
        Ok(id)
    }
    pub fn login(
        &self,
        phone: &str,
        secret: &str,
        password: bool,
        device_id: String,
    ) -> Result<AccountSummary> {
        if !valid_phone(phone)
            || secret.is_empty()
            || (!password
                && (!(4..=8).contains(&secret.len())
                    || !secret.bytes().all(|b| b.is_ascii_digit())))
        {
            return Err(failure("invalid_input", "请检查手机号及密码或短信验证码."));
        }
        self.vault.summaries()?;
        let device = Device::new(if device_id.is_empty() {
            nonce()?[..32].into()
        } else {
            device_id
        })?;
        let mut values = Parameters::from([
            ("appId".into(), "10518".into()),
            ("channelId".into(), "9".into()),
            ("bid".into(), device.package_name.clone()),
            ("deviceId".into(), device.device_id.clone()),
            ("deviceType".into(), device.device_type.clone()),
            ("deviceName".into(), "Oddity".into()),
            ("deviceModel".into(), device.device_model.clone()),
            ("deviceSys".into(), device.device_sys.clone()),
            ("version".into(), device.app_version.clone()),
            ("sdkVersion".into(), device.sdk_version.clone()),
            (
                "mac".into(),
                format!("02{}", &device.device_id[..10]).to_uppercase(),
            ),
            ("adm".into(), String::new()),
        ]);
        let route = if password {
            values.insert(
                "username".into(),
                encrypt(phone.as_bytes(), &self.config.channel_key)?,
            );
            values.insert(
                "password".into(),
                encrypt(secret.as_bytes(), &self.config.channel_key)?,
            );
            "/openApi/secureLogin"
        } else {
            values.insert(
                "cellphone".into(),
                encrypt(phone.as_bytes(), &self.config.channel_key)?,
            );
            values.insert(
                "captcha".into(),
                encrypt(secret.as_bytes(), &self.config.channel_key)?,
            );
            values.insert("areaCodeId".into(), "1".into());
            values.insert("type".into(), "18".into());
            "/openApi/sms/new/login"
        };
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct User {
            user_id: Id,
            token: String,
        }
        let user: User =
            serde_json::from_value(self.channel(route, values)?).map_err(|_| protocol())?;
        let record = Record {
            version: 1,
            account_id: nonce()?[..32].into(),
            phone: phone.into(),
            channel_uid: user.user_id.text(),
            channel_token: user.token,
            device,
        };
        let mut session = self.exchange(record)?;
        self.vault.save(&mut session.record)?;
        Ok(session.record.summary())
    }
    pub fn exchange(&self, record: Record) -> Result<Session> {
        let payload = Zeroizing::new(
            serde_json::to_vec(
                &json!({"appId":"1289","channelId":"9","platformId":"4","version":"2",
            "extraJson":"{\"channelId\":\"9\",\"subchannelId\":\"9\"}",
            "channelUid":record.channel_uid,"channelToken":record.channel_token,
            "device":serde_json::to_string(&record.device).map_err(|_| protocol())?}),
            )
            .map_err(|_| protocol())?,
        );
        let body = Zeroizing::new(encrypt(&payload, &self.config.app_key)?);
        let request = prepared(HttpClient::shared().post_as(
            &format!("{}/sdkapi/v2/user/login", self.endpoints.api),
            TIMEOUT,
            USER_AGENT,
        ));
        let mut response = request
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("appId", "1289")
            .send(body.as_bytes())
            .map_err(|_| failure("network", "交换游戏登录状态失败, 请检查网络后重试."))?;
        let raw = Zeroizing::new(
            response
                .body_mut()
                .with_config()
                .limit(1_048_576)
                .read_to_vec()
                .map_err(|_| protocol())?,
        );
        if raw.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'{') {
            return Err(failure("login_required", "游戏登录状态不可用, 请重新登录."));
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Master {
            code: i64,
            uid: Option<Id>,
            token: Option<String>,
            #[serde(default)]
            extra_one: Value,
        }
        let decoded: Master = serde_json::from_slice(&decrypt(&raw, &self.config.app_key)?)
            .map_err(|_| protocol())?;
        if decoded.code != 0 {
            return Err(failure("login_required", "游戏登录状态已失效, 请重新登录."));
        }
        let uid = decoded.uid.ok_or_else(protocol)?.text();
        let token = decoded
            .token
            .filter(|s| !s.is_empty())
            .ok_or_else(protocol)?;
        if uid.is_empty() || !decoded.extra_one.is_object() {
            return Err(protocol());
        }
        Ok(Session {
            uid,
            token: Zeroizing::new(token),
            extra_one: decoded.extra_one,
            record,
        })
    }
    pub fn sdk_query(
        &self,
        session: &Session,
        kind: u32,
        supplied: Value,
    ) -> Result<(i32, String, Vec<u8>)> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct Query {
            user_id: Option<String>,
            token: Option<String>,
            server_id: Option<String>,
        }
        let supplied: Query = serde_json::from_value(supplied).map_err(|_| protocol())?;
        let device = &session.record.device;
        let (url, get, mut values) = match kind {
            0 => {
                let mut device_value = serde_json::to_value(device).map_err(|_| protocol())?;
                device_value
                    .as_object_mut()
                    .ok_or_else(protocol)?
                    .remove("sdkVersion");
                let (uid, token) = match (supplied.user_id, supplied.token) {
                    (Some(uid), Some(token)) if !uid.is_empty() && !token.is_empty() => {
                        (uid, token)
                    }
                    _ => (session.uid.clone(), session.token.to_string()),
                };
                (
                    format!("{}/c/api/get/roles", self.endpoints.gm),
                    true,
                    Parameters::from([
                        ("appId".into(), "1289".into()),
                        ("userId".into(), uid),
                        ("token".into(), token),
                        ("serverId".into(), supplied.server_id.unwrap_or_default()),
                        ("timestamp".into(), (now() / 1000).to_string()),
                        ("os".into(), "4".into()),
                        ("device".into(), device_value.to_string()),
                    ]),
                )
            }
            1 => (
                format!("{}/sdkapi/preLogin", self.endpoints.antifatigue),
                false,
                Parameters::from([
                    ("appId".into(), "1289".into()),
                    ("channelId".into(), "9".into()),
                    ("platformId".into(), "4".into()),
                    ("userId".into(), session.uid.clone()),
                    ("udid".into(), device.device_id.clone()),
                    ("ndid".into(), device.device_id.clone()),
                    ("deviceName".into(), "WindowsPc".into()),
                    ("timestamp".into(), now().to_string()),
                ]),
            ),
            _ => return Ok((-1, "Unsupported SDK query".into(), Vec::new())),
        };
        let key = &self.config.app_key;
        let mut canonical =
            Zeroizing::new([&key[1..3], &key[7..12], &key[15..18], &key[25..31]].concat());
        canonical.extend_from_slice(
            values
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&")
                .as_bytes(),
        );
        values.insert("sign".into(), STANDARD.encode(Sha256::digest(&canonical)));
        let result = if get {
            prepared(HttpClient::shared().request_as(&url, TIMEOUT, USER_AGENT))
                .query_pairs(values.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .call()
        } else {
            prepared(HttpClient::shared().post_as(&url, TIMEOUT, USER_AGENT))
                .send_form(values.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        };
        let mut response = result.map_err(|_| failure("network", "游戏登录前查询失败."))?;
        let raw = response
            .body_mut()
            .with_config()
            .limit(65536 - 0x422)
            .read_to_vec()
            .map_err(|_| protocol())?;
        let reply: Reply = serde_json::from_slice(&raw).map_err(|_| protocol())?;
        if kind == 0 {
            let code = i32::try_from(reply.code).map_err(|_| protocol())?;
            if code == 0 && !reply.result.is_array() {
                return Err(protocol());
            }
            Ok((
                code,
                reply.message,
                if code == 0 {
                    serde_json::to_vec(&reply.result).map_err(|_| protocol())?
                } else {
                    Vec::new()
                },
            ))
        } else {
            // 防沉迷限制由真实服务端JSON传给游戏, 回调码仅表示传输结果.
            Ok((0, reply.message, raw))
        }
    }
}

#[cfg(test)]
mod tests;
