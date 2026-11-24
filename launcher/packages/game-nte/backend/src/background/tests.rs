//! 背景素材扫描与命令分发测试.

use std::path::PathBuf;

use serde_json::json;

use super::*;
use crate::testing::{block_on, TempDir, TestHost};

/// 写入带 GameID 的官方启动器配置, 返回启动器根目录.
fn launcher(dir: &TempDir, game_id: &str) -> PathBuf {
    dir.write(
        "launcher/Config/Config.ini",
        format!("[General]\nGameID={game_id}\n"),
    );
    dir.join("launcher")
}

fn background_dir(dir: &TempDir, game_id: &str) -> PathBuf {
    dir.join(format!("launcher/ResFilesM/{game_id}/bgimgs"))
}

fn canonical(path: PathBuf) -> String {
    std::fs::canonicalize(path)
        .expect("规范化路径")
        .display()
        .to_string()
}

#[test]
fn reads_video_and_selected_image_from_manifest() {
    let dir = TempDir::new("background-manifest");
    let root = launcher(&dir, "1289");
    let bgimgs = background_dir(&dir, "1289");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg.mp4", b"video");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"first");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_1.png", b"second");
    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"video":"bg.mp4","imgs":[{"file":"bg_0.png"},{"file":"bg_1.png"}],"first":1}"#,
    );

    let background = resolve(&root).expect("find background");
    assert_eq!(background.video, Some(bgimgs.join("bg.mp4")));
    assert_eq!(background.image, Some(bgimgs.join("bg_1.png")));
}

#[test]
fn ignores_manifest_paths_outside_the_background_dir() {
    let dir = TempDir::new("background-traversal");
    let root = launcher(&dir, "1289");
    dir.write("outside.png", b"outside");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"inside");
    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"video":"../../outside.png","imgs":[{"file":"../outside.png"}],"noVideoBg":"bg_0.png"}"#,
    );

    let background = resolve(&root).expect("find background");
    assert_eq!(background.video, None);
    assert_eq!(
        background.image,
        Some(background_dir(&dir, "1289").join("bg_0.png"))
    );
}

#[test]
fn falls_back_to_known_names_without_a_manifest() {
    let dir = TempDir::new("background-names");
    let root = launcher(&dir, "1289");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg.mp4", b"video");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"image");

    let background = resolve(&root).expect("find background");
    assert_eq!(
        background.video,
        Some(background_dir(&dir, "1289").join("bg.mp4"))
    );
    assert_eq!(
        background.image,
        Some(background_dir(&dir, "1289").join("bg_0.png"))
    );
}

#[test]
fn uses_no_video_bg_when_images_are_missing() {
    let dir = TempDir::new("background-no-video");
    let root = launcher(&dir, "1289");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"image");
    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"imgs":[{"file":"missing.png"}],"noVideoBg":"bg_0.png"}"#,
    );

    let background = resolve(&root).expect("find background");
    assert_eq!(
        background.image,
        Some(background_dir(&dir, "1289").join("bg_0.png"))
    );
}

#[test]
fn scans_game_directories_without_a_launcher_config() {
    let dir = TempDir::new("background-scan");
    let root = dir.join("launcher");
    dir.write("launcher/ResFilesM/3000001/bgimgs/bg.mp4", b"video");
    dir.write(
        "launcher/ResFilesM/3000001/bgimgs/config.json",
        r#"{"video":"bg.mp4"}"#,
    );

    let background = resolve(&root).expect("find background");
    assert_eq!(
        background.video,
        Some(dir.join("launcher/ResFilesM/3000001/bgimgs/bg.mp4"))
    );
    assert_eq!(background.image, None);
}

#[test]
fn configured_game_id_does_not_use_another_distributions_background() {
    let dir = TempDir::new("background-region");
    let root = launcher(&dir, "3000001");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg.mp4", b"cn video");

    assert_eq!(resolve(&root), None);
}

#[test]
fn launcher_directory_names_fill_in_a_missing_game_id() {
    let dir = TempDir::new("background-launcher-dir");
    dir.write(
        "launcher/NTEGlobal/Config/Config.ini",
        "[General]\nGameID=0\n",
    );
    dir.write(
        "launcher/NTEGlobal/ResFilesM/3000001/bgimgs/bg_0.png",
        b"global",
    );
    let background = resolve(&dir.join("launcher/NTEGlobal")).expect("find global background");
    assert_eq!(
        background.image,
        Some(dir.join("launcher/NTEGlobal/ResFilesM/3000001/bgimgs/bg_0.png"))
    );

    dir.write(
        "launcher/NTELauncher/Config/Config.ini",
        "[General]\nGameID=0\n",
    );
    dir.write("launcher/NTELauncher/ResFilesM/1289/bgimgs/bg_0.png", b"cn");
    let background = resolve(&dir.join("launcher/NTELauncher")).expect("find cn background");
    assert_eq!(
        background.image,
        Some(dir.join("launcher/NTELauncher/ResFilesM/1289/bgimgs/bg_0.png"))
    );
}

#[test]
fn game_id_falls_back_to_scanning_for_an_unknown_launcher_directory() {
    let dir = TempDir::new("background-unknown-dir");
    dir.write("launcher/Config/Config.ini", "[General]\nGameID=0\n");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"image");

    let background = resolve(&dir.join("launcher")).expect("find background");
    assert_eq!(
        background.image,
        Some(background_dir(&dir, "1289").join("bg_0.png"))
    );
}

#[test]
fn config_game_id_reads_only_the_general_section() {
    assert_eq!(
        config_game_id("\u{feff}[General]\nGameID=3000001\n"),
        Some(3_000_001)
    );
    assert_eq!(
        config_game_id("; 注释\n[Other]\nGameID=1\n[General]\nGameID = 1289\n"),
        Some(1289)
    );
    assert_eq!(
        config_game_id("[General]\nLauncher=NTELauncher.exe\n"),
        None
    );
    assert_eq!(
        config_game_id("[General]\nGameID=1289\nGameID=invalid\n"),
        None
    );
    assert_eq!(
        config_game_id("[General]\nGameID=invalid\nGameID=1289\n"),
        Some(1289)
    );
    assert_eq!(
        config_game_id("[General]\nGameID=1289\nno separator\n"),
        Some(1289)
    );
}

#[test]
fn manifest_image_index_falls_back_to_the_first_image() {
    let dir = TempDir::new("background-index");
    let root = launcher(&dir, "1289");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"first");
    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"imgs":[{"file":"bg_0.png"}],"first":9}"#,
    );
    assert_eq!(
        resolve(&root).expect("find background").image,
        Some(background_dir(&dir, "1289").join("bg_0.png"))
    );

    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"imgs":[{"file":"bg_0.png"}],"first":-1}"#,
    );
    assert_eq!(
        resolve(&root).expect("find background").image,
        Some(background_dir(&dir, "1289").join("bg_0.png"))
    );
}

#[test]
fn asset_path_accepts_only_single_file_names() {
    let dir = TempDir::new("background-asset");
    let bgimgs = dir.join("bgimgs");
    dir.write("bgimgs/bg.mp4", b"video");

    assert_eq!(asset_path(&bgimgs, "bg.mp4"), Some(bgimgs.join("bg.mp4")));
    assert_eq!(asset_path(&bgimgs, "../bg.mp4"), None);
    assert_eq!(asset_path(&bgimgs, "/bg.mp4"), None);
    assert_eq!(asset_path(&bgimgs, "missing.mp4"), None);
    assert_eq!(asset_path(&bgimgs, ""), None);
}

#[test]
fn returns_none_without_usable_assets() {
    let dir = TempDir::new("background-empty");
    let root = launcher(&dir, "1289");
    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"video":"bg.mp4","imgs":[{"file":"bg_0.png"}]}"#,
    );

    assert_eq!(resolve(&root), None);
}

#[test]
fn command_returns_canonical_media_paths() {
    let dir = TempDir::new("background-command");
    let root = launcher(&dir, "1289");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg.mp4", b"video");
    dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", b"image");
    dir.write(
        "launcher/ResFilesM/1289/bgimgs/config.json",
        r#"{"video":"bg.mp4","imgs":[{"file":"bg_0.png"}]}"#,
    );

    let host = TestHost::new(Some(root));
    let media =
        block_on(command(&host, "launcher_background", json!(null))).expect("背景命令应返回素材");
    assert_eq!(
        media["video"],
        json!(canonical(background_dir(&dir, "1289").join("bg.mp4")))
    );
    assert_eq!(
        media["image"],
        json!(canonical(background_dir(&dir, "1289").join("bg_0.png")))
    );
}

#[test]
fn command_returns_empty_media_without_a_launcher_root() {
    let media = block_on(command(
        &TestHost::new(None),
        "launcher_background",
        json!(null),
    ))
    .expect("未安装时也要返回空素材");

    assert_eq!(media, json!({"video": null, "image": null}));
}

#[test]
fn command_returns_empty_media_when_assets_are_missing() {
    let dir = TempDir::new("background-command-empty");
    let root = launcher(&dir, "1289");

    // 参数与旧实现一样被忽略, 只有区服外的载荷也不会报错.
    let media = block_on(command(
        &TestHost::new(Some(root)),
        "launcher_background",
        json!({"unexpected": true}),
    ))
    .expect("没有素材时返回空形状");
    assert_eq!(media, json!({"video": null, "image": null}));
}

#[test]
fn command_rejects_unknown_background_command() {
    let error = block_on(command(&TestHost::new(None), "unknown", json!({}))).unwrap_err();

    assert_eq!(error, json!("无法识别的背景命令"));
}

/// 归档开启的测试宿主: 数据目录指向临时目录, 归档参数写在索引里.
fn archive_host(dir: &TempDir) -> TestHost {
    let mut host = TestHost::new(Some(launcher(dir, "1289")));
    host.data_dir = Some(dir.join("data"));
    host
}

fn store(host: &TestHost) -> archive::Store {
    archive::store(host).expect("归档位置")
}

/// 在当前素材目录里写入一套素材; 同一路径反复写入即模拟官方覆盖.
fn live(dir: &TempDir, video: Option<&[u8]>, image: Option<&[u8]>) -> LauncherBackground {
    let video = video.map(|bytes| dir.write("launcher/ResFilesM/1289/bgimgs/bg.mp4", bytes));
    let image = image.map(|bytes| dir.write("launcher/ResFilesM/1289/bgimgs/bg_0.png", bytes));
    LauncherBackground {
        images: image.iter().cloned().collect(),
        video,
        image,
        manifest: Some("{\"video\":\"bg.mp4\"}".into()),
    }
}

fn preferences(entries: &std::path::Path, retention: u32) -> archive::Preferences {
    archive::Preferences {
        archive: true,
        dir: entries.display().to_string(),
        retention,
    }
}

#[test]
fn archive_keeps_one_entry_per_content() {
    let dir = TempDir::new("archive-dedupe");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"video-a"), Some(b"image-a"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    assert!(state.archive);
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.current, Some(state.entries[0].0.clone()));
    assert!(entries.join(&state.entries[0].0).is_dir());

    // 内容没变时不重复归档.
    let state = archive::sync_optional(&store, Some(&first));
    assert_eq!(state.entries.len(), 1);

    // 同名文件换了内容: 另一套.
    let second = live(&dir, Some(b"video-b"), Some(b"image-b"));
    let state = archive::sync_optional(&store, Some(&second));
    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.current, Some(state.entries[1].0.clone()));
}

#[test]
fn archive_prunes_to_retention_and_keeps_selection() {
    let dir = TempDir::new("archive-prune");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 2)).expect("配置归档");
    let selected = state.entries[0].0.clone();
    archive::select(&store, Some(&first), Some(&selected)).expect("选择第一套");

    for index in 2..=5 {
        let video = format!("v{index}");
        let image = format!("i{index}");
        let live = live(&dir, Some(video.as_bytes()), Some(image.as_bytes()));
        archive::sync_optional(&store, Some(&live));
    }

    let state = archive::sync_optional(&store, Some(&first));
    // 上限 2 套, 正在使用的那套占一个名额.
    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.selected, Some(selected.clone()));
    assert!(entries.join(&selected).is_dir(), "正在使用的一套不能被剪枝");
}

#[test]
fn archive_drops_entries_with_missing_files() {
    let dir = TempDir::new("archive-missing");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();
    std::fs::remove_dir_all(entries.join(&id)).expect("删除归档条目");

    // 换成另一套素材, 失踪的条目不会再被当前素材生成回来.
    let second = live(&dir, Some(b"v2"), Some(b"i2"));
    let state = archive::sync_optional(&store, Some(&second));
    assert_eq!(state.entries.len(), 1);
    assert_ne!(state.entries[0].0, id);
}

#[test]
fn archive_keeps_video_without_static_image() {
    let dir = TempDir::new("archive-video-only");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let live = live(&dir, Some(b"video-only"), None);
    let state =
        archive::configure(&store, Some(&live), preferences(&entries, 3)).expect("配置归档");
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0].1.video.is_some());
    assert!(state.entries[0].1.image.is_none());
}

#[test]
fn archive_writes_nothing_when_disabled() {
    let dir = TempDir::new("archive-disabled");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let live = live(&dir, Some(b"v1"), Some(b"i1"));
    let state = archive::sync_optional(&store, Some(&live));
    assert!(!state.archive);
    assert!(state.entries.is_empty());
    assert!(!entries.exists());
}

#[test]
fn archive_moves_entries_when_directory_changes() {
    let dir = TempDir::new("archive-migrate");
    let first_dir = dir.join("archive-first");
    let second_dir = dir.join("archive-second");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&first_dir, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    let state =
        archive::configure(&store, Some(&first), preferences(&second_dir, 3)).expect("配置归档");
    assert_eq!(state.migration, archive::Migration::Moved(1));
    assert!(second_dir.join(&id).is_dir());
    assert!(!first_dir.join(&id).exists());
    assert_eq!(state.entries.len(), 1);
}

#[test]
fn archive_keeps_old_directory_when_migration_fails() {
    let dir = TempDir::new("archive-migrate-fail");
    let first_dir = dir.join("archive-first");
    let second_dir = dir.join("archive-second");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&first_dir, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 目标目录里已有别的内容, 迁移必须放弃而不是合并.
    std::fs::create_dir_all(second_dir.join("keep")).expect("创建目标目录");
    std::fs::write(second_dir.join("keep/file"), b"x").expect("写入目标内容");

    let error = archive::configure(&store, Some(&first), preferences(&second_dir, 3))
        .expect_err("迁移失败要报错");
    assert_eq!(error, "目标目录里已有其他文件, 请先清空");
    assert!(first_dir.join(&id).is_dir());
    let state = archive::sync_optional(&store, Some(&first));
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0]
        .1
        .video
        .as_ref()
        .is_some_and(|path| path.exists()));
}

#[test]
fn command_returns_selected_archive_media() {
    let dir = TempDir::new("archive-command-select");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();
    let archived_video = state.entries[0].1.video.clone().expect("归档视频");

    // 换成新素材后, 选中的旧素材仍应作为背景返回.
    let second = live(&dir, Some(b"v2"), Some(b"i2"));
    archive::sync_optional(&store, Some(&second));
    archive::select(&store, Some(&second), Some(&id)).expect("选择历史素材");

    let media = block_on(command(&host, "launcher_background", json!(null))).expect("读取背景");
    let video = media
        .get("video")
        .and_then(|value| value.as_str())
        .expect("视频路径");
    assert_eq!(video, canonical(archived_video));
}

#[test]
fn archive_keeps_current_entry_when_pruning() {
    let dir = TempDir::new("archive-prune-current");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 2)).expect("配置归档");
    let oldest = state.entries[0].0.clone();

    for index in 2..=4 {
        let video = format!("v{index}");
        let image = format!("i{index}");
        let live = live(&dir, Some(video.as_bytes()), Some(image.as_bytes()));
        archive::sync_optional(&store, Some(&live));
    }

    // 官方素材轮回到了最早那一套, 它不能被剪掉.
    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state = archive::sync_optional(&store, Some(&first));
    assert_eq!(state.current, Some(oldest.clone()));
    assert!(state.entries.iter().any(|(id, _)| id == &oldest));
}

#[test]
fn configure_normalizes_retention() {
    let dir = TempDir::new("archive-retention");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let mut preferences = preferences(&entries, 0);
    let state = archive::configure(&store, None, preferences.clone()).expect("配置归档");
    assert_eq!(state.retention, 3);

    preferences.retention = 99;
    let state = archive::configure(&store, None, preferences).expect("配置归档");
    assert_eq!(state.retention, 10);
}

#[test]
fn configure_rejects_unusable_directory() {
    let dir = TempDir::new("archive-bad-dir");
    let host = archive_host(&dir);
    let store = store(&host);

    let relative = archive::Preferences {
        archive: true,
        dir: "relative/backgrounds".into(),
        retention: 3,
    };
    assert!(archive::configure(&store, None, relative).is_err());

    let file = dir.write("archive-file", b"x");
    let occupied = archive::Preferences {
        archive: true,
        dir: file.display().to_string(),
        retention: 3,
    };
    assert!(archive::configure(&store, None, occupied).is_err());
}

#[test]
fn archive_resumes_a_partial_migration() {
    let dir = TempDir::new("archive-migrate-resume");
    let first_dir = dir.join("archive-first");
    let second_dir = dir.join("archive-second");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&first_dir, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 模拟上次迁移中断: 条目已经在目标目录, 索引还指向旧目录.
    std::fs::create_dir_all(&second_dir).expect("创建目标目录");
    std::fs::rename(first_dir.join(&id), second_dir.join(&id)).expect("搬移条目");

    let state =
        archive::configure(&store, Some(&first), preferences(&second_dir, 3)).expect("配置归档");
    assert_eq!(state.migration, archive::Migration::Idle);
    assert_eq!(state.entries.len(), 1);
    assert!(second_dir.join(&id).is_dir());
}

#[test]
fn archive_commands_report_state_and_reject_unknown_selection() {
    let dir = TempDir::new("archive-commands");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    // 未开启时列表为空, 选择直接被拒绝.
    let view =
        block_on(command(&host, "launcher_background_archive", json!(null))).expect("读取列表");
    assert_eq!(view.get("archive"), Some(&json!(false)));
    assert_eq!(view.get("entries"), Some(&json!([])));
    assert_eq!(view.get("retention"), Some(&json!(3)));
    let error = block_on(command(
        &host,
        "launcher_background_select",
        json!({ "id": "missing" }),
    ))
    .expect_err("未启用时不能选择");
    assert_eq!(error, json!("背景素材归档未启用"));

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    archive::sync_optional(&store, Some(&first));
    let view = block_on(command(
        &host,
        "launcher_background_configure",
        json!({
            "archive": true,
            "dir": entries.display().to_string(),
            "retention": 3,
        }),
    ))
    .expect("写入归档偏好");
    assert_eq!(view.get("archive"), Some(&json!(true)));
    assert_eq!(
        view.get("entries")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );

    let error = block_on(command(
        &host,
        "launcher_background_select",
        json!({ "id": "missing" }),
    ))
    .expect_err("未知条目要报错");
    assert_eq!(error, json!("找不到该背景素材"));

    let error = block_on(command(
        &host,
        "launcher_background_configure",
        json!({ "archive": true }),
    ))
    .expect_err("缺少目录参数要报错");
    assert_eq!(error, json!("无效的归档目录"));
}

#[test]
fn archive_keeps_index_when_directory_is_unreachable() {
    let dir = TempDir::new("archive-unreachable");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 挂载点掉线时目录还在, 但内容没了; 索引不能被清空.
    std::fs::remove_dir_all(&entries).expect("清空归档目录");
    std::fs::create_dir_all(&entries).expect("重建空目录");
    let state = archive::sync_optional(&store, None);
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.current, None);
    assert!(state.entries.iter().any(|(entry, _)| entry == &id));
}

#[test]
fn archive_readopts_an_entry_directory_missing_from_the_index() {
    let dir = TempDir::new("archive-readopt");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 索引丢了但条目目录还在, 下一次同步要把它接回索引而不是再复制一份.
    std::fs::remove_file(dir.join("data/backgrounds/nte/cn.json")).expect("删除索引");
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].0, id);
}

#[test]
fn archive_recopies_a_partial_entry_left_by_an_interrupted_move() {
    let dir = TempDir::new("archive-migrate-partial");
    let first_dir = dir.join("archive-first");
    let second_dir = dir.join("archive-second");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&first_dir, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 上次迁移断在复制中途: 目标里只有一半文件.
    std::fs::create_dir_all(second_dir.join(&id)).expect("创建目标条目");
    std::fs::copy(
        first_dir.join(&id).join("entry.json"),
        second_dir.join(&id).join("entry.json"),
    )
    .expect("复制元数据");

    let state =
        archive::configure(&store, Some(&first), preferences(&second_dir, 3)).expect("配置归档");
    assert_eq!(state.entries.len(), 1);
    assert!(second_dir.join(&id).join("bg.mp4").is_file());
    assert!(second_dir.join(&id).join("bg_0.png").is_file());
}

#[test]
fn archive_rebuilds_a_corrupt_index_and_keeps_a_copy() {
    let dir = TempDir::new("archive-corrupt");
    let host = archive_host(&dir);
    let store = store(&host);
    let index = dir.join("data/backgrounds/nte/cn.json");
    std::fs::create_dir_all(index.parent().expect("索引目录")).expect("创建索引目录");
    std::fs::write(&index, b"{ not json").expect("写入坏索引");

    let state = archive::sync_optional(&store, None);
    assert!(state.entries.is_empty());
    let backups = std::fs::read_dir(index.parent().expect("索引目录"))
        .expect("读取索引目录")
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("cn.json.corrupt-")
        })
        .count();
    assert_eq!(backups, 1);
}

#[test]
fn configure_keeps_the_index_when_the_old_directory_is_unreachable() {
    let dir = TempDir::new("archive-offline");
    let first_dir = dir.join("archive-first");
    let second_dir = dir.join("archive-second");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&first_dir, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 挂载点掉线: 目录还在, 但条目与标记都没了.
    std::fs::remove_dir_all(&first_dir).expect("清空旧目录");
    std::fs::create_dir_all(&first_dir).expect("重建空目录");

    let error = archive::configure(&store, Some(&first), preferences(&second_dir, 3))
        .expect_err("旧目录不可达时不能迁移");
    assert_eq!(error, "归档目录暂时不可用, 请检查磁盘或挂载");
    assert!(!second_dir.exists());

    // 索引没有被清空, 也没有指到新目录.
    let state = archive::sync_optional(&store, Some(&first));
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries.iter().any(|(entry, _)| entry == &id));
}

#[test]
fn archive_reports_unavailable_for_a_newer_index() {
    let dir = TempDir::new("archive-newer-index");
    let host = archive_host(&dir);
    let store = store(&host);
    let index = dir.join("data/backgrounds/nte/cn.json");
    std::fs::create_dir_all(index.parent().expect("索引目录")).expect("创建索引目录");
    let original = br#"{"version":9,"archive":true,"entries":[]}"#;
    std::fs::write(&index, original).expect("写入新版本索引");

    let state = archive::sync_optional(&store, None);
    assert!(!state.available);
    assert!(state.entries.is_empty());
    // 文件必须原样保留, 不能被旧版本覆盖.
    assert_eq!(std::fs::read(&index).expect("读取索引"), original);
}

#[test]
fn archive_drops_entries_pointing_outside_their_directory() {
    let dir = TempDir::new("archive-traversal");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);
    std::fs::create_dir_all(&entries).expect("创建归档目录");
    std::fs::write(entries.join(".oddity-backgrounds"), b"oddity").expect("写标记");
    std::fs::create_dir_all(entries.join("bad")).expect("创建坏条目");
    std::fs::write(
        entries.join("bad/entry.json"),
        r#"{"id":"bad","capturedAt":1,"video":"../../escape.mp4","cover":null,"images":["../../escape.png"]}"#,
    )
    .expect("写入坏条目");
    let index = dir.join("data/backgrounds/nte/cn.json");
    std::fs::create_dir_all(index.parent().expect("索引目录")).expect("创建索引目录");
    std::fs::write(
        &index,
        format!(
            r#"{{"version":1,"archive":true,"dir":"{}","retention":3,"entries":[{{"id":"bad","capturedAt":1,"video":"../../escape.mp4","cover":null,"images":["../../escape.png"]}}]}}"#,
            entries.display()
        ),
    )
    .expect("写入索引");

    // 索引里的条目不可信, 解出来就丢掉, 也不会去动它指向的目录.
    let state = archive::sync_optional(&store, None);
    assert!(state.entries.is_empty());
    assert!(entries.join("bad").is_dir());
}

#[test]
fn archive_ignores_entries_with_a_traversing_id() {
    let dir = TempDir::new("archive-id-traversal");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);
    let sentinel = dir.join("keep");
    std::fs::create_dir_all(&sentinel).expect("创建哨兵目录");
    std::fs::write(sentinel.join("file"), b"keep").expect("写入哨兵");

    let index = dir.join("data/backgrounds/nte/cn.json");
    std::fs::create_dir_all(index.parent().expect("索引目录")).expect("创建索引目录");
    std::fs::write(
        &index,
        format!(
            r#"{{"version":1,"archive":true,"dir":"{}","retention":3,"entries":[{{"id":"../../keep","capturedAt":1,"video":null,"cover":null,"images":[]}},{{"id":"..","capturedAt":1,"video":null,"cover":null,"images":[]}}]}}"#,
            entries.display()
        ),
    )
    .expect("写入索引");

    let state = archive::sync_optional(&store, None);
    assert!(state.entries.is_empty());
    assert!(
        sentinel.join("file").is_file(),
        "索引里的路径不能逃出归档目录"
    );
}

#[test]
fn archive_rebuilds_its_directory_after_it_disappears() {
    let dir = TempDir::new("archive-rebuild");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    assert_eq!(state.entries.len(), 1);

    // 目录整个不见了(挂载点被移除/用户清理), 下一次读取要能重建并继续归档.
    std::fs::remove_dir_all(&entries).expect("删除归档目录");
    let state = archive::sync_optional(&store, Some(&first));
    assert_eq!(state.entries.len(), 1);
    assert!(entries.join(".oddity-backgrounds").is_file());
}

#[test]
fn configure_migrates_without_the_current_media() {
    let dir = TempDir::new("archive-migrate-no-live");
    let first_dir = dir.join("archive-first");
    let second_dir = dir.join("archive-second");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&first_dir, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 游戏未安装、解析不到当前素材时, 目录迁移仍要发生.
    let state = archive::configure(&store, None, preferences(&second_dir, 3)).expect("配置归档");
    assert_eq!(state.migration, archive::Migration::Moved(1));
    assert!(second_dir.join(&id).is_dir());
    assert_eq!(state.entries.len(), 1);
}

#[test]
fn archive_readopts_entries_after_the_index_is_rebuilt() {
    let dir = TempDir::new("archive-rebuild-index");
    let entries = dir.join("archive");
    let host = archive_host(&dir);
    let store = store(&host);

    let first = live(&dir, Some(b"v1"), Some(b"i1"));
    let state =
        archive::configure(&store, Some(&first), preferences(&entries, 3)).expect("配置归档");
    let id = state.entries[0].0.clone();

    // 索引损坏后目录还在; 重新指到同一个目录时条目要接回来, 不能算外来文件.
    let index = dir.join("data/backgrounds/nte/cn.json");
    std::fs::write(&index, b"{ broken").expect("写坏索引");
    let state = archive::configure(&store, None, preferences(&entries, 3)).expect("配置归档");
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].0, id);
}
