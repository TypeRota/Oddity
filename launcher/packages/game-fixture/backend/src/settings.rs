//! fixture 包声明的设置 section: 默认值, 校验与版本迁移.

use oddity_runtime::{SettingsField, SettingsFieldKind, SettingsSchema};
use serde_json::{Map, Value};

/// 测试设置字段; 新增字段只改这里.
const FIELDS: &[SettingsField] = &[
    SettingsField {
        key: "nickname",
        label: "测试昵称",
        description: "用于验证文本字段与默认值.",
        order: 1,
        kind: SettingsFieldKind::Text {
            default: "旅行者",
            max_chars: 24,
        },
    },
    SettingsField {
        key: "showGreeting",
        label: "显示问候",
        description: "关闭后页面只显示标题.",
        order: 2,
        kind: SettingsFieldKind::Boolean { default: true },
    },
    SettingsField {
        key: "greetingStyle",
        label: "问候样式",
        description: "选择页面上问候语的写法.",
        order: 3,
        kind: SettingsFieldKind::Select {
            default: "plain",
            options: &["plain", "formal"],
        },
    },
    SettingsField {
        key: "maxRecords",
        label: "记录数量",
        description: "限制演示页展示的记录条数.",
        order: 4,
        kind: SettingsFieldKind::Integer {
            default: 5,
            min: 1,
            max: 20,
        },
    },
];

/// 跨字段校验: 昵称去掉空白后不能为空.
fn validate(values: &Map<String, Value>) -> Result<(), String> {
    let nickname = values
        .get("nickname")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if nickname.trim().is_empty() {
        return Err("测试昵称不能为空".into());
    }
    Ok(())
}

/// 版本 1 的字段名迁移为版本 2 的 nickname.
fn migrate(from: u32, values: &mut Map<String, Value>) -> Result<(), String> {
    if from != 1 {
        return Err(format!("不支持从版本 {from} 迁移"));
    }
    let Some(display_name) = values.remove("displayName") else {
        return Ok(());
    };
    values.insert("nickname".into(), display_name);
    Ok(())
}

pub(crate) static SCHEMAS: &[SettingsSchema] = &[SettingsSchema {
    owner: "game:fixture",
    version: 2,
    fields: FIELDS,
    validate: Some(validate),
    migrate: Some(migrate),
}];
