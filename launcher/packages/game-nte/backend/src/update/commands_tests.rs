use super::*;
use crate::testing::{HttpServer, TempDir};
use crate::update::launcher::ManifestFile;
use crate::update::region::{profile, Region};
use crate::update::testing::{bind, reset, serial, FakeHost};
use oddity_platform::hash::{md5_bytes, md5_file};
use oddity_runtime::SettingsSnapshot;
use std::io::{Cursor, Write};
use std::sync::{Arc, MutexGuard};

const EXE: &str = "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe";
const DATA: &str = "Client/WindowsNoEditor/HT/Content/data.pak";

fn zip(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(name, zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap().into_inner()
}

struct Fixture {
    dir: TempDir,
    server: HttpServer,
    settings: Settings,
    distribution: Region,
    files: Vec<ManifestFile>,
    data: Vec<u8>,
    host: Arc<FakeHost>,
    ctx: Ctx,
    _serial: MutexGuard<'static, ()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        reset();
    }
}

impl Fixture {
    fn new(distribution: Region, installed: bool) -> Self {
        let _serial = serial();
        reset();
        let dir = TempDir::new("download-task");
        let server = HttpServer::start();
        let root = profile(distribution).launcher_dir;
        let id = if distribution == Region::Cn {
            "1289"
        } else {
            "3000001"
        };
        let ini = format!(
            "[General]\nGameID={id}\nClient={}\n",
            profile(distribution).client_exe
        );
        let patcher = serde_json::json!({
            "gameResUrl": [server.base()], "branchName": "test", "appId": id
        })
        .to_string();
        let config_path = format!("ResFilesM/{id}/PatcherConfig/PatcherConfig.json");
        let mut files = Vec::new();
        for (path, content) in [
            ("Config/Config.ini", ini.as_bytes()),
            (&config_path, patcher.as_bytes()),
        ] {
            let bytes = zip(path, content);
            server.route(format!("/launcher/{path}.zip"), &bytes);
            files.push(ManifestFile {
                path: path.into(),
                checksum: md5_bytes(content),
                size: content.len() as u64,
                zip_size: Some(bytes.len() as u64),
                zip_checksum: Some(md5_bytes(&bytes)),
            });
            if installed {
                dir.write(format!("{root}/{path}"), content);
            }
        }
        let data = vec![23; 21 * 1024 * 1024];
        let exe = b"fixture game executable";
        let list = format!("<ResList version=\"1.0\" tag=\"baseTag\"><Res filename=\"{EXE}\" md5=\"{}\" filesize=\"{}\"/><Res filename=\"{DATA}\" md5=\"{}\" filesize=\"{}\"/></ResList>", md5_bytes(exe), exe.len(), md5_bytes(&data), data.len());
        server.route("/test/Version/Windows/config.xml", format!("<Config><ResVersion>1.0</ResVersion><Hash>hash</Hash><listHash>{}</listHash></Config>", md5_bytes(list.as_bytes())));
        let encoded = game::pxml::encode(list.as_bytes(), id).unwrap();
        server.route(
            "/test/Version/Windows/version/1.0/ResList.bin.zip",
            zip("ResList.bin", &encoded),
        );
        for bytes in [exe.as_slice(), data.as_slice()] {
            let hash = md5_bytes(bytes);
            server.route(
                format!("/test/Res/{}/{}.{}", &hash[..1], hash, bytes.len()),
                bytes,
            );
        }
        if installed {
            dir.write(EXE, exe);
            dir.write(DATA, &data);
        }
        let settings = Settings(SettingsSnapshot {
            game: "nte".into(),
            distribution: distribution.as_str().into(),
            language: "zh-CN".into(),
            reduce_motion: false,
            theme: "light".into(),
            install_root: Some(dir.path().display().to_string()),
            preload_root: None,
            update: Default::default(),
        });
        let host = FakeHost::new();
        host.set_settings(settings.0.clone());
        host.set_launcher_root(Some(dir.path().join(root)));
        bind(&host);
        let ctx = Ctx::new().expect("更新域上下文");
        Self {
            dir,
            server,
            settings,
            distribution,
            files,
            data,
            host,
            ctx,
            _serial,
        }
    }

    fn edit_settings(&mut self, change: impl FnOnce(&mut SettingsSnapshot)) {
        let mut settings = self.host.settings();
        change(&mut settings);
        self.host.set_settings(settings.clone());
        self.settings = Settings(settings);
        self.ctx = Ctx::new().expect("更新域上下文");
    }

    fn set_game(&mut self, game: &str) {
        let mut settings = self.host.settings();
        settings.game = game.into();
        self.host.set_settings(settings.clone());
        self.settings = Settings(settings);
        self.ctx = Ctx::new().expect("更新域上下文");
    }

    fn set_starting(&self, starting: bool) {
        let mut activity = self.host.activity().expect("activity");
        activity.game_starting = starting;
        self.host.set_activity(activity);
    }

    fn set_running(&self, running: bool) {
        let mut activity = self.host.activity().expect("activity");
        activity.game_running = running;
        self.host.set_activity(activity);
    }

    fn plan(&self, install: bool) -> LauncherPlan {
        LauncherPlan {
            root: self.dir.join(profile(self.distribution).launcher_dir),
            version: "1.0".into(),
            files: if install { self.files.clone() } else { vec![] },
            download_bytes: self.files.iter().map(|file| file.zip_size.unwrap()).sum(),
            zip_bases: vec![self.server.url("/launcher")],
        }
    }

    fn run(&self, mode: &str, full: bool, install: bool) -> Result<(), String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let (_, _, full) = prepare_task(
            &self.ctx,
            self.ctx.downloads(),
            mode,
            full,
            "manual",
            &cancel,
        )?;
        run_plan(
            &self.ctx,
            &self.settings,
            mode,
            full,
            &cancel,
            self.plan(install),
        )
    }

    fn select(&self, tags: Vec<String>) -> Result<ResourceCatalog, String> {
        let (settings, config, bundle) = load_catalog_context(&self.ctx)?;
        game::optional::save_selection(&config, &bundle, &tags)
            .map_err(|error| error.to_string())?;
        catalog_payload(&settings, &config, &bundle)
    }

    fn status(&self) -> DownloadStatus {
        game_download_status()
    }

    fn ledger(&self) {
        let id = if self.distribution == Region::Cn {
            "1289"
        } else {
            "3000001"
        };
        let xml = format!("<ResList version=\"1.0\" tag=\"baseTag\"><Res filename=\"{DATA}\" md5=\"{}\" filesize=\"{}\"/></ResList>", md5_bytes(&self.data), self.data.len());
        self.dir.write(
            format!(
                "{}/UserData/Patcher/PatcherSDK/ResList.xml",
                profile(self.distribution).launcher_dir
            ),
            game::pxml::encode(xml.as_bytes(), id).unwrap(),
        );
    }
}

#[test]
fn preload_marker_errors_are_returned_before_publishing_completion() {
    for download in [false, true] {
        let fixture = Fixture::new(Region::Cn, true);
        let root = fixture.dir.join("NTELauncher");
        let mut config = game::GameUpdateConfig::load(&root, 1289).unwrap();
        config.branch = "future".into();
        let content = b"future bytes";
        let hash = md5_bytes(content);
        fixture.server.route(
            format!("/future/Res/{}/{hash}.{}", &hash[..1], content.len()),
            content,
        );
        let mut plan = game::UpdatePlan {
            remote_version: "2.0".into(),
            ..Default::default()
        };
        if download {
            let res = game::manifest::ResEntry {
                filename: DATA.into(),
                filesize: content.len() as u64,
                md5: hash.clone(),
                ..Default::default()
            };
            plan.items.push(game::UpdateItem {
                path: DATA.into(),
                size: res.filesize,
                md5: hash,
                reason: game::check::UpdateReason::Size,
                kind: game::UpdateKind::Res { res },
                patches: vec![],
                download_size: content.len() as u64,
            });
        }
        // 目录占据标记文件名, 稳定复现最终写入失败.
        std::fs::create_dir_all(game::preload::root(&config).join("future/2.0/complete.json"))
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        prepare_task(
            &fixture.ctx,
            fixture.ctx.downloads(),
            "preload",
            false,
            "manual",
            &cancel,
        )
        .unwrap();
        let error = apply_game_plan(
            &fixture.ctx,
            &fixture.settings,
            &cancel,
            &root,
            &config,
            &plan,
            true,
        )
        .unwrap_err();
        assert!(error.contains("写入预下载完成记录失败"), "{error}");
        assert_ne!(fixture.status().phase, "preloaded");
        assert!(game::preload::completion(&config, "future", "2.0").is_none());
        assert_eq!(std::fs::read(fixture.dir.join(DATA)).unwrap(), fixture.data);
    }
}

#[test]
fn download_tasks_reject_games_without_an_install_recipe() {
    let mut fixture = Fixture::new(Region::Cn, true);
    fixture.set_game("hsr");
    let error = prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "check",
        false,
        "manual",
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap_err();
    assert!(error.contains("尚未支持"), "{error}");
}

#[test]
fn preload_disk_budget_uses_the_custom_cache_filesystem() {
    let mut fixture = Fixture::new(Region::Cn, true);
    let cache_root =
        std::path::Path::new("/dev/shm").join(format!("oddity-disk-test-{}", std::process::id()));
    fixture.edit_settings(|settings| {
        settings.preload_root = Some(cache_root.display().to_string());
    });
    let settings = fixture.settings.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    let root = fixture.dir.join("NTELauncher");
    let plan = game::UpdatePlan::default();
    let before = disk::available_bytes(&cache_root).unwrap();
    publish_game_plan(&settings, &root, &plan);
    let reported = fixture.status().disk_available_bytes.unwrap();
    let after = disk::available_bytes(&cache_root).unwrap();
    assert!((before.min(after)..=before.max(after)).contains(&reported));
    publish_cancelled();
    let reported = fixture.status().disk_available_bytes.unwrap();
    let final_space = disk::available_bytes(&cache_root).unwrap();
    assert!((after.min(final_space)..=after.max(final_space)).contains(&reported));
}

#[test]
fn preload_uses_existing_launcher_and_preserves_live_resources() {
    let fixture = Fixture::new(Region::Cn, true);
    fixture.ledger();
    let root = fixture.dir.join("NTELauncher");
    let ledger = root.join("UserData/Patcher/PatcherSDK/ResList.xml");
    let old_ledger = std::fs::read(&ledger).unwrap();
    let old_exe = std::fs::read(fixture.dir.join(EXE)).unwrap();
    let future = b"future game resource";
    let hash = md5_bytes(future);
    let list = format!("<ResList version=\"2.0\" tag=\"baseTag\"><Res filename=\"{DATA}\" md5=\"{hash}\" filesize=\"{}\"/></ResList>", future.len());
    fixture.server.route("/test/Version/Windows/config.xml", "<config><ResVersion>1.0</ResVersion><PreReleaseBranch>future</PreReleaseBranch><PreReleaseVersion>2.0</PreReleaseVersion><PreReleaseStatus>start</PreReleaseStatus></config>");
    fixture.server.route(
        "/future/Version/Windows/version/2.0/config.xml",
        format!(
            "<config><ResVersion>2.0</ResVersion><listHash>{}</listHash></config>",
            md5_bytes(list.as_bytes())
        ),
    );
    fixture.server.route(
        "/future/Version/Windows/version/2.0/ResList.bin.zip",
        zip(
            "ResList.bin",
            &game::pxml::encode(list.as_bytes(), "1289").unwrap(),
        ),
    );
    fixture.server.route(
        format!("/future/Res/{}/{}.{}", &hash[..1], hash, future.len()),
        future,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    // 此入口若执行启动器更新, 会访问未配置的官方启动器服务并失败.
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();
    let status = fixture.status();
    assert_eq!(status.mode, "preload");
    assert_eq!(status.phase, "preloaded");
    assert_eq!(status.remote_version, "2.0");
    assert_eq!(status.pending_files, 0);
    assert_eq!(std::fs::read(&ledger).unwrap(), old_ledger);
    assert_eq!(std::fs::read(fixture.dir.join(EXE)).unwrap(), old_exe);
    assert_eq!(std::fs::read(fixture.dir.join(DATA)).unwrap(), fixture.data);

    fixture.server.route(
        "/test/Version/Windows/config.xml",
        "<config><ResVersion>1.0</ResVersion></config>",
    );
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();
    // 公告消失后仍按缓存中较新的版本校验.
    assert_eq!(fixture.status().phase, "preloaded");
    assert_eq!(fixture.status().pending_files, 0);

    game_preload_delete(&fixture.ctx).unwrap();
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();
    // 缓存清理后没有公告或缓存目标, 预下载不可用.
    assert_eq!(fixture.status().phase, "preload-unavailable");
    assert_eq!(std::fs::read(&ledger).unwrap(), old_ledger);
}

#[test]
fn installs_from_an_empty_directory_for_both_regions() {
    for distribution in [Region::Cn, Region::Global] {
        let fixture = Fixture::new(distribution, false);
        fixture.run("check", false, true).unwrap();
        let launcher_disk = fixture.plan(true).required_disk_bytes();
        let status = fixture.status();
        assert_eq!(status.phase, "needs-install");
        assert!(!status.installed);
        assert_eq!(status.disk_required_bytes, launcher_disk);
        assert!(status.disk_available_bytes.is_some_and(|bytes| bytes > 0));
        fixture.run("apply", false, true).unwrap();
        let status = fixture.status();
        assert_eq!(status.phase, "ready");
        assert!(status.disk_required_bytes > launcher_disk);
        assert_eq!(status.pending_files, 0);
        assert_eq!(status.completed_files, status.total_files);
        assert!(status.current_file.is_empty());
        assert!(status.stage.is_empty());
        assert_eq!(
            md5_file(&fixture.dir.join(DATA)).unwrap(),
            md5_bytes(&fixture.data)
        );
        fixture.run("check", true, false).unwrap();
        assert_eq!(fixture.status().phase, "ready");
        assert!(fixture.status().installed);
    }
}

#[cfg(debug_assertions)]
#[test]
fn launcher_mode_repairs_same_size_launcher_content_changes() {
    let fixture = Fixture::new(Region::Cn, true);
    let relative = format!("{}/Config/Config.ini", profile(Region::Cn).launcher_dir);
    let original = fixture.dir.read(&relative);
    let mut corrupted = original.clone();
    corrupted[0] ^= 0xff;
    fixture.dir.write(&relative, &corrupted);

    let manifest_url = fixture.server.url("/launcher/AllFiles.xml");
    fixture.server.route(
        "/Version.ini",
        format!("[VERSION]\nVersion=1.0\nFileListURL={manifest_url}\n"),
    );
    let files: String = fixture
        .files
        .iter()
        .map(|file| {
            format!(
                r#"<File Path="{}" Size="{}" Checksum="{}" ZipChecksum="{}" ZipSize="{}"/>"#,
                file.path,
                file.size,
                file.checksum,
                file.zip_checksum.as_deref().unwrap_or_default(),
                file.zip_size.unwrap_or_default()
            )
        })
        .collect();
    fixture.server.route(
        "/launcher/AllFiles.xml",
        format!(r#"<AllFiles><ProductVersion Version="1.0"/>{files}</AllFiles>"#),
    );

    let mut env = crate::testing::EnvGuard::new();
    env.set(
        "ODDITY_LAUNCHER_VERSION_URL",
        fixture.server.url("/Version.ini"),
    );
    run_task(
        &fixture.ctx,
        &fixture.settings,
        "launcher",
        false,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(fixture.dir.read(&relative), original);
}

#[test]
fn launcher_mode_keeps_the_previous_full_check_for_game_resources() {
    let fixture = Fixture::new(Region::Cn, true);
    {
        let mut inner = state::manager().inner.lock().unwrap();
        inner.status = DownloadStatus {
            phase: "needs-update".into(),
            mode: "check".into(),
            distribution: "cn".into(),
            install_root: fixture.settings.current().install_root.clone().unwrap(),
            full: true,
            ..DownloadStatus::default()
        };
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (_, _, full) = prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "launcher",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    assert!(full);
}

#[test]
fn launcher_mode_fully_verifies_launcher_files() {
    // 同尺寸内容变更只能被完整校验发现, 启动器更新必须走完整校验.
    assert!(launcher_plan_full("launcher", false));
    assert!(launcher_plan_full("apply", true));
    assert!(!launcher_plan_full("check", false));
}

#[test]
fn launcher_mode_installs_launcher_files_without_downloading_game_resources() {
    let fixture = Fixture::new(Region::Cn, false);
    fixture.run("launcher", false, true).unwrap();
    let status = fixture.status();
    assert_eq!(status.mode, "launcher");
    assert_eq!(status.phase, "needs-update");
    assert!(fixture
        .dir
        .path()
        .join("NTELauncher/Config/Config.ini")
        .is_file());
    assert!(!fixture.dir.path().join(DATA).is_file());
    assert!(!fixture
        .server
        .targets()
        .iter()
        .any(|path| path.contains("/Res/")));
}

#[test]
fn a_full_check_is_preserved_until_corruption_is_repaired() {
    let fixture = Fixture::new(Region::Cn, true);
    fixture.ledger();
    fixture.dir.write(DATA, vec![9; fixture.data.len()]);
    fixture.run("check", true, false).unwrap();
    assert_eq!(fixture.status().phase, "needs-update");
    assert!(fixture.status().installed);
    assert_eq!(fixture.status().pending_files, 1);
    // 按钮传 false 时，后端仍须保留先前的完整校验要求。
    fixture.run("apply", false, false).unwrap();
    assert!(fixture.status().full);
    assert_eq!(fixture.status().phase, "ready");
    assert_eq!(
        md5_file(&fixture.dir.join(DATA)).unwrap(),
        md5_bytes(&fixture.data)
    );
}

#[test]
fn unknown_large_files_require_verification_and_apply_repairs_them() {
    let fixture = Fixture::new(Region::Global, true);
    fixture.dir.write(DATA, vec![7; fixture.data.len()]);
    fixture.run("check", false, false).unwrap();
    assert_eq!(fixture.status().phase, "needs-verify");
    assert_eq!(fixture.status().unverified_files, 1);
    fixture.run("apply", false, false).unwrap();
    assert_eq!(fixture.status().phase, "ready");
    assert_eq!(fixture.status().unverified_files, 0);
    assert!(fixture.status().full);
    assert_eq!(
        md5_file(&fixture.dir.join(DATA)).unwrap(),
        md5_bytes(&fixture.data)
    );
}

#[test]
fn check_publishes_file_progress_with_current_file() {
    let fixture = Fixture::new(Region::Cn, true);
    let seen = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let observed = seen.clone();
    fixture.host.subscribe("game-download-state", move |event| {
        observed
            .lock()
            .unwrap()
            .push(serde_json::from_value(event.clone()).unwrap());
    });
    fixture.run("check", false, false).unwrap();

    let events = seen.lock().unwrap();
    let checking: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["phase"] == "checking" && event["totalFiles"] == 2)
        .collect();
    let first = checking.first().expect("check progress");
    assert_eq!(first["completedFiles"], 0);
    assert_eq!(first["currentFile"], EXE);
    let last = checking.last().unwrap();
    assert_eq!(last["completedFiles"], 2);
    assert_eq!(last["currentFile"], "");
    assert_eq!(fixture.status().phase, "needs-verify");
}

#[test]
fn check_progress_publishes_the_latest_file_after_throttling() {
    let fixture = Fixture::new(Region::Cn, true);
    let seen = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let observed = seen.clone();
    fixture.host.subscribe("game-download-state", move |event| {
        observed
            .lock()
            .unwrap()
            .push(serde_json::from_value(event.clone()).unwrap());
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let _app = &fixture.ctx;
    let publisher = std::thread::spawn(move || publish_check_progress(&rx));
    tx.send(game::check::CheckProgress {
        checked: 0,
        total: 2,
        current: "a.bin".into(),
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(10));
    tx.send(game::check::CheckProgress {
        checked: 1,
        total: 2,
        current: "b.pak".into(),
    })
    .unwrap();
    drop(tx);
    publisher.join().unwrap();

    let events = seen.lock().unwrap();
    let checking: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["stage"] == "check")
        .collect();
    assert_eq!(checking.len(), 2, "{events:?}");
    assert_eq!(checking[0]["currentFile"], "a.bin");
    assert_eq!(checking[1]["currentFile"], "b.pak");
}

#[test]
fn verified_existing_files_are_recorded_by_apply_without_redownloading() {
    let fixture = Fixture::new(Region::Cn, true);
    fixture.run("apply", false, false).unwrap();
    assert_eq!(fixture.status().phase, "ready");
    assert_eq!(fixture.status().downloaded_bytes, 0);
    fixture.run("check", false, false).unwrap();
    assert_eq!(fixture.status().phase, "ready");
    assert_eq!(fixture.status().unverified_files, 0);
    assert!(!fixture
        .server
        .targets()
        .iter()
        .any(|path| path.contains("/Res/")));
}

#[test]
fn cancellation_preserves_the_task_and_allows_a_fresh_attempt() {
    let fixture = Fixture::new(Region::Cn, false);
    let cancel = Arc::new(AtomicBool::new(false));
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "apply",
        true,
        "manual",
        &cancel,
    )
    .unwrap();
    game_download_cancel(&fixture.ctx);
    assert!(cancel.load(Ordering::Relaxed));
    run_plan(
        &fixture.ctx,
        &fixture.settings,
        "apply",
        true,
        &cancel,
        fixture.plan(true),
    )
    .unwrap();
    assert_eq!(fixture.status().phase, "cancelled");
    assert!(!fixture.dir.join(EXE).exists());
    fixture.run("apply", false, true).unwrap();
    assert_eq!(fixture.status().phase, "ready");
}

#[test]
fn commands_reject_overlapping_tasks_and_running_games() {
    let fixture = Fixture::new(Region::Cn, true);
    assert!(game_download_start(&fixture.ctx, "invalid".into(), false).is_err());
    fixture.set_starting(true);
    assert!(game_download_start(&fixture.ctx, "apply".into(), false)
        .unwrap_err()
        .contains("游戏正在运行"));
    fixture.set_starting(false);
    // UMU 进程组已消失时, 后台会话仍阻止资源写入.
    fixture.set_running(true);
    fixture.set_running(false);
    publish(|status| status.phase = "game".into());
    assert!(game_download_start(&fixture.ctx, "check".into(), false)
        .unwrap_err()
        .contains("任务正在进行"));
}

#[test]
fn full_check_requirement_is_scoped_to_the_installation() {
    let fixture = Fixture::new(Region::Cn, true);
    publish(|status| {
        status.phase = "needs-update".into();
        status.full = true;
        status.install_root = "/another-installation".into();
    });
    let (_, _, full) = prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "check",
        false,
        "manual",
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(!full);
}

#[test]
fn check_falls_back_to_detected_launcher_parent_for_an_unset_install_root() {
    let mut fixture = Fixture::new(Region::Cn, true);
    let launcher_root = fixture.dir.join(profile(Region::Cn).launcher_dir);
    fixture.edit_settings(|settings| settings.install_root = None);
    let (status, settings, _) = prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "check",
        false,
        "manual",
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let expected = launcher_root.parent().unwrap().display().to_string();
    assert_eq!(
        settings.current().install_root.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(status.install_root, expected);
}

#[test]
fn emitted_stage_and_current_file_always_refer_to_the_same_item() {
    let fixture = Fixture::new(Region::Cn, false);
    let seen = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let observed = seen.clone();
    fixture.host.subscribe("game-download-state", move |event| {
        observed
            .lock()
            .unwrap()
            .push(serde_json::from_value(event.clone()).unwrap());
    });
    fixture.run("apply", false, true).unwrap();
    let events = seen.lock().unwrap();
    assert!(events.iter().any(|event| event["stage"] == "verify"));
    for event in events.iter().filter(|event| {
        event["phase"] == "game" && event["stage"] != "" && event["currentFile"] != ""
    }) {
        let path = event["currentFile"].as_str().unwrap();
        let size = event["currentFileBytes"].as_u64().unwrap();
        assert_eq!(
            size,
            if path == DATA {
                fixture.data.len() as u64
            } else {
                b"fixture game executable".len() as u64
            }
        );
    }
}

#[test]
fn byte_events_are_throttled_without_losing_progress() {
    let fixture = Fixture::new(Region::Cn, true);
    let seen = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let observed = seen.clone();
    fixture.host.subscribe("game-download-state", move |event| {
        observed
            .lock()
            .unwrap()
            .push(serde_json::from_value(event.clone()).unwrap());
    });
    for _ in 0..100 {
        publish_bytes(1024);
    }
    assert_eq!(fixture.status().downloaded_bytes, 100 * 1024);
    assert_eq!(seen.lock().unwrap().len(), 1);
    state::manager().inner.lock().unwrap().last_progress =
        Some(Instant::now() - Duration::from_millis(150));
    publish_bytes(1024);
    assert_eq!(seen.lock().unwrap().len(), 2);
    publish(|status| status.phase = "ready".into());
    let events = seen.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events.last().unwrap()["downloadedBytes"], 101 * 1024);
    assert_eq!(events.last().unwrap()["phase"], "ready");
}

#[test]
fn start_command_publishes_a_fast_worker_failure() {
    let fixture = Fixture::new(Region::Cn, true);
    fixture.dir.write(
        "NTEGlobal/Config/Config.ini",
        b"[General]\nGameID=3000001\n",
    );
    let (send, receive) = std::sync::mpsc::channel();
    fixture.host.subscribe("game-download-state", move |event| {
        let status: serde_json::Value = serde_json::from_value(event.clone()).unwrap();
        if status["phase"] == "failed" {
            send.send(status).unwrap();
        }
    });
    let initial = game_download_start(&fixture.ctx, "check".into(), false).unwrap();
    assert_eq!(initial.phase, "checking");
    let failed = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(failed["error"].as_str().unwrap().contains("另一地区"));
    assert_eq!(fixture.status().phase, "failed");
}

#[test]
fn installation_lock_blocks_gui_before_contacting_the_cdn() {
    let fixture = Fixture::new(Region::Cn, true);
    let _lock = InstallLock::acquire(fixture.dir.path()).unwrap();
    let error = run_task(
        &fixture.ctx,
        &fixture.settings,
        "apply",
        false,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("其他任务"), "{error}");
}
#[test]
fn resource_catalog_saves_selection_and_records_optional_tags() {
    let fixture = Fixture::new(Region::Cn, true);
    fixture.ledger();
    let tag = "pakchunk104";
    let path = "Client/WindowsNoEditor/HT/Content/TagPatchPaks/pakchunk104-Windows.pak";
    let bytes = b"optional resource";
    let hash = md5_bytes(bytes);
    let exe = b"fixture game executable";
    let list = format!(
        "<ResList version=\"1.0\" tag=\"baseTag\"><Res filename=\"{EXE}\" md5=\"{}\" filesize=\"{}\"/><Res filename=\"{DATA}\" md5=\"{}\" filesize=\"{}\"/><BaseVersion version=\"0.104.0\" tag=\"{tag}\"><ResList><Res filename=\"{path}\" md5=\"{hash}\" filesize=\"{}\"/></ResList></BaseVersion></ResList>",
        md5_bytes(exe),
        exe.len(),
        md5_bytes(&fixture.data),
        fixture.data.len(),
        bytes.len()
    );
    fixture.server.route(
        "/test/Version/Windows/config.xml",
        format!(
            "<Config><ResVersion>1.0</ResVersion><Hash>hash</Hash><listHash>{}</listHash><PreReleaseBranch>future</PreReleaseBranch><PreReleaseVersion>2.0</PreReleaseVersion><PreReleaseStatus>start</PreReleaseStatus></Config>",
            md5_bytes(list.as_bytes())
        ),
    );
    fixture.server.route(
        "/test/Version/Windows/version/1.0/ResList.bin.zip",
        zip(
            "ResList.bin",
            &game::pxml::encode(list.as_bytes(), "1289").unwrap(),
        ),
    );
    fixture.server.route(
        format!("/test/Res/{}/{}.{}", &hash[..1], hash, bytes.len()),
        bytes,
    );

    let catalog = game_resource_catalog(&fixture.ctx).unwrap();
    assert_eq!(catalog.version, "1.0");
    assert_eq!(catalog.preload_version, "2.0");
    assert_eq!(catalog.resources.len(), 1);
    assert_eq!(catalog.resources[0].tag, tag);
    assert_eq!(
        catalog.resources[0].state,
        game::optional::ResourceState::Absent
    );
    assert!(!catalog.resources[0].selected);

    let selected = fixture.select(vec![tag.to_string()]).unwrap();
    assert!(selected.resources[0].selected);
    assert!(fixture
        .dir
        .join("NTELauncher/UserData/Patcher/PatcherSDK/nte-resource-selection.json")
        .is_file());
    let error = fixture.select(vec!["pakchunk999".into()]).unwrap_err();
    assert!(error.contains("未知的可选资源"), "{error}");

    assert!(!fixture.dir.join(path).exists());
    fixture.run("apply", false, false).unwrap();
    assert_eq!(fixture.status().phase, "ready");
    assert_eq!(fixture.dir.read(path), bytes);

    let config = game::GameUpdateConfig::load(&fixture.dir.join("NTELauncher"), 1289).unwrap();
    let ledger = game::check::load_local_ledger(&config).unwrap().unwrap();
    let record = ledger
        .base_versions
        .iter()
        .find(|record| record.tag == tag)
        .expect("可选资源记录");
    assert_eq!(record.version, "0.104.0");
    assert!(record
        .res
        .iter()
        .any(|entry| entry.filename == path && entry.md5 == hash));
    assert!(ledger.res.iter().all(|entry| entry.filename != path));

    let ledger_before = fixture
        .dir
        .read("NTELauncher/UserData/Patcher/PatcherSDK/ResList.xml");
    let selection_before = fixture
        .dir
        .read("NTELauncher/UserData/Patcher/PatcherSDK/nte-resource-selection.json");
    let error = game_resource_remove(&fixture.ctx, "pakchunk999".into(), false).unwrap_err();
    assert!(error.contains("未知的可选资源"), "{error}");
    fixture.set_starting(true);
    let error = game_resource_remove(&fixture.ctx, tag.into(), false).unwrap_err();
    assert!(error.contains("游戏正在运行"), "{error}");
    fixture.set_starting(false);
    state::manager().inner.lock().unwrap().status.phase = "game".into();
    let error = game_resource_remove(&fixture.ctx, tag.into(), false).unwrap_err();
    assert!(error.contains("游戏资源任务正在进行"), "{error}");
    state::manager().inner.lock().unwrap().status.phase = "ready".into();
    assert_eq!(fixture.dir.read(path), bytes);
    assert_eq!(
        fixture
            .dir
            .read("NTELauncher/UserData/Patcher/PatcherSDK/ResList.xml"),
        ledger_before
    );
    assert_eq!(
        fixture
            .dir
            .read("NTELauncher/UserData/Patcher/PatcherSDK/nte-resource-selection.json"),
        selection_before
    );

    let removed = game_resource_remove(&fixture.ctx, tag.into(), false).unwrap();
    assert_eq!(
        removed.resources[0].state,
        game::optional::ResourceState::Absent
    );
    assert!(!removed.resources[0].selected);
    assert!(!fixture.dir.join(path).exists());
    let ledger = game::check::load_local_ledger(&config).unwrap().unwrap();
    assert!(ledger.base_versions.iter().all(|record| record.tag != tag));
    assert_eq!(fixture.dir.read(DATA), fixture.data);
    assert_eq!(fixture.status().phase, "idle");
    fixture.run("apply", false, false).unwrap();
    assert_eq!(fixture.status().phase, "ready");
    assert!(fixture.status().optional_tags.is_empty());
    assert!(!fixture.dir.join(path).exists());
    let selected = fixture.select(vec![tag.into()]).unwrap();
    assert!(selected.resources[0].selected);
    fixture.run("apply", false, false).unwrap();
    assert_eq!(fixture.dir.read(path), bytes);
}
#[test]
fn preload_includes_selected_optional_resources_without_touching_live_files() {
    let fixture = Fixture::new(Region::Cn, true);
    fixture.ledger();
    let tag = "pakchunk104";
    let path = "Client/WindowsNoEditor/HT/Content/TagPatchPaks/pakchunk104-Windows.pak";
    let current = b"current optional";
    let future = b"future optional";
    let future_base = b"future base data";
    let current_hash = md5_bytes(current);
    let future_hash = md5_bytes(future);
    let future_base_hash = md5_bytes(future_base);
    let exe = b"fixture game executable";

    let current_list = format!(
        "<ResList version=\"1.0\" tag=\"baseTag\"><Res filename=\"{EXE}\" md5=\"{}\" filesize=\"{}\"/><Res filename=\"{DATA}\" md5=\"{}\" filesize=\"{}\"/><BaseVersion version=\"0.104.0\" tag=\"{tag}\"><ResList><Res filename=\"{path}\" md5=\"{current_hash}\" filesize=\"{}\"/></ResList></BaseVersion></ResList>",
        md5_bytes(exe),
        exe.len(),
        md5_bytes(&fixture.data),
        fixture.data.len(),
        current.len()
    );
    let future_list = format!(
        "<ResList version=\"2.0\" tag=\"baseTag\"><Res filename=\"{DATA}\" md5=\"{future_base_hash}\" filesize=\"{}\"/><BaseVersion version=\"0.104.0\" tag=\"{tag}\"><ResList><Res filename=\"{path}\" md5=\"{future_hash}\" filesize=\"{}\"/></ResList></BaseVersion></ResList>",
        future_base.len(),
        future.len()
    );
    fixture.server.route(
        "/test/Version/Windows/config.xml",
        format!(
            "<Config><ResVersion>1.0</ResVersion><Hash>hash</Hash><listHash>{}</listHash><PreReleaseBranch>future</PreReleaseBranch><PreReleaseVersion>2.0</PreReleaseVersion><PreReleaseStatus>start</PreReleaseStatus></Config>",
            md5_bytes(current_list.as_bytes())
        ),
    );
    fixture.server.route(
        "/test/Version/Windows/version/1.0/ResList.bin.zip",
        zip(
            "ResList.bin",
            &game::pxml::encode(current_list.as_bytes(), "1289").unwrap(),
        ),
    );
    fixture.server.route(
        "/future/Version/Windows/version/2.0/config.xml",
        format!(
            "<config><ResVersion>2.0</ResVersion><listHash>{}</listHash></config>",
            md5_bytes(future_list.as_bytes())
        ),
    );
    fixture.server.route(
        "/future/Version/Windows/version/2.0/ResList.bin.zip",
        zip(
            "ResList.bin",
            &game::pxml::encode(future_list.as_bytes(), "1289").unwrap(),
        ),
    );
    for bytes in [future_base.as_slice(), future.as_slice()] {
        let hash = md5_bytes(bytes);
        fixture.server.route(
            format!("/future/Res/{}/{}.{}", &hash[..1], hash, bytes.len()),
            bytes,
        );
    }

    let cache_root = TempDir::new("preload-cache-root");
    let custom_root = cache_root.path().to_path_buf();
    let mut fixture = fixture;
    fixture.settings.current_mut().preload_root = Some(custom_root.display().to_string());
    fixture
        .edit_settings(|settings| settings.preload_root = Some(custom_root.display().to_string()));

    let catalog = game_resource_catalog(&fixture.ctx).unwrap();
    assert_eq!(catalog.resources.len(), 1);
    assert!(!catalog.resources[0].selected);
    assert!(!catalog.preload_completed);
    fixture.select(vec![tag.to_string()]).unwrap();

    let ledger = fixture
        .dir
        .join("NTELauncher/UserData/Patcher/PatcherSDK/ResList.xml");
    let old_ledger = std::fs::read(&ledger).unwrap();
    let old_data = std::fs::read(fixture.dir.join(DATA)).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();

    let status = fixture.status();
    assert_eq!(status.mode, "preload");
    assert_eq!(status.phase, "preloaded");
    assert_eq!(status.total_files, 2);
    assert_eq!(status.pending_files, 0);
    assert_eq!(std::fs::read(&ledger).unwrap(), old_ledger);
    assert_eq!(std::fs::read(fixture.dir.join(DATA)).unwrap(), old_data);
    assert!(!fixture.dir.join(path).exists());

    let staging = custom_root.join("future/2.0/nte-native");
    let mut downloaded = Vec::new();
    collect_files(&staging, &mut downloaded);
    downloaded.sort();
    assert_eq!(downloaded.len(), 2, "{downloaded:?}");
    let contents: Vec<_> = downloaded
        .iter()
        .map(|file| std::fs::read(file).unwrap())
        .collect();
    assert!(contents.iter().any(|bytes| bytes == future_base));
    assert!(contents.iter().any(|bytes| bytes == future));

    let catalog = game_resource_catalog(&fixture.ctx).unwrap();
    assert!(catalog.preload_available);
    assert!(catalog.preload_completed);
    assert!(catalog.preload_cached);
    assert_eq!(catalog.preload_cached_version, "2.0");
    assert!(catalog.preload_cached_bytes > 0);
    assert_eq!(
        catalog.preload_resource_bytes.get(tag),
        Some(&(future.len() as u64))
    );

    // 已完成时再次预下载只做校验, 不进入下载阶段.
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();
    assert_eq!(fixture.status().phase, "preloaded");
    assert_eq!(fixture.status().pending_files, 0);

    // 只清理当前语种的预下载, 基础缓存和正式文件继续保留.
    let removed = game_resource_remove(&fixture.ctx, tag.into(), true).unwrap();
    assert!(!removed.preload_completed);
    assert!(removed.preload_cached);
    assert!(!removed.resources[0].selected);
    assert!(!removed.preload_resource_bytes.contains_key(tag));
    assert!(game_resource_remove(&fixture.ctx, tag.into(), true)
        .unwrap_err()
        .contains("没有可删除"));
    let mut remaining = Vec::new();
    collect_files(&staging, &mut remaining);
    assert_eq!(remaining.len(), 1);
    assert_eq!(std::fs::read(&remaining[0]).unwrap(), future_base);
    assert_eq!(std::fs::read(&ledger).unwrap(), old_ledger);
    assert_eq!(std::fs::read(fixture.dir.join(DATA)).unwrap(), old_data);
    fixture.select(vec![tag.into()]).unwrap();

    let deleted = game_preload_delete(&fixture.ctx).unwrap();
    assert!(!deleted.preload_completed);
    assert!(!deleted.preload_cached);
    assert!(!staging.exists());
    assert_eq!(std::fs::read(&ledger).unwrap(), old_ledger);
    assert_eq!(std::fs::read(fixture.dir.join(DATA)).unwrap(), old_data);

    // 公告消失后仍按缓存中较新的版本校验和清理.
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();
    assert!(staging.exists());
    fixture.server.route(
        "/test/Version/Windows/config.xml",
        format!(
            "<Config><ResVersion>1.0</ResVersion><Hash>hash</Hash><listHash>{}</listHash></Config>",
            md5_bytes(current_list.as_bytes())
        ),
    );
    let orphan = game_resource_catalog(&fixture.ctx).unwrap();
    assert!(orphan.preload_available);
    assert_eq!(orphan.preload_version, "2.0");
    assert!(orphan.preload_completed);
    assert!(orphan.preload_cached);
    assert_eq!(orphan.preload_cached_version, "2.0");

    // 公告消失后仍按缓存版本校验.
    prepare_task(
        &fixture.ctx,
        fixture.ctx.downloads(),
        "preload",
        false,
        "manual",
        &cancel,
    )
    .unwrap();
    run_task(&fixture.ctx, &fixture.settings, "preload", false, &cancel).unwrap();
    assert_eq!(fixture.status().phase, "preloaded");
    assert_eq!(fixture.status().pending_files, 0);
    assert!(staging.exists());

    let deleted = game_preload_delete(&fixture.ctx).unwrap();
    assert!(!deleted.preload_available);
    assert!(!deleted.preload_cached);
    assert!(!staging.exists());
    assert_eq!(std::fs::read(&ledger).unwrap(), old_ledger);
    assert_eq!(std::fs::read(fixture.dir.join(DATA)).unwrap(), old_data);
}

fn collect_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn configure_voice_downloads(fixture: &Fixture) -> [(String, Vec<u8>, String); 2] {
    let voices = [
        ("pakchunk101", vec![31; 64 * 1024]),
        ("pakchunk102", vec![47; 64 * 1024]),
    ]
    .map(|(tag, bytes)| {
        let path = format!("Client/WindowsNoEditor/HT/Content/TagPatchPaks/{tag}-Windows.pak");
        (tag.to_string(), bytes, path)
    });
    let mut xml = format!("<ResList version=\"2.0\" tag=\"baseTag\"><Res filename=\"{DATA}\" md5=\"{}\" filesize=\"{}\"/>", md5_bytes(b"new base version"), 16);
    for (tag, bytes, path) in &voices {
        let hash = md5_bytes(bytes);
        xml.push_str(&format!("<BaseVersion version=\"0.{}.0\" tag=\"{tag}\"><ResList><Res filename=\"{path}\" md5=\"{hash}\" filesize=\"{}\"/></ResList></BaseVersion>", &tag[8..], bytes.len()));
        fixture.server.route(
            format!("/test/Res/{}/{hash}.{}", &hash[..1], bytes.len()),
            bytes,
        );
    }
    xml.push_str("</ResList>");
    fixture.server.route(
        "/test/Version/Windows/config.xml",
        format!(
            "<Config><ResVersion>2.0</ResVersion><Hash>hash</Hash><listHash>{}</listHash></Config>",
            md5_bytes(xml.as_bytes())
        ),
    );
    fixture.server.route(
        "/test/Version/Windows/version/2.0/ResList.bin.zip",
        zip(
            "ResList.bin",
            &game::pxml::encode(xml.as_bytes(), "1289").unwrap(),
        ),
    );
    voices
}

fn wait_resource_tasks(fixture: &Fixture) -> DownloadStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = fixture.status();
        if !busy(&status.phase) {
            return status;
        }
        assert!(Instant::now() < deadline, "语音任务未结束: {status:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn voice_downloads_run_concurrently_without_updating_base_and_keep_both_ledger_tags() {
    let mut fixture = Fixture::new(Region::Cn, true);
    let mut env = crate::testing::EnvGuard::new();
    env.set("XDG_DATA_HOME", fixture.dir.join("xdg-data"));
    fixture.ledger();
    let voices = configure_voice_downloads(&fixture);
    fixture.edit_settings(|settings| settings.update.limit_bytes_per_second = 64 * 1024);
    let first = game_resource_download(&fixture.ctx, voices[0].0.clone()).unwrap();
    assert_eq!(first.resource_tasks[&voices[0].0].phase, "checking");
    let second = game_resource_download(&fixture.ctx, voices[1].0.clone()).unwrap();
    assert_eq!(second.mode, "resources");
    assert_eq!(second.resource_tasks.len(), 2);
    assert!(game_resource_download(&fixture.ctx, voices[0].0.clone())
        .unwrap_err()
        .contains("正在下载"));
    assert!(game_download_start(&fixture.ctx, "apply".into(), false).is_err());
    assert!(game_resource_remove(&fixture.ctx, voices[0].0.clone(), false).is_err());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = fixture.status();
        let targets = fixture.server.targets();
        let both_requested = voices.iter().all(|(_, bytes, _)| {
            let hash = md5_bytes(bytes);
            let target = format!("/test/Res/{}/{hash}.{}", &hash[..1], bytes.len());
            targets
                .iter()
                .any(|request| request.split('?').next() == Some(target.as_str()))
        });
        if status
            .resource_tasks
            .values()
            .all(|task| task.phase == "downloading")
            && both_requested
        {
            break;
        }
        assert!(Instant::now() < deadline, "语音未并发下载: {status:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = Instant::now();
    let status = wait_resource_tasks(&fixture);
    assert_eq!(status.phase, "ready", "{status:?}");
    assert!(status
        .resource_tasks
        .values()
        .all(|task| task.phase == "ready"));
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "未遵守两个语种共享的限速"
    );
    assert_eq!(fixture.dir.read(DATA), fixture.data);
    let (_, config, _) = load_catalog_context(&fixture.ctx).unwrap();
    let ledger = game::check::load_local_ledger(&config).unwrap().unwrap();
    assert_eq!(ledger.version, "1.0");
    assert_eq!(ledger.base_versions.len(), 2);
    for (tag, bytes, path) in voices {
        assert_eq!(fixture.dir.read(path), bytes);
        assert!(ledger.base_versions.iter().any(|base| base.tag == tag));
    }
    let selection =
        std::fs::read_to_string(config.patcher_dir.join("nte-resource-selection.json")).unwrap();
    assert!(selection.contains("pakchunk101") && selection.contains("pakchunk102"));
    assert!(InstallLock::acquire(fixture.dir.path()).is_ok());
}

#[test]
fn voice_downloads_cancel_together_and_failed_tags_can_be_retried() {
    let mut fixture = Fixture::new(Region::Cn, true);
    let mut env = crate::testing::EnvGuard::new();
    env.set("XDG_DATA_HOME", fixture.dir.join("xdg-data"));
    let voices = configure_voice_downloads(&fixture);
    fixture.edit_settings(|settings| settings.update.limit_bytes_per_second = 1024);
    game_resource_download(&fixture.ctx, voices[0].0.clone()).unwrap();
    game_resource_download(&fixture.ctx, voices[1].0.clone()).unwrap();
    game_download_cancel(&fixture.ctx);
    let status = wait_resource_tasks(&fixture);
    assert_eq!(status.phase, "cancelled");
    assert!(status
        .resource_tasks
        .values()
        .all(|task| task.phase == "cancelled"));
    assert_eq!(fixture.dir.read(DATA), fixture.data);
    assert!(game_resource_download(&fixture.ctx, "unknown".into())
        .unwrap_err()
        .contains("未知的可选资源"));
    fixture.edit_settings(|settings| settings.update.limit_bytes_per_second = 0);
    let hash = md5_bytes(&voices[0].1);
    fixture.server.route(
        format!("/test/Res/{}/{hash}.{}", &hash[..1], voices[0].1.len()),
        b"invalid content",
    );
    game_resource_download(&fixture.ctx, voices[0].0.clone()).unwrap();
    let status = wait_resource_tasks(&fixture);
    assert_eq!(status.phase, "failed", "{status:?}");
    assert!(!status.resource_tasks[&voices[0].0].error.is_empty());
    fixture.server.route(
        format!("/test/Res/{}/{hash}.{}", &hash[..1], voices[0].1.len()),
        &voices[0].1,
    );
    game_resource_download(&fixture.ctx, voices[0].0.clone()).unwrap();
    assert_eq!(wait_resource_tasks(&fixture).phase, "ready");
    assert_eq!(fixture.dir.read(&voices[0].2), voices[0].1);
}
