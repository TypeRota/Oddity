//! 读取 UI 声明, 校验贡献 id 与能力依赖.
//!
//! 页面、首页卡片与设置分区使用包命名空间前缀. 游戏展示的 id 和预览的
//! namespace 与包命名空间一致. 贡献以及能力依赖都必须能静态确定.

use crate::capabilities::Catalog;
use std::collections::BTreeMap;
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
pub fn read(dir: &str, src: &Path, catalog: &Catalog) -> Result<PackageIds, String> {
    let contribution = read_file(dir, src, "contribution.ts")?;
    let pages = read_file(dir, src, "pages.ts")?;
    let contribution = Manifest::new(
        &file_path(dir, "contribution.ts"),
        src,
        &contribution,
        catalog,
    )?;
    let pages = Manifest::new(&file_path(dir, "pages.ts"), src, &pages, catalog)?;
    let (namespace, cards, sections) = contribution.contribution_ids()?;
    let page_ids = pages.array_ids(&pages.path, pages.declaration("pages", true)?, false)?;
    let game = match optional_file(dir, src, "game.ts")? {
        Some(source) => {
            let game = Manifest::new(&file_path(dir, "game.ts"), src, &source, catalog)?;
            let fields = object(&game.path, game.declaration("game", true)?)?;
            let id = string_property(&game.path, &fields, "id")?;
            if let Some(modes) = fields.get("launchModeRequires") {
                let modes = object(&game.path, modes)?;
                for (mode, requires) in modes {
                    game.requires(&format!("{}: {id}/{mode}", game.path), requires)?;
                }
            }
            Some(id)
        }
        None => None,
    };
    let preview = match optional_file(dir, src, "preview.ts")? {
        Some(source) => {
            let preview = Manifest::new(&file_path(dir, "preview.ts"), src, &source, catalog)?;
            let fields = object(&preview.path, preview.declaration("preview", true)?)?;
            Some(string_property(&preview.path, &fields, "namespace")?)
        }
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
pub fn host_page_ids(root: &Path, catalog: &Catalog) -> Result<Vec<String>, String> {
    let dir = root.join("ui/src/host");
    let source = fs::read_to_string(dir.join("pages.ts"))
        .map_err(|error| format!("{HOST_PAGES}: 无法读取: {error}"))?;
    let pages = Manifest::new(HOST_PAGES, &dir, &source, catalog)?;
    pages.array_ids(
        HOST_PAGES,
        pages.declaration("hostPageEntries", true)?,
        false,
    )
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

struct Manifest<'a> {
    path: String,
    dir: &'a Path,
    tokens: Vec<Token>,
    catalog: &'a Catalog,
}

impl<'a> Manifest<'a> {
    fn new(path: &str, dir: &'a Path, source: &str, catalog: &'a Catalog) -> Result<Self, String> {
        let tokens = lex(source).map_err(|error| format!("{path}: {error}"))?;
        Ok(Self {
            path: path.into(),
            dir,
            tokens,
            catalog,
        })
    }

    fn declaration(&self, name: &str, exported: bool) -> Result<&[Token], String> {
        let value = top_level(&self.tokens)
            .find_map(|tokens| {
                let tokens = match tokens {
                    [Token::Ident(keyword), rest @ ..] if keyword == "export" => rest,
                    _ if !exported => tokens,
                    _ => return None,
                };
                match tokens {
                    [Token::Ident(keyword), Token::Ident(binding), Token::Punct('='), value @ ..]
                        if keyword == "const" && binding == name =>
                    {
                        Some(value)
                    }
                    _ => None,
                }
            })
            .ok_or_else(|| format!("{}: 找不到常量 {name} 的声明", self.path))?;
        let value = top_level(value)
            .find(|tail| tail.first() == Some(&Token::Punct(';')))
            .map_or(value, |tail| &value[..value.len() - tail.len()]);
        let assertion = top_level(value).find(|tail| {
            let [Token::Ident(as_kw), Token::Ident(const_kw), suffix @ ..] = tail else {
                return false;
            };
            if as_kw != "as" || const_kw != "const" {
                return false;
            }
            match suffix {
                [] => true,
                [Token::Ident(satisfies), types @ ..] if satisfies == "satisfies" => {
                    !types.is_empty()
                        && types.iter().all(|token| {
                            matches!(
                                token,
                                Token::Ident(_)
                                    | Token::Str(_)
                                    | Token::Punct('<' | '>' | '[' | ']' | '.')
                            )
                        })
                }
                _ => false,
            }
        });
        Ok(assertion.map_or(value, |tail| &value[..value.len() - tail.len()]))
    }

    fn named_import(&self, name: &str) -> Option<(&str, &str)> {
        top_level(&self.tokens).find_map(|tokens| {
            let [Token::Ident(keyword), Token::Punct('{'), rest @ ..] = tokens else {
                return None;
            };
            if keyword != "import" {
                return None;
            }
            let end = rest.iter().position(|token| *token == Token::Punct('}'))?;
            let [Token::Ident(from), Token::Str(module), ..] = &rest[end + 1..] else {
                return None;
            };
            if from != "from" {
                return None;
            }
            rest[..end]
                .split(|token| *token == Token::Punct(','))
                .find_map(|binding| {
                    let original = match binding {
                        [Token::Ident(original)] if original == name => original,
                        [Token::Ident(original), Token::Ident(alias), Token::Ident(local)]
                            if alias == "as" && local == name =>
                        {
                            original
                        }
                        _ => return None,
                    };
                    Some((module.as_str(), original.as_str()))
                })
        })
    }

    fn contribution_ids(&self) -> Result<(String, Vec<String>, Vec<String>), String> {
        let path = self.path.as_str();
        let fields = object(path, self.declaration("contribution", true)?)?;
        let namespace = string_property(path, &fields, "namespace")?;
        let mut cards = Vec::new();
        let mut sections = Vec::new();
        for (key, value) in fields {
            match key {
                "namespace" => {}
                "pageContributions" => {
                    let [Token::Ident(pages), Token::Punct('.'), Token::Ident(map), Token::Punct('('), Token::Punct('('), Token::Ident(param), Token::Punct(')'), Token::Punct('='), Token::Punct('>'), Token::Punct('('), Token::Punct('{'), Token::Punct('.'), Token::Punct('.'), Token::Punct('.'), Token::Ident(spread), rest @ ..] =
                        value
                    else {
                        return Err(format!(
                            "{path}: pageContributions 必须从 pages 继承页面声明"
                        ));
                    };
                    let rest = if rest.first() == Some(&Token::Punct(',')) {
                        &rest[1..]
                    } else {
                        rest
                    };
                    let end = if matches!(rest, [Token::Ident(key), Token::Punct(':'), ..] if key == "component")
                    {
                        let mut end = skip_value(path, rest, 2)?;
                        if rest[end] == Token::Punct(',') {
                            end += 1;
                        }
                        end
                    } else {
                        0
                    };
                    if map != "map"
                        || !matches!(
                            self.named_import(pages),
                            Some(("./pages" | "./pages.ts", "pages"))
                        )
                        || param != spread
                        || rest[end..] != [Token::Punct('}'), Token::Punct(')'), Token::Punct(')')]
                    {
                        return Err(format!(
                            "{path}: pageContributions 必须从 pages 继承页面声明, 只绑定 component"
                        ));
                    }
                }
                "homeCards" => cards = self.array_ids(path, value, false)?,
                "settingsSections" => sections = self.array_ids(path, value, true)?,
                other => {
                    return Err(format!(
                        "{path}: 未知的贡献键 \"{other}\", 新增贡献种类时必须同步生成器"
                    ))
                }
            }
        }
        Ok((namespace, cards, sections))
    }

    fn array_ids(
        &self,
        path: &str,
        value: &[Token],
        settings_sections: bool,
    ) -> Result<Vec<String>, String> {
        let items = array(path, value)?;
        let mut ids = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let location = format!("{path}: 数组第 {} 个元素", index + 1);
            let fields = object(&location, item)?;
            let id = string_property(&location, &fields, "id")?;
            let location = format!("{path}: {id}");
            let requires = fields.get("requires").ok_or_else(|| {
                format!("{location}: 缺少 requires, 无能力依赖时请声明 requires: []")
            })?;
            self.requires(&format!("{location}.requires"), requires)?;
            if settings_sections {
                let entries = fields
                    .get("entries")
                    .ok_or_else(|| format!("{location}: 缺少 entries"))?;
                self.array_ids(&location, entries, false)?;
            }
            ids.push(id);
        }
        Ok(ids)
    }

    fn requires(&self, location: &str, value: &[Token]) -> Result<(), String> {
        for requirement in array(location, value)? {
            let imported;
            let (path, value) = if let [Token::Ident(name)] = requirement {
                if let Some((module, original)) = self.named_import(name) {
                    if !module.starts_with("./") {
                        return Err(format!(
                            "{location}: 能力要求 {name} 必须从同包的相对路径导入"
                        ));
                    }
                    let file = if module.ends_with(".ts") {
                        module.to_owned()
                    } else {
                        format!("{module}.ts")
                    };
                    let source = fs::read_to_string(self.dir.join(&file))
                        .map_err(|error| format!("{location}: 无法读取 {file}: {error}"))?;
                    let path = format!("{}: {file}", self.path);
                    imported = Manifest::new(&path, self.dir, &source, self.catalog)?;
                    (
                        format!("{location} ({file} 中的 {original})"),
                        imported.declaration(original, true)?,
                    )
                } else {
                    (
                        format!("{location} ({name})"),
                        self.declaration(name, false)?,
                    )
                }
            } else {
                (location.to_owned(), requirement)
            };
            let fields = object(&path, value)?;
            let id = string_property(&path, &fields, "id")?;
            let revision = match fields.get("revision").copied() {
                Some([Token::Number(value)]) => value.parse::<u16>().ok(),
                _ => None,
            }
            .ok_or_else(|| {
                format!("{path}: 能力 \"{id}\" 的 revision 必须是 0 到 65535 的整数字面量")
            })?;
            let necessity = string_property(&path, &fields, "necessity")?;
            if !matches!(necessity.as_str(), "required" | "optional") {
                return Err(format!(
                    "{path}: 能力 \"{id}\" 的 necessity 必须是 required 或 optional"
                ));
            }
            let expected = self
                .catalog
                .get(&id)
                .ok_or_else(|| format!("{path}: 未知能力 \"{id}\""))?;
            if revision != *expected {
                return Err(format!(
                    "{path}: 能力 \"{id}\" 的 revision 为 {revision}, 后端声明为 {expected}"
                ));
            }
        }
        Ok(())
    }
}

fn top_level(tokens: &[Token]) -> impl Iterator<Item = &[Token]> {
    let mut depth = 0;
    tokens.iter().enumerate().filter_map(move |(index, token)| {
        let at_top = depth == 0;
        match token {
            Token::Punct('(' | '[' | '{') => depth += 1,
            Token::Punct(')' | ']' | '}') => depth -= 1,
            _ => {}
        }
        at_top.then_some(&tokens[index..])
    })
}

fn array<'a>(path: &str, tokens: &'a [Token]) -> Result<Vec<&'a [Token]>, String> {
    if tokens.first() != Some(&Token::Punct('[')) {
        return Err(format!("{path}: 需要数组字面量"));
    }
    let mut values = Vec::new();
    let mut index = 1;
    while tokens.get(index) != Some(&Token::Punct(']')) {
        let end = skip_value(path, tokens, index)?;
        if end == index {
            return Err(format!("{path}: 数组缺少元素"));
        }
        values.push(&tokens[index..end]);
        index = end + usize::from(tokens[end] == Token::Punct(','));
    }
    if index + 1 != tokens.len() {
        return Err(format!("{path}: 需要数组字面量"));
    }
    Ok(values)
}

type Properties<'a> = BTreeMap<&'a str, &'a [Token]>;

fn object<'a>(path: &str, tokens: &'a [Token]) -> Result<Properties<'a>, String> {
    if tokens.first() != Some(&Token::Punct('{')) {
        return Err(format!("{path}: 必须是对象字面量"));
    }
    let mut fields = BTreeMap::new();
    let mut index = 1;
    while tokens.get(index) != Some(&Token::Punct('}')) {
        if tokens.get(index..index + 3)
            == Some(&[Token::Punct('.'), Token::Punct('.'), Token::Punct('.')])
        {
            return Err(format!("{path}: 对象不能使用展开语法"));
        }
        let key = match tokens.get(index..index + 2) {
            Some([Token::Ident(key) | Token::Str(key), Token::Punct(':')]) => key.as_str(),
            _ => return Err(format!("{path}: 无法识别的对象属性")),
        };
        let end = skip_value(path, tokens, index + 2)?;
        if fields.insert(key, &tokens[index + 2..end]).is_some() {
            return Err(format!("{path}: 重复声明 {key}"));
        }
        index = end + usize::from(tokens[end] == Token::Punct(','));
    }
    if index + 1 != tokens.len() {
        return Err(format!("{path}: 必须是对象字面量"));
    }
    Ok(fields)
}

fn string_property(path: &str, fields: &Properties<'_>, key: &str) -> Result<String, String> {
    match fields.get(key).copied() {
        Some([Token::Str(value)]) => Ok(value.clone()),
        Some(_) => Err(format!("{path}: {key} 必须是字符串字面量")),
        None => Err(format!("{path}: 缺少 {key}")),
    }
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
    OtherLiteral,
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
        if matches!(byte, b'"' | b'\'' | b'`') {
            let (token, next) = string_literal(source, index, byte)?;
            tokens.push(token);
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

fn string_literal(source: &str, start: usize, quote: u8) -> Result<(Token, usize), String> {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    let mut plain = quote != b'`';
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                plain = false;
                index += 2;
            }
            byte if byte == quote => {
                let token = if plain {
                    Token::Str(source[start + 1..index].to_owned())
                } else {
                    Token::OtherLiteral
                };
                return Ok((token, index + 1));
            }
            b'\n' if quote != b'`' => break,
            _ => index += 1,
        }
    }
    Err("字符串没有闭合".into())
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

    const CONTRIBUTION: &str = r#"import { pages } from "./pages";
export const contribution = {
  namespace: "nte",
  pageContributions: pages.map((page) => ({ ...page })),
  homeCards: [{ id: "nte-card", requires: [], order: 1 }],
  settingsSections: [
    {
      id: "nte-game",
      requires: [],
      entries: [{ id: "distribution", requires: [] }],
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
                    "export const pages = [{ id: \"nte-update\", requires: [] }];\n",
                ),
                ("contribution.ts", CONTRIBUTION),
                ("game.ts", "export const game = { id: \"nte\" };\n"),
                (
                    "preview.ts",
                    "export const preview = { namespace: \"nte\" };\n",
                ),
            ],
        );
        let ids = read(
            "ids-ok",
            &root.join("packages/ids-ok/ui/src"),
            &Catalog::new(),
        )
        .expect("读取 id");
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
                    "export const pages = [{ \"id\": \"nte-update\", requires: [] }];\n",
                ),
                (
                    "contribution.ts",
                    "import { pages } from \"./pages\";\nexport const contribution = {\n  \"namespace\": \"nte\",\n  \"pageContributions\": pages.map((page) => ({ ...page })),\n  \"homeCards\": [{ \"id\": \"nte-card\", requires: [] }],\n  \"settingsSections\": [{ \"id\": \"nte-game\", requires: [], entries: [] }],\n} as const;\n",
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
            &Catalog::new(),
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
            let error = read(
                dir,
                &root.join(format!("packages/{dir}/ui/src")),
                &Catalog::new(),
            )
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
                    "export const pages = [{ id: \"nte-update\", requires: [] }];\n",
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
            &Catalog::new(),
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
                    "export const pages = [{ id: \"nte-update\", requires: [] }];\n",
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
            &Catalog::new(),
        )
        .expect_err("页面来源必须限定在 pages.ts");
        assert!(error.contains("pageContributions"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn rejects_overriding_page_requirements_in_component_bindings() {
        let root = write(
            "ids-page-requires",
            &[
                (
                    "pages.ts",
                    r#"export const pages = [{ id: "nte-update", requires: [] }];"#,
                ),
                (
                    "contribution.ts",
                    r#"import { pages } from "./pages";
export const contribution = {
  namespace: "nte",
  pageContributions: pages.map((page) => ({ ...page, requires: [] })),
};"#,
                ),
            ],
        );
        let error = read(
            "ids-page-requires",
            &root.join("packages/ids-page-requires/ui/src"),
            &Catalog::new(),
        )
        .expect_err("组件绑定不能覆盖页面的能力声明");
        assert!(error.contains("只绑定 component"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn host_page_ids_come_from_the_host_declaration() {
        let root = root("ids-host");
        let host = root.join("ui/src/host");
        fs::create_dir_all(&host).expect("创建宿主目录");
        fs::write(
            host.join("pages.ts"),
            "export const hostPageEntries = [{ id: \"home\", requires: [] }, { \"id\": \"game-mode\", requires: [] }] as const;\n",
        )
        .expect("写入宿主页面");
        assert_eq!(
            host_page_ids(&root, &Catalog::new()).expect("读取宿主页面"),
            vec!["home".to_string(), "game-mode".to_string()]
        );
        fs::remove_dir_all(root).expect("清理临时目录");
    }

    #[test]
    fn rejects_ids_without_the_package_namespace() {
        let root = write(
            "ids-unprefixed",
            &[
                (
                    "pages.ts",
                    "export const pages = [{ id: \"update\", requires: [] }];\n",
                ),
                ("contribution.ts", CONTRIBUTION),
            ],
        );
        let error = read(
            "ids-unprefixed",
            &root.join("packages/ids-unprefixed/ui/src"),
            &Catalog::new(),
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
            let pages = format!("export const pages = [{{ id: \"{id}\", requires: [] }}];\n");
            let root = write(
                dir,
                &[
                    ("pages.ts", pages.as_str()),
                    ("contribution.ts", CONTRIBUTION),
                ],
            );
            let error = read(
                dir,
                &root.join(format!("packages/{dir}/ui/src")),
                &Catalog::new(),
            )
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
                        "export const pages = [{ id: \"nte-update\", requires: [] }];\n",
                    ),
                    ("contribution.ts", CONTRIBUTION),
                    (file, declaration),
                ],
            );
            let error = read(
                dir,
                &root.join(format!("packages/{dir}/ui/src")),
                &Catalog::new(),
            )
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
        let error = read(
            "ids-computed",
            &root.join("packages/ids-computed/ui/src"),
            &Catalog::new(),
        )
        .expect_err("非字面量 id 必须失败");
        assert!(error.contains("字符串字面量"), "{error}");
        fs::remove_dir_all(root).expect("清理临时目录");
    }
}
