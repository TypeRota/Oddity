use super::*;
use std::path::Path;

pub(super) struct Config {
    pub app_key: Zeroizing<Vec<u8>>,
    pub channel_key: Zeroizing<Vec<u8>>,
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
        let ini = crate::config::Ini::parse(
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
