//! 会话运行中的游戏崩溃现场扫描.
//!
//! 卡死或崩溃时 UE 会在 prefix 内写下 UECC-* 目录, 而游戏进程常驻在上报
//! 界面不退出. 现场目录的发现与文本读取复用日志导出侧的实现, 这里只负责
//! 基线比较与读出 CrashType / ErrorMessage.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use oddity_runtime::SessionHang;
use quick_xml::events::Event;

use crate::logs::{crash_sites, read_context_text, site_modified};

/// 两次扫描的间隔; 崩溃目录只在游戏异常之后出现, 不需要跟采样同频.
pub(crate) const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// 读到崩溃类型后继续补读错误说明的窗口; 该字段本身可能就没有记录.
pub(crate) const ENRICH_WINDOW: Duration = Duration::from_secs(6);

/// 现场目录里记录的崩溃上下文文件名.
const CONTEXT_NAME: &str = "CrashContext.runtime-xml";

/// 会话起点已经存在的现场; 基线之外的目录才属于本轮会话.
pub(crate) fn baseline(prefix: &Path) -> BTreeSet<PathBuf> {
    if prefix.as_os_str().is_empty() {
        return BTreeSet::new();
    }
    crash_sites(prefix).into_iter().collect()
}

/// 一次现场发现.
pub(crate) struct Hit {
    /// 现场目录; 上下文还没写出时用于补读.
    pub(crate) site: PathBuf,
    /// 给界面的摘要; 上下文读不到时字段为空串.
    pub(crate) hang: SessionHang,
}

/// 基线之外最新的一份现场; 没有新目录时返回 None.
pub(crate) fn scan(prefix: &Path, baseline: &BTreeSet<PathBuf>) -> Option<Hit> {
    // 没有 prefix 时不扫描, 相对路径会落到进程工作目录.
    if prefix.as_os_str().is_empty() {
        return None;
    }
    let site = crash_sites(prefix)
        .into_iter()
        .filter(|site| !baseline.contains(site))
        .max_by_key(|site| site_modified(site))?;
    let name = site.file_name()?.to_string_lossy().into_owned();
    let (crash_type, error_message) = context(&site);
    Some(Hit {
        site,
        hang: SessionHang {
            crash_type,
            error_message,
            site: name,
            detected_at: crate::now_millis(),
        },
    })
}

/// 现场目录里记录的 CrashType 与 ErrorMessage; 读不到时为空串.
pub(crate) fn context(site: &Path) -> (String, String) {
    let Some(text) = read_context_text(&site.join(CONTEXT_NAME)) else {
        return (String::new(), String::new());
    };
    let (crash_type, error_message) = properties(&text);
    (
        crash_type.unwrap_or_default(),
        error_message.unwrap_or_default(),
    )
}

/// 崩溃上下文里给用户看的取值.
#[derive(Clone, Copy)]
enum Field {
    CrashType,
    ErrorMessage,
}

impl Field {
    fn of(name: &str) -> Option<Self> {
        match name {
            "CrashType" => Some(Self::CrashType),
            "ErrorMessage" => Some(Self::ErrorMessage),
            _ => None,
        }
    }
}

/// 按标签名查找 CrashType 与 ErrorMessage; 键值可能在任意层级.
///
/// 元素结构不固定, 解析失败时保留已经读到的部分, 残缺内容不影响整体判断.
fn properties(xml: &str) -> (Option<String>, Option<String>) {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().allow_dangling_amp = true;
    // 文本按事件分段到达, 实体两侧的空白要留到收集结束再统一去掉.
    let mut stack: Vec<String> = Vec::new();
    // 正在收集的字段, 开始收集时的元素深度与已读到的文本.
    let mut capturing: Option<(Field, usize, String)> = None;
    let mut crash_type = None;
    let mut error_message = None;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(tag) => {
                stack.push(tag.local_name().into_inner().to_string());
                if capturing.is_none() {
                    if let Some(field) = stack.last().and_then(|name| Field::of(name)) {
                        capturing = Some((field, stack.len(), String::new()));
                    }
                }
            }
            Event::Text(text) => {
                if let Some((_, _, value)) = capturing.as_mut() {
                    value.push_str(&text);
                }
            }
            Event::CData(text) => {
                if let Some((_, _, value)) = capturing.as_mut() {
                    value.push_str(&text);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some((_, _, value)) = capturing.as_mut() {
                    match reference.resolve_char_ref() {
                        Ok(Some(character)) => value.push(character),
                        Ok(None) => {
                            if let Some(entity) =
                                quick_xml::escape::resolve_predefined_entity(&reference)
                            {
                                value.push_str(entity);
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
            Event::End(_) => {
                stack.pop();
                let closed = capturing
                    .as_ref()
                    .is_some_and(|(_, depth, _)| *depth == stack.len() + 1);
                if closed {
                    if let Some((field, _, text)) = capturing.take() {
                        let value = text.trim();
                        if !value.is_empty() {
                            match field {
                                Field::CrashType => crash_type.get_or_insert(value.to_string()),
                                Field::ErrorMessage => {
                                    error_message.get_or_insert(value.to_string())
                                }
                            };
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    (crash_type, error_message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{crash_context, crash_site, TempDir};
    use std::fs;
    use std::os::unix::fs::symlink;

    /// 在 prefix 里写一份带上下文的现场.
    fn site(prefix: &Path, name: &str, context: &str) -> PathBuf {
        let site = crash_site(prefix, "player", name);
        fs::write(site.join(CONTEXT_NAME), context).unwrap();
        site
    }

    #[test]
    fn properties_are_read_from_any_nesting_level() {
        let xml = crash_context("Hang", "watchdog &amp; timeout");
        assert_eq!(
            properties(&xml),
            (Some("Hang".into()), Some("watchdog & timeout".into()))
        );
        // 属性里的键值不参与匹配.
        let attributes =
            r#"<Context CrashType="Crash"><ErrorMessage> boom </ErrorMessage></Context>"#;
        assert_eq!(properties(attributes), (None, Some("boom".into())));
        let nested = "<Root><A><B><CrashType>Hang</CrashType></B></A></Root>";
        assert_eq!(properties(nested), (Some("Hang".into()), None));
    }

    #[test]
    fn broken_context_keeps_whatever_was_read_without_panicking() {
        assert_eq!(properties(""), (None, None));
        assert_eq!(properties("not xml at all"), (None, None));
        assert_eq!(
            properties("<Root><CrashType>Hang</CrashType><ErrorMessage>unclosed"),
            (Some("Hang".into()), None)
        );
    }

    /// UTF-16 写出的上下文同样能读出字段.
    #[test]
    fn utf16_context_is_normalized() {
        let dir = TempDir::new("hang-utf16");
        let prefix = dir.join("prefix");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in crash_context("Hang", "utf16 & timeout").encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let baseline = baseline(&prefix);
        let site = crash_site(&prefix, "player", "UECC-utf16_0000");
        fs::write(site.join(CONTEXT_NAME), bytes).unwrap();

        let found = scan(&prefix, &baseline).expect("现场应被发现");
        assert_eq!(found.hang.crash_type, "Hang");
        assert_eq!(found.hang.error_message, "utf16 & timeout");
    }

    #[test]
    fn unreadable_context_is_reported_without_details() {
        let dir = TempDir::new("hang-unreadable");
        let prefix = dir.join("prefix");
        // 符号链接的上下文不被跟随.
        let secret = dir.join("secret.txt");
        fs::write(&secret, crash_context("Hang", "leaked")).unwrap();
        let linked = crash_site(&prefix, "player", "UECC-link_0000");
        symlink(&secret, linked.join(CONTEXT_NAME)).unwrap();
        assert_eq!(context(&linked), (String::new(), String::new()));

        // 超过读取上限的上下文不解析.
        let huge = crash_site(&prefix, "player", "UECC-huge_0000");
        let padding = "x".repeat(crate::logs::MAX_TEXT_BYTES as usize + 1);
        fs::write(
            huge.join(CONTEXT_NAME),
            format!("<CrashType>Hang</CrashType>{padding}"),
        )
        .unwrap();
        assert_eq!(context(&huge), (String::new(), String::new()));
    }

    #[test]
    fn only_new_real_sites_are_scanned() {
        let dir = TempDir::new("hang-scan");
        let prefix = dir.join("prefix");
        site(&prefix, "UECC-old_0000", &crash_context("Hang", "old"));
        let baseline = baseline(&prefix);
        assert_eq!(baseline.len(), 1);
        assert!(scan(&prefix, &baseline).is_none());

        // 符号链接目录与非 UECC- 目录都不是现场.
        let outside = dir.join("outside/UECC-linked_0000");
        fs::create_dir_all(&outside).unwrap();
        let crashes = prefix.join("drive_c/users/player/AppData/Local/HT/Saved/Crashes");
        symlink(&outside, crashes.join("UECC-linked_0000")).unwrap();
        site(&prefix, "Other-0000", &crash_context("Hang", "not a site"));
        assert!(scan(&prefix, &baseline).is_none());

        site(
            &prefix,
            "UECC-new_0000",
            &crash_context("Crash", "gpu hang"),
        );
        let found = scan(&prefix, &baseline).expect("新现场应被发现");
        assert_eq!(found.hang.site, "UECC-new_0000");
        assert_eq!(found.hang.crash_type, "Crash");
        assert_eq!(found.hang.error_message, "gpu hang");
        assert!(found.hang.detected_at > 1_600_000_000_000);
    }

    #[test]
    fn site_without_a_context_is_still_a_site() {
        let dir = TempDir::new("hang-no-context");
        let prefix = dir.join("prefix");
        let baseline = baseline(&prefix);
        crash_site(&prefix, "player", "UECC-bare_0000");
        let found = scan(&prefix, &baseline).expect("目录出现即视为现场");
        assert_eq!(found.hang.site, "UECC-bare_0000");
        assert_eq!(found.hang.crash_type, "");
        assert_eq!(found.hang.error_message, "");
    }
}
