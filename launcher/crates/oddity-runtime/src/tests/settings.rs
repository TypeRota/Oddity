//! 设置 schema 的声明校验与默认值.

use crate::{
    GamePackage, PackageDescriptor, Registry, SettingsField, SettingsFieldKind, SettingsSchema,
};
use serde_json::{json, Map, Value};

const FIELDS: &[SettingsField] = &[
    SettingsField {
        key: "enabled",
        label: "启用",
        description: "",
        order: 2,
        kind: SettingsFieldKind::Boolean { default: true },
    },
    SettingsField {
        key: "label",
        label: "名称",
        description: "",
        order: 1,
        kind: SettingsFieldKind::Text {
            default: "旅行者",
            max_chars: 8,
        },
    },
    SettingsField {
        key: "limit",
        label: "数量",
        description: "",
        order: 3,
        kind: SettingsFieldKind::Integer {
            default: 5,
            min: 1,
            max: 10,
        },
    },
    SettingsField {
        key: "style",
        label: "样式",
        description: "",
        order: 4,
        kind: SettingsFieldKind::Select {
            default: "plain",
            options: &["plain", "formal"],
        },
    },
];

static SCHEMA: SettingsSchema = SettingsSchema {
    owner: "game:demo",
    version: 2,
    fields: FIELDS,
    validate: None,
    migrate: None,
};

struct DemoSettings;

impl GamePackage for DemoSettings {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "demo",
            name: "演示",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn settings(&self) -> &'static [SettingsSchema] {
        std::slice::from_ref(&SCHEMA)
    }
}

fn values() -> Map<String, Value> {
    SCHEMA.defaults()
}

#[test]
fn defaults_are_typed_and_ordered_by_declaration() {
    let values = values();
    assert_eq!(values["label"], json!("旅行者"));
    assert_eq!(values["enabled"], json!(true));
    assert_eq!(values["limit"], json!(5));
    assert_eq!(values["style"], json!("plain"));
    assert_eq!(
        SCHEMA
            .ordered_fields()
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>(),
        ["label", "enabled", "limit", "style"]
    );
    SCHEMA.validate_values(&values).expect("默认值应通过校验");
}

#[test]
fn field_types_and_ranges_are_checked() {
    let mut values = values();
    values.insert("label".into(), json!(7));
    assert!(SCHEMA
        .validate_values(&values)
        .unwrap_err()
        .contains("必须是字符串"));

    let mut values = self::values();
    values.insert("limit".into(), json!(11));
    assert!(SCHEMA
        .validate_values(&values)
        .unwrap_err()
        .contains("必须在 1 到 10 之间"));

    let mut values = self::values();
    values.insert("style".into(), json!("missing"));
    assert!(SCHEMA
        .validate_values(&values)
        .unwrap_err()
        .contains("必须是以下值之一"));

    let mut values = self::values();
    values.insert("unknown".into(), json!(true));
    assert!(SCHEMA
        .validate_values(&values)
        .unwrap_err()
        .contains("未知字段"));
}

#[test]
fn version_migration_and_missing_migrations_are_reported() {
    let mut values = Map::new();
    let error = SCHEMA.migrate_from(1, &mut values).unwrap_err();
    assert!(error.contains("缺少版本 1 到 2 的迁移"), "{error}");

    fn migrate(from: u32, values: &mut Map<String, Value>) -> Result<(), String> {
        if from != 1 {
            return Err("只支持从版本 1 迁移".into());
        }
        let old = values.remove("name").unwrap_or(Value::Null);
        values.insert("label".into(), old);
        Ok(())
    }
    static MIGRATING: SettingsSchema = SettingsSchema {
        owner: "game:demo",
        version: 2,
        fields: FIELDS,
        validate: None,
        migrate: Some(migrate),
    };
    let mut values: Map<String, Value> = [("name".to_string(), json!("旧名称"))]
        .into_iter()
        .collect();
    MIGRATING.migrate_from(1, &mut values).unwrap();
    assert_eq!(values["label"], json!("旧名称"));
}

#[test]
fn definition_issues_cover_keys_labels_defaults_and_options() {
    const BAD_FIELDS: &[SettingsField] = &[
        SettingsField {
            key: "Bad-Key",
            label: "",
            description: "",
            order: 1,
            kind: SettingsFieldKind::Integer {
                default: 9,
                min: 10,
                max: 1,
            },
        },
        SettingsField {
            key: "Bad-Key",
            label: "重复",
            description: "",
            order: 2,
            kind: SettingsFieldKind::Select {
                default: "missing",
                options: &["a", "a"],
            },
        },
    ];
    static BAD: SettingsSchema = SettingsSchema {
        owner: "demo",
        version: 0,
        fields: BAD_FIELDS,
        validate: None,
        migrate: None,
    };
    let issues = BAD.definition_issues().join("; ");
    assert!(
        issues.contains("owner demo 必须是 host 或 game:<游戏 id>"),
        "{issues}"
    );
    assert!(issues.contains("版本必须大于 0"), "{issues}");
    assert!(issues.contains("字段键 Bad-Key"), "{issues}");
    assert!(issues.contains("展示名为空"), "{issues}");
    assert!(issues.contains("最小值大于最大值"), "{issues}");
    assert!(issues.contains("默认值无效"), "{issues}");
    assert!(issues.contains("字段键重复"), "{issues}");
    assert!(issues.contains("选项重复"), "{issues}");
}

#[test]
fn registry_collects_package_schemas_and_checks_the_owner() {
    let mut registry = Registry::new();
    registry.install(&DemoSettings);
    assert_eq!(
        registry
            .settings_schemas()
            .map(|schema| schema.owner)
            .collect::<Vec<_>>(),
        ["game:demo"]
    );
    assert_eq!(
        registry
            .settings_schema("game:demo")
            .map(|schema| schema.version),
        Some(2)
    );
    assert_eq!(registry.settings_games(), ["demo"]);
    assert!(registry
        .validate(&[])
        .expect_err("演示包没有装配命令")
        .iter()
        .any(|issue| issue.to_string().contains("命令")));
}

#[test]
fn registry_rejects_a_schema_owned_by_another_game() {
    static OUTSIDE: SettingsSchema = SettingsSchema {
        owner: "game:other",
        version: 1,
        fields: FIELDS,
        validate: None,
        migrate: None,
    };
    struct WrongOwner;

    impl GamePackage for WrongOwner {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "demo",
                name: "演示",
            }
        }

        fn register(&self, _registry: &mut Registry) {}

        fn settings(&self) -> &'static [SettingsSchema] {
            std::slice::from_ref(&OUTSIDE)
        }
    }

    let mut registry = Registry::new();
    registry.install(&WrongOwner);
    let error = registry
        .validate(&[])
        .expect_err("跨包 owner 应装配失败")
        .into_iter()
        .map(|issue| issue.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        error.contains("包 demo 只能声明 owner game:demo"),
        "{error}"
    );
}
