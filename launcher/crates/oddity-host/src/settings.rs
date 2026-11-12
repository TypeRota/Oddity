//! 启动器设置及用户目录, 供主程序和应急入口共用.
use crate::channel::ChannelId;
use crate::identity::GameId;
use oddity_platform::Error;
use oddity_runtime::{Registry, SettingsSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// 单个游戏的设置. 渠道与安装目录随游戏走, 其余偏好保持全局.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GameSettings {
    pub audio: oddity_platform::audio::Options,
    /// 当前渠道; 缺省表示该游戏没有渠道或尚未选择.
    pub distribution: Option<ChannelId>,
    /// 按安装根键保存的安装目录; 共享启动器根的渠道共用一个值.
    pub install_roots: BTreeMap<String, String>,
    /// 自定义预下载缓存目录; 空值使用安装目录内的默认路径.
    pub preload_root: Option<String>,
    pub launch_mode: String,
    pub game_account_id: Option<String>,
    pub game_mode: crate::game_mode::Options,
    pub updates: crate::updates::Options,
    pub active_environment: Option<String>,
    pub stable_environment: Option<String>,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            audio: oddity_platform::audio::Options::default(),
            distribution: None,
            install_roots: BTreeMap::new(),
            preload_root: None,
            launch_mode: "background".to_string(),
            game_account_id: None,
            game_mode: crate::game_mode::Options::default(),
            updates: crate::updates::Options::default(),
            active_environment: None,
            stable_environment: None,
        }
    }
}

impl GameSettings {
    fn fallback() -> &'static Self {
        static FALLBACK: std::sync::LazyLock<GameSettings> =
            std::sync::LazyLock::new(GameSettings::default);
        &FALLBACK
    }

    /// 指定安装根键对应的安装目录.
    pub fn install_root_for_key(&self, key: &str) -> Option<&str> {
        self.install_roots.get(key).map(String::as_str)
    }
}

/// 一个 owner 声明的设置 section; 版本与字段值一起保存.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsSectionData {
    pub version: u32,
    pub values: Map<String, Value>,
}

impl Default for SettingsSectionData {
    fn default() -> Self {
        Self {
            version: 1,
            values: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// 界面和服务当前展示的游戏.
    pub active_game: GameId,
    /// 按游戏保存的安装位置与地区.
    pub games: BTreeMap<GameId, GameSettings>,
    pub runtime_environments: Vec<crate::environment::RuntimeEnvironment>,
    pub gamepad: crate::gamepad::Options,
    pub language: String,
    pub reduce_motion: bool,
    pub stack_bulletins: bool,
    pub theme: String,
    pub hero_media: String,
    pub pause_hero_on_blur: bool,
    pub adaptive_theme: bool,
    /// 热力图按最近 12 个月的相对游玩时长着色; 关闭后按固定时长分档.
    pub relative_heatmap: bool,
    pub minimize_to_tray_on_close: bool,
    pub show_launcher_after_game_exit: bool,
    pub snapshot_success_minutes: u64,
    /// 包声明设置 section 的数据; 没有声明 section 时不写入文件.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<String, SettingsSectionData>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_game: GameId::default(),
            games: BTreeMap::from([(GameId::default(), GameSettings::default())]),
            runtime_environments: Vec::new(),
            gamepad: crate::gamepad::Options::default(),
            language: "zh-CN".to_string(),
            reduce_motion: false,
            stack_bulletins: false,
            theme: "light".to_string(),
            hero_media: "video".to_string(),
            pause_hero_on_blur: true,
            adaptive_theme: true,
            relative_heatmap: true,
            minimize_to_tray_on_close: true,
            show_launcher_after_game_exit: true,
            snapshot_success_minutes: crate::snapshots::DEFAULT_SUCCESS_MINUTES,
            sections: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// 指定游戏的设置; 尚未写入时使用默认值.
    pub fn game(&self, game: GameId) -> &GameSettings {
        self.games.get(&game).unwrap_or(GameSettings::fallback())
    }

    pub fn game_mut(&mut self, game: GameId) -> &mut GameSettings {
        self.games.entry(game).or_default()
    }

    /// 当前游戏 (activeGame) 的设置.
    pub fn current(&self) -> &GameSettings {
        self.game(self.active_game.clone())
    }

    pub fn current_mut(&mut self) -> &mut GameSettings {
        self.games.entry(self.active_game.clone()).or_default()
    }

    /// 补齐当前游戏条目, 保证界面总能读到一种设置.
    pub fn normalize(&mut self) {
        self.games.entry(self.active_game.clone()).or_default();
        self.games.entry(GameId::default()).or_default();
    }

    /// 读取设置文件. 旧版顶层游戏字段会迁移到 `games.nte`, 迁移前把原始
    /// 文件备份为 `settings.json.bak`; 文件不存在时返回默认值.
    pub fn load(path: &Path) -> oddity_platform::Result<Self> {
        let original = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
        };
        let mut value: Value = serde_json::from_slice(&original)
            .map_err(|error| Error::Parse(format!("设置文件格式错误: {error}")))?;
        if !value.is_object() {
            return Err(Error::Parse("设置必须是 JSON 对象".into()));
        }
        let migrated = migrate_value(&mut value);
        let mut settings: Settings = serde_json::from_value(value.clone())
            .map_err(|error| Error::Parse(format!("设置字段无效: {error}")))?;
        settings.normalize();
        for game in settings.games.values() {
            game.audio.validate().map_err(Error::Parse)?;
        }
        if migrated {
            // 备份或写入失败时继续使用内存中的迁移结果, 原文件留待下次启动重试.
            if let Err(error) = persist_migration(path, &original, &value) {
                eprintln!("迁移设置文件失败: {error}");
            }
        }
        Ok(settings)
    }
}

/// 已装配包声明的设置 section schema.
#[derive(Debug, Clone, Default)]
pub struct SettingsSchemas {
    entries: Vec<&'static SettingsSchema>,
}

impl SettingsSchemas {
    pub(crate) fn from_registry(registry: &Registry) -> Self {
        Self {
            entries: registry.settings_schemas().collect(),
        }
    }

    pub(crate) fn schemas(&self) -> &[&'static SettingsSchema] {
        &self.entries
    }

    pub(crate) fn get(&self, owner: &str) -> Option<&'static SettingsSchema> {
        self.entries
            .iter()
            .copied()
            .find(|schema| schema.owner == owner)
    }

    /// 补齐默认值并升级旧版本; 返回是否改动内存中的设置.
    pub(crate) fn materialize(&self, settings: &mut Settings) -> Result<bool, String> {
        let mut changed = false;
        for schema in &self.entries {
            let values = section_values(settings, schema)?;
            let next = SettingsSectionData {
                version: schema.version,
                values,
            };
            if settings.sections.get(schema.owner) == Some(&next) {
                continue;
            }
            settings.sections.insert(schema.owner.to_string(), next);
            changed = true;
        }
        Ok(changed)
    }
}

/// 一个 owner 声明的 section 当前值; 不修改设置本身.
pub(crate) fn section_values(
    settings: &Settings,
    schema: &SettingsSchema,
) -> Result<Map<String, Value>, String> {
    let Some(data) = settings.sections.get(schema.owner) else {
        let values = schema.defaults();
        schema.validate_values(&values)?;
        return Ok(values);
    };
    if data.version > schema.version {
        return Err(format!(
            "设置 section {} 的版本 {} 高于声明的 {}",
            schema.owner, data.version, schema.version
        ));
    }
    let mut values = data.values.clone();
    if data.version < schema.version {
        schema.migrate_from(data.version, &mut values)?;
    }
    schema.fill_defaults(&mut values);
    schema.validate_values(&values)?;
    Ok(values)
}

/// 设置 owner 可以引用的游戏 id: 可选择的游戏与声明了设置 section 的游戏.
pub(crate) fn owner_games(registry: &Registry) -> Vec<&'static str> {
    let mut games = registry.selectable_games();
    for game in registry.settings_games() {
        if !games.contains(&game) {
            games.push(game);
        }
    }
    games
}

const LEGACY_KEYS: [&str; 9] = [
    "installRoot",
    "globalInstallRoot",
    "preloadRoot",
    "distribution",
    "launchMode",
    "gameMode",
    "updates",
    "activeEnvironment",
    "stableEnvironment",
];

/// 把旧形状的设置值迁移到当前字段; 返回是否发生改动.
///
/// 0.1 的顶层游戏字段移入 `games.nte`, 每个游戏的 `installRoot` /
/// `globalInstallRoot` 移入按安装根键保存的 `installRoots`, 渠道 id 去空白
/// 并转小写.
pub(crate) fn migrate_value(value: &mut Value) -> bool {
    let top_level = migrate_top_level(value);
    let install_roots = migrate_install_roots(value);
    let distributions = normalize_distributions(value);
    top_level || install_roots || distributions
}

/// 把 0.1 的顶层游戏字段移入 `games.nte`. 已有的 `games` 数据优先.
fn migrate_top_level(value: &mut Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if !LEGACY_KEYS.iter().any(|key| object.contains_key(*key)) {
        return false;
    }
    let games = object.get("games");
    if games.is_some_and(|games| !games.is_object()) {
        return false;
    }
    if games
        .and_then(|games| games.get("nte"))
        .is_some_and(|nte| !nte.is_object())
    {
        return false;
    }

    let moved: Vec<(String, Value)> = LEGACY_KEYS
        .iter()
        .filter_map(|key| object.remove(*key).map(|value| ((*key).to_string(), value)))
        .collect();
    let nte = object
        .entry("games")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .expect("games 已确认是对象")
        .entry("nte")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .expect("games.nte 已确认是对象");
    for (key, value) in moved {
        nte.entry(key).or_insert(value);
    }
    object
        .entry("activeGame")
        .or_insert_with(|| Value::String(GameId::default().as_str().into()));
    true
}

/// 把两个固定槽位迁入 `installRoots`; 已存在的安装根键优先.
fn migrate_install_roots(value: &mut Value) -> bool {
    let Some(games) = value.get_mut("games").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut changed = false;
    for game in games.values_mut() {
        let Some(object) = game.as_object_mut() else {
            continue;
        };
        let mut invalid = false;
        let mut roots = match object.get("installRoots") {
            Some(Value::Object(map)) => map.clone(),
            Some(_) => {
                changed = true;
                invalid = true;
                serde_json::Map::new()
            }
            None => serde_json::Map::new(),
        };
        let mut legacy = false;
        for (key, root_key) in [("installRoot", "cn"), ("globalInstallRoot", "global")] {
            let Some(raw) = object.remove(key) else {
                continue;
            };
            changed = true;
            legacy = true;
            if let Value::String(text) = raw {
                roots
                    .entry(root_key.to_string())
                    .or_insert(Value::String(text));
            }
        }
        if roots.is_empty() {
            // 只清理由旧字段迁移出来的空容器; 新格式的空映射保持原样.
            if (legacy || invalid) && object.remove("installRoots").is_some() {
                changed = true;
            }
        } else {
            let next = Value::Object(roots);
            if object.get("installRoots") != Some(&next) {
                object.insert("installRoots".into(), next);
                changed = true;
            }
        }
    }
    changed
}

/// 渠道 id 只保留字符串; 空值与非法类型视为尚未选择.
fn normalize_distributions(value: &mut Value) -> bool {
    let Some(games) = value.get_mut("games").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut changed = false;
    for game in games.values_mut() {
        let Some(object) = game.as_object_mut() else {
            continue;
        };
        let Some(raw) = object.get("distribution") else {
            continue;
        };
        match raw {
            Value::String(text) => {
                let normalized = text.trim().to_ascii_lowercase();
                if normalized.is_empty() {
                    object.remove("distribution");
                    changed = true;
                } else if &normalized != text {
                    object.insert("distribution".into(), Value::String(normalized));
                    changed = true;
                }
            }
            _ => {
                object.remove("distribution");
                changed = true;
            }
        }
    }
    changed
}

/// 设置文件是否还含旧形状的字段.
fn has_legacy_shape(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if LEGACY_KEYS.iter().any(|key| object.contains_key(*key)) {
        return true;
    }
    object
        .get("games")
        .and_then(Value::as_object)
        .is_some_and(|games| {
            games.values().any(|game| {
                game.as_object().is_some_and(|game| {
                    game.contains_key("installRoot") || game.contains_key("globalInstallRoot")
                })
            })
        })
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_string());
    path.with_file_name(format!("{name}.bak"))
}

/// 备份先写临时文件再改名, 写入失败时不会留下残缺的 `.bak` 被当成有效备份.
fn write_backup(backup: &Path, original: &[u8]) -> oddity_platform::Result<()> {
    let mut temp = oddity_platform::paths::TemporaryFile::with_mode(backup, 0o600)?;
    temp.file.write_all(original)?;
    temp.commit(backup, &|| false)?;
    Ok(())
}

/// 保存前为仍是旧格式的设置文件补一次备份. 迁移写盘失败时内存里已经是新设置,
/// 后续任何一次保存都会覆盖原文件; 这里保证覆盖前先留下原始内容.
pub fn backup_legacy(path: &Path) -> oddity_platform::Result<()> {
    let original = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let value: Value = match serde_json::from_slice(&original) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if !has_legacy_shape(&value) {
        return Ok(());
    }
    let backup = backup_path(path);
    if backup.try_exists()? {
        return Ok(());
    }
    write_backup(&backup, &original)
}

fn persist_migration(path: &Path, original: &[u8], value: &Value) -> oddity_platform::Result<()> {
    let backup = backup_path(path);
    // 只保留第一次迁移前的原始文件, 重复启动不会覆盖.
    if !backup.try_exists()? {
        write_backup(&backup, original)?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| Error::Parse(format!("序列化设置失败: {error}")))?;
    let mut temp = oddity_platform::paths::TemporaryFile::new(path)?;
    temp.file.write_all(text.as_bytes())?;
    temp.commit(path, &|| false)?;
    Ok(())
}

/// 设置 owner: 宿主通用设置与按游戏命名的游戏设置.
///
/// owner 字符串同时是契约命令里的标识: `host` 或 `game:<游戏 id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsOwner {
    Host,
    Game(GameId),
}

/// 一个 owner 更新后的视图.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    pub owner: String,
    pub fields: Value,
}

/// 宿主 owner 可以写的字段; `games` 属于游戏 owner.
const HOST_FIELDS: [&str; 14] = [
    "activeGame",
    "runtimeEnvironments",
    "gamepad",
    "language",
    "reduceMotion",
    "stackBulletins",
    "theme",
    "heroMedia",
    "pauseHeroOnBlur",
    "adaptiveTheme",
    "relativeHeatmap",
    "minimizeToTrayOnClose",
    "showLauncherAfterGameExit",
    "snapshotSuccessMinutes",
];

/// 游戏 owner 可以写的字段; 宿主字段不能从这里写入.
const GAME_FIELDS: [&str; 10] = [
    "audio",
    "distribution",
    "installRoots",
    "preloadRoot",
    "launchMode",
    "gameAccountId",
    "gameMode",
    "updates",
    "activeEnvironment",
    "stableEnvironment",
];

impl SettingsOwner {
    /// 宿主 owner 的固定名称.
    pub const HOST: &'static str = "host";

    /// 解析 owner 标识, 只接受 `host` 与当前装配的 `game:<游戏 id>`.
    pub fn parse(owner: &str, known: &[&str]) -> Result<Self, String> {
        let owner = owner.trim();
        if owner == Self::HOST {
            return Ok(Self::Host);
        }
        let game = owner.strip_prefix("game:").ok_or_else(|| {
            format!("无法识别的设置 owner: {owner}. 可用值: host, game:<游戏 id>.")
        })?;
        if game.is_empty() {
            return Err("设置 owner 缺少游戏 id".into());
        }
        GameId::parse_known(game, known).map(Self::Game)
    }

    /// 该 owner 的规范名称.
    pub fn id(&self) -> String {
        match self {
            Self::Host => Self::HOST.into(),
            Self::Game(game) => format!("game:{}", game.as_str()),
        }
    }

    /// 该 owner 当前的完整视图.
    ///
    /// 声明了 schema 的 owner 只返回 schema 字段; 没有声明的宿主或游戏 owner
    /// 继续返回内置字段.
    pub fn view(
        &self,
        settings: &Settings,
        schemas: &SettingsSchemas,
    ) -> Result<SettingsSection, String> {
        if let Some(schema) = schemas.get(&self.id()) {
            let values = section_values(settings, schema)?;
            return Ok(SettingsSection {
                owner: self.id(),
                fields: Value::Object(values),
            });
        }
        let fields = match self {
            Self::Host => {
                let mut value =
                    serde_json::to_value(settings).map_err(|error| error.to_string())?;
                let object = value
                    .as_object_mut()
                    .ok_or_else(|| "设置不是 JSON 对象".to_string())?;
                object.remove("games");
                object.remove("sections");
                value
            }
            Self::Game(ref game) => {
                let game = settings
                    .games
                    .get(game)
                    .ok_or_else(|| format!("设置 owner 不存在: {}", self.id()))?;
                serde_json::to_value(game).map_err(|error| error.to_string())?
            }
        };
        Ok(SettingsSection {
            owner: self.id(),
            fields,
        })
    }

    /// 只把补丁里的字段合并进该 owner; 未知字段与跨 owner 字段直接拒绝.
    pub fn merge(
        &self,
        settings: &mut Settings,
        patch: &Value,
        schemas: &SettingsSchemas,
    ) -> Result<(), String> {
        let patch = patch
            .as_object()
            .ok_or_else(|| "设置补丁必须是 JSON 对象".to_string())?;
        let schema = schemas.get(&self.id());
        let allowed: Vec<&str> = match (&schema, self) {
            (Some(schema), _) => schema.fields.iter().map(|field| field.key).collect(),
            (None, Self::Host) => HOST_FIELDS.to_vec(),
            (None, Self::Game(_)) => GAME_FIELDS.to_vec(),
        };
        for key in patch.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(format!("设置 owner {} 不能修改字段: {key}", self.id()));
            }
        }
        let Some(schema) = schema else {
            if let Self::Game(ref game) = self {
                if !settings.games.contains_key(game) {
                    return Err(format!("设置 owner 不存在: {}", self.id()));
                }
            }
            if patch.is_empty() {
                return Ok(());
            }
            return Self::merge_builtin(self, settings, patch);
        };

        let mut values = section_values(settings, schema)?;
        for (key, value) in patch {
            values.insert(key.clone(), value.clone());
        }
        schema.validate_values(&values)?;
        settings.sections.insert(
            schema.owner.to_string(),
            SettingsSectionData {
                version: schema.version,
                values,
            },
        );
        Ok(())
    }

    /// 没有 schema 的 owner 继续写内置字段.
    fn merge_builtin(
        &self,
        settings: &mut Settings,
        patch: &Map<String, Value>,
    ) -> Result<(), String> {
        let mut root = serde_json::to_value(&*settings).map_err(|error| error.to_string())?;
        {
            let object = root
                .as_object_mut()
                .ok_or_else(|| "设置不是 JSON 对象".to_string())?;
            match self {
                Self::Host => {
                    for (key, value) in patch {
                        object.insert(key.clone(), value.clone());
                    }
                }
                Self::Game(ref game) => {
                    let section = object
                        .get_mut("games")
                        .and_then(Value::as_object_mut)
                        .and_then(|games| games.get_mut(game.as_str()))
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| format!("设置 owner 不存在: {}", self.id()))?;
                    for (key, value) in patch {
                        section.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        let mut next: Settings =
            serde_json::from_value(root).map_err(|error| format!("设置字段无效: {error}"))?;
        next.normalize();
        *settings = next;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct UserPaths {
    pub config_base: PathBuf,
    pub data_base: PathBuf,
}

impl UserPaths {
    pub fn discover() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            config_base: std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config")),
            data_base: std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share")),
        }
    }
    pub fn config_dir(&self) -> PathBuf {
        self.config_base.join("oddity")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.data_base.join("oddity")
    }
    pub fn settings(&self) -> PathBuf {
        self.config_dir().join("settings.json")
    }
    pub fn logs(&self) -> PathBuf {
        self.data_dir().join("logs")
    }
    pub fn instance_lock(&self) -> PathBuf {
        self.config_base.join("oddity-instance.lock")
    }
}

/// 读取设置并收敛手柄, 更新与快照字段.
fn load_normalized(path: &Path) -> oddity_platform::Result<Settings> {
    let mut settings = Settings::load(path)?;
    if settings.gamepad.validate().is_err() {
        settings.gamepad = Default::default();
    }
    for game in settings.games.values_mut() {
        game.updates.normalize();
    }
    settings.snapshot_success_minutes =
        crate::snapshots::normalize_success_minutes(settings.snapshot_success_minutes);
    Ok(settings)
}

/// 读取设置; 读取失败或手柄配置越界时回到默认值.
#[cfg(test)]
pub(crate) fn load() -> Settings {
    load_normalized(&crate::paths::settings_path()).unwrap_or_default()
}

/// 读取设置并应用已装配包的 section schema.
///
/// 默认值与旧版本迁移来自包声明; 已有设置文件在迁移后立即落盘, 写入失败时
/// 继续使用内存结果.
pub(crate) fn load_with_schemas(registry: &Registry) -> Settings {
    let path = crate::paths::settings_path();
    let existing = path.try_exists().unwrap_or(false);
    let Ok(mut settings) = load_normalized(&path) else {
        return Settings::default();
    };
    match SettingsSchemas::from_registry(registry).materialize(&mut settings) {
        Ok(true) if existing => {
            if let Err(error) = persist(&settings) {
                eprintln!("保存设置 section 迁移失败: {error}");
            }
        }
        Ok(_) => {}
        Err(error) => eprintln!("设置 section 无效: {error}"),
    }
    settings
}

pub(crate) fn persist(settings: &Settings) -> Result<(), String> {
    let dir = crate::paths::config_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = crate::paths::settings_path();
    // 迁移写盘失败后第一次保存会覆盖旧文件, 这里先把原始内容补备份.
    backup_legacy(&path).map_err(|error| format!("备份旧设置失败: {error}"))?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = dir.join("settings.json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename settings: {e}"))?;
    Ok(())
}

/// 下载任务只依赖当前游戏的渠道和各安装根键下的目录.
pub(crate) fn download_paths_changed(current: &Settings, next: &Settings) -> bool {
    if current.active_game != next.active_game {
        return true;
    }
    let current = current.current();
    let next = next.current();
    current.distribution != next.distribution
        || current.install_roots != next.install_roots
        || current.preload_root != next.preload_root
}

/// 校验设置边界并把越界值收敛到默认值.
pub(crate) fn normalize_settings(
    next: &mut Settings,
    registry: &oddity_runtime::Registry,
) -> Result<(), String> {
    next.snapshot_success_minutes =
        crate::snapshots::normalize_success_minutes(next.snapshot_success_minutes);
    next.gamepad.validate()?;
    for game in next.games.values() {
        game.audio.validate()?;
    }
    next.current()
        .game_mode
        .gamescope
        .validate()
        .map_err(|error| error.to_string())?;
    next.current().updates.validate()?;
    if !matches!(
        next.current().launch_mode.as_str(),
        "official" | "background" | "direct"
    ) {
        next.current_mut().launch_mode = "background".into();
    }
    if next.theme != "light" && next.theme != "dark" {
        next.theme = "light".into();
    }
    if next.hero_media != "video" && next.hero_media != "photo" {
        next.hero_media = "video".into();
    }
    for (game, settings) in &mut next.games {
        settings.distribution =
            crate::channel::effective(registry, game.as_str(), settings.distribution.as_ref())
                .map(|channel| ChannelId::new(channel.id));
    }
    SettingsSchemas::from_registry(registry).materialize(next)?;
    Ok(())
}

/// 按 owner 合并设置: 只写该 owner 的字段, 其余 owner 的值保持不变.
pub(crate) fn update_settings_section(
    app: &crate::service::Service,
    owner: String,
    patch: Value,
) -> Result<SettingsSection, String> {
    let known = owner_games(&app.state.packages);
    let owner = SettingsOwner::parse(&owner, &known)?;
    let schemas = SettingsSchemas::from_registry(&app.state.packages);
    let state = &app.state;
    // 与下载启动/写盘共用同一把状态锁, 避免设置与下载并发修改安装目录.
    let download_gates = app.state.packages.gates()?;
    let mut current = state.settings.lock().map_err(|error| error.to_string())?;
    let mut next = current.clone();
    owner.merge(&mut next, &patch, &schemas)?;
    normalize_settings(&mut next, &state.packages)?;
    let game = state.game.lock().map_err(|error| error.to_string())?;
    let busy = game.migration_busy
        || crate::migration_commands::repair_pending()
        || game.environment_busy
        || game.starting
        || game
            .session
            .as_ref()
            .is_some_and(|session| session.closing || crate::session::session_alive(session));
    crate::environment_commands::validate_settings_change(&current, &next, busy)?;
    if download_gates.iter().any(|gate| gate.blocks_changes())
        && download_paths_changed(&current, &next)
    {
        return Err("游戏资源任务正在进行, 暂时无法修改游戏地区或安装目录".into());
    }
    let section = owner.view(&next, &schemas)?;
    // 先落盘再更新内存: 写入失败时原文件与内存中的设置都保持原样.
    persist(&next)?;
    app.audio.preferences(next.current().audio.clone());
    *current = next;
    drop(game);
    drop(current);
    Ok(section)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn write_settings(dir: &TempDir, text: &str) -> PathBuf {
        let path = dir.join("settings.json");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn legacy_settings_enable_adaptive_theme() {
        let settings: Settings =
            serde_json::from_str(r#"{"theme":"dark","heroMedia":"photo"}"#).unwrap();
        assert!(settings.adaptive_theme);
        assert!(settings.pause_hero_on_blur);
        assert_eq!(settings.theme, "dark");
    }

    #[test]
    fn adaptive_theme_can_be_disabled() {
        let settings: Settings = serde_json::from_str(r#"{"adaptiveTheme":false}"#).unwrap();
        assert!(!settings.adaptive_theme);
    }

    #[test]
    fn default_settings_keep_a_current_game_entry() {
        let settings = Settings::default();
        assert_eq!(settings.active_game, GameId::from("nte"));
        assert!(settings.games.contains_key(&GameId::from("nte")));
        assert_eq!(settings.current(), &GameSettings::default());
        // 未登记的游戏回退默认值, 不 panic.
        assert_eq!(settings.game(GameId::from("hsr")), &GameSettings::default());
    }

    #[test]
    fn game_settings_keep_region_install_roots() {
        let mut settings = Settings::default();
        settings.current_mut().distribution = Some(ChannelId::new("global"));
        settings
            .current_mut()
            .install_roots
            .insert("cn".into(), "/games/cn".into());
        settings
            .current_mut()
            .install_roots
            .insert("global".into(), "/games/global".into());
        assert_eq!(
            settings.current().install_root_for_key("global"),
            Some("/games/global")
        );
        assert_eq!(
            settings.current().install_root_for_key("cn"),
            Some("/games/cn")
        );
        // 新增游戏不会影响当前游戏的设置.
        let before = settings.current().clone();
        settings
            .game_mut(GameId::from("hsr"))
            .install_roots
            .insert("cn".into(), "/hsr".into());
        assert_eq!(settings.current(), &before);
        assert_eq!(
            settings
                .game(GameId::from("hsr"))
                .install_root_for_key("cn"),
            Some("/hsr")
        );
    }

    #[test]
    fn each_game_keeps_its_own_launch_mode_game_mode_updates_and_environments() {
        let mut settings = Settings::default();
        settings.current_mut().launch_mode = "direct".into();
        settings.current_mut().game_mode.performance = true;
        settings.current_mut().updates.auto_download = true;
        settings.current_mut().active_environment = Some("a".repeat(32));
        settings.current_mut().stable_environment = Some("b".repeat(32));
        let nte = settings.current().clone();

        settings.active_game = GameId::from("hsr");
        assert_eq!(settings.current().launch_mode, "background");
        assert!(!settings.current().game_mode.performance);
        assert!(!settings.current().updates.auto_download);
        assert!(settings.current().active_environment.is_none());
        assert!(settings.current().stable_environment.is_none());

        settings.current_mut().launch_mode = "official".into();
        settings.current_mut().game_mode.inhibit = true;
        settings.current_mut().updates.auto_download = false;
        settings.current_mut().active_environment = Some("c".repeat(32));
        settings.active_game = GameId::from("nte");
        assert_eq!(settings.current(), &nte);
    }

    #[test]
    fn legacy_fields_migrate_into_games_nte_and_keep_unknown_data() {
        let dir = TempDir::new("settings-migrate");
        let original = br#"{
            "distribution": "global",
            "installRoot": "/games/cn",
            "globalInstallRoot": "/games/global",
            "preloadRoot": "/cache",
            "launchMode": "direct",
            "gameMode": {"inhibit": true},
            "updates": {"autoDownload": true, "metered": "allow"},
            "activeEnvironment": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "stableEnvironment": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "theme": "dark",
            "pauseHeroOnBlur": false,
            "futureOption": 7
        }"#;
        let path = write_settings(&dir, std::str::from_utf8(original).unwrap());

        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.active_game, GameId::from("nte"));
        let game = settings.current();
        assert_eq!(game.distribution.as_ref(), Some(&ChannelId::new("global")));
        assert_eq!(game.install_root_for_key("cn"), Some("/games/cn"));
        assert_eq!(game.install_root_for_key("global"), Some("/games/global"));
        assert_eq!(game.preload_root.as_deref(), Some("/cache"));
        assert_eq!(game.launch_mode, "direct");
        assert!(game.game_mode.inhibit);
        assert!(game.updates.auto_download);
        assert_eq!(game.updates.metered, crate::updates::MeteredPolicy::Allow);
        assert_eq!(
            game.active_environment.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            game.stable_environment.as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );

        // 迁移前备份原始字节, 迁移后保留未知字段并移除旧键.
        assert_eq!(fs::read(dir.join("settings.json.bak")).unwrap(), original);
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(value.get("installRoot").is_none());
        assert!(value.get("globalInstallRoot").is_none());
        assert!(value.get("preloadRoot").is_none());
        assert!(value.get("distribution").is_none());
        assert!(value.get("launchMode").is_none());
        assert!(value.get("gameMode").is_none());
        assert!(value.get("updates").is_none());
        assert!(value.get("activeEnvironment").is_none());
        assert!(value.get("stableEnvironment").is_none());
        assert_eq!(value["games"]["nte"]["launchMode"], "direct");
        assert_eq!(value["games"]["nte"]["gameMode"]["inhibit"], true);
        assert_eq!(value["futureOption"], 7);
        assert_eq!(value["pauseHeroOnBlur"], false);
        assert!(!settings.pause_hero_on_blur);
        assert_eq!(value["games"]["nte"]["distribution"], "global");
        assert_eq!(value["games"]["nte"]["installRoots"]["cn"], "/games/cn");
        assert_eq!(value["activeGame"], "nte");
    }

    #[test]
    fn game_level_install_roots_migrate_by_install_root_key() {
        let dir = TempDir::new("settings-game-roots");
        let original = br#"{"games":{"nte":{"distribution":" Global ","installRoot":"/games/cn","globalInstallRoot":"/games/global"}}}"#;
        let path = write_settings(&dir, std::str::from_utf8(original).unwrap());

        let settings = Settings::load(&path).unwrap();
        let game = settings.current();
        assert_eq!(game.distribution.as_ref(), Some(&ChannelId::new("global")));
        assert_eq!(game.install_root_for_key("cn"), Some("/games/cn"));
        assert_eq!(game.install_root_for_key("global"), Some("/games/global"));
        assert_eq!(fs::read(dir.join("settings.json.bak")).unwrap(), original);
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(value["games"]["nte"].get("installRoot").is_none());
        assert_eq!(value["games"]["nte"]["installRoots"]["cn"], "/games/cn");
        assert_eq!(
            value["games"]["nte"]["installRoots"]["global"],
            "/games/global"
        );
        assert_eq!(value["games"]["nte"]["distribution"], "global");
    }

    #[test]
    fn unknown_channels_normalize_to_the_declared_default() {
        let mut settings = Settings::default();
        let current = settings.current_mut();
        current.distribution = Some(ChannelId::new("eu"));
        current
            .install_roots
            .insert("cn".into(), "/games/cn".into());
        current
            .install_roots
            .insert("orphan".into(), "/games/orphan".into());

        normalize_settings(&mut settings, &crate::testing::packages()).unwrap();
        assert_eq!(
            settings.current().distribution.as_ref(),
            Some(&ChannelId::new("cn"))
        );
        assert_eq!(
            settings.current().install_root_for_key("cn"),
            Some("/games/cn")
        );
        assert_eq!(
            settings.current().install_root_for_key("orphan"),
            Some("/games/orphan")
        );
    }

    #[test]
    fn saving_over_legacy_settings_creates_the_missing_backup() {
        let dir = TempDir::new("settings-backup-before-save");
        let original = br#"{"installRoot":"/games/cn","theme":"dark","futureOption":7}"#;
        let path = write_settings(&dir, std::str::from_utf8(original).unwrap());
        backup_legacy(&path).unwrap();
        assert_eq!(fs::read(dir.join("settings.json.bak")).unwrap(), original);

        // 新格式文件不再补备份, 已有备份也不会被覆盖.
        fs::remove_file(dir.join("settings.json.bak")).unwrap();
        fs::write(&path, br#"{"activeGame":"nte","games":{"nte":{}}}"#).unwrap();
        backup_legacy(&path).unwrap();
        assert!(!dir.join("settings.json.bak").exists());
    }

    #[test]
    fn migration_is_idempotent_and_keeps_the_first_backup() {
        let dir = TempDir::new("settings-migrate-twice");
        let path = write_settings(&dir, r#"{"installRoot":"/one","theme":"dark"}"#);
        Settings::load(&path).unwrap();
        let backup = fs::read(dir.join("settings.json.bak")).unwrap();
        let first = fs::read(&path).unwrap();

        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.current().install_root_for_key("cn"), Some("/one"));
        assert_eq!(fs::read(dir.join("settings.json.bak")).unwrap(), backup);
        assert_eq!(fs::read(&path).unwrap(), first);
    }

    #[test]
    fn existing_game_settings_win_over_legacy_fields() {
        let dir = TempDir::new("settings-merge");
        let path = write_settings(
            &dir,
            r#"{
                "installRoot": "/old",
                "distribution": "cn",
                "launchMode": "direct",
                "games": {"nte": {"installRoot": "/new", "distribution": "global", "launchMode": "official"}}
            }"#,
        );
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.current().install_root_for_key("cn"), Some("/new"));
        assert_eq!(
            settings.current().distribution.as_ref(),
            Some(&ChannelId::new("global"))
        );
        assert_eq!(settings.current().launch_mode, "official");
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(value.get("installRoot").is_none());
        assert_eq!(value["games"]["nte"]["installRoots"]["cn"], "/new");
    }

    #[test]
    fn settings_without_legacy_game_fields_are_not_rewritten() {
        let dir = TempDir::new("settings-current");
        let path = write_settings(&dir, r#"{"theme":"dark"}"#);
        let before = fs::read(&path).unwrap();
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.theme, "dark");
        assert!(settings.games.contains_key(&GameId::from("nte")));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!dir.join("settings.json.bak").exists());
    }

    #[test]
    fn invalid_settings_are_rejected_without_touching_the_file() {
        let dir = TempDir::new("settings-invalid");
        let path = write_settings(&dir, "{broken");
        assert!(Settings::load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        let list = write_settings(&dir, "[]");
        assert!(Settings::load(&list).is_err());
        assert!(!dir.join("settings.json.bak").exists());
    }

    #[test]
    fn missing_settings_return_defaults() {
        let dir = TempDir::new("settings-missing");
        let settings = Settings::load(&dir.join("settings.json")).unwrap();
        assert_eq!(settings.active_game, GameId::from("nte"));
        assert_eq!(settings.current().launch_mode, "background");
    }

    #[test]
    fn settings_owner_parses_host_and_game_namespaces() {
        let known = ["nte", "hsr"];
        assert_eq!(
            SettingsOwner::parse("host", &known),
            Ok(SettingsOwner::Host)
        );
        assert_eq!(
            SettingsOwner::parse(" host ", &known),
            Ok(SettingsOwner::Host)
        );
        assert_eq!(
            SettingsOwner::parse("game:nte", &known),
            Ok(SettingsOwner::Game(GameId::from("nte")))
        );
        assert_eq!(
            SettingsOwner::parse("game:HSR", &known),
            Ok(SettingsOwner::Game(GameId::from("hsr")))
        );
        assert_eq!(SettingsOwner::Host.id(), "host");
        assert_eq!(SettingsOwner::Game(GameId::from("nte")).id(), "game:nte");

        for owner in ["nte", "games:nte", "game:", ""] {
            let error = SettingsOwner::parse(owner, &known).unwrap_err();
            assert!(error.contains("owner"), "{owner}: {error}");
        }
        let error = SettingsOwner::parse("game:genshin", &known).unwrap_err();
        assert!(error.contains("无法识别游戏"), "{error}");
        assert!(error.contains("nte, hsr"), "{error}");
    }

    #[test]
    fn host_owner_merges_only_host_fields() {
        let mut settings = Settings::default();
        settings.current_mut().launch_mode = "official".into();
        SettingsOwner::Host
            .merge(
                &mut settings,
                &serde_json::json!({"theme": "dark"}),
                &SettingsSchemas::default(),
            )
            .unwrap();
        assert_eq!(settings.theme, "dark");
        // 游戏字段不属于宿主 owner, 原值保持不动.
        assert_eq!(settings.current().launch_mode, "official");

        for patch in [
            serde_json::json!({"games": {"nte": {"launchMode": "direct"}}}),
            serde_json::json!({"unknown": 1}),
        ] {
            let error = SettingsOwner::Host
                .merge(&mut settings, &patch, &SettingsSchemas::default())
                .unwrap_err();
            assert!(error.contains("不能修改字段"), "{patch}: {error}");
        }
        assert_eq!(settings.current().launch_mode, "official");
    }

    #[test]
    fn game_owner_merges_only_its_own_fields() {
        let mut settings = Settings {
            theme: "dark".into(),
            ..Settings::default()
        };
        settings.game_mut(GameId::from("hsr")).launch_mode = "official".into();
        SettingsOwner::Game(GameId::from("nte"))
            .merge(
                &mut settings,
                &serde_json::json!({"launchMode": "direct"}),
                &SettingsSchemas::default(),
            )
            .unwrap();
        assert_eq!(settings.current().launch_mode, "direct");
        // 宿主字段与其他游戏不受影响.
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.game(GameId::from("hsr")).launch_mode, "official");
    }

    #[test]
    fn game_owner_rejects_cross_owner_and_unknown_fields() {
        let mut settings = Settings::default();
        for patch in [
            serde_json::json!({"theme": "dark"}),
            serde_json::json!({"activeGame": "hsr"}),
            serde_json::json!({"unknown": 1}),
        ] {
            let error = SettingsOwner::Game(GameId::from("nte"))
                .merge(&mut settings, &patch, &SettingsSchemas::default())
                .unwrap_err();
            assert!(error.contains("不能修改字段"), "{patch}: {error}");
        }
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.active_game, GameId::from("nte"));
    }

    #[test]
    fn game_owner_must_exist_before_merging() {
        let mut settings = Settings::default();
        let error = SettingsOwner::Game(GameId::from("hsr"))
            .merge(
                &mut settings,
                &serde_json::json!({"launchMode": "official"}),
                &SettingsSchemas::default(),
            )
            .unwrap_err();
        assert_eq!(error, "设置 owner 不存在: game:hsr");
        assert!(SettingsOwner::Game(GameId::from("hsr"))
            .view(&settings, &SettingsSchemas::default())
            .is_err());
    }

    #[test]
    fn owner_field_lists_cover_every_setting_field() {
        let settings = Settings::default();
        let host = SettingsOwner::Host
            .view(&settings, &SettingsSchemas::default())
            .unwrap();
        let mut host_keys: Vec<String> = host.fields.as_object().unwrap().keys().cloned().collect();
        host_keys.sort();
        let mut host_listed: Vec<String> = HOST_FIELDS
            .iter()
            .map(|field| (*field).to_string())
            .collect();
        host_listed.sort();
        assert_eq!(host_listed, host_keys);

        let game = SettingsOwner::Game(GameId::from("nte"))
            .view(&settings, &SettingsSchemas::default())
            .unwrap();
        let mut game_keys: Vec<String> = game.fields.as_object().unwrap().keys().cloned().collect();
        game_keys.sort();
        let mut game_listed: Vec<String> = GAME_FIELDS
            .iter()
            .map(|field| (*field).to_string())
            .collect();
        game_listed.sort();
        assert_eq!(game_listed, game_keys);
    }

    #[test]
    fn settings_owner_view_only_exposes_its_own_fields() {
        let settings = Settings::default();
        let host = SettingsOwner::Host
            .view(&settings, &SettingsSchemas::default())
            .unwrap();
        let host_fields = host.fields.as_object().unwrap();
        assert_eq!(host.owner, "host");
        assert_eq!(host_fields["theme"], "light");
        assert!(!host_fields.contains_key("games"));

        let game = SettingsOwner::Game(GameId::from("nte"))
            .view(&settings, &SettingsSchemas::default())
            .unwrap();
        let game_fields = game.fields.as_object().unwrap();
        assert_eq!(game.owner, "game:nte");
        assert_eq!(game_fields["launchMode"], "background");
        assert!(!game_fields.contains_key("theme"));
    }

    #[test]
    fn settings_patch_rejects_non_objects_and_invalid_values() {
        let mut settings = Settings::default();
        let before = serde_json::to_value(&settings).unwrap();
        let error = SettingsOwner::Host
            .merge(
                &mut settings,
                &serde_json::json!(["theme"]),
                &SettingsSchemas::default(),
            )
            .unwrap_err();
        assert_eq!(error, "设置补丁必须是 JSON 对象");

        let error = SettingsOwner::Host
            .merge(
                &mut settings,
                &serde_json::json!({"snapshotSuccessMinutes": "soon"}),
                &SettingsSchemas::default(),
            )
            .unwrap_err();
        assert!(error.contains("设置字段无效"), "{error}");
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);
    }

    /// 设置文件的 JSON 形状在身份字符串化后逐字段不变.
    #[test]
    fn default_settings_json_matches_the_golden_shape() {
        let expected: Value =
            serde_json::from_str(include_str!("tests/data/settings_default.json")).unwrap();
        assert_eq!(serde_json::to_value(Settings::default()).unwrap(), expected);
    }

    /// `games` 的键仍是裸字符串, 取值与新类型无关.
    #[test]
    fn game_scoped_settings_json_matches_the_golden_shape() {
        let mut settings: Settings = serde_json::from_str(
            r#"{"activeGame":"hsr","games":{"nte":{"distribution":"global","installRoots":{"cn":"/games/nte"}},"hsr":{"launchMode":"official","updates":{"autoDownload":true}}},"theme":"dark"}"#,
        )
        .unwrap();
        settings.normalize();
        let expected: Value =
            serde_json::from_str(include_str!("tests/data/settings_multi_game.json")).unwrap();
        assert_eq!(serde_json::to_value(settings).unwrap(), expected);
    }
}

#[cfg(test)]
mod audio_tests {
    #[test]
    fn audio_preferences_persist_and_old_settings_use_defaults() {
        use super::*;
        let mut settings = Settings::default();
        settings.current_mut().audio = oddity_platform::audio::Options {
            device: Some("headphones".into()),
            volume: 130,
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().current().audio,
            settings.current().audio
        );
        fs::write(&path, r#"{"games":{"nte":{}}}"#).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().current().audio,
            oddity_platform::audio::Options::default()
        );
        fs::write(&path, r#"{"games":{"nte":{"audio":{"volume":151}}}}"#).unwrap();
        assert!(Settings::load(&path).is_err());
    }
}
#[cfg(test)]
mod schema_tests {
    use super::*;
    use oddity_runtime::{SettingsField, SettingsFieldKind, SettingsSchema};

    const FIELDS: &[SettingsField] = &[
        SettingsField {
            key: "label",
            label: "名称",
            description: "显示名称",
            order: 1,
            kind: SettingsFieldKind::Text {
                default: "旅行者",
                max_chars: 12,
            },
        },
        SettingsField {
            key: "enabled",
            label: "启用",
            description: "",
            order: 2,
            kind: SettingsFieldKind::Boolean { default: true },
        },
        SettingsField {
            key: "limit",
            label: "数量",
            description: "",
            order: 3,
            kind: SettingsFieldKind::Integer {
                default: 5,
                min: 1,
                max: 10,
            },
        },
        SettingsField {
            key: "style",
            label: "样式",
            description: "",
            order: 4,
            kind: SettingsFieldKind::Select {
                default: "plain",
                options: &["plain", "formal"],
            },
        },
    ];

    fn migrate(from: u32, values: &mut Map<String, Value>) -> Result<(), String> {
        if from != 1 {
            return Err(format!("不支持从版本 {from} 迁移"));
        }
        let old = values.remove("displayName").unwrap_or(Value::Null);
        values.insert("label".into(), old);
        Ok(())
    }

    static DEMO: SettingsSchema = SettingsSchema {
        owner: "game:demo",
        version: 2,
        fields: FIELDS,
        validate: None,
        migrate: Some(migrate),
    };

    fn schemas() -> SettingsSchemas {
        SettingsSchemas {
            entries: vec![&DEMO],
        }
    }

    fn owner() -> SettingsOwner {
        SettingsOwner::Game(GameId::from("demo"))
    }

    #[test]
    fn declared_section_round_trips_through_view_and_merge() {
        let mut settings = Settings::default();
        let schemas = schemas();
        let section = owner()
            .view(&settings, &schemas)
            .expect("默认值应可直接读取");
        assert_eq!(section.owner, "game:demo");
        assert_eq!(section.fields["label"], "旅行者");
        assert_eq!(section.fields["enabled"], true);
        assert_eq!(section.fields["limit"], 5);
        assert_eq!(section.fields["style"], "plain");

        owner()
            .merge(
                &mut settings,
                &serde_json::json!({"label": "新名字", "limit": 9}),
                &schemas,
            )
            .expect("声明字段应可写入");
        let section = owner().view(&settings, &schemas).unwrap();
        assert_eq!(section.fields["label"], "新名字");
        assert_eq!(section.fields["limit"], 9);
        assert_eq!(section.fields["style"], "plain");

        let stored: Value = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            stored["sections"]["game:demo"]["version"],
            serde_json::json!(2)
        );
        assert_eq!(
            stored["sections"]["game:demo"]["values"]["label"],
            serde_json::json!("新名字")
        );
        let reloaded: Settings = serde_json::from_value(stored).unwrap();
        assert_eq!(
            owner().view(&reloaded, &schemas).unwrap().fields,
            section.fields
        );
    }

    #[test]
    fn old_section_versions_are_migrated_and_defaults_filled() {
        let mut settings = Settings::default();
        settings.sections.insert(
            "game:demo".into(),
            SettingsSectionData {
                version: 1,
                values: [("displayName".to_string(), Value::String("旧名字".into()))]
                    .into_iter()
                    .collect(),
            },
        );
        schemas().materialize(&mut settings).expect("旧版本应迁移");
        let stored = &settings.sections["game:demo"];
        assert_eq!(stored.version, 2);
        assert_eq!(stored.values["label"], "旧名字");
        assert_eq!(stored.values["enabled"], true);
        assert!(!stored.values.contains_key("displayName"));

        let section = owner().view(&settings, &schemas()).unwrap();
        assert_eq!(section.fields["label"], "旧名字");
    }

    #[test]
    fn invalid_section_values_are_rejected_without_touching_settings() {
        let mut settings = Settings::default();
        let schemas = schemas();
        let before = serde_json::to_value(&settings).unwrap();

        let error = owner()
            .merge(&mut settings, &serde_json::json!({"limit": 11}), &schemas)
            .unwrap_err();
        assert!(error.contains("必须在 1 到 10 之间"), "{error}");
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);

        let error = owner()
            .merge(
                &mut settings,
                &serde_json::json!({"unknown": true}),
                &schemas,
            )
            .unwrap_err();
        assert!(error.contains("不能修改字段: unknown"), "{error}");
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);
    }

    #[test]
    fn newer_section_versions_are_not_silently_downgraded() {
        let mut settings = Settings::default();
        settings.sections.insert(
            "game:demo".into(),
            SettingsSectionData {
                version: 3,
                values: DEMO.defaults(),
            },
        );
        let error = schemas().materialize(&mut settings).unwrap_err();
        assert!(error.contains("版本 3 高于声明的 2"), "{error}");
        assert_eq!(settings.sections["game:demo"].version, 3);
    }

    #[test]
    fn section_defaults_keep_the_legacy_json_shape_out_of_settings() {
        let stored = serde_json::to_value(Settings::default()).unwrap();
        assert!(stored.get("sections").is_none());
    }
}
