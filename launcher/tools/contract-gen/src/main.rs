//! 从 `oddity-contract` 生成界面命令表、TypeScript 类型与契约目录.
//!
//! 用法: `cargo run -p contract-gen` 写盘; 加 `--check` 只比对, 不一致时打印
//! 第一处差异并以非零码退出.

mod render;

use std::fs;
use std::path::{Path, PathBuf};

/// 一个生成物.
struct Output {
    /// 相对 `launcher/` 的路径, 同时用于提示.
    path: &'static str,
    content: String,
}

fn main() {
    let check = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => {
            eprintln!("用法: contract-gen [--check]");
            std::process::exit(2);
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let outputs = render::outputs();
    let failed = if check {
        check_outputs(&root, &outputs)
    } else {
        write_outputs(&root, &outputs)
    };
    if failed {
        std::process::exit(1);
    }
}

fn write_outputs(root: &Path, outputs: &[Output]) -> bool {
    for output in outputs {
        let path = root.join(output.path);
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!("{}: 无法创建目录: {error}", output.path);
                return true;
            }
        }
        if let Err(error) = fs::write(&path, &output.content) {
            eprintln!("{}: 无法写入: {error}", output.path);
            return true;
        }
        println!("写入 {}", output.path);
    }
    false
}

fn check_outputs(root: &Path, outputs: &[Output]) -> bool {
    let mut failed = false;
    for output in outputs {
        let path = root.join(output.path);
        let actual = match fs::read_to_string(&path) {
            Ok(actual) => actual,
            Err(error) => {
                eprintln!("{}: 无法读取生成物: {error}", output.path);
                failed = true;
                continue;
            }
        };
        if actual == output.content {
            continue;
        }
        failed = true;
        report_difference(output, &actual);
    }
    if !failed {
        println!("契约生成物与声明一致.");
    }
    failed
}

fn report_difference(output: &Output, actual: &str) {
    let generated: Vec<&str> = output.content.lines().collect();
    let found: Vec<&str> = actual.lines().collect();
    let limit = generated.len().max(found.len());
    let line = (0..limit)
        .find(|index| generated.get(*index) != found.get(*index))
        .unwrap_or(0);
    eprintln!(
        "{}: 生成物与声明不一致, 第一处差异在第 {} 行",
        output.path,
        line + 1
    );
    eprintln!(
        "  生成: {}",
        generated
            .get(line)
            .map(|line| line.to_string())
            .unwrap_or_default()
    );
    eprintln!(
        "  实际: {}",
        found
            .get(line)
            .map(|line| line.to_string())
            .unwrap_or_default()
    );
    eprintln!("请运行 cargo run -p contract-gen 重新生成并提交.");
}

/// `launcher/` 目录, 生成物的相对路径都从这里算.
pub fn launcher_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_first_differing_line() {
        let output = Output {
            path: "x.ts",
            content: "a\nb\nc\n".into(),
        };
        let generated: Vec<&str> = output.content.lines().collect();
        let found: Vec<&str> = "a\nx\nc\n".lines().collect();
        let line = (0..generated.len().max(found.len()))
            .find(|index| generated.get(*index) != found.get(*index))
            .unwrap();
        assert_eq!(line, 1);
    }

    #[test]
    fn check_mode_detects_stale_outputs() {
        let root = std::env::temp_dir().join(format!("contract-gen-check-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("创建临时目录");
        let output = Output {
            path: "generated.txt",
            content: "a\n".into(),
        };
        std::fs::write(root.join("generated.txt"), "a\n").expect("写入生成物");
        assert!(!check_outputs(&root, std::slice::from_ref(&output)));
        std::fs::write(root.join("generated.txt"), "b\n").expect("改写生成物");
        assert!(check_outputs(&root, std::slice::from_ref(&output)));
        std::fs::remove_dir_all(&root).expect("清理临时目录");
    }

    #[test]
    fn launcher_root_points_at_the_workspace_root() {
        let root = launcher_root();
        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join("crates/oddity-contract/src/lib.rs").is_file());
    }
}
