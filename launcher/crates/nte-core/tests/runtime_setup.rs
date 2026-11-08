#[allow(dead_code)]
#[path = "../src/testing.rs"]
mod testing;
use nte_core::runtime_setup::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use testing::{EnvGuard, HttpServer, TempDir};

#[test]
fn downloads_verify_before_publishing_and_reuse_verified_cache() {
    let mut env = EnvGuard::new();
    env.set("NO_PROXY", "127.0.0.1,localhost");
    let server = HttpServer::start();
    server.route("/component", b"runtime");
    let dir = TempDir::new("runtime-download");
    let target = dir.join("component.tar");
    let hash: String = Sha256::digest(b"runtime")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    download(&server.url("/component"), &hash, &target, &mut |_| {}).unwrap();
    download(&server.url("/component"), &hash, &target, &mut |_| {}).unwrap();
    assert_eq!(server.targets().len(), 1);
    fs::write(&target, "corrupt").unwrap();
    download(&server.url("/component"), &hash, &target, &mut |_| {}).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"runtime");
    fs::remove_file(&target).unwrap();
    assert!(download(
        &server.url("/component"),
        &"0".repeat(64),
        &target,
        &mut |_| {}
    )
    .is_err());
    assert!(!target.exists());
    assert!(!target.with_extension("part").exists());
}

#[test]
fn steam_directory_respects_existing_root_and_does_not_require_steam() {
    let dir = TempDir::new("runtime-steam");
    assert_eq!(
        steam_tools(dir.path()),
        dir.join(".local/share/Steam/compatibilitytools.d")
    );
    fs::create_dir_all(dir.join("custom-steam")).unwrap();
    fs::create_dir_all(dir.join(".steam")).unwrap();
    std::os::unix::fs::symlink(dir.join("custom-steam"), dir.join(".steam/root")).unwrap();
    assert_eq!(
        steam_tools(dir.path()).canonicalize().err().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        steam_tools(dir.path()),
        dir.join(".steam/root/compatibilitytools.d")
    );
}

#[test]
fn system_commands_use_distribution_package_manager_without_enabling_repositories() {
    assert_eq!(
        system_install("ID=cachyos\nID_LIKE=arch").unwrap(),
        [
            "/usr/bin/pacman",
            "-S",
            "--needed",
            "--noconfirm",
            "umu-launcher"
        ]
    );
    assert_eq!(
        system_install("ID=linuxmint\nID_LIKE=\"ubuntu debian\"").unwrap()[0],
        "/usr/bin/apt-get"
    );
    assert!(system_install("ID=nixos").is_err());
}

#[test]
fn preparation_checks_only_dependencies_needed_for_the_installation() {
    let mut env = EnvGuard::new();
    let directory = TempDir::new("runtime-dependencies");
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).unwrap();
    env.set("PATH", &bin);
    let error = plan(directory.path(), directory.path(), false, None, "ID=arch").unwrap_err();
    assert!(error.to_string().contains("缺少 Python"));

    let python = directory.write("bin/python3", "#!/bin/sh\nexit 0\n");
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    let user = plan(directory.path(), directory.path(), false, None, "ID=arch").unwrap();
    assert!(user.install_umu);
    assert!(user.system_command.is_none());
    fs::write(&python, "#!/bin/sh\nexit 1\n").unwrap();
    assert!(
        plan(directory.path(), directory.path(), false, None, "ID=arch")
            .unwrap_err()
            .to_string()
            .contains("Python 3.10")
    );
    fs::remove_file(python).unwrap();

    let umu = directory.write("bin/umu-run", "#!/bin/sh\nexit 0\n");
    fs::set_permissions(&umu, fs::Permissions::from_mode(0o755)).unwrap();
    let reused = plan(directory.path(), directory.path(), false, None, "ID=arch").unwrap();
    assert_eq!(reused.umu, umu);
    assert!(!reused.install_umu);
    assert!(reused.system_command.is_none());
    let system = plan(directory.path(), directory.path(), true, None, "ID=arch");
    if system_umu().is_some() {
        assert!(!system.unwrap().install_umu);
    } else {
        assert!(system.unwrap_err().to_string().contains("缺少 pkexec"));
    }
}

#[test]
fn preparation_uses_normal_user_and_records_proton_only_after_success() {
    let dir = TempDir::new("runtime prepare with spaces");
    let script = dir.write("umu-run", "#!/bin/sh\n[ \"$1\" = --version ] && exit 0\n[ \"$1\" = wineboot ] && [ \"$2\" = -u ] && [ \"$PROTON_VERB\" = waitforexitandrun ] || exit 12\nprintf '%s' \"$PROTONPATH\" > \"$WINEPREFIX/initialized\"\nprintf registry > \"$WINEPREFIX/system.reg\"\n");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let plan = Plan {
        umu: script,
        proton: dir.join("Proton"),
        install_umu: false,
        install_proton: false,
        system_command: None,
        umu_version: "test".into(),
        log: dir.join("logs/setup.log"),
    };
    dir.write("Proton/proton", "proton");
    dir.write("Proton/toolmanifest.vdf", "manifest");
    let mut stages = Vec::new();
    install(dir.path(), &plan, |p| stages.push(p.phase)).unwrap();
    assert!(stages.contains(&"runtime".into()));
    let receipt = Receipt::load(dir.path()).unwrap();
    assert!(receipt.seen);
    assert_eq!(receipt.proton, Some(plan.proton));
    let prefix = dir.join("runtime");
    assert_eq!(
        fs::read_dir(prefix).unwrap().count(),
        2,
        "临时初始化 prefix 应被清理"
    );
}

#[test]
fn rejected_authorization_does_not_mark_setup_complete() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("runtime-pkexec");
    let script = dir.write("bin/pkexec", "#!/bin/sh\nexit 126\n");
    fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    env.set("PATH", dir.join("bin"));
    let plan = Plan {
        umu: dir.join("umu-run"),
        proton: dir.join("Proton"),
        install_umu: true,
        install_proton: true,
        system_command: Some(vec![
            "/usr/bin/pacman".into(),
            "-S".into(),
            "umu-launcher".into(),
        ]),
        umu_version: "system".into(),
        log: dir.join("logs/setup.log"),
    };
    let error = install(dir.path(), &plan, |_| {}).unwrap_err();
    assert!(error.to_string().contains("取消或被拒绝"));
    assert!(!Receipt::load(dir.path()).unwrap().seen);
    assert!(!plan.proton.exists());
}

#[test]
fn initialization_requires_successful_wineboot_and_a_registry() {
    let directory = TempDir::new("wineboot-result");
    for (name, body, message) in [
        (
            "failed",
            "printf registry > \"$WINEPREFIX/system.reg\"\nexit 9",
            "退出码 9",
        ),
        ("incomplete", "exit 0", "未生成 Windows 注册表"),
    ] {
        let prefix = directory.join(name);
        fs::create_dir_all(&prefix).unwrap();
        let umu = directory.write(format!("umu-{name}"), format!("#!/bin/sh\n{body}\n"));
        fs::set_permissions(&umu, fs::Permissions::from_mode(0o755)).unwrap();
        let error = initialize(
            &umu,
            &directory.join("Proton"),
            &prefix,
            &directory.join("setup.log"),
        )
        .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
#[ignore = "需设置 ODDITY_TEST_UMU 和 ODDITY_TEST_PROTON, 在隔离目录验证本机运行时"]
fn real_runtime_initializes_an_isolated_prefix() {
    let umu = std::path::PathBuf::from(
        std::env::var_os("ODDITY_TEST_UMU").expect("设置 ODDITY_TEST_UMU"),
    );
    let proton = std::path::PathBuf::from(
        std::env::var_os("ODDITY_TEST_PROTON").expect("设置 ODDITY_TEST_PROTON"),
    );
    let directory = TempDir::new("real runtime with spaces");
    let plan = Plan {
        umu,
        proton: proton.clone(),
        install_umu: false,
        install_proton: false,
        system_command: None,
        umu_version: "local".into(),
        log: directory.join("logs/runtime.log"),
    };
    let mut stages = Vec::new();
    install(directory.path(), &plan, |progress| {
        stages.push(progress.phase)
    })
    .unwrap_or_else(|error| {
        panic!(
            "{error:#}\n{}",
            String::from_utf8_lossy(&fs::read(&plan.log).unwrap())
        )
    });
    let receipt = Receipt::load(directory.path()).unwrap();
    assert!(receipt.seen);
    assert_eq!(receipt.proton, Some(proton));
    assert!(stages.contains(&"runtime".to_string()));
    let prefix = directory.join("registered prefix with spaces");
    fs::create_dir_all(prefix.join("drive_c")).unwrap();
    initialize(&plan.umu, &plan.proton, &prefix, &plan.log).unwrap();
    assert!(prefix.join("system.reg").is_file());
}
