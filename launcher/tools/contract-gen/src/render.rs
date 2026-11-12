//! 生成物的具体内容; 格式化规则对齐 prettier 的 80 列与续行方式.

use crate::Output;
use oddity_contract::{Audience, Command, NamedType, TsType, TypeModule, COMMANDS, EVENTS};
use std::collections::{BTreeMap, BTreeSet};

const GENERATED: &str = "由 launcher/tools/contract-gen 生成, 请勿手改.";
const DECLARATION: &str = "声明源: launcher/crates/oddity-contract/src/lib.rs.";
const PRINT_WIDTH: usize = 80;

pub fn outputs() -> Vec<Output> {
    vec![
        Output {
            path: "ui/electron/commands.ts",
            content: electron_commands(),
        },
        Output {
            path: "ui/src/types/generated/commands.ts",
            content: generated_types(),
        },
        Output {
            path: "ui/src/core/generated/command-ids.ts",
            content: command_ids(),
        },
        Output {
            path: "CONTRACTS.md",
            content: catalog(),
        },
    ]
}

/// 渲染层可调用的命令: 白名单、TypeScript 类型和演示回调都覆盖这些.
fn ui_commands() -> impl Iterator<Item = &'static Command> {
    COMMANDS
        .iter()
        .filter(|command| matches!(command.audience, Audience::Renderer | Audience::Window))
}

fn electron_commands() -> String {
    let mut text = format!("// {GENERATED}\n// {DECLARATION}\n");
    text.push_str("import type { Commands } from \"../src/types/launcher\";\n");
    text.push_str("export const commands = {\n");
    for command in ui_commands() {
        text.push_str(&format!("  {}: true,\n", command.id));
    }
    text.push_str("} satisfies Record<keyof Commands, true>;\n");
    text
}

fn generated_types() -> String {
    let mut text = format!("// {GENERATED}\n// {DECLARATION}\n");
    let mut imports: BTreeMap<&'static str, BTreeSet<&'static str>> = BTreeMap::new();
    for command in ui_commands() {
        collect_named(&command.result, &mut imports);
        collect_named(&command.args, &mut imports);
    }
    for (module, names) in &imports {
        write_import(&mut text, module, names);
    }
    text.push_str("\nexport interface Commands {\n");
    for command in ui_commands() {
        write_member(&mut text, 2, command.id, &command.result);
    }
    text.push_str("}\n\nexport interface CommandArgs {\n");
    for command in ui_commands() {
        write_member(&mut text, 2, command.id, &command.args);
    }
    text.push_str("}\n");
    text
}

fn command_ids() -> String {
    let mut text = format!("// {GENERATED}\n// {DECLARATION}\n");
    text.push_str("// 渲染层命令集合; api.ts 的演示分发表按这里做穷尽性检查.\n");
    text.push_str("export const uiCommandIds = [\n");
    for command in ui_commands() {
        text.push_str(&format!("  \"{}\",\n", command.id));
    }
    text.push_str("] as const;\nexport type UiCommand = (typeof uiCommandIds)[number];\n");
    text
}

fn catalog() -> String {
    let mut text = format!("<!-- {GENERATED} -->\n");
    text.push_str(&format!("<!-- {DECLARATION} -->\n\n"));
    text.push_str("# 契约目录\n\n");
    text.push_str(
        "受众: 渲染层可经 Electron 白名单调用; 仅宿主只由 Electron 主进程调用; 窗口只在 Electron 窗口层处理, 不经过后端.\n\n",
    );
    text.push_str("## 命令\n\n");
    text.push_str("| 命令 | 受众 | 需要当前游戏 | 参数 | 返回 |\n");
    text.push_str("| --- | --- | --- | --- | --- |\n");
    for command in COMMANDS {
        text.push_str(&format!(
            "| `{}` | {} | {} | `{}` | `{}` |\n",
            command.id,
            audience_label(command.audience),
            if command.current_game { "是" } else { "否" },
            table_type(&command.args),
            table_type(&command.result),
        ));
    }
    text.push_str("\n## 事件\n\n");
    text.push_str("| 事件 | 发射领域 |\n");
    text.push_str("| --- | --- |\n");
    for event in EVENTS {
        text.push_str(&format!("| `{}` | {} |\n", event.id, event.domain));
    }
    text.push_str(
        "\n事件只声明名称与发射领域; 发射调用必须使用这里声明的事件名, 且每条声明至少被发射一次.\n\
         发送走宿主的事件出口 `oddity_runtime::EventSink`; payload 类型仍由渲染层手写.\n",
    );
    text
}

fn audience_label(audience: Audience) -> &'static str {
    match audience {
        Audience::Renderer => "渲染层",
        Audience::Host => "仅宿主",
        Audience::Window => "窗口",
    }
}

fn table_type(ty: &TsType) -> String {
    inline_type(ty).replace('|', "\\|")
}

fn module_path(module: TypeModule) -> &'static str {
    match module {
        TypeModule::Launcher => "../launcher",
        TypeModule::Gamepad => "../gamepad",
        TypeModule::FilePicker => "../../core/filePicker",
    }
}

fn collect_named(ty: &TsType, imports: &mut BTreeMap<&'static str, BTreeSet<&'static str>>) {
    match ty {
        TsType::Named(NamedType { name, module }) => {
            imports
                .entry(module_path(*module))
                .or_default()
                .insert(name);
        }
        TsType::Object(fields) => {
            for field in *fields {
                collect_named(&field.ty, imports);
            }
        }
        TsType::Array(inner) => collect_named(inner, imports),
        TsType::Union(types) => {
            for item in *types {
                collect_named(item, imports);
            }
        }
        TsType::Literal(_) | TsType::Primitive(_) | TsType::NoArgs => {}
    }
}

fn write_import(text: &mut String, module: &str, names: &BTreeSet<&'static str>) {
    let joined = names.iter().copied().collect::<Vec<_>>().join(", ");
    let one_line = format!("import type {{ {joined} }} from \"{module}\";");
    if width(&one_line) <= PRINT_WIDTH {
        text.push_str(&one_line);
        text.push('\n');
        return;
    }
    text.push_str("import type {\n");
    for name in names {
        text.push_str(&format!("  {name},\n"));
    }
    text.push_str(&format!("}} from \"{module}\";\n"));
}

fn write_member(text: &mut String, indent: usize, name: &str, ty: &TsType) {
    let pad = " ".repeat(indent);
    let inline = inline_type(ty);
    let one_line = format!("{pad}{name}: {inline};");
    if width(&one_line) <= PRINT_WIDTH {
        text.push_str(&one_line);
        text.push('\n');
        return;
    }
    match ty {
        TsType::Object(fields) => {
            text.push_str(&format!("{pad}{name}: {{\n"));
            for field in *fields {
                write_member(text, indent + 2, field.name, &field.ty);
            }
            text.push_str(&format!("{pad}}};\n"));
        }
        TsType::Union(types) => {
            text.push_str(&format!("{pad}{name}:\n"));
            for (index, item) in types.iter().enumerate() {
                let terminator = if index + 1 == types.len() { ";" } else { "" };
                text.push_str(&format!(
                    "{}| {}{terminator}\n",
                    " ".repeat(indent + 2),
                    inline_type(item)
                ));
            }
        }
        _ => {
            text.push_str(&one_line);
            text.push('\n');
        }
    }
}

fn inline_type(ty: &TsType) -> String {
    match ty {
        TsType::Named(named) => named.name.to_string(),
        TsType::Literal(value) => format!("\"{value}\""),
        TsType::Primitive(name) => (*name).to_string(),
        TsType::Object(fields) => {
            let fields = fields
                .iter()
                .map(|field| format!("{}: {}", field.name, inline_type(&field.ty)))
                .collect::<Vec<_>>()
                .join("; ");
            format!("{{ {fields} }}")
        }
        TsType::Array(inner) => format!("{}[]", inline_type(inner)),
        TsType::Union(types) => types
            .iter()
            .map(inline_type)
            .collect::<Vec<_>>()
            .join(" | "),
        TsType::NoArgs => "Record<string, never>".to_string(),
    }
}

fn width(value: &str) -> usize {
    value.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRING_OR_NULL: &[TsType] = &[TsType::STRING, TsType::NULL];
    const PATH_FIELD: OddityField = OddityField::new("path", TsType::union(STRING_OR_NULL));
    const PATH_OBJECT: TsType = TsType::Object(&[PATH_FIELD]);
    const GAME_ACCOUNTS: TsType = TsType::array(&TsType::named("GameAccount"));
    const PLAN_FIELDS: &[OddityField] = &[
        OddityField::new("allUsers", TsType::BOOLEAN),
        OddityField::new("proton", TsType::union(STRING_OR_NULL)),
        OddityField::new("createEnvironment", TsType::BOOLEAN),
    ];
    const CN_OR_GLOBAL: TsType = TsType::union(&[TsType::literal("cn"), TsType::literal("global")]);

    use oddity_contract::TsField as OddityField;

    #[test]
    fn inline_types_match_typescript_syntax() {
        assert_eq!(inline_type(&TsType::UNDEFINED), "undefined");
        assert_eq!(inline_type(&TsType::NO_ARGS), "Record<string, never>");
        assert_eq!(inline_type(&TsType::union(STRING_OR_NULL)), "string | null");
        assert_eq!(inline_type(&PATH_OBJECT), "{ path: string | null }");
        assert_eq!(inline_type(&GAME_ACCOUNTS), "GameAccount[]");
        assert_eq!(inline_type(&TsType::gamepad("Profile")), "Profile");
    }

    #[test]
    fn long_object_members_break_like_prettier() {
        let ty = TsType::Object(PLAN_FIELDS);
        let mut text = String::new();
        write_member(&mut text, 2, "runtime_setup_plan", &ty);
        assert_eq!(
            text,
            "  runtime_setup_plan: {\n    allUsers: boolean;\n    proton: string | null;\n    createEnvironment: boolean;\n  };\n"
        );
    }

    #[test]
    fn short_object_members_stay_on_one_line() {
        let mut text = String::new();
        write_member(&mut text, 2, "browse_files", &PATH_OBJECT);
        assert_eq!(text, "  browse_files: { path: string | null };\n");
    }

    #[test]
    fn the_whitelist_only_contains_ui_commands() {
        let text = electron_commands();
        assert!(text.contains("runtime_setup_info: true,"));
        assert!(text.contains("win_hide: true,"));
        assert!(!text.contains("host_state: true,"));
        assert!(!text.contains("external_launch: true,"));
    }

    #[test]
    fn catalog_escapes_union_pipes() {
        assert_eq!(table_type(&CN_OR_GLOBAL), "\"cn\" \\| \"global\"");
    }

    #[test]
    fn catalog_lists_events_without_hand_counts() {
        let text = catalog();
        assert!(text.contains("| 事件 | 发射领域 |"));
        assert!(text.contains("| `game-state` | 游戏进程 |"));
        assert!(!text.contains("| 发射点 |"));
        assert!(text.contains("oddity_runtime::EventSink"));
    }

    #[test]
    fn generated_types_import_within_their_modules() {
        let text = generated_types();
        assert!(text.contains("from \"../gamepad\";"));
        assert!(text.contains("from \"../../core/filePicker\";"));
        assert!(text.contains("export interface Commands {"));
        assert!(text.contains("export interface CommandArgs {"));
        assert!(text.contains("  game_accounts: GameAccount[];"));
    }

    #[test]
    fn every_output_starts_with_the_generated_marker() {
        for output in outputs() {
            assert!(
                output
                    .content
                    .starts_with("// 由 launcher/tools/contract-gen 生成, 请勿手改.")
                    || output
                        .content
                        .starts_with("<!-- 由 launcher/tools/contract-gen 生成, 请勿手改. -->"),
                "{} 缺少生成标记",
                output.path
            );
        }
    }
}
