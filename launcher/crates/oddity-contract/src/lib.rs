//! 命令与事件的唯一声明源.
//!
//! `launcher/tools/contract-gen` 从这里生成 Electron 白名单、渲染层 TypeScript
//! 类型、浏览器演示的命令键和 `launcher/CONTRACTS.md`. 改契约后运行
//! `cargo run -p contract-gen` 并提交生成物.

/// 命令的调用方.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// 渲染层可以通过 Electron 白名单调用.
    Renderer,
    /// 只由 Electron 主进程调用, 不进渲染层白名单.
    Host,
    /// 只在 Electron 窗口层处理, 不经过后端.
    Window,
}

/// 业务类型所在的 TypeScript 模块.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeModule {
    Launcher,
    Gamepad,
    FilePicker,
}

/// 生成 TypeScript 时引用的已有业务类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedType {
    pub name: &'static str,
    pub module: TypeModule,
}

/// 对象类型里的一个字段.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TsField {
    pub name: &'static str,
    pub ty: TsType,
}

/// 参数与返回值的 TypeScript 形状.
///
/// 只描述生成物需要的部分, 不把业务类型 schema 化.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsType {
    /// 引用已有业务类型.
    Named(NamedType),
    /// 字符串字面量, 例如 `"cn"`.
    Literal(&'static str),
    /// 基础类型: `string`、`number`、`boolean`、`null`、`undefined`、`void`.
    Primitive(&'static str),
    /// 内联对象类型.
    Object(&'static [TsField]),
    /// 数组类型.
    Array(&'static TsType),
    /// 联合类型.
    Union(&'static [TsType]),
    /// 没有参数, 生成 `Record<string, never>`.
    NoArgs,
}

impl TsField {
    pub const fn new(name: &'static str, ty: TsType) -> Self {
        Self { name, ty }
    }
}

impl TsType {
    pub const STRING: Self = Self::Primitive("string");
    pub const NUMBER: Self = Self::Primitive("number");
    pub const BOOLEAN: Self = Self::Primitive("boolean");
    pub const NULL: Self = Self::Primitive("null");
    pub const UNDEFINED: Self = Self::Primitive("undefined");
    pub const VOID: Self = Self::Primitive("void");
    pub const NO_ARGS: Self = Self::NoArgs;

    pub const fn named(name: &'static str) -> Self {
        Self::Named(NamedType {
            name,
            module: TypeModule::Launcher,
        })
    }

    pub const fn gamepad(name: &'static str) -> Self {
        Self::Named(NamedType {
            name,
            module: TypeModule::Gamepad,
        })
    }

    pub const fn file_picker(name: &'static str) -> Self {
        Self::Named(NamedType {
            name,
            module: TypeModule::FilePicker,
        })
    }

    pub const fn literal(value: &'static str) -> Self {
        Self::Literal(value)
    }

    pub const fn union(types: &'static [TsType]) -> Self {
        Self::Union(types)
    }

    pub const fn array(inner: &'static TsType) -> Self {
        Self::Array(inner)
    }
}

/// 一条宿主命令.
#[derive(Debug, Clone, Copy)]
pub struct Command {
    /// 命令 id, 同时是请求里的 `command` 字段.
    pub id: &'static str,
    /// 谁能调用.
    pub audience: Audience,
    /// 是否要求与当前游戏一致; 发现类命令不需要.
    pub current_game: bool,
    /// 由包内命令组处理的命令所属包 id; 宿主实现或通用命令为 None.
    pub owner: Option<&'static str>,
    /// 参数类型, 用于生成 TypeScript 与目录.
    pub args: TsType,
    /// 返回类型, 用于生成 TypeScript 与目录.
    pub result: TsType,
}

/// 《异环》包 id.
pub const PACKAGE_NTE: &str = "nte";

/// 测试游戏包 id.
pub const PACKAGE_FIXTURE: &str = "fixture";

/// 已注册的游戏包 id; `Command.owner` 只能取这里列出的值.
pub const PACKAGE_IDS: &[&str] = &[PACKAGE_NTE, PACKAGE_FIXTURE];

/// 一个宿主事件.
///
/// 只声明身份和归属; payload 形状由发射方与渲染层各自手写, 契约不做 schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    /// 事件名, 同时是 Electron 转发给渲染层的事件名.
    pub id: &'static str,
    /// 发射领域, 写进目录供人查阅.
    pub domain: &'static str,
}

/// 全部命令; 顺序同时决定生成物里的顺序.
pub const COMMANDS: &[Command] = &[
    Command {
        id: "runtime_setup_info",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("RuntimeSetupInfo"),
    },
    Command {
        id: "runtime_setup_plan",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("allUsers", TsType::BOOLEAN),
            TsField::new("proton", TsType::union(&[TsType::STRING, TsType::NULL])),
            TsField::new("createEnvironment", TsType::BOOLEAN),
        ]),
        result: TsType::named("RuntimeSetupPlan"),
    },
    Command {
        id: "runtime_setup_apply",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
    Command {
        id: "runtime_setup_status",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("RuntimeSetupStatus"),
    },
    Command {
        id: "runtime_setup_dismiss",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
    Command {
        id: "runtime_setup_environment",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("Bootstrap"),
    },
    Command {
        id: "game_accounts",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::array(&TsType::named("GameAccount")),
    },
    Command {
        id: "game_send_code",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new("phone", TsType::STRING)]),
        result: TsType::STRING,
    },
    Command {
        id: "game_login",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[
            TsField::new("phone", TsType::STRING),
            TsField::new("secret", TsType::STRING),
            TsField::new("password", TsType::BOOLEAN),
            TsField::new("deviceId", TsType::STRING),
        ]),
        result: TsType::named("GameAccount"),
    },
    Command {
        id: "game_account_remove",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new("accountId", TsType::STRING)]),
        result: TsType::NULL,
    },
    Command {
        id: "browse_files",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new(
            "path",
            TsType::union(&[TsType::STRING, TsType::NULL]),
        )]),
        result: TsType::file_picker("FileListing"),
    },
    Command {
        id: "gamepad_poll",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::gamepad("GamepadSnapshot"),
    },
    Command {
        id: "gamepad_release_idle",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
    Command {
        id: "gamepad_feedback",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("deviceId", TsType::NUMBER),
            TsField::new(
                "effect",
                TsType::union(&[TsType::gamepad("Feedback"), TsType::literal("stop")]),
            ),
        ]),
        result: TsType::NULL,
    },
    Command {
        id: "gamepad_profile_validate",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new("profile", TsType::gamepad("Profile"))]),
        result: TsType::gamepad("Profile"),
    },
    Command {
        id: "gamepad_profile_load",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new("path", TsType::STRING)]),
        result: TsType::gamepad("Profile"),
    },
    Command {
        id: "gamepad_profile_save",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("path", TsType::STRING),
            TsField::new("profile", TsType::gamepad("Profile")),
        ]),
        result: TsType::VOID,
    },
    Command {
        id: "attendance_status",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_refresh_game_records",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_login",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[
            TsField::new("phone", TsType::STRING),
            TsField::new("password", TsType::STRING),
            TsField::new("rememberPassword", TsType::BOOLEAN),
        ]),
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_send_code",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new("phone", TsType::STRING)]),
        result: TsType::STRING,
    },
    Command {
        id: "attendance_login_sms",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[
            TsField::new("phone", TsType::STRING),
            TsField::new("code", TsType::STRING),
            TsField::new("deviceId", TsType::STRING),
        ]),
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_configure",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new(
            "preferences",
            TsType::named("AttendancePreferences"),
        )]),
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_run",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_logout",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new("accountId", TsType::STRING)]),
        result: TsType::named("AttendanceStatus"),
    },
    Command {
        id: "attendance_keyring_check",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::VOID,
    },
    Command {
        id: "launcher_background",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("LauncherBackgroundMedia"),
    },
    Command {
        id: "bootstrap",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("Bootstrap"),
    },
    Command {
        id: "integrity_acknowledge",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new("notice", TsType::named("IntegrityNotice"))]),
        result: TsType::NULL,
    },
    Command {
        id: "check_update",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new("full", TsType::BOOLEAN)]),
        result: TsType::named("CheckResult"),
    },
    Command {
        id: "fetch_bulletins",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new(
            "distribution",
            TsType::union(&[TsType::literal("cn"), TsType::literal("global")]),
        )]),
        result: TsType::named("BulletinFeed"),
    },
    Command {
        id: "update_settings_section",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("owner", TsType::STRING),
            TsField::new("patch", TsType::named("SettingsPatch")),
        ]),
        result: TsType::named("SettingsSection"),
    },
    Command {
        id: "environment_create",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("distribution", TsType::STRING),
            TsField::new("name", TsType::STRING),
            TsField::new("protonPath", TsType::STRING),
            TsField::new("sourceId", TsType::union(&[TsType::STRING, TsType::NULL])),
        ]),
        result: TsType::named("Bootstrap"),
    },
    Command {
        id: "environment_action",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new(
                "action",
                TsType::union(&[
                    TsType::literal("switch"),
                    TsType::literal("keep"),
                    TsType::literal("discard"),
                ]),
            ),
            TsField::new("id", TsType::union(&[TsType::STRING, TsType::NULL])),
        ]),
        result: TsType::named("Bootstrap"),
    },
    Command {
        id: "migration_plan",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("distribution", TsType::STRING),
            TsField::new("target", TsType::STRING),
        ]),
        result: TsType::named("MigrationPlan"),
    },
    Command {
        id: "migration_start",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("distribution", TsType::STRING),
            TsField::new("target", TsType::STRING),
        ]),
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "migration_status",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "migration_cancel",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "migration_cleanup",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "migration_abandon",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "migration_dismiss",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "migration_retry",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("MigrationStatus"),
    },
    Command {
        id: "prepare_log_export",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("LogPreview"),
    },
    Command {
        id: "save_log_export",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("id", TsType::STRING),
            TsField::new("destination", TsType::STRING),
        ]),
        result: TsType::STRING,
    },
    Command {
        id: "discard_log_export",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new("id", TsType::STRING)]),
        result: TsType::VOID,
    },
    Command {
        id: "environment_snapshot_overview",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("SnapshotOverview"),
    },
    Command {
        id: "environment_snapshot_compare",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[TsField::new(
            "target",
            TsType::union(&[TsType::literal("baseline"), TsType::literal("reference")]),
        )]),
        result: TsType::named("SnapshotComparison"),
    },
    Command {
        id: "environment_snapshot_save_reference",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("label", TsType::union(&[TsType::STRING, TsType::NULL])),
            TsField::new("attemptId", TsType::union(&[TsType::STRING, TsType::NULL])),
        ]),
        result: TsType::named("SnapshotReference"),
    },
    Command {
        id: "environment_snapshot_delete_reference",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::VOID,
    },
    Command {
        id: "environment_snapshot_clear_history",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::VOID,
    },
    Command {
        id: "open_directory",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new("path", TsType::STRING)]),
        result: TsType::NULL,
    },
    Command {
        id: "open_url",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[TsField::new("url", TsType::STRING)]),
        result: TsType::NULL,
    },
    Command {
        id: "launch_official",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("LaunchResult"),
    },
    Command {
        id: "cancel_launch",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("GameStatus"),
    },
    Command {
        id: "game_status",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("GameStatus"),
    },
    Command {
        id: "take_launch_error",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::union(&[TsType::STRING, TsType::NULL]),
    },
    Command {
        id: "performance_overview",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("PerformanceOverview"),
    },
    Command {
        id: "performance_recording",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[TsField::new("enabled", TsType::BOOLEAN)]),
        result: TsType::NULL,
    },
    Command {
        id: "performance_pin",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("id", TsType::STRING),
            TsField::new("pinned", TsType::BOOLEAN),
        ]),
        result: TsType::NULL,
    },
    Command {
        id: "performance_delete",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[TsField::new("id", TsType::STRING)]),
        result: TsType::NULL,
    },
    Command {
        id: "performance_report",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("a", TsType::STRING),
            TsField::new("b", TsType::STRING),
        ]),
        result: TsType::STRING,
    },
    Command {
        id: "performance_export",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[
            TsField::new("a", TsType::STRING),
            TsField::new("b", TsType::STRING),
            TsField::new("destination", TsType::STRING),
        ]),
        result: TsType::STRING,
    },
    Command {
        id: "playtime_history",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("PlaytimeHistory"),
    },
    Command {
        id: "playtime_export_poster",
        audience: Audience::Renderer,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("destination", TsType::STRING),
            TsField::new("data", TsType::STRING),
        ]),
        result: TsType::STRING,
    },
    Command {
        id: "audio_status",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::NO_ARGS,
        result: TsType::named("AudioStatus"),
    },
    Command {
        id: "game_mode_status",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("GameModeStatus"),
    },
    Command {
        id: "game_mode_gpu_info",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("GpuInfo"),
    },
    Command {
        id: "game_mode_tuning_info",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("GameModeTuning"),
    },
    Command {
        id: "game_mode_tuning_apply",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[TsField::new("config", TsType::named("GameModeTuning"))]),
        result: TsType::named("GameModeTuning"),
    },
    Command {
        id: "game_mode_gpu_apply",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::Object(&[TsField::new("config", TsType::named("GpuConfig"))]),
        result: TsType::named("GpuInfo"),
    },
    Command {
        id: "game_resource_catalog",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("ResourceCatalog"),
    },
    Command {
        id: "game_resource_remove",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[
            TsField::new("tag", TsType::STRING),
            TsField::new("preload", TsType::BOOLEAN),
        ]),
        result: TsType::named("ResourceCatalog"),
    },
    Command {
        id: "game_resource_download",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[TsField::new("tag", TsType::STRING)]),
        result: TsType::named("DownloadStatus"),
    },
    Command {
        id: "game_preload_delete",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("ResourceCatalog"),
    },
    Command {
        id: "game_download_status",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("DownloadStatus"),
    },
    Command {
        id: "game_download_start",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::Object(&[
            TsField::new(
                "mode",
                TsType::union(&[
                    TsType::literal("check"),
                    TsType::literal("apply"),
                    TsType::literal("launcher"),
                    TsType::literal("preload"),
                ]),
            ),
            TsField::new("full", TsType::BOOLEAN),
        ]),
        result: TsType::named("DownloadStatus"),
    },
    Command {
        id: "game_download_cancel",
        audience: Audience::Renderer,
        current_game: true,
        owner: Some("nte"),
        args: TsType::UNDEFINED,
        result: TsType::named("DownloadStatus"),
    },
    Command {
        id: "close_game",
        audience: Audience::Renderer,
        current_game: true,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::named("GameStatus"),
    },
    // fixture 包的命令: 只用于装配与契约测试, 不面向玩家.
    Command {
        id: "fixture_echo",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("fixture"),
        args: TsType::Object(&[TsField::new("message", TsType::STRING)]),
        result: TsType::Object(&[
            TsField::new("message", TsType::STRING),
            TsField::new("length", TsType::NUMBER),
        ]),
    },
    Command {
        id: "fixture_info",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("fixture"),
        args: TsType::UNDEFINED,
        result: TsType::Object(&[
            TsField::new("game", TsType::literal("fixture")),
            TsField::new("name", TsType::STRING),
            TsField::new("capabilities", TsType::array(&TsType::STRING)),
        ]),
    },
    Command {
        id: "fixture_fail",
        audience: Audience::Renderer,
        current_game: false,
        owner: Some("fixture"),
        args: TsType::Object(&[TsField::new("message", TsType::STRING)]),
        result: TsType::NULL,
    },
    Command {
        id: "host_state",
        audience: Audience::Host,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("focused", TsType::BOOLEAN),
            TsField::new("visible", TsType::BOOLEAN),
        ]),
        result: TsType::NULL,
    },
    Command {
        id: "external_launch",
        audience: Audience::Host,
        current_game: false,
        owner: None,
        args: TsType::Object(&[
            TsField::new("argv", TsType::array(&TsType::STRING)),
            TsField::new("cold", TsType::BOOLEAN),
        ]),
        result: TsType::NULL,
    },
    Command {
        id: "win_minimize",
        audience: Audience::Window,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
    Command {
        id: "win_toggle_maximize",
        audience: Audience::Window,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
    Command {
        id: "win_close",
        audience: Audience::Window,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
    Command {
        id: "win_hide",
        audience: Audience::Window,
        current_game: false,
        owner: None,
        args: TsType::UNDEFINED,
        result: TsType::NULL,
    },
];

/// 全部事件; 顺序同时决定生成物里的顺序.
///
/// 发射调用由本 crate 的扫描测试核对: 每个发射的 id 必须在这里声明, 每条声明
/// 至少被发射一次. 发射点数量不写进契约.
pub const EVENTS: &[Event] = &[
    Event {
        id: "game-download-state",
        domain: "下载",
    },
    Event {
        id: "game-state",
        domain: "游戏进程",
    },
    Event {
        id: "host-exit",
        domain: "宿主窗口",
    },
    Event {
        id: "host-hide",
        domain: "宿主窗口",
    },
    Event {
        id: "host-show",
        domain: "宿主窗口",
    },
    Event {
        id: "integrity-state",
        domain: "完整性",
    },
    Event {
        id: "migration-state",
        domain: "环境迁移",
    },
    Event {
        id: "performance-updated",
        domain: "性能记录",
    },
    Event {
        id: "playtime-started",
        domain: "游玩时长",
    },
    Event {
        id: "runtime-setup-state",
        domain: "运行时准备",
    },
    Event {
        id: "settings-changed",
        domain: "设置",
    },
    Event {
        id: "task-progress",
        domain: "后台任务",
    },
];

/// 按 id 查找命令.
pub fn command(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

/// 按 id 查找事件.
pub fn event(id: &str) -> Option<&'static Event> {
    EVENTS.iter().find(|event| event.id == id)
}

/// 命令是否需要与当前游戏一致.
pub fn requires_current_game(id: &str) -> bool {
    command(id).is_some_and(|command| command.current_game)
}

#[cfg(test)]
mod audit;
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn is_snake_case(value: &str) -> bool {
        !value.is_empty()
            && !value.starts_with('_')
            && !value.ends_with('_')
            && !value.contains("__")
            && value
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    }

    #[test]
    fn command_ids_are_unique_and_snake_case() {
        let mut seen = HashSet::new();
        for command in COMMANDS {
            assert!(
                is_snake_case(command.id),
                "命令 id 不是 snake_case: {}",
                command.id
            );
            assert!(seen.insert(command.id), "命令 id 重复: {}", command.id);
        }
    }

    #[test]
    fn window_commands_never_require_a_game() {
        for command in COMMANDS {
            if command.audience == Audience::Window {
                assert!(
                    !command.current_game,
                    "窗口命令不应要求当前游戏: {}",
                    command.id
                );
            }
        }
    }

    #[test]
    fn command_owners_are_registered_packages() {
        for command in COMMANDS {
            let Some(owner) = command.owner else {
                continue;
            };
            assert!(
                command.audience != Audience::Window,
                "窗口命令 {} 不应有包归属 {owner}",
                command.id
            );
            assert!(
                PACKAGE_IDS.contains(&owner),
                "命令 {} 的包归属 {owner} 未注册",
                command.id
            );
        }
    }

    #[test]
    fn event_ids_are_unique_and_have_a_domain() {
        let mut seen = HashSet::new();
        for event in EVENTS {
            assert!(
                is_snake_case(&event.id.replace('-', "_")),
                "事件 id 格式不对: {}",
                event.id
            );
            assert!(!event.domain.is_empty(), "事件缺少发射领域: {}", event.id);
            assert!(seen.insert(event.id), "事件 id 重复: {}", event.id);
        }
    }

    #[test]
    fn lookups_only_find_declared_entries() {
        assert_eq!(
            command("bootstrap").map(|command| command.current_game),
            Some(false)
        );
        assert_eq!(
            command("game_status").map(|command| command.current_game),
            Some(true)
        );
        assert!(command("not_a_command").is_none());
        assert!(!requires_current_game("not_a_command"));
        assert!(requires_current_game("game_status"));
        assert!(event("game-download-state").is_some());
        assert!(event("not-an-event").is_none());
    }
}
