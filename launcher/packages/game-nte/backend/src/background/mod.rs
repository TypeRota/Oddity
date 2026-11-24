//! 官方启动器自带的本地动态背景.
//!
//! 只读取官方启动器安装目录里的视频与静态壁纸, 不随 Oddity 分发.
//! 安装位置探测属于宿主的安装域; 本域只消费宿主给出的启动器根目录.
//! 开启归档后, 换下来的素材按内容哈希留在本地, 供用户切换.

use std::path::{Component, Path, PathBuf};

use oddity_runtime::HostContext;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod archive;

/// 背景域注册到宿主命令组里的命令 id.
pub(crate) const COMMANDS: &[&str] = &[
    "launcher_background",
    "launcher_background_archive",
    "launcher_background_select",
    "launcher_background_configure",
];

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
    /// 清单里的全部静态图, 归档时整保存.
    images: Vec<PathBuf>,
    /// 清单原文; 读不到时为空.
    manifest: Option<String>,
}

/// 返回给渲染层的形状, 与宿主原先的 `LauncherBackgroundMedia` 逐字段一致.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    video: Option<String>,
    image: Option<String>,
}

/// 归档列表里的一个条目; 静态图用于界面缩略图.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveEntry {
    id: String,
    video: Option<String>,
    image: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveMigration {
    state: &'static str,
    moved: usize,
    reason: Option<&'static str>,
}

/// 归档列表与当前选择; 界面据此渲染缩略图并标记使用中的一套.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveView {
    available: bool,
    entries: Vec<ArchiveEntry>,
    selected: Option<String>,
    current: Option<String>,
    migration: ArchiveMigration,
    archive: bool,
    dir: String,
    retention: u32,
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
    let manifest_text = std::fs::read_to_string(dir.join("config.json")).ok();
    let manifest = manifest_text
        .as_deref()
        .and_then(|text| serde_json::from_str::<BackgroundManifest>(text).ok())
        .unwrap_or_default();
    let video = resolve_video(dir, &manifest);
    let image = resolve_image(dir, &manifest);
    (video.is_some() || image.is_some()).then(|| LauncherBackground {
        images: resolve_images(dir, &manifest, image.as_ref()),
        video,
        image,
        manifest: manifest_text,
    })
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

/// 清单里能读到的全部静态图; 归档时一套素材整保存.
fn resolve_images(
    dir: &Path,
    manifest: &BackgroundManifest,
    selected: Option<&PathBuf>,
) -> Vec<PathBuf> {
    let mut images: Vec<PathBuf> = manifest
        .imgs
        .iter()
        .filter_map(|image| asset_path(dir, &image.file))
        .collect();
    if images.is_empty() {
        if let Some(selected) = selected {
            images.push(selected.clone());
        }
    }
    images.dedup();
    images
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

/// 分发背景域命令; 结果形状与宿主实现一致.
pub(crate) async fn command(
    host: &dyn HostContext,
    command: &str,
    args: Value,
) -> std::result::Result<Value, Value> {
    match command {
        "launcher_background" => launcher_background(host).await,
        "launcher_background_archive" => archive_view(host, None, None).await,
        "launcher_background_select" => {
            let id = select_id(&args)?;
            archive_view(host, Some(id), None).await
        }
        "launcher_background_configure" => {
            let preferences = preferences(&args)?;
            archive_view(host, None, Some(preferences)).await
        }
        _ => Err(json!("无法识别的背景命令")),
    }
}

/// 选择命令的参数: `id` 为空表示跟随官方当前素材.
fn select_id(args: &Value) -> std::result::Result<String, Value> {
    match args.get("id") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(id)) => Ok(id.clone()),
        _ => Err(json!("无效的背景素材")),
    }
}

/// 归档偏好由界面整份写回, 缺失字段沿用默认值.
fn preferences(args: &Value) -> std::result::Result<archive::Preferences, Value> {
    let archive = args
        .get("archive")
        .and_then(Value::as_bool)
        .ok_or_else(|| json!("无效的归档设置"))?;
    let dir = args
        .get("dir")
        .and_then(Value::as_str)
        .ok_or_else(|| json!("无效的归档目录"))?
        .to_string();
    let retention = args
        .get("retention")
        .and_then(Value::as_u64)
        .ok_or_else(|| json!("无效的保留套数"))? as u32;
    Ok(archive::Preferences {
        archive,
        dir,
        retention,
    })
}

async fn launcher_background(host: &dyn HostContext) -> std::result::Result<Value, Value> {
    let store = archive::store(host);
    let media = match host.official_launcher_root() {
        Some(root) => {
            // 扫描背景目录与归档都要读文件系统, 和公告抓取一样移出异步线程.
            let (live, state) = tokio::task::spawn_blocking(move || {
                let live = resolve(&root);
                let state = store
                    .as_ref()
                    .map(|store| archive::sync_optional(store, live.as_ref()));
                (live, state)
            })
            .await
            .map_err(|error| json!(error.to_string()))?;
            let fallback = live.map_or_else(Media::default, |live| Media {
                video: canonical_path(live.video),
                image: canonical_path(live.image),
            });
            match state.as_ref().and_then(|state| state.selected_media()) {
                // 归档目录掉线时路径无法规范化, 这时仍显示官方当前素材.
                Some(media) => match archive_media(media) {
                    archived if archived.video.is_none() && archived.image.is_none() => fallback,
                    archived => archived,
                },
                None => fallback,
            }
        }
        None => Media::default(),
    };
    serde_json::to_value(media).map_err(|error| json!(error.to_string()))
}

/// 归档列表; 传入选中的 id 或偏好时先写入再返回最新状态.
async fn archive_view(
    host: &dyn HostContext,
    selection: Option<String>,
    preferences: Option<archive::Preferences>,
) -> std::result::Result<Value, Value> {
    let store = archive::store(host);
    let root = host.official_launcher_root();
    let state = tokio::task::spawn_blocking(move || {
        let store = store.ok_or_else(|| "背景素材归档不可用".to_string())?;
        let live = root.as_deref().and_then(resolve);
        match (preferences, selection) {
            (Some(preferences), _) => archive::configure(&store, live.as_ref(), preferences),
            (None, Some(id)) => {
                let id = (!id.is_empty()).then_some(id.as_str());
                archive::select(&store, live.as_ref(), id)
            }
            (None, None) => Ok(archive::sync_optional(&store, live.as_ref())),
        }
    })
    .await
    .map_err(|error| json!(error.to_string()))?
    .map_err(|error| json!(error))?;
    let view = ArchiveView {
        available: state.available,
        entries: state
            .entries
            .iter()
            .map(|(id, media)| ArchiveEntry {
                id: id.clone(),
                video: canonical_path(media.video.clone()),
                image: canonical_path(media.image.clone()),
            })
            .collect(),
        selected: state.selected,
        current: state.current,
        migration: ArchiveMigration {
            state: state.migration.state(),
            moved: state.migration.moved(),
            reason: state.migration.reason(),
        },
        archive: state.archive,
        dir: state.dir,
        retention: state.retention,
    };
    serde_json::to_value(view).map_err(|error| json!(error.to_string()))
}

fn archive_media(media: &archive::Media) -> Media {
    Media {
        video: canonical_path(media.video.clone()),
        image: canonical_path(media.image.clone()),
    }
}

/// 素材路径先规范化, 界面拿到的是可以直接播放的绝对路径.
fn canonical_path(path: Option<PathBuf>) -> Option<String> {
    path.and_then(|path| std::fs::canonicalize(path).ok())
        .map(|path| path.display().to_string())
}

#[cfg(test)]
mod tests;
