//! 塔吉多账号保存在桌面 Keyring 的单一条目里。
//!
//! 读写经 oddity-platform 的密钥环能力缝: 生产用 Secret Service, 测试注入
//! 内存实现。条目属性沿用旧版 (service + username), 旧数据可直接读出。

use std::sync::Arc;

use oddity_platform::keyring::{SecretQuery, SecretServiceStore, SecretStore};
use oddity_platform::Error;
use zeroize::Zeroizing;

use super::{account_id, storage_error, Account, Failure, FailureKind, Result};

const SERVICE: &str = "io.github.oddity.Launcher.taygedo";
const LABEL: &str = "Oddity 塔吉多账号";
const CONTENT_TYPE: &str = "text/plain; charset=utf-8";

pub trait Vault {
    fn load_all(&self) -> Result<Vec<Account>>;
    fn save(&self, account: &Account) -> Result<()>;
    fn delete(&self, account_id: &str) -> Result<()>;
}
pub struct KeyringVault {
    store: Arc<dyn SecretStore>,
    scope: String,
}
impl KeyringVault {
    pub fn new(scope: impl Into<String>) -> Self {
        Self::with_store(scope, Arc::new(SecretServiceStore::new()))
    }
    fn with_store(scope: impl Into<String>, store: Arc<dyn SecretStore>) -> Self {
        Self {
            store,
            scope: scope.into(),
        }
    }
    /// 所有账号放在同一个条目里, 由配置目录派生的 scope 定位。
    fn query(&self) -> SecretQuery {
        SecretQuery::new(SERVICE).with("username", self.scope.as_str())
    }
    fn secrets(&self) -> Result<Vec<Vec<u8>>> {
        let items = self.store.search(&self.query()).map_err(keyring_error)?;
        if items.len() > 1 {
            return Err(ambiguous());
        }
        Ok(items.into_iter().map(|entry| entry.secret).collect())
    }
    fn write_all(&self, accounts: &[Account]) -> Result<()> {
        let secret = Zeroizing::new(serde_json::to_vec(accounts).map_err(storage_error)?);
        // 重复条目先按错误上报, 不覆盖用户已有的条目。
        self.secrets()?;
        self.store
            .write(&self.query(), LABEL, CONTENT_TYPE, &secret)
            .map_err(keyring_error)
    }
}
fn keyring_error(_: Error) -> Failure {
    Failure::new(
        FailureKind::Keyring,
        "无法访问 Keyring, 请确认桌面密钥环已启动并解锁",
    )
}
fn ambiguous() -> Failure {
    Failure::new(
        FailureKind::Keyring,
        "Keyring 中存在重复的塔吉多条目, 请在密钥环管理器中检查",
    )
}
impl Vault for KeyringVault {
    fn load_all(&self) -> Result<Vec<Account>> {
        match self.secrets()?.first() {
            Some(secret) => decode_accounts(secret),
            None => Ok(vec![]),
        }
    }
    fn save(&self, account: &Account) -> Result<()> {
        let mut accounts = self.load_all()?;
        if let Some(existing) = accounts.iter_mut().find(|a| a.phone == account.phone) {
            *existing = account.clone();
        } else {
            accounts.push(account.clone());
        }
        self.write_all(&accounts)
    }
    fn delete(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.load_all()?;
        accounts.retain(|a| super::account_id(&a.phone) != account_id);
        if !accounts.is_empty() {
            return self.write_all(&accounts);
        }
        self.secrets()?;
        self.store.delete(&self.query()).map_err(keyring_error)
    }
}
fn decode_accounts(secret: &[u8]) -> Result<Vec<Account>> {
    serde_json::from_slice::<Vec<Account>>(secret)
        .or_else(|_| serde_json::from_slice::<Account>(secret).map(|a| vec![a]))
        .map_err(|_| Failure::new(FailureKind::Keyring, "Keyring 中的塔吉多账号格式无效"))
}
/// 只读写独立的临时条目, 不访问用户已有账号。
pub fn check_keyring() -> Result<()> {
    check_keyring_with(Arc::new(SecretServiceStore::new()))
}
fn check_keyring_with(store: Arc<dyn SecretStore>) -> Result<()> {
    let scope = format!(
        "check-{}-{}",
        std::process::id(),
        super::client::random_hex(8)?
    );
    let vault = KeyringVault::with_store(scope, store);
    let account = Account {
        phone: String::new(),
        password: None,
        device_id: "check".into(),
        uid: "check".into(),
        refresh_token: "Oddity Keyring check".into(),
        laohu_token: String::new(),
        laohu_user_id: String::new(),
    };
    vault.save(&account)?;
    let result = vault.load_all().and_then(|value| {
        if value.len() == 1 && value[0].refresh_token == "Oddity Keyring check" {
            Ok(())
        } else {
            Err(keyring_error(Error::Keyring("roundtrip".into())))
        }
    });
    let cleanup = vault.delete(&account_id(&account.phone));
    result.and(cleanup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oddity_platform::keyring::{MemorySecretStore, SecretEntry};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn account(phone: &str, refresh: &str) -> Account {
        Account {
            phone: phone.into(),
            password: None,
            device_id: "device".into(),
            uid: "uid".into(),
            refresh_token: refresh.into(),
            laohu_token: "laohu-secret".into(),
            laohu_user_id: "user".into(),
        }
    }

    fn store() -> Arc<MemorySecretStore> {
        Arc::new(MemorySecretStore::new())
    }

    #[test]
    fn reads_legacy_account_and_array_without_exposing_invalid_secret() {
        let legacy = serde_json::json!({
            "phone": "13800138000", "password": null, "deviceId": "device",
            "uid": "uid", "refreshToken": "refresh-secret",
            "laohuToken": "laohu-secret", "laohuUserId": "user"
        });
        let accounts = decode_accounts(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].phone, "13800138000");
        let encoded = serde_json::to_vec(&accounts).unwrap();
        assert!(encoded.starts_with(b"["));
        assert_eq!(
            decode_accounts(&encoded).unwrap()[0].refresh_token,
            "refresh-secret"
        );
        assert!(decode_accounts(b"[]").unwrap().is_empty());
        let error = decode_accounts(b"private-secret").err().unwrap();
        assert_eq!(error.kind, FailureKind::Keyring);
        assert!(!error.message.contains("private-secret"));
    }

    #[test]
    fn single_entry_round_trip_reuses_the_legacy_attributes() {
        let store = store();
        let vault = KeyringVault::with_store("scope-a", store.clone());
        assert!(vault.load_all().unwrap().is_empty());

        vault.save(&account("13800138000", "refresh-1")).unwrap();
        let entries = store
            .search(&SecretQuery::new(SERVICE).with("username", "scope-a"))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].attributes.len(), 1);
        assert_eq!(entries[0].attributes["username"], "scope-a");

        vault.save(&account("13800138000", "refresh-2")).unwrap();
        let accounts = vault.load_all().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].refresh_token, "refresh-2");

        // 第二个账号写进同一条目, 密钥环里仍然只有一个条目。
        vault.save(&account("13900139000", "refresh-3")).unwrap();
        assert_eq!(vault.load_all().unwrap().len(), 2);
        assert_eq!(store.search(&SecretQuery::new(SERVICE)).unwrap().len(), 1);

        vault
            .delete(&super::super::account_id("13800138000"))
            .unwrap();
        assert_eq!(vault.load_all().unwrap().len(), 1);
        vault
            .delete(&super::super::account_id("13900139000"))
            .unwrap();
        assert!(vault.load_all().unwrap().is_empty());
        assert!(store.search(&SecretQuery::new(SERVICE)).unwrap().is_empty());
    }

    #[test]
    fn duplicate_entries_are_reported_before_reading_or_writing() {
        struct DuplicateStore {
            store: Arc<MemorySecretStore>,
            duplicate: AtomicBool,
        }
        impl SecretStore for DuplicateStore {
            fn search(&self, query: &SecretQuery) -> oddity_platform::Result<Vec<SecretEntry>> {
                let mut entries = self.store.search(query)?;
                if self.duplicate.load(Ordering::Relaxed) {
                    entries.push(SecretEntry {
                        attributes: query.attributes.clone(),
                        secret: b"[]".to_vec(),
                    });
                }
                Ok(entries)
            }
            fn write(
                &self,
                query: &SecretQuery,
                label: &str,
                content_type: &str,
                secret: &[u8],
            ) -> oddity_platform::Result<()> {
                self.store.write(query, label, content_type, secret)
            }
            fn delete(&self, query: &SecretQuery) -> oddity_platform::Result<()> {
                self.store.delete(query)
            }
        }

        let inner = store();
        let store = Arc::new(DuplicateStore {
            store: inner,
            duplicate: AtomicBool::new(false),
        });
        let vault = KeyringVault::with_store("scope-b", store.clone());
        vault.save(&account("13800138000", "refresh")).unwrap();
        store.duplicate.store(true, Ordering::Relaxed);
        for error in [
            vault.load_all().err().unwrap(),
            vault
                .save(&account("13900139000", "refresh"))
                .err()
                .unwrap(),
            vault
                .delete(&super::super::account_id("13800138000"))
                .err()
                .unwrap(),
        ] {
            assert_eq!(error.kind, FailureKind::Keyring);
            assert!(
                error.message.contains("重复的塔吉多条目"),
                "{}",
                error.message
            );
        }
    }

    #[test]
    fn store_failure_is_reported_as_a_keyring_failure() {
        let store = store();
        store.set_failing(true);
        let vault = KeyringVault::with_store("scope-c", store);
        for error in [
            vault.load_all().err().unwrap(),
            vault
                .save(&account("13800138000", "refresh"))
                .err()
                .unwrap(),
            vault.delete("missing").err().unwrap(),
        ] {
            assert_eq!(error.kind, FailureKind::Keyring);
            assert_eq!(
                error.message,
                "无法访问 Keyring, 请确认桌面密钥环已启动并解锁"
            );
        }
    }

    #[test]
    fn check_writes_reads_and_removes_its_temporary_entry() {
        let store = store();
        check_keyring_with(store.clone()).expect("临时条目应完成一次往返并清理");
        assert!(store.search(&SecretQuery::new(SERVICE)).unwrap().is_empty());

        store.set_failing(true);
        assert_eq!(
            check_keyring_with(store).err().unwrap().kind,
            FailureKind::Keyring
        );
    }
}
