//! 扫描 `launcher/packages/*/ui` 并渲染宿主注册表生成物.
//!
//! 生成物只装配各包声明, 不执行包代码; 贡献 id 从声明字面量读取并校验,
//! 格式化规则对齐 prettier 的 80 列与常见的换行方式.

use crate::ids;
use crate::Output;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const GENERATED: &str = "由 launcher/tools/ui-registry-gen 生成, 请勿手改.";
const PAGES_DECLARATION: &str = "声明源: launcher/packages/*/ui/src/pages.ts, game.ts, preview.ts.";
const CONTRIBUTIONS_DECLARATION: &str = "声明源: launcher/packages/*/ui/src/contribution.ts.";
const PRINT_WIDTH: usize = 80;

const PAGES_OUTPUT: &str = "ui/src/host/generated/page-entries.ts";
const CONTRIBUTIONS_OUTPUT: &str = "ui/src/host/generated/package-contributions.ts";

/// 一个参与装配的游戏包.
struct Package {
    /// 包目录名, 例如 `game-nte`.
    dir: String,
    /// 目录名转换出的标识符前缀, 例如 `gameNte`.
    ident: String,
    has_game: bool,
    has_preview: bool,
}

pub fn outputs(root: &Path) -> Result<Vec<Output>, String> {
    let packages = packages(root)?;
    Ok(vec![
        Output {
            path: PAGES_OUTPUT,
            content: page_entries(&packages),
        },
        Output {
            path: CONTRIBUTIONS_OUTPUT,
            content: package_contributions(&packages),
        },
    ])
}

fn packages(root: &Path) -> Result<Vec<Package>, String> {
    let host = ids::host_page_ids(root)?;
    let dir = root.join("packages");
    let entries =
        fs::read_dir(&dir).map_err(|error| format!("launcher/packages: 无法读取: {error}"))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("launcher/packages: 无法读取: {error}"))?;
        let ui = entry.path().join("ui");
        if !ui.join("src").is_dir() {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    let mut packages = Vec::new();
    let mut namespaces = BTreeMap::new();
    let mut contributions = BTreeMap::new();
    let mut games = BTreeMap::new();
    for id in &host {
        register(&mut contributions, "宿主页面 id", id, ids::HOST_PAGES)?;
    }
    for name in names {
        let src = dir.join(&name).join("ui/src");
        require(&name, &src, "pages.ts")?;
        require(&name, &src, "contribution.ts")?;
        let ids = ids::read(&name, &src)?;
        let owner = format!("launcher/packages/{name}/ui");
        register(&mut namespaces, "包命名空间", &ids.namespace, &owner)?;
        for id in ids.contributions() {
            register(&mut contributions, "贡献 id", id, &owner)?;
        }
        if let Some(game) = &ids.game {
            register(&mut games, "游戏展示 id", game, &owner)?;
        }
        let ident = identifier(&name)?;
        packages.push(Package {
            has_game: ids.game.is_some(),
            has_preview: ids.preview.is_some(),
            dir: name,
            ident,
        });
    }
    Ok(packages)
}

fn register(
    seen: &mut BTreeMap<String, String>,
    kind: &str,
    id: &str,
    owner: &str,
) -> Result<(), String> {
    if let Some(existing) = seen.get(id) {
        return Err(format!("{owner}: {kind} \"{id}\" 与 {existing} 重复"));
    }
    seen.insert(id.to_string(), owner.to_string());
    Ok(())
}

fn require(package: &str, src: &Path, file: &str) -> Result<(), String> {
    if src.join(file).is_file() {
        return Ok(());
    }
    Err(format!(
        "launcher/packages/{package}/ui: 缺少 src/{file}; 每个包必须提供 pages.ts 与 contribution.ts"
    ))
}

/// 目录名转标识符: `game-nte` 转 `gameNte`.
fn identifier(dir: &str) -> Result<String, String> {
    if dir.is_empty()
        || !dir
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || !dir.as_bytes()[0].is_ascii_lowercase()
    {
        return Err(format!(
            "launcher/packages/{dir}/ui: 目录名只能包含小写字母, 数字与连字符, 且以字母开头"
        ));
    }
    let mut ident = String::new();
    for (index, part) in dir.split('-').enumerate() {
        if part.is_empty() {
            return Err(format!(
                "launcher/packages/{dir}/ui: 目录名不能包含连续的连字符"
            ));
        }
        if index == 0 {
            ident.push_str(part);
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                ident.push(first.to_ascii_uppercase());
                ident.push_str(chars.as_str());
            }
        }
    }
    Ok(ident)
}

fn page_entries(packages: &[Package]) -> String {
    let mut text = header(PAGES_DECLARATION);
    for package in packages {
        import(
            &mut text,
            &format!("pages as {}Pages", package.ident),
            &format!("@packages/{}/ui/src/pages", package.dir),
        );
        if package.has_game {
            import(
                &mut text,
                &format!("game as {}Game", package.ident),
                &format!("@packages/{}/ui/src/game", package.dir),
            );
        }
        if package.has_preview {
            import(
                &mut text,
                &format!("preview as {}Preview", package.ident),
                &format!("@packages/{}/ui/src/preview", package.dir),
            );
        }
    }
    text.push('\n');
    let sections = [
        array(
            "packagePages",
            packages.iter().map(|package| {
                (
                    package.dir.as_str(),
                    "pages",
                    format!("{}Pages", package.ident),
                )
            }),
        ),
        array(
            "packageGames",
            packages
                .iter()
                .filter(|package| package.has_game)
                .map(|package| {
                    (
                        package.dir.as_str(),
                        "game",
                        format!("{}Game", package.ident),
                    )
                }),
        ),
        array(
            "packagePreviews",
            packages
                .iter()
                .filter(|package| package.has_preview)
                .map(|package| {
                    (
                        package.dir.as_str(),
                        "preview",
                        format!("{}Preview", package.ident),
                    )
                }),
        ),
    ];
    text.push_str(&sections.join("\n\n"));
    text.push('\n');
    text
}

fn package_contributions(packages: &[Package]) -> String {
    let mut text = header(CONTRIBUTIONS_DECLARATION);
    for package in packages {
        import(
            &mut text,
            &format!("contribution as {}Contribution", package.ident),
            &format!("@packages/{}/ui/src/contribution", package.dir),
        );
    }
    text.push('\n');
    text.push_str(&array(
        "packageContributions",
        packages.iter().map(|package| {
            (
                package.dir.as_str(),
                "contribution",
                format!("{}Contribution", package.ident),
            )
        }),
    ));
    text.push('\n');
    text
}

fn header(declaration: &str) -> String {
    format!("// {GENERATED}\n// {declaration}\n")
}

fn import(text: &mut String, specifier: &str, module: &str) {
    // prettier 不拆分只含一个具名导入的 import 行, 这里保持单行.
    text.push_str(&format!("import {{ {specifier} }} from \"{module}\";\n"));
}

/// 渲染一个 `export const ... = [...] as const;`.
///
/// prettier 会把只有一个元素且未超宽度的数组压成单行; 其余情况每个元素
/// 独占一行, 数组之间空一行.
fn array<'a>(name: &str, entries: impl Iterator<Item = (&'a str, &'a str, String)>) -> String {
    let lines: Vec<Vec<String>> = entries
        .map(|(dir, key, value)| entry_lines(dir, key, &value))
        .collect();
    if lines.is_empty() {
        return format!("export const {name} = [] as const;");
    }
    if lines.len() == 1 && lines[0].len() == 1 {
        let inner = lines[0][0].trim_start().trim_end_matches(',');
        let single = format!("export const {name} = [{inner}] as const;");
        if width(&single) <= PRINT_WIDTH {
            return single;
        }
    }
    let mut text = format!("export const {name} = [\n");
    for entry in lines {
        for line in entry {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text.push_str("] as const;");
    text
}

fn entry_lines(dir: &str, key: &str, value: &str) -> Vec<String> {
    let one_line = format!("  {{ dir: \"{dir}\", {key}: {value} }},");
    if width(&one_line) <= PRINT_WIDTH {
        return vec![one_line];
    }
    let member = format!("    {key}: {value},");
    let mut lines = vec!["  {".to_string(), format!("    dir: \"{dir}\",")];
    if width(&member) <= PRINT_WIDTH {
        lines.push(member);
    } else {
        lines.push(format!("    {key}:"));
        lines.push(format!("      {value},"));
    }
    lines.push("  },".to_string());
    lines
}

fn width(value: &str) -> usize {
    value.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ui-registry-gen-{name}-{}", std::process::id()))
    }

    fn write_declarations(root: &Path, dir: &str, page_id: &str) {
        let src = root.join("packages").join(dir).join("ui/src");
        fs::create_dir_all(&src).expect("创建包目录");
        fs::write(
            src.join("pages.ts"),
            format!("export const pages = [{{ id: \"{page_id}\" }}];\n"),
        )
        .expect("写入 pages");
        fs::write(
            src.join("contribution.ts"),
            format!(
                "export const contribution = {{ namespace: \"{dir}\", pageContributions: pages.map((page) => ({{ ...page }})), homeCards: [], settingsSections: [] }};\n"
            ),
        )
        .expect("写入 contribution");
    }

    fn write_package(root: &Path, dir: &str, game: bool, preview: bool) {
        write_declarations(root, dir, &format!("{dir}-page"));
        let src = root.join("packages").join(dir).join("ui/src");
        if game {
            fs::write(
                src.join("game.ts"),
                format!("export const game = {{ id: \"{dir}\" }};\n"),
            )
            .expect("写入 game");
        }
        if preview {
            fs::write(
                src.join("preview.ts"),
                format!("export const preview = {{ namespace: \"{dir}\" }};\n"),
            )
            .expect("写入 preview");
        }
    }

    fn write_host_pages(root: &Path, ids: &[&str]) {
        let host = root.join("ui/src/host");
        fs::create_dir_all(&host).expect("创建宿主目录");
        let entries = ids
            .iter()
            .map(|id| format!("  {{ id: \"{id}\" }},\n"))
            .collect::<String>();
        fs::write(
            host.join("pages.ts"),
            format!("export const hostPageEntries = [\n{entries}] as const;\n"),
        )
        .expect("写入宿主页面");
    }

    #[test]
    fn identifiers_are_camel_case() {
        assert_eq!(identifier("game-nte").unwrap(), "gameNte");
        assert_eq!(identifier("game-fixture-2").unwrap(), "gameFixture2");
        assert!(identifier("Game-nte").is_err());
        assert!(identifier("game_nte").is_err());
        assert!(identifier("game--nte").is_err());
    }

    #[test]
    fn outputs_are_sorted_by_directory() {
        let root = temp_root("sort");
        write_host_pages(&root, &["home"]);
        write_package(&root, "game-zeta", true, false);
        write_package(&root, "game-alpha", false, true);
        let outputs = outputs(&root).expect("生成");
        assert!(outputs[0].content.contains("game-alpha"));
        assert!(
            outputs[0].content.find("game-alpha").unwrap()
                < outputs[0].content.find("game-zeta").unwrap()
        );
        assert!(outputs[0].content.contains("export const packageGames"));
        assert!(outputs[0].content.contains("export const packagePreviews"));
        fs::remove_dir_all(&root).expect("清理临时目录");
    }

    #[test]
    fn missing_assembly_module_names_the_package() {
        let root = temp_root("missing");
        write_host_pages(&root, &["home"]);
        let src = root.join("packages/game-broken/ui/src");
        fs::create_dir_all(&src).expect("创建包目录");
        fs::write(src.join("pages.ts"), "export const pages = [];\n").expect("写入 pages");
        let error = outputs(&root).expect_err("缺少清单必须失败");
        assert!(error.contains("launcher/packages/game-broken/ui"));
        assert!(error.contains("src/contribution.ts"));
        fs::remove_dir_all(&root).expect("清理临时目录");
    }

    #[test]
    fn non_namespaced_ids_fail_generation() {
        let root = temp_root("unprefixed");
        write_host_pages(&root, &["home"]);
        let src = root.join("packages/game-broken/ui/src");
        fs::create_dir_all(&src).expect("创建包目录");
        fs::write(
            src.join("pages.ts"),
            "export const pages = [{ id: \"update\" }];\n",
        )
        .expect("写入 pages");
        fs::write(
            src.join("contribution.ts"),
            "export const contribution = { namespace: \"broken\", homeCards: [], settingsSections: [] };\n",
        )
        .expect("写入 contribution");
        let error = outputs(&root).expect_err("未命名空间的 id 必须失败");
        assert!(error.contains("\"update\""), "{error}");
        assert!(error.contains("为前缀"), "{error}");
        fs::remove_dir_all(&root).expect("清理临时目录");
    }

    #[test]
    fn duplicate_contribution_ids_are_rejected() {
        let mut seen = BTreeMap::new();
        register(
            &mut seen,
            "贡献 id",
            "nte-update",
            "launcher/packages/game-nte/ui",
        )
        .expect("首次注册");
        let error = register(
            &mut seen,
            "贡献 id",
            "nte-update",
            "launcher/packages/game-other/ui",
        )
        .expect_err("重复 id 必须失败");
        assert!(error.contains("重复"), "{error}");
        assert!(error.contains("game-nte"), "{error}");
    }

    #[test]
    fn package_ids_cannot_repeat_host_page_ids() {
        let root = temp_root("host-conflict");
        write_host_pages(&root, &["home", "runtime-setup"]);
        write_declarations(&root, "runtime", "runtime-setup");
        let error = outputs(&root).expect_err("与宿主页面重名必须失败");
        assert!(error.contains("runtime-setup"), "{error}");
        assert!(error.contains("launcher/ui/src/host/pages.ts"), "{error}");
        fs::remove_dir_all(&root).expect("清理临时目录");
    }

    #[test]
    fn every_output_starts_with_the_generated_marker() {
        for output in outputs(&crate::launcher_root()).expect("生成") {
            assert!(
                output
                    .content
                    .starts_with("// 由 launcher/tools/ui-registry-gen 生成, 请勿手改."),
                "{} 缺少生成标记",
                output.path
            );
        }
    }

    #[test]
    fn entries_stay_within_the_print_width() {
        assert_eq!(
            entry_lines("game-nte", "contribution", "gameNteContribution"),
            vec!["  { dir: \"game-nte\", contribution: gameNteContribution },"]
        );
    }

    #[test]
    fn single_element_arrays_collapse_like_prettier() {
        let text = array(
            "packageGames",
            [("game-nte", "game", "gameNteGame".to_string())].into_iter(),
        );
        assert_eq!(
            text,
            "export const packageGames = [{ dir: \"game-nte\", game: gameNteGame }] as const;"
        );
    }
}
