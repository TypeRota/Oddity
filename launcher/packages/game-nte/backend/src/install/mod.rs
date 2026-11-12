//! 安装探测: 官方启动器根目录、默认安装位置与游戏文件定位.
//!
//! NTE_ROOT/NTE_INSTALL、默认安装目录的拼写与官方配置格式都属于《异环》
//! 自身, 区服与布局常量与更新域共用 [`crate::update::region`]; 宿主只通过
//! [`oddity_runtime::InstallLocator`] 消费探测结果.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use oddity_runtime::{InstallLocator, InstallQuery, LauncherInfo};

use crate::update::region::{Region, LAYOUT};

/// 官方启动器目录的默认安装位置优先从这两个变量读取.
const ENV_KEYS: [&str; 2] = ["NTE_ROOT", "NTE_INSTALL"];
/// 官方早期安装目录使用过的小写拼写.
const LEGACY_INSTALL_NAMES: [&str; 2] = ["Neverness To Everness", "Neverness to Everness"];
/// 国服安装目录使用过的中文名.
const CHINESE_HOME_SEED: &str = "异环";
/// 可移动磁盘的挂载位置.
const MOUNT_ROOTS: [&str; 3] = ["/run/media", "/media", "/mnt"];

/// 探测用到的进程环境快照.
///
/// 家目录、工作目录、安装环境变量与挂载点都从这里读取, 测试可以注入固定值,
/// 不依赖进程的全局状态.
struct Probe {
    home: PathBuf,
    cwd: Option<PathBuf>,
    vars: Vec<(&'static str, OsString)>,
    mounts: Vec<PathBuf>,
}

impl Probe {
    fn process() -> Self {
        Self {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp")),
            cwd: std::env::current_dir().ok(),
            vars: ENV_KEYS
                .iter()
                .filter_map(|key| std::env::var_os(key).map(|value| (*key, value)))
                .collect(),
            mounts: MOUNT_ROOTS.iter().map(PathBuf::from).collect(),
        }
    }

    fn var(&self, key: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_os_str())
    }

    fn expand(&self, raw: &OsStr) -> PathBuf {
        expand(raw.to_string_lossy().as_ref(), &self.home)
    }
}

/// 只展开设置界面接受的 `~` 形式, 其余路径保持原样.
fn expand(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

fn detect(probe: &Probe, query: &InstallQuery) -> Option<PathBuf> {
    let region = Region::parse(&query.distribution)?;
    // 已配置安装目录时只认该目录. 找不到就是未安装, 不能静默改用其他副本.
    if let Some(configured) = &query.configured_install_root {
        return normalize(configured, region);
    }

    for key in ENV_KEYS {
        if let Some(raw) = probe.var(key) {
            if let Some(root) = normalize(&probe.expand(raw), region) {
                return Some(root);
            }
        }
    }

    let games = probe.home.join("Games");
    let mut seeds = vec![games
        .join(region.profile().default_install_name)
        .join(region.profile().launcher_dir)];
    install_seeds(&games, region, &mut seeds);
    if region == Region::Cn {
        seeds.push(
            games
                .join(CHINESE_HOME_SEED)
                .join(region.profile().launcher_dir),
        );
    }

    if let Some(cwd) = &probe.cwd {
        for ancestor in cwd.ancestors() {
            install_seeds(ancestor, region, &mut seeds);
        }
    }

    // 可移动磁盘: /run/media 与 /media 下是每用户目录, /mnt 通常直接是挂载点.
    // 两级都探测, 安装目录在哪种约定下都能被发现.
    for mount_root in &probe.mounts {
        let Ok(entries) = std::fs::read_dir(mount_root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            install_seeds(&path, region, &mut seeds);
            if let Ok(children) = std::fs::read_dir(&path) {
                for child in children.flatten() {
                    install_seeds(&child.path(), region, &mut seeds);
                }
            }
        }
    }

    seeds.into_iter().find_map(|seed| normalize(&seed, region))
}

/// 把一个基准目录下的常见安装布局加入候选.
fn install_seeds(base: &Path, region: Region, seeds: &mut Vec<PathBuf>) {
    for name in [
        region.profile().default_install_name,
        LEGACY_INSTALL_NAMES[0],
        LEGACY_INSTALL_NAMES[1],
    ] {
        for prefix in [
            base.join("Program Files (x86)"),
            base.join("Program Files"),
            base.to_path_buf(),
        ] {
            let candidate = prefix.join(name).join(region.profile().launcher_dir);
            if !seeds.contains(&candidate) {
                seeds.push(candidate);
            }
        }
    }
    seeds.push(base.join(region.profile().launcher_dir));
}

/// 把候选目录规范成该地区的官方启动器根目录.
fn normalize(path: &Path, region: Region) -> Option<PathBuf> {
    let root = if path.join("Config").join("Config.ini").is_file() {
        path.to_path_buf()
    } else {
        let nested = path.join(region.profile().launcher_dir);
        if !nested.join("Config").join("Config.ini").is_file() {
            return None;
        }
        nested
    };
    // 同一安装根下的其他地区启动器不能当成当前地区.
    if root
        .file_name()
        .is_some_and(|name| name == region.other().profile().launcher_dir)
    {
        return None;
    }
    match config_game_id(&root) {
        Some(game_id) if game_id != 0 && game_id != region.profile().app_id => None,
        _ => Some(root),
    }
}

/// 官方 Config.ini 的 `[General] GameID`; 读不到时按没有填写处理.
fn config_game_id(root: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(root.join("Config").join("Config.ini")).ok()?;
    Some(ini_value(&text, "General", "GameID").map_or(0, |value| value.parse().unwrap_or(0)))
}

/// 只取指定 section 的键值, 容忍 BOM、CRLF、注释与重复键.
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut current: &str = "General";
    let mut value = None;
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            current = name.trim();
            continue;
        }
        if current != section {
            continue;
        }
        if let Some((name, found)) = line.split_once('=') {
            if name.trim() == key {
                value = Some(found.trim());
            }
        }
    }
    value
}

/// 由设置快照构造安装查询; 快照里的路径已经展开.
fn snapshot_query(settings: &oddity_runtime::SettingsSnapshot) -> InstallQuery {
    InstallQuery {
        distribution: settings.distribution.clone(),
        configured_install_root: settings.install_root.clone().map(PathBuf::from),
    }
}

/// 设置里选中的安装根目录; 未配置时给出该地区的默认位置.
pub(crate) fn selected_install_root_for(settings: &oddity_runtime::SettingsSnapshot) -> PathBuf {
    selected_install_root(&Probe::process(), &snapshot_query(settings))
}

/// 设置里显式配置的安装根目录.
pub(crate) fn configured_install_root_for(
    settings: &oddity_runtime::SettingsSnapshot,
) -> Option<PathBuf> {
    snapshot_query(settings).configured_install_root
}

/// 探测官方启动器根目录.
pub(crate) fn detect_launcher_root_for(
    settings: &oddity_runtime::SettingsSnapshot,
) -> Option<PathBuf> {
    detect(&Probe::process(), &snapshot_query(settings))
}

/// 当前区服的官方启动器目录名; 只覆盖《异环》.
pub(crate) fn launcher_dir_for(settings: &oddity_runtime::SettingsSnapshot) -> &'static str {
    Region::parse(&settings.distribution)
        .map(|region| region.profile().launcher_dir)
        .unwrap_or("")
}

/// 安装目录下的游戏可执行文件.
pub(crate) fn game_exe_for(launcher_root: &Path) -> Option<PathBuf> {
    game_exe(launcher_root)
}

/// 与宿主安装域同源的有效安装根目录.
///
/// 配置了目录时以它本身为准, 否则取已探测启动器根目录的父目录; 两者都
/// 没有时退回该地区的默认安装位置. 更新域的自检也走这里.
pub(crate) fn effective_install_root_for(
    query: &InstallQuery,
    launcher_root: Option<&Path>,
) -> PathBuf {
    effective_install_root(&Probe::process(), query, launcher_root)
}

fn selected_install_root(probe: &Probe, query: &InstallQuery) -> PathBuf {
    let Some(region) = Region::parse(&query.distribution) else {
        return probe.home.join("Games");
    };
    let path = query.configured_install_root.clone().unwrap_or_else(|| {
        probe
            .home
            .join("Games")
            .join(region.profile().default_install_name)
    });
    if path
        .file_name()
        .is_some_and(|name| name == region.profile().launcher_dir)
    {
        path.parent().unwrap_or(&path).to_path_buf()
    } else {
        path
    }
}

fn effective_install_root(
    probe: &Probe,
    query: &InstallQuery,
    launcher_root: Option<&Path>,
) -> PathBuf {
    if query.configured_install_root.is_some() {
        return selected_install_root(probe, query);
    }
    launcher_root
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| selected_install_root(probe, query))
}

fn launcher_dir(query: &InstallQuery) -> Option<&'static str> {
    Region::parse(&query.distribution).map(|region| region.profile().launcher_dir)
}

fn game_exe(launcher_root: &Path) -> Option<PathBuf> {
    find_relative_ci(launcher_root.parent()?, LAYOUT.game_exe)
}

fn game_version(launcher_root: &Path) -> String {
    let path = launcher_root
        .join("UserData")
        .join("Config")
        .join("Config.ini");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    ini_value(&text, "Game", "Version_")
        .unwrap_or_default()
        .to_string()
}

/// 官方 Config/Config.ini 的 [VERSION] 段; 文件读不到时返回 None.
///
/// 段里缺项时照旧给空字符串: 配置存在但没有写版本, 与旧配置读取一致.
fn launcher_info(launcher_root: &Path) -> Option<LauncherInfo> {
    let text = std::fs::read_to_string(launcher_root.join("Config").join("Config.ini")).ok()?;
    Some(LauncherInfo {
        version: ini_value(&text, "VERSION", "Version")
            .unwrap_or_default()
            .to_string(),
        build: ini_value(&text, "VERSION", "Build")
            .unwrap_or_default()
            .to_string(),
    })
}

pub(crate) fn game_exe_hint() -> String {
    match Path::new(LAYOUT.game_exe).file_name() {
        Some(name) => format!("未找到 {}（注意 Win64 大小写）", name.to_string_lossy()),
        None => "未找到游戏可执行文件".to_string(),
    }
}

fn find_relative_ci(base: &Path, relative: &str) -> Option<PathBuf> {
    let components: Vec<&str> = relative
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    find_ci(base, &components)
}

/// 逐级按文件名大小写不敏感地查找路径, 与 Wine 下的实际布局一致.
fn find_ci(base: &Path, components: &[&str]) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for component in components {
        let entries = std::fs::read_dir(&current).ok()?;
        let mut found = None;
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(component)
            {
                found = Some(entry.path());
                break;
            }
        }
        current = found?;
    }
    Some(current)
}

/// Wine prefix 内的游戏安装链接名; 与区服无关, 迁移时沿用同一路径.
const PREFIX_LINK_NAME: &str = "Program Files (x86)/Neverness To Everness";

/// 安装域实例.
struct Install;

static INSTALL: Install = Install;

/// 交给宿主的安装域.
pub(crate) fn locator() -> &'static dyn InstallLocator {
    &INSTALL
}

impl InstallLocator for Install {
    fn launcher_root(&self, query: &InstallQuery) -> Option<PathBuf> {
        detect(&Probe::process(), query)
    }

    fn launcher_dir(&self, query: &InstallQuery) -> Option<&'static str> {
        launcher_dir(query)
    }

    fn selected_install_root(&self, query: &InstallQuery) -> PathBuf {
        selected_install_root(&Probe::process(), query)
    }

    fn effective_install_root(
        &self,
        query: &InstallQuery,
        launcher_root: Option<&Path>,
    ) -> PathBuf {
        effective_install_root(&Probe::process(), query, launcher_root)
    }

    fn game_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        game_exe(launcher_root)
    }

    fn game_version(&self, launcher_root: &Path) -> String {
        game_version(launcher_root)
    }

    fn launcher_info(&self, launcher_root: &Path) -> Option<LauncherInfo> {
        launcher_info(launcher_root)
    }

    fn game_exe_hint(&self) -> String {
        game_exe_hint()
    }
    fn sdk_dir(&self, launcher_root: &Path) -> Option<PathBuf> {
        let component = find_ci(launcher_root, &[LAYOUT.sdk_dll])?;
        component.parent().map(Path::to_path_buf)
    }

    fn launcher_config_path(&self, install_root: &Path) -> Option<PathBuf> {
        let config = install_root
            .join(LAYOUT.game_exe)
            .with_file_name(LAYOUT.launcher_config);
        config.is_file().then_some(config)
    }

    fn official_client_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        // 先认官方目录名, 再按 Config.ini 的 GameID 兜底; 自定义目录名也能定位客户端.
        let name = launcher_root.file_name()?.to_string_lossy();
        let region = crate::update::region::region_for_dir_name(&name).or_else(|| {
            config_game_id(launcher_root).and_then(crate::update::region::region_for_app_id)
        })?;
        Some(launcher_root.join(region.profile().client_exe))
    }

    fn prefix_link_name(&self) -> Option<&'static str> {
        Some(PREFIX_LINK_NAME)
    }
}

#[cfg(test)]
mod tests;
