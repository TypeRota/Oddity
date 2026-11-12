//! Secret Service 实现. 连接桌面会话总线, 读写默认集合中按属性定位的条目.

use secret_service::blocking::{Collection, SecretService};
use secret_service::EncryptionType;
use std::collections::HashMap;

use super::{SecretEntry, SecretQuery, SecretStore};
use crate::{Error, Result};

/// 桌面 Secret Service 中的默认集合.
#[derive(Clone, Copy, Debug, Default)]
pub struct SecretServiceStore {
    // 测试里的模拟服务只实现 plain 会话; 生产始终使用 DH.
    #[cfg(test)]
    plain: bool,
}

impl SecretServiceStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn plain() -> Self {
        Self { plain: true }
    }

    fn with_collection<T>(&self, action: impl FnOnce(&Collection<'_>) -> Result<T>) -> Result<T> {
        let service = SecretService::connect(self.encryption()).map_err(keyring)?;
        let collection = service.get_default_collection().map_err(keyring)?;
        collection.ensure_unlocked().map_err(keyring)?;
        action(&collection)
    }

    fn encryption(&self) -> EncryptionType {
        #[cfg(test)]
        if self.plain {
            return EncryptionType::Plain;
        }
        EncryptionType::Dh
    }
}

impl SecretStore for SecretServiceStore {
    fn search(&self, query: &SecretQuery) -> Result<Vec<SecretEntry>> {
        self.with_collection(|collection| {
            collection
                .search_items(attributes(query))
                .map_err(keyring)?
                .into_iter()
                .map(|item| {
                    item.ensure_unlocked().map_err(keyring)?;
                    let secret = item.get_secret().map_err(keyring)?;
                    let mut attributes = item.get_attributes().map_err(keyring)?;
                    attributes.remove("service");
                    Ok(SecretEntry {
                        attributes: attributes.into_iter().collect(),
                        secret,
                    })
                })
                .collect()
        })
    }

    fn write(
        &self,
        query: &SecretQuery,
        label: &str,
        content_type: &str,
        secret: &[u8],
    ) -> Result<()> {
        self.with_collection(|collection| {
            collection
                .create_item(label, attributes(query), secret, true, content_type)
                .map_err(keyring)?;
            Ok(())
        })
    }

    fn delete(&self, query: &SecretQuery) -> Result<()> {
        self.with_collection(|collection| {
            for item in collection
                .search_items(attributes(query))
                .map_err(keyring)?
            {
                item.ensure_unlocked().map_err(keyring)?;
                item.delete().map_err(keyring)?;
            }
            Ok(())
        })
    }
}

fn attributes(query: &SecretQuery) -> HashMap<&str, &str> {
    let mut attributes = HashMap::with_capacity(query.attributes.len() + 1);
    for (key, value) in &query.attributes {
        attributes.insert(key.as_str(), value.as_str());
    }
    // service 是保留属性, 不允许被附加条件覆盖.
    attributes.insert("service", query.service.as_str());
    attributes
}

fn keyring(error: impl std::fmt::Display) -> Error {
    Error::Keyring(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex, MutexGuard};

    use crate::testing::TestBus;
    use zbus::blocking::{connection::Builder, Connection};
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

    const SERVICE_PATH: &str = "/org/freedesktop/secrets";
    const COLLECTION_PATH: &str = "/org/freedesktop/secrets/collection/default";
    const SESSION_PATH: &str = "/org/freedesktop/secrets/session/session";
    const ITEM_PATHS: [&str; 8] = [
        "/org/freedesktop/secrets/collection/default/item/0",
        "/org/freedesktop/secrets/collection/default/item/1",
        "/org/freedesktop/secrets/collection/default/item/2",
        "/org/freedesktop/secrets/collection/default/item/3",
        "/org/freedesktop/secrets/collection/default/item/4",
        "/org/freedesktop/secrets/collection/default/item/5",
        "/org/freedesktop/secrets/collection/default/item/6",
        "/org/freedesktop/secrets/collection/default/item/7",
    ];

    #[derive(Clone, Default)]
    struct MockItem {
        present: bool,
        attributes: HashMap<String, String>,
        secret: Vec<u8>,
        content_type: String,
    }

    struct MockState {
        items: Vec<MockItem>,
    }

    impl MockState {
        fn new() -> Self {
            Self {
                items: vec![MockItem::default(); ITEM_PATHS.len()],
            }
        }
    }

    struct MockSecretService {
        _connection: Connection,
    }

    impl MockSecretService {
        fn start() -> Self {
            let state = Arc::new(Mutex::new(MockState::new()));
            let mut builder = Builder::session()
                .expect("连接测试总线")
                .name("org.freedesktop.secrets")
                .expect("占用 Secret Service 名称")
                .serve_at(SERVICE_PATH, ServiceMock)
                .expect("注册服务接口")
                .serve_at(
                    COLLECTION_PATH,
                    CollectionMock {
                        state: state.clone(),
                    },
                )
                .expect("注册集合接口");
            for (index, path) in ITEM_PATHS.iter().enumerate() {
                builder = builder
                    .serve_at(
                        *path,
                        ItemMock {
                            index,
                            state: state.clone(),
                        },
                    )
                    .expect("注册条目接口");
            }
            Self {
                _connection: builder.build().expect("启动模拟 Secret Service"),
            }
        }
    }

    struct ServiceMock;

    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl ServiceMock {
        fn open_session(
            &self,
            _algorithm: &str,
            _input: Value<'_>,
        ) -> (OwnedValue, OwnedObjectPath) {
            (
                OwnedValue::try_from(Value::from("")).expect("空会话输出"),
                object_path(SESSION_PATH),
            )
        }

        fn read_alias(&self, _name: &str) -> OwnedObjectPath {
            object_path(COLLECTION_PATH)
        }
    }

    struct CollectionMock {
        state: Arc<Mutex<MockState>>,
    }

    #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
    impl CollectionMock {
        #[zbus(property)]
        fn locked(&self) -> bool {
            false
        }

        fn search_items(&self, attributes: HashMap<String, String>) -> Vec<OwnedObjectPath> {
            let state = lock(&self.state);
            state
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.present && contains(item, &attributes))
                .map(|(index, _)| object_path(ITEM_PATHS[index]))
                .collect()
        }

        fn create_item(
            &self,
            properties: HashMap<String, OwnedValue>,
            secret: (OwnedObjectPath, Vec<u8>, Vec<u8>, String),
            replace: bool,
        ) -> (OwnedObjectPath, OwnedObjectPath) {
            let attributes: HashMap<String, String> = properties
                .get("org.freedesktop.Secret.Item.Attributes")
                .expect("条目属性")
                .try_clone()
                .expect("复制条目属性")
                .try_into()
                .expect("解析条目属性");
            let (_session, _parameters, secret, content_type) = secret;
            let mut state = lock(&self.state);
            let existing = if replace {
                state
                    .items
                    .iter()
                    .position(|item| item.present && item.attributes == attributes)
            } else {
                None
            };
            let index = match existing {
                Some(index) => {
                    state.items[index].secret = secret;
                    state.items[index].content_type = content_type;
                    index
                }
                None => {
                    let index = state
                        .items
                        .iter()
                        .position(|item| !item.present)
                        .expect("空闲条目槽位");
                    state.items[index] = MockItem {
                        present: true,
                        attributes,
                        secret,
                        content_type,
                    };
                    index
                }
            };
            (object_path(ITEM_PATHS[index]), object_path("/"))
        }
    }

    struct ItemMock {
        index: usize,
        state: Arc<Mutex<MockState>>,
    }

    #[zbus::interface(name = "org.freedesktop.Secret.Item")]
    impl ItemMock {
        #[zbus(property)]
        fn locked(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn attributes(&self) -> HashMap<String, String> {
            lock(&self.state).items[self.index].attributes.clone()
        }

        fn get_secret(
            &self,
            _session: OwnedObjectPath,
        ) -> (OwnedObjectPath, Vec<u8>, Vec<u8>, String) {
            let state = lock(&self.state);
            let item = &state.items[self.index];
            (
                object_path(SESSION_PATH),
                Vec::new(),
                item.secret.clone(),
                item.content_type.clone(),
            )
        }

        fn delete(&self) -> OwnedObjectPath {
            let mut state = lock(&self.state);
            state.items[self.index].present = false;
            object_path("/")
        }
    }

    fn lock(state: &Mutex<MockState>) -> MutexGuard<'_, MockState> {
        state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn object_path(path: &str) -> OwnedObjectPath {
        ObjectPath::try_from(path).expect("有效对象路径").into()
    }

    fn contains(item: &MockItem, attributes: &HashMap<String, String>) -> bool {
        attributes
            .iter()
            .all(|(key, value)| item.attributes.get(key) == Some(value))
    }

    #[test]
    fn plain_session_round_trips_entries_through_the_default_collection() {
        let _bus = TestBus::start();
        let _service = MockSecretService::start();
        let store = SecretServiceStore::plain();
        let query = SecretQuery::new("io.github.oddity.test").with("game", "nte");

        assert!(store.search(&query).unwrap().is_empty());
        store
            .write(
                &query.clone().with("account", "a"),
                "甲",
                "application/json",
                b"secret-a",
            )
            .unwrap();
        store
            .write(
                &query.clone().with("account", "b"),
                "乙",
                "text/plain",
                b"secret-b",
            )
            .unwrap();

        let entries = store.search(&query).unwrap();
        assert_eq!(entries.len(), 2);
        let first = entries
            .iter()
            .find(|entry| entry.attributes["account"] == "a")
            .expect("账号甲");
        assert_eq!(first.secret, b"secret-a");
        assert_eq!(first.attributes["game"], "nte");
        assert!(!first.attributes.contains_key("service"));
        assert_eq!(
            store
                .search(&query.clone().with("account", "b"))
                .unwrap()
                .len(),
            1
        );

        store
            .write(
                &query.clone().with("account", "a"),
                "甲",
                "application/json",
                b"updated",
            )
            .unwrap();
        let entries = store.search(&query).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.attributes["account"] == "a")
                .expect("账号甲")
                .secret,
            b"updated"
        );

        store.delete(&query.clone().with("account", "a")).unwrap();
        let entries = store.search(&query).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].attributes["account"], "b");

        store.delete(&query).unwrap();
        assert!(store.search(&query).unwrap().is_empty());
    }

    #[test]
    fn attributes_merge_service_and_query_attributes() {
        let query = SecretQuery::new("io.github.oddity.test")
            .with("game", "nte")
            .with("account", "a");
        let attributes = attributes(&query);
        assert_eq!(attributes.len(), 3);
        assert_eq!(attributes["service"], "io.github.oddity.test");
        assert_eq!(attributes["game"], "nte");
        assert_eq!(attributes["account"], "a");
    }

    #[test]
    fn query_attributes_cannot_replace_the_service_namespace() {
        let query = SecretQuery::new("io.github.oddity.test").with("service", "other");
        assert_eq!(attributes(&query)["service"], "io.github.oddity.test");
    }

    #[test]
    fn missing_secret_service_is_reported_as_a_keyring_error() {
        let _bus = crate::testing::TestBus::start();
        let store = SecretServiceStore::new();
        let query = SecretQuery::new("io.github.oddity.test").with("game", "nte");
        for result in [
            store.search(&query).map(|_| ()),
            store.write(&query, "标签", "text/plain", b"value"),
            store.delete(&query),
        ] {
            assert!(matches!(result, Err(Error::Keyring(_))));
        }
    }
}
