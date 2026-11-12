//! 包声明设置 section 的字段, 默认值, 类型校验与版本迁移.
//!
//! schema 是静态数据加两个可选函数; 宿主按 owner 保存字段值, 不解释业务规则.

use serde_json::{Map, Value};

/// 跨字段校验函数.
pub type SettingsValidate = fn(&Map<String, Value>) -> Result<(), String>;

/// 旧版本迁移函数; 第一个参数是落盘版本.
pub type SettingsMigrate = fn(u32, &mut Map<String, Value>) -> Result<(), String>;

/// 设置字段的取值类型与默认值.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFieldKind {
    Boolean {
        default: bool,
    },
    Integer {
        default: i64,
        min: i64,
        max: i64,
    },
    Text {
        default: &'static str,
        max_chars: usize,
    },
    Select {
        default: &'static str,
        options: &'static [&'static str],
    },
}

impl SettingsFieldKind {
    /// 字段未写入时使用的值.
    pub fn default_value(self) -> Value {
        match self {
            Self::Boolean { default } => Value::Bool(default),
            Self::Integer { default, .. } => Value::from(default),
            Self::Text { default, .. } | Self::Select { default, .. } => {
                Value::String(default.to_string())
            }
        }
    }

    /// 生成物里的字段类型标签.
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Boolean { .. } => "boolean",
            Self::Integer { .. } => "integer",
            Self::Text { .. } => "text",
            Self::Select { .. } => "select",
        }
    }

    /// 单个值是否符合类型与取值范围.
    pub fn validate(self, value: &Value) -> Result<(), String> {
        match self {
            Self::Boolean { .. } => {
                if value.is_boolean() {
                    Ok(())
                } else {
                    Err("必须是布尔值".into())
                }
            }
            Self::Integer { min, max, .. } => {
                let number = value.as_i64().ok_or_else(|| "必须是整数".to_string())?;
                if number < min || number > max {
                    return Err(format!("必须在 {min} 到 {max} 之间"));
                }
                Ok(())
            }
            Self::Text { max_chars, .. } => {
                let text = value.as_str().ok_or_else(|| "必须是字符串".to_string())?;
                if text.chars().count() > max_chars {
                    return Err(format!("不能超过 {max_chars} 个字符"));
                }
                Ok(())
            }
            Self::Select { options, .. } => {
                let text = value.as_str().ok_or_else(|| "必须是字符串".to_string())?;
                if options.contains(&text) {
                    Ok(())
                } else {
                    Err(format!("必须是以下值之一: {}", options.join(", ")))
                }
            }
        }
    }

    fn definition_issues(self, key: &str, issues: &mut Vec<String>) {
        match self {
            Self::Integer { min, max, .. } if min > max => {
                issues.push(format!("字段 {key} 的最小值大于最大值"));
            }
            Self::Text { max_chars: 0, .. } => {
                issues.push(format!("字段 {key} 的最大长度必须大于 0"));
            }
            Self::Select { options, .. } => {
                if options.is_empty() {
                    issues.push(format!("字段 {key} 没有可选项"));
                }
                let mut seen = std::collections::HashSet::new();
                for option in options {
                    if !seen.insert(*option) {
                        issues.push(format!("字段 {key} 的选项重复: {option}"));
                    }
                }
            }
            _ => {}
        }
    }
}

/// 设置 section 的一个字段.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsField {
    /// JSON 里的字段键.
    pub key: &'static str,
    /// 设置页展示名.
    pub label: &'static str,
    /// 设置页说明.
    pub description: &'static str,
    /// 字段排序; 同序按 key 排序.
    pub order: u16,
    pub kind: SettingsFieldKind,
}

impl SettingsField {
    /// 字段默认值.
    pub fn default_value(&self) -> Value {
        self.kind.default_value()
    }

    /// 单个值校验; 错误信息带字段键.
    pub fn validate(&self, value: &Value) -> Result<(), String> {
        self.kind
            .validate(value)
            .map_err(|reason| format!("设置字段 {} {reason}", self.key))
    }

    /// schema 声明本身的问题.
    pub fn definition_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !valid_key(self.key) {
            issues.push(format!(
                "字段键 {} 不符合小写开头的字母数字格式或长度超过 48",
                self.key
            ));
        }
        if self.label.trim().is_empty() {
            issues.push(format!("字段 {} 的展示名为空", self.key));
        }
        self.kind.definition_issues(self.key, &mut issues);
        if let Err(reason) = self.validate(&self.default_value()) {
            issues.push(format!("字段 {} 的默认值无效: {reason}", self.key));
        }
        issues
    }
}

/// 一个声明方提供的设置 section schema.
///
/// owner 与设置命令里的标识一致: `host` 或 `game:<游戏 id>`.
#[derive(Debug, Clone, Copy)]
pub struct SettingsSchema {
    pub owner: &'static str,
    /// 落盘数据的当前版本; 旧版本经 migrate 升级.
    pub version: u32,
    pub fields: &'static [SettingsField],
    /// 跨字段校验; 字段类型与单字段范围由宿主按 kind 校验.
    pub validate: Option<SettingsValidate>,
    /// 把 old_version 的数据就地升级到当前版本.
    pub migrate: Option<SettingsMigrate>,
}

impl SettingsSchema {
    /// 按 (order, key) 排序后的字段.
    pub fn ordered_fields(&self) -> Vec<&SettingsField> {
        let mut fields: Vec<&SettingsField> = self.fields.iter().collect();
        fields.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.key.cmp(right.key))
        });
        fields
    }

    /// 按字段键查找声明.
    pub fn field(&self, key: &str) -> Option<&SettingsField> {
        self.fields.iter().find(|field| field.key == key)
    }

    /// 全部字段的默认值.
    pub fn defaults(&self) -> Map<String, Value> {
        self.fields
            .iter()
            .map(|field| (field.key.to_string(), field.default_value()))
            .collect()
    }

    /// 补齐缺失字段; 返回是否发生改动.
    pub fn fill_defaults(&self, values: &mut Map<String, Value>) -> bool {
        let mut changed = false;
        for field in self.fields {
            if !values.contains_key(field.key) {
                values.insert(field.key.to_string(), field.default_value());
                changed = true;
            }
        }
        changed
    }

    /// 校验完整字段集合; 未知字段与缺失字段都拒绝.
    pub fn validate_values(&self, values: &Map<String, Value>) -> Result<(), String> {
        for key in values.keys() {
            if self.field(key).is_none() {
                return Err(format!("设置 section {} 包含未知字段: {key}", self.owner));
            }
        }
        for field in self.fields {
            let value = values
                .get(field.key)
                .ok_or_else(|| format!("设置字段 {} 缺少值", field.key))?;
            field.validate(value)?;
        }
        if let Some(validate) = self.validate {
            validate(values)?;
        }
        Ok(())
    }

    /// 把 from 版本的数据升级到当前版本.
    pub fn migrate_from(&self, from: u32, values: &mut Map<String, Value>) -> Result<(), String> {
        if from >= self.version {
            return Ok(());
        }
        let migrate = self.migrate.ok_or_else(|| {
            format!(
                "设置 section {} 缺少版本 {from} 到 {} 的迁移",
                self.owner, self.version
            )
        })?;
        migrate(from, values)
    }

    /// schema 声明本身的问题; 装配期发现后拒绝启动.
    pub fn definition_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !self.owner.starts_with("game:") && self.owner != "host" {
            issues.push(format!(
                "owner {} 必须是 host 或 game:<游戏 id>",
                self.owner
            ));
        }
        if self.version == 0 {
            issues.push("版本必须大于 0".into());
        }
        if self.fields.is_empty() {
            issues.push("没有声明字段".into());
        }
        let mut keys = std::collections::HashSet::new();
        for field in self.fields {
            if !keys.insert(field.key) {
                issues.push(format!("字段键重复: {}", field.key));
            }
            issues.extend(field.definition_issues());
        }
        if issues.is_empty() {
            if let Err(reason) = self.validate_values(&self.defaults()) {
                issues.push(format!("默认值未通过校验: {reason}"));
            }
        }
        issues
    }
}

/// 字段键: 小写字母开头, 其余为 ASCII 字母或数字.
fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= 48 && first.is_ascii_lowercase() && rest.iter().all(u8::is_ascii_alphanumeric)
}
