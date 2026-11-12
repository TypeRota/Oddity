use super::*;
use crate::bootstrap::{bootstrap_impl, CapabilityFact, CapabilityState};
use crate::channel::ChannelId;
use crate::identity::GameId;
use crate::install::{launcher_root as detect_launcher_root, selected_install_root};
use crate::paths::data_dir;
use crate::settings::Settings;
use oddity_runtime::{
    Availability, Capability, CapabilityContext, CapabilityDescriptor, CapabilityId,
    CapabilityRevision, GamePackage, PackageDescriptor, Registry, Scope,
};

/// 一个最小可用的安装目录树：<root>/install/NTELauncher/...
struct Install {
    dir: TempDir,
    launcher: PathBuf,
}

impl Install {
    fn new(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let launcher = dir.join("install/NTELauncher");
        std::fs::create_dir_all(&launcher).expect("create launcher dir");
        Self { dir, launcher }
    }

    fn launcher_config(&self, body: &str) {
        self.dir
            .write_text("install/NTELauncher/Config/Config.ini", body);
    }

    fn user_config(&self, body: &str) {
        self.dir
            .write_text("install/NTELauncher/UserData/Config/Config.ini", body);
    }

    fn game_exe(&self) {
        self.dir.write(
            "install/Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
            b"MZ",
        );
    }

    fn settings(&self) -> Settings {
        crate::tests::settings_with_game(ChannelId::new("cn"), Some(self.launcher.clone()), None)
    }
}

const PROBE_CONTEXT: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.probe.context"),
    revision: CapabilityRevision(2),
    scope: Scope::GAME,
};

const PROBE_OFFLINE: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.probe.offline"),
    revision: CapabilityRevision(1),
    scope: Scope::GAME,
};

fn context_availability(context: &CapabilityContext) -> Availability {
    if context.game == "probe" && context.distribution == "cn" {
        Availability::Available
    } else {
        Availability::Unsupported
    }
}

fn offline_availability(_: &CapabilityContext) -> Availability {
    Availability::Unavailable {
        code: "offline",
        message: "网络不可用",
    }
}

static PROBE_CAPABILITIES: &[Capability] = &[
    Capability::new(PROBE_CONTEXT, context_availability),
    Capability::new(PROBE_OFFLINE, offline_availability),
];

static PROBE_CHANNELS: &[oddity_runtime::ChannelDescriptor] = &[
    oddity_runtime::ChannelDescriptor {
        id: "cn",
        label: "国服",
        is_default: true,
        install_root_key: "cn",
    },
    oddity_runtime::ChannelDescriptor {
        id: "global",
        label: "国际服",
        is_default: false,
        install_root_key: "global",
    },
];

/// 只声明能力 sheet 的测试包.
struct CapabilityProbe;

impl GamePackage for CapabilityProbe {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "probe",
            name: "能力测试包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn capabilities(&self) -> &'static [Capability] {
        PROBE_CAPABILITIES
    }

    fn channels(&self) -> &'static [oddity_runtime::ChannelDescriptor] {
        PROBE_CHANNELS
    }
}

/// 隔离所有会影响安装目录探测的环境变量。
fn isolated_env(dir: &TempDir) -> EnvGuard {
    let mut env = EnvGuard::new();
    env.set("HOME", dir.join("home"));
    env.set("XDG_DATA_HOME", dir.join("data"));
    env.set("XDG_CONFIG_HOME", dir.join("config"));
    env.remove("WINEPREFIX");
    env.remove("PROTONPATH");
    env.remove("NTE_ROOT");
    env.remove("NTE_INSTALL");
    env.remove("STEAM_COMPAT_CLIENT_INSTALL_PATH");
    std::fs::create_dir_all(dir.join("home")).expect("create fake home");
    env
}

#[test]
fn bootstrap_reports_a_complete_install() {
    let install = Install::new("bootstrap");
    let mut env = isolated_env(&install.dir);
    install.launcher_config(
        "[General]\nLauncher=NTELauncher.exe\nPatcher=NTEUpdate.exe\nClient=NTEGame.exe\nGameID=1289\n\
         [VERSION]\nVersion=1.4.16\nBuild=20240101\n",
    );
    install.user_config("[Game]\nVersion_=1.4.20\n");
    install.game_exe();
    install
        .dir
        .write("install/NTELauncher/WPGameClientSDK_64.dll", b"MZ");

    install.dir.write("install/NTELauncher/NTEGame.exe", b"MZ");

    std::fs::create_dir_all(install.dir.join("prefix/drive_c")).expect("create prefix");
    let proton = fake_proton(&install.dir.join("tools"), "GE-Proton9-1");
    let bin = install.dir.join("bin");
    write_script(&bin, "umu-run", "#!/bin/sh\nexit 0\n");
    env.set("PATH", &bin);

    let profile = managed_environment(&proton, &install.dir.join("install"), ChannelId::new("cn"));
    let mut settings = install.settings();
    settings.runtime_environments = vec![profile.clone()];
    settings.current_mut().active_environment = Some(profile.id.clone());
    let bootstrap = bootstrap_impl(&crate::testing::packages(), &settings);

    assert!(bootstrap.ok);
    assert!(bootstrap.error.is_none());
    assert_eq!(
        bootstrap.install_root,
        install.dir.join("install").display().to_string()
    );
    assert_eq!(
        bootstrap.launcher_root,
        install.launcher.display().to_string()
    );
    assert_eq!(
        bootstrap.prefix,
        profile.prefix(&data_dir()).unwrap().display().to_string()
    );
    assert!(bootstrap.prefix_exists);
    assert_eq!(bootstrap.proton_name, "GE-Proton9-1");
    assert_eq!(bootstrap.proton_path, proton.display().to_string());
    assert!(bootstrap.umu_available);
    assert_eq!(bootstrap.launcher_version, "1.4.16");
    assert_eq!(bootstrap.launcher_build, "20240101");
    assert_eq!(bootstrap.game_version, "1.4.20");
    assert!(bootstrap.game_exists);
    assert!(bootstrap.game_exe.ends_with("HTGame.exe"));

    let labels: Vec<&str> = bootstrap
        .checks
        .iter()
        .map(|check| check.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec![
            "启动器配置",
            "游戏可执行文件",
            "umu-launcher",
            "Proton",
            "Wine prefix",
            "官方启动器"
        ]
    );
    assert!(
        bootstrap.checks.iter().all(|check| check.ok),
        "{:?}",
        bootstrap.checks
    );
    assert_eq!(
        bootstrap.checks[5].detail,
        install
            .dir
            .join("install/NTELauncher/NTEGame.exe")
            .display()
            .to_string()
    );
    assert_eq!(
        bootstrap.settings.current().install_roots,
        settings.current().install_roots
    );
    assert_eq!(bootstrap.app_version, env!("CARGO_PKG_VERSION"));
    assert!(bootstrap.os.starts_with(std::env::consts::OS));
}

#[test]
fn bootstrap_reports_an_unreadable_launcher_config() {
    let install = Install::new("bootstrap-broken");
    let _env = isolated_env(&install.dir);
    // 非法 UTF-8：文件存在但读不出配置
    install
        .dir
        .write("install/NTELauncher/Config/Config.ini", [0xff, 0xfe, 0x00]);

    let bootstrap = bootstrap_impl(&crate::testing::packages(), &install.settings());
    assert!(!bootstrap.ok);
    assert!(
        bootstrap
            .error
            .as_deref()
            .expect("error text")
            .contains("读取启动器配置失败"),
        "{:?}",
        bootstrap.error
    );
    assert!(bootstrap.launcher_root.ends_with("NTELauncher"));
    assert_eq!(bootstrap.launcher_version, "");
}

#[test]
fn bootstrap_reports_a_missing_install_when_nothing_is_found() {
    let dir = TempDir::new("bootstrap-missing");
    let _env = isolated_env(&dir);
    let settings = Settings::default();
    let bootstrap = bootstrap_impl(&crate::testing::packages(), &settings);

    // 机器上 /mnt、/media 里可能真的装着游戏，只有探测不到时才校验失败分支
    if detect_launcher_root(&crate::testing::packages(), &settings).is_none() {
        assert!(!bootstrap.ok);
        assert!(
            bootstrap
                .error
                .as_deref()
                .expect("error text")
                .contains("尚未找到"),
            "{:?}",
            bootstrap.error
        );
        assert_eq!(bootstrap.launcher_root, "");
        // 未找到安装时仍会给出所选区服的默认目录，供首次安装使用。
        assert_eq!(
            bootstrap.install_root,
            selected_install_root(&crate::testing::packages(), &settings)
                .display()
                .to_string()
        );
        assert!(!bootstrap.game_exists);
        assert_eq!(bootstrap.checks[0].detail, "未找到");
        assert_eq!(
            bootstrap.checks[1].detail,
            "未找到 HTGame.exe（注意 Win64 大小写）"
        );
        assert!(bootstrap.checks[3].detail.contains("未找到 Proton"));
        assert_eq!(bootstrap.checks[4].detail, bootstrap.prefix);
        assert_eq!(bootstrap.checks[5].detail, "未找到");
    }
}

#[test]
fn bootstrap_payload_keeps_its_field_shape() {
    let install = Install::new("bootstrap-shape");
    let mut env = isolated_env(&install.dir);
    install.launcher_config(
        "[VERSION]
Version=1.4.16
Build=20240101
",
    );
    install.user_config(
        "[Game]
Version_=1.4.20
",
    );
    install.game_exe();
    install
        .dir
        .write("install/NTELauncher/WPGameClientSDK_64.dll", b"MZ");
    install.dir.write("install/NTELauncher/NTEGame.exe", b"MZ");

    std::fs::create_dir_all(install.dir.join("prefix/drive_c")).expect("create prefix");
    let proton = fake_proton(&install.dir.join("tools"), "GE-Proton9-1");
    let bin = install.dir.join("bin");
    write_script(&bin, "umu-run", "#!/bin/sh\nexit 0\n");
    env.set("PATH", &bin);
    let profile = managed_environment(&proton, &install.dir.join("install"), ChannelId::new("cn"));
    let mut settings = install.settings();
    settings.runtime_environments = vec![profile.clone()];
    settings.current_mut().active_environment = Some(profile.id.clone());

    let bootstrap = bootstrap_impl(&crate::testing::packages(), &settings);
    let value = serde_json::to_value(&bootstrap).expect("bootstrap 序列化");
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "appVersion",
            "capabilities",
            "channels",
            "checks",
            "error",
            "gameExe",
            "gameExists",
            "gameVersion",
            "installRoot",
            "integrity",
            "launcherBuild",
            "launcherRoot",
            "launcherVersion",
            "ok",
            "os",
            "prefix",
            "prefixExists",
            "protonName",
            "protonPath",
            "runtimeInstallations",
            "sdkPath",
            "settings",
            "settingsSchemas",
            "umuAvailable",
        ]
    );
    assert_eq!(value["capabilities"], serde_json::json!([]));
    // 启动器版本, 游戏版本与 SDK 目录仍来自游戏包的安装域.
    assert_eq!(value["launcherVersion"], "1.4.16");
    assert_eq!(value["launcherBuild"], "20240101");
    assert_eq!(value["gameVersion"], "1.4.20");
    assert_eq!(value["sdkPath"], install.launcher.display().to_string());
    // 区服清单同样来自包: 两个发行渠道.
    assert_eq!(value["runtimeInstallations"][0]["distribution"], "cn");
    assert_eq!(value["runtimeInstallations"][1]["distribution"], "global");
    assert_eq!(value["settings"]["activeGame"], "nte");
    assert_eq!(value["checks"][5]["label"], "官方启动器");
}

#[test]
fn bootstrap_reports_capability_facts_for_the_current_game() {
    let dir = TempDir::new("bootstrap-capabilities");
    let _env = isolated_env(&dir);
    let mut registry = Registry::new();
    registry.install(&CapabilityProbe);
    let mut settings = Settings {
        active_game: GameId::from("probe"),
        ..Settings::default()
    };

    let bootstrap = bootstrap_impl(&registry, &settings);
    assert_eq!(
        serde_json::to_value(&bootstrap.capabilities).expect("能力事实序列化"),
        serde_json::json!([
            {
                "id": "oddity.probe.context",
                "revision": 2,
                "state": "available",
                "code": null,
                "message": null
            },
            {
                "id": "oddity.probe.offline",
                "revision": 1,
                "state": "unavailable",
                "code": "offline",
                "message": "网络不可用"
            }
        ])
    );

    // 同一份 sheet 在其它区服下按上下文重新求值.
    settings.current_mut().distribution = Some(ChannelId::new("global"));
    let bootstrap = bootstrap_impl(&registry, &settings);
    assert_eq!(
        bootstrap.capabilities,
        [
            CapabilityFact {
                id: "oddity.probe.context",
                revision: 2,
                state: CapabilityState::Unsupported,
                code: None,
                message: None,
            },
            CapabilityFact {
                id: "oddity.probe.offline",
                revision: 1,
                state: CapabilityState::Unavailable,
                code: Some("offline"),
                message: Some("网络不可用"),
            }
        ]
    );
}

#[test]
fn bootstrap_requires_a_managed_environment() {
    let install = Install::new("bootstrap-defaults");
    let _env = isolated_env(&install.dir);
    install.launcher_config("[VERSION]\nVersion=1.4.16\n");
    let bootstrap = bootstrap_impl(&crate::testing::packages(), &install.settings());
    assert!(!bootstrap.ok);
    assert!(bootstrap.error.unwrap().contains("请先"));
    assert!(bootstrap.prefix.is_empty());
    assert!(!bootstrap.prefix_exists);
    assert!(bootstrap.proton_path.is_empty());
}

#[test]
fn bootstrap_does_not_fall_back_from_a_configured_empty_root() {
    let dir = TempDir::new("bootstrap-selected-root");
    let _env = isolated_env(&dir);
    // 旧安装仍在默认位置，但不能覆盖用户明确选择的新目录。
    dir.write_text(
        "home/Games/Neverness To Everness/NTELauncher/Config/Config.ini",
        "[VERSION]\nVersion=1.4.16\n",
    );
    let selected = dir.join("new-install");
    let settings =
        crate::tests::settings_with_game(ChannelId::new("cn"), Some(selected.clone()), None);

    let bootstrap = bootstrap_impl(&crate::testing::packages(), &settings);
    assert_eq!(bootstrap.install_root, selected.display().to_string());
    assert_eq!(bootstrap.launcher_root, "");
    assert!(!bootstrap.ok);
    assert!(
        bootstrap
            .error
            .as_deref()
            .expect("error text")
            .contains("尚未安装启动器文件"),
        "{:?}",
        bootstrap.error
    );
    assert!(!bootstrap.game_exists);

    // 未指定目录时仍使用探测结果。
    let detected = bootstrap_impl(&crate::testing::packages(), &Settings::default());
    assert_eq!(
        detected.install_root,
        dir.join("home/Games/Neverness To Everness")
            .display()
            .to_string()
    );
    assert!(!detected.ok);
    assert!(detected.error.unwrap().contains("请先"));
}
