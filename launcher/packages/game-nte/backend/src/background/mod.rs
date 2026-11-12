//! 官方启动器自带的本地动态背景.
//!
//! 只读取官方启动器安装目录里的视频与静态壁纸, 不复制也不随 Oddity 分发.
//! 安装位置探测属于宿主的安装域; 本域只消费宿主给出的启动器根目录.

use std::path::{Component, Path, PathBuf};

use oddity_runtime::HostContext;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 背景域注册到宿主命令组里的命令 id.
pub(crate) const COMMANDS: &[&str] = &["launcher_background"];

/// 官方资源根目录, 相对启动器根目录.
const RES_DIR: &str = "ResFilesM";
/// 背景素材目录名, 位于 `ResFilesM/<appId>/` 下.
const BACKGROUND_DIR: &str = "bgimgs";
/// 国服的 GameID 与启动器目录名.
const CN_APP_ID: i64 = 1289;
const CN_LAUNCHER_DIR: &str = "NTELauncher";
/// 国际服的 GameID 与启动器目录名.
const GLOBAL_APP_ID: i64 = 3_000_001;
const GLOBAL_LAUNCHER_DIR: &str = "NTEGlobal";

/// 官方启动器提供的本地背景素材.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LauncherBackground {
    video: Option<PathBuf>,
    image: Option<PathBuf>,
}

/// 返回给渲染层的形状, 与宿主原先的 `LauncherBackgroundMedia` 逐字段一致.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    video: Option<String>,
    image: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct BackgroundManifest {
    #[serde(default)]
    imgs: Vec<BackgroundImage>,
    #[serde(default)]
    video: Option<String>,
    #[serde(default, rename = "noVideoBg")]
    no_video_bg: Option<String>,
    #[serde(default)]
    first: i64,
}

#[derive(Debug, Deserialize)]
struct BackgroundImage {
    file: String,
}

/// 在官方启动器根目录中查找视频和静态壁纸.
fn resolve(launcher_root: &Path) -> Option<LauncherBackground> {
    // 配置里写明 GameID 时只认该区服的目录, 不再搜索其他区服.
    if let Some(app_id) = read_app_id(launcher_root) {
        return read_background(&background_dir(launcher_root, app_id));
    }

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(launcher_root.join(RES_DIR))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(BACKGROUND_DIR))
        .collect();
    candidates.sort();
    candidates.into_iter().find_map(|dir| read_background(&dir))
}

fn background_dir(launcher_root: &Path, app_id: i64) -> PathBuf {
    launcher_root
        .join(RES_DIR)
        .join(app_id.to_string())
        .join(BACKGROUND_DIR)
}

/// 官方 Config.ini 里的 GameID, 缺省时按启动器目录名区分区服.
fn read_app_id(launcher_root: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(launcher_root.join("Config").join("Config.ini")).ok()?;
    let app_id = match config_game_id(&text).unwrap_or(0) {
        0 => launcher_dir_app_id(launcher_root),
        configured => configured,
    };
    (app_id > 0).then_some(app_id)
}

fn launcher_dir_app_id(launcher_root: &Path) -> i64 {
    match launcher_root.file_name() {
        Some(name) if name == GLOBAL_LAUNCHER_DIR => GLOBAL_APP_ID,
        Some(name) if name == CN_LAUNCHER_DIR => CN_APP_ID,
        _ => 0,
    }
}

/// 只取 `[General] GameID`; 官方配置的其余字段与本域无关.
fn config_game_id(text: &str) -> Option<i64> {
    let mut section = "General";
    let mut game_id = None;
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.trim();
            continue;
        }
        if section != "General" {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "GameID" {
                game_id = value.trim().parse().ok();
            }
        }
    }
    game_id
}

fn read_background(dir: &Path) -> Option<LauncherBackground> {
    if !dir.is_dir() {
        return None;
    }
    let manifest = std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<BackgroundManifest>(&text).ok())
        .unwrap_or_default();
    let video = resolve_video(dir, &manifest);
    let image = resolve_image(dir, &manifest);
    (video.is_some() || image.is_some()).then_some(LauncherBackground { video, image })
}

fn resolve_video(dir: &Path, manifest: &BackgroundManifest) -> Option<PathBuf> {
    manifest
        .video
        .as_deref()
        .and_then(|name| asset_path(dir, name))
        .or_else(|| asset_path(dir, "bg.mp4"))
}

fn resolve_image(dir: &Path, manifest: &BackgroundManifest) -> Option<PathBuf> {
    let index = if manifest.first >= 0 && (manifest.first as usize) < manifest.imgs.len() {
        manifest.first as usize
    } else {
        0
    };
    manifest
        .imgs
        .get(index)
        .or_else(|| manifest.imgs.first())
        .and_then(|image| asset_path(dir, &image.file))
        .or_else(|| {
            manifest
                .no_video_bg
                .as_deref()
                .and_then(|name| asset_path(dir, name))
        })
        .or_else(|| asset_path(dir, "bg_0.png"))
}

/// 只接受背景目录下的单层文件名, 配置无法把路径引到目录外.
fn asset_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return None;
    }
    let path = dir.join(name);
    path.is_file().then_some(path)
}

/// 分发背景域命令; 参数为空, 结果形状与宿主实现一致.
pub(crate) async fn command(
    host: &dyn HostContext,
    command: &str,
    _args: Value,
) -> std::result::Result<Value, Value> {
    match command {
        "launcher_background" => launcher_background(host).await,
        _ => Err(json!("无法识别的背景命令")),
    }
}

async fn launcher_background(host: &dyn HostContext) -> std::result::Result<Value, Value> {
    let media = match host.official_launcher_root() {
        Some(root) => {
            // 扫描背景目录会读文件系统, 和公告抓取一样移出异步线程.
            let background = tokio::task::spawn_blocking(move || resolve(&root))
                .await
                .map_err(|error| json!(error.to_string()))?;
            background.map_or_else(Media::default, |background| Media {
                video: canonical_path(background.video),
                image: canonical_path(background.image),
            })
        }
        None => Media::default(),
    };
    serde_json::to_value(media).map_err(|error| json!(error.to_string()))
}

/// 素材路径先规范化, 界面拿到的是可以直接播放的绝对路径.
fn canonical_path(path: Option<PathBuf>) -> Option<String> {
    path.and_then(|path| std::fs::canonicalize(path).ok())
        .map(|path| path.display().to_string())
}

#[cfg(test)]
mod tests;
