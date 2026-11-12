//! 静态装配表: 宿主在这里选择要加载的游戏包.
//!
//! 新增游戏包在 [`PACKAGES`] 加一行, 并在 `Cargo.toml` 引入包 crate 与同名
//! feature; 宿主其余代码不变.

use oddity_runtime::{CapabilityIssue, GamePackage, Registry, ValidationIssue};

/// 装配表; 每个包由同名 feature 决定是否编入.
pub(crate) const PACKAGES: &[&dyn GamePackage] = &[
    #[cfg(feature = "game-nte")]
    &oddity_game_nte::NTE_PACKAGE,
    #[cfg(feature = "game-fixture")]
    &oddity_game_fixture::FIXTURE_PACKAGE,
];

/// 按装配表构建注册表.
pub fn build() -> Registry {
    let mut registry = Registry::new();
    for &package in PACKAGES {
        registry.install(package);
    }
    registry
}

/// 启动时校验命令归属.
///
/// 契约里每条后端命令要么由宿主 dispatch 处理, 要么由恰好一个命令组声明;
/// 包未装配时其命令随包一起缺席.
pub(crate) fn check(registry: &Registry) -> Result<(), String> {
    validate(registry)
}

/// 启动时校验能力声明.
///
/// 缺 required provider, revision 不等, scope 不覆盖或同游戏重复 provider
/// 都会让启动失败.
pub(crate) fn check_capabilities(registry: &Registry) -> Result<(), String> {
    registry
        .validate_capabilities()
        .map_err(|issues| format_capability_issues(&issues))
}

/// 用宿主 dispatch 的实际覆盖范围校验注册表.
fn validate(registry: &Registry) -> Result<(), String> {
    let host = oddity_host::protocol::dispatch_command_ids();
    let host: Vec<&str> = host.iter().map(String::as_str).collect();
    registry
        .validate(&host)
        .map_err(|issues| format_issues(&issues))
}

fn format_issues(issues: &[ValidationIssue]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_capability_issues(issues: &[CapabilityIssue]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(all(test, feature = "game-nte", feature = "game-fixture"))]
mod tests {
    use super::*;

    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use oddity_host::service::{Service, ServiceConfig};
    use oddity_runtime::{CommandDispatch, CommandGroup, DispatchFuture, PackageDescriptor};
    use serde_json::{json, Value};

    fn discard<'a>(
        _: &'a dyn oddity_runtime::HostContext,
        _: &'a str,
        payload: Value,
    ) -> DispatchFuture<'a> {
        Box::pin(async move { Ok(payload) })
    }

    /// 装配表构造的服务; 事件出口留在宿主里, 用例不读.
    fn test_service() -> oddity_host::service::ServiceHandle {
        test_service_with_output().0
    }

    /// 同上, 保留事件接收端与临时目录.
    fn test_service_with_output() -> (
        oddity_host::service::ServiceHandle,
        mpsc::Receiver<Value>,
        TempDir,
    ) {
        let (events, receiver) = mpsc::sync_channel(256);
        let (input, jobs) = mpsc::sync_channel(256);
        drop(jobs);
        let directory = TempDir::new("assembly");
        let data_dir = directory.path().to_path_buf();
        let service = Service::new(ServiceConfig {
            settings: oddity_host::settings::Settings::default(),
            data_dir,
            registry: Arc::new(build()),
            events,
            input,
        });
        (service, receiver, directory)
    }

    /// 析构时自动删除的临时目录.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "oddity-assembly-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("创建临时目录");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn join(&self, rel: impl AsRef<std::path::Path>) -> std::path::PathBuf {
            self.0.join(rel)
        }

        fn write(&self, rel: impl AsRef<std::path::Path>, contents: impl AsRef<[u8]>) {
            let full = self.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("创建父目录");
            }
            std::fs::write(&full, contents).expect("写入文件");
        }

        fn write_text(&self, rel: impl AsRef<std::path::Path>, contents: &str) {
            self.write(rel, contents.as_bytes());
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 独占进程环境变量并在用例结束时恢复.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
            Self {
                saved: Vec::new(),
                _lock: lock,
            }
        }

        fn set(&mut self, key: &'static str, value: impl AsRef<std::ffi::OsStr>) {
            self.saved.push((key, std::env::var_os(key)));
            std::env::set_var(key, value);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn assembly_covers_each_backend_command_exactly_once() {
        check(&build()).expect("装配表与宿主 dispatch 的命令覆盖不一致");
    }

    #[test]
    fn only_packages_with_an_install_domain_are_selectable_games() {
        let registry = build();
        let selectable: Vec<&str> = registry
            .packages()
            .iter()
            .filter(|package| registry.install_locator(package.id).is_some())
            .map(|package| package.id)
            .collect();
        assert_eq!(selectable, ["nte"]);
        assert!(registry.install_locator("fixture").is_none());
    }

    #[test]
    fn assembly_loads_every_package_and_its_groups() {
        let registry = build();
        let ids: Vec<&str> = registry
            .packages()
            .iter()
            .map(|package| package.id)
            .collect();
        assert_eq!(ids, ["nte", "fixture"]);

        let groups: Vec<(Option<&str>, &str, Vec<&str>)> = registry
            .groups()
            .iter()
            .map(|group| (group.game(), group.domain(), group.commands().to_vec()))
            .collect();
        assert_eq!(
            groups,
            [
                (Some("nte"), "bulletin", vec!["fetch_bulletins"]),
                (Some("nte"), "background", vec!["launcher_background"]),
                (
                    Some("nte"),
                    "attendance",
                    vec![
                        "attendance_status",
                        "attendance_refresh_game_records",
                        "attendance_login",
                        "attendance_send_code",
                        "attendance_login_sms",
                        "attendance_configure",
                        "attendance_run",
                        "attendance_logout",
                        "attendance_keyring_check",
                    ]
                ),
                (
                    Some("nte"),
                    "auth",
                    vec![
                        "game_accounts",
                        "game_send_code",
                        "game_login",
                        "game_account_remove",
                    ]
                ),
                (Some("nte"), "update", vec!["check_update"]),
                (
                    Some("nte"),
                    "download",
                    vec![
                        "game_resource_catalog",
                        "game_resource_remove",
                        "game_resource_download",
                        "game_preload_delete",
                        "game_download_status",
                        "game_download_start",
                        "game_download_cancel",
                    ]
                ),
                (
                    Some("fixture"),
                    "fixture",
                    vec!["fixture_echo", "fixture_fail"]
                ),
                (Some("fixture"), "fixture-info", vec!["fixture_info"]),
            ]
        );
        assert!(registry.group_for("nte", "fetch_bulletins").is_some());
        assert!(registry.group_for("nte", "launcher_background").is_some());
        assert!(registry.group_for("nte", "attendance_run").is_some());
        assert!(registry.group_for("nte", "game_accounts").is_some());
        assert!(registry.group_for("fixture", "fixture_echo").is_some());
        assert!(registry.group_for("nte", "fixture_echo").is_none());
    }

    #[test]
    fn assembly_registers_the_install_domain_of_each_package() {
        let registry = build();
        assert!(registry.install_locator("nte").is_some());
        assert!(registry.install_locator("fixture").is_none());
        assert!(registry.install_locator("hsr").is_none());
    }

    #[test]
    fn attendance_commands_route_through_the_shared_registry() {
        let app = test_service();
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app.clone(),
            "attendance_login".into(),
            None,
            json!(false),
        ))
        .unwrap_err();
        assert!(error.is_string(), "{error}");
        assert!(
            error.as_str().unwrap().contains("expected struct Args"),
            "{error}"
        );

        // 需要当前游戏的命令在进入命令组前先校验游戏; 当前游戏可以是没有安装域的包.
        app.state.settings.lock().unwrap().active_game =
            oddity_host::identity::GameId::from("fixture");
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            "attendance_run".into(),
            Some("nte".into()),
            json!({}),
        ))
        .unwrap_err();
        assert!(error.as_str().unwrap().contains("当前游戏是"), "{error}");
    }

    #[test]
    fn game_account_commands_route_through_the_shared_registry() {
        let app = test_service();
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app.clone(),
            "game_send_code".into(),
            None,
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("游戏账号请求参数无效"));

        // 需要当前游戏的命令在进入命令组前先校验游戏; 当前游戏可以是没有安装域的包.
        app.state.settings.lock().unwrap().active_game =
            oddity_host::identity::GameId::from("fixture");
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            "game_accounts".into(),
            Some("nte".into()),
            json!({}),
        ))
        .unwrap_err();
        assert!(error.as_str().unwrap().contains("当前游戏是"), "{error}");
    }

    #[test]
    fn fixture_commands_route_with_active_game_fixture() {
        let app = test_service();

        // activeGame 不属于 fixture 时, 组命令不可达.
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app.clone(),
            "fixture_echo".into(),
            None,
            json!({"message": "你好"}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("《异环》不提供命令 fixture_echo."));

        app.state.settings.lock().unwrap().active_game =
            oddity_host::identity::GameId::from("fixture");
        let echoed = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app.clone(),
            "fixture_echo".into(),
            None,
            json!({"message": "你好"}),
        ))
        .expect("fixture 回显应经 dispatch 路由到包内");
        assert_eq!(echoed, json!({"message": "你好", "length": 2}));

        let info = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app.clone(),
            "fixture_info".into(),
            None,
            json!({}),
        ))
        .expect("fixture 查询应经 dispatch 路由到包内");
        assert_eq!(info["game"], json!("fixture"));
        assert_eq!(info["name"], json!("测试游戏"));

        let failure = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            "fixture_fail".into(),
            None,
            json!({"message": "注入失败"}),
        ))
        .expect_err("fixture 失败注入应原样返回错误载荷");
        assert_eq!(
            failure,
            json!({"code": "fixture_failure", "message": "注入失败"})
        );
    }

    /// 测试包: 故意声明 fixture 包已经拥有的命令, 模拟两个包重复声明.
    struct DuplicateFixturePackage;

    impl GamePackage for DuplicateFixturePackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "fixture-copy",
                name: "重复 fixture",
            }
        }

        fn register(&self, registry: &mut Registry) {
            registry.register(CommandGroup::for_game(
                "fixture",
                "fixture-copy",
                &["fixture_echo"],
                discard as CommandDispatch,
            ));
        }
    }

    #[test]
    fn duplicate_command_in_the_same_game_is_rejected() {
        let mut registry = build();
        registry.install(&DuplicateFixturePackage);
        assert_eq!(registry.packages().len(), 3);
        assert_eq!(registry.packages()[2].id, "fixture-copy");

        let error = validate(&registry).expect_err("同一游戏内两个组声明同一命令时应装配失败");
        assert!(
            error.contains(
                "命令 fixture_echo 在游戏 fixture 下被多个命令组声明: fixture, fixture-copy"
            ),
            "{error}"
        );
    }

    /// 第二个游戏声明同名命令组: 契约 owner 固定为 nte, 跨包认领应装配失败.
    struct SecondGamePackage;

    impl GamePackage for SecondGamePackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "hsr",
                name: "测试第二游戏",
            }
        }

        fn register(&self, registry: &mut Registry) {
            registry.register(CommandGroup::for_game(
                "hsr",
                "update",
                &["check_update"],
                discard as CommandDispatch,
            ));
        }
    }

    #[test]
    fn a_group_cannot_claim_another_packages_command() {
        let mut registry = build();
        registry.install(&SecondGamePackage);
        let error = validate(&registry).expect_err("跨包认领应装配失败");
        assert_eq!(
            error,
            "命令 check_update 的包归属是 nte, 命令组 update 属于游戏 hsr"
        );
    }

    #[test]
    fn a_group_cannot_take_over_a_host_command() {
        let mut registry = build();
        registry.register(CommandGroup::for_game(
            "nte",
            "test",
            &["game_status"],
            discard as CommandDispatch,
        ));
        let error = validate(&registry).unwrap_err();
        assert!(error.contains("game_status"), "{error}");
        assert!(
            error.contains("命令 game_status 同时由宿主与命令组 test 声明"),
            "{error}"
        );
    }

    static BROKEN_REQUIREMENT: &[oddity_runtime::CapabilityRequirement] =
        &[oddity_runtime::CapabilityRequirement {
            id: oddity_runtime::CapabilityId("oddity.test.broken"),
            revision: oddity_runtime::CapabilityRevision(1),
            necessity: oddity_runtime::Necessity::Required,
            scope: oddity_runtime::Scope::GAME,
        }];

    /// 只声明一个缺失依赖的测试包.
    struct BrokenCapabilityPackage;

    impl GamePackage for BrokenCapabilityPackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "broken",
                name: "能力缺失包",
            }
        }

        fn register(&self, _registry: &mut Registry) {}

        fn requires(&self) -> &'static [oddity_runtime::CapabilityRequirement] {
            BROKEN_REQUIREMENT
        }
    }

    #[test]
    fn capability_check_runs_at_assembly() {
        let mut registry = build();
        registry.install(&BrokenCapabilityPackage);
        let error = check_capabilities(&registry).expect_err("缺失 required provider 应装配失败");
        assert!(error.contains("oddity.test.broken"), "{error}");
        assert!(error.contains("缺少必需能力"), "{error}");
    }

    #[test]
    fn production_sheet_passes_capability_check() {
        check_capabilities(&build()).expect("生产装配的能力声明应通过校验");
    }

    #[test]
    fn issues_are_joined_for_startup_output() {
        let issues = [
            ValidationIssue::Unclaimed {
                command: "game_status",
            },
            ValidationIssue::InvalidClaim {
                command: "not_a_command",
                group: "test",
            },
        ];
        let message = format_issues(&issues);
        assert!(message.contains("game_status"), "{message}");
        assert!(message.contains("not_a_command"), "{message}");
    }

    #[test]
    fn attendance_run_uses_the_host_executor_through_the_registry() {
        let config = TempDir::new("assembly-run");
        let mut env = EnvGuard::new();
        env.set("XDG_CONFIG_HOME", config.path());

        let app = test_service();
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            "attendance_run".into(),
            None,
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error["kind"], json!("auth"), "{error}");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("请先登录塔吉多账号"),
            "{error}"
        );
    }

    #[test]
    fn group_commands_declare_their_package_owner() {
        for group in build().groups() {
            let game = group.game().expect("生产装配只有游戏专属命令组");
            for command in group.commands() {
                let declared = oddity_contract::command(command).expect("命令组声明了契约外命令");
                assert_eq!(
                    declared.owner,
                    Some(game),
                    "命令 {command} 的包归属与命令组不一致"
                );
            }
        }
    }

    #[test]
    fn group_commands_route_after_host_arms() {
        let (app, _events, _dir) = test_service_with_output();
        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app.clone(),
            "fetch_bulletins".into(),
            None,
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("missing field `distribution`"));

        let error = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            "not_a_command".into(),
            None,
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("无法识别的宿主命令"));
    }

    #[test]
    fn resolves_the_official_background_through_the_package_group() {
        let directory = TempDir::new("background");
        let mut env = EnvGuard::new();
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        directory.write_text(
            "install/NTELauncher/Config/Config.ini",
            "[General]\nGameID=1289\n[VERSION]\nVersion=1.0\n",
        );
        directory.write("install/NTELauncher/ResFilesM/1289/bgimgs/bg.mp4", b"video");
        directory.write(
            "install/NTELauncher/ResFilesM/1289/bgimgs/bg_0.png",
            b"image",
        );
        directory.write_text(
            "install/NTELauncher/ResFilesM/1289/bgimgs/config.json",
            r#"{"video":"bg.mp4","imgs":[{"file":"bg_0.png"}]}"#,
        );

        // 命令组从宿主能力拿启动器根目录, 素材解析发生在包内.
        let (app, _events, _guard) = test_service_with_output();
        {
            let mut settings = app.state.settings.lock().unwrap();
            settings.current_mut().install_roots.insert(
                "cn".into(),
                directory.join("install/NTELauncher").display().to_string(),
            );
        }
        let media = oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            "launcher_background".into(),
            None,
            json!({}),
        ))
        .expect("背景命令应经命令组返回素材");

        for (key, relative) in [
            ("video", "install/NTELauncher/ResFilesM/1289/bgimgs/bg.mp4"),
            (
                "image",
                "install/NTELauncher/ResFilesM/1289/bgimgs/bg_0.png",
            ),
        ] {
            let expected = std::fs::canonicalize(directory.join(relative)).expect("规范化路径");
            assert_eq!(media[key], json!(expected.display().to_string()));
        }
    }
}

/// 当前装配表的冒烟: 默认装配与只装配部分包的构建都要通过启动校验.
#[cfg(test)]
mod package_set {
    use super::*;

    #[test]
    fn assembled_packages_pass_the_startup_check() {
        check(&build()).expect("当前装配表应通过启动校验");
    }
}

/// 空装配冒烟: 没有游戏包时宿主仍能构造, 查询返回空态, 包命令报未装配.
///
/// 与装配表无关, 所有 feature 组合都会跑.
#[cfg(test)]
mod empty_assembly {
    use super::*;

    use std::sync::mpsc;
    use std::sync::Arc;

    use oddity_host::service::{Service, ServiceConfig};
    use oddity_runtime::Registry;
    use serde_json::{json, Value};

    /// 析构时删除的临时目录.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "oddity-empty-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("创建临时目录");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 空注册表构造的服务; 事件出口留在宿主里, 用例不读.
    fn empty_service() -> (oddity_host::service::ServiceHandle, TempDir) {
        let (events, _receiver) = mpsc::sync_channel(64);
        let (input, jobs) = mpsc::sync_channel(64);
        drop(jobs);
        let directory = TempDir::new("assembly");
        let service = Service::new(ServiceConfig {
            settings: oddity_host::settings::Settings::default(),
            data_dir: directory.0.clone(),
            registry: Arc::new(Registry::new()),
            events,
            input,
        });
        (service, directory)
    }

    fn dispatch(
        app: oddity_host::service::ServiceHandle,
        command: &str,
        game: Option<&str>,
        args: Value,
    ) -> Result<Value, Value> {
        oddity_host::service::block_on(oddity_host::protocol::dispatch(
            app,
            command.into(),
            game.map(str::to_string),
            args,
        ))
    }

    #[test]
    fn empty_assembly_passes_the_startup_check() {
        check(&Registry::new()).expect("空装配表应通过启动校验");
        check_capabilities(&Registry::new()).expect("空装配表应通过能力校验");
    }

    #[test]
    fn empty_assembly_bootstrap_reports_the_empty_state() {
        let (app, _directory) = empty_service();
        let bootstrap = dispatch(app, "bootstrap", None, json!({})).expect("bootstrap");
        assert_eq!(bootstrap["ok"], json!(false), "{bootstrap}");
        assert_eq!(bootstrap["launcherRoot"], json!(""), "{bootstrap}");
        assert_eq!(bootstrap["gameVersion"], json!(""), "{bootstrap}");
        assert_eq!(bootstrap["gameExists"], json!(false), "{bootstrap}");
        let installations = bootstrap["runtimeInstallations"]
            .as_array()
            .expect("安装清单是数组");
        assert!(
            installations
                .iter()
                .all(|installation| installation["installed"] == json!(false)),
            "{bootstrap}"
        );
        assert!(bootstrap["error"].is_string(), "{bootstrap}");
    }

    #[test]
    fn empty_assembly_status_is_idle_and_package_commands_are_unavailable() {
        let (app, _directory) = empty_service();
        let status = dispatch(app.clone(), "game_status", None, json!({})).expect("game_status");
        assert_eq!(status["running"], json!(false), "{status}");

        // 契约里的包命令仍被识别, 错误按命令所属包归因.
        for (command, owner) in [("attendance_status", "nte"), ("fixture_info", "fixture")] {
            let error = dispatch(app.clone(), command, None, json!({}))
                .expect_err("空装配下包命令不应被处理");
            assert_eq!(
                error,
                json!(format!("游戏包 {owner} 未装配, 不处理命令 {command}.")),
                "{command}: {error}"
            );
        }
        let error =
            dispatch(app, "not_a_command", None, json!({})).expect_err("契约外命令不应被识别");
        assert_eq!(error, json!("无法识别的宿主命令"), "{error}");
    }

    #[test]
    fn empty_assembly_rejects_a_game_that_has_no_package() {
        let (app, _directory) = empty_service();
        let error = dispatch(app, "game_status", Some("genshin"), json!({}))
            .expect_err("未装配的游戏不能被选中");
        assert!(
            error
                .as_str()
                .unwrap_or_default()
                .contains("无法识别游戏: genshin"),
            "{error}"
        );
    }
}
