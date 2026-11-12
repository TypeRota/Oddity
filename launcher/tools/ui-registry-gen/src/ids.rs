//! 读取包声明里的贡献 id, 校验命名空间, 前缀与唯一性.
//!
//! 生成器不执行包代码, 只扫描声明文件里的字面量 id. 页面, 首页卡片与设置
//! 分区必须带上包命名空间前缀; 游戏展示的 id 与预览的 namespace 必须与包
//! 命名空间一致. 声明里出现无法静态确定的贡献时直接失败, 避免新增贡献种类
//! 后漏检.

use std::fs;
use std::path::Path;

/// 宿主页面声明; 包贡献不得占用同名 id.
pub const HOST_PAGES: &str = "launcher/ui/src/host/pages.ts";

/// 一个包声明的全部 id.
#[derive(Debug)]
pub struct PackageIds {
    pub namespace: String,
    pub pages: Vec<String>,
    pub cards: Vec<String>,
    pub sections: Vec<String>,
    pub game: Option<String>,
    /// 预览声明的命名空间.
    pub preview: Option<String>,
}

impl PackageIds {
    /// 参与全局唯一性校验的贡献 id.
    pub fn contributions(&self) -> impl Iterator<Item = &String> {
        self.pages.iter().chain(&self.cards).chain(&self.sections)
    }
}

/// 读取并校验一个包的声明; 错误信息带上包目录与文件名.
pub fn read(dir: &str, src: &Path) -> Result<PackageIds, String> {
    let contribution = read_file(dir, src, "contribution.ts")?;
    let pages = read_file(dir, src, "pages.ts")?;
    let (namespace, cards, sections) = contribution_ids(dir, &contribution)?;
    let page_ids = page_ids(dir, &pages)?;
    let game = match optional_file(dir, src, "game.ts")? {
        Some(source) => Some(top_level_literal(dir, "game.ts", "game", "id", &source)?),
        None => None,
    };
    let preview = match optional_file(dir, src, "preview.ts")? {
        Some(source) => Some(top_level_literal(
            dir,
            "preview.ts",
            "preview",
            "namespace",
            &source,
        )?),
        None => None,
    };
    let ids = PackageIds {
        namespace,
        pages: page_ids,
        cards,
        sections,
        game,
        preview,
    };
    validate(dir, &ids)?;
    Ok(ids)
}

/// 宿主页面声明的 id; 与包贡献共用同一个唯一性集合.
pub fn host_page_ids(root: &Path) -> Result<Vec<String>, String> {
    let source = fs::read_to_string(root.join("ui/src/host/pages.ts"))
        .map_err(|error| format!("{HOST_PAGES}: 无法读取: {error}"))?;
    let (ids, _) = declaration_ids(HOST_PAGES, &source, "hostPageEntries")?;
    Ok(ids)
}

fn read_file(dir: &str, src: &Path, file: &str) -> Result<String, String> {
    fs::read_to_string(src.join(file))
        .map_err(|error| format!("{}: 无法读取: {error}", file_path(dir, file)))
}

fn optional_file(dir: &str, src: &Path, file: &str) -> Result<Option<String>, String> {
    if src.join(file).is_file() {
        read_file(dir, src, file).map(Some)
    } else {
        Ok(None)
    }
}

fn file_path(dir: &str, file: &str) -> String {
    format!("launcher/packages/{dir}/ui/src/{file}")
}

/// 页面声明的 id 必须是对象属性里的字符串字面量.
fn page_ids(dir: &str, source: &str) -> Result<Vec<String>, String> {
    let path = file_path(dir, "pages.ts");
    let (ids, _) = declaration_ids(&path, source, "pages")?;
    Ok(ids)
}

/// 读取 `export const <name> = [...]` 里每个元素的字面量 id.
fn declaration_ids(path: &str, source: &str, name: &str) -> Result<(Vec<String>, usize), String> {
    let tokens = declaration_tokens(path, source, name)?;
    if tokens.first() != Some(&Token::Punct('[')) {
        return Err(format!("{path}: {name} 必须是数组字面量"));
    }
    array_ids(path, &tokens, 0)
}

/// 从 contribution 声明里取出命名空间, 卡片与设置分区的 id.
fn contribution_ids(dir: &str, source: &str) -> Result<(String, Vec<String>, Vec<String>), String> {
    let path = file_path(dir, "contribution.ts");
    let tokens = declaration_tokens(&path, source, "contribution")?;
    if tokens.first() != Some(&Token::Punct('{')) {
        return Err(format!("{path}: 贡献声明必须是对象字面量"));
    }
    let mut namespace: Option<String> = None;
    let mut cards: Option<Vec<String>> = None;
    let mut sections: Option<Vec<String>> = None;
    let mut index = 1;
    loop {
        match tokens.get(index) {
            Some(Token::Punct('}')) => break,
            Some(Token::Punct(',')) => index += 1,
            Some(_) => {
                let key = property_name(&tokens, index)
                    .ok_or_else(|| format!("{path}: 无法识别的贡献声明"))?
                    .to_string();
                match key.as_str() {
                    "namespace" => {
                        let value = literal_value(&tokens, index)
                            .ok_or_else(|| format!("{path}: namespace 必须是字符串字面量"))?;
                        if namespace.replace(value).is_some() {
                            return Err(format!("{path}: 重复声明 namespace"));
                        }
                        index += 3;
                    }
                    "pageContributions" => {
                        if !maps_pages(&tokens, index) {
                            return Err(format!(
                                "{path}: pageContributions 必须由 pages 声明派生, 页面只能写在 pages.ts"
                            ));
                        }
                        index = skip_value(&path, &tokens, index + 2)?;
                    }
                    "homeCards" | "settingsSections" => {
                        if tokens.get(index + 2) != Some(&Token::Punct('[')) {
                            return Err(format!("{path}: {key} 必须是数组字面量"));
                        }
                        let (ids, next) = array_ids(&path, &tokens, index + 2)?;
                        let slot = if key == "homeCards" {
                            &mut cards
                        } else {
                            &mut sections
                        };
                        if slot.replace(ids).is_some() {
                            return Err(format!("{path}: 重复声明 {key}"));
                        }
                        index = next;
                    }
                    other => {
                        return Err(format!(
                            "{path}: 未知的贡献键 \"{other}\", 新增贡献种类时必须同步生成器"
                        ));
                    }
                }
            }
            None => return Err(format!("{path}: 贡献对象没有闭合")),
        }
    }
    let namespace = namespace.ok_or_else(|| format!("{path}: 缺少 namespace 声明"))?;
    Ok((
        namespace,
        cards.unwrap_or_default(),
        sections.unwrap_or_default(),
    ))
}

/// 游戏展示 (`id`) 与预览 (`namespace`) 声明的顶层标识.
fn top_level_literal(
    dir: &str,
    file: &str,
    name: &str,
    key: &str,
    source: &str,
) -> Result<String, String> {
    let path = file_path(dir, file);
    let tokens = declaration_tokens(&path, source, name)?;
    if tokens.first() != Some(&Token::Punct('{')) {
        return Err(format!("{path}: {name} 必须是对象字面量"));
    }
    let mut literal: Option<String> = None;
    let mut index = 1;
    loop {
        match tokens.get(index) {
            Some(Token::Punct('}')) => break,
            Some(Token::Punct(',')) => index += 1,
            Some(_) => {
                let property = property_name(&tokens, index)
                    .ok_or_else(|| format!("{path}: 无法识别的 {name} 声明"))?;
                if property == key {
                    let value = literal_value(&tokens, index)
                        .ok_or_else(|| format!("{path}: {key} 必须是字符串字面量"))?;
                    if literal.replace(value).is_some() {
                        return Err(format!("{path}: 重复声明 {key}"));
                    }
                }
                index = skip_value(&path, &tokens, index + 2)?;
            }
            None => return Err(format!("{path}: {name} 对象没有闭合")),
        }
    }
    literal.ok_or_else(|| format!("{path}: 缺少 {key} 声明"))
}

/// 数组元素的直接 id; 元素必须是对象字面量, 嵌套对象与设置项里的 id 不计入.
fn array_ids(path: &str, tokens: &[Token], start: usize) -> Result<(Vec<String>, usize), String> {
    if tokens.get(start) != Some(&Token::Punct('[')) {
        return Err(format!("{path}: 需要数组字面量"));
    }
    let mut ids = Vec::new();
    let mut index = start + 1;
    loop {
        match tokens.get(index) {
            Some(Token::Punct(']')) => return Ok((ids, index + 1)),
            Some(Token::Punct(',')) => index += 1,
            Some(Token::Punct('{')) => {
                let (id, next) = element_id(path, tokens, index, ids.len() + 1)?;
                ids.push(id);
                index = next;
            }
            Some(_) => {
                return Err(format!(
                    "{path}: 数组第 {} 个元素必须是带 id 的对象字面量",
                    ids.len() + 1
                ));
            }
            None => return Err(format!("{path}: 数组没有闭合")),
        }
    }
}

/// 数组元素对象的直接 id; 展开语法会让 id 无法静态确定, 直接失败.
fn element_id(
    path: &str,
    tokens: &[Token],
    start: usize,
    position: usize,
) -> Result<(String, usize), String> {
    let mut id: Option<String> = None;
    let mut index = start + 1;
    loop {
        match tokens.get(index) {
            Some(Token::Punct('}')) => {
                let id = id.ok_or_else(|| format!("{path}: 数组第 {position} 个元素缺少 id"))?;
                return Ok((id, index + 1));
            }
            Some(Token::Punct(',')) => index += 1,
            Some(Token::Punct('.')) if tokens.get(index + 1) == Some(&Token::Punct('.')) => {
                return Err(format!("{path}: 数组第 {position} 个元素不能使用展开语法"));
            }
            Some(_) => {
                let key = property_name(tokens, index)
                    .ok_or_else(|| format!("{path}: 数组第 {position} 个元素无法解析"))?;
                if key == "id" {
                    let value = literal_value(tokens, index).ok_or_else(|| {
                        format!("{path}: 数组第 {position} 个元素的 id 必须是字符串字面量")
                    })?;
                    if id.replace(value).is_some() {
                        return Err(format!("{path}: 数组第 {position} 个元素重复声明 id"));
                    }
                }
                index = skip_value(path, tokens, index + 2)?;
            }
            None => return Err(format!("{path}: 数组第 {position} 个元素没有闭合")),
        }
    }
}

/// 属性名: 标识符或字符串字面量, 且紧跟冒号.
fn property_name(tokens: &[Token], index: usize) -> Option<&str> {
    if tokens.get(index + 1) != Some(&Token::Punct(':')) {
        return None;
    }
    match tokens.get(index) {
        Some(Token::Ident(name)) | Some(Token::Str(name)) => Some(name),
        _ => None,
    }
}

/// 属性的字符串字面量值.
fn literal_value(tokens: &[Token], index: usize) -> Option<String> {
    if tokens.get(index + 1) != Some(&Token::Punct(':')) {
        return None;
    }
    match tokens.get(index + 2) {
        Some(Token::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

/// `pageContributions: pages.map(...)`: 页面条目只能来自 pages 声明.
fn maps_pages(tokens: &[Token], index: usize) -> bool {
    matches!(tokens.get(index + 2), Some(Token::Ident(name)) if name == "pages")
        && matches!(tokens.get(index + 3), Some(Token::Punct('.')))
        && matches!(tokens.get(index + 4), Some(Token::Ident(name)) if name == "map")
        && matches!(tokens.get(index + 5), Some(Token::Punct('(')))
}

/// 跳过一个属性值, 停在同层的逗号或外层对象的右括号.
fn skip_value(path: &str, tokens: &[Token], start: usize) -> Result<usize, String> {
    let mut depth = 0i32;
    let mut index = start;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Punct('{') | Token::Punct('[') | Token::Punct('(') => depth += 1,
            Token::Punct('}') | Token::Punct(']') | Token::Punct(')') => {
                if depth == 0 {
                    return Ok(index);
                }
                depth -= 1;
            }
            Token::Punct(',') if depth == 0 => return Ok(index),
            _ => {}
        }
        index += 1;
    }
    Err(format!("{path}: 声明没有闭合"))
}

/// 找到 `export const <name> =` 并把等号后面的源码交给词法扫描.
fn declaration_tokens(path: &str, source: &str, name: &str) -> Result<Vec<Token>, String> {
    let start = declaration_value_start(source, name)
        .ok_or_else(|| format!("{path}: 找不到 export const {name} 声明"))?;
    lex(&source[start..]).map_err(|message| format!("{path}: {message}"))
}

fn declaration_value_start(source: &str, name: &str) -> Option<usize> {
    let needle = "export const ";
    let mut offset = 0;
    while let Some(relative) = source[offset..].find(needle) {
        let after_keyword = offset + relative + needle.len();
        let rest = &source[after_keyword..];
        let ident_len = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$')
            .count();
        if ident_len == name.len() && &rest[..ident_len] == name {
            let tail = &rest[ident_len..];
            let trimmed = tail.trim_start();
            if trimmed
                .strip_prefix('=')
                .is_some_and(|value| !value.starts_with('='))
            {
                return Some(after_keyword + ident_len + tail.len() - trimmed.len() + 1);
            }
        }
        offset = after_keyword;
    }
    None
}

fn validate(dir: &str, ids: &PackageIds) -> Result<(), String> {
    if !is_kebab(&ids.namespace) {
        return Err(format!(
            "{}: 包命名空间 \"{}\" 只能包含小写字母, 数字与连字符, 且以字母开头",
            file_path(dir, "contribution.ts"),
            ids.namespace
        ));
    }
    for (kind, file, values) in [
        ("页面", "pages.ts", &ids.pages),
        ("首页卡片", "contribution.ts", &ids.cards),
        ("设置分区", "contribution.ts", &ids.sections),
    ] {
        for id in values {
            if !is_kebab(id) {
                return Err(format!(
                    "{}: {kind} id \"{}\" 只能包含小写字母, 数字与连字符, 且以字母开头",
                    file_path(dir, file),
                    id
                ));
            }
            let prefixed = id
                .strip_prefix(&ids.namespace)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|name| !name.is_empty());
            if !prefixed {
                return Err(format!(
                    "{}: {kind} id \"{}\" 必须以包命名空间 \"{}\" 为前缀",
                    file_path(dir, file),
                    id,
                    ids.namespace
                ));
            }
        }
    }
    if let Some(game) = &ids.game {
        if game != &ids.namespace {
            return Err(format!(
                "{}: 游戏展示 id \"{}\" 必须与包命名空间 \"{}\" 一致",
                file_path(dir, "game.ts"),
                game,
                ids.namespace
            ));
        }
    }
    if let Some(preview) = &ids.preview {
        if preview != &ids.namespace {
            return Err(format!(
                "{}: 预览命名空间 \"{}\" 必须与包命名空间 \"{}\" 一致",
                file_path(dir, "preview.ts"),
                preview,
                ids.namespace
            ));
        }
    }
    Ok(())
}

fn is_kebab(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_lowercase()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Str(String),
    Number(String),
    Punct(char),
}

fn lex(source: &str) -> Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if byte == b'/' {
            match bytes.get(index + 1) {
                Some(b'/') => {
                    index += 2;
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                    continue;
                }
                Some(b'*') => {
                    index += 2;
                    while index + 1 < bytes.len()
                        && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                    {
                        index += 1;
                    }
                    if index + 1 >= bytes.len() {
                        return Err("块注释没有闭合".into());
                    }
                    index += 2;
                    continue;
                }
                _ => {}
            }
        }
        if byte == b'"' || byte == b'\'' {
            let (value, next) = string_literal(source, index, byte)?;
            tokens.push(Token::Str(value));
            index = next;
        } else if byte == b'`' {
            let (value, next) = template_literal(source, index)?;
            tokens.push(Token::Str(value));
            index = next;
        } else if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric()
                    || bytes[index] == b'_'
                    || bytes[index] == b'$')
            {
                index += 1;
            }
            tokens.push(Token::Ident(source[start..index].to_string()));
        } else if byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric()
                    || bytes[index] == b'.'
                    || bytes[index] == b'_')
            {
                index += 1;
            }
            tokens.push(Token::Number(source[start..index].to_string()));
        } else {
            tokens.push(Token::Punct(byte as char));
            index += 1;
        }
    }
    Ok(tokens)
}

fn string_literal(source: &str, start: usize, quote: u8) -> Result<(String, usize), String> {
    let bytes = source.as_bytes();
    let mut value = String::new();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                let escaped = bytes.get(index + 1).ok_or("字符串没有闭合")?;
                value.push(*escaped as char);
                index += 2;
            }
            byte if byte == quote => return Ok((value, index + 1)),
            b'\n' => return Err("字符串没有闭合".into()),
            byte => {
                value.push(byte as char);
                index += 1;
            }
        }
    }
    Err("字符串没有闭合".into())
}

fn template_literal(source: &str, start: usize) -> Result<(String, usize), String> {
    let bytes = source.as_bytes();
    let mut value = String::new();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                let escaped = bytes.get(index + 1).ok_or("模板字符串没有闭合")?;
                value.push(*escaped as char);
                index += 2;
            }
            b'`' => return Ok((value, index + 1)),
            byte => {
                value.push(byte as char);
                index += 1;
            }
        }
    }
    Err("模板字符串没有闭合".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ui-registry-ids-{name}-{}", std::process::id()))
    }

    fn write(dir: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let root = root(dir);
        let src = root.join("packages").join(dir).join("ui/src");
        fs::create_dir_all(&src).expect("创建包目录");
        for (file, content) in files {
            fs::write(src.join(file), content).expect("写入声明");
        }
        root
    }

    const CONTRIBUTION: &str = r#"export const contribution = {
  namespace: "nte",
  pageContributions: pages.map((page) => ({ ...page })),
  homeCards: [{ id: "nte-card", order: 1 }],
  settingsSections: [
    {
      id: "nte-game",
      entries: [{ id: "distribution" }],
    },
  ],
} as const;
"#;

    #[test]
    fn reads_namespaced_ids_and_ignores_nested_entries() {
        let root = write(
            "ids-ok",
            &[
                (
                    "pages.ts",
                    "export const pages = [{ id: \"nte-update\" }];\n",
                ),
                ("contribution.ts", CONTRIBUTION),
                ("game.ts", "export const game = { id: \"nte\" };\n"),
                (
                    "preview.ts",
                    "export const preview = { namespace: \"nte\" };\n",
                ),
            ],
        );
        let ids = read("ids-ok", &root.join("packages/ids-ok/ui/src")).expect("读取 id");
        assert_eq!(ids.namespace, "nte");
        assert_eq!(ids.pages, vec!["nte-update".to_string()]);
        assert_eq!(ids.cards, vec!["nte-card".to_string()]);
        assert_eq!(ids.sections, vec!["nte-game".to_string()]);
        assert_eq!(ids.game.as_deref(), Some("nte"));
        assert_eq!(ids.preview.as_deref(), Some("nte"));
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn reads_ids_written_as_string_keys() {
        let root = write(
            "ids-string-keys",
            &[
                (
                    "pages.ts",
                    "export const pages = [{ \"id\": \"nte-update\" }];\n",
                ),
                (
                    "contribution.ts",
                    "export const contribution = {\n  \"namespace\": \"nte\",\n  \"pageContributions\": pages.map((page) => ({ ...page })),\n  \"homeCards\": [{ \"id\": \"nte-card\" }],\n  \"settingsSections\": [{ \"id\": \"nte-game\" }],\n} as const;\n",
                ),
                ("game.ts", "export const game = { \"id\": \"nte\" };\n"),
                (
                    "preview.ts",
                    "export const preview = { \"namespace\": \"nte\" };\n",
                ),
            ],
        );
        let ids = read(
            "ids-string-keys",
            &root.join("packages/ids-string-keys/ui/src"),
        )
        .expect("读取字符串键");
        assert_eq!(ids.namespace, "nte");
        assert_eq!(ids.pages, vec!["nte-update".to_string()]);
        assert_eq!(ids.cards, vec!["nte-card".to_string()]);
        assert_eq!(ids.sections, vec!["nte-game".to_string()]);
        assert_eq!(ids.game.as_deref(), Some("nte"));
        assert_eq!(ids.preview.as_deref(), Some("nte"));
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn rejects_array_elements_without_literal_ids() {
        for (dir, pages, message) in [
            (
                "ids-no-id",
                "export const pages = [{ slot: \"drawer\" }];\n",
                "缺少 id",
            ),
            (
                "ids-spread",
                "export const pages = [...sharedPages];\n",
                "对象字面量",
            ),
            (
                "ids-imported",
                "export const pages = [sharedPage];\n",
                "对象字面量",
            ),
            (
                "ids-spread-object",
                "export const pages = [{ ...sharedPage }];\n",
                "展开语法",
            ),
        ] {
            let root = write(
                dir,
                &[("pages.ts", pages), ("contribution.ts", CONTRIBUTION)],
            );
            let error = read(dir, &root.join(format!("packages/{dir}/ui/src")))
                .expect_err("无法确定 id 的数组元素必须失败");
            assert!(error.contains(message), "{error}");
            fs::remove_dir_all(root).expect("清理临时目录");
        }
    }

    #[test]
    fn rejects_unknown_contribution_keys() {
        let root = write(
            "ids-unknown-key",
            &[
                (
                    "pages.ts",
                    "export const pages = [{ id: \"nte-update\" }];\n",
                ),
                (
                    "contribution.ts",
                    "export const contribution = { namespace: \"nte\", footers: [] };\n",
                ),
            ],
        );
        let error = read(
            "ids-unknown-key",
            &root.join("packages/ids-unknown-key/ui/src"),
        )
        .expect_err("未知贡献键必须失败");
        assert!(error.contains("未知的贡献键"), "{error}");
        assert!(error.contains("footers"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn rejects_page_contributions_that_do_not_come_from_pages() {
        let root = write(
            "ids-page-source",
            &[
                (
                    "pages.ts",
                    "export const pages = [{ id: \"nte-update\" }];\n",
                ),
                (
                    "contribution.ts",
                    "export const contribution = { namespace: \"nte\", pageContributions: importedPages };\n",
                ),
            ],
        );
        let error = read(
            "ids-page-source",
            &root.join("packages/ids-page-source/ui/src"),
        )
        .expect_err("页面来源必须限定在 pages.ts");
        assert!(error.contains("pageContributions"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn host_page_ids_come_from_the_host_declaration() {
        let root = root("ids-host");
        let host = root.join("ui/src/host");
        fs::create_dir_all(&host).expect("创建宿主目录");
        fs::write(
            host.join("pages.ts"),
            "export const hostPageEntries = [{ id: \"home\" }, { \"id\": \"game-mode\" }] as const;\n",
        )
        .expect("写入宿主页面");
        assert_eq!(
            host_page_ids(&root).expect("读取宿主页面"),
            vec!["home".to_string(), "game-mode".to_string()]
        );
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn rejects_ids_without_the_package_namespace() {
        let root = write(
            "ids-unprefixed",
            &[
                ("pages.ts", "export const pages = [{ id: \"update\" }];\n"),
                ("contribution.ts", CONTRIBUTION),
            ],
        );
        let error = read(
            "ids-unprefixed",
            &root.join("packages/ids-unprefixed/ui/src"),
        )
        .expect_err("未命名空间的 id 必须失败");
        assert!(error.contains("\"update\""), "{error}");
        assert!(error.contains("为前缀"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn rejects_ids_that_are_not_kebab_case() {
        for (dir, id) in [
            ("ids-dot", "nte.update"),
            ("ids-upper", "nte-Update"),
            ("ids-space", "nte update"),
        ] {
            let pages = format!("export const pages = [{{ id: \"{id}\" }}];\n");
            let root = write(
                dir,
                &[
                    ("pages.ts", pages.as_str()),
                    ("contribution.ts", CONTRIBUTION),
                ],
            );
            let error = read(dir, &root.join(format!("packages/{dir}/ui/src")))
                .expect_err("非 kebab 的 id 必须失败");
            assert!(error.contains(id), "{error}");
            fs::remove_dir_all(root).expect("清理临时目录");
        }
    }

    #[test]
    fn rejects_game_and_preview_literals_that_differ_from_the_namespace() {
        for (dir, file, declaration) in [
            (
                "ids-game",
                "game.ts",
                "export const game = { id: \"fixture\" };\n",
            ),
            (
                "ids-preview",
                "preview.ts",
                "export const preview = { namespace: \"fixture\" };\n",
            ),
        ] {
            let root = write(
                dir,
                &[
                    (
                        "pages.ts",
                        "export const pages = [{ id: \"nte-update\" }];\n",
                    ),
                    ("contribution.ts", CONTRIBUTION),
                    (file, declaration),
                ],
            );
            let error = read(dir, &root.join(format!("packages/{dir}/ui/src")))
                .expect_err("与命名空间不一致的 id 必须失败");
            assert!(error.contains("必须与包命名空间"), "{error}");
            fs::remove_dir_all(root).expect("清理临时目录");
        }
    }

    #[test]
    fn rejects_ids_that_are_not_string_literals() {
        let root = write(
            "ids-computed",
            &[
                ("pages.ts", "export const pages = [{ id: pageId }];\n"),
                ("contribution.ts", CONTRIBUTION),
            ],
        );
        let error = read("ids-computed", &root.join("packages/ids-computed/ui/src"))
            .expect_err("非字面量 id 必须失败");
        assert!(error.contains("字符串字面量"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }
}
