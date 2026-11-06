//! 国服游戏账号, 与塔吉多身份和官方启动器缓存分别管理.
use base64::{engine::general_purpose::STANDARD, Engine};
use cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, time::SystemTime};
use zeroize::{Zeroize, Zeroizing};

mod client;
mod config;
mod vault;
mod wire;
pub use client::Client;
pub use vault::Vault;
pub use wire::query_reply;

pub type Result<T> = std::result::Result<T, Failure>;
#[derive(Debug, Serialize)]
pub struct Failure {
    pub kind: &'static str,
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
        message: message.into(),
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn query_reply_preserves_real_restrictions_and_signed_error_code() {
        let body = br#"{"code":12001,"message":"restricted"}"#;
        let reply = query_reply(1, 7, -1, "校验失败", body).unwrap();
        assert_eq!(
            u32::from_le_bytes(reply[24..28].try_into().unwrap()),
            u32::MAX
        );
        assert_eq!(&reply[4 + 0x41c..4 + 0x41c + body.len()], body);
        assert!(query_reply(1, 7, 0, "", b"bad\0json").is_err());
    }
}
