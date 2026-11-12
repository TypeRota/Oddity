//! 极简 INI 解析: 容忍 UTF-8 BOM, CRLF, 注释与重复键.
//!
//! 只服务于启动器配置文件, 不追求通用 INI 规范.

use std::collections::BTreeMap;
use std::path::Path;

use crate::Result;

/// 只保留启动器配置需要的 INI 子集; 重复键以最后一次出现的值为准.
#[derive(Debug, Default, Clone)]
pub struct Ini {
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
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

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text))
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(String::as_str)
    }

    pub fn get_int(&self, section: &str, key: &str) -> Option<i64> {
        self.get(section, key)?.parse().ok()
    }

    pub fn section(&self, section: &str) -> Option<&BTreeMap<String, String>> {
        self.sections.get(section)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn ini_handles_bom_comments_duplicates_and_bad_lines() {
        let text = "\u{feff}top=1\r\n; 注释\r\n# 另一种注释\r\n[Main]\r\nKey = Value \r\nKey=Second\r\nempty=\r\nno_separator\r\n[Other]\r\nn=42\r\nnotnum=abc\r\n";
        let ini = Ini::parse(text);
        assert_eq!(ini.get("General", "top"), Some("1"));
        // 重复键取最后一次出现的值
        assert_eq!(ini.get("Main", "Key"), Some("Second"));
        assert_eq!(ini.get("Main", "empty"), Some(""));
        assert_eq!(ini.get("Main", "no_separator"), None);
        assert_eq!(ini.get_int("Other", "n"), Some(42));
        assert_eq!(ini.get_int("Other", "notnum"), None);
        assert_eq!(ini.get_int("Other", "missing"), None);
        assert_eq!(ini.get("Missing", "n"), None);
        assert_eq!(ini.section("Other").map(|section| section.len()), Some(2));
        assert!(ini.section("Nope").is_none());
    }

    #[test]
    fn ini_load_reads_files_and_reports_missing_ones() {
        let dir = TempDir::new("ini");
        let path = dir.write("Config/Config.ini", "[VERSION]\nVersion=1.2.3\n");
        let ini = Ini::load(&path).expect("load ini");
        assert_eq!(ini.get("VERSION", "Version"), Some("1.2.3"));
        assert!(Ini::load(&dir.join("nope.ini")).is_err());
    }
}
