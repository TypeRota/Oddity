use std::collections::BTreeMap;
use std::{fs, io, path::Path};

use syn::{Expr, Item, Lit, Member};

pub type Catalog = BTreeMap<String, u16>;

/// 只收录能力表登记的描述符, 未登记的常量不提供能力.
pub fn read(root: &Path, file: &str) -> Result<Catalog, String> {
    let source = match fs::read_to_string(root.join(file)) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Catalog::new()),
        Err(error) => return Err(format!("launcher/{file}: 无法读取: {error}")),
    };
    let syntax = syn::parse_file(&source).map_err(|error| format!("launcher/{file}: {error}"))?;
    let Some(sheet) = syntax.items.iter().find_map(|item| match item {
        Item::Static(item) if item.ident == "CAPABILITIES" => Some(item.expr.as_ref()),
        _ => None,
    }) else {
        return Ok(Catalog::new());
    };
    let Expr::Reference(sheet) = sheet else {
        return Err(format!("launcher/{file}: CAPABILITIES 必须引用数组字面量"));
    };
    let Expr::Array(sheet) = sheet.expr.as_ref() else {
        return Err(format!("launcher/{file}: CAPABILITIES 必须引用数组字面量"));
    };
    let mut catalog = Catalog::new();
    for entry in &sheet.elems {
        let invalid_entry =
            || format!("launcher/{file}: 能力表条目必须使用 Capability::new 和本文件的描述符常量");
        let Expr::Call(entry) = entry else {
            return Err(invalid_entry());
        };
        let (Expr::Path(constructor), Some(Expr::Path(descriptor))) =
            (entry.func.as_ref(), entry.args.first())
        else {
            return Err(invalid_entry());
        };
        let mut segments = constructor.path.segments.iter().rev();
        if !matches!((segments.next(), segments.next()), (Some(method), Some(kind))
            if method.ident == "new" && kind.ident == "Capability")
            || entry.args.len() != 2
        {
            return Err(invalid_entry());
        }
        let name = descriptor.path.get_ident().ok_or_else(invalid_entry)?;
        let descriptor = syntax.items.iter().find_map(|item| match item {
            Item::Const(item) if item.ident == *name => Some(item.expr.as_ref()),
            _ => None,
        });
        let Some(Expr::Struct(descriptor)) = descriptor else {
            return Err(invalid_entry());
        };
        let field = |key: &str| {
            descriptor.fields.iter().find_map(|field| {
                if matches!(&field.member, Member::Named(member) if member == key) {
                    if let Expr::Call(value) = &field.expr {
                        if let Some(Expr::Lit(value)) = value.args.first() {
                            return Some(&value.lit);
                        }
                    }
                }
                None
            })
        };
        let (Some(Lit::Str(id)), Some(Lit::Int(revision))) = (field("id"), field("revision"))
        else {
            return Err(format!(
                "launcher/{file}: {name} 必须声明字面量 id 和 revision"
            ));
        };
        let id = id.value();
        let revision = revision
            .base10_parse::<u16>()
            .map_err(|error| format!("launcher/{file}: {name} 的 revision 无效: {error}"))?;
        if catalog.insert(id.clone(), revision).is_some() {
            return Err(format!("launcher/{file}: 重复登记能力 \"{id}\""));
        }
    }
    Ok(catalog)
}
