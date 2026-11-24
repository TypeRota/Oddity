use super::*;

#[derive(Default)]
struct Writer(Vec<bool>);

impl Writer {
    fn put(&mut self, value: u64, count: usize) {
        self.0
            .extend((0..count).map(|index| value >> index & 1 != 0));
    }

    fn packed(&mut self, mut value: u32) {
        loop {
            let byte = (value & 127) << 1;
            value >>= 7;
            self.put(u64::from(byte | u32::from(value != 0)), 8);
            if value == 0 {
                break;
            }
        }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.put(u64::from(*byte), 8);
        }
    }

    fn string(&mut self, value: &str) {
        self.put(value.len() as u64 + 1, 32);
        self.bytes(value.as_bytes());
        self.put(0, 8);
    }

    fn append(&mut self, value: &Writer) {
        self.0.extend(&value.0);
    }

    fn data(&self) -> Vec<u8> {
        let mut bytes = vec![0; self.0.len().div_ceil(8)];
        for (index, value) in self.0.iter().enumerate() {
            if *value {
                bytes[index / 8] |= 1 << (index % 8);
            }
        }
        bytes
    }
}

fn packet(
    sequence: u16,
    opened: bool,
    exports: bool,
    partial: (bool, bool, bool),
    must_map: bool,
    body: &Writer,
) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.put(0, 6);
    writer.put(u64::from(sequence) << 18, 32);
    writer.put(0, 32);
    // 同时覆盖有 packet-info 的动态 Bunch 起点.
    writer.put(1, 1);
    writer.put(12, 10);
    writer.put(1, 1);
    writer.put(4, 8);
    writer.put(u64::from(opened), 1);
    if opened {
        writer.put(1, 1);
        writer.put(0, 1);
    }
    writer.put(0, 1);
    writer.put(1, 1);
    writer.packed(4);
    writer.put(u64::from(exports), 1);
    writer.put(u64::from(must_map), 1);
    writer.put(u64::from(partial.0), 1);
    writer.put(u64::from(sequence), 10);
    if partial.0 {
        writer.put(u64::from(partial.1), 1);
        writer.put(0, 1);
        writer.put(u64::from(partial.2), 1);
    }
    writer.put(1, 1);
    writer.packed(102);
    writer.put(body.0.len() as u64, 13);
    writer.append(body);
    writer.put(3, 2);
    writer.data()
}

fn class_export(name: &str) -> Writer {
    let mut body = Writer::default();
    body.put(0, 1);
    body.put(1, 32);
    body.packed(17);
    body.put(1, 8);
    body.packed(0);
    body.string(name);
    body
}

fn weekly_value(used: Option<u32>) -> Writer {
    let mut body = Writer::default();
    body.put(0, 1);
    body.packed(108);
    body.put(u64::from(used.is_some()), 16);
    if let Some(used) = used {
        body.packed(1);
        body.put(1, 5);
        body.packed(2);
        body.put(u64::from(used), 32);
    }
    body.packed(0);
    body
}

fn native(used: Option<u32>) -> Writer {
    let mut native = Writer::default();
    native.put(0, 1);
    native.packed(5);
    native.put(0, 32);
    native.packed(30);
    native.put(900000001, 64);
    native.packed(38);
    native.put(0, 24);
    if let Some(used) = used {
        let entry = weekly_value(Some(used));
        native.0.extend(&entry.0[1..]);
        native.packed(110);
        native.put(0, 32);
    }
    native
}

fn actor_open(used: Option<u32>) -> Writer {
    let native = native(used);
    let mut body = Writer::default();
    body.put(1, 16);
    body.packed(17);
    body.packed(2);
    body.packed(17);
    body.packed(3);
    body.put(16, 5);
    body.put(3, 2);
    body.packed(native.0.len() as u32);
    body.append(&native);
    body
}

fn content(replicated: bool, value: &Writer) -> Writer {
    let mut body = Writer::default();
    body.put(u64::from(replicated), 1);
    body.put(1, 1);
    body.packed(value.0.len() as u32);
    body.append(value);
    body
}

fn stamina_data() -> Writer {
    let mut data = Writer::default();
    for (name, current, max) in [("Stamina", 129, 240), ("CityStamina", 190, 700)] {
        data.string("FCommonStaminaSaveData");
        data.put(0, 32);
        data.put(53 + name.len() as u64 + 1, 32);
        data.string(name);
        for value in [current, max, 0, if name == "Stamina" { 169 } else { 0 }] {
            data.put(value, 32);
        }
        for _ in 0..3 {
            data.put(0, 64);
        }
        data.put(0, 32);
        data.put(0, 32);
        data.put(0, 8);
    }
    data
}

#[test]
fn reliable_reordering_wrap_and_retransmission_preserve_role_and_resources() {
    let mut decoder = Decoder::new();
    let export = packet(
        1023,
        true,
        true,
        (true, true, false),
        false,
        &class_export("Default__BP_PlayerStateBase_C"),
    );
    let opened = packet(
        0,
        false,
        false,
        (true, false, true),
        true,
        &actor_open(Some(1)),
    );
    let resources = packet(
        1,
        false,
        false,
        (false, false, false),
        false,
        &content(false, &stamina_data()),
    );
    assert!(decoder.ingest(&opened).is_empty());
    let early = decoder.ingest(&resources);
    assert_eq!(early.len(), 1);
    assert!(early[0].role_id.is_empty());
    let snapshots = decoder.ingest(&export);
    assert_eq!(snapshots.len(), 1);
    let record = &snapshots[0];
    assert_eq!(record.role_id, "900000001");
    assert_eq!(record.stamina, Some(129));
    assert_eq!(record.stamina_max, Some(360));
    assert_eq!(record.city_stamina, Some(190));
    assert_eq!(record.city_stamina_max, Some(700));
    assert_eq!(record.stamina_backup, Some(169));
    assert_eq!(record.city_stamina_backup, Some(0));
    assert_eq!(record.weekly_remaining, Some(2));
    assert!(decoder.ingest(&export).is_empty());
    assert!(decoder.ingest(&opened).is_empty());
    assert!(decoder.ingest(&resources).is_empty());
    let update = packet(
        2,
        false,
        false,
        (false, false, false),
        false,
        &content(true, &weekly_value(Some(2))),
    );
    assert_eq!(decoder.ingest(&update)[0].weekly_remaining, Some(1));
    let clear = packet(
        3,
        false,
        false,
        (false, false, false),
        false,
        &content(true, &weekly_value(None)),
    );
    assert_eq!(decoder.ingest(&clear)[0].weekly_remaining, Some(3));
    let mut map = Writer::default();
    map.packed(112);
    map.put(2, 16);
    for (name, current, max, header) in [("Stamina", 360, 240, 300), ("CityStamina", 121, 700, 44)]
    {
        map.put(0, 1);
        map.string(name);
        if name == "Stamina" {
            map.bytes(&[0, 0, 125, 3]);
        } else {
            map.packed(0);
            map.packed(64);
            map.packed(header);
        }
        map.put(current, 32);
        map.put(max, 32);
    }
    let mut fields = Writer::default();
    fields.string("CommonStaminaSaveDatas");
    fields.put(0, 32);
    fields.put(map.data().len() as u64, 32);
    fields.bytes(&map.data());
    let refresh = packet(
        6,
        false,
        false,
        (false, false, false),
        false,
        &content(false, &fields),
    );
    let updates = decoder.ingest(&refresh);
    assert_eq!(updates[0].stamina, Some(360));
    assert_eq!(updates[0].stamina_max, Some(360));
    assert_eq!(updates[0].city_stamina, Some(121));
    assert_eq!(updates[0].stamina_backup, Some(169));
}

#[test]
fn unidentified_resource_messages_publish_values_without_inventing_a_role() {
    let mut decoder = Decoder::new();
    let updates = decoder.ingest(&packet(
        3,
        false,
        false,
        (false, false, false),
        false,
        &content(false, &stamina_data()),
    ));
    assert_eq!(updates.len(), 1);
    assert!(updates[0].role_id.is_empty());
    assert_eq!(updates[0].stamina, Some(129));
    assert_eq!(updates[0].city_stamina, Some(190));
    assert_eq!(updates[0].weekly_remaining, None);
}

#[test]
fn initial_empty_weekly_and_ambiguous_identity_are_distinguished() {
    let data = native(None);
    assert_eq!(
        initial(&data.data(), data.0.len()),
        Some(("900000001".into(), 3))
    );
    let mut duplicate = data;
    duplicate.packed(30);
    duplicate.put(900000002, 64);
    duplicate.packed(38);
    duplicate.put(0, 8);
    assert!(initial(&duplicate.data(), duplicate.0.len()).is_none());
    assert!(weekly(
        &weekly_value(Some(9)).data(),
        weekly_value(Some(9)).0.len(),
        false
    )
    .is_none());
}

#[test]
fn truncated_packets_cannot_publish_partial_values_or_panic() {
    let raw = packet(
        1,
        true,
        true,
        (true, true, false),
        false,
        &class_export("Default__BP_PlayerStateBase_C"),
    );
    for length in 0..raw.len() {
        let mut decoder = Decoder::new();
        assert!(decoder.ingest(&raw[..length]).is_empty());
    }
    let raw = native(Some(2)).data();
    for size in 0..raw.len() * 8 {
        let _ = initial(&raw, size);
    }
    let mut bits = Bits::new(&[1, 1, 1, 1, 0xff], 40).unwrap();
    assert!(bits.packed().is_none());
}

#[test]
fn resource_deltas_update_during_play_without_an_actor_login() {
    let record: GameRecord = serde_json::from_value(serde_json::json!({
        "roleId":"900000001", "roleName":"测试角色", "serverName":"国服",
        "stamina":129,"staminaMax":360,"cityStamina":424,"cityStaminaMax":700,
        "activity":null,"weeklyRemaining":2,"updatedAt":1
    }))
    .unwrap();
    let mut decoder = Decoder::with_record(Some(record));
    for (sequence, name, before, after) in [
        (1, "Stamina", 129, 69),
        (2, "Stamina", 69, 129),
        (3, "CityStamina", 424, 352),
        (4, "CityStamina", 352, 424),
    ] {
        let mut delta = Writer::default();
        delta.put(15, 8);
        delta.put(name.len() as u64 + 5, 32);
        delta.string(name);
        for value in [before, after] {
            delta.put(6, 8);
            delta.put(4, 32);
            delta.put(value, 32);
        }
        delta.put(3, 8);
        let updates = decoder.ingest(&packet(
            sequence,
            false,
            false,
            (false, false, false),
            false,
            &content(false, &delta),
        ));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].role_name, "测试角色");
        if name == "Stamina" {
            assert_eq!(updates[0].stamina, Some(after as u32));
        } else {
            assert_eq!(updates[0].city_stamina, Some(after as u32));
        }
        assert_eq!(updates[0].weekly_remaining, Some(2));
    }
    let mut properties = Writer::default();
    properties.put(0, 1);
    properties.packed(6);
    properties.put(100, 32);
    properties.0.extend(&weekly_value(Some(2)).0[1..]);
    let updates = decoder.ingest(&packet(
        5,
        false,
        false,
        (false, false, false),
        false,
        &content(true, &properties),
    ));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].weekly_remaining, Some(1));
    assert_eq!(updates[0].stamina, Some(129));
    assert_eq!(updates[0].city_stamina, Some(424));
}

#[test]
fn private_city_snapshot_arriving_before_role_assembly_is_not_lost() {
    let mut decoder = Decoder::new();
    let early = decoder.ingest(&packet(
        1,
        false,
        false,
        (false, false, false),
        false,
        &content(false, &stamina_data()),
    ));
    assert_eq!(early.len(), 1);
    assert!(early[0].role_id.is_empty());
    assert!(decoder
        .ingest(&packet(
            2,
            true,
            true,
            (true, true, false),
            false,
            &class_export("Default__BP_PlayerStateBase_C")
        ))
        .is_empty());
    let updates = decoder.ingest(&packet(
        3,
        false,
        false,
        (true, false, true),
        true,
        &actor_open(Some(1)),
    ));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].city_stamina, Some(190));
    assert_eq!(updates[0].city_stamina_max, Some(700));
    assert_eq!(updates[0].stamina, Some(129));
    assert_eq!(updates[0].weekly_remaining, Some(2));
}

#[test]
fn city_native_map_accepts_a_variable_record_header() {
    let mut map = Writer::default();
    map.packed(112);
    map.put(1, 16);
    map.put(0, 1);
    map.string("CityStamina");
    map.bytes(&[0, 0, 125, 3]);
    map.put(121, 32);
    map.put(700, 32);
    let mut fields = Writer::default();
    fields.string("CommonStaminaSaveDatas");
    fields.put(0, 32);
    fields.put(map.data().len() as u64, 32);
    fields.bytes(&map.data());
    let mut record: GameRecord = serde_json::from_value(serde_json::json!({
        "roleId":"900000001","roleName":"","serverName":"国服","stamina":null,"staminaMax":null,
        "cityStamina":null,"cityStaminaMax":null,"activity":null,"weeklyRemaining":null,"updatedAt":0
    })).unwrap();
    assert!(stamina(&fields.data(), fields.0.len(), &mut record));
    assert_eq!(record.city_stamina, Some(121));
    assert_eq!(record.city_stamina_max, Some(700));
}

#[test]
fn city_backup_full_records_publish_nonzero_and_zero_then_survive_stamina_only_updates() {
    let mut decoder = Decoder::new();
    for (sequence, backup) in [(1, 73), (2, 0)] {
        let mut data = Writer::default();
        data.string("FCommonStaminaSaveData");
        data.put(0, 32);
        data.put(65, 32);
        data.string("CityStamina");
        for value in [190, 700, 19978, backup] {
            data.put(value, 32);
        }
        for _ in 0..3 {
            data.put(0, 64);
        }
        data.put(0, 32);
        data.put(0, 32);
        data.put(0, 8);
        let records = decoder.ingest(&packet(
            sequence,
            false,
            false,
            (false, false, false),
            false,
            &content(false, &data),
        ));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].city_stamina_backup, Some(backup as u32));
    }
    let mut data = Writer::default();
    data.put(15, 8);
    data.put(12, 32);
    data.string("Stamina");
    for value in [100, 101] {
        data.put(6, 8);
        data.put(4, 32);
        data.put(value, 32);
    }
    data.put(0, 8);
    let records = decoder.ingest(&packet(
        3,
        false,
        false,
        (false, false, false),
        false,
        &content(false, &data),
    ));
    assert_eq!(records[0].stamina, Some(101));
    assert_eq!(records[0].city_stamina_backup, Some(0));
}

fn function_index(writer: &mut Writer, value: u32) {
    let mut current = 0u32;
    let mut mask = 1u32;
    while current + mask < 882 {
        let bit = u64::from(value & mask != 0);
        writer.put(bit, 1);
        current |= (bit as u32) * mask;
        mask <<= 1;
    }
}

fn commands(items: &[(u32, Writer)]) -> Writer {
    let mut stream = Writer::default();
    for (index, payload) in items {
        function_index(&mut stream, *index);
        stream.packed(payload.0.len() as u32);
        stream.append(payload);
    }
    stream
}

fn activation_snapshot(value: u32) -> Writer {
    let mut payload = Writer::default();
    // 属性数组按位打包, 名称前的标志位使名称不落在字节边界.
    payload.put(0, 4);
    payload.put(11, 32);
    payload.bytes(b"Activation\0");
    payload.put(0, 32);
    payload.put(4, 32);
    payload.put(u64::from(value), 32);
    payload
}

fn activation_delta(before: u32, after: u32) -> Writer {
    let mut payload = Writer::default();
    for value in [before, after] {
        payload.put(1, 1);
        payload.put(u64::from(value), 32);
    }
    payload
}

fn settle_response(remaining: u64, obtained: u64, maximum: u64, days: u32) -> Writer {
    let mut payload = Writer::default();
    payload.put(1, 1);
    payload.put(4, 16);
    for (kind, value) in [
        (26u64, remaining),
        (7, obtained),
        (7, maximum),
        (6, u64::from(days)),
    ] {
        let size = if kind == 6 { 4 } else { 8 };
        payload.put(kind, 8);
        payload.put(size, 32);
        payload.put(value, size as usize * 8);
    }
    payload.put(0, 32);
    payload.put(152, 32);
    payload
}

fn observed_record() -> GameRecord {
    serde_json::from_value(serde_json::json!({
        "roleId":"900000001", "roleName":"测试角色", "serverName":"国服",
        "stamina":129,"staminaMax":360,"cityStamina":424,"cityStaminaMax":700,
        "activity":null,"weeklyRemaining":2,"updatedAt":1
    }))
    .unwrap()
}

#[test]
fn activity_reads_the_named_property_and_change_notifications() {
    let mut decoder = Decoder::with_record(Some(observed_record()));
    let updates = decoder.ingest(&packet(
        1,
        false,
        false,
        (false, false, false),
        false,
        &content(
            false,
            &commands(&[
                (486, activation_snapshot(20)),
                (390, activation_delta(20, 40)),
            ]),
        ),
    ));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].activity, Some(40));
    assert!(updates[0]
        .recovery
        .as_ref()
        .and_then(|recovery| recovery.activity_at)
        .is_some());
    assert_eq!(updates[0].stamina, Some(129));

    let mut unknown = Decoder::new();
    let updates = unknown.ingest(&packet(
        1,
        false,
        false,
        (false, false, false),
        false,
        &content(false, &commands(&[(486, activation_snapshot(60))])),
    ));
    assert_eq!(updates.len(), 1);
    assert!(updates[0].role_id.is_empty());
    assert_eq!(updates[0].activity, Some(60));
}

#[test]
fn settle_responses_publish_the_incentive_amounts_and_period() {
    let mut decoder = Decoder::with_record(Some(observed_record()));
    let updates = decoder.ingest(&packet(
        2,
        false,
        false,
        (false, false, false),
        false,
        &content(
            false,
            &commands(&[(
                397,
                settle_response(11_495_160_490_000, 350_000, 1_150_000, 14),
            )]),
        ),
    ));
    assert_eq!(updates.len(), 1);
    let record = &updates[0];
    assert_eq!(record.incentive_obtained, Some(350_000));
    assert_eq!(record.incentive_maximum, Some(1_150_000));
    assert_eq!(record.incentive_period_days, Some(14));
    assert_eq!(record.incentive_remaining, Some(1_149_516_049));
    let recovery = record.recovery.as_ref().unwrap();
    assert_eq!(recovery.incentive_remaining_ms, Some(1_149_516_049));
    assert!(recovery.incentive_at.is_some());
}

#[test]
fn malformed_resource_commands_are_ignored_instead_of_publishing() {
    let over_cap = settle_response(1_000, 1_200_000, 1_150_000, 14);
    let mut wrong_message = settle_response(1_000, 0, 1_150_000, 14);
    // 消息编号不在末尾时整条响应作废.
    wrong_message.0.truncate(wrong_message.0.len() - 32);
    wrong_message.put(151, 32);
    let mut truncated = commands(&[(397, settle_response(1_000, 0, 1_150_000, 14))]);
    while truncated.0.len() > 40 {
        truncated.0.pop();
    }
    for stream in [over_cap, wrong_message, truncated] {
        let mut decoder = Decoder::with_record(Some(observed_record()));
        assert!(decoder
            .ingest(&packet(
                3,
                false,
                false,
                (false, false, false),
                false,
                &content(false, &stream),
            ))
            .is_empty());
    }
    let over_activity = activation_delta(20, 140);
    let mut decoder = Decoder::with_record(Some(observed_record()));
    assert!(decoder
        .ingest(&packet(
            4,
            false,
            false,
            (false, false, false),
            false,
            &content(false, &commands(&[(390, over_activity)])),
        ))
        .is_empty());
}
