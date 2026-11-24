use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::attendance::GameRecord;

/// 当前 profile 的 ClassNetCache 索引上界, 函数索引按它决定读取位数.
const INDEX_MAXIMUM: u32 = 882;
/// 活跃度上限.
const ACTIVATION_MAXIMUM: u32 = 100;
/// 大亨计划结算响应的消息编号.
const SETTLE_MESSAGE: u64 = 152;
/// 100 ns tick 与毫秒的换算.
const TICKS_PER_MILLISECOND: u64 = 10_000;

#[derive(Clone)]
struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
    end: usize,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8], end: usize) -> Option<Self> {
        (end <= bytes.len().checked_mul(8)?).then_some(Self {
            bytes,
            position: 0,
            end,
        })
    }

    fn read(&mut self, count: usize) -> Option<u64> {
        if count > 64 || self.position.checked_add(count)? > self.end {
            return None;
        }
        let mut value = 0;
        for index in 0..count {
            let bit = self.position + index;
            value |= u64::from((self.bytes[bit / 8] >> (bit % 8)) & 1) << index;
        }
        self.position += count;
        Some(value)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.position = self.position.checked_add(count)?;
        (self.position <= self.end).then_some(())
    }

    fn packed(&mut self) -> Option<u32> {
        let mut value = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = self.read(8)? as u32;
            if shift == 28 && byte >> 1 > 15 {
                return None;
            }
            value |= (byte >> 1) << shift;
            if byte & 1 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn data(&mut self, count: usize) -> Option<Vec<u8>> {
        if self.position.checked_add(count)? > self.end {
            return None;
        }
        let mut bytes = Vec::with_capacity(count.div_ceil(8));
        for _ in 0..count / 8 {
            bytes.push(self.read(8)? as u8);
        }
        if !count.is_multiple_of(8) {
            bytes.push(self.read(count % 8)? as u8);
        }
        Some(bytes)
    }

    fn string(&mut self) -> Option<String> {
        let length = self.read(32)? as u32 as i32;
        if !(1..=1024).contains(&length) {
            return None;
        }
        let bytes = self.data(length as usize * 8)?;
        if bytes.last() != Some(&0) {
            return None;
        }
        String::from_utf8(bytes[..bytes.len() - 1].to_vec()).ok()
    }
}

#[derive(Clone)]
struct Bunch {
    channel: u32,
    sequence: Option<u16>,
    opened: bool,
    closed: bool,
    exports: bool,
    must_map: bool,
    partial: bool,
    initial: bool,
    final_part: bool,
    data: Vec<u8>,
    bits: usize,
}

#[derive(Default)]
struct Channel {
    latest: Option<(u16, u64)>,
    last: Option<u64>,
    pending: BTreeMap<u64, Bunch>,
    partial: Option<Assembly>,
    tracked: bool,
    record: Option<GameRecord>,
    last_packet: Option<u16>,
}

struct Assembly {
    data: Vec<u8>,
    bits: usize,
    opened: bool,
}

pub(super) struct Decoder {
    channels: HashMap<u32, Channel>,
    player_archetypes: HashSet<u32>,
    current: Option<GameRecord>,
}

impl Decoder {
    pub(super) fn new() -> Self {
        Self {
            channels: HashMap::new(),
            player_archetypes: HashSet::new(),
            current: None,
        }
    }

    pub(super) fn with_record(record: Option<GameRecord>) -> Self {
        Self {
            current: record,
            ..Self::new()
        }
    }

    pub(super) fn ingest(&mut self, payload: &[u8]) -> Vec<GameRecord> {
        let Some((packet_sequence, bunches)) = packet(payload) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        for bunch in bunches {
            if bunch.closed {
                self.channels.remove(&bunch.channel);
                continue;
            }
            if !self.channels.contains_key(&bunch.channel) && self.channels.len() >= 1024 {
                continue;
            }
            let channel = self.channels.entry(bunch.channel).or_default();
            if bunch.exports {
                let mut reader = Bits::new(&bunch.data, bunch.bits);
                if let Some(reader) = reader.as_mut() {
                    let _ = exports(reader, &mut self.player_archetypes);
                }
            }
            if let Some(sequence) = bunch.sequence {
                let absolute = if let Some((previous, index)) = channel.latest {
                    let distance = sequence.wrapping_sub(previous) & 1023;
                    if distance < 512 {
                        let next = index + u64::from(distance);
                        channel.latest = Some((sequence, next));
                        next
                    } else {
                        index.saturating_sub(u64::from(previous.wrapping_sub(sequence) & 1023))
                    }
                } else {
                    let index = 1024 + u64::from(sequence);
                    channel.latest = Some((sequence, index));
                    index
                };
                if channel.last.is_some_and(|last| absolute <= last)
                    && !(channel
                        .record
                        .as_ref()
                        .is_some_and(|record| record.role_id.is_empty())
                        && (bunch.opened || bunch.exports || bunch.partial))
                {
                    continue;
                }
                if !bunch.partial {
                    let pending = (!channel.tracked).then(|| bunch.clone());
                    if let Some(record) = consume(
                        channel,
                        bunch,
                        &self.player_archetypes,
                        self.current.as_ref(),
                    ) {
                        channel.last = Some(absolute);
                        records.push(record);
                    } else if let Some(pending) = pending {
                        channel.pending.entry(absolute).or_insert(pending);
                        if let Some((_, latest)) = channel.latest {
                            channel.pending.retain(|index, _| *index + 1024 >= latest);
                        }
                    }
                    continue;
                }
                channel.pending.entry(absolute).or_insert(bunch);
                let starts: Vec<_> = channel
                    .pending
                    .iter()
                    .filter(|(_, part)| part.initial)
                    .map(|(index, _)| *index)
                    .collect();
                for start in starts {
                    let mut end = None;
                    for step in 0..1024 {
                        let index = start + step;
                        let Some(part) = channel.pending.get(&index) else {
                            break;
                        };
                        if !part.partial || (step != 0 && part.initial) {
                            break;
                        }
                        if part.final_part {
                            end = Some(index);
                            break;
                        }
                    }
                    let Some(end) = end else { continue };
                    if channel.last.is_none_or(|last| end > last)
                        || channel
                            .record
                            .as_ref()
                            .is_some_and(|record| record.role_id.is_empty())
                    {
                        for index in start..=end {
                            let part = channel
                                .pending
                                .remove(&index)
                                .expect("complete fragment group");
                            let opening = channel
                                .partial
                                .as_ref()
                                .is_some_and(|assembly| assembly.opened)
                                || part.opened;
                            let final_part = part.final_part;
                            if let Some(record) = consume(
                                channel,
                                part,
                                &self.player_archetypes,
                                self.current.as_ref(),
                            ) {
                                channel.last = Some(channel.last.map_or(end, |last| last.max(end)));
                                records.push(record);
                            } else if opening && final_part && channel.tracked {
                                channel.last = Some(channel.last.map_or(end, |last| last.max(end)));
                            }
                        }
                    } else {
                        for index in start..=end {
                            channel.pending.remove(&index);
                        }
                    }
                }
                if channel.tracked {
                    let ready: Vec<_> = channel
                        .pending
                        .iter()
                        .filter(|(_, part)| !part.partial)
                        .map(|(index, _)| *index)
                        .collect();
                    for index in ready {
                        let part = channel
                            .pending
                            .remove(&index)
                            .expect("queued reliable message");
                        if channel.last.is_none_or(|last| index > last) {
                            if let Some(record) = consume(
                                channel,
                                part,
                                &self.player_archetypes,
                                self.current.as_ref(),
                            ) {
                                channel.last = Some(index);
                                records.push(record);
                            }
                        } else {
                            // 角色建档重组前到达的私有记录只补齐未知字段.
                            let previous = channel.record.clone();
                            if let Some(older) = consume(
                                channel,
                                part,
                                &self.player_archetypes,
                                self.current.as_ref(),
                            ) {
                                if let Some(mut current) = previous {
                                    current.fill_missing(&older);
                                    current.updated_at = older.updated_at;
                                    channel.record = Some(current.clone());
                                    records.push(current);
                                }
                            }
                        }
                    }
                }
                if let Some((_, latest)) = channel.latest {
                    channel.pending.retain(|index, _| *index + 1024 >= latest);
                }
            } else {
                if channel.last_packet.is_some_and(|last| {
                    let distance = packet_sequence.wrapping_sub(last) & 16383;
                    distance >= 8192
                }) {
                    continue;
                }
                channel.last_packet = Some(packet_sequence);
                if let Some(record) = consume(
                    channel,
                    bunch,
                    &self.player_archetypes,
                    self.current.as_ref(),
                ) {
                    records.push(record);
                }
            }
        }
        if let Some(record) = records.last() {
            self.current = Some(record.clone());
        }
        records
    }
}

fn packet(payload: &[u8]) -> Option<(u16, Vec<Bunch>)> {
    let last = *payload.last()?;
    if payload.len() < 12 || last == 0 {
        return None;
    }
    let outer = (payload.len() - 1) * 8 + (7 - last.leading_zeros() as usize);
    let end = outer.checked_sub(1)?;
    let mut reader = Bits::new(payload, outer + 1)?;
    reader.position = end;
    if reader.read(2)? != 3 {
        return None;
    }
    reader.position = 6;
    let header = reader.read(32)? as u32;
    let history = (header & 15) + 1;
    if history > 8 {
        return None;
    }
    reader.skip(history as usize * 32)?;
    if reader.read(1)? != 0 {
        reader.skip(10)?;
        if reader.read(1)? != 0 {
            reader.skip(8)?;
        }
    }
    reader.end = end;
    let mut bunches = Vec::new();
    while reader.position < end {
        if bunches.len() >= 256 {
            return None;
        }
        let control = reader.read(1)? != 0;
        let opened = control && reader.read(1)? != 0;
        let closed = control && reader.read(1)? != 0;
        if closed {
            let mut value = 0;
            for mask in [1, 2, 4, 8] {
                if value + mask < 15 && reader.read(1)? != 0 {
                    value += mask;
                }
            }
        }
        reader.skip(1)?;
        let reliable = reader.read(1)? != 0;
        let channel = reader.packed()?;
        if channel > 65535 {
            return None;
        }
        let exports = reader.read(1)? != 0;
        let must_map = reader.read(1)? != 0;
        let partial = reader.read(1)? != 0;
        let sequence = if reliable {
            Some(reader.read(10)? as u16)
        } else {
            None
        };
        let initial = partial && reader.read(1)? != 0;
        if partial {
            reader.skip(1)?;
        }
        let final_part = partial && reader.read(1)? != 0;
        let actor = if opened || reliable {
            if reader.read(1)? != 0 {
                reader.packed()? == 102
            } else {
                reader.string()?;
                reader.read(32)?;
                false
            }
        } else {
            true
        };
        let bits = reader.read(13)? as usize;
        let data = reader.data(bits)?;
        if !actor {
            continue;
        }
        bunches.push(Bunch {
            channel,
            sequence,
            opened,
            closed,
            exports,
            must_map,
            partial,
            initial,
            final_part,
            data,
            bits,
        });
    }
    Some(((header >> 18) as u16, bunches))
}

fn exports(reader: &mut Bits<'_>, archetypes: &mut HashSet<u32>) -> Option<()> {
    if reader.read(1)? != 0 {
        return None;
    }
    let count = reader.read(32)?;
    if count > 256 {
        return None;
    }
    for _ in 0..count {
        export_object(reader, archetypes, 0)?;
    }
    Some(())
}

fn export_object(
    reader: &mut Bits<'_>,
    archetypes: &mut HashSet<u32>,
    depth: usize,
) -> Option<u32> {
    if depth > 16 {
        return None;
    }
    let guid = reader.packed()?;
    if guid == 0 {
        return Some(0);
    }
    let flags = reader.read(8)?;
    if flags & 1 != 0 {
        export_object(reader, archetypes, depth + 1)?;
        let name = reader.string()?;
        if flags & 4 != 0 {
            reader.skip(32)?;
        }
        if name == "Default__BP_PlayerStateBase_C" {
            archetypes.insert(guid);
        }
    }
    Some(guid)
}

fn consume(
    channel: &mut Channel,
    bunch: Bunch,
    archetypes: &HashSet<u32>,
    current: Option<&GameRecord>,
) -> Option<GameRecord> {
    let mut opened = bunch.opened;
    let mut bytes = bunch.data;
    let mut bits = bunch.bits;
    if bunch.partial {
        if bunch.initial {
            channel.partial = Some(Assembly {
                data: Vec::new(),
                bits: 0,
                opened,
            });
        }
        let assembly = channel.partial.as_mut()?;
        if !bunch.exports {
            if !bunch.final_part && !bits.is_multiple_of(8) {
                channel.partial = None;
                return None;
            }
            if assembly.bits + bits > 8 * 1024 * 1024 {
                channel.partial = None;
                return None;
            }
            // 非末尾片段按字节拼接, export 数据不进入 Actor payload.
            assembly.data.extend_from_slice(&bytes);
            assembly.bits += bits;
        }
        if !bunch.final_part {
            return None;
        }
        let assembly = channel.partial.take()?;
        bytes = assembly.data;
        bits = assembly.bits;
        opened = assembly.opened;
    } else if bunch.exports {
        return None;
    }
    let mut reader = Bits::new(&bytes, bits)?;
    if bunch.must_map {
        let count = reader.read(16)?;
        if count > 256 {
            return None;
        }
        for _ in 0..count {
            reader.packed()?;
        }
    }
    if opened {
        let unidentified = channel
            .record
            .take()
            .filter(|record| record.role_id.is_empty());
        channel.tracked = false;
        reader.packed()?;
        let archetype = reader.packed()?;
        if !archetypes.contains(&archetype) {
            return None;
        }
        reader.packed()?;
        if reader.read(5)? != 16 || reader.read(2)? != 3 {
            return None;
        }
        let count = reader.packed()? as usize;
        let native = reader.data(count)?;
        let (uid, weekly) = initial(&native, count)?;
        channel.tracked = true;
        channel.record = Some(GameRecord {
            role_id: uid,
            weekly_remaining: Some(weekly),
            recovery: Some(super::Recovery {
                weekly_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|duration| duration.as_millis() as u64),
                ..Default::default()
            }),
            ..empty_record()
        });
        if let Some(mut observed) = unidentified {
            let identity = channel.record.as_ref().expect("角色身份已读取");
            observed.role_id.clone_from(&identity.role_id);
            observed.fill_missing(identity);
            channel.record = Some(observed.clone());
            return Some(observed);
        }
        return None;
    }
    let body_bits = reader.end - reader.position;
    let body = reader.data(body_bits)?;
    let mut changed = false;
    if !channel.tracked {
        let mut record = current.cloned().unwrap_or_else(empty_record);
        let observed = stamina(&body, body_bits, &mut record) | blocks(&body, &mut record);
        if !observed {
            return None;
        }
        channel.tracked = true;
        channel.record = Some(record);
        changed = true;
    }
    let record = channel.record.as_mut()?;
    if !changed {
        changed = stamina(&body, body_bits, record) | blocks(&body, record);
    }
    let observed = record.stamina.is_some()
        || record.city_stamina.is_some()
        || record.activity.is_some()
        || record.incentive_obtained.is_some();
    if !changed || !observed {
        return None;
    }
    record.updated_at = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(record.clone())
}

fn initial(bytes: &[u8], bits: usize) -> Option<(String, u32)> {
    let mut header = Bits::new(bytes, bits)?;
    if header.read(1)? != 0 || header.packed()? != 5 {
        return None;
    }
    let mut identities = Vec::new();
    for position in 9..bits.saturating_sub(80) {
        let mut reader = Bits::new(bytes, bits)?;
        reader.position = position;
        if reader.read(8)? != 60 {
            continue;
        }
        let value = reader.read(64)?;
        if value != 0 && reader.read(8)? == 76 {
            identities.push(value);
        }
    }
    #[cfg(debug_assertions)]
    eprintln!("游戏同步: 初始角色标识候选数量 {}.", identities.len());
    if identities.len() != 1 {
        return None;
    }
    Some((
        identities[0].to_string(),
        weekly(bytes, bits, true).unwrap_or(3),
    ))
}

fn weekly(bytes: &[u8], bits: usize, initial: bool) -> Option<u32> {
    let mut candidates = Vec::new();
    for position in 1..bits.saturating_sub(31) {
        let mut reader = Bits::new(bytes, bits)?;
        reader.position = position;
        if reader.read(8)? != 216 {
            continue;
        }
        let count = reader.read(16)? as usize;
        if count > 64 || (initial && count == 0) || (count == 0 && position != 1) {
            continue;
        }
        let mut types = HashMap::new();
        let mut values = HashMap::new();
        let mut valid = false;
        for _ in 0..count * 2 + 1 {
            let Some(handle) = reader.packed() else { break };
            if handle == 0 {
                valid = true;
                break;
            }
            if handle as usize > count * 2 {
                break;
            }
            let element = (handle - 1) / 2;
            if handle % 2 == 1 {
                let Some(value) = reader.read(5) else { break };
                types.insert(element, value);
            } else {
                let Some(value) = reader.read(32) else { break };
                values.insert(element, value);
            }
        }
        if !valid || types.len() != count || values.len() != count {
            continue;
        }
        let used = types
            .iter()
            .find_map(|(index, kind)| (*kind == 1).then(|| values.get(index).copied()).flatten())
            .unwrap_or(0);
        if used <= 3 {
            candidates.push(3 - used as u32);
        }
    }
    (candidates.len() == 1).then(|| candidates[0])
}

fn stamina(bytes: &[u8], bits: usize, record: &mut GameRecord) -> bool {
    let mut changed = false;
    for shift in 0..8 {
        let Some(mut reader) = Bits::new(bytes, bits.min(bytes.len() * 8)) else {
            continue;
        };
        if reader.skip(shift).is_none() {
            continue;
        }
        let size = (reader.end - shift) / 8;
        let Some(aligned) = reader.data(size * 8) else {
            continue;
        };
        // 资源变化通知携带两项 Int32: 变化前值和变化后值.
        for (name, size) in [
            (b"Stamina\0".as_slice(), 8usize),
            (b"CityStamina\0".as_slice(), 12),
        ] {
            for position in 9..aligned.len().saturating_sub(size + 18) {
                if !aligned[position..].starts_with(name)
                    || aligned[position - 9] != 15
                    || aligned[position - 8..position - 4] != ((size + 4) as u32).to_le_bytes()
                    || aligned[position - 4..position] != (size as u32).to_le_bytes()
                {
                    continue;
                }
                let values = &aligned[position + size..position + size + 18];
                if values[..5] != [6, 4, 0, 0, 0] || values[9..14] != [6, 4, 0, 0, 0] {
                    continue;
                }
                let current =
                    u32::from_le_bytes(values[14..18].try_into().expect("delta value boundary"));
                let maximum = if name == b"Stamina\0" {
                    record.stamina_max
                } else {
                    record.city_stamina_max
                };
                if let Some(maximum) = maximum {
                    changed |= apply_stamina(record, name, current, maximum);
                } else {
                    let recovery = record.recovery.get_or_insert_default();
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .map(|duration| duration.as_millis() as u64);
                    if name == b"Stamina\0" {
                        record.stamina = Some(current);
                        recovery.stamina_at = now;
                    } else {
                        record.city_stamina = Some(current);
                        recovery.city_at = now;
                    }
                    changed = true;
                }
            }
        }
        let marker = b"FCommonStaminaSaveData\0";
        for position in 4..aligned.len().saturating_sub(marker.len()) {
            if !aligned[position..].starts_with(marker) {
                continue;
            }
            let start = position + marker.len();
            let Some(head) = aligned.get(start..start + 12) else {
                continue;
            };
            let number = u32::from_le_bytes(head[0..4].try_into().expect("field size"));
            let length = u32::from_le_bytes(head[4..8].try_into().expect("field size")) as usize;
            let name_length =
                u32::from_le_bytes(head[8..12].try_into().expect("field size")) as usize;
            if number != 0 || !matches!(name_length, 8 | 12) || length != 53 + name_length {
                continue;
            }
            let Some(body) = aligned.get(start + 8..start + 8 + length) else {
                continue;
            };
            if body.last().copied().is_none_or(|value| value > 1) {
                continue;
            }
            let values = 4 + name_length;
            let current =
                u32::from_le_bytes(body[values..values + 4].try_into().expect("body size"));
            let maximum =
                u32::from_le_bytes(body[values + 4..values + 8].try_into().expect("body size"));
            let backup = u32::from_le_bytes(
                body[values + 12..values + 16]
                    .try_into()
                    .expect("body size"),
            );
            if apply_stamina(record, &body[4..values], current, maximum) {
                changed = true;
                match &body[4..values] {
                    b"Stamina\0" => record.stamina_backup = Some(backup),
                    b"CityStamina\0" => record.city_stamina_backup = Some(backup),
                    _ => {}
                }
                if &body[4..values] == b"Stamina\0" {
                    let recovery = record.recovery.get_or_insert_default();
                    let ticks = |offset| {
                        u64::from_le_bytes(
                            body[values + offset..values + offset + 8]
                                .try_into()
                                .expect("body size"),
                        )
                    };
                    recovery.stamina_at = ticks(16)
                        .checked_sub(621_355_968_000_000_000 + 288_000_000_000)
                        .map(|value| value / 10_000);
                    recovery.backup_at = ticks(24)
                        .checked_sub(621_355_968_000_000_000 + 288_000_000_000)
                        .map(|value| value / 10_000);
                    recovery.backup_progress_ms = Some(ticks(32) / 10_000);
                }
            }
        }
        let marker = b"CommonStaminaSaveDatas\0";
        for position in 4..aligned.len().saturating_sub(marker.len()) {
            if !aligned[position..].starts_with(marker) {
                continue;
            }
            let end = position + marker.len();
            let Some(head) = aligned.get(end..end + 8) else {
                continue;
            };
            if head[..4] != [0; 4] {
                continue;
            }
            let length = u32::from_le_bytes(head[4..8].try_into().expect("field size")) as usize;
            if !(16..=4096).contains(&length) {
                continue;
            }
            let Some(map) = aligned.get(end + 8..end + 8 + length) else {
                continue;
            };
            for phase in 0..8 {
                let Some(mut entries) = Bits::new(map, map.len() * 8) else {
                    continue;
                };
                if entries.skip(phase).is_none() {
                    continue;
                }
                let count = (entries.end - phase) / 8;
                let Some(view) = entries.data(count * 8) else {
                    continue;
                };
                for (name, name_length) in
                    [(b"Stamina".as_slice(), 8), (b"CityStamina".as_slice(), 12)]
                {
                    for at in 4..view.len().saturating_sub(name.len()) {
                        if !view[at..].starts_with(name)
                            || view[at - 4..at] != (name_length as u32).to_le_bytes()
                        {
                            continue;
                        }
                        let start = (at + name_length) * 8 + phase;
                        let mut values = Bits::new(map, map.len() * 8).expect("map boundary");
                        values.position = start;
                        let packed = (|| {
                            values.packed()?;
                            values.packed()?;
                            values.packed()?;
                            let current = values.read(32)?;
                            let maximum = values.read(32)?;
                            (maximum > 0 && maximum <= if name == b"Stamina" { 360 } else { 700 })
                                .then_some((current, maximum))
                        })();
                        let pair = packed.or_else(|| {
                            let mut pairs = Vec::new();
                            for header in (16..=64).step_by(8) {
                                let mut values =
                                    Bits::new(map, map.len() * 8).expect("map boundary");
                                values.position = start + header;
                                if let (Some(current), Some(maximum)) =
                                    (values.read(32), values.read(32))
                                {
                                    if maximum == if name == b"Stamina" { 240 } else { 700 } {
                                        pairs.push((current, maximum));
                                    }
                                }
                            }
                            (pairs.len() == 1).then(|| pairs[0])
                        });
                        let Some((current, maximum)) = pair else {
                            continue;
                        };
                        let mut key = name.to_vec();
                        key.push(0);
                        changed |= apply_stamina(record, &key, current as u32, maximum as u32);
                    }
                }
            }
        }
    }
    changed
}

fn apply_stamina(record: &mut GameRecord, name: &[u8], current: u32, maximum: u32) -> bool {
    if maximum == 0 {
        return false;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64);
    let projected = now.map(|now| record.projected(now));
    match name {
        b"Stamina\0" if maximum <= 360 => {
            if record.stamina != Some(current) {
                let was_full = projected.as_ref().is_some_and(|value| {
                    value
                        .stamina
                        .zip(value.stamina_max)
                        .is_some_and(|(value, maximum)| value >= maximum)
                });
                if let Some(projected) = &projected {
                    record.stamina_backup = projected.stamina_backup;
                }
                let recovery = record.recovery.get_or_insert_default();
                if let (Some(anchor), Some(now)) = (recovery.stamina_at, now) {
                    if let Some(backup_at) = recovery.backup_at {
                        let full_at = anchor
                            + u64::from(maximum.saturating_sub(record.stamina.unwrap_or(current)))
                                * 360_000;
                        if now >= full_at {
                            let progress = recovery.backup_progress_ms.unwrap_or(0)
                                + now.saturating_sub(full_at.max(backup_at));
                            recovery.backup_progress_ms = Some(progress % 1_080_000);
                            recovery.backup_at = Some(now);
                        }
                    }
                    recovery.stamina_at = Some(if was_full {
                        now
                    } else {
                        anchor + now.saturating_sub(anchor) / 360_000 * 360_000
                    });
                } else {
                    recovery.stamina_at = now;
                }
            }
            record.stamina = Some(current);
            record.stamina_max = Some(if maximum == 240 { 360 } else { maximum });
        }
        b"CityStamina\0" if maximum <= 700 => {
            if let Some(projected) = &projected {
                record.city_stamina_backup = projected.city_stamina_backup;
            }
            record.recovery.get_or_insert_default().city_at = now;
            record.city_stamina = Some(current);
            record.city_stamina_max = Some(maximum);
        }
        _ => return false,
    }
    true
}

fn empty_record() -> GameRecord {
    GameRecord {
        role_id: String::new(),
        role_name: String::new(),
        server_name: "国服".into(),
        stamina: None,
        stamina_max: Some(360),
        stamina_backup: None,
        city_stamina: None,
        city_stamina_max: None,
        city_stamina_backup: None,
        activity: None,
        weekly_remaining: None,
        incentive_obtained: None,
        incentive_maximum: None,
        incentive_period_days: None,
        incentive_remaining: None,
        updated_at: 0,
        recovery: None,
    }
}

fn timestamp() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

/// Actor 内容块: 复制数组清空消息更新周本, 其余块承载资源 RPC.
fn blocks(body: &[u8], record: &mut GameRecord) -> bool {
    let Some(mut blocks) = Bits::new(body, body.len() * 8) else {
        return false;
    };
    let now = timestamp();
    let mut changed = false;
    while blocks.position + 10 <= blocks.end {
        let Some(replicated) = blocks.read(1) else {
            break;
        };
        // 非 Actor 内容块按子对象解析, 资源读数只跟随已识别的 Actor 块.
        if blocks.read(1) != Some(1) {
            break;
        }
        let Some(length) = blocks.packed() else {
            break;
        };
        let Some(data) = blocks.data(length as usize) else {
            break;
        };
        if replicated != 0 {
            if let Some(remaining) = weekly(&data, length as usize, false) {
                record.weekly_remaining = Some(remaining);
                record.recovery.get_or_insert_default().weekly_at = now;
                changed = true;
            }
        } else if let Some(resources) = resources(&data, length as usize) {
            changed |= apply_resources(record, resources, now);
        }
    }
    changed
}

/// 一块中的资源读数; 任一处越界都整块丢弃, 半截 RPC 流不能当作读数.
fn resources(data: &[u8], bits: usize) -> Option<Resources> {
    let mut reader = Bits::new(data, bits)?;
    let mut resources = Resources::default();
    while reader.position < reader.end {
        let index = function_index(&mut reader)?;
        let size = reader.packed()? as usize;
        let payload = reader.data(size)?;
        match index {
            390 => {
                if let Some(activity) = activation_delta(&payload, size) {
                    resources.activity = Some(activity);
                }
            }
            397 => {
                if let Some(incentive) = settle_response(&payload, size) {
                    resources.incentive = Some(incentive);
                }
            }
            486 => {
                if let Some(activity) = activation_snapshot(&payload, size) {
                    resources.activity = Some(activity);
                }
            }
            _ => {}
        }
    }
    Some(resources)
}

#[derive(Default)]
struct Resources {
    activity: Option<u32>,
    incentive: Option<Incentive>,
}

struct Incentive {
    obtained: u64,
    maximum: u64,
    period_days: u32,
    remaining_ms: u64,
}

fn apply_resources(record: &mut GameRecord, resources: Resources, now: Option<u64>) -> bool {
    let mut changed = false;
    if let Some(activity) = resources.activity {
        record.activity = Some(activity);
        record.recovery.get_or_insert_default().activity_at = now;
        changed = true;
    }
    if let Some(incentive) = resources.incentive {
        record.incentive_obtained = Some(incentive.obtained);
        record.incentive_maximum = Some(incentive.maximum);
        record.incentive_period_days = Some(incentive.period_days);
        record.incentive_remaining = Some(incentive.remaining_ms);
        let recovery = record.recovery.get_or_insert_default();
        recovery.incentive_at = now;
        recovery.incentive_remaining_ms = Some(incentive.remaining_ms);
        changed = true;
    }
    changed
}

/// 函数索引按 ClassNetCache 上界自适应读取: 更高位已不可能落入有效范围时停止.
fn function_index(reader: &mut Bits<'_>) -> Option<u32> {
    let mut value = 0u32;
    let mut mask = 1u32;
    while value + mask < INDEX_MAXIMUM {
        value |= u32::from(reader.read(1)? != 0) * mask;
        mask <<= 1;
    }
    Some(value)
}

/// 活跃度变化通知携带变化前值和变化后值.
fn activation_delta(data: &[u8], bits: usize) -> Option<u32> {
    let mut reader = Bits::new(data, bits)?;
    let old = if reader.read(1)? != 0 {
        reader.read(32)? as u32
    } else {
        0
    };
    let new = if reader.read(1)? != 0 {
        reader.read(32)? as u32
    } else {
        0
    };
    (reader.position == bits && old <= ACTIVATION_MAXIMUM && new <= ACTIVATION_MAXIMUM)
        .then_some(new)
}

/// 属性数组中的具名 Activation: 名称长度, 名称, FName 编号, 数据长度, 4 字节值.
fn activation_snapshot(data: &[u8], bits: usize) -> Option<u32> {
    const NAME: &[u8] = b"Activation\0";
    let mut found = Vec::new();
    for shift in 0..8 {
        let mut reader = Bits::new(data, bits.min(data.len() * 8))?;
        if reader.skip(shift).is_none() {
            continue;
        }
        let size = (reader.end - shift) / 8;
        let Some(view) = reader.data(size * 8) else {
            continue;
        };
        let mut position = 4;
        while position + NAME.len() < view.len() {
            if view[position..].starts_with(NAME)
                && view[position - 4..position] == (NAME.len() as u32).to_le_bytes()
            {
                if let Some(value) = activation_value(&view[position + NAME.len()..]) {
                    found.push(value);
                }
            }
            position += 1;
        }
    }
    found.sort_unstable();
    found.dedup();
    (found.len() == 1).then(|| found[0])
}

fn activation_value(values: &[u8]) -> Option<u32> {
    let head: [u8; 12] = values.get(..12)?.try_into().ok()?;
    let number = u32::from_le_bytes(head[0..4].try_into().ok()?);
    let length = u32::from_le_bytes(head[4..8].try_into().ok()?);
    let value = u32::from_le_bytes(head[8..12].try_into().ok()?);
    (number == 0 && length == 4 && value <= ACTIVATION_MAXIMUM).then_some(value)
}

/// 大亨计划结算响应: 一阶数组承载剩余时长, 已开采值, 生效上限和周期天数.
fn settle_response(data: &[u8], bits: usize) -> Option<Incentive> {
    let mut reader = Bits::new(data, bits)?;
    if reader.read(1)? != 1 || reader.read(16)? != 4 {
        return None;
    }
    let mut values = Vec::new();
    for (kind, size) in [(26u64, 8usize), (7, 8), (7, 8), (6, 4)] {
        if reader.read(8)? != kind || reader.read(32)? != size as u64 {
            return None;
        }
        values.push(reader.read(size * 8)?);
    }
    reader.read(32)?;
    if reader.read(32)? != SETTLE_MESSAGE || reader.position != bits {
        return None;
    }
    let [remaining, obtained, maximum, period_days] = values[..] else {
        return None;
    };
    if maximum == 0 || obtained > maximum || !(1..=366).contains(&period_days) {
        return None;
    }
    Some(Incentive {
        obtained,
        maximum,
        period_days: period_days as u32,
        remaining_ms: remaining / TICKS_PER_MILLISECOND,
    })
}

#[cfg(test)]
mod tests;
