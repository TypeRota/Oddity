use crate::{
    Capability, CapabilityDescriptor, CapabilityRequirement, ChannelDescriptor, CommandGroup,
    GamePackage, HeadlessCli, InstallLocator, LaunchProvider, Necessity, PackageDescriptor,
    PackageGate, PackageRuntime, Scope, SettingsSchema,
};
use oddity_contract::{Audience, COMMANDS};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// 装配校验发现的问题.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationIssue {
    /// 契约命令既没有宿主处理, 也没有命令组声明.
    Unclaimed { command: &'static str },
    /// 同一 (game, command) 被多个组声明; game=None 表示通用组之间的冲突.
    /// 宿主 arm 与组的冲突也是这条, sources 以 "host" 开头.
    Duplicate {
        command: &'static str,
        game: Option<&'static str>,
        sources: Vec<&'static str>,
    },
    /// 命令组声明了契约外或不由后端处理的命令.
    InvalidClaim {
        command: &'static str,
        group: &'static str,
    },
    /// 组绑定了未装配的游戏 id.
    UnknownGroupGame {
        group: &'static str,
        game: &'static str,
    },
    /// 两个包声明了同一条无头 CLI 命令.
    DuplicateHeadlessCommand {
        command: &'static str,
        sources: Vec<&'static str>,
    },
    /// 两个包使用同一 id; 否则 install_locator/launch_provider/能力 sheet 与无头 CLI 会被静默覆盖.
    DuplicatePackage { id: &'static str },
    /// 契约命令的包归属为空或不是已注册包 id.
    InvalidOwner {
        command: &'static str,
        owner: &'static str,
    },
    /// 窗口命令标注了包归属; 窗口命令由窗口层处理, 不属于任何包.
    WindowOwner {
        command: &'static str,
        owner: &'static str,
    },
    /// 命令组认领了与自身游戏不一致的命令.
    OwnerMismatch {
        command: &'static str,
        group: &'static str,
        game: Option<&'static str>,
        owner: Option<&'static str>,
    },
    /// 游戏包的渠道声明非法.
    Channel { game: &'static str, reason: String },
    /// 游戏包的设置 section schema 非法.
    Settings { owner: &'static str, reason: String },
}

impl fmt::Display for ValidationIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unclaimed { command } => {
                write!(formatter, "命令 {command} 既没有宿主处理, 也没有命令组声明")
            }
            Self::Duplicate {
                command,
                game,
                sources,
            } => match (sources.first(), game) {
                (Some(&"host"), _) => write!(
                    formatter,
                    "命令 {command} 同时由宿主与命令组 {} 声明",
                    sources[1..].join(", ")
                ),
                (_, Some(game)) => write!(
                    formatter,
                    "命令 {command} 在游戏 {game} 下被多个命令组声明: {}",
                    sources.join(", ")
                ),
                (_, None) => write!(
                    formatter,
                    "命令 {command} 在通用命令组下被多个命令组声明: {}",
                    sources.join(", ")
                ),
            },
            Self::InvalidClaim { command, group } => {
                write!(formatter, "命令组 {group} 声明了契约外命令 {command}")
            }
            Self::UnknownGroupGame { group, game } => {
                write!(formatter, "命令组 {group} 绑定了未装配的游戏 {game}")
            }
            Self::DuplicateHeadlessCommand { command, sources } => {
                write!(
                    formatter,
                    "无头命令 {command} 被多个包声明: {}",
                    sources.join(", ")
                )
            }
            Self::DuplicatePackage { id } => {
                write!(formatter, "游戏包 id 重复: {id}")
            }
            Self::InvalidOwner { command, owner: "" } => {
                write!(formatter, "命令 {command} 的包归属为空")
            }
            Self::InvalidOwner { command, owner } => {
                write!(
                    formatter,
                    "命令 {command} 的包归属 {owner} 不是已注册的游戏包"
                )
            }
            Self::WindowOwner { command, owner } => {
                write!(formatter, "窗口命令 {command} 不应标注包归属 {owner}")
            }
            Self::OwnerMismatch {
                command,
                group,
                game,
                owner,
            } => {
                let owner = owner.unwrap_or("无");
                match game {
                    Some(game) => write!(
                        formatter,
                        "命令 {command} 的包归属是 {owner}, 命令组 {group} 属于游戏 {game}"
                    ),
                    None => write!(
                        formatter,
                        "命令 {command} 的包归属是 {owner}, 通用命令组 {group} 不能认领"
                    ),
                }
            }
            Self::Channel { game, reason } => {
                write!(formatter, "游戏 {game} 的渠道声明无效: {reason}")
            }
            Self::Settings { owner, reason } => {
                write!(formatter, "设置 section {owner} 的声明无效: {reason}")
            }
        }
    }
}

/// 能力装配校验发现的问题.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityIssue {
    InvalidId {
        capability: &'static str,
    },
    /// 同一游戏的能力 sheet 里出现两个同 id provider.
    DuplicateProvider {
        game: &'static str,
        capability: &'static str,
    },
    DuplicateRequirement {
        game: &'static str,
        consumer: &'static str,
        capability: &'static str,
    },
    MissingProvider {
        game: &'static str,
        consumer: &'static str,
        capability: &'static str,
    },
    RevisionMismatch {
        game: &'static str,
        consumer: &'static str,
        capability: &'static str,
        required: u16,
        provided: u16,
    },
    ScopeMismatch {
        game: &'static str,
        consumer: &'static str,
        capability: &'static str,
        required: Scope,
        provided: Scope,
    },
}

impl fmt::Display for CapabilityIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { capability } => {
                write!(formatter, "能力 {capability} 不符合 oddity.<family>.<name>")
            }
            Self::DuplicateProvider { game, capability } => {
                write!(formatter, "能力 {capability} 在游戏 {game} 有多个提供者")
            }
            Self::DuplicateRequirement {
                consumer,
                capability,
                ..
            } => write!(formatter, "能力 {capability} 被 {consumer} 重复依赖"),
            Self::MissingProvider {
                game,
                consumer,
                capability,
            } => write!(
                formatter,
                "游戏 {game} 缺少必需能力 {capability}, 由 {consumer} 要求"
            ),
            Self::RevisionMismatch {
                game,
                consumer,
                capability,
                required,
                provided,
            } => write!(
                formatter,
                "能力 {capability} 的 revision 不一致: {consumer} 要求 {required}, 游戏 {game} 提供 {provided}"
            ),
            Self::ScopeMismatch {
                consumer,
                capability,
                required,
                provided,
                ..
            } => write!(
                formatter,
                "能力 {capability} 的 scope 不覆盖 {consumer} 的要求: 要求 {required}, 提供 {provided}"
            ),
        }
    }
}

/// 一个命令的归属: 游戏专属组或通用组.
#[derive(Debug, Clone, Copy)]
struct GroupRef {
    game: Option<&'static str>,
    index: usize,
}

/// 宿主收集的命令组.
pub struct Registry {
    packages: Vec<PackageDescriptor>,
    groups: Vec<CommandGroup>,
    /// command -> 该命令的所有者列表; 每个命令至多 1 个通用组 + 每个游戏至多 1 个组.
    owners: HashMap<&'static str, Vec<GroupRef>>,
    install_locators: HashMap<&'static str, &'static dyn InstallLocator>,
    launch_providers: HashMap<&'static str, &'static dyn LaunchProvider>,
    headless_clis: HashMap<&'static str, &'static dyn HeadlessCli>,
    /// 按装配顺序保存, 保证门禁与关闭顺序确定.
    runtimes: Vec<(&'static str, &'static dyn PackageRuntime)>,
    /// game -> 能力 sheet; 键是包 id, 与 descriptor.id 一致.
    capabilities: HashMap<&'static str, &'static [Capability]>,
    /// game -> 包级能力依赖.
    requires: HashMap<&'static str, &'static [CapabilityRequirement]>,
    /// game -> 包声明的发行渠道.
    channels: HashMap<&'static str, &'static [ChannelDescriptor]>,
    /// game -> 包声明的设置 section schema.
    settings: HashMap<&'static str, &'static [SettingsSchema]>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            groups: Vec::new(),
            owners: HashMap::new(),
            install_locators: HashMap::new(),
            launch_providers: HashMap::new(),
            headless_clis: HashMap::new(),
            runtimes: Vec::new(),
            capabilities: HashMap::new(),
            requires: HashMap::new(),
            channels: HashMap::new(),
            settings: HashMap::new(),
        }
    }

    /// 加载一个游戏包: 记录其声明, 命令组, 安装域, 启动实现与能力声明.
    pub fn install(&mut self, package: &dyn GamePackage) {
        let descriptor = package.descriptor();
        package.register(self);
        if let Some(locator) = package.install_locator() {
            self.install_locators.insert(descriptor.id, locator);
        }
        if let Some(provider) = package.launch_provider() {
            self.launch_providers.insert(descriptor.id, provider);
        }
        if let Some(cli) = package.headless_cli() {
            self.headless_clis.insert(descriptor.id, cli);
        }
        if let Some(runtime) = package.runtime() {
            self.runtimes.push((descriptor.id, runtime));
        }
        self.capabilities
            .insert(descriptor.id, package.capabilities());
        self.requires.insert(descriptor.id, package.requires());
        self.channels.insert(descriptor.id, package.channels());
        self.settings.insert(descriptor.id, package.settings());
        self.packages.push(descriptor);
    }

    /// 登记一个命令组; 重复的 (game, command) 留给 Self::validate 报告.
    pub fn register(&mut self, group: CommandGroup) {
        let index = self.groups.len();
        let game = group.game();
        for &command in group.commands() {
            self.owners
                .entry(command)
                .or_default()
                .push(GroupRef { game, index });
        }
        self.groups.push(group);
    }

    /// 已加载游戏包的声明.
    pub fn packages(&self) -> &[PackageDescriptor] {
        &self.packages
    }

    /// 已登记的命令组.
    pub fn groups(&self) -> &[CommandGroup] {
        &self.groups
    }

    /// 可作为当前游戏选择的包 id: 已装载且提供安装域.
    ///
    /// 只有安装域能决定"装在哪里"; 没有安装域的包不进入设置与外部入口.
    pub fn selectable_games(&self) -> Vec<&'static str> {
        self.packages
            .iter()
            .filter(|package| self.install_locators.contains_key(package.id))
            .map(|package| package.id)
            .collect()
    }

    /// 按游戏解析: (game, command) 精确匹配优先, 其次通用组.
    pub fn group_for(&self, game: &str, command: &str) -> Option<&CommandGroup> {
        let owners = self.owners.get(command)?;
        owners
            .iter()
            .find(|owner| owner.game == Some(game))
            .or_else(|| owners.iter().find(|owner| owner.game.is_none()))
            .map(|owner| &self.groups[owner.index])
    }

    /// 该命令是否被任何游戏或通用组认领.
    ///
    /// 用于区分"未知命令"与"本游戏不提供".
    pub fn claims(&self, command: &str) -> bool {
        self.owners.contains_key(command)
    }

    /// 按 id 查找已装配的游戏包.
    pub fn package(&self, id: &str) -> Option<&PackageDescriptor> {
        self.packages.iter().find(|package| package.id == id)
    }

    /// 某游戏的能力 sheet; 未装配该游戏时 None.
    pub fn capabilities(&self, game: &str) -> Option<&'static [Capability]> {
        self.capabilities.get(game).copied()
    }

    /// 某游戏的包级能力依赖; 未装配该游戏时 None.
    pub fn requires(&self, game: &str) -> Option<&'static [CapabilityRequirement]> {
        self.requires.get(game).copied()
    }

    /// 某游戏声明的发行渠道; 未装配该游戏时 None.
    pub fn channels(&self, game: &str) -> Option<&'static [ChannelDescriptor]> {
        self.channels.get(game).copied()
    }

    /// 已装配包声明的设置 section schema, 顺序与装配表一致.
    pub fn settings_schemas(&self) -> impl Iterator<Item = &'static SettingsSchema> + '_ {
        self.packages.iter().flat_map(move |package| {
            let schemas: &'static [SettingsSchema] =
                self.settings.get(package.id).copied().unwrap_or(&[]);
            schemas.iter()
        })
    }

    /// 按 owner 查找设置 section schema.
    pub fn settings_schema(&self, owner: &str) -> Option<&'static SettingsSchema> {
        self.settings_schemas().find(|schema| schema.owner == owner)
    }

    /// 按游戏 id 查找安装域.
    ///
    /// 没有加载该游戏的包, 或包不提供安装位置知识时返回 None.
    pub fn install_locator(&self, game: &str) -> Option<&'static dyn InstallLocator> {
        self.install_locators.get(game).copied()
    }

    /// 按游戏 id 查找启动实现.
    ///
    /// 没有加载该游戏的包, 或包不提供启动实现时返回 None.
    pub fn launch_provider(&self, game: &str) -> Option<&'static dyn LaunchProvider> {
        self.launch_providers.get(game).copied()
    }

    /// 已装配包的无头 CLI, 顺序与装配表一致.
    pub fn headless_clis(
        &self,
    ) -> impl Iterator<Item = (&'static str, &'static dyn HeadlessCli)> + '_ {
        self.packages.iter().filter_map(|package| {
            self.headless_clis
                .get(package.id)
                .map(|cli| (package.id, *cli))
        })
    }

    /// 按游戏 id 查找无头 CLI.
    ///
    /// 没有加载该游戏的包, 或包不提供 CLI 入口时返回 None.
    pub fn headless_cli(&self, game: &str) -> Option<&'static dyn HeadlessCli> {
        self.headless_clis.get(game).copied()
    }

    /// 已装配包的长驻域, 顺序与装配表一致.
    pub fn runtimes(&self) -> impl Iterator<Item = &'static dyn PackageRuntime> + '_ {
        self.runtimes.iter().map(|(_, runtime)| *runtime)
    }

    /// 按游戏 id 查找长驻域.
    ///
    /// 没有加载该游戏的包, 或包没有长驻任务时返回 None.
    pub fn runtime(&self, game: &str) -> Option<&'static dyn PackageRuntime> {
        self.runtimes
            .iter()
            .find(|(id, _)| *id == game)
            .map(|(_, runtime)| *runtime)
    }

    /// 任一长驻域正在写盘.
    pub fn busy(&self) -> bool {
        self.runtimes.iter().any(|(_, runtime)| runtime.busy())
    }

    /// 任一长驻域会阻塞设置保存与启动.
    pub fn blocks_changes(&self) -> bool {
        self.runtimes
            .iter()
            .any(|(_, runtime)| runtime.blocks_changes())
    }

    /// 取得所有长驻域的变更门, 顺序与装配表一致.
    ///
    /// 调用方持有返回的门闩做跨域互斥; 任一包取不到门闩时整体失败,
    /// 已取得的门闩随错误一起释放.
    pub fn gates(&self) -> Result<Vec<Box<dyn PackageGate>>, String> {
        self.runtimes
            .iter()
            .map(|(_, runtime)| runtime.gate())
            .collect()
    }

    /// 校验命令归属与包身份.
    ///
    /// host_commands 是宿主 dispatch 覆盖的命令 id. 契约命令的 owner 必须是
    /// 已注册包 id, 窗口命令不得有 owner; 每条非窗口命令要么由宿主处理, 要么
    /// 由 owner 相同的命令组认领. 所属包未装配时命令随包缺席, 但 owner 拼错
    /// 或为空不是"未装配"; 同一 (game, command) 只能有一个组.
    pub fn validate(&self, host_commands: &[&str]) -> Result<(), Vec<ValidationIssue>> {
        let host: HashSet<&str> = host_commands.iter().copied().collect();
        let mut issues = Vec::new();

        for id in self.duplicate_package_ids() {
            issues.push(ValidationIssue::DuplicatePackage { id });
        }

        for game in self.game_order() {
            issues.extend(self.channel_issues(game));
            issues.extend(self.settings_issues(game));
        }

        issues.extend(COMMANDS.iter().filter_map(owner_issue));

        for (command, sources) in self.headless_command_claims() {
            if sources.len() > 1 {
                issues.push(ValidationIssue::DuplicateHeadlessCommand { command, sources });
            }
        }

        let mut order: Vec<&'static str> = Vec::new();
        let mut claims: HashMap<&'static str, Vec<(Option<&'static str>, &'static str)>> =
            HashMap::new();

        for group in &self.groups {
            if let Some(game) = group.game() {
                if self.package(game).is_none() {
                    issues.push(ValidationIssue::UnknownGroupGame {
                        group: group.domain(),
                        game,
                    });
                }
            }
            for &command in group.commands() {
                match oddity_contract::command(command) {
                    Some(declared) if declared.audience != Audience::Window => {
                        if group.game() != declared.owner {
                            issues.push(ValidationIssue::OwnerMismatch {
                                command,
                                group: group.domain(),
                                game: group.game(),
                                owner: declared.owner,
                            });
                        }
                    }
                    _ => issues.push(ValidationIssue::InvalidClaim {
                        command,
                        group: group.domain(),
                    }),
                }
                if !claims.contains_key(command) {
                    order.push(command);
                }
                claims
                    .entry(command)
                    .or_default()
                    .push((group.game(), group.domain()));
            }
        }

        for command in order {
            let claim = &claims[command];
            if host.contains(command) {
                let mut sources = vec!["host"];
                sources.extend(claim.iter().map(|(_, domain)| *domain));
                issues.push(ValidationIssue::Duplicate {
                    command,
                    game: None,
                    sources,
                });
                continue;
            }
            let mut owners: Vec<Option<&'static str>> = Vec::new();
            for (owner, _) in claim {
                if !owners.contains(owner) {
                    owners.push(*owner);
                }
            }
            for owner in owners {
                let sources: Vec<&'static str> = claim
                    .iter()
                    .filter(|(candidate, _)| *candidate == owner)
                    .map(|(_, domain)| *domain)
                    .collect();
                if sources.len() > 1 {
                    issues.push(ValidationIssue::Duplicate {
                        command,
                        game: owner,
                        sources,
                    });
                }
            }
        }

        for declared in COMMANDS {
            if declared.audience == Audience::Window {
                continue;
            }
            if host.contains(declared.id) || claims.contains_key(declared.id) {
                continue;
            }
            // 包没装配就不存在认领者, 命令随包一起缺席是预期状态.
            if let Some(owner) = declared.owner {
                if oddity_contract::PACKAGE_IDS.contains(&owner) && self.package(owner).is_none() {
                    continue;
                }
            }
            issues.push(ValidationIssue::Unclaimed {
                command: declared.id,
            });
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// 校验能力声明.
    ///
    /// 解析域固定在同一游戏: required 必须有 provider, optional 缺失放行;
    /// revision 精确相等, provider.scope 必须覆盖 requirement.scope.
    pub fn validate_capabilities(&self) -> Result<(), Vec<CapabilityIssue>> {
        let mut issues = Vec::new();
        for game in self.game_order() {
            let providers = self.capabilities.get(game).copied().unwrap_or(&[]);
            let requirements = self.requires.get(game).copied().unwrap_or(&[]);

            let mut sheet: HashMap<&'static str, CapabilityDescriptor> = HashMap::new();
            for capability in providers {
                let descriptor = capability.descriptor;
                if !descriptor.id.is_well_formed() {
                    issues.push(CapabilityIssue::InvalidId {
                        capability: descriptor.id.0,
                    });
                }
                if sheet.contains_key(descriptor.id.0) {
                    issues.push(CapabilityIssue::DuplicateProvider {
                        game,
                        capability: descriptor.id.0,
                    });
                } else {
                    sheet.insert(descriptor.id.0, descriptor);
                }
            }

            let mut seen: HashSet<&'static str> = HashSet::new();
            for requirement in requirements {
                let id = requirement.id.0;
                if !requirement.id.is_well_formed() {
                    issues.push(CapabilityIssue::InvalidId { capability: id });
                }
                if !seen.insert(id) {
                    issues.push(CapabilityIssue::DuplicateRequirement {
                        game,
                        consumer: game,
                        capability: id,
                    });
                }
                match sheet.get(id) {
                    Some(provided) => {
                        if provided.revision != requirement.revision {
                            issues.push(CapabilityIssue::RevisionMismatch {
                                game,
                                consumer: game,
                                capability: id,
                                required: requirement.revision.0,
                                provided: provided.revision.0,
                            });
                        }
                        if !provided.scope.contains(requirement.scope) {
                            issues.push(CapabilityIssue::ScopeMismatch {
                                game,
                                consumer: game,
                                capability: id,
                                required: requirement.scope,
                                provided: provided.scope,
                            });
                        }
                    }
                    None if requirement.necessity == Necessity::Required => {
                        issues.push(CapabilityIssue::MissingProvider {
                            game,
                            consumer: game,
                            capability: id,
                        });
                    }
                    None => {}
                }
            }
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// 一个游戏的渠道声明问题.
    fn channel_issues(&self, game: &'static str) -> Vec<ValidationIssue> {
        let channels = self.channels.get(game).copied().unwrap_or(&[]);
        let mut issues = Vec::new();
        if channels.is_empty() {
            if self.install_locators.contains_key(game) {
                issues.push(ValidationIssue::Channel {
                    game,
                    reason: "有安装域但没有声明渠道".into(),
                });
            }
            return issues;
        }

        let mut ids = HashSet::new();
        let mut defaults = 0;
        for channel in channels {
            if !valid_identifier(channel.id) {
                issues.push(ValidationIssue::Channel {
                    game,
                    reason: if channel.id.is_empty() {
                        "渠道 id 为空".into()
                    } else {
                        format!(
                            "渠道 id {} 不符合 [a-z][a-z0-9_]* 且长度不超过 32",
                            channel.id
                        )
                    },
                });
            } else if !ids.insert(channel.id) {
                issues.push(ValidationIssue::Channel {
                    game,
                    reason: format!("渠道 id 重复: {}", channel.id),
                });
            }
            if channel.label.trim().is_empty() {
                issues.push(ValidationIssue::Channel {
                    game,
                    reason: format!("渠道 {} 的展示名为空", channel.id),
                });
            } else if channel.label.chars().count() > 32 {
                issues.push(ValidationIssue::Channel {
                    game,
                    reason: format!("渠道 {} 的展示名超过 32 个字符", channel.id),
                });
            }
            if !valid_identifier(channel.install_root_key) {
                issues.push(ValidationIssue::Channel {
                    game,
                    reason: format!(
                        "渠道 {} 的安装根键 {} 不符合 [a-z][a-z0-9_]* 且长度不超过 32",
                        channel.id, channel.install_root_key
                    ),
                });
            }
            if channel.is_default {
                defaults += 1;
            }
        }
        if defaults != 1 {
            issues.push(ValidationIssue::Channel {
                game,
                reason: format!("默认渠道数量为 {defaults}, 应恰好为 1"),
            });
        }
        issues
    }

    /// 一个游戏的设置 section schema 问题.
    fn settings_issues(&self, game: &'static str) -> Vec<ValidationIssue> {
        let schemas = self.settings.get(game).copied().unwrap_or(&[]);
        let expected_owner = format!("game:{game}");
        let mut issues = Vec::new();
        let mut owners = HashSet::new();
        for schema in schemas {
            if schema.owner != expected_owner {
                issues.push(ValidationIssue::Settings {
                    owner: schema.owner,
                    reason: format!("包 {game} 只能声明 owner {expected_owner}"),
                });
            }
            if !owners.insert(schema.owner) {
                issues.push(ValidationIssue::Settings {
                    owner: schema.owner,
                    reason: "owner 重复声明".into(),
                });
            }
            for reason in schema.definition_issues() {
                issues.push(ValidationIssue::Settings {
                    owner: schema.owner,
                    reason,
                });
            }
        }
        issues
    }

    /// 无头命令的声明者, 保持装配顺序.
    fn headless_command_claims(&self) -> Vec<(&'static str, Vec<&'static str>)> {
        let mut claims: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
        for (game, cli) in self.headless_clis() {
            for command in cli.commands() {
                match claims.iter_mut().find(|(name, _)| *name == command.name) {
                    Some((_, sources)) => {
                        if !sources.contains(&game) {
                            sources.push(game);
                        }
                    }
                    None => claims.push((command.name, vec![game])),
                }
            }
        }
        claims
    }

    /// 声明设置 section 的游戏 id, 保持装配顺序且去重.
    pub fn settings_games(&self) -> Vec<&'static str> {
        self.packages
            .iter()
            .filter(|package| {
                self.settings
                    .get(package.id)
                    .is_some_and(|schemas| !schemas.is_empty())
            })
            .map(|package| package.id)
            .collect()
    }

    /// 已装配包的 id, 保持装配顺序且去重.
    fn game_order(&self) -> Vec<&'static str> {
        let mut seen = HashSet::new();
        self.packages
            .iter()
            .map(|package| package.id)
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// 重复的包 id, 保持装配顺序.
    fn duplicate_package_ids(&self) -> Vec<&'static str> {
        let mut seen = HashSet::new();
        let mut duplicates = Vec::new();
        for package in &self.packages {
            if !seen.insert(package.id) && !duplicates.contains(&package.id) {
                duplicates.push(package.id);
            }
        }
        duplicates
    }
}

/// 渠道 id 与安装根键的字符集.
fn valid_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= 32
        && first.is_ascii_lowercase()
        && rest
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// 契约命令的 owner 合法性; 只检查声明本身, 与包是否装配无关.
pub(crate) fn owner_issue(declared: &oddity_contract::Command) -> Option<ValidationIssue> {
    let owner = declared.owner?;
    if declared.audience == Audience::Window {
        return Some(ValidationIssue::WindowOwner {
            command: declared.id,
            owner,
        });
    }
    if !oddity_contract::PACKAGE_IDS.contains(&owner) {
        return Some(ValidationIssue::InvalidOwner {
            command: declared.id,
            owner,
        });
    }
    None
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
