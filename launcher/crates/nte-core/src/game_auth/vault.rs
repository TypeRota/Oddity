use super::*;
use secret_service::{
    blocking::{Collection, SecretService},
    EncryptionType,
};
use std::collections::HashMap;

const SERVICE: &str = "io.github.oddity.Launcher.game-auth";
pub struct Vault;
fn keyring(_: impl std::fmt::Display) -> Failure {
    failure(
        "keyring",
        "无法访问游戏账号密钥环, 请确认桌面密钥环已启动并解锁.",
    )
}
impl Vault {
    fn with_collection<T>(action: impl FnOnce(&Collection<'_>) -> Result<T>) -> Result<T> {
        let service = SecretService::connect(EncryptionType::Dh).map_err(keyring)?;
        let collection = service.get_default_collection().map_err(keyring)?;
        collection.ensure_unlocked().map_err(keyring)?;
        action(&collection)
    }
    fn attributes() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("service", SERVICE),
            ("game", "nte"),
            ("distribution", "cn"),
        ])
    }
    pub fn records() -> Result<Vec<Record>> {
        Self::with_collection(|collection| {
            collection
                .search_items(Self::attributes())
                .map_err(keyring)?
                .into_iter()
                .map(|item| {
                    item.ensure_unlocked().map_err(keyring)?;
                    let secret = Zeroizing::new(item.get_secret().map_err(keyring)?);
                    let record: Record = serde_json::from_slice(&secret).map_err(|_| protocol())?;
                    if record.version != 1
                        || record.account_id.len() != 32
                        || !record.account_id.bytes().all(|b| b.is_ascii_hexdigit())
                        || record.channel_uid.is_empty()
                        || record.channel_token.is_empty()
                        || record.device.device_id.is_empty()
                    {
                        return Err(protocol());
                    }
                    Ok(record)
                })
                .collect()
        })
    }
    pub fn summaries() -> Result<Vec<AccountSummary>> {
        Ok(Self::records()?.iter().map(Record::summary).collect())
    }
    pub fn load(id: Option<&str>) -> Result<Record> {
        let mut records: Vec<_> = Self::records()?
            .into_iter()
            .filter(|r| id.is_none_or(|id| r.account_id == id))
            .collect();
        if records.len() != 1 {
            return Err(failure("login_required", "请先选择或登录游戏账号."));
        }
        Ok(records.remove(0))
    }
    pub fn save(record: &mut Record) -> Result<()> {
        if let Some(existing) = Self::records()?
            .into_iter()
            .find(|r| r.channel_uid == record.channel_uid)
        {
            record.account_id = existing.account_id.clone();
        }
        let secret = Zeroizing::new(serde_json::to_vec(record).map_err(|_| protocol())?);
        Self::with_collection(|collection| {
            let mut attributes = Self::attributes();
            attributes.insert("account_id", &record.account_id);
            collection
                .create_item(
                    "Oddity 游戏账号",
                    attributes,
                    &secret,
                    true,
                    "application/json",
                )
                .map_err(keyring)?;
            Ok(())
        })
    }
    pub fn delete(id: &str) -> Result<()> {
        Self::with_collection(|collection| {
            let mut attributes = Self::attributes();
            attributes.insert("account_id", id);
            for item in collection.search_items(attributes).map_err(keyring)? {
                item.ensure_unlocked().map_err(keyring)?;
                item.delete().map_err(keyring)?;
            }
            Ok(())
        })
    }
}
