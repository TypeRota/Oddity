//! 官方启动器里的账号认证配置: 资源包中的 game.ini 与加密 SDK 配置.
//!
//! 只读取 `ResData/1289.dat` 里的两个文件, 校验协议版本与渠道参数;
//! 任何结构或取值不符都按协议错误处理, 不读取其他资源.

use super::{decrypt, protocol, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use zeroize::Zeroizing;

pub(super) struct Config {
    pub(super) app_key: Zeroizing<Vec<u8>>,
    pub(super) channel_key: Zeroizing<Vec<u8>>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let blob = std::fs::read(root.join("ResData/1289.dat")).map_err(|_| protocol())?;
        let number = |at: usize| -> Result<usize> {
            Ok(u32::from_be_bytes(
                blob.get(at..at + 4)
                    .ok_or_else(protocol)?
                    .try_into()
                    .map_err(|_| protocol())?,
            ) as usize)
        };
        if blob.get(..4) != Some(b"qres") || number(4)? != 3 {
            return Err(protocol());
        }
        let (tree, data, names) = (number(8)?, number(12)?, number(16)?);
        let mut pending = vec![(0usize, String::new())];
        let mut found = BTreeMap::new();
        let mut visited = std::collections::HashSet::new();
        while let Some((index, prefix)) = pending.pop() {
            if !visited.insert(index) {
                return Err(protocol());
            }
            let node = tree
                .checked_add(index.checked_mul(22).ok_or_else(protocol)?)
                .ok_or_else(protocol)?;
            let name_at = names.checked_add(number(node)?).ok_or_else(protocol)?;
            let len = u16::from_be_bytes(
                blob.get(name_at..name_at + 2)
                    .ok_or_else(protocol)?
                    .try_into()
                    .map_err(|_| protocol())?,
            ) as usize;
            let name_bytes = blob
                .get(name_at + 6..name_at + 6 + len * 2)
                .ok_or_else(protocol)?;
            let name = String::from_utf16(
                &name_bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| protocol())?;
            let flags = u16::from_be_bytes(
                blob.get(node + 4..node + 6)
                    .ok_or_else(protocol)?
                    .try_into()
                    .map_err(|_| protocol())?,
            );
            let (count, child) = (number(node + 6)?, number(node + 10)?);
            let path = format!("{prefix}/{name}");
            if flags & 2 != 0 {
                if count > blob.len() / 22 {
                    return Err(protocol());
                }
                pending.extend((child..child + count).map(|i| (i, path.clone())));
            } else if path.ends_with("/main/game.ini")
                || path.ends_with("/main/oneSdkConfig.config")
            {
                if flags != 0 || found.contains_key(&name) {
                    return Err(protocol());
                }
                let at = data.checked_add(child).ok_or_else(protocol)?;
                let size = number(at)?;
                found.insert(name, blob.get(at + 4..at + 4 + size).ok_or_else(protocol)?);
            }
        }
        let ini = Ini::parse(
            std::str::from_utf8(found.get("game.ini").ok_or_else(protocol)?)
                .map_err(|_| protocol())?,
        );
        let app_key = Zeroizing::new(
            ini.get("Game", "oneKey")
                .ok_or_else(protocol)?
                .bytes()
                .rev()
                .collect::<Vec<_>>(),
        );
        if app_key.len() != 32 || ini.get("Game", "oneAppId") != Some("1289") {
            return Err(protocol());
        }
        let plain = decrypt(
            found.get("oneSdkConfig.config").ok_or_else(protocol)?,
            &app_key,
        )?;
        #[derive(Deserialize)]
        struct Sdk {
            meta: Meta,
            init: Init,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Meta {
            channel_id: u32,
            platform: u32,
        }
        #[derive(Deserialize)]
        struct Init {
            channel: Channel,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Channel {
            channel_app_id: String,
            channel_app_key: String,
        }
        let sdk: Sdk = serde_json::from_slice(&plain).map_err(|_| protocol())?;
        let channel_key = Zeroizing::new(sdk.init.channel.channel_app_key.into_bytes());
        if sdk.meta.channel_id != 9
            || sdk.meta.platform != 4
            || sdk.init.channel.channel_app_id != "10518"
            || channel_key.len() < 16
        {
            return Err(protocol());
        }
        Ok(Self {
            app_key,
            channel_key,
        })
    }
}

/// 精简 INI 读取: 容忍 BOM, CRLF, 注释与重复键.
#[derive(Default)]
struct Ini {
    sections: BTreeMap<String, BTreeMap<String, String>>,
}
impl Ini {
    fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        let mut section = String::from("General");
        ini.sections.entry(section.clone()).or_default();
        for raw in text.trim_start_matches('\u{feff}').lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = rest.trim().to_string();
                ini.sections.entry(section.clone()).or_default();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                ini.sections
                    .entry(section.clone())
                    .or_default()
                    .insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        ini
    }
    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::{self, APP_KEY, CHANNEL_KEY};
    use crate::testing::TempDir;

    fn set(blob: &mut [u8], at: usize, value: u32) {
        blob[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    fn section(blob: &[u8], at: usize) -> usize {
        u32::from_be_bytes(blob[at..at + 4].try_into().unwrap()) as usize
    }
    fn node(blob: &[u8], index: usize) -> usize {
        section(blob, 8) + index * 22
    }
    fn loaded(blob: &[u8]) -> Result<Config> {
        let dir = TempDir::new("auth-config");
        dir.write("ResData/1289.dat", blob);
        Config::load(dir.path())
    }

    #[test]
    fn reads_the_app_and_channel_keys_from_the_shipped_archive() {
        let config = loaded(&testing::blob(APP_KEY, CHANNEL_KEY, None)).expect("配置应通过校验");
        assert_eq!(config.app_key.as_slice(), APP_KEY.as_bytes());
        assert_eq!(config.channel_key.as_slice(), CHANNEL_KEY.as_bytes());
    }

    #[test]
    fn missing_archives_are_protocol_failures() {
        let dir = TempDir::new("auth-config-missing");
        assert!(matches!(Config::load(dir.path()), Err(error) if error.kind == "protocol"));
        for blob in [b"".to_vec(), b"qres".to_vec(), b"xres\0\0\0\x03".to_vec()] {
            assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");
        }
    }

    #[test]
    fn unsupported_versions_and_truncated_sections_are_rejected() {
        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        set(&mut blob, 4, 2);
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");

        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        set(&mut blob, 8, u32::MAX);
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");

        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        let data = section(&blob, 12);
        set(&mut blob, data, u32::MAX);
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");

        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        let tree = section(&blob, 8);
        set(&mut blob, tree, u32::MAX);
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");

        // 目录节点成环时按已访问节点终止.
        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        let main = node(&blob, 1);
        set(&mut blob, main + 10, 0);
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");

        // 子节点数量超过树容量时终止.
        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        let root = node(&blob, 0);
        set(&mut blob, root + 6, 1000);
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");
    }

    #[test]
    fn missing_or_duplicated_files_are_rejected() {
        assert_eq!(
            loaded(&testing::blob(
                APP_KEY,
                CHANNEL_KEY,
                Some(("game.ini", false))
            ))
            .err()
            .unwrap()
            .kind,
            "protocol"
        );
        assert_eq!(
            loaded(&testing::blob(
                APP_KEY,
                CHANNEL_KEY,
                Some(("oneSdkConfig.config", false))
            ))
            .err()
            .unwrap()
            .kind,
            "protocol"
        );
        assert_eq!(
            loaded(&testing::blob(
                APP_KEY,
                CHANNEL_KEY,
                Some(("game.ini", true))
            ))
            .err()
            .unwrap()
            .kind,
            "protocol"
        );
    }

    #[test]
    fn invalid_keys_and_sdk_metadata_are_rejected() {
        for (app_key, channel_key) in [("short", CHANNEL_KEY), (APP_KEY, "short-key")] {
            let error = loaded(&testing::blob(app_key, channel_key, None))
                .err()
                .unwrap();
            assert_eq!(error.kind, "protocol");
        }

        let mut ini = testing::ini(APP_KEY);
        ini = ini.replace("oneAppId=1289", "oneAppId=3000001");
        assert_eq!(
            loaded(&testing::blob_with_ini(&ini, APP_KEY, CHANNEL_KEY))
                .err()
                .unwrap()
                .kind,
            "protocol"
        );

        let mut sdk = testing::sdk_config(CHANNEL_KEY);
        sdk = sdk.replace("\"channelId\":9", "\"channelId\":8");
        assert_eq!(
            loaded(&testing::blob_with_sdk(APP_KEY, &sdk))
                .err()
                .unwrap()
                .kind,
            "protocol"
        );
        let mut sdk = testing::sdk_config(CHANNEL_KEY);
        sdk = sdk.replace("\"platform\":4", "\"platform\":3");
        assert_eq!(
            loaded(&testing::blob_with_sdk(APP_KEY, &sdk))
                .err()
                .unwrap()
                .kind,
            "protocol"
        );
        let mut sdk = testing::sdk_config(CHANNEL_KEY);
        sdk = sdk.replace("\"channelAppId\":\"10518\"", "\"channelAppId\":\"10517\"");
        assert_eq!(
            loaded(&testing::blob_with_sdk(APP_KEY, &sdk))
                .err()
                .unwrap()
                .kind,
            "protocol"
        );
        assert_eq!(
            loaded(&testing::blob_with_sdk(APP_KEY, "not-json"))
                .err()
                .unwrap()
                .kind,
            "protocol"
        );
    }

    #[test]
    fn invalid_text_and_ciphertext_are_rejected() {
        let error = loaded(&testing::pack(&[
            ("game.ini", b"\xff\xfe".as_slice()),
            ("oneSdkConfig.config", b"sealed".as_slice()),
        ]))
        .err()
        .unwrap();
        assert_eq!(error.kind, "protocol");

        let ini_text = testing::ini(APP_KEY);
        let error = loaded(&testing::pack(&[
            ("game.ini", ini_text.as_bytes()),
            ("oneSdkConfig.config", b"invalid".as_slice()),
        ]))
        .err()
        .unwrap();
        assert_eq!(error.kind, "protocol");
    }

    #[test]
    fn file_nodes_with_directory_flags_are_rejected() {
        let mut blob = testing::blob(APP_KEY, CHANNEL_KEY, None);
        let file = node(&blob, 2);
        blob[file + 4..file + 6].copy_from_slice(&1u16.to_be_bytes());
        assert_eq!(loaded(&blob).err().unwrap().kind, "protocol");
    }

    #[test]
    fn ini_reader_tolerates_comments_duplicates_and_bad_lines() {
        let ini = Ini::parse(
            "\u{feff}top=1\r\n; 注释\r\n[Game]\r\nKey = Value \r\nKey=Second\r\nempty=\r\nno_separator\r\n",
        );
        assert_eq!(ini.get("General", "top"), Some("1"));
        assert_eq!(ini.get("Game", "Key"), Some("Second"));
        assert_eq!(ini.get("Game", "empty"), Some(""));
        assert_eq!(ini.get("Game", "no_separator"), None);
        assert_eq!(ini.get("Missing", "Key"), None);
    }
}
