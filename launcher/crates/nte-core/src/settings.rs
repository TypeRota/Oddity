//! 启动器设置及用户目录, 供主程序和应急入口共用.
use crate::distribution::Distribution;
use crate::game_profile::GameId;
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// 单个游戏的设置. 地区与安装目录随游戏走, 其余偏好保持全局.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GameSettings {
    pub distribution: Distribution,
    pub install_root: Option<String>,
    pub global_install_root: Option<String>,
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
            distribution: Distribution::Cn,
            install_root: None,
            global_install_root: None,
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

    /// 当前地区对应的安装目录.
    pub fn install_root_for(&self, distribution: Distribution) -> Option<&str> {
        match distribution {
            Distribution::Cn => self.install_root.as_deref(),
            Distribution::Global => self.global_install_root.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// 界面和服务当前展示的游戏; 本阶段只有《异环》.
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_game: GameId::Nte,
            games: BTreeMap::from([(GameId::Nte, GameSettings::default())]),
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
        self.game(self.active_game)
    }

    pub fn current_mut(&mut self) -> &mut GameSettings {
        self.games.entry(self.active_game).or_default()
    }

    /// 补齐当前游戏条目, 保证界面总能读到一种设置.
    pub fn normalize(&mut self) {
        self.games.entry(self.active_game).or_default();
        self.games.entry(GameId::Nte).or_default();
    }

    /// 读取设置文件. 旧版顶层游戏字段会迁移到 `games.nte`, 迁移前把原始
    /// 文件备份为 `settings.json.bak`; 文件不存在时返回默认值.
    pub fn load(path: &Path) -> crate::Result<Self> {
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
        let migrated = migrate_legacy(&mut value);
        let mut settings: Settings = serde_json::from_value(value.clone())
            .map_err(|error| Error::Parse(format!("设置字段无效: {error}")))?;
        settings.normalize();
        if migrated {
            // 备份或写入失败时继续使用内存中的迁移结果, 原文件留待下次启动重试.
            if let Err(error) = persist_migration(path, &original, &value) {
                eprintln!("迁移设置文件失败: {error}");
            }
        }
        Ok(settings)
    }
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

/// 把 0.1 的顶层游戏字段移入 `games.nte`. 已有的 `games` 数据优先.
fn migrate_legacy(value: &mut Value) -> bool {
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
        .or_insert_with(|| Value::String(GameId::Nte.as_str().into()));
    true
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_string());
    path.with_file_name(format!("{name}.bak"))
}

/// 备份先写临时文件再改名, 写入失败时不会留下残缺的 `.bak` 被当成有效备份.
fn write_backup(backup: &Path, original: &[u8]) -> crate::Result<()> {
    let mut temp = crate::paths::TemporaryFile::with_mode(backup, 0o600)?;
    temp.file.write_all(original)?;
    temp.commit(backup, &|| false)?;
    Ok(())
}

/// 保存前为仍是旧格式的设置文件补一次备份. 迁移写盘失败时内存里已经是新设置,
/// 后续任何一次保存都会覆盖原文件; 这里保证覆盖前先留下原始内容.
pub fn backup_legacy(path: &Path) -> crate::Result<()> {
    let original = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let value: Value = match serde_json::from_slice(&original) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if !value.is_object() || !LEGACY_KEYS.iter().any(|key| value.get(*key).is_some()) {
        return Ok(());
    }
    let backup = backup_path(path);
    if backup.try_exists()? {
        return Ok(());
    }
    write_backup(&backup, &original)
}

fn persist_migration(path: &Path, original: &[u8], value: &Value) -> crate::Result<()> {
    let backup = backup_path(path);
    // 只保留第一次迁移前的原始文件, 重复启动不会覆盖.
    if !backup.try_exists()? {
        write_backup(&backup, original)?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| Error::Parse(format!("序列化设置失败: {error}")))?;
    let mut temp = crate::paths::TemporaryFile::new(path)?;
    temp.file.write_all(text.as_bytes())?;
    temp.commit(path, &|| false)?;
    Ok(())
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
        assert_eq!(settings.active_game, GameId::Nte);
        assert!(settings.games.contains_key(&GameId::Nte));
        assert_eq!(settings.current(), &GameSettings::default());
        // 未登记的游戏回退默认值, 不 panic.
        assert_eq!(settings.game(GameId::Hsr), &GameSettings::default());
    }

    #[test]
    fn game_settings_keep_region_install_roots() {
        let mut settings = Settings::default();
        settings.current_mut().distribution = Distribution::Global;
        settings.current_mut().install_root = Some("/games/cn".into());
        settings.current_mut().global_install_root = Some("/games/global".into());
        assert_eq!(
            settings.current().install_root_for(Distribution::Global),
            Some("/games/global")
        );
        assert_eq!(
            settings.current().install_root_for(Distribution::Cn),
            Some("/games/cn")
        );
        // 新增游戏不会影响《异环》的设置.
        let before = settings.current().clone();
        settings.game_mut(GameId::Hsr).install_root = Some("/hsr".into());
        assert_eq!(settings.current(), &before);
        assert_eq!(
            settings.game(GameId::Hsr).install_root.as_deref(),
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

        settings.active_game = GameId::Hsr;
        assert_eq!(settings.current().launch_mode, "background");
        assert!(!settings.current().game_mode.performance);
        assert!(!settings.current().updates.auto_download);
        assert!(settings.current().active_environment.is_none());
        assert!(settings.current().stable_environment.is_none());

        settings.current_mut().launch_mode = "official".into();
        settings.current_mut().game_mode.inhibit = true;
        settings.current_mut().updates.auto_download = false;
        settings.current_mut().active_environment = Some("c".repeat(32));
        settings.active_game = GameId::Nte;
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
        assert_eq!(settings.active_game, GameId::Nte);
        let game = settings.current();
        assert_eq!(game.distribution, Distribution::Global);
        assert_eq!(game.install_root.as_deref(), Some("/games/cn"));
        assert_eq!(game.global_install_root.as_deref(), Some("/games/global"));
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
        assert_eq!(value["games"]["nte"]["installRoot"], "/games/cn");
        assert_eq!(value["activeGame"], "nte");
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
        assert_eq!(settings.current().install_root.as_deref(), Some("/one"));
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
        assert_eq!(settings.current().install_root.as_deref(), Some("/new"));
        assert_eq!(settings.current().distribution, Distribution::Global);
        assert_eq!(settings.current().launch_mode, "official");
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(value.get("installRoot").is_none());
        assert_eq!(value["games"]["nte"]["installRoot"], "/new");
    }

    #[test]
    fn settings_without_legacy_game_fields_are_not_rewritten() {
        let dir = TempDir::new("settings-current");
        let path = write_settings(&dir, r#"{"theme":"dark"}"#);
        let before = fs::read(&path).unwrap();
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.theme, "dark");
        assert!(settings.games.contains_key(&GameId::Nte));
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
        assert_eq!(settings.active_game, GameId::Nte);
        assert_eq!(settings.current().launch_mode, "background");
    }
}
