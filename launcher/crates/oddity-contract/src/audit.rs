//! 事件发射扫描: 契约声明与源码里的发射调用必须互为覆盖.
//!
//! 扫描 `launcher/crates/` 与 `launcher/packages/*/backend/` 下的 Rust 源码,
//! 跳过名为 `tests` 的目录. 注释和字符串里的文本不算发射; 发射调用只有一种
//! 形状: `.emit("<事件 id>", 载荷)`, 第一个参数必须是字面量, 否则无法核对.

use super::EVENTS;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 扫描 `launcher/` 下的发射调用, 校验事件声明与发射调用互为覆盖.
#[test]
fn declared_events_and_emission_calls_cover_each_other() {
    let launcher = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems = problems(&scan(&launcher));
    assert!(
        problems.is_empty(),
        "事件契约与发射调用不一致:\n{}\n声明源: crates/oddity-contract/src/lib.rs 的 EVENTS.",
        problems.join("\n")
    );
}

/// 契约声明与扫描结果的差异; 空表示两者互为覆盖.
fn problems(scan: &Scan) -> Vec<String> {
    let mut problems: Vec<String> = scan
        .unresolved
        .iter()
        .map(|location| format!("发射调用的第一个参数必须是字面量事件 id: {location}"))
        .collect();
    for (id, location) in &scan.emissions {
        if super::event(id).is_none() {
            problems.push(format!("事件未在契约里声明: {id} ({location})"));
        }
    }
    for event in EVENTS {
        if !scan.emissions.contains_key(event.id) {
            problems.push(format!("事件没有任何发射点: {}", event.id));
        }
    }
    problems
}

/// 扫描结果.
#[derive(Default)]
struct Scan {
    /// 事件 id 到首次出现的 `文件:行`.
    emissions: BTreeMap<String, String>,
    /// 第一个参数不是字面量的发射调用.
    unresolved: Vec<String>,
}

fn scan(launcher: &Path) -> Scan {
    let mut scan = Scan::default();
    collect(&launcher.join("crates"), launcher, &mut scan);
    for package in entries(&launcher.join("packages")) {
        let backend = package.join("backend");
        if backend.is_dir() {
            collect(&backend, launcher, &mut scan);
        }
    }
    scan
}

/// 按路径排序的目录项, 保证扫描结果和首次出现位置稳定.
fn entries(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", directory.display()))
        .map(|entry| entry.expect("读取目录项").path())
        .collect();
    paths.sort();
    paths
}

fn collect(directory: &Path, launcher: &Path, scan: &mut Scan) {
    for path in entries(directory) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if name == "tests" || name == "target" {
                continue;
            }
            collect(&path, launcher, scan);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
            let file = path
                .strip_prefix(launcher)
                .unwrap_or(&path)
                .display()
                .to_string();
            collect_source(&source, &file, scan);
        }
    }
}

fn collect_source(source: &str, file: &str, scan: &mut Scan) {
    let tokens = tokenize(source);
    for window in tokens.windows(4) {
        let [dot, emit, open, argument] = window else {
            continue;
        };
        if !(dot.is_punct('.') && emit.is_ident("emit") && open.is_punct('(')) {
            continue;
        }
        let location = format!("{file}:{}", line_of(source, argument.offset));
        match &argument.kind {
            Kind::Str(id) => {
                scan.emissions.entry(id.clone()).or_insert(location);
            }
            _ => scan.unresolved.push(location),
        }
    }
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

struct Token {
    kind: Kind,
    offset: usize,
}

enum Kind {
    Ident(String),
    Str(String),
    Punct(char),
}

impl Token {
    fn is_punct(&self, expected: char) -> bool {
        matches!(&self.kind, Kind::Punct(value) if *value == expected)
    }

    fn is_ident(&self, expected: &str) -> bool {
        matches!(&self.kind, Kind::Ident(value) if value == expected)
    }
}

/// 只保留定位发射调用所需的记号: 标识符、字符串与标点.
fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let rest = &source[index..];
        if rest.starts_with("//") {
            index = rest
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset + 1);
            continue;
        }
        if rest.starts_with("/*") {
            index = block_comment_end(bytes, index);
            continue;
        }
        if let Some((end, value)) = raw_string(source, index) {
            tokens.push(Token {
                kind: Kind::Str(value),
                offset: index,
            });
            index = end;
            continue;
        }
        if byte == b'"' {
            let (end, value) = string(source, index);
            tokens.push(Token {
                kind: Kind::Str(value),
                offset: index,
            });
            index = end;
            continue;
        }
        if byte == b'\'' {
            index = char_end(source, index);
            continue;
        }
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            tokens.push(Token {
                kind: Kind::Ident(source[start..index].to_string()),
                offset: start,
            });
            continue;
        }
        let character = rest.chars().next().expect("字符边界");
        tokens.push(Token {
            kind: Kind::Punct(character),
            offset: index,
        });
        index += character.len_utf8();
    }
    tokens
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    let mut depth = 1;
    while index + 1 < bytes.len() {
        if bytes[index] == b'/' && bytes[index + 1] == b'*' {
            depth += 1;
            index += 2;
        } else if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    bytes.len()
}

/// 原始字符串 `r"..."` / `r#"..."#` 及其字节前缀; 不是原始字符串时返回 `None`.
fn raw_string(source: &str, start: usize) -> Option<(usize, String)> {
    let bytes = source.as_bytes();
    let mut index = start;
    if matches!(bytes.get(index), Some(b'b' | b'c')) {
        index += 1;
    }
    if bytes.get(index) != Some(&b'r') {
        return None;
    }
    index += 1;
    let mut hashes = 0;
    while bytes.get(index) == Some(&b'#') {
        hashes += 1;
        index += 1;
    }
    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    let content = index + 1;
    index = content;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            let mut end = index + 1;
            let mut count = 0;
            while count < hashes && bytes.get(end) == Some(&b'#') {
                count += 1;
                end += 1;
            }
            if count == hashes {
                return Some((end, source[content..index].to_string()));
            }
        }
        index += 1;
    }
    Some((bytes.len(), source[content..].to_string()))
}

fn string(source: &str, start: usize) -> (usize, String) {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return (index + 1, source[start + 1..index].to_string()),
            _ => index += 1,
        }
    }
    (bytes.len(), String::new())
}

/// 跳过字符字面量; 生命周期标记只跳过引号本身.
fn char_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let index = start + 1;
    if index < bytes.len() && bytes[index] == b'\\' {
        let mut end = index + 2;
        while end < bytes.len() && bytes[end] != b'\'' {
            end += 1;
        }
        return (end + 1).min(bytes.len());
    }
    if let Some(character) = source[index..].chars().next() {
        let after = index + character.len_utf8();
        if after < bytes.len() && bytes[after] == b'\'' {
            return after + 1;
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(source: &str) -> Vec<String> {
        let mut scan = Scan::default();
        collect_source(source, "probe.rs", &mut scan);
        assert!(
            scan.unresolved.is_empty(),
            "探针里不该有无法核对的发射调用: {:?}",
            scan.unresolved
        );
        scan.emissions.into_keys().collect()
    }

    #[test]
    fn comments_and_strings_are_not_emissions() {
        let source = r#"
            // sink.emit("line-comment", ());
            /* sink.emit("block-comment", ()); */
            let text = ".emit(\"inside-string\", ())";
        "#;
        assert!(ids(source).is_empty());
    }

    #[test]
    fn emission_calls_are_read_from_code() {
        let source = "sink.emit(\"game-state\", status);
            let _ = other.emit(\"host-show\", ());";
        assert_eq!(ids(source), ["game-state", "host-show"]);
    }

    #[test]
    fn dynamic_event_ids_are_reported() {
        let mut scan = Scan::default();
        collect_source("sink.emit(event, ());", "probe.rs", &mut scan);
        assert!(scan.emissions.is_empty());
        assert_eq!(scan.unresolved, ["probe.rs:1"]);
    }

    #[test]
    fn raw_strings_chars_and_lifetimes_do_not_confuse_the_scanner() {
        let source = r##"
            let text = r#"sink.emit("raw-string", ())"#;
            let quote = '"';
            let life: &'static str = "x";
            let chinese = '中';
            sink.emit("game-state", ());
        "##;
        assert_eq!(ids(source), ["game-state"]);
    }

    #[test]
    fn the_first_location_wins() {
        let source = "a.emit(\"game-state\", ());\nb.emit(\"game-state\", ());";
        let mut scan = Scan::default();
        collect_source(source, "probe.rs", &mut scan);
        assert_eq!(scan.emissions["game-state"], "probe.rs:1");
    }

    #[test]
    fn unterminated_literals_and_nested_comments_are_skipped() {
        let source = "/* outer /* inner */ sink.emit(\"nested\", ()); */\n\
                      let text = \"sink.emit(\\\"string\\\", ())\";\n\
                      let raw = r#\"sink.emit(\"raw\", ())\"#;\n\
                      let bytes = b\"raw\";\n\
                      let tab = '\\t';\n\
                      let life: &'static str = \"x\";\n\
                      sink.emit(\"game-state\", ());";
        assert_eq!(ids(source), ["game-state"]);
    }

    #[test]
    fn unterminated_literals_end_at_end_of_file() {
        assert!(ids("let text = \"never closed").is_empty());
        assert!(ids("let text = r#\"never closed").is_empty());
        assert!(ids("/* never closed").is_empty());
    }

    #[test]
    fn problems_cover_both_directions() {
        let mut scan = Scan::default();
        scan.emissions
            .insert("not-an-event".into(), "probe.rs:1".into());
        scan.emissions
            .insert("game-state".into(), "probe.rs:2".into());
        let problems = problems(&scan);
        assert!(problems
            .iter()
            .any(|problem| problem.contains("事件未在契约里声明: not-an-event (probe.rs:1)")));
        assert!(problems
            .iter()
            .any(|problem| problem.contains("事件没有任何发射点: game-download-state")));
        assert!(!problems
            .iter()
            .any(|problem| problem.contains("game-state (")));
    }

    #[test]
    fn problems_report_dynamic_ids() {
        let mut scan = Scan::default();
        scan.unresolved.push("probe.rs:3".into());
        assert_eq!(
            problems(&scan)[0],
            "发射调用的第一个参数必须是字面量事件 id: probe.rs:3"
        );
    }
}
