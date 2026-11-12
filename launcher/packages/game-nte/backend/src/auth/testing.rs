//! 账号域测试辅助: 合成的官方资源包, 内存密钥环与回环主机.

use super::{client::Endpoints, Device, Record, Session, Vault};
use crate::testing::{HttpServer, TempDir};
use oddity_platform::keyring::MemorySecretStore;
use std::sync::Arc;
use zeroize::Zeroizing;

pub(crate) const APP_KEY: &str = "0123456789abcdef0123456789abcdef";
pub(crate) const CHANNEL_KEY: &str = "oddity-channel-key-0123456789";
pub(crate) const PHONE: &str = "13800138000";

/// game.ini 里的密钥按相反顺序存放, 读取时再反转.
pub(crate) fn ini(app_key: &str) -> String {
    let reversed = String::from_utf8(app_key.bytes().rev().collect()).expect("测试密钥为 ASCII");
    format!("[Game]\noneKey={reversed}\noneAppId=1289\n")
}

/// 加密前的 SDK 配置; 渠道参数必须与国服启动器一致.
pub(crate) fn sdk_config(channel_key: &str) -> String {
    format!(
        r#"{{"meta":{{"channelId":9,"platform":4}},"init":{{"channel":{{"channelAppId":"10518","channelAppKey":"{channel_key}"}}}}}}"#
    )
}

fn sealed_config(channel_key: &str, app_key: &str) -> Vec<u8> {
    // 无效应用密钥在解密前就会被拒绝, 占位内容不需要能解开.
    super::encrypt(sdk_config(channel_key).as_bytes(), app_key.as_bytes())
        .map(String::into_bytes)
        .unwrap_or_default()
}

/// 完整资源包, 或只保留指定文件 (可选重复) 用于缺失与重复校验.
pub(crate) fn blob(app_key: &str, channel_key: &str, only: Option<(&str, bool)>) -> Vec<u8> {
    let ini_text = ini(app_key);
    let sdk = sealed_config(channel_key, app_key);
    let all: [(&str, &[u8]); 2] = [
        ("game.ini", ini_text.as_bytes()),
        ("oneSdkConfig.config", sdk.as_slice()),
    ];
    match only {
        None => pack(&all),
        Some((name, duplicate)) => {
            let entry = all
                .iter()
                .find(|(file, _)| *file == name)
                .expect("测试文件存在");
            if duplicate {
                pack(&[*entry, *entry])
            } else {
                pack(&[*entry])
            }
        }
    }
}

/// 替换 game.ini 的完整资源包.
pub(crate) fn blob_with_ini(ini_text: &str, app_key: &str, channel_key: &str) -> Vec<u8> {
    let sdk = sealed_config(channel_key, app_key);
    pack(&[
        ("game.ini", ini_text.as_bytes()),
        ("oneSdkConfig.config", sdk.as_slice()),
    ])
}

/// 替换加密前 SDK 配置的完整资源包.
pub(crate) fn blob_with_sdk(app_key: &str, sdk: &str) -> Vec<u8> {
    let sealed = super::encrypt(sdk.as_bytes(), app_key.as_bytes())
        .expect("加密测试 SDK 配置")
        .into_bytes();
    pack(&[
        ("game.ini", ini(app_key).as_bytes()),
        ("oneSdkConfig.config", sealed.as_slice()),
    ])
}

/// 官方资源包格式: qres 头, 扁平目录树, 数据区与 UTF-16BE 名称区.
pub(crate) fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut names = Vec::new();
    let mut index: Vec<(&str, u32)> = Vec::new();
    for name in ["root", "main"]
        .into_iter()
        .chain(files.iter().map(|(name, _)| *name))
    {
        if index.iter().any(|(known, _)| *known == name) {
            continue;
        }
        let offset = names.len() as u32;
        let units: Vec<u16> = name.encode_utf16().collect();
        names.extend_from_slice(&(units.len() as u16).to_be_bytes());
        names.extend_from_slice(&[0, 0, 0, 0]);
        for unit in units {
            names.extend_from_slice(&unit.to_be_bytes());
        }
        index.push((name, offset));
    }
    let offset = |name: &str| {
        index
            .iter()
            .find(|(known, _)| *known == name)
            .expect("名称已登记")
            .1
    };
    let mut data = Vec::new();
    let mut data_offsets = Vec::new();
    for (_, content) in files {
        data_offsets.push(data.len() as u32);
        data.extend_from_slice(&(content.len() as u32).to_be_bytes());
        data.extend_from_slice(content);
    }
    let mut tree = vec![0u8; (files.len() + 2) * 22];
    put_node(&mut tree, 0, offset("root"), 2, 1, 1);
    put_node(&mut tree, 1, offset("main"), 2, files.len() as u32, 2);
    for (slot, (name, _)) in files.iter().enumerate() {
        put_node(&mut tree, slot + 2, offset(name), 0, 0, data_offsets[slot]);
    }
    let tree_at = 20u32;
    let data_at = tree_at + tree.len() as u32;
    let names_at = data_at + data.len() as u32;
    let mut blob = Vec::new();
    blob.extend_from_slice(b"qres");
    for value in [3, tree_at, data_at, names_at] {
        blob.extend_from_slice(&value.to_be_bytes());
    }
    blob.extend_from_slice(&tree);
    blob.extend_from_slice(&data);
    blob.extend_from_slice(&names);
    blob
}

fn put_node(tree: &mut [u8], slot: usize, name: u32, flags: u16, count: u32, child: u32) {
    let at = slot * 22;
    tree[at..at + 4].copy_from_slice(&name.to_be_bytes());
    tree[at + 4..at + 6].copy_from_slice(&flags.to_be_bytes());
    tree[at + 6..at + 10].copy_from_slice(&count.to_be_bytes());
    tree[at + 10..at + 14].copy_from_slice(&child.to_be_bytes());
}

/// 写入合成资源包的启动器根目录.
pub(crate) fn config_dir(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("ResData/1289.dat", blob(APP_KEY, CHANNEL_KEY, None));
    dir
}

/// 内存密钥环与注入后的仓库.
pub(crate) fn vault() -> (Arc<MemorySecretStore>, Vault) {
    let store = Arc::new(MemorySecretStore::new());
    let vault = Vault::with_store(store.clone());
    (store, vault)
}

/// 全部接口指向同一个回环服务.
pub(crate) fn endpoints(server: &HttpServer) -> Endpoints {
    endpoints_at(&server.base())
}

/// 全部接口指向同一个主机, 用于连接不可达服务的用例.
pub(crate) fn endpoints_at(base: &str) -> Endpoints {
    Endpoints {
        user: base.into(),
        api: base.into(),
        gm: base.into(),
        antifatigue: base.into(),
    }
}

/// 与官方实现一致的主账号响应密文.
pub(crate) fn sealed(app_key: &str, plain: &str) -> Vec<u8> {
    super::encrypt(plain.as_bytes(), app_key.as_bytes())
        .expect("加密测试响应")
        .into_bytes()
}

pub(crate) fn record(account_id: &str, channel_uid: &str) -> Record {
    Record {
        version: 1,
        account_id: account_id.into(),
        phone: PHONE.into(),
        channel_uid: channel_uid.into(),
        channel_token: "channel-token".into(),
        device: Device::new("b".repeat(32)).expect("测试设备编号"),
    }
}

pub(crate) fn session() -> Session {
    Session {
        uid: "master-uid".into(),
        token: Zeroizing::new("master-token".into()),
        extra_one: serde_json::json!({"userIp": "192.0.2.1", "version": 1}),
        record: record(&"a".repeat(32), "channel-1"),
    }
}
