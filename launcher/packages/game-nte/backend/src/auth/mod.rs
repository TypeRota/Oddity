//! 国服游戏账号: 登录, 设备与会话, 短信验证码.
//!
//! 命令组声明 `game_accounts` / `game_send_code` / `game_login` /
//! `game_account_remove`; 参数与返回沿用宿主载荷, 凭据只写入平台密钥环,
//! 网络经平台共享连接池. 国服能力门同时供宿主直接启动使用.

mod client;
mod config;
mod vault;
mod wire;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use base64::{engine::general_purpose::STANDARD, Engine};
use cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use oddity_runtime::{Availability, CapabilityContext, CapabilityFailure, HostContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read, path::PathBuf, time::SystemTime};
use zeroize::{Zeroize, Zeroizing};

pub use client::Client;
pub use vault::Vault;
pub use wire::query_reply;

/// 账号域注册到宿主命令组里的命令 id。
pub(crate) const COMMANDS: &[&str] = &[
    "game_accounts",
    "game_send_code",
    "game_login",
    "game_account_remove",
];

pub type Result<T> = std::result::Result<T, Failure>;
#[derive(Debug, Serialize)]
pub struct Failure {
    pub kind: &'static str,
    /// 能力 gate 的稳定错误码; 非能力错误不输出.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// 能力 `Unavailable` 的 provider 原因码.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub message: String,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
pub(super) fn failure(kind: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        kind,
        code: None,
        reason: None,
        message: message.into(),
    }
}

/// 能力 gate 失败映射为账号域错误载荷.
fn capability_failure(error: CapabilityFailure) -> Failure {
    Failure {
        kind: "unsupported",
        code: Some(error.code),
        reason: error.reason,
        message: error.message.into(),
    }
}
pub(super) fn protocol() -> Failure {
    failure("protocol", "游戏账号协议或配置与当前支持版本不符.")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub device_id: String,
    pub device_type: String,
    pub device_model: String,
    pub device_sys: String,
    pub app_version: String,
    pub sdk_version: String,
    pub package_name: String,
}
impl Device {
    pub fn new(id: String) -> Result<Self> {
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(failure("invalid_input", "设备编号无效, 请重新获取验证码."));
        }
        Ok(Self {
            device_id: id,
            device_type: "Linux x86_64".into(),
            device_model: "x86_64".into(),
            device_sys: "Linux".into(),
            app_version: "1.0.0".into(),
            sdk_version: "5.159.0".into(),
            package_name: "10518".into(),
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    pub account_id: String,
    pub phone: String,
    pub channel_uid: String,
    pub channel_token: String,
    pub device: Device,
}
impl Drop for Record {
    fn drop(&mut self) {
        self.channel_token.zeroize();
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub account_id: String,
    pub masked_phone: String,
}
impl Record {
    pub fn summary(&self) -> AccountSummary {
        AccountSummary {
            account_id: self.account_id.clone(),
            masked_phone: if valid_phone(&self.phone) {
                format!("{}****{}", &self.phone[..3], &self.phone[7..])
            } else {
                "游戏账号".into()
            },
        }
    }
}
pub struct Session {
    pub uid: String,
    pub token: Zeroizing<String>,
    pub extra_one: serde_json::Value,
    pub record: Record,
}

pub fn nonce() -> Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| failure("storage", "无法生成游戏登录设备编号."))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn valid_phone(phone: &str) -> bool {
    phone.len() == 11 && phone.starts_with('1') && phone.bytes().all(|b| b.is_ascii_digit())
}

pub(super) fn encrypt(raw: &[u8], key: &[u8]) -> Result<String> {
    let cipher = aes::Aes128::new_from_slice(
        key.get(key.len().saturating_sub(16)..)
            .ok_or_else(protocol)?,
    )
    .map_err(|_| protocol())?;
    let mut bytes = Zeroizing::new(raw.to_vec());
    let padding = 16 - bytes.len() % 16;
    let size = bytes.len() + padding;
    bytes.resize(size, padding as u8);
    for chunk in bytes.as_chunks_mut::<16>().0 {
        let mut block = cipher::Block::<aes::Aes128>::default();
        block.copy_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    Ok(STANDARD.encode(bytes.as_slice()))
}
pub(super) fn decrypt(raw: &[u8], key: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(STANDARD.decode(raw).map_err(|_| protocol())?);
    if bytes.is_empty() || bytes.len() % 16 != 0 {
        return Err(protocol());
    }
    let cipher = aes::Aes128::new_from_slice(
        key.get(key.len().saturating_sub(16)..)
            .ok_or_else(protocol)?,
    )
    .map_err(|_| protocol())?;
    for chunk in bytes.as_chunks_mut::<16>().0 {
        let mut block = cipher::Block::<aes::Aes128>::default();
        block.copy_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    let padding = usize::from(*bytes.last().ok_or_else(protocol)?);
    if !(1..=16).contains(&padding)
        || bytes[bytes.len() - padding..]
            .iter()
            .any(|&v| usize::from(v) != padding)
    {
        return Err(protocol());
    }
    let size = bytes.len() - padding;
    bytes.truncate(size);
    Ok(bytes)
}
type Parameters = BTreeMap<String, String>;

/// 命令依赖: 生产用桌面密钥环与官方主机, 测试注入内存密钥环与回环地址.
struct Deps {
    vault: Vault,
    endpoints: client::Endpoints,
}
impl Deps {
    fn production() -> Self {
        Self {
            vault: Vault::new(),
            endpoints: client::Endpoints::default(),
        }
    }
}

/// 分发账号域命令; 参数与结果都是宿主载荷。
pub(crate) async fn command(
    host: &dyn HostContext,
    command: &str,
    args: Value,
) -> std::result::Result<Value, Value> {
    dispatch(host, command, args, Deps::production()).await
}

/// 按命令解析参数并交给业务层; 测试注入内存密钥环与回环服务.
async fn dispatch(
    host: &dyn HostContext,
    command: &str,
    args: Value,
    deps: Deps,
) -> std::result::Result<Value, Value> {
    // 能力 gate 在参数解析与凭据访问之前.
    require_account(host).map_err(payload)?;
    match command {
        "game_accounts" => {
            let accounts = accounts(deps.vault).await.map_err(payload)?;
            serde_json::to_value(accounts).map_err(|_| json!("游戏账号状态格式无效"))
        }
        "game_send_code" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                phone: String,
            }
            let Args { phone } =
                serde_json::from_value(args).map_err(|_| json!("游戏账号请求参数无效"))?;
            let code = send_code(host, deps.vault, deps.endpoints, phone)
                .await
                .map_err(payload)?;
            Ok(json!(code))
        }
        "game_login" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
                secret: String,
                password: bool,
                device_id: String,
            }
            let args: Args =
                serde_json::from_value(args).map_err(|_| json!("游戏账号请求参数无效"))?;
            let account = login(
                host,
                deps.vault,
                deps.endpoints,
                args.phone,
                args.secret,
                args.password,
                args.device_id,
            )
            .await
            .map_err(payload)?;
            Ok(json!(account))
        }
        "game_account_remove" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                account_id: String,
            }
            let Args { account_id } =
                serde_json::from_value(args).map_err(|_| json!("游戏账号请求参数无效"))?;
            remove(deps.vault, account_id).await.map_err(payload)?;
            Ok(Value::Null)
        }
        _ => Err(json!("无法识别的游戏账号命令")),
    }
}

async fn accounts(vault: Vault) -> Result<Vec<AccountSummary>> {
    blocking(move || vault.summaries(), "读取游戏账号任务异常.").await
}
async fn send_code(
    host: &dyn HostContext,
    vault: Vault,
    endpoints: client::Endpoints,
    phone: String,
) -> Result<String> {
    let root = launcher_root(host)?;
    blocking(
        move || Client::open(&root, vault, endpoints)?.send_code(phone.trim()),
        "获取游戏验证码任务异常.",
    )
    .await
}
async fn login(
    host: &dyn HostContext,
    vault: Vault,
    endpoints: client::Endpoints,
    phone: String,
    secret: String,
    password: bool,
    device_id: String,
) -> Result<AccountSummary> {
    let secret = Zeroizing::new(secret);
    let root = launcher_root(host)?;
    blocking(
        move || {
            Client::open(&root, vault, endpoints)?.login(phone.trim(), &secret, password, device_id)
        },
        "游戏账号认证任务异常.",
    )
    .await
}
async fn remove(vault: Vault, id: String) -> Result<()> {
    blocking(move || vault.delete(&id), "删除游戏账号任务异常.").await
}

/// 官方客户端账号能力的可用性: 只对《异环》国服提供.
pub(crate) fn availability(context: &CapabilityContext) -> Availability {
    if context.game == "nte" && context.distribution == "cn" {
        Availability::Available
    } else {
        Availability::Unsupported
    }
}

/// 能力门文案; 直接启动与账号命令共用.
const UNSUPPORTED: &str = "直接启动和游戏账号登录目前仅支持《异环》国服, 请使用后台或前台启动.";

/// 直接启动的能力门: 只做静态区服判断, 不接触设置或文件系统.
pub fn require_supported(game: &str, distribution: &str) -> std::result::Result<(), String> {
    let context = CapabilityContext {
        game: game.to_owned(),
        distribution: distribution.to_owned(),
        account_id: None,
        session_running: false,
        launcher_present: true,
    };
    availability(&context)
        .into_gate(UNSUPPORTED)
        .map_err(|error| error.message.to_string())
}

/// 账号命令的能力门: 先读设置与活动状态, 再接触密钥环或网络.
fn require_account(host: &dyn HostContext) -> Result<()> {
    let context = host
        .capability_context()
        .map_err(|message| failure("service", message))?;
    availability(&context)
        .into_gate(UNSUPPORTED)
        .map_err(capability_failure)
}

/// 认证配置与 SDK 组件都在官方启动器目录里, 未安装时先给出提示。
///
/// 安装缺失是命令级前置, 不是能力级 Unavailable: 未安装时账号列表仍可读.
fn launcher_root(host: &dyn HostContext) -> Result<PathBuf> {
    host.official_launcher_root().ok_or_else(|| Failure {
        kind: "unsupported",
        code: Some("capability_unavailable"),
        reason: Some("launcher_missing"),
        message: "请先安装官方组件和游戏资源.".into(),
    })
}

/// 账号命令的阻塞工作交给执行器, 与宿主原有的后台线程一致。
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
    message: &'static str,
) -> Result<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| failure("service", message))?
}

/// 业务错误序列化为宿主的错误载荷.
fn payload(error: Failure) -> Value {
    serde_json::to_value(error).unwrap_or(Value::Null)
}
