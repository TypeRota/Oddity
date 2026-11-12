//! 无头更新命令的测试.
//!
//! 这些函数只往标准输出打印进度, 断言集中在返回值与文件系统结果上;
//! 网络请求交给回环 HTTP 服务.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::*;
use crate::testing::{HttpServer, TempDir};

fn write_file(root: &Path, rel: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("创建父目录");
    }
    fs::write(&path, contents).expect("写入文件");
    path
}

fn launcher_config(root: &Path, version_url: &str) -> LauncherConfig {
    LauncherConfig {
        root: root.to_path_buf(),
        launcher: "NTELauncher.exe".to_string(),
        patcher: "NTEUpdate.exe".to_string(),
        client: "NTEGame.exe".to_string(),
        game_id: 1289,
        launch_type: 0,
        version: "1.4.16".to_string(),
        build: "20240101".to_string(),
        version_url: version_url.to_string(),
        backup_version_url: "https://cdn.test/Backup.ini".to_string(),
        branch: "publish_ob".to_string(),
        patcher_ignored_sections: Vec::new(),
        mutex_name: "nte-mutex".to_string(),
    }
}

/// 一个最小可用的 Patcher 配置目录.
fn game_fixture(dir: &TempDir, launcher: &str) {
    write_file(
        &dir.join(launcher),
        "ResFilesM/1289/PatcherConfig/PatcherConfig.json",
        "{\"gameResUrl\":[\"{cdn}\"],\"branchName\":\"publish_ob\",\"appId\":\"1289\"}",
    );
}

fn rewrite_cdn(root: &Path, cdn: &str) {
    write_file(
        root,
        "ResFilesM/1289/PatcherConfig/PatcherConfig.json",
        format!("{{\"gameResUrl\":[\"{cdn}\"],\"branchName\":\"publish_ob\",\"appId\":\"1289\"}}"),
    );
}

fn zip_one(name: &str, contents: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(name, zip::write::SimpleFileOptions::default())
        .expect("start zip entry");
    writer.write_all(contents).expect("write zip entry");
    writer.finish().expect("finish zip").into_inner()
}

fn res_zip(res_list: &str, diff: Option<&str>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("ResList.bin", zip::write::SimpleFileOptions::default())
        .expect("start entry");
    writer
        .write_all(&crate::update::pxml::encode(res_list.as_bytes(), "1289").expect("encode"))
        .expect("write entry");
    if let Some(diff) = diff {
        writer
            .start_file("LastDiff.bin", zip::write::SimpleFileOptions::default())
            .expect("start entry");
        writer
            .write_all(&crate::update::pxml::encode(diff.as_bytes(), "1289").expect("encode"))
            .expect("write entry");
    }
    writer.finish().expect("finish zip").into_inner()
}

fn game_config_xml(res_version: &str, res_list: &str, diff_hash: Option<&str>) -> String {
    let mut xml = format!(
        "<Config><ResVersion>{res_version}</ResVersion><Hash>HASH</Hash><Tag>baseTag</Tag><listHash>{}</listHash>",
        oddity_platform::hash::md5_bytes(res_list.as_bytes())
    );
    if let Some(hash) = diff_hash {
        xml.push_str(&format!("<diffHash>{hash}</diffHash>"));
    }
    xml.push_str("</Config>");
    xml
}

// ---------------------------------------------------------------------------
// 参数与本地检查
// ---------------------------------------------------------------------------

#[test]
fn usage_names_every_command() {
    for command in HEADLESS_COMMANDS {
        assert!(
            USAGE.iter().any(|line| line.contains(command.name)),
            "用法里缺少 {}",
            command.name
        );
    }
    assert!(USAGE.iter().any(|line| line.contains("--game <游戏 id>")));
}

#[test]
fn headless_cli_exposes_the_command_surface() {
    let cli = headless_cli();
    assert_eq!(cli.commands(), HEADLESS_COMMANDS);
    assert_eq!(cli.usage(), USAGE);

    let updates: Vec<&str> = cli
        .commands()
        .iter()
        .filter(|command| command.needs_root)
        .map(|command| command.name)
        .collect();
    assert_eq!(
        updates,
        [
            "check",
            "apply",
            "doctor",
            "game-resources",
            "game-check",
            "game-apply",
            "game-preload",
        ]
    );
    let standalone: Vec<&str> = cli
        .commands()
        .iter()
        .filter(|command| !command.needs_root)
        .map(|command| command.name)
        .collect();
    assert_eq!(standalone, ["attendance", "keyring-check"]);
}

#[test]
fn headless_cli_keeps_its_error_text() {
    let cli = headless_cli();
    let dir = TempDir::new("headless-attendance");
    let args = vec!["--config".to_string(), dir.path().display().to_string()];
    let request = HeadlessRequest {
        command: "attendance",
        root: None,
        args: &args,
    };
    let error = cli.run(&request).expect_err("没有签到账号时应失败");
    assert!(error.contains("请先登录塔吉多账号"), "{error}");

    let request = HeadlessRequest {
        command: "doctor",
        root: None,
        args: &[],
    };
    let error = cli.run(&request).expect_err("缺少启动器目录时应失败");
    assert_eq!(error, "命令 doctor 缺少启动器目录");
}

#[test]
fn run_dispatches_by_command_name() {
    let server = HttpServer::start();
    let install = game_install("run-dispatch");
    // run 从启动器目录读 Config.ini, 与命令行入口一致.
    write_file(
        &install.launcher_root,
        "Config/Config.ini",
        "[General]\nGameID=1289\n[VERSION]\nVersion=1.4.16\n",
    );
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    install.resource("data/present.bin", b"hello");

    let res_list = format!(
        "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/present.bin\" md5=\"{}\" filesize=\"5\"/></ResList>",
        oddity_platform::hash::md5_bytes(b"hello")
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", &res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(&res_list, None),
    );

    run("game-check", &install.launcher_root, &[]).expect("game-check");
    run("doctor", &install.launcher_root, &[]).expect("doctor");
    run(
        "game-apply",
        &install.launcher_root,
        &["--dry-run".to_string()],
    )
    .expect("game-apply");

    let error = run("not-a-command", &install.launcher_root, &[]).unwrap_err();
    assert!(error.to_string().contains("无法识别的更新命令"), "{error}");

    // 启动器配置读不到时, 错误仍定位到同一个文件.
    let missing = TempDir::new("run-missing");
    let error = run("doctor", missing.path(), &[]).unwrap_err();
    assert!(
        format!("{error:#}").contains("Config/Config.ini"),
        "{error:#}"
    );
}

#[test]
fn doctor_prints_the_launcher_layout() {
    let dir = TempDir::new("doctor");
    let config = launcher_config(dir.path(), "https://cdn.test/Version.ini");
    doctor(&config).expect("doctor");
}

#[test]
fn game_config_reports_a_missing_patcher_directory() {
    let dir = TempDir::new("game-config");
    let config = launcher_config(dir.path(), "https://cdn.test/Version.ini");
    let error = game_config(&config).expect_err("no PatcherConfig");
    assert!(
        format!("{error:#}").contains("load PatcherConfig"),
        "{error:#}"
    );
}

#[test]
fn check_reports_the_local_and_remote_versions() {
    let dir = TempDir::new("check");
    let server = HttpServer::start();
    write_file(dir.path(), "data/ok.txt", b"hello");
    server.route(
        "/Version.ini",
        format!(
            "[VERSION]\nVersion=1.4.20\nBuild=20240102\nFileListURL={}\n",
            server.url("/files/AllFiles.xml")
        ),
    );
    server.route(
        "/files/AllFiles.xml",
        format!(
            "<AllFiles><ProductVersion Version=\"1.4.20\"/><Url BaseUrl=\"{}/files/\"/><File Path=\"/data/ok.txt\" Checksum=\"{}\" Size=\"5\"/><File Path=\"/data/gone.txt\" Checksum=\"00\" Size=\"3\"/></AllFiles>",
            server.base(),
            oddity_platform::hash::md5_bytes(b"hello")
        ),
    );
    let config = launcher_config(dir.path(), &server.url("/Version.ini"));

    check(&config, true).expect("quick check");
    check(&config, false).expect("full check");
    assert!(!dir.path().join("data/gone.txt").exists());
}

#[test]
fn check_reports_fetch_failures() {
    let dir = TempDir::new("check-errors");
    let server = HttpServer::start();

    // Version.ini 取不到
    let config = launcher_config(dir.path(), &server.url("/Version.ini"));
    let error = check(&config, true).expect_err("no version");
    assert!(
        format!("{error:#}").contains("fetch Version.ini"),
        "{error:#}"
    );

    // 远端缺少 FileListURL
    server.route("/Version.ini", "[VERSION]\nVersion=1.4.20\n");
    let error = check(&config, true).expect_err("no file list");
    assert!(
        format!("{error:#}").contains("has no FileListURL"),
        "{error:#}"
    );

    // AllFiles.xml 取不到
    server.route(
        "/Version.ini",
        format!(
            "[VERSION]\nVersion=1.4.20\nFileListURL={}\n",
            server.url("/AllFiles.xml")
        ),
    );
    let error = check(&config, true).expect_err("no manifest");
    assert!(
        format!("{error:#}").contains("fetch AllFiles.xml"),
        "{error:#}"
    );
}

#[test]
fn apply_downloads_missing_files() {
    let install = TempDir::new("apply");
    let root = install.join("NTELauncher");
    fs::create_dir_all(&root).unwrap();
    let server = HttpServer::start();
    let content = b"new content";
    let zip = zip_one("data/new.txt", content);
    server.route("/files/data/new.txt.zip", &zip);
    server.route(
        "/Version.ini",
        format!(
            "[VERSION]\nVersion=1.4.20\nFileListURL={}\n",
            server.url("/files/AllFiles.xml")
        ),
    );
    server.route(
        "/files/AllFiles.xml",
        format!(
            "<AllFiles><File Path=\"/data/new.txt\" Checksum=\"{}\" Size=\"{}\"/><File Path=\"/data/skip.txt\" Checksum=\"{}\" Size=\"{}\"/></AllFiles>",
            oddity_platform::hash::md5_bytes(content),
            content.len(),
            oddity_platform::hash::md5_bytes(b"kept"),
            4
        ),
    );
    write_file(&root, "data/skip.txt", b"evil");
    server.route(
        "/files/data/skip.txt.zip",
        zip_one("data/skip.txt", b"kept"),
    );
    let config = launcher_config(&root, &server.url("/Version.ini"));

    apply(&config, None).expect("apply");
    assert_eq!(
        fs::read(root.join("data/new.txt")).expect("read file"),
        content
    );

    assert_eq!(fs::read(root.join("data/skip.txt")).unwrap(), b"kept");

    // --only 过滤掉不匹配的文件
    let filtered = launcher_config(&root, &server.url("/Version.ini"));
    apply(&filtered, Some("nothing-matches".to_string())).expect("apply with filter");
}

#[test]
fn apply_reports_version_failures() {
    let install = TempDir::new("apply-errors");
    let root = install.join("NTELauncher");
    fs::create_dir_all(&root).unwrap();
    let server = HttpServer::start();
    let config = launcher_config(&root, &server.url("/Version.ini"));

    let error = apply(&config, None).expect_err("no version");
    assert!(
        format!("{error:#}").contains("fetch Version.ini"),
        "{error:#}"
    );

    server.route("/Version.ini", "[VERSION]\nVersion=1.4.20\n");
    let error = apply(&config, None).expect_err("no file list");
    assert!(
        format!("{error:#}").contains("has no FileListURL"),
        "{error:#}"
    );

    // 文件失败必须反馈给脚本调用方。
    server.route(
        "/Version.ini",
        format!(
            "[VERSION]\nVersion=1.4.20\nFileListURL={}\n",
            server.url("/files/AllFiles.xml")
        ),
    );
    server.route(
        "/files/AllFiles.xml",
        "<AllFiles><File Path=\"/data/gone.txt\" Checksum=\"00\" Size=\"3\"/></AllFiles>",
    );
    assert!(apply(&config, None)
        .unwrap_err()
        .to_string()
        .contains("1 launcher update file(s) failed"));
    assert!(!root.join("data/gone.txt").exists());
}

// ---------------------------------------------------------------------------
// 游戏资源更新
// ---------------------------------------------------------------------------

/// 游戏安装目录形态：<base>/install/NTELauncher，资源装在 <base>/install 下。
struct GameInstall {
    dir: TempDir,
    launcher_root: PathBuf,
}

fn game_install(tag: &str) -> GameInstall {
    let dir = TempDir::new(tag);
    let launcher_root = dir.join("install/NTELauncher");
    fs::create_dir_all(&launcher_root).expect("创建启动器目录");
    game_fixture(&dir, "install/NTELauncher");
    GameInstall { dir, launcher_root }
}

impl GameInstall {
    fn launcher(&self, version_url: &str) -> LauncherConfig {
        launcher_config(&self.launcher_root, version_url)
    }

    fn resource(&self, rel: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        write_file(&self.dir.join("install"), rel, contents)
    }

    fn resource_path(&self, rel: &str) -> PathBuf {
        self.dir.join("install").join(rel)
    }
}

#[test]
fn game_check_walks_the_remote_plan() {
    let server = HttpServer::start();
    let install = game_install("game-check");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    install.resource("data/present.bin", b"hello");

    let res_list = "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/present.bin\" md5=\"00\" filesize=\"5\"/><Res filename=\"data/gone.bin\" md5=\"11\" filesize=\"7\"/></ResList>";
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(res_list, None),
    );

    let launcher = install.launcher("https://cdn.test/Version.ini");
    game_check(&launcher, &[]).expect("game check");

    // --full 会重新哈希本地文件
    let full: Vec<String> = vec!["--full".to_string(), "--include-optional".to_string()];
    game_check(&launcher, &full).expect("full game check");

    // config.xml 请求带上了时间戳参数
    assert!(
        server
            .targets()
            .iter()
            .any(|target| target.starts_with("/cdn/publish_ob/Version/Windows/config.xml?tValue=")),
        "{:?}",
        server.targets()
    );
}

#[test]
fn game_check_reports_unverified_large_files() {
    let server = HttpServer::start();
    let install = game_install("game-check-unverified");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));

    // 超过 20 MiB 且没有本地账本：只上报待确认，不哈希
    let big = install.resource_path("data/big.pak");
    fs::create_dir_all(big.parent().expect("parent")).expect("创建目录");
    fs::File::create(&big)
        .expect("创建文件")
        .set_len(21 * 1024 * 1024)
        .expect("设置长度");

    let res_list = format!(
        "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/big.pak\" md5=\"{}\" filesize=\"{}\"/></ResList>",
        "cc".repeat(16),
        21 * 1024 * 1024
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", &res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(&res_list, None),
    );

    let launcher = install.launcher("https://cdn.test/Version.ini");
    game_check(&launcher, &[]).expect("game check");
    // 没有 --full 时大文件不会被下载
    assert!(!install.resource_path("data/big.pak.tmp").exists());
}

#[test]
fn game_check_fails_without_a_patcher_config() {
    let dir = TempDir::new("game-check-missing");
    let launcher = launcher_config(dir.path(), "https://cdn.test/Version.ini");
    let error = game_check(&launcher, &[]).expect_err("no config");
    assert!(
        format!("{error:#}").contains("load PatcherConfig"),
        "{error:#}"
    );
}

#[test]
fn game_preload_dry_run_and_download_leave_live_resources_unchanged() {
    let server = HttpServer::start();
    let install = game_install("game-preload");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    let launcher = install.launcher("https://cdn.test/Version.ini");
    server.route("/cdn/publish_ob/Version/Windows/config.xml", "<config><ResVersion>1.4.16</ResVersion><PreReleaseBranch>future</PreReleaseBranch><PreReleaseVersion>2.0</PreReleaseVersion><PreReleaseStatus>start</PreReleaseStatus></config>");
    let bytes = b"future content";
    let hash = oddity_platform::hash::md5_bytes(bytes);
    let list = format!("<ResList version=\"2.0\" tag=\"baseTag\"><Res filename=\"data/game.bin\" md5=\"{hash}\" filesize=\"{}\"/></ResList>", bytes.len());
    server.route(
        "/cdn/future/Version/Windows/version/2.0/config.xml",
        game_config_xml("2.0", &list, None),
    );
    server.route(
        "/cdn/future/Version/Windows/version/2.0/ResList.bin.zip",
        res_zip(&list, None),
    );
    let path = format!("/cdn/future/Res/{}/{}.{}", &hash[..1], hash, bytes.len());
    server.route(&path, bytes);
    install.resource("data/game.bin", b"current game");
    game_download(&launcher, &["--dry-run".into()], true).unwrap();
    assert!(!server.targets().contains(&path));
    assert!(!install
        .launcher_root
        .join("UserData/Patcher/PatcherSDK/preload")
        .exists());
    game_download(&launcher, &[], true).unwrap();
    assert_eq!(
        fs::read(install.resource_path("data/game.bin")).unwrap(),
        b"current game"
    );
    assert!(!install
        .launcher_root
        .join("UserData/Patcher/PatcherSDK/ResList.xml")
        .exists());
    assert_eq!(
        server
            .targets()
            .iter()
            .filter(|target| *target == &path)
            .count(),
        1
    );
    let marker = install
        .launcher_root
        .join("UserData/Patcher/PatcherSDK/preload/future/2.0/complete.json");
    fs::remove_file(&marker).unwrap();
    fs::create_dir(&marker).unwrap();
    let error = game_download(&launcher, &[], true).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("write preload completion marker"),
        "{error}"
    );
}

#[test]
fn game_apply_dry_run_and_filters() {
    let server = HttpServer::start();
    let install = game_install("game-apply-dry");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));

    let res_list = "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/gone.bin\" md5=\"11\" filesize=\"7\"/></ResList>";
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(res_list, None),
    );
    let launcher = install.launcher("https://cdn.test/Version.ini");

    let dry_run: Vec<String> = vec!["--dry-run".to_string()];
    game_apply(&launcher, &dry_run).expect("dry run");
    assert!(!install.resource_path("data/gone.bin").exists());

    // 过滤条件没有命中任何文件
    let filtered: Vec<String> = vec!["--only".to_string(), "nothing".to_string()];
    game_apply(&launcher, &filtered).expect("filtered apply");
    assert!(!install.resource_path("data/gone.bin").exists());
}

#[test]
fn game_apply_reports_nothing_to_do() {
    let server = HttpServer::start();
    let install = game_install("game-apply-empty");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    install.resource("data/present.bin", b"hello");

    let res_list = format!(
        "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/present.bin\" md5=\"{}\" filesize=\"5\"/></ResList>",
        oddity_platform::hash::md5_bytes(b"hello")
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", &res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(&res_list, None),
    );

    let launcher = install.launcher("https://cdn.test/Version.ini");
    game_apply(&launcher, &[]).expect("nothing to do");
    let cfg = game_config(&launcher).unwrap();
    assert!(crate::update::check::load_local_ledger(&cfg)
        .unwrap()
        .is_some());
}

#[test]
fn game_apply_downloads_and_updates_the_ledger() {
    let server = HttpServer::start();
    let install = game_install("game-apply");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    let content = b"fresh content";
    let md5 = oddity_platform::hash::md5_bytes(content);
    server.route(
        format!("/cdn/publish_ob/Res/{}/{md5}.{}", &md5[..1], content.len()),
        content,
    );

    let res_list = format!(
        "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/fresh.bin\" md5=\"{md5}\" filesize=\"{}\"/></ResList>",
        content.len()
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", &res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(&res_list, None),
    );

    let launcher = install.launcher("https://cdn.test/Version.ini");
    game_apply(&launcher, &[]).expect("apply");
    assert!(install.resource_path("data/fresh.bin").is_file());
    assert_eq!(
        fs::read(install.resource_path("data/fresh.bin")).expect("读取文件"),
        content
    );
    // 账本被写回，可以直接读出来
    assert!(install
        .launcher_root
        .join("UserData/Patcher/PatcherSDK/ResList.xml")
        .is_file());
}

#[test]
fn game_apply_fails_when_files_cannot_be_downloaded() {
    let server = HttpServer::start();
    let install = game_install("game-apply-failed");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));

    let res_list = "<ResList version=\"1.4.16\" tag=\"baseTag\"><Res filename=\"data/gone.bin\" md5=\"11\" filesize=\"7\"/></ResList>";
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(res_list, None),
    );

    let launcher = install.launcher("https://cdn.test/Version.ini");
    let error = game_apply(&launcher, &[]).expect_err("download failed");
    assert!(
        format!("{error:#}").contains("game update file(s) failed"),
        "{error:#}"
    );
}

#[test]
fn game_apply_reports_check_failures() {
    let server = HttpServer::start();
    let install = game_install("game-apply-check");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    let launcher = install.launcher("https://cdn.test/Version.ini");

    let error = game_apply(&launcher, &[]).expect_err("no cdn data");
    assert!(
        format!("{error:#}").contains("check game resources"),
        "{error:#}"
    );
}

#[test]
fn game_apply_verifies_unknown_files_and_repairs_corruption() {
    let server = HttpServer::start();
    let install = game_install("game-apply-unverified");
    rewrite_cdn(&install.launcher_root, &server.url("/cdn"));
    let content = vec![7u8; 21 * 1024 * 1024];
    let md5 = oddity_platform::hash::md5_bytes(&content);
    let path = "data/big.pak";
    install.resource(path, &content);
    let res_list = format!("<ResList version=\"1.4.16\"><Res filename=\"{path}\" md5=\"{md5}\" filesize=\"{}\"/></ResList>", content.len());
    server.route(
        "/cdn/publish_ob/Version/Windows/config.xml",
        game_config_xml("1.4.16", &res_list, None),
    );
    server.route(
        "/cdn/publish_ob/Version/Windows/version/1.4.16/ResList.bin.zip",
        res_zip(&res_list, None),
    );
    let launcher = install.launcher("https://cdn.test/Version.ini");
    game_apply(&launcher, &[]).expect("verify without download");
    let cfg = game_config(&launcher).unwrap();
    assert!(crate::update::check::load_local_ledger(&cfg)
        .unwrap()
        .is_some());
    assert!(!server
        .targets()
        .iter()
        .any(|target| target.contains("/Res/")));

    fs::remove_file(cfg.patcher_dir.join("ResList.xml")).unwrap();
    fs::write(install.resource_path(path), vec![0u8; content.len()]).unwrap();
    server.route(
        format!("/cdn/publish_ob/Res/{}/{md5}.{}", &md5[..1], content.len()),
        &content,
    );
    game_apply(&launcher, &[]).expect("repair unknown file");
    assert_eq!(
        oddity_platform::hash::md5_file(&install.resource_path(path)).unwrap(),
        md5
    );
    assert!(crate::update::check::load_local_ledger(&cfg)
        .unwrap()
        .is_some());
}

#[test]
fn installation_lock_blocks_both_cli_writers_before_network_requests() {
    let dir = TempDir::new("lock-cli-writers");
    let root = dir.join("NTELauncher");
    game_fixture(&dir, "NTELauncher");
    let server = HttpServer::start();
    rewrite_cdn(&root, &server.base());
    let config = launcher_config(&root, &server.url("/Version.ini"));
    let _lock = InstallLock::acquire(dir.path()).unwrap();
    assert!(apply(&config, None)
        .unwrap_err()
        .to_string()
        .contains("其他任务"));
    assert!(game_apply(&config, &[])
        .unwrap_err()
        .to_string()
        .contains("其他任务"));
}

#[test]
fn attendance_reports_missing_login_without_touching_the_keyring() {
    let dir = TempDir::new("attendance");
    let error = crate::attendance::run_manual(Some(dir.path().to_path_buf()))
        .expect_err("没有账号时手动签到应失败");
    assert!(error.to_string().contains("请先登录塔吉多账号"), "{error}");
}
