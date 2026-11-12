//! 国服游戏账号保存在桌面密钥环, 每个账号一个条目.
//!
//! 读写经 oddity-platform 的密钥环能力缝: 生产用 Secret Service, 测试注入
//! 内存实现. 条目属性保留 game 与 distribution, 旧数据可直接读出.

use super::{failure, protocol, AccountSummary, Record, Result};
use oddity_platform::keyring::{SecretEntry, SecretQuery, SecretServiceStore, SecretStore};
use std::sync::Arc;
use zeroize::Zeroizing;

const SERVICE: &str = "io.github.oddity.Launcher.game-auth";
const LABEL: &str = "Oddity 游戏账号";

/// 账号仓库; 克隆共享同一个密钥环.
#[derive(Clone)]
pub struct Vault {
    store: Arc<dyn SecretStore>,
}

impl Default for Vault {
    fn default() -> Self {
        Self::with_store(Arc::new(SecretServiceStore::new()))
    }
}

impl Vault {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn with_store(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    fn query() -> SecretQuery {
        SecretQuery::new(SERVICE)
            .with("game", "nte")
            .with("distribution", "cn")
    }

    pub fn records(&self) -> Result<Vec<Record>> {
        self.store
            .search(&Self::query())
            .map_err(keyring)?
            .into_iter()
            .map(decode)
            .collect()
    }

    pub fn summaries(&self) -> Result<Vec<AccountSummary>> {
        Ok(self.records()?.iter().map(Record::summary).collect())
    }

    pub fn load(&self, id: Option<&str>) -> Result<Record> {
        let mut records: Vec<_> = self
            .records()?
            .into_iter()
            .filter(|r| id.is_none_or(|id| r.account_id == id))
            .collect();
        if records.len() != 1 {
            return Err(failure("login_required", "请先选择或登录游戏账号."));
        }
        Ok(records.remove(0))
    }

    pub fn save(&self, record: &mut Record) -> Result<()> {
        if let Some(existing) = self
            .records()?
            .into_iter()
            .find(|r| r.channel_uid == record.channel_uid)
        {
            record.account_id = existing.account_id.clone();
        }
        let secret = Zeroizing::new(serde_json::to_vec(record).map_err(|_| protocol())?);
        self.store
            .write(
                &Self::query().with("account_id", record.account_id.as_str()),
                LABEL,
                "application/json",
                &secret,
            )
            .map_err(keyring)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.store
            .delete(&Self::query().with("account_id", id))
            .map_err(keyring)
    }
}

fn decode(entry: SecretEntry) -> Result<Record> {
    let secret = Zeroizing::new(entry.secret);
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
}

fn keyring(_: oddity_platform::Error) -> super::Failure {
    failure(
        "keyring",
        "无法访问游戏账号密钥环, 请确认桌面密钥环已启动并解锁.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::record;
    use oddity_platform::keyring::MemorySecretStore;

    fn memory_vault() -> (Arc<MemorySecretStore>, Vault) {
        let store = Arc::new(MemorySecretStore::new());
        let vault = Vault::with_store(store.clone());
        (store, vault)
    }

    #[test]
    fn save_keeps_account_id_per_channel_and_scopes_attributes_by_account() {
        let (store, vault) = memory_vault();
        let mut first = record(&"a".repeat(32), "channel-1");
        vault.save(&mut first).unwrap();
        assert_eq!(vault.records().unwrap().len(), 1);

        let mut same_channel = record(&"c".repeat(32), "channel-1");
        same_channel.phone = "13900139000".into();
        vault.save(&mut same_channel).unwrap();
        assert_eq!(same_channel.account_id, first.account_id);

        let records = vault.records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].account_id, first.account_id);
        assert_eq!(records[0].phone, "13900139000");

        let entries = store
            .search(&Vault::query().with("account_id", first.account_id.as_str()))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].attributes["game"], "nte");
        assert_eq!(entries[0].attributes["distribution"], "cn");
        assert!(!entries[0].attributes.contains_key("service"));

        vault.delete(&first.account_id).unwrap();
        assert!(vault.records().unwrap().is_empty());
    }

    #[test]
    fn load_requires_one_record_and_summaries_mask_the_phone() {
        let (_store, vault) = memory_vault();
        assert_eq!(vault.load(None).err().unwrap().kind, "login_required");

        let mut first = record(&"a".repeat(32), "channel-1");
        vault.save(&mut first).unwrap();
        let mut second = record(&"b".repeat(32), "channel-2");
        second.phone = "13900139000".into();
        vault.save(&mut second).unwrap();

        assert_eq!(
            vault.load(Some(first.account_id.as_str())).unwrap().phone,
            "13800138000"
        );
        assert_eq!(vault.load(None).err().unwrap().kind, "login_required");
        assert_eq!(
            vault.load(Some("missing")).err().unwrap().kind,
            "login_required"
        );

        let summaries = vault.summaries().unwrap();
        assert_eq!(summaries.len(), 2);
        assert!(summaries
            .iter()
            .any(|summary| summary.masked_phone == "138****8000"));
        assert!(summaries
            .iter()
            .any(|summary| summary.masked_phone == "139****9000"));
    }

    #[test]
    fn malformed_or_unversioned_records_are_rejected_without_leaking_content() {
        let (store, vault) = memory_vault();
        store
            .write(
                &Vault::query(),
                LABEL,
                "application/json",
                b"private-secret",
            )
            .unwrap();
        let error = vault.records().err().unwrap();
        assert_eq!(error.kind, "protocol");
        assert!(!error.message.contains("private-secret"));

        let valid = serde_json::to_value(record(&"a".repeat(32), "channel-1")).unwrap();
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("account_id", serde_json::json!("short")),
            ("channel_uid", serde_json::json!("")),
            ("channel_token", serde_json::json!("")),
        ] {
            let (store, vault) = memory_vault();
            let mut changed = valid.clone();
            changed[key] = value;
            store
                .write(
                    &Vault::query(),
                    LABEL,
                    "application/json",
                    &serde_json::to_vec(&changed).unwrap(),
                )
                .unwrap();
            assert_eq!(vault.records().err().unwrap().kind, "protocol");
        }

        let mut missing_device = valid;
        missing_device["device"]["deviceId"] = serde_json::json!("");
        let (store, vault) = memory_vault();
        store
            .write(
                &Vault::query(),
                LABEL,
                "application/json",
                &serde_json::to_vec(&missing_device).unwrap(),
            )
            .unwrap();
        assert_eq!(vault.records().err().unwrap().kind, "protocol");
    }

    #[test]
    fn store_failure_is_reported_as_a_keyring_failure() {
        let (store, vault) = memory_vault();
        store.set_failing(true);
        let mut sample = record(&"a".repeat(32), "channel-1");
        for error in [
            vault.records().err().unwrap(),
            vault.summaries().err().unwrap(),
            vault.load(None).err().unwrap(),
            vault.save(&mut sample).err().unwrap(),
            vault.delete("missing").err().unwrap(),
        ] {
            assert_eq!(error.kind, "keyring");
            assert_eq!(
                error.message,
                "无法访问游戏账号密钥环, 请确认桌面密钥环已启动并解锁."
            );
        }
    }

    #[test]
    fn records_only_read_entries_under_the_game_service() {
        let (store, vault) = memory_vault();
        store
            .write(
                &SecretQuery::new("io.github.oddity.Launcher.other").with("game", "nte"),
                LABEL,
                "application/json",
                &serde_json::to_vec(&record(&"a".repeat(32), "channel-1")).unwrap(),
            )
            .unwrap();
        assert!(vault.records().unwrap().is_empty());
    }
}
