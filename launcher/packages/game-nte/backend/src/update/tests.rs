//! 更新域测试: check_update 命令载荷与官方启动器自检流程.

use super::*;
use crate::testing::{block_on, HttpServer, TempDir, TestHost};
use serde_json::json;

/// 一个最小可用的安装目录树: <root>/install/NTELauncher/...
struct Install {
    dir: TempDir,
    launcher: PathBuf,
}

impl Install {
    fn new(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let launcher = dir.join("install/NTELauncher");
        std::fs::create_dir_all(&launcher).expect("create launcher dir");
        Self { dir, launcher }
    }

    fn install_root(&self) -> PathBuf {
        self.dir.join("install")
    }

    fn launcher_config(&self, body: &str) {
        self.dir
            .write("install/NTELauncher/Config/Config.ini", body);
    }

    fn host(&self) -> TestHost {
        let mut host = TestHost::new(Some(self.launcher.clone()));
        host.install_root = Some(self.install_root().display().to_string());
        host
    }

    fn check(&self, distribution: &str, full: bool) -> Result<CheckResult, String> {
        check_impl(&self.launcher, &self.install_root(), distribution, full)
    }
}

fn all_files(entries: &[(&str, u64, &str)]) -> String {
    let mut xml = String::from(
        "<AllFiles><ProductVersion Version=\"1.4.20\"/><Url BaseUrl=\"http://cdn.test/\"/>",
    );
    for (path, size, checksum) in entries {
        xml.push_str(&format!(
            "<File Path=\"{path}\" Checksum=\"{checksum}\" Size=\"{size}\"/>"
        ));
    }
    xml.push_str("</AllFiles>");
    xml
}

/// 测试里只需要一个稳定的 md5 十六进制串.
fn md5_hex(bytes: &[u8]) -> String {
    oddity_platform::hash::md5_bytes(bytes)
}

/// 发布一份可用的 Version.ini/AllFiles.xml, 返回远端版本.
fn publish(install: &Install, server: &HttpServer, manifest: &str) -> String {
    install.launcher_config(&format!(
        "[VERSION]\nVersion=1.4.16\nVersionInfoFileURL={}\n",
        server.url("/Version.ini")
    ));
    server.route(
        "/Version.ini",
        format!(
            "[VERSION]\nVersion=1.4.20\nBuild=20240102\nFileListURL={}\n",
            server.url("/AllFiles.xml")
        ),
    );
    server.route("/AllFiles.xml", manifest);
    "1.4.20".to_string()
}

#[test]
fn check_update_returns_every_check_result_field() {
    let server = HttpServer::start();
    let install = Install::new("check-update");
    let content = b"hello";
    install
        .dir
        .write("install/NTELauncher/data/ok.txt", content);
    install
        .dir
        .write("install/NTELauncher/data/changed.txt", b"hi");
    let remote = publish(
        &install,
        &server,
        &all_files(&[
            ("/data/ok.txt", content.len() as u64, &md5_hex(content)),
            ("/data/changed.txt", 9, "00"),
            ("/missing.txt", 7, "11"),
        ]),
    );
    assert_eq!(remote, "1.4.20");

    let host = install.host();
    let value = block_on(command(&host, "check_update", json!({"full": false})))
        .expect("check_update 应返回自检结果");
    assert_eq!(value.as_object().expect("object").len(), 14);
    assert_eq!(value["installed"], json!(true));
    assert_eq!(value["distribution"], json!("cn"));
    assert_eq!(
        value["installRoot"],
        json!(install.install_root().display().to_string())
    );
    assert_eq!(value["remoteVersion"], json!("1.4.20"));
    assert_eq!(value["localVersion"], json!("1.4.16"));
    assert_eq!(value["productVersion"], json!("1.4.20"));
    assert_eq!(value["files"], json!(3));
    assert_eq!(value["ok"], json!(1));
    assert_eq!(value["missing"], json!(1));
    assert_eq!(value["changed"], json!(1));
    assert_eq!(value["bytes"], json!(16));
    assert_eq!(value["missingPreview"], json!(["/missing.txt"]));
    assert_eq!(value["changedPreview"], json!(["/data/changed.txt"]));
    // 耗时随请求变化, 只断言字段类型; 其余字段逐项固定.
    assert!(value["durationMs"].is_u64(), "{value}");
    // Version.ini 请求带上了时间戳参数.
    assert!(
        server
            .targets()
            .iter()
            .any(|target| target.starts_with("/Version.ini?t=")),
        "{:?}",
        server.targets()
    );
}

#[test]
fn check_update_reports_a_missing_launcher_as_not_installed() {
    let dir = TempDir::new("check-update-missing");
    let mut host = TestHost::new(None);
    host.install_root = Some(dir.path().display().to_string());

    let value = block_on(command(&host, "check_update", json!({"full": true})))
        .expect("未安装时仍返回自检结果");
    assert_eq!(value["installed"], json!(false));
    assert_eq!(value["distribution"], json!("cn"));
    assert_eq!(
        value["installRoot"],
        json!(dir.path().display().to_string())
    );
    for field in ["remoteVersion", "localVersion", "productVersion"] {
        assert_eq!(value[field], json!(""), "{field}");
    }
    for field in ["missingPreview", "changedPreview"] {
        assert_eq!(value[field], json!([]), "{field}");
    }
    for field in ["files", "ok", "missing", "changed", "bytes", "durationMs"] {
        assert_eq!(value[field], json!(0), "{field}");
    }
}

#[test]
fn check_update_reports_another_game_as_not_installed() {
    let mut host = TestHost::new(None);
    host.game = "hsr".into();

    let value = block_on(command(&host, "check_update", json!({"full": false})))
        .expect("其他游戏按未安装处理");
    assert_eq!(value["installed"], json!(false));
    assert_eq!(value["distribution"], json!("cn"));
    // 宿主没有其他游戏的安装域, 安装根保持空串.
    assert_eq!(value["installRoot"], json!(""));
}

#[test]
fn check_update_refuses_while_the_install_directory_is_moving() {
    for (busy, repair) in [(true, false), (false, true)] {
        let mut host = TestHost::new(None);
        host.activity.migration_busy = busy;
        host.activity.migration_repair_pending = repair;
        let error = block_on(command(&host, "check_update", json!({"full": false})))
            .expect_err("迁移期间应拒绝自检");
        assert_eq!(error, json!("安装目录迁移正在进行"));
    }
}

#[test]
fn check_update_rejects_unknown_commands_and_payloads() {
    let host = TestHost::new(None);
    for payload in [
        json!(false),
        json!({"full": 1}),
        json!({"full": false, "x": 1}),
    ] {
        let error = block_on(command(&host, "check_update", payload)).expect_err("参数无效");
        assert!(error.is_string(), "{error}");
    }
    let error = block_on(command(&host, "unknown", json!({}))).expect_err("未知命令");
    assert_eq!(error, json!("无法识别的更新命令"));
}

#[test]
fn check_impl_reports_the_local_state_against_the_manifest() {
    let server = HttpServer::start();
    let install = Install::new("check-impl");
    let content = b"hello";
    install
        .dir
        .write("install/NTELauncher/data/ok.txt", content);
    install
        .dir
        .write("install/NTELauncher/data/changed.txt", b"hi");
    publish(
        &install,
        &server,
        &all_files(&[
            ("/data/ok.txt", content.len() as u64, &md5_hex(content)),
            ("/data/changed.txt", 9, "00"),
            ("/missing.txt", 7, "11"),
        ]),
    );

    let quick = install.check("cn", false).expect("quick check");
    assert!(quick.installed);
    assert_eq!(quick.distribution, "cn");
    assert_eq!(
        quick.install_root,
        install.install_root().display().to_string()
    );
    assert_eq!(quick.remote_version, "1.4.20");
    assert_eq!(quick.local_version, "1.4.16");
    assert_eq!(quick.product_version, "1.4.20");
    assert_eq!(quick.files, 3);
    assert_eq!(quick.ok, 1);
    assert_eq!(quick.missing, 1);
    assert_eq!(quick.changed, 1);
    assert_eq!(quick.bytes, 16);
    assert_eq!(quick.missing_preview, vec!["/missing.txt"]);
    assert_eq!(quick.changed_preview, vec!["/data/changed.txt"]);

    // 完整校验会重新算 md5，结果一致
    let full = install.check("cn", true).expect("full check");
    assert_eq!(full.ok, 1);
    assert_eq!(full.missing, 1);
    assert_eq!(full.changed, 1);
    assert_eq!(full.files, 3);
}

#[test]
fn check_impl_truncates_the_preview() {
    let server = HttpServer::start();
    let install = Install::new("check-impl-preview");
    let entries: Vec<(String, u64, String)> = (0..15)
        .map(|index| (format!("/missing-{index}.txt"), 1, "00".to_string()))
        .collect();
    let borrowed: Vec<(&str, u64, &str)> = entries
        .iter()
        .map(|(path, size, checksum)| (path.as_str(), *size, checksum.as_str()))
        .collect();
    publish(&install, &server, &all_files(&borrowed));

    let result = install.check("cn", false).expect("check");
    assert_eq!(result.files, 15);
    assert_eq!(result.missing, 15);
    assert_eq!(result.missing_preview.len(), 12);
    assert!(result.changed_preview.is_empty());
}

#[test]
fn check_impl_reports_every_failure_step() {
    let server = HttpServer::start();
    let install = Install::new("check-impl-errors");

    // 启动器配置读不出来
    let error = install.check("cn", false).expect_err("no config");
    assert!(error.contains("读取启动器配置失败"), "{error}");

    // Version.ini 取不到
    install.launcher_config(&format!(
        "[VERSION]\nVersion=1.4.16\nVersionInfoFileURL={}\n",
        server.url("/Version.ini")
    ));
    let error = install.check("cn", false).expect_err("no version");
    assert!(error.contains("拉取 Version.ini 失败"), "{error}");

    // 远端缺少 FileListURL
    server.route("/Version.ini", "[VERSION]\nVersion=1.4.20\n");
    let error = install.check("cn", false).expect_err("no file list");
    assert!(error.contains("缺少 FileListURL"), "{error}");

    // AllFiles.xml 取不到
    server.route(
        "/Version.ini",
        format!(
            "[VERSION]\nVersion=1.4.20\nFileListURL={}\n",
            server.url("/AllFiles.xml")
        ),
    );
    let error = install.check("cn", false).expect_err("no manifest");
    assert!(error.contains("拉取 AllFiles.xml 失败"), "{error}");

    // 清单里没有文件同样算失败
    server.route("/AllFiles.xml", "<AllFiles/>");
    let error = install.check("cn", false).expect_err("empty manifest");
    assert!(error.contains("拉取 AllFiles.xml 失败"), "{error}");
}
