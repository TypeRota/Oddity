//! 内存密钥环, 供测试注入, 不访问桌面服务.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{SecretEntry, SecretQuery, SecretStore};
use crate::{Error, Result};

#[derive(Clone, Debug)]
struct MemoryEntry {
    service: String,
    attributes: BTreeMap<String, String>,
    secret: Vec<u8>,
}

/// 进程内的密钥环实现, 不访问桌面服务.
///
/// 克隆共享同一份条目; 需要模拟密钥环不可用的错误路径时用
/// [set_failing](Self::set_failing).
#[derive(Clone, Default)]
pub struct MemorySecretStore {
    entries: Arc<Mutex<Vec<MemoryEntry>>>,
    failing: Arc<AtomicBool>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 为 true 时所有操作返回 [Error::Keyring], 用于测试错误处理.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::Relaxed);
    }

    fn entries(&self) -> MutexGuard<'_, Vec<MemoryEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn check(&self) -> Result<()> {
        if self.failing.load(Ordering::Relaxed) {
            return Err(Error::Keyring("内存密钥环不可用".into()));
        }
        Ok(())
    }
}

fn attributes(query: &SecretQuery) -> BTreeMap<String, String> {
    // service 由查询单独定位, 不参与附加属性匹配.
    query
        .attributes
        .iter()
        .filter(|(key, _)| key.as_str() != "service")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn matches(entry: &MemoryEntry, service: &str, attributes: &BTreeMap<String, String>) -> bool {
    entry.service == service
        && attributes
            .iter()
            .all(|(key, value)| entry.attributes.get(key) == Some(value))
}

impl SecretStore for MemorySecretStore {
    fn search(&self, query: &SecretQuery) -> Result<Vec<SecretEntry>> {
        self.check()?;
        let attributes = attributes(query);
        Ok(self
            .entries()
            .iter()
            .filter(|entry| matches(entry, &query.service, &attributes))
            .map(|entry| SecretEntry {
                attributes: entry.attributes.clone(),
                secret: entry.secret.clone(),
            })
            .collect())
    }

    fn write(
        &self,
        query: &SecretQuery,
        _label: &str,
        _content_type: &str,
        secret: &[u8],
    ) -> Result<()> {
        self.check()?;
        let attributes = attributes(query);
        let mut entries = self.entries();
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.service == query.service && entry.attributes == attributes)
        {
            entry.secret = secret.to_vec();
        } else {
            entries.push(MemoryEntry {
                service: query.service.clone(),
                attributes,
                secret: secret.to_vec(),
            });
        }
        Ok(())
    }

    fn delete(&self, query: &SecretQuery) -> Result<()> {
        self.check()?;
        let attributes = attributes(query);
        self.entries()
            .retain(|entry| !matches(entry, &query.service, &attributes));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: &str = "application/octet-stream";

    fn query() -> SecretQuery {
        SecretQuery::new("io.github.oddity.test").with("game", "nte")
    }

    #[test]
    fn write_search_replace_and_delete_follow_the_locator() {
        let store = MemorySecretStore::new();
        assert!(store.search(&query()).unwrap().is_empty());

        store
            .write(
                &query().with("account", "a"),
                "账号甲",
                CONTENT,
                b"secret-a",
            )
            .unwrap();
        store
            .write(
                &query().with("account", "b"),
                "账号乙",
                CONTENT,
                b"secret-b",
            )
            .unwrap();

        let all = store.search(&query()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            store
                .search(&query().with("account", "b"))
                .unwrap()
                .remove(0)
                .secret,
            b"secret-b"
        );
        assert!(store
            .search(&SecretQuery::new("io.github.oddity.other"))
            .unwrap()
            .is_empty());

        store
            .write(&query().with("account", "a"), "账号甲", CONTENT, b"updated")
            .unwrap();
        let all = store.search(&query()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            store
                .search(&query().with("account", "a"))
                .unwrap()
                .remove(0)
                .secret,
            b"updated"
        );

        store.delete(&query().with("account", "a")).unwrap();
        let remaining = store.search(&query()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].attributes,
            BTreeMap::from([
                ("account".to_string(), "b".to_string()),
                ("game".to_string(), "nte".to_string()),
            ])
        );

        store.delete(&query()).unwrap();
        assert!(store.search(&query()).unwrap().is_empty());
    }

    #[test]
    fn a_service_change_is_a_different_entry() {
        let store = MemorySecretStore::new();
        store.write(&query(), "甲", CONTENT, b"a").unwrap();
        store
            .write(
                &SecretQuery::new("io.github.oddity.other").with("game", "nte"),
                "乙",
                CONTENT,
                b"b",
            )
            .unwrap();
        assert_eq!(store.search(&query()).unwrap().len(), 1);
        assert_eq!(
            store
                .search(&SecretQuery::new("io.github.oddity.other"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_service_attribute_in_the_query_does_not_override_the_namespace() {
        let store = MemorySecretStore::new();
        let query = query().with("service", "other");
        store.write(&query, "标签", CONTENT, b"value").unwrap();
        let entries = store.search(&query).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].attributes.contains_key("service"));
    }

    #[test]
    fn unavailable_store_reports_keyring_errors_for_every_operation() {
        let store = MemorySecretStore::new();
        store.set_failing(true);
        for result in [
            store.search(&query()).map(|_| ()),
            store.write(&query(), "标签", CONTENT, b"value"),
            store.delete(&query()),
        ] {
            assert!(matches!(result, Err(Error::Keyring(_))));
        }
    }

    #[test]
    fn cloned_store_shares_entries_across_the_dyn_trait() {
        let store = Arc::new(MemorySecretStore::new());
        let injected: Arc<dyn SecretStore> = store.clone();
        injected
            .write(&query(), "标签", CONTENT, b"shared")
            .unwrap();
        assert_eq!(store.search(&query()).unwrap()[0].secret, b"shared");
    }
}
