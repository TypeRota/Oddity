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
