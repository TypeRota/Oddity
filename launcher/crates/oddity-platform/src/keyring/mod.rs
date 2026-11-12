//! 桌面密钥环: 按 service 与属性定位条目的最小读取, 写入与删除接口.
//!
//! 接口只处理原始字节与属性, 不感知账号模型; 真实实现使用 Secret Service,
//! 内存实现供测试注入.

mod memory;
mod secret_service;

pub use memory::MemorySecretStore;
pub use secret_service::SecretServiceStore;

use std::collections::BTreeMap;

use crate::Result;

/// 密钥环查询条件.
///
/// service 是应用命名空间, 写入时落到 Secret Service 的 service 属性,
/// 同时也是保留属性名, attributes 中的同名键不会覆盖它.
/// attributes 是附加定位条件, 读取与删除要求条目同时包含全部键值.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SecretQuery {
    pub service: String,
    pub attributes: BTreeMap<String, String>,
}

impl SecretQuery {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// 从密钥环读取的条目.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretEntry {
    /// 条目除 service 外的完整属性.
    pub attributes: BTreeMap<String, String>,
    /// 原始密钥内容, 由调用方解释; 敏感内容由调用方负责清零.
    pub secret: Vec<u8>,
}

/// 可注入的密钥环, 按 service 与属性定位条目.
///
/// 写入的定位条件与已有条目完全一致时覆盖该条目, 否则新建.
pub trait SecretStore: Send + Sync {
    /// 列出同时匹配 service 与全部 attributes 的条目.
    fn search(&self, query: &SecretQuery) -> Result<Vec<SecretEntry>>;

    /// 按定位条件写入原始内容.
    fn write(
        &self,
        query: &SecretQuery,
        label: &str,
        content_type: &str,
        secret: &[u8],
    ) -> Result<()>;

    /// 删除所有匹配的条目.
    fn delete(&self, query: &SecretQuery) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_builder_keeps_service_and_attributes_separate() {
        let query = SecretQuery::new("io.github.oddity.test")
            .with("game", "nte")
            .with("distribution", "cn");
        assert_eq!(query.service, "io.github.oddity.test");
        assert_eq!(
            query.attributes,
            BTreeMap::from([
                ("distribution".to_string(), "cn".to_string()),
                ("game".to_string(), "nte".to_string()),
            ])
        );
        assert!(SecretQuery::default().attributes.is_empty());
    }
}
