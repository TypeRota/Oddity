//! 运行环境操作与启动/资源任务共用互斥边界.
use crate::service::ServiceHandle;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::channel::ChannelId;
use crate::environment::{self, EnvironmentState, RuntimeEnvironment};
use crate::environment_inventory;
use crate::identity::GameId;
use oddity_platform::install_lock::InstallLock;
use oddity_runtime::{ChannelDescriptor, Registry};
use serde::Serialize;

use crate::bootstrap::{bootstrap_impl, detect_proton, Bootstrap};
use crate::paths::{data_dir, expand_path};
use crate::session::session_alive;
use crate::settings::{persist as persist_settings, Settings};
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub distribution: ChannelId,
    pub install_root: PathBuf,
    pub installed: bool,
}

fn installation(
    registry: &Registry,
    settings: &Settings,
    channel: &ChannelDescriptor,
) -> Installation {
    let mut target = settings.clone();
    target.current_mut().distribution = Some(ChannelId::new(channel.id));
    let launcher = crate::install::launcher_root(registry, &target);
    Installation {
        distribution: ChannelId::new(channel.id),
        install_root: crate::install::effective_install_root(
            registry,
            &target,
            launcher.as_deref(),
        ),
        installed: launcher.as_deref().is_some_and(|root| {
            crate::install::game_exe(registry, &settings.active_game, root)
                .is_some_and(|game| game.is_file())
        }),
    }
}

/// 当前游戏声明的全部渠道及其安装状态.
pub fn installations(registry: &Registry, settings: &Settings) -> Vec<Installation> {
    crate::channel::channels(registry, settings.active_game.as_str())
        .iter()
        .map(|channel| installation(registry, settings, channel))
        .collect()
}

/// 区服还没有可用的游戏安装时的可操作提示.
pub(crate) fn missing_install_error(channel: &ChannelDescriptor, install_root: &Path) -> String {
    format!(
        "{} 的安装目录未找到官方启动器或游戏文件: {}. 请先在游戏更新页完成安装.",
        channel.label,
        install_root.display()
    )
}

/// 生成未被占用的运行环境名称: base-N 递增, 并按名称上限截断.
pub(crate) fn suggested_name(settings: &Settings, base: &str) -> String {
    // 与 environment::validate_name 的字符上限保持一致.
    const NAME_MAX_CHARS: usize = 64;
    let base = base.trim();
    let mut next = settings
        .runtime_environments
        .iter()
        .filter_map(|environment| {
            let name = environment.name.as_str();
            if name == base {
                return Some(0);
            }
            let suffix = name.strip_prefix(base)?.strip_prefix('-')?;
            if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            suffix.parse::<u64>().ok()
        })
        .max()
        .map_or(1, |max| max.saturating_add(1));
    loop {
        let suffix = format!("-{next}");
        let keep = NAME_MAX_CHARS.saturating_sub(suffix.chars().count());
        let prefix: String = base.chars().take(keep).collect();
        let candidate = format!("{prefix}{suffix}");
        if environment::validate_name(&candidate).is_ok()
            && !settings
                .runtime_environments
                .iter()
                .any(|environment| environment.name == candidate)
        {
            return candidate;
        }
        next = next.saturating_add(1);
    }
}

/// 当前游戏当前区服实际使用的运行环境及其启动目标.
pub struct Selection<'a> {
    pub environment: &'a RuntimeEnvironment,
    pub channel: &'static ChannelDescriptor,
    /// 该区服当前生效的游戏安装根目录.
    pub install_root: PathBuf,
}

/// 当前区服的键; 游戏没有声明渠道时为空串.
pub fn channel_key(registry: &Registry, settings: &Settings) -> String {
    crate::channel::effective_id(
        registry,
        settings.active_game.as_str(),
        settings.current().distribution.as_ref(),
    )
}

/// 该区服生效的安装根目录.
fn install_root(registry: &Registry, settings: &Settings) -> PathBuf {
    let launcher = crate::install::launcher_root(registry, settings);
    crate::install::effective_install_root(registry, settings, launcher.as_deref())
}

/// 按当前区服解析要用的运行环境.
pub fn resolve<'a>(registry: &Registry, settings: &'a Settings) -> Result<Selection<'a>, String> {
    let channel = crate::channel::effective(
        registry,
        settings.active_game.as_str(),
        settings.current().distribution.as_ref(),
    )
    .ok_or("当前游戏没有可用的发行渠道")?;
    let current = settings.current();
    let id = current
        .active_environment(channel.id)
        .ok_or("请先在设置中创建并选择 Proton 运行环境")?;
    let environment = settings
        .runtime_environments
        .iter()
        .find(|environment| environment.id == id)
        .ok_or_else(|| "所选运行环境不存在, 请重新选择环境".to_string())?;
    if !environment.belongs_to(&settings.active_game) {
        return Err(format!(
            "运行环境「{}」属于其他游戏, 请重新选择环境.",
            environment.name
        ));
    }
    Ok(Selection {
        environment,
        channel,
        install_root: install_root(registry, settings),
    })
}

pub fn selected<'a>(
    registry: &Registry,
    settings: &'a Settings,
) -> Result<&'a RuntimeEnvironment, String> {
    Ok(resolve(registry, settings)?.environment)
}

pub fn prefix(registry: &Registry, settings: &Settings) -> Result<PathBuf, String> {
    resolve(registry, settings)?
        .environment
        .prefix(&data_dir())
        .map_err(|error| format!("{error:#}"))
}

pub fn verify_proton(registry: &Registry, settings: &Settings) -> Result<(), String> {
    resolve(registry, settings)?
        .environment
        .proton
        .verify()
        .map_err(|error| format!("{error:#}"))
}

pub fn proton_path<'a>(registry: &Registry, settings: &'a Settings) -> Option<&'a Path> {
    resolve(registry, settings)
        .ok()
        .map(|selection| selection.environment.proton.path.as_path())
}

pub fn log_dir(registry: &Registry, settings: &Settings) -> Result<PathBuf, String> {
    let environment = resolve(registry, settings)?.environment;
    environment
        .directory(&data_dir())
        .map_err(|error| error.to_string())?;
    Ok(data_dir().join("logs/environments").join(&environment.id))
}

/// 锁定本次操作的安装目录与 prefix, 再对齐绑定.
pub(crate) fn lock_and_align(
    state: &AppState,
    settings: &Settings,
    target: &Selection<'_>,
) -> Result<(PathBuf, InstallLock, environment::PrefixLock), String> {
    let prefix = target
        .environment
        .prefix(&data_dir())
        .map_err(|error| format!("{error:#}"))?;
    let install_lock =
        InstallLock::acquire_existing(&target.install_root).map_err(|error| error.to_string())?;
    let prefix_lock = environment::lock_prefix(&prefix).map_err(|error| format!("{error:#}"))?;
    if !target.environment.targets(&target.install_root) {
        let mut live = state.settings.lock().map_err(|error| error.to_string())?;
        let mut next = live.clone();
        let environment = next
            .runtime_environments
            .iter_mut()
            .find(|environment| environment.id == target.environment.id)
            .ok_or("所选运行环境不存在, 请重新选择环境")?;
        rewrite_binding(
            &state.packages,
            settings,
            target.channel.id,
            &prefix,
            &environment.install_root,
            &target.install_root,
        )?;
        environment.install_root = target
            .install_root
            .canonicalize()
            .unwrap_or_else(|_| target.install_root.clone());
        persist_settings(&next)?;
        *live = next;
    }
    Ok((prefix, install_lock, prefix_lock))
}

/// 把 prefix 内的游戏链接, 注册表与官方配置改写到新的安装目录.
fn rewrite_binding(
    registry: &Registry,
    settings: &Settings,
    channel_id: &str,
    prefix: &Path,
    stale: &Path,
    target: &Path,
) -> Result<(), String> {
    if let Some(link_name) = crate::install::prefix_link_name(registry, &settings.active_game) {
        oddity_platform::wine::rewrite_prefix(prefix, link_name, stale, target)
            .map_err(|error| format!("更新 prefix 链接失败: {error}"))?;
        oddity_platform::wine::rewrite_registry(prefix, stale, target)
            .map_err(|error| format!("更新注册表失败: {error}"))?;
    }
    if let Some(launcher_dir) =
        crate::install::launcher_dir_for(registry, &settings.active_game, channel_id)
    {
        let config = crate::install::launcher_config_path(registry, &settings.active_game, target);
        oddity_platform::wine::rewrite_official(
            target,
            launcher_dir,
            config.as_deref(),
            stale,
            target,
        )
        .map_err(|error| format!("更新官方配置失败: {error}"))?;
    }
    Ok(())
}

/// 读取 prefix 内游戏链接指向的安装目录; 相对目标按链接父目录解析.
fn link_install_root(link: &Path) -> Option<PathBuf> {
    let target = std::fs::read_link(link).ok()?;
    let target = if target.is_absolute() {
        target
    } else {
        link.parent()?.join(target)
    };
    let target = environment::normalize(&target);
    Some(target.canonicalize().unwrap_or(target))
}

/// 单个运行环境的健康状态与安装目录.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentHealth {
    pub id: String,
    pub name: String,
    /// 契约里的环境状态; 目录可用但固定 Proton 已变化时为 proton-invalid.
    pub state: &'static str,
    pub install_root: String,
    pub install_root_current: String,
}

/// 环境数据审计: 当前游戏登记的环境与未登记目录.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentAudit {
    pub environments: Vec<EnvironmentHealth>,
    pub orphans: Vec<environment_inventory::Orphan>,
}

/// 只读审计当前游戏的运行环境与数据目录.
pub(crate) fn audit(state: &AppState) -> Result<EnvironmentAudit, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let current = install_root(&state.packages, &settings);
    let install_root_current = current
        .canonicalize()
        .unwrap_or(current)
        .display()
        .to_string();
    let environments = settings
        .runtime_environments
        .iter()
        .filter(|environment| environment.belongs_to(&settings.active_game))
        .map(|environment| {
            let state = match environment.inspect(&data_dir()) {
                EnvironmentState::Ready if environment.proton.verify().is_err() => "proton-invalid",
                state => state.as_str(),
            };
            EnvironmentHealth {
                id: environment.id.clone(),
                name: environment.name.clone(),
                state,
                install_root: environment
                    .install_root
                    .canonicalize()
                    .unwrap_or_else(|_| environment.install_root.clone())
                    .display()
                    .to_string(),
                install_root_current: install_root_current.clone(),
            }
        })
        .collect();
    let registered: BTreeSet<String> = settings
        .runtime_environments
        .iter()
        .map(|environment| environment.id.clone())
        .collect();
    let orphans = environment_inventory::scan(&data_dir(), &registered)?;
    Ok(EnvironmentAudit {
        environments,
        orphans,
    })
}

/// 删除一个未登记的环境或日志目录.
pub(crate) fn environment_orphan_delete(
    state: &AppState,
    kind: &str,
    name: &str,
) -> Result<(), String> {
    let _task = Task::begin(state)?;
    let kind = environment_inventory::OrphanKind::parse(kind).ok_or("未登记目录类型无效")?;
    let registered = {
        let settings = state.settings.lock().map_err(|error| error.to_string())?;
        settings
            .runtime_environments
            .iter()
            .map(|environment| environment.id.clone())
            .collect::<BTreeSet<_>>()
    };
    environment_inventory::remove(&data_dir(), &registered, kind, name)
}

pub(crate) struct Task<'a>(&'a AppState);

impl<'a> Task<'a> {
    pub(crate) fn begin(state: &'a AppState) -> Result<Self, String> {
        if state.packages.busy() {
            return Err("游戏资源任务正在进行, 暂时无法操作运行环境".into());
        }
        let mut game = state.game.lock().map_err(|error| error.to_string())?;
        if game.migration_busy
            || crate::migration_commands::repair_pending()
            || game.environment_busy
            || game.starting
            || game
                .session
                .as_ref()
                .is_some_and(|session| session.closing || session_alive(session))
        {
            return Err("游戏或运行环境任务正在进行, 请结束后重试".into());
        }
        game.environment_busy = true;
        Ok(Self(state))
    }
}

impl Drop for Task<'_> {
    fn drop(&mut self) {
        if let Ok(mut game) = self.0.game.lock() {
            game.environment_busy = false;
        }
    }
}

fn validate_selection(registry: &Registry, settings: &Settings) -> Result<(), String> {
    prefix(registry, settings)?;
    verify_proton(registry, settings)?;
    if detect_proton(registry, settings).is_none() {
        return Err("未找到所选环境的 Proton".into());
    }
    Ok(())
}

pub fn validate_settings_change(
    current: &Settings,
    next: &Settings,
    busy: bool,
) -> Result<(), String> {
    if current.runtime_environments != next.runtime_environments
        || current.game_pairs(next).any(|(before, after)| {
            before.active_environments != after.active_environments
                || before.stable_environments != after.stable_environments
        })
    {
        return Err("运行环境已变化, 请重新检测后重试".into());
    }
    if busy
        && (crate::settings::download_paths_changed(current, next)
            || current
                .game_pairs(next)
                .any(|(before, after)| before.launch_mode != after.launch_mode))
    {
        return Err("游戏或运行环境任务正在进行, 暂时无法修改运行路径或启动方式".into());
    }
    Ok(())
}

pub async fn environment_create(
    app: ServiceHandle,
    name: String,
    proton_path: String,
    source_id: Option<String>,
) -> Result<Bootstrap, String> {
    let worker = app.clone();
    let mut bootstrap = crate::service::spawn_blocking(move || {
        let state = &worker.state;
        create_impl(state, source_id.as_deref(), &name, &proton_path)
    })
    .await
    .map_err(|error| format!("创建运行环境任务失败: {error}"))??;
    bootstrap.integrity = app.integrity.status();
    Ok(bootstrap)
}

pub(crate) fn create_impl(
    state: &AppState,
    source_id: Option<&str>,
    name: &str,
    proton_path: &str,
) -> Result<Bootstrap, String> {
    let _task = Task::begin(state)?;
    let snapshot = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let channel = crate::channel::resolve(
        &state.packages,
        snapshot.active_game.as_str(),
        snapshot.current().distribution.as_ref(),
    )
    .map_err(|error| error.to_string())?;
    if snapshot
        .runtime_environments
        .iter()
        .any(|entry| entry.name == name.trim())
    {
        return Err(format!(
            "运行环境名称「{}」已存在, 请换一个名称",
            name.trim()
        ));
    }
    let target = installation(&state.packages, &snapshot, channel);
    if !target.installed {
        return Err(missing_install_error(channel, &target.install_root));
    }
    let source = if let Some(id) = source_id {
        let source = snapshot
            .runtime_environments
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("源运行环境不存在")?;
        source
            .proton
            .verify()
            .map_err(|error| format!("{error:#}"))?;
        Some((
            source
                .prefix(&data_dir())
                .map_err(|error| format!("{error:#}"))?,
            source.proton.path.clone(),
        ))
    } else {
        None
    };
    let install = target.install_root;
    let _install_lock = oddity_platform::install_lock::InstallLock::acquire_existing(&install)
        .map_err(|error| error.to_string())?;
    let link_name = crate::install::prefix_link_name(&state.packages, &snapshot.active_game);
    let created = environment::create(
        &data_dir(),
        source
            .as_ref()
            .map(|(prefix, proton)| (prefix.as_path(), proton.as_path())),
        &expand_path(proton_path),
        &install,
        crate::environment::EnvironmentTarget {
            game: snapshot.active_game,
            link_name,
        },
        name,
    )
    .map_err(|error| format!("创建运行环境失败: {error:#}"))?;
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    let mut next = settings.clone();
    // 首个环境创建后即可使用, 后续副本不改变当前选择.
    let key = channel.id;
    if next.current().active_environment(key).is_none() {
        next.current_mut()
            .set_active_environment(key, Some(&created.id));
        next.current_mut()
            .set_stable_environment(key, Some(&created.id));
    }
    next.runtime_environments.push(created.clone());
    if let Err(error) = persist_settings(&next) {
        let cleanup = environment::discard(&data_dir(), &created);
        return Err(format!(
            "{error}{}",
            cleanup
                .err()
                .map(|error| format!("; 副本清理失败: {error:#}"))
                .unwrap_or_default()
        ));
    }
    *settings = next.clone();
    Ok(bootstrap_impl(&state.packages, &next))
}

pub async fn environment_import(
    app: ServiceHandle,
    id: String,
    name: String,
    proton_path: String,
) -> Result<Bootstrap, String> {
    let worker = app.clone();
    let mut bootstrap = crate::service::spawn_blocking(move || {
        let state = &worker.state;
        import_impl(state, &id, &name, &proton_path)
    })
    .await
    .map_err(|error| format!("导入运行环境任务失败: {error}"))??;
    bootstrap.integrity = app.integrity.status();
    Ok(bootstrap)
}

pub(crate) fn import_impl(
    state: &AppState,
    id: &str,
    name: &str,
    proton_path: &str,
) -> Result<Bootstrap, String> {
    let _task = Task::begin(state)?;
    let snapshot = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    if !environment::valid_id(id) {
        return Err("运行环境 ID 无效".into());
    }
    if snapshot
        .runtime_environments
        .iter()
        .any(|entry| entry.id == id)
    {
        return Err(format!("运行环境 {id} 已登记"));
    }
    let directory = data_dir().join("environments").join(id);
    let metadata = std::fs::symlink_metadata(&directory)
        .map_err(|_| format!("运行环境目录不存在: {}", directory.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("运行环境目录无效: {}", directory.display()));
    }
    if !environment::owned_identity(&directory, id) {
        return Err("运行环境目录身份文件不匹配, 拒绝导入".into());
    }
    if !environment::prefix_ready(&directory) {
        return Err("运行环境 prefix 不完整, 拒绝导入".into());
    }
    let name = environment::validate_name(name).map_err(|error| format!("{error:#}"))?;
    if snapshot
        .runtime_environments
        .iter()
        .any(|entry| entry.name == name)
    {
        return Err(format!("运行环境名称「{name}」已存在, 请换一个名称"));
    }
    let channel = crate::channel::resolve(
        &state.packages,
        snapshot.active_game.as_str(),
        snapshot.current().distribution.as_ref(),
    )
    .map_err(|error| error.to_string())?;
    let target = installation(&state.packages, &snapshot, channel);
    if !target.installed {
        return Err(missing_install_error(channel, &target.install_root));
    }
    let proton = environment::ProtonPin::capture(&expand_path(proton_path))
        .map_err(|error| format!("{error:#}"))?;
    let install_root = target
        .install_root
        .canonicalize()
        .unwrap_or(target.install_root);
    // 导入的 prefix 可能仍绑定旧安装目录, 登记前先改绑到当前目录.
    if let Some(link_name) =
        crate::install::prefix_link_name(&state.packages, &snapshot.active_game)
    {
        let prefix = directory.join("prefix");
        let link = prefix.join("drive_c").join(link_name);
        match link_install_root(&link) {
            Some(old) if !oddity_platform::fs_tree::same_path(&old, &install_root) => {
                let _lock =
                    environment::lock_prefix(&prefix).map_err(|error| format!("{error:#}"))?;
                rewrite_binding(
                    &state.packages,
                    &snapshot,
                    channel.id,
                    &prefix,
                    &old,
                    &install_root,
                )
                .map_err(|error| format!("{error:#}"))?;
            }
            Some(_) => {}
            None => {
                // 链接缺失时创建到当前安装根; 注册表与官方配置没有旧值可替换.
                let _lock =
                    environment::lock_prefix(&prefix).map_err(|error| format!("{error:#}"))?;
                rewrite_binding(
                    &state.packages,
                    &snapshot,
                    channel.id,
                    &prefix,
                    &install_root,
                    &install_root,
                )
                .map_err(|error| format!("{error:#}"))?;
            }
        }
    }
    let imported = RuntimeEnvironment {
        id: id.to_string(),
        name: name.to_string(),
        proton,
        install_root,
        game: snapshot.active_game,
        created_at: crate::now_millis() / 1000,
    };
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    let mut next = settings.clone();
    // 当前区服的首个环境导入后即可使用, 后续导入不改变当前选择.
    let key = channel.id;
    if next.current().active_environment(key).is_none() {
        next.current_mut()
            .set_active_environment(key, Some(&imported.id));
        next.current_mut()
            .set_stable_environment(key, Some(&imported.id));
    }
    next.runtime_environments.push(imported);
    crate::settings::persist_from(&next, "environment.import")?;
    *settings = next.clone();
    Ok(bootstrap_impl(&state.packages, &next))
}

pub async fn environment_action(
    app: ServiceHandle,
    action: String,
    id: Option<String>,
) -> Result<Bootstrap, String> {
    let worker = app.clone();
    let mut bootstrap = crate::service::spawn_blocking(move || {
        let state = &worker.state;
        action_impl(state, &action, id)
    })
    .await
    .map_err(|error| format!("运行环境任务失败: {error}"))??;
    bootstrap.integrity = app.integrity.status();
    Ok(bootstrap)
}

pub(crate) fn action_impl(
    state: &AppState,
    action: &str,
    id: Option<String>,
) -> Result<Bootstrap, String> {
    let _task = Task::begin(state)?;
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    let mut next = settings.clone();
    let id = id.ok_or("请选择运行环境")?;
    let environment = next
        .runtime_environments
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or("运行环境不存在")?;
    if !environment.belongs_to(&next.active_game) {
        return Err(format!(
            "运行环境「{}」属于其他游戏, 请重新选择环境.",
            environment.name
        ));
    }
    let channel = channel_key(&state.packages, &next);
    match action {
        "switch" => {
            next.current_mut()
                .set_active_environment(&channel, Some(&id));
            validate_selection(&state.packages, &next)?;
        }
        "keep" => {
            let mut candidate = next.clone();
            candidate
                .current_mut()
                .set_active_environment(&channel, Some(&id));
            validate_selection(&state.packages, &candidate)?;
            next.current_mut()
                .set_stable_environment(&channel, Some(&id));
        }
        "discard" => {
            let entry = environment;
            if next
                .current()
                .active_environments
                .values()
                .any(|active| active == &entry.id)
                || next
                    .current()
                    .stable_environments
                    .values()
                    .any(|stable| stable == &entry.id)
            {
                return Err("不能删除当前环境或稳定环境, 请先切换并选择其他稳定环境".into());
            }
            // 设置提交失败时恢复目录, 不把删除到一半的 prefix 重新登记.
            let pending = environment::prepare_discard(&data_dir(), &entry)
                .map_err(|error| format!("{error:#}"))?;
            next.runtime_environments
                .retain(|profile| profile.id != entry.id);
            persist_settings(&next)?;
            *settings = next.clone();
            pending.commit().map_err(|error| format!("{error:#}"))?;
            return Ok(bootstrap_impl(&state.packages, &next));
        }
        "rebind" => {
            let channel = crate::channel::resolve(
                &state.packages,
                next.active_game.as_str(),
                next.current().distribution.as_ref(),
            )
            .map_err(|error| error.to_string())?;
            let target = installation(&state.packages, &next, channel);
            if !target.installed {
                return Err(missing_install_error(channel, &target.install_root));
            }
            // 固定的 Proton 必须始终可用, 即使安装目录已经一致.
            environment
                .proton
                .verify()
                .map_err(|error| format!("{error:#}"))?;
            // 记录统一存规范化路径, 比较时不受符号链接影响.
            let root = target
                .install_root
                .canonicalize()
                .unwrap_or(target.install_root);
            let index = next
                .runtime_environments
                .iter()
                .position(|entry| entry.id == environment.id)
                .ok_or("运行环境不存在")?;
            if next.runtime_environments[index].targets(&root) {
                return Ok(bootstrap_impl(&state.packages, &next));
            }
            let prefix = environment
                .prefix(&data_dir())
                .map_err(|error| format!("{error:#}"))?;
            let stale = environment.install_root.clone();
            {
                let _lock =
                    environment::lock_prefix(&prefix).map_err(|error| format!("{error:#}"))?;
                rewrite_binding(&state.packages, &next, channel.id, &prefix, &stale, &root)?;
            }
            next.runtime_environments[index].install_root = root;
            crate::settings::persist_from(&next, "environment.rebind")?;
            *settings = next.clone();
            return Ok(bootstrap_impl(&state.packages, &next));
        }
        "forget" => {
            let usable = environment.inspect(&data_dir()) == EnvironmentState::Ready
                && environment.proton.verify().is_ok();
            if usable {
                return Err("运行环境目录仍然可用, 请使用删除环境.".into());
            }
            next.runtime_environments
                .retain(|entry| entry.id != environment.id);
            let current = next.current_mut();
            current
                .active_environments
                .retain(|_, value| value != &environment.id);
            current
                .stable_environments
                .retain(|_, value| value != &environment.id);
            crate::settings::persist_from(&next, "environment.forget")?;
            *settings = next.clone();
            return Ok(bootstrap_impl(&state.packages, &next));
        }
        _ => return Err("不支持的运行环境操作".into()),
    }
    persist_settings(&next)?;
    *settings = next.clone();
    Ok(bootstrap_impl(&state.packages, &next))
}

pub async fn environment_rename(
    app: ServiceHandle,
    name: String,
    id: Option<String>,
) -> Result<Bootstrap, String> {
    let worker = app.clone();
    let mut bootstrap = crate::service::spawn_blocking(move || {
        let state = &worker.state;
        rename_impl(state, &name, id)
    })
    .await
    .map_err(|error| format!("运行环境任务失败: {error}"))??;
    bootstrap.integrity = app.integrity.status();
    Ok(bootstrap)
}

pub(crate) fn rename_impl(
    state: &AppState,
    name: &str,
    id: Option<String>,
) -> Result<Bootstrap, String> {
    let _task = Task::begin(state)?;
    let name = environment::validate_name(name).map_err(|error| format!("{error:#}"))?;
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    let mut next = settings.clone();
    let id = id.ok_or("请选择运行环境")?;
    let index = next
        .runtime_environments
        .iter()
        .position(|entry| entry.id == id)
        .ok_or("运行环境不存在")?;
    if next
        .runtime_environments
        .iter()
        .enumerate()
        .any(|(position, entry)| position != index && entry.name == name)
    {
        return Err(format!("运行环境名称「{name}」已存在, 请换一个名称"));
    }
    next.runtime_environments[index].name = name.to_string();
    persist_settings(&next)?;
    *settings = next.clone();
    Ok(bootstrap_impl(&state.packages, &next))
}

/// 外部启动时选择并持久化地区与运行环境.
#[derive(Debug, Clone)]
pub struct LaunchSelection {
    pub settings: Settings,
    pub changed: bool,
}

/// 该环境被哪些区服记住; 用于没有显式地区时反推启动地区.
fn environment_channel(registry: &Registry, settings: &Settings, id: &str) -> Option<ChannelId> {
    let current = settings.current();
    let found: Vec<&str> = crate::channel::channels(registry, settings.active_game.as_str())
        .iter()
        .map(|channel| channel.id)
        .filter(|channel| current.active_environment(channel) == Some(id))
        .collect();
    match found.as_slice() {
        [only] => Some(ChannelId::new(only)),
        _ => None,
    }
}

fn resolve_launch_target(
    registry: &Registry,
    settings: &Settings,
    region: Option<ChannelId>,
    environment: Option<&str>,
) -> Result<(ChannelId, String), String> {
    let game = settings.active_game.as_str();
    let requested = match environment {
        Some(value) => {
            let found = if let Some(entry) = settings
                .runtime_environments
                .iter()
                .find(|entry| entry.id == value)
            {
                entry
            } else {
                let mut matches = settings
                    .runtime_environments
                    .iter()
                    .filter(|entry| entry.name == value);
                let first = matches
                    .next()
                    .ok_or_else(|| format!("运行环境不存在: {value}"))?;
                if matches.next().is_some() {
                    return Err(format!("运行环境名称重复: {value}. 请改用环境 ID."));
                }
                first
            };
            if !found.belongs_to(&settings.active_game) {
                return Err(format!(
                    "运行环境「{}」属于其他游戏, 请重新选择环境.",
                    found.name
                ));
            }
            Some(found.id.clone())
        }
        None => None,
    };

    // 地区优先级: 显式参数 > 该环境被记住的区服 > 当前设置.
    let channel = match region {
        Some(region) => crate::channel::resolve(registry, game, Some(&region))
            .map_err(|error| error.to_string())?,
        None => {
            let fallback = requested
                .as_deref()
                .and_then(|id| environment_channel(registry, settings, id));
            match fallback {
                Some(region) => crate::channel::resolve(registry, game, Some(&region))
                    .map_err(|error| error.to_string())?,
                None => crate::channel::resolve(
                    registry,
                    game,
                    settings.current().distribution.as_ref(),
                )
                .map_err(|error| error.to_string())?,
            }
        }
    };

    let mut target = settings.clone();
    target.current_mut().distribution = Some(ChannelId::new(channel.id));
    crate::install::launcher_root(registry, &target)
        .ok_or_else(|| format!("尚未找到{}安装, 请先在 Oddity 中完成安装.", channel.label))?;

    let id = match requested {
        Some(id) => id,
        None => settings
            .current()
            .active_environment(channel.id)
            .ok_or_else(|| format!("尚未为{}选择运行环境, 请先在 Oddity 中选择.", channel.label))?
            .to_string(),
    };
    let environment = settings
        .runtime_environments
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "所选运行环境不存在, 请重新选择环境".to_string())?;
    if !environment.belongs_to(&settings.active_game) {
        return Err(format!(
            "运行环境「{}」属于其他游戏, 请重新选择环境.",
            environment.name
        ));
    }
    Ok((ChannelId::new(channel.id), id))
}

/// 解析请求游戏, 地区与环境, 必要时切换并持久化. 没有指定时沿用该游戏的当前设置.
pub fn select_for_launch(
    state: &AppState,
    game: &GameId,
    region: Option<ChannelId>,
    environment: Option<&str>,
) -> Result<LaunchSelection, String> {
    let current = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    if region.is_none() && environment.is_none() && &current.active_game == game {
        return Ok(LaunchSelection {
            settings: current,
            changed: false,
        });
    }

    let mut next = current.clone();
    next.active_game = game.clone();
    next.normalize();
    apply_launch_target(&state.packages, &mut next, region.clone(), environment)?;
    if launch_selection_matches(&current, &next) {
        return Ok(LaunchSelection {
            settings: current,
            changed: false,
        });
    }

    let task = Task::begin(state)?;
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    let mut next = settings.clone();
    next.active_game = game.clone();
    next.normalize();
    apply_launch_target(&state.packages, &mut next, region, environment)?;
    if launch_selection_matches(&settings, &next) {
        let snapshot = settings.clone();
        drop(settings);
        drop(task);
        return Ok(LaunchSelection {
            settings: snapshot,
            changed: false,
        });
    }

    validate_selection(&state.packages, &next)?;
    persist_settings(&next)?;
    *settings = next.clone();
    drop(settings);
    drop(task);
    Ok(LaunchSelection {
        settings: next,
        changed: true,
    })
}

fn apply_launch_target(
    registry: &Registry,
    settings: &mut Settings,
    region: Option<ChannelId>,
    environment: Option<&str>,
) -> Result<(), String> {
    if region.is_none() && environment.is_none() {
        return Ok(());
    }
    let (distribution, environment_id) =
        resolve_launch_target(registry, settings, region, environment)?;
    let channel = distribution.as_str().to_string();
    settings.current_mut().distribution = Some(distribution);
    settings
        .current_mut()
        .set_active_environment(&channel, Some(&environment_id));
    Ok(())
}

fn launch_selection_matches(current: &Settings, next: &Settings) -> bool {
    let channel = next
        .current()
        .distribution
        .as_ref()
        .map(ChannelId::as_str)
        .unwrap_or_default()
        .to_string();
    current.active_game == next.active_game
        && current.current().distribution == next.current().distribution
        && current.current().active_environment(&channel)
            == next.current().active_environment(&channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Registry;
    use crate::tests::{fake_proton, EnvGuard, TempDir};
    use std::sync::{Mutex, MutexGuard};

    // 需要同时持有包级下载状态锁和环境锁时, 固定先取 serial 再取 EnvGuard,
    // 避免不同测试之间形成 ABBA 锁序. 字段按声明顺序析构, 所以环境锁在前,
    // 释放顺序与获取顺序相反.
    struct Fixture {
        directory: TempDir,
        state: AppState,
        _env: EnvGuard,
        _serial: MutexGuard<'static, ()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            crate::testing::reset();
        }
    }

    impl Fixture {
        fn new() -> Self {
            let _serial = crate::testing::serial();
            let mut env = EnvGuard::new();
            let directory = TempDir::new("environment-commands");
            env.set("HOME", directory.join("home"));
            env.set("XDG_CONFIG_HOME", directory.join("config"));
            env.set("XDG_DATA_HOME", directory.join("data"));
            env.remove("NTE_ROOT");
            env.remove("NTE_INSTALL");
            directory.write_text(
                format!("install/{}/Config/Config.ini", crate::testing::LAUNCHER_DIR),
                "[VERSION]\nVersion=1.0\n",
            );
            directory.write(
                format!(
                    "install/{}/{}",
                    crate::testing::LAUNCHER_DIR,
                    crate::testing::CLIENT_EXE
                ),
                "MZ",
            );
            directory.write(format!("install/{}", crate::testing::GAME_EXE), "MZ");
            fake_proton(directory.path(), "Proton-A");
            fake_proton(directory.path(), "Proton-B");
            let state = AppState {
                settings: Mutex::new(crate::tests::settings_with_game(
                    ChannelId::new("cn"),
                    Some(directory.join("install")),
                    Some(directory.join("global")),
                )),
                game: Mutex::new(Registry::default()),
                packages: crate::testing::packages(),
            };
            crate::testing::reset();
            Self {
                directory,
                state,
                _env: env,
                _serial,
            }
        }

        fn create(
            &self,
            name: &str,
            source: Option<&str>,
            proton: &str,
        ) -> Result<Bootstrap, String> {
            create_impl(
                &self.state,
                source,
                name,
                &self.directory.join(proton).display().to_string(),
            )
        }

        fn install_global(&self) {
            self.directory.write_text(
                "global/NTEGlobal/Config/Config.ini",
                "[General]\nGameID=3000001\n[VERSION]\nVersion=1.0\n",
            );
            self.directory
                .write("global/NTEGlobal/NTEGlobalGame.exe", "MZ");
            self.directory
                .write(format!("global/{}", crate::testing::GAME_EXE), "MZ");
        }

        fn create_global(&self, name: &str, source: Option<&str>) -> Result<Bootstrap, String> {
            // 创建绑定当前地区的安装目录; 结束后恢复调用方的地区选择.
            let before = {
                let mut settings = self.state.settings.lock().unwrap();
                let before = settings.current().distribution.clone();
                settings.current_mut().distribution = Some(ChannelId::new("global"));
                before
            };
            let result = create_impl(
                &self.state,
                source,
                name,
                &self.directory.join("Proton-B").display().to_string(),
            );
            self.state
                .settings
                .lock()
                .unwrap()
                .current_mut()
                .distribution = before;
            result
        }

        fn first(&self) -> RuntimeEnvironment {
            self.create("A", None, "Proton-A")
                .unwrap()
                .settings
                .runtime_environments[0]
                .clone()
        }

        /// 在测试目录下准备一份可用的国服安装, 返回安装根目录.
        fn install_root(&self, name: &str) -> PathBuf {
            let root = self.directory.join(name);
            self.directory.write_text(
                format!("{name}/{}/Config/Config.ini", crate::testing::LAUNCHER_DIR),
                "[VERSION]\nVersion=1.0\n",
            );
            self.directory
                .write(format!("{name}/{}", crate::testing::GAME_EXE), "MZ");
            root
        }

        /// 把国服的安装目录指向给定路径.
        fn use_install_root(&self, root: &Path) {
            self.state
                .settings
                .lock()
                .unwrap()
                .current_mut()
                .install_roots
                .insert("cn".into(), root.display().to_string());
        }

        /// 手工构造一个可导入的环境目录.
        fn importable(&self, id: &str) -> PathBuf {
            let directory = data_dir().join("environments").join(id);
            std::fs::create_dir_all(directory.join("prefix/drive_c")).unwrap();
            std::fs::write(directory.join(".oddity-environment"), id).unwrap();
            directory
        }

        fn import(&self, id: &str, name: &str, proton: &str) -> Result<Bootstrap, String> {
            import_impl(
                &self.state,
                id,
                name,
                &self.directory.join(proton).display().to_string(),
            )
        }

        fn audit(&self) -> Result<EnvironmentAudit, String> {
            audit(&self.state)
        }

        fn select(&self, distribution: &str) {
            self.state
                .settings
                .lock()
                .unwrap()
                .current_mut()
                .distribution = Some(ChannelId::new(distribution));
        }

        /// 当前区服记住的当前与稳定环境.
        fn remembered(&self, channel: &str) -> (Option<String>, Option<String>) {
            let settings = self.state.settings.lock().unwrap();
            (
                settings
                    .current()
                    .active_environment(channel)
                    .map(str::to_string),
                settings
                    .current()
                    .stable_environment(channel)
                    .map(str::to_string),
            )
        }

        fn action(&self, action: &str, id: Option<&str>) -> Result<Bootstrap, String> {
            action_impl(&self.state, action, id.map(str::to_string))
        }

        fn rename(&self, name: &str, id: Option<&str>) -> Result<Bootstrap, String> {
            rename_impl(&self.state, name, id.map(str::to_string))
        }
    }

    #[test]
    fn first_environment_is_required_and_automatically_selected_and_kept() {
        let fixture = Fixture::new();
        let original = fixture.state.settings.lock().unwrap().clone();
        assert!(prefix(&fixture.state.packages, &original)
            .unwrap_err()
            .contains("请先"));
        assert!(verify_proton(&fixture.state.packages, &original).is_err());
        assert!(log_dir(&fixture.state.packages, &original).is_err());
        assert!(detect_proton(&fixture.state.packages, &original).is_none());
        let a = fixture.first();
        let settings = crate::settings::load();
        assert_eq!(
            settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );
        assert_eq!(
            settings.current().stable_environment("cn"),
            settings.current().active_environment("cn")
        );
        assert!(settings.current().active_environment("global").is_none());
        assert!(prefix(&fixture.state.packages, &settings)
            .unwrap()
            .ends_with(format!("{}/prefix", a.id)));
        assert!(log_dir(&fixture.state.packages, &settings)
            .unwrap()
            .ends_with(&a.id));
        assert!(fixture.action("switch", None).is_err());
        assert!(fixture.action("keep", None).is_err());
        assert!(fixture
            .action("discard", Some(&a.id))
            .unwrap_err()
            .contains("当前环境"));
        assert!(fixture
            .create("A", None, "Proton-B")
            .unwrap_err()
            .contains("运行环境名称「A」已存在"));
        assert!(fixture.action("switch", Some("missing")).is_err());
        assert!(fixture.action("unsupported", Some(&a.id)).is_err());
    }

    #[test]
    fn rename_keeps_current_and_stable_environment_and_rejects_duplicate_names() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let b = fixture
            .create("B", None, "Proton-B")
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();

        let renamed = fixture.rename(" A 稳定 ", Some(&a.id)).unwrap();
        let entry = &renamed.settings.runtime_environments[0];
        assert_eq!(entry.id, a.id);
        assert_eq!(entry.name, "A 稳定");
        assert_eq!(
            renamed.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );
        assert_eq!(
            renamed.settings.current().stable_environment("cn"),
            Some(a.id.as_str())
        );
        assert_eq!(
            prefix(&fixture.state.packages, &renamed.settings)
                .unwrap()
                .to_str(),
            a.prefix(&data_dir()).unwrap().to_str()
        );
        assert!(fixture
            .rename("A 稳定", Some(&b.id))
            .unwrap_err()
            .contains("「A 稳定」已存在"));
        assert!(fixture
            .rename("", Some(&b.id))
            .unwrap_err()
            .contains("1-64"));
        assert!(fixture
            .rename(&"名".repeat(65), Some(&b.id))
            .unwrap_err()
            .contains("1-64"));
        assert!(fixture
            .rename("B 改名", Some("missing"))
            .unwrap_err()
            .contains("不存在"));

        let settings = crate::settings::load();
        assert_eq!(settings.runtime_environments[0].name, "A 稳定");
    }

    #[test]
    fn rename_follows_the_environment_task_lock() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let task = Task::begin(&fixture.state).unwrap();
        assert!(fixture
            .rename("A 改名", Some(&a.id))
            .unwrap_err()
            .contains("运行环境任务"));
        drop(task);
        fixture.state.game.lock().unwrap().starting = true;
        assert!(fixture.rename("A 改名", Some(&a.id)).is_err());
        fixture.state.game.lock().unwrap().starting = false;
        fixture.rename("A 改名", Some(&a.id)).unwrap();
    }

    #[test]
    fn selected_source_can_be_cloned_while_another_environment_is_active() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let relative = "drive_c/settings.ini";
        std::fs::write(a.prefix(&data_dir()).unwrap().join(relative), "A settings").unwrap();
        let created = fixture.create("B", Some(&a.id), "Proton-B").unwrap();
        let b = created
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();
        assert_eq!(
            created.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );
        fixture.action("switch", Some(&b.id)).unwrap();
        std::fs::write(b.prefix(&data_dir()).unwrap().join(relative), "B settings").unwrap();
        let copied = fixture.create("A 副本", Some(&a.id), "Proton-B").unwrap();
        let copy = copied.settings.runtime_environments.last().unwrap();
        assert_eq!(
            std::fs::read_to_string(copy.prefix(&data_dir()).unwrap().join(relative)).unwrap(),
            "A settings"
        );
        assert_eq!(
            copied.settings.current().active_environment("cn"),
            Some(b.id.as_str())
        );
        assert_eq!(
            copied.settings.current().stable_environment("cn"),
            Some(a.id.as_str())
        );
        assert!(fixture
            .create("无效源", Some("missing"), "Proton-A")
            .unwrap_err()
            .contains("不存在"));
        fixture.action("keep", Some(&b.id)).unwrap();
        let deleted = fixture.action("discard", Some(&a.id)).unwrap();
        assert_eq!(deleted.settings.runtime_environments.len(), 2);
        assert!(a.prefix(&data_dir()).is_err());
        assert_eq!(
            std::fs::read_to_string(b.prefix(&data_dir()).unwrap().join(relative)).unwrap(),
            "B settings"
        );
        fixture.action("switch", Some(&copy.id)).unwrap();
        assert!(fixture
            .action("discard", Some(&b.id))
            .unwrap_err()
            .contains("稳定环境"));
    }

    #[test]
    fn changed_proton_blocks_selection_and_dangling_records_are_reported() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let created = fixture.create("B", Some(&a.id), "Proton-B").unwrap();
        let b = created.settings.runtime_environments.last().unwrap();
        fixture.directory.write_text("Proton-B/proton", "updated");
        assert!(fixture
            .action("switch", Some(&b.id))
            .unwrap_err()
            .contains("Proton 已变化"));
        assert!(fixture.action("keep", Some(&b.id)).is_err());
        assert_eq!(
            crate::settings::load().current().active_environment("cn"),
            Some(a.id.as_str())
        );

        let mut settings = created.settings.clone();
        settings
            .current_mut()
            .set_active_environment("cn", Some("missing"));
        assert!(selected(&fixture.state.packages, &settings).is_err());
        assert!(verify_proton(&fixture.state.packages, &settings).is_err());
        assert!(detect_proton(&fixture.state.packages, &settings).is_none());
    }

    #[test]
    fn switching_channels_restores_that_channels_environment() {
        let fixture = Fixture::new();
        fixture.install_global();
        let a = fixture.first();
        let global = fixture
            .create_global("国际服", None)
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();
        // 两个区服各记各的: 国际服第一次创建环境时占住自己的槽位, 不动国服.
        assert_eq!(fixture.remembered("cn").0.as_deref(), Some(a.id.as_str()));
        assert_eq!(
            fixture.remembered("global").0.as_deref(),
            Some(global.id.as_str())
        );

        // 切换环境只影响当前地区, 所以要先切到国际服再选.
        fixture.select("global");
        fixture.action("switch", Some(&global.id)).unwrap();
        assert_eq!(
            fixture.remembered("global").0.as_deref(),
            Some(global.id.as_str())
        );

        fixture.select("cn");
        assert_eq!(
            prefix(
                &fixture.state.packages,
                &fixture.state.settings.lock().unwrap()
            )
            .unwrap()
            .to_str(),
            a.prefix(&data_dir()).unwrap().to_str()
        );
        fixture.select("global");
        assert_eq!(
            prefix(
                &fixture.state.packages,
                &fixture.state.settings.lock().unwrap()
            )
            .unwrap()
            .to_str(),
            global.prefix(&data_dir()).unwrap().to_str()
        );
    }

    #[test]
    fn switching_environments_keeps_the_current_channel() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let b = fixture
            .create("B", Some(&a.id), "Proton-B")
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();
        let switched = fixture.action("switch", Some(&b.id)).unwrap();
        assert_eq!(
            switched.settings.current().distribution.as_ref(),
            Some(&ChannelId::new("cn"))
        );
        assert_eq!(
            switched.settings.current().active_environment("cn"),
            Some(b.id.as_str())
        );
    }

    #[test]
    fn aligning_retargets_the_prefix_to_the_current_installation() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let prefix = a.prefix(&data_dir()).unwrap();
        let relative = crate::testing::PREFIX_LINK;
        std::fs::create_dir_all(prefix.join(relative).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&a.install_root, prefix.join(relative)).unwrap();
        std::fs::write(prefix.join("system.reg"), "").unwrap();

        let moved = fixture.directory.join("moved");
        std::fs::create_dir_all(&moved).unwrap();
        fixture
            .state
            .settings
            .lock()
            .unwrap()
            .current_mut()
            .install_roots
            .insert("cn".into(), moved.display().to_string());

        let mut settings = fixture.state.settings.lock().unwrap().clone();
        settings.current_mut().launch_mode = "direct".into();
        fixture.state.settings.lock().unwrap().theme = "dark".into();
        let target = resolve(&fixture.state.packages, &settings).unwrap();
        let lock = environment::lock_prefix(&prefix).unwrap();
        let error = lock_and_align(&fixture.state, &settings, &target).unwrap_err();
        assert!(error.contains("Wine prefix 正在被其他任务使用"), "{error}");
        assert_eq!(
            std::fs::read_link(prefix.join(relative)).unwrap(),
            a.install_root
        );
        drop(lock);

        let (_prefix, _install_lock, _prefix_lock) =
            lock_and_align(&fixture.state, &settings, &target).unwrap();
        assert_eq!(
            std::fs::read_link(prefix.join(relative)).unwrap(),
            moved.canonicalize().unwrap()
        );
        let saved = crate::settings::load();
        assert_eq!(
            saved.runtime_environments[0].install_root,
            moved.canonicalize().unwrap()
        );
        assert_eq!(saved.theme, "dark");
        assert_eq!(
            saved.current().launch_mode,
            fixture.state.settings.lock().unwrap().current().launch_mode
        );
        assert_ne!(saved.current().launch_mode, settings.current().launch_mode);
    }

    #[test]
    fn missing_game_cannot_be_used_to_create_an_environment() {
        let fixture = Fixture::new();
        let availability = installations(
            &fixture.state.packages,
            &fixture.state.settings.lock().unwrap(),
        );
        assert!(availability[0].installed);
        assert!(!availability[1].installed);
        let error = fixture.create_global("国际服", None).unwrap_err();
        assert!(
            error.contains("国际服 的安装目录未找到官方启动器或游戏文件"),
            "{error}"
        );
        std::fs::remove_file(
            fixture
                .directory
                .join(format!("install/{}", crate::testing::GAME_EXE)),
        )
        .unwrap();
        let error = fixture.create("国服", None, "Proton-A").unwrap_err();
        assert!(
            error.contains("国服 的安装目录未找到官方启动器或游戏文件"),
            "{error}"
        );
        assert!(
            !installations(
                &fixture.state.packages,
                &fixture.state.settings.lock().unwrap()
            )[0]
            .installed
        );
        assert!(fixture
            .state
            .settings
            .lock()
            .unwrap()
            .runtime_environments
            .is_empty());
        assert!(!data_dir().join("environments").exists());
    }

    #[test]
    fn copies_follow_the_channel_they_were_created_in() {
        let fixture = Fixture::new();
        fixture.install_global();
        let a = fixture.first();
        let source = a.prefix(&data_dir()).unwrap();
        std::fs::write(source.join("user.reg"), "CN settings").unwrap();
        let relative = crate::testing::PREFIX_LINK;
        std::fs::create_dir_all(source.join(relative).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&a.install_root, source.join(relative)).unwrap();
        let created = fixture.create_global("国际服副本", Some(&a.id)).unwrap();
        let global = created.settings.runtime_environments.last().unwrap();
        // 副本绑定国际服当前生效的安装目录.
        assert_eq!(global.install_root, fixture.directory.join("global"));
        let copied = global.prefix(&data_dir()).unwrap();
        assert_eq!(
            std::fs::read_to_string(copied.join("user.reg")).unwrap(),
            "CN settings"
        );
        assert_eq!(
            std::fs::read_link(copied.join(relative)).unwrap(),
            global.install_root
        );
        assert_eq!(
            created.settings.current().active_environment("global"),
            Some(global.id.as_str())
        );
        // 创建副本不会动国服的选择.
        assert_eq!(
            created.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );

        // 切换环境不改地区; 当前仍在国服, 所以单独切到国际服再选.
        fixture.select("global");
        let switched = fixture.action("switch", Some(&global.id)).unwrap();
        assert_eq!(
            switched.settings.current().distribution.as_ref(),
            Some(&ChannelId::new("global"))
        );
        assert_eq!(
            switched.install_root,
            global.install_root.display().to_string()
        );
        assert!(switched.game_exists);
        assert!(switched
            .runtime_installations
            .iter()
            .all(|entry| entry.installed));

        fixture.select("cn");
        let back = fixture
            .create("国服副本", Some(&global.id), "Proton-A")
            .unwrap();
        let cn = back.settings.runtime_environments.last().unwrap();
        assert_eq!(cn.install_root, a.install_root);
        assert_eq!(
            back.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );
    }

    #[test]
    fn first_environment_activates_the_selected_installed_region() {
        let fixture = Fixture::new();
        fixture.install_global();
        let created = fixture.create_global("国际服", None).unwrap();
        let global = &created.settings.runtime_environments[0];
        assert_eq!(
            created.settings.current().distribution.as_ref(),
            Some(&ChannelId::new("global"))
        );
        assert_eq!(
            created.settings.current().active_environment("global"),
            Some(global.id.as_str())
        );
        assert_eq!(
            created.settings.current().active_environment("global"),
            created.settings.current().stable_environment("global")
        );
        assert!(created.game_exists);
        assert!(created.ok);
    }

    #[test]
    fn failed_persistence_cleans_environment_and_does_not_replace_memory() {
        let fixture = Fixture::new();
        // 把设置文件替换成目录, 提交设置必定失败; 不依赖配置目录的创建时序.
        std::fs::create_dir_all(crate::paths::settings_path().parent().unwrap()).unwrap();
        std::fs::create_dir(crate::paths::settings_path()).unwrap();
        assert!(fixture.create("A", None, "Proton-A").is_err());
        let settings = fixture.state.settings.lock().unwrap();
        assert!(settings.runtime_environments.is_empty());
        assert!(settings.current().active_environment("cn").is_none());
        assert_eq!(
            std::fs::read_dir(data_dir().join("environments"))
                .unwrap()
                .count(),
            0
        );
        assert!(!fixture.state.game.lock().unwrap().environment_busy);
    }

    #[test]
    fn failed_persistence_keeps_selection_and_restores_deleted_directory() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let created = fixture.create("B", Some(&a.id), "Proton-B").unwrap();
        let b = created.settings.runtime_environments.last().unwrap();
        std::fs::remove_file(crate::paths::settings_path()).unwrap();
        std::fs::create_dir(crate::paths::settings_path()).unwrap();
        assert!(fixture.action("switch", Some(&b.id)).is_err());
        assert!(fixture.action("discard", Some(&b.id)).is_err());
        assert!(b.prefix(&data_dir()).is_ok());
        assert_eq!(
            fixture
                .state
                .settings
                .lock()
                .unwrap()
                .current()
                .active_environment("cn"),
            Some(a.id.as_str())
        );
    }

    #[test]
    fn environment_tasks_block_overlaps_and_runtime_settings_changes() {
        let fixture = Fixture::new();
        {
            let task = Task::begin(&fixture.state).unwrap();
            assert!(fixture.create("A", None, "Proton-A").is_err());
            assert!(fixture.action("switch", None).is_err());
            drop(task);
        }
        fixture.state.game.lock().unwrap().starting = true;
        assert!(fixture.create("A", None, "Proton-A").is_err());
        fixture.state.game.lock().unwrap().starting = false;
        crate::testing::set_package_busy(true, true);
        assert!(fixture
            .create("A", None, "Proton-A")
            .unwrap_err()
            .contains("资源任务正在进行"));
        assert!(fixture.action("switch", None).is_err());
        crate::testing::reset();
        let original = Settings::default();
        let mut next = original.clone();
        next.current_mut()
            .install_roots
            .insert("cn".into(), "/other".into());
        assert!(validate_settings_change(&original, &next, true).is_err());
        assert!(validate_settings_change(&original, &next, false).is_ok());
        next = original.clone();
        next.theme = "dark".into();
        assert!(validate_settings_change(&original, &next, true).is_ok());
        next.current_mut()
            .set_active_environment("cn", Some("foreign"));
        assert!(validate_settings_change(&original, &next, false).is_err());
    }

    #[test]
    fn external_prefix_process_blocks_copy_until_it_exits() {
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let fixture = Fixture::new();
        let a = fixture.first();
        let mut child = Command::new("/bin/sh")
            .args(["-c", "echo ready; read -r release"])
            .env("WINEPREFIX", a.prefix(&data_dir()).unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // 子进程确认 exec 完成后才扫描 /proc, 避免读到继承自测试进程的旧环境变量.
        let mut ready = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let result = fixture.create("B", Some(&a.id), "Proton-B");
        drop(child.stdin.take());
        child.wait().unwrap();
        assert_eq!(ready.trim(), "ready");
        assert!(result.unwrap_err().contains("仍被进程"));
        fixture.create("B", Some(&a.id), "Proton-B").unwrap();
    }

    #[test]
    fn launch_selection_switches_region_and_restores_its_environment() {
        let fixture = Fixture::new();
        fixture.install_global();
        let a = fixture.first();
        let created = fixture.create_global("国际服", None).unwrap();
        let global = created
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();

        let selection = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("global")),
            None,
        )
        .unwrap();
        assert!(selection.changed);
        assert_eq!(
            selection.settings.current().distribution.as_ref(),
            Some(&ChannelId::new("global"))
        );
        assert_eq!(
            selection.settings.current().active_environment("global"),
            Some(global.id.as_str())
        );
        assert_eq!(
            crate::settings::load().current().distribution.as_ref(),
            Some(&ChannelId::new("global"))
        );

        // 切回国服时自动回到国服记住的环境, 不需要再指定.
        let selection = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("cn")),
            None,
        )
        .unwrap();
        assert!(selection.changed);
        assert_eq!(
            selection.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );

        let selection =
            select_for_launch(&fixture.state, &GameId::from("nte"), None, Some("A")).unwrap();
        assert!(!selection.changed);
        assert_eq!(
            selection.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );
    }

    #[test]
    fn launch_selection_validates_region_environment_and_install() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let error = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("global")),
            None,
        )
        .unwrap_err();
        assert!(error.contains("尚未找到国际服安装"));
        assert!(
            select_for_launch(&fixture.state, &GameId::from("nte"), None, Some("missing"),)
                .is_err()
        );
        // 国际服还没装, 地区校验先于环境选择.
        let error = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("global")),
            Some(&a.id),
        )
        .unwrap_err();
        assert!(error.contains("尚未找到国际服安装"), "{error}");
        let selection = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("cn")),
            None,
        )
        .unwrap();
        assert!(!selection.changed);
        assert_eq!(
            selection.settings.current().active_environment("cn"),
            Some(a.id.as_str())
        );
    }

    #[test]
    fn launch_selection_uses_the_environment_remembered_for_the_requested_channel() {
        let fixture = Fixture::new();
        fixture.install_global();
        let a = fixture.first();
        let b = fixture
            .create("B", Some(&a.id), "Proton-B")
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();
        let global = fixture
            .create_global("国际服", None)
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();

        // 国服记着 B, 国际服记着国际服环境; 选国际服不会串到国服的环境.
        fixture.select("cn");
        fixture.action("switch", Some(&b.id)).unwrap();
        fixture.select("global");
        fixture.action("switch", Some(&global.id)).unwrap();
        fixture.select("cn");

        let selection = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("global")),
            None,
        )
        .unwrap();
        assert_eq!(
            selection.settings.current().active_environment("global"),
            Some(global.id.as_str())
        );
        assert_eq!(
            selection.settings.current().active_environment("cn"),
            Some(b.id.as_str())
        );

        let selection = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("cn")),
            Some("B"),
        )
        .unwrap();
        assert_eq!(
            selection.settings.current().active_environment("cn"),
            Some(b.id.as_str())
        );
    }

    #[test]
    fn launch_selection_keeps_current_settings_without_arguments() {
        let fixture = Fixture::new();
        fixture.first();
        let before = crate::settings::load();
        let selection =
            select_for_launch(&fixture.state, &GameId::from("nte"), None, None).unwrap();
        assert!(!selection.changed);
        assert_eq!(
            selection.settings.current().distribution,
            before.current().distribution
        );
        assert_eq!(
            selection.settings.current().active_environments,
            before.current().active_environments
        );
    }

    #[test]
    fn launch_selection_switches_game_and_rejects_environments_of_other_games() {
        let fixture = Fixture::new();
        let _a = fixture.first();
        let error =
            select_for_launch(&fixture.state, &GameId::from("hsr"), None, Some("A")).unwrap_err();
        assert!(error.contains("属于其他游戏"), "{error}");

        fixture.state.settings.lock().unwrap().active_game = GameId::from("hsr");
        let selection =
            select_for_launch(&fixture.state, &GameId::from("nte"), None, None).unwrap();
        assert!(selection.changed);
        assert_eq!(selection.settings.active_game, GameId::from("nte"));
        assert_eq!(
            crate::settings::load().active_game,
            GameId::from("nte"),
            "切换游戏需要写回设置"
        );
    }

    #[test]
    fn launch_selection_honors_busy_state() {
        let fixture = Fixture::new();
        fixture.install_global();
        let _a = fixture.first();
        fixture.create_global("国际服", None).unwrap();
        fixture.state.game.lock().unwrap().starting = true;
        let error = select_for_launch(
            &fixture.state,
            &GameId::from("nte"),
            Some(ChannelId::new("global")),
            None,
        )
        .unwrap_err();
        assert!(error.contains("正在进行"));
        assert_eq!(
            fixture
                .state
                .settings
                .lock()
                .unwrap()
                .current()
                .distribution
                .as_ref(),
            Some(&ChannelId::new("cn"))
        );
    }

    fn named_environment(name: &str) -> RuntimeEnvironment {
        RuntimeEnvironment {
            id: "0".repeat(32),
            name: name.into(),
            proton: crate::environment::ProtonPin {
                path: "/proton".into(),
                identity: "identity".into(),
            },
            install_root: "/install".into(),
            game: GameId::from("nte"),
            created_at: 0,
        }
    }

    #[test]
    fn suggested_names_avoid_occupied_ones_and_fit_the_limit() {
        let mut settings = Settings::default();
        let base = "异环-国服";
        assert_eq!(suggested_name(&settings, base), "异环-国服-1");

        settings.runtime_environments = vec![
            named_environment(&format!("{base}-1")),
            named_environment(&format!("{base}-3")),
            named_environment(&format!("{base}-旧")),
        ];
        assert_eq!(suggested_name(&settings, base), "异环-国服-4");

        settings.runtime_environments.push(named_environment(base));
        settings
            .runtime_environments
            .push(named_environment(&format!("{base}-5")));
        assert_eq!(suggested_name(&settings, base), "异环-国服-6");

        let long = "名".repeat(80);
        settings.runtime_environments.clear();
        let truncated = suggested_name(&settings, &long);
        assert_eq!(truncated.chars().count(), 64);
        assert!(truncated.ends_with("-1"));
        assert!(environment::validate_name(&truncated).is_ok());
        assert_eq!(suggested_name(&settings, &long), truncated);

        settings
            .runtime_environments
            .push(named_environment(&truncated));
        let next = suggested_name(&settings, &long);
        assert_ne!(next, truncated);
        assert_eq!(next.chars().count(), 64);
        assert!(next.ends_with("-2"));
        assert_eq!(suggested_name(&settings, &long), next);
    }

    #[test]
    fn rebinding_moves_the_environment_to_the_current_install_root() {
        let fixture = Fixture::new();
        let a = fixture.first();
        // 目录没有变化时直接成功, 不校验目录之外的记录.
        fixture.action("rebind", Some(&a.id)).unwrap();

        let moved = fixture.install_root("moved");
        fixture.use_install_root(&moved);
        let rebound = fixture.action("rebind", Some(&a.id)).unwrap();
        let entry = rebound.settings.runtime_environments[0].clone();
        assert_eq!(entry.install_root, moved.canonicalize().unwrap());
        assert!(entry.targets(&moved));
        assert_eq!(
            std::fs::read_link(
                entry
                    .prefix(&data_dir())
                    .unwrap()
                    .join(crate::testing::PREFIX_LINK)
            )
            .unwrap(),
            moved.canonicalize().unwrap()
        );
        assert_eq!(
            crate::settings::load().runtime_environments[0].install_root,
            moved.canonicalize().unwrap()
        );
        // 已经对齐后再次更新绑定不产生改动.
        fixture.action("rebind", Some(&a.id)).unwrap();

        // 目标安装目录不完整时拒绝更新绑定.
        fixture.use_install_root(&fixture.directory.join("missing"));
        let error = fixture.action("rebind", Some(&a.id)).unwrap_err();
        assert!(error.contains("未找到官方启动器或游戏文件"), "{error}");
        assert_eq!(
            crate::settings::load().runtime_environments[0].install_root,
            moved.canonicalize().unwrap()
        );

        // 环境目录失效时校验失败, 不写入新记录.
        let other = fixture.install_root("other");
        fixture.use_install_root(&other);
        std::fs::remove_dir_all(entry.directory(&data_dir()).unwrap()).unwrap();
        let error = fixture.action("rebind", Some(&a.id)).unwrap_err();
        assert!(error.contains("拒绝操作"), "{error}");
        assert_eq!(
            crate::settings::load().runtime_environments[0].install_root,
            moved.canonicalize().unwrap()
        );
    }

    #[test]
    fn rebind_stores_the_canonical_install_root_for_symlinked_paths() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let moved = fixture.install_root("moved");
        let alias = fixture.directory.join("alias");
        std::os::unix::fs::symlink(&moved, &alias).unwrap();
        fixture.use_install_root(&alias);

        let rebound = fixture.action("rebind", Some(&a.id)).unwrap();
        let entry = &rebound.settings.runtime_environments[0];
        assert_eq!(entry.install_root, moved.canonicalize().unwrap());
        assert!(entry.targets(&moved));
        assert_eq!(
            std::fs::read_link(
                entry
                    .prefix(&data_dir())
                    .unwrap()
                    .join(crate::testing::PREFIX_LINK)
            )
            .unwrap(),
            moved.canonicalize().unwrap()
        );
        assert_eq!(
            crate::settings::load().runtime_environments[0].install_root,
            moved.canonicalize().unwrap()
        );
    }

    #[test]
    fn rebind_verifies_the_pinned_proton_even_when_the_install_root_matches() {
        let fixture = Fixture::new();
        let a = fixture.first();
        fixture.directory.write("Proton-A/proton", "updated");
        let error = fixture.action("rebind", Some(&a.id)).unwrap_err();
        assert!(error.contains("Proton 已变化"), "{error}");
        assert_eq!(
            crate::settings::load().runtime_environments[0].install_root,
            a.install_root
        );
    }

    #[test]
    fn forget_removes_only_unusable_environments_and_clears_selection() {
        let fixture = Fixture::new();
        let a = fixture.first();
        assert_eq!(fixture.remembered("cn").0.as_deref(), Some(a.id.as_str()));
        let error = fixture.action("forget", Some(&a.id)).unwrap_err();
        assert!(error.contains("仍然可用"), "{error}");
        assert_eq!(crate::settings::load().runtime_environments.len(), 1);

        // 固定 Proton 失效但目录仍完好的环境可以仅移除登记.
        fixture.directory.write("Proton-A/proton", "updated");
        let forgotten = fixture.action("forget", Some(&a.id)).unwrap();
        assert!(forgotten.settings.runtime_environments.is_empty());
        assert_eq!(fixture.remembered("cn"), (None, None));
        let persisted = crate::settings::load();
        assert!(persisted.runtime_environments.is_empty());
        assert!(persisted.current().active_environment("cn").is_none());
        assert!(persisted.current().stable_environment("cn").is_none());
        assert!(a.directory(&data_dir()).unwrap().is_dir());

        // 目录被删除的环境同样只能移除登记, 并清空该环境占用的槽位.
        let b = fixture
            .create("B", None, "Proton-B")
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();
        std::fs::remove_dir_all(b.directory(&data_dir()).unwrap()).unwrap();
        let forgotten = fixture.action("forget", Some(&b.id)).unwrap();
        assert!(forgotten.settings.runtime_environments.is_empty());
        assert_eq!(fixture.remembered("cn"), (None, None));
        assert!(crate::settings::load().runtime_environments.is_empty());
    }

    #[test]
    fn audit_reports_environment_health_and_unregistered_directories() {
        let fixture = Fixture::new();
        let a = fixture.first();
        let b = fixture
            .create("B", Some(&a.id), "Proton-B")
            .unwrap()
            .settings
            .runtime_environments
            .last()
            .unwrap()
            .clone();
        fixture.directory.write("Proton-B/proton", "updated");
        // 其他游戏的环境不出现在当前游戏的审计里.
        {
            let mut foreign = named_environment("其他游戏");
            foreign.id = "c".repeat(32);
            foreign.game = GameId::from("hsr");
            fixture
                .state
                .settings
                .lock()
                .unwrap()
                .runtime_environments
                .push(foreign);
        }
        let orphan_environment = "a".repeat(32);
        let orphan_directory = data_dir().join("environments").join(&orphan_environment);
        std::fs::create_dir_all(orphan_directory.join("prefix/drive_c")).unwrap();
        std::fs::write(
            orphan_directory.join(".oddity-environment"),
            &orphan_environment,
        )
        .unwrap();
        let orphan_logs = "b".repeat(32);
        std::fs::create_dir_all(data_dir().join("logs/environments").join(&orphan_logs)).unwrap();

        let audit = fixture.audit().unwrap();
        assert_eq!(audit.environments.len(), 2);
        let first = audit
            .environments
            .iter()
            .find(|entry| entry.id == a.id)
            .unwrap();
        assert_eq!(first.name, "A");
        assert_eq!(first.state, "ready");
        assert_eq!(first.install_root, a.install_root.display().to_string());
        assert_eq!(
            first.install_root_current,
            fixture.directory.join("install").display().to_string()
        );
        let second = audit
            .environments
            .iter()
            .find(|entry| entry.id == b.id)
            .unwrap();
        assert_eq!(second.state, "proton-invalid");

        assert_eq!(audit.orphans.len(), 2);
        assert_eq!(audit.orphans[0].kind.as_str(), "environment");
        assert_eq!(audit.orphans[0].name, orphan_environment);
        assert!(audit.orphans[0].identity_valid);
        assert!(audit.orphans[0].prefix_valid);
        assert_eq!(audit.orphans[1].kind.as_str(), "logs");
        assert_eq!(audit.orphans[1].name, orphan_logs);
        let value = serde_json::to_value(&audit).unwrap();
        assert_eq!(value["environments"][0]["state"], "ready");
        assert!(value["environments"][0]["installRoot"].is_string());
        assert!(value["environments"][0]["installRootCurrent"].is_string());
        assert_eq!(value["orphans"][0]["identityValid"], true);

        // 当前安装目录变化后, 审计反映新的目标目录.
        let moved = fixture.directory.join("moved");
        fixture.use_install_root(&moved);
        let audit = fixture.audit().unwrap();
        assert!(audit
            .environments
            .iter()
            .all(|entry| entry.install_root_current == moved.display().to_string()));
        assert_ne!(
            audit.environments[0].install_root,
            moved.display().to_string()
        );

        // 目录缺失与身份不符分别映射到对应状态.
        std::fs::remove_dir_all(b.directory(&data_dir()).unwrap()).unwrap();
        let audit = fixture.audit().unwrap();
        assert_eq!(
            audit
                .environments
                .iter()
                .find(|entry| entry.id == b.id)
                .unwrap()
                .state,
            "directory-missing"
        );
        let directory = a.directory(&data_dir()).unwrap();
        std::fs::write(directory.join(".oddity-environment"), "other").unwrap();
        let audit = fixture.audit().unwrap();
        assert_eq!(
            audit
                .environments
                .iter()
                .find(|entry| entry.id == a.id)
                .unwrap()
                .state,
            "identity-invalid"
        );
    }

    #[test]
    fn import_registers_an_existing_environment_directory() {
        let fixture = Fixture::new();
        let id = "ab".repeat(16);
        fixture.importable(&id);
        let bootstrap = fixture.import(&id, " 导入环境 ", "Proton-A").unwrap();
        let imported = bootstrap.settings.runtime_environments[0].clone();
        assert_eq!(imported.id, id);
        assert_eq!(imported.name, "导入环境");
        assert_eq!(imported.game, GameId::from("nte"));
        assert_eq!(
            imported.install_root,
            fixture.directory.join("install").canonicalize().unwrap()
        );
        // 创建时间取导入时刻, 不依赖目录的 ctime.
        assert!((crate::now_millis() / 1000).saturating_sub(imported.created_at) <= 5);
        // 缺少游戏链接时补建到当前安装根目录.
        assert_eq!(
            std::fs::read_link(
                data_dir()
                    .join("environments")
                    .join(&id)
                    .join("prefix")
                    .join(crate::testing::PREFIX_LINK)
            )
            .unwrap(),
            imported.install_root
        );
        assert_eq!(
            bootstrap.settings.current().active_environment("cn"),
            Some(id.as_str())
        );
        assert_eq!(
            bootstrap.settings.current().stable_environment("cn"),
            Some(id.as_str())
        );
        let persisted = crate::settings::load();
        assert_eq!(persisted.runtime_environments.len(), 1);
        assert_eq!(persisted.runtime_environments[0].id, id);

        // 重复登记与重复名称都会被拒绝.
        let error = fixture.import(&id, "再次导入", "Proton-A").unwrap_err();
        assert!(error.contains("已登记"), "{error}");
        let duplicate_name = "cd".repeat(16);
        fixture.importable(&duplicate_name);
        let error = fixture
            .import(&duplicate_name, "导入环境", "Proton-A")
            .unwrap_err();
        assert!(error.contains("运行环境名称「导入环境」已存在"), "{error}");

        // 身份文件与 prefix 不完整时不能导入.
        let invalid_identity = "ef".repeat(16);
        let directory = data_dir().join("environments").join(&invalid_identity);
        std::fs::create_dir_all(directory.join("prefix/drive_c")).unwrap();
        std::fs::write(directory.join(".oddity-environment"), "other").unwrap();
        let error = fixture
            .import(&invalid_identity, "身份不符", "Proton-A")
            .unwrap_err();
        assert!(error.contains("身份文件不匹配"), "{error}");

        let broken_prefix = "12".repeat(16);
        let directory = data_dir().join("environments").join(&broken_prefix);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(".oddity-environment"), &broken_prefix).unwrap();
        let error = fixture
            .import(&broken_prefix, "prefix 不完整", "Proton-A")
            .unwrap_err();
        assert!(error.contains("prefix 不完整"), "{error}");

        let missing = "34".repeat(16);
        let error = fixture
            .import(&missing, "目录不存在", "Proton-A")
            .unwrap_err();
        assert!(error.contains("目录不存在"), "{error}");
        let error = fixture
            .import("invalid", "无效 ID", "Proton-A")
            .unwrap_err();
        assert!(error.contains("运行环境 ID 无效"), "{error}");

        // 当前渠道没有完整安装时不能导入, 也不修改设置.
        let valid = "56".repeat(16);
        fixture.importable(&valid);
        fixture.use_install_root(&fixture.directory.join("missing"));
        let error = fixture.import(&valid, "缺少安装", "Proton-A").unwrap_err();
        assert!(error.contains("未找到官方启动器或游戏文件"), "{error}");
        assert_eq!(crate::settings::load().runtime_environments.len(), 1);

        // Proton 目录无效时拒绝导入.
        fixture.use_install_root(&fixture.directory.join("install"));
        let error = fixture
            .import(&valid, "缺少 Proton", "missing")
            .unwrap_err();
        assert!(error.contains("Proton"), "{error}");
        assert_eq!(crate::settings::load().runtime_environments.len(), 1);
    }

    #[test]
    fn import_rebinds_a_prefix_that_still_points_to_an_old_install_root() {
        let fixture = Fixture::new();
        let id = "ba".repeat(16);
        let old_root = fixture.directory.join("old-install");
        std::fs::create_dir_all(&old_root).unwrap();
        let directory = fixture.importable(&id);
        std::fs::create_dir_all(directory.join("prefix/drive_c/Program Files (x86)")).unwrap();
        std::os::unix::fs::symlink(
            &old_root,
            directory.join("prefix").join(crate::testing::PREFIX_LINK),
        )
        .unwrap();
        let old_wine = oddity_platform::wine::wine_path(&old_root).replace('\\', "\\\\");
        std::fs::write(
            directory.join("prefix/system.reg"),
            format!("\"ImagePath\"=str(2):\"{old_wine}\\\\x.sys\""),
        )
        .unwrap();

        let imported = fixture.import(&id, "旧安装环境", "Proton-B").unwrap();
        let environment = &imported.settings.runtime_environments[0];
        let current = fixture.directory.join("install").canonicalize().unwrap();
        assert_eq!(environment.install_root, current);
        assert_eq!(
            std::fs::read_link(directory.join("prefix").join(crate::testing::PREFIX_LINK)).unwrap(),
            current
        );
        let current_wine = oddity_platform::wine::wine_path(&current).replace('\\', "\\\\");
        let registry = std::fs::read_to_string(directory.join("prefix/system.reg")).unwrap();
        assert!(registry.contains(&current_wine), "{registry}");

        // 链接与记录都已指向当前目录时再次导入到其他名字不会改动 prefix.
        let second = "bc".repeat(16);
        let copy = fixture.importable(&second);
        std::fs::create_dir_all(copy.join("prefix/drive_c/Program Files (x86)")).unwrap();
        std::os::unix::fs::symlink(
            &current,
            copy.join("prefix").join(crate::testing::PREFIX_LINK),
        )
        .unwrap();
        let imported = fixture.import(&second, "已对齐环境", "Proton-A").unwrap();
        assert_eq!(
            imported.settings.runtime_environments[1].install_root,
            current
        );
        assert_eq!(
            std::fs::read_link(copy.join("prefix").join(crate::testing::PREFIX_LINK)).unwrap(),
            current
        );
    }

    #[test]
    fn orphan_cleanup_rejects_registered_targets_and_removes_unregistered_logs() {
        let fixture = Fixture::new();
        let logs = "99".repeat(16);
        let directory = data_dir().join("logs/environments").join(&logs);
        std::fs::create_dir_all(&directory).unwrap();
        let error = environment_orphan_delete(&fixture.state, "other", &logs).unwrap_err();
        assert!(error.contains("未登记目录类型无效"), "{error}");
        environment_orphan_delete(&fixture.state, "logs", &logs).unwrap();
        assert!(!directory.exists());

        let a = fixture.first();
        let error = environment_orphan_delete(&fixture.state, "environment", &a.id).unwrap_err();
        assert!(error.contains("已登记"), "{error}");
        assert!(a.directory(&data_dir()).unwrap().is_dir());
    }
}
