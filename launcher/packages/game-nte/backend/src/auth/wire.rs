//! SDK 会话帧与查询回包: 定长小端字段, 由游戏侧按官方协议解析.

use super::{protocol, Result, Session};
use zeroize::Zeroizing;

fn word(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
impl Session {
    pub fn packet(&self) -> Result<Zeroizing<Vec<u8>>> {
        let extra = Zeroizing::new(serde_json::to_vec(&self.extra_one).map_err(|_| protocol())?);
        let ip = self
            .extra_one
            .get("userIp")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let extra_size = extra.len() + 17;
        let size = 20 + 0x594 + 0x70 + extra_size;
        if size > 65536 || ip.len() >= 100 || ip.contains('\0') {
            return Err(protocol());
        }
        let mut packet = Zeroizing::new(vec![0u8; size]);
        for (i, value) in [0x5253444f, 1, 0x594, 0x70, extra_size as u32]
            .into_iter()
            .enumerate()
        {
            word(&mut packet, i * 4, value);
        }
        let login = &mut packet[20..20 + 0x594];
        for (i, value) in [0x6b647331, 2, 0x594].into_iter().enumerate() {
            word(login, i * 4, value);
        }
        for (value, offset, capacity) in [
            (self.uid.as_str(), 12, 64),
            (self.token.as_str(), 0x4c, 128),
            (self.record.channel_uid.as_str(), 0xd4, 64),
            (self.record.channel_token.as_str(), 0x114, 128),
        ] {
            if value.is_empty() || value.len() >= capacity || value.contains('\0') {
                return Err(protocol());
            }
            login[offset..offset + value.len()].copy_from_slice(value.as_bytes());
        }
        word(login, 0xcc, 4);
        word(login, 0xd0, 9);
        let start = 20 + 0x594;
        for (i, value) in [0x6b647331, 0x3ed, 0x70].into_iter().enumerate() {
            word(&mut packet, start + i * 4, value);
        }
        packet[start + 12..start + 12 + ip.len()].copy_from_slice(ip.as_bytes());
        let start = start + 0x70;
        for (i, value) in [0x6b647331, 0x40d, extra_size as u32, extra.len() as u32]
            .into_iter()
            .enumerate()
        {
            word(&mut packet, start + i * 4, value);
        }
        packet[start + 16..start + 16 + extra.len()].copy_from_slice(&extra);
        Ok(packet)
    }
}
pub fn query_reply(kind: u32, id: u32, code: i32, message: &str, body: &[u8]) -> Result<Vec<u8>> {
    if body.len() > 65536 - 0x422 || body.contains(&0) {
        return Err(protocol());
    }
    let size = body.len() + 0x422;
    let mut result = vec![0; size + 4];
    word(&mut result, 0, size as u32);
    for (i, value) in [
        0x6b647331,
        49,
        size as u32,
        (size - 17) as u32,
        kind,
        code as u32,
        id,
    ]
    .into_iter()
    .enumerate()
    {
        word(&mut result, 4 + i * 4, value);
    }
    for (i, value) in message.encode_utf16().take(511).enumerate() {
        result[4 + 0x1c + i * 2..4 + 0x1c + i * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    result[4 + 0x41c..4 + 0x41c + body.len()].copy_from_slice(body);
    Ok(result)
}
