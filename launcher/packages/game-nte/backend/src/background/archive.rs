//! 背景素材归档: 官方启动器换下来的视频与壁纸按内容哈希留在本地.
//!
//! 官方启动器原地覆盖 `bgimgs` 下的同名文件, 只能按内容判断一套素材是否换过.
//! 归档只在读取背景时顺带进行, 任何失败都不影响当前素材的展示.
//! 归档偏好与条目索引一起保存在用户数据目录, 不占用宿主设置字段.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use oddity_platform::clock;
use oddity_platform::hash::{md5_bytes, md5_file};
use oddity_runtime::HostContext;
use serde::{Deserialize, Serialize};

use super::LauncherBackground;

/// 索引格式版本; 高于本值的索引不读取, 避免旧版本覆盖新格式.
const INDEX_VERSION: u32 = 1;
/// 条目自身的元数据文件.
const ENTRY_META: &str = "entry.json";
/// 归档目录属于自己的标记; 挂载点掉线时目录还在但没有它.
const MARKER: &str = ".oddity-backgrounds";
/// 官方清单的文件名, 归档时随素材一起保存.
const MANIFEST: &str = "config.json";
const DEFAULT_RETENTION: u32 = 3;
const MIN_RETENTION: u32 = 2;
const MAX_RETENTION: u32 = 10;
const MAX_DIR_CHARS: usize = 1024;
/// 命令在同一进程里并发读改写索引, 按顺序落盘才不会丢条目.
static INDEX_LOCK: Mutex<()> = Mutex::new(());

/// 归档在数据目录里的固定位置; 索引不随条目目录迁移.
pub(crate) struct Store {
    index: PathBuf,
    default_dir: PathBuf,
}

/// 归档偏好; 与索引一起保存, 由界面完整写回.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preferences {
    pub archive: bool,
    pub dir: String,
    pub retention: u32,
}

/// 归档的运行参数.
struct Config {
    enabled: bool,
    /// 条目目录; 自定义目录与默认目录共用同一套索引.
    entries: PathBuf,
    index: PathBuf,
    retention: u32,
}

/// 一套素材的条目记录.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    captured_at: u64,
    video: Option<String>,
    cover: Option<String>,
    images: Vec<String>,
}

impl Entry {
    /// 条目记录的文件都已存在时才算有效.
    fn files(&self, dir: &Path) -> Option<Vec<PathBuf>> {
        if !self.valid() {
            return None;
        }
        let root = dir.join(&self.id);
        let mut files = Vec::new();
        for name in self.names() {
            files.push(root.join(name));
        }
        files.iter().all(|file| file.is_file()).then_some(files)
    }

    /// 条目引用的文件名; 非单层文件名视为损坏, 不能让归档把路径引到目录外.
    fn names(&self) -> Vec<&str> {
        self.video
            .iter()
            .chain(self.cover.iter())
            .chain(self.images.iter())
            .map(String::as_str)
            .collect()
    }

    /// id 与引用的文件名都必须是单层, 索引里的路径不能离开归档目录.
    fn valid(&self) -> bool {
        single_component(&self.id) && self.names().iter().all(|name| single_component(name))
    }

    fn media(&self, dir: &Path) -> Media {
        if !self.valid() {
            return Media::default();
        }
        let root = dir.join(&self.id);
        Media {
            video: self
                .video
                .as_ref()
                .map(|name| root.join(name))
                .filter(|path| path.is_file()),
            // 封面单独丢失时只丢缩略图, 视频仍然可用.
            image: self
                .cover
                .as_ref()
                .map(|name| root.join(name))
                .filter(|path| path.is_file()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Index {
    version: u32,
    /// 条目当前所在的目录, 迁移失败时保持上一个可用值.
    dir: Option<String>,
    /// 界面填写的归档目录; 空值表示使用默认目录.
    custom: Option<String>,
    selected: Option<String>,
    archive: bool,
    retention: u32,
    entries: Vec<Entry>,
}

/// 一套素材的可播放路径.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Media {
    pub video: Option<PathBuf>,
    pub image: Option<PathBuf>,
}

/// 归档状态; 渲染层与设置界面共用这一份.
#[derive(Debug)]
pub(crate) struct State {
    pub available: bool,
    pub entries: Vec<(String, Media)>,
    pub selected: Option<String>,
    pub current: Option<String>,
    pub migration: Migration,
    pub archive: bool,
    pub dir: String,
    pub retention: u32,
}

impl State {
    /// 索引暂时读不了时的空状态; 界面据此显示未启用.
    fn unavailable() -> Self {
        Self {
            available: false,
            entries: Vec::new(),
            selected: None,
            current: None,
            migration: Migration::Idle,
            archive: false,
            dir: String::new(),
            retention: DEFAULT_RETENTION,
        }
    }

    /// 当前选择的那套素材; 选择失效时回到官方当前素材.
    pub(crate) fn selected_media(&self) -> Option<&Media> {
        let selected = self.selected.as_deref()?;
        self.entries
            .iter()
            .find(|(id, _)| id == selected)
            .map(|(_, media)| media)
    }
}

/// 目录迁移结果; 界面据此决定是否提示.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Migration {
    Idle,
    Moved(usize),
    Failed(&'static str),
}

impl Migration {
    pub(crate) fn state(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Moved(_) => "moved",
            Self::Failed(_) => "failed",
        }
    }

    /// 失败原因; 界面直接展示给用户.
    pub(crate) fn reason(self) -> Option<&'static str> {
        match self {
            Self::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    pub(crate) fn moved(self) -> usize {
        match self {
            Self::Moved(moved) => moved,
            _ => 0,
        }
    }
}

/// 数据目录不可用时归档整体不可用.
pub(crate) fn store(host: &dyn HostContext) -> Option<Store> {
    let channel = host
        .settings()
        .map(|settings| settings.distribution)
        .unwrap_or_else(|_| "cn".into());
    let data = host.data_dir()?;
    let root = data.join("backgrounds").join("nte");
    Some(Store {
        default_dir: root.join(&channel),
        index: root.join(format!("{channel}.json")),
    })
}

fn config(store: &Store, index: &Index) -> Config {
    let target = custom_dir(index.custom.as_deref().unwrap_or_default())
        .unwrap_or_else(|| store.default_dir.clone());
    Config {
        enabled: index.archive,
        entries: target,
        index: store.index.clone(),
        retention: retention(index.retention),
    }
}

fn retention(configured: u32) -> u32 {
    let configured = if configured == 0 {
        DEFAULT_RETENTION
    } else {
        configured
    };
    configured.clamp(MIN_RETENTION, MAX_RETENTION)
}

/// 自定义目录只接受绝对路径与 `~` 开头的写法; 其余写法由调用方报错.
fn custom_dir(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw.chars().count() > MAX_DIR_CHARS {
        return None;
    }
    let expanded = match raw {
        "~" => home_dir(),
        rest if rest.starts_with("~/") => home_dir().join(&rest[2..]),
        rest => PathBuf::from(rest),
    };
    if !expanded.is_absolute() {
        return None;
    }
    // `..` 与 `.` 会让归档目录落在意料之外的位置.
    expanded
        .components()
        .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        .then_some(expanded)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// 读取索引; `None` 表示这次不能安全读写索引.
fn load_index(path: &Path) -> Option<Index> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(Index::default());
        }
        // 读不出来时分不清是权限还是瞬时故障, 这次先不动索引.
        Err(_) => return None,
    };
    match serde_json::from_str::<RawIndex>(&text) {
        Ok(raw) if raw.version <= INDEX_VERSION => Some(sanitize(raw)),
        // 新版本写的索引不能按旧格式覆盖, 交给用户升级启动器.
        Ok(_) => None,
        Err(_) => {
            // 坏文件带时间戳留档再重建, 避免下一次写入把现场覆盖掉.
            let stamp = clock::unix_seconds();
            let _ = fs::rename(path, path.with_extension(format!("json.corrupt-{stamp}")));
            Some(Index::default())
        }
    }
}

/// 单条记录损坏时只丢这一条, 不放弃整份索引.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawIndex {
    version: u32,
    dir: Option<String>,
    custom: Option<String>,
    selected: Option<String>,
    archive: bool,
    retention: u32,
    entries: Vec<serde_json::Value>,
}

fn sanitize(raw: RawIndex) -> Index {
    Index {
        version: INDEX_VERSION,
        dir: raw.dir,
        custom: raw.custom,
        selected: raw.selected,
        archive: raw.archive,
        retention: raw.retention,
        entries: raw
            .entries
            .into_iter()
            .filter_map(|value| serde_json::from_value::<Entry>(value).ok())
            // 路径穿越或字段损坏的条目直接丢弃, 不碰磁盘.
            .filter(Entry::valid)
            .collect(),
    }
}

fn save_index(path: &Path, index: &Index) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut next = index.clone();
    next.version = INDEX_VERSION;
    // 先写同目录临时文件再改名, 中途退出不会留下半个索引.
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(&next)?)?;
    fs::rename(&temp, path)
}

/// 读取当前素材并归档, 返回可展示的条目列表.
pub(crate) fn sync_optional(store: &Store, live: Option<&LauncherBackground>) -> State {
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    sync_optional_locked(store, live)
}

fn sync_optional_locked(store: &Store, live: Option<&LauncherBackground>) -> State {
    let Some(index) = load_index(&store.index) else {
        return State::unavailable();
    };
    let config = config(store, &index);
    if !config.enabled {
        let mut index = index;
        // 归档关闭或索引刚重建时, 目录里的条目仍要接回索引.
        if adopt_orphans(&config, &mut index) {
            let _ = save_index(&store.index, &index);
        }
        return state(&config, &index, None, Migration::Idle);
    }
    sync(&config, &index, live)
}

fn sync(config: &Config, index: &Index, live: Option<&LauncherBackground>) -> State {
    let mut index = index.clone();
    let dir_before = index.dir.clone();
    let migration = ensure_dir(config, &mut index);
    if let Migration::Failed(_) = migration {
        // 迁移没成功时这一轮什么都不改, 保持旧目录可用.
        return state(config, &index, None, migration);
    }
    let mut changed = index.dir != dir_before;
    changed |= adopt_orphans(config, &mut index);
    // 目录看起来被卸载时只跳过清理, 归档仍会重建目录并写标记.
    let owned = owned(config, &index);
    changed |= owned && drop_missing(config, &mut index);
    let before = index.entries.len();
    let current = live.and_then(|live| archive_live(config, &mut index, live));
    changed |= index.entries.len() != before;
    if owned {
        changed |= prune(config, &mut index, current.as_deref());
    }
    sweep_temp(&config.entries);
    if index.selected.is_some()
        && !index
            .entries
            .iter()
            .any(|entry| Some(&entry.id) == index.selected.as_ref())
    {
        index.selected = None;
        changed = true;
    }
    if changed || migration != Migration::Idle {
        let _ = save_index(&config.index, &index);
    }
    state(config, &index, current, migration)
}

/// 清理崩溃留下的临时条目目录.
fn sweep_temp(base: &Path) {
    let Ok(dir) = fs::read_dir(base) else {
        return;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(".tmp-") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 3600);
        if stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// 写入归档偏好并立即生效; 预设置空表示使用默认目录.
pub(crate) fn configure(
    store: &Store,
    live: Option<&LauncherBackground>,
    preferences: Preferences,
) -> Result<State, String> {
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let dir = preferences.dir.trim().to_string();
    let custom = if dir.is_empty() {
        None
    } else {
        let path = custom_dir(&dir).ok_or_else(|| "归档目录必须是绝对路径".to_string())?;
        if path.exists() && !path.is_dir() {
            return Err("归档目录是一个文件".into());
        }
        Some(dir)
    };
    let Some(mut index) = load_index(&store.index) else {
        return Err("归档索引来自更新版本的启动器".into());
    };
    let previous = index.clone();
    index.archive = preferences.archive;
    index.custom = custom;
    index.retention = retention(preferences.retention);
    save_index(&store.index, &index).map_err(|error| error.to_string())?;
    let config = config(store, &index);
    let next = if config.enabled {
        sync(&config, &index, live)
    } else {
        state(&config, &index, None, Migration::Idle)
    };
    if let Migration::Failed(reason) = next.migration {
        // 迁移没成功就保持旧目录, 不把界面上的新路径当成已生效.
        let _ = save_index(&store.index, &previous);
        return Err(reason.to_string());
    }
    Ok(next)
}

fn state(config: &Config, index: &Index, current: Option<String>, migration: Migration) -> State {
    let base = root(config, index);
    State {
        available: true,
        entries: if config.enabled {
            index
                .entries
                .iter()
                .map(|entry| (entry.id.clone(), entry.media(&base)))
                .collect()
        } else {
            Vec::new()
        },
        selected: if config.enabled {
            index.selected.clone()
        } else {
            None
        },
        current,
        migration,
        archive: config.enabled,
        dir: index.custom.clone().unwrap_or_default(),
        retention: config.retention,
    }
}

/// 选择一套归档素材; `None` 表示跟随官方当前素材.
pub(crate) fn select(
    store: &Store,
    live: Option<&LauncherBackground>,
    id: Option<&str>,
) -> Result<State, String> {
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(mut index) = load_index(&store.index) else {
        return Err("归档索引来自更新版本的启动器".into());
    };
    let config = config(store, &index);
    if !config.enabled {
        return Err("背景素材归档未启用".into());
    }
    if let Some(id) = id {
        if !index.entries.iter().any(|entry| entry.id == id) {
            return Err("找不到该背景素材".into());
        }
    }
    index.selected = id.map(str::to_string);
    save_index(&store.index, &index).map_err(|error| error.to_string())?;
    Ok(sync_optional_locked(store, live))
}

/// 目录里是否已经有归档的东西; 只有这种情况才允许按缺失清理条目.
fn owned(config: &Config, index: &Index) -> bool {
    if index.entries.is_empty() {
        return true;
    }
    let base = root(config, index);
    base.join(MARKER).is_file()
        || index
            .entries
            .iter()
            .any(|entry| base.join(&entry.id).is_dir())
}

/// 把目录里存在但索引里没有的条目接回索引.
///
/// 索引写盘失败或损坏重建后, 条目目录还在, 不能因此变成"外来目录".
fn adopt_orphans(config: &Config, index: &mut Index) -> bool {
    let base = root(config, index);
    let Ok(dir) = fs::read_dir(&base) else {
        return false;
    };
    let mut added = false;
    for entry in dir.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(read) = read_entry(&entry.path()) else {
            continue;
        };
        if index.entries.iter().any(|known| known.id == read.id) {
            continue;
        }
        index.entries.push(read);
        added = true;
    }
    added
}

/// 条目实际所在的目录; 迁移失败时仍是上一个可用目录.
fn root(config: &Config, index: &Index) -> PathBuf {
    index
        .dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| config.entries.clone())
}

/// 把当前素材按内容哈希放进归档; 已存在时不动.
fn archive_live(config: &Config, index: &mut Index, live: &LauncherBackground) -> Option<String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut seen = BTreeSet::new();
    let sources = live
        .video
        .iter()
        .chain(live.images.iter())
        .cloned()
        .collect::<Vec<_>>();
    for source in sources {
        let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name == MANIFEST || name == ENTRY_META || !seen.insert(name.to_string()) {
            continue;
        }
        files.push((name.to_string(), source));
    }
    if files.is_empty() {
        return None;
    }
    let base = root(config, index);
    let present = |id: &str| {
        index
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .is_some_and(|entry| entry.files(&base).is_some())
    };
    // 官方启动器原地覆盖同名文件, 只能按内容判断是否换过.
    let id = content_id(&files)?;
    if present(&id) {
        return Some(id.clone());
    }
    // 条目记录还在但文件已经没了: 丢掉旧记录按当前素材重写.
    index.entries.retain(|entry| entry.id != id);
    let entry = write_entry(&base, live, &id, &files)?;
    index.entries.push(entry);
    Some(id)
}

/// 条目目录里的文件是否正好是这一套素材.
fn matches_files(entry: &Entry, files: &[(String, PathBuf)]) -> bool {
    let mut names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    let mut stored: Vec<&str> = entry
        .video
        .iter()
        .chain(entry.images.iter())
        .map(String::as_str)
        .collect();
    stored.sort_unstable();
    names == stored
}

/// 从条目目录读回元数据; 目录是上次写完还没落索引时用它接回.
fn read_entry(dir: &Path) -> Option<Entry> {
    let text = fs::read_to_string(dir.join(ENTRY_META)).ok()?;
    let entry: Entry = serde_json::from_str(&text).ok()?;
    entry.files(dir.parent()?).map(|_| entry)
}

/// 只接受单层文件名, 归档条目不能引用目录外的路径.
fn single_component(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// 一套素材的身份: 归档文件名与内容哈希的组合.
fn content_id(files: &[(String, PathBuf)]) -> Option<String> {
    let mut parts = Vec::new();
    for (name, path) in files {
        parts.push(format!("{name}={}", md5_file(path).ok()?));
    }
    parts.sort();
    Some(md5_bytes(parts.join("\n").as_bytes()))
}

fn write_entry(
    base: &Path,
    live: &LauncherBackground,
    id: &str,
    files: &[(String, PathBuf)],
) -> Option<Entry> {
    let target = base.join(id);
    if target.is_dir() {
        // 目录是上次写完条目还没落索引留下的, 内容对得上就直接接回来.
        if let Some(entry) =
            read_entry(&target).filter(|entry| entry.id == id && matches_files(entry, files))
        {
            return Some(entry);
        }
        let _ = fs::remove_dir_all(&target);
    }
    fs::create_dir_all(base).ok()?;
    let _ = fs::write(base.join(MARKER), b"oddity");
    let temp = base.join(format!(".tmp-{id}"));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).ok()?;
    let mut images = Vec::new();
    let mut video = None;
    for (name, source) in files {
        let destination = temp.join(name);
        if fs::copy(source, &destination).is_err() {
            let _ = fs::remove_dir_all(&temp);
            return None;
        }
        if Some(source) == live.video.as_ref() {
            video = Some(name.clone());
        } else {
            images.push(name.clone());
        }
    }
    if let Some(manifest) = &live.manifest {
        let _ = fs::write(temp.join(MANIFEST), manifest);
    }
    let cover = live
        .image
        .as_ref()
        .and_then(|image| image.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| images.iter().any(|image| image == name))
        .map(str::to_string)
        .or_else(|| images.first().cloned());
    let entry = Entry {
        id: id.to_string(),
        captured_at: clock::unix_millis(),
        video,
        cover,
        images,
    };
    fs::write(
        temp.join(ENTRY_META),
        serde_json::to_vec_pretty(&entry).ok()?,
    )
    .ok()?;
    fs::rename(&temp, &target).ok()?;
    Some(entry)
}

/// 文件缺失的条目静默移除, 连同残留目录.
fn drop_missing(config: &Config, index: &mut Index) -> bool {
    let base = root(config, index);
    // 归档目录整体不可达时保留条目, 等它回来再判断.
    if !base.is_dir() {
        return false;
    }
    let mut removed = Vec::new();
    index.entries.retain(|entry| {
        if entry.files(&base).is_some() {
            return true;
        }
        removed.push(entry.id.clone());
        false
    });
    for id in &removed {
        let _ = fs::remove_dir_all(base.join(id));
    }
    !removed.is_empty()
}

/// 只保留最近几套; 正在使用的一套不参与剪枝.
fn prune(config: &Config, index: &mut Index, current: Option<&str>) -> bool {
    let retention = config.retention as usize;
    if index.entries.len() <= retention {
        return false;
    }
    let base = root(config, index);
    let selected = index.selected.clone();
    let mut order: Vec<(u64, String)> = index
        .entries
        .iter()
        .map(|entry| (entry.captured_at, entry.id.clone()))
        .collect();
    order.sort_by(|left, right| right.cmp(left));
    let mut kept = BTreeSet::new();
    // 正在使用的两套先占名额, 其余按新旧补满.
    for id in [selected.as_deref(), current].into_iter().flatten() {
        if !index.entries.iter().any(|entry| entry.id == id) {
            continue;
        }
        kept.insert(id.to_string());
    }
    for (_, id) in order {
        if kept.len() >= retention {
            break;
        }
        kept.insert(id);
    }
    let mut removed = Vec::new();
    index.entries.retain(|entry| {
        if kept.contains(&entry.id) {
            return true;
        }
        removed.push(entry.id.clone());
        false
    });
    for id in &removed {
        let _ = fs::remove_dir_all(base.join(id));
    }
    !removed.is_empty()
}

/// 目录变化时把条目搬到新位置; 索引留在数据目录, 不跟着搬.
fn ensure_dir(config: &Config, index: &mut Index) -> Migration {
    let target = config.entries.clone();
    let previous = index.dir.as_deref().map(PathBuf::from);
    if previous.as_deref() == Some(target.as_path()) {
        return Migration::Idle;
    }
    let Some(previous) = previous else {
        index.dir = Some(target.display().to_string());
        return Migration::Idle;
    };
    if index.entries.is_empty() {
        index.dir = Some(target.display().to_string());
        return Migration::Idle;
    }
    if !previous.is_dir() {
        // 旧目录已经没了, 条目本来就读不到, 交给缺失清理.
        index.dir = Some(target.display().to_string());
        return Migration::Idle;
    }
    // 上次已经搬完但索引没来得及落盘: 目标目录完整就直接指过去.
    if target.join(MARKER).is_file()
        && index
            .entries
            .iter()
            .all(|entry| entry.files(&target).is_some())
    {
        index.dir = Some(target.display().to_string());
        return Migration::Idle;
    }
    // 旧目录看起来在, 但一个条目都没有: 多半是挂载点掉线, 不能把索引指过去.
    if !owned(config, index) {
        return Migration::Failed("归档目录暂时不可用, 请检查磁盘或挂载");
    }
    let existed = target.exists();
    if existed {
        match has_foreign_entries(&target, index) {
            Ok(true) => return Migration::Failed("目标目录里已有其他文件, 请先清空"),
            Ok(false) => {}
            Err("permission") => return Migration::Failed("没有权限读取归档目录"),
            Err(_) => return Migration::Failed("无法读取归档目录"),
        }
    }
    if fs::create_dir_all(&target).is_err() {
        return Migration::Failed("无法创建归档目录");
    }
    if let Err(error) = fs::write(target.join(MARKER), b"oddity") {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            return Migration::Failed("没有权限写入归档目录");
        }
        return Migration::Failed("无法写入归档目录");
    }
    let mut moved = 0;
    for entry in &index.entries {
        let source = previous.join(&entry.id);
        let destination = target.join(&entry.id);
        if !source.is_dir() {
            continue;
        }
        // 上次中断时已经搬完的条目直接算完成; 半成品删掉重拷.
        if entry.files(&target).is_some() {
            continue;
        }
        if destination.is_dir() {
            let _ = fs::remove_dir_all(&destination);
        }
        if move_dir(&source, &destination).is_err() {
            // 把已经搬过去的条目搬回来, 保持索引与旧目录一致.
            for entry in &index.entries {
                let destination = target.join(&entry.id);
                if destination.is_dir() {
                    let _ = move_dir(&destination, &previous.join(&entry.id));
                }
            }
            if !existed {
                let _ = fs::remove_file(target.join(MARKER));
                let _ = fs::remove_dir(&target);
            }
            return Migration::Failed("移动归档素材失败, 请检查目录权限");
        }
        moved += 1;
    }
    // 旧目录只剩下自己的标记时才算清空; 挂载点掉线时不能删.
    if previous.join(MARKER).is_file() {
        let _ = fs::remove_file(previous.join(MARKER));
        let _ = fs::remove_dir(&previous);
    }
    index.dir = Some(target.display().to_string());
    if moved == 0 {
        Migration::Idle
    } else {
        Migration::Moved(moved)
    }
}

/// 目标目录里是否已经有别的东西; 归档自己的条目目录与临时目录不算.
fn has_foreign_entries(target: &Path, index: &Index) -> Result<bool, &'static str> {
    let ids: BTreeSet<&str> = index
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    let dir = fs::read_dir(target).map_err(|error| match error.kind() {
        std::io::ErrorKind::PermissionDenied => "permission",
        _ => "io",
    })?;
    Ok(dir.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if ids.contains(name.as_ref()) || name.starts_with(".tmp-") || name == MARKER {
            return false;
        }
        // 上次中断留下的合法条目目录也是自己的.
        read_entry(&entry.path()).is_none()
    }))
}

/// 同文件系统直接改名, 跨文件系统逐个复制后删除源目录.
fn move_dir(source: &Path, target: &Path) -> std::io::Result<()> {
    if fs::rename(source, target).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(std::io::Error::other("归档目录包含非普通文件"));
        }
        fs::copy(entry.path(), target.join(entry.file_name()))?;
    }
    fs::remove_dir_all(source)
}
