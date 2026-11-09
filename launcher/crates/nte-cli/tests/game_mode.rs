use std::fs;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};

#[test]
fn privileged_configuration_commands_reject_unprivileged_or_empty_input() {
    // 空标准输入保证即使测试以 root 运行, 也不会写入系统配置.
    let root = fs::metadata("/proc/self").unwrap().uid() == 0;
    let bin = std::env::var("CARGO_BIN_EXE_nte-cli").expect("CARGO_BIN_EXE_nte-cli");
    for operation in ["game-mode-gpu-apply", "game-mode-tuning-apply"] {
        let output = Command::new(&bin)
            .arg(operation)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains(if root {
                "配置无效"
            } else {
                "需要管理员权限"
            }),
            "{operation}: {error}"
        );
        let output = Command::new(&bin)
            .args([operation, "--unexpected"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("missing --root"));
    }
}
