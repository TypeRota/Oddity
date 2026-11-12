//! 安装探测测试: 环境变量、候选目录、版本文件与游戏文件.

use super::*;
use crate::testing::{EnvGuard, TempDir};
use std::ffi::OsString;

fn probe_for(home: &TempDir) -> Probe {
    Probe {
        home: home.path().to_path_buf(),
        cwd: None,
        vars: Vec::new(),
        mounts: Vec::new(),
    }
}

fn query(distribution: &str) -> InstallQuery {
    InstallQuery {
        distribution: distribution.into(),
        configured_install_root: None,
    }
}

fn configured(distribution: &str, path: PathBuf) -> InstallQuery {
    InstallQuery {
        distribution: distribution.into(),
        configured_install_root: Some(path),
    }
}

fn var(key: &'static str, value: &TempDir) -> (&'static str, OsString) {
    (key, value.path().as_os_str().to_owned())
}

#[test]
fn expands_only_home_paths() {
    let home = Path::new("/home/test");
    assert_eq!(expand("~", home), PathBuf::from("/home/test"));
    assert_eq!(expand("~/games", home), PathBuf::from("/home/test/games"));
    assert_eq!(expand("~other", home), PathBuf::from("~other"));
    assert_eq!(expand("/abs/path", home), PathBuf::from("/abs/path"));
    assert_eq!(expand("rel/path", home), PathBuf::from("rel/path"));
}

#[test]
fn seeds_cover_the_known_layouts() {
    let mut seeds = Vec::new();
    install_seeds(Path::new("/mnt/games"), Region::Cn, &mut seeds);
    assert!(seeds.contains(&PathBuf::from(
        "/mnt/games/Program Files (x86)/Neverness To Everness/NTELauncher"
    )));
    assert!(seeds.contains(&PathBuf::from(
        "/mnt/games/Program Files/Neverness to Everness/NTELauncher"
    )));
    assert!(seeds.contains(&PathBuf::from("/mnt/games/NTELauncher")));
    seeds.clear();
    install_seeds(Path::new("/mnt/games"), Region::Global, &mut seeds);
    assert!(seeds.contains(&PathBuf::from(
        "/mnt/games/Program Files (x86)/Neverness To Everness Global/NTEGlobal"
    )));
    assert!(seeds.iter().all(|path| path.ends_with("NTEGlobal")));
}

#[test]
fn normalizes_candidate_launcher_roots() {
    let dir = TempDir::new("install-normalize");
    assert_eq!(normalize(dir.path(), Region::Cn), None);
    dir.write("Config/Config.ini", "[VERSION]\n");
    assert_eq!(
        normalize(dir.path(), Region::Cn),
        Some(dir.path().to_path_buf())
    );

    // 安装目录形态: <install>/NTELauncher/Config/Config.ini
    let install = TempDir::new("install-normalize-nested");
    install.write("NTELauncher/Config/Config.ini", "[VERSION]\n");
    assert_eq!(
        normalize(install.path(), Region::Cn),
        Some(install.join("NTELauncher"))
    );
    // 同一个安装根下的国际服目录不会被当成国服.
    assert_eq!(normalize(install.path(), Region::Global), None);
    install.write("NTEGlobal/Config/Config.ini", "[VERSION]\n");
    assert_eq!(
        normalize(install.path(), Region::Global),
        Some(install.join("NTEGlobal"))
    );
}

#[test]
fn rejects_launcher_roots_of_another_region_or_game() {
    let install = TempDir::new("install-game-id");
    install.write(
        "NTELauncher/Config/Config.ini",
        "[General]\nGameID=3000001\n",
    );
    assert_eq!(normalize(install.path(), Region::Cn), None);

    install.write("NTELauncher/Config/Config.ini", "[General]\nGameID=1289\n");
    assert_eq!(
        normalize(install.path(), Region::Cn),
        Some(install.join("NTELauncher"))
    );

    // 非法 GameID 与没有填写等价, 由启动器目录名决定区服.
    install.write("NTELauncher/Config/Config.ini", "[General]\nGameID=abc\n");
    assert_eq!(
        normalize(install.path(), Region::Cn),
        Some(install.join("NTELauncher"))
    );
    install.write("NTELauncher/Config/Config.ini", "[General]\nGameID=0\n");
    assert_eq!(
        normalize(install.path(), Region::Cn),
        Some(install.join("NTELauncher"))
    );
    install.write("NTELauncher/Config/Config.ini", "[General]\nGameID=-1\n");
    assert_eq!(normalize(install.path(), Region::Cn), None);
}

#[test]
fn rejects_another_regions_launcher_directory() {
    let cn = TempDir::new("install-cross-region-cn");
    cn.write("NTELauncher/Config/Config.ini", "[General]\nGameID=1289\n");
    // 国服的启动器目录不能被国际服探测选中, 反之亦然.
    assert_eq!(normalize(&cn.join("NTELauncher"), Region::Global), None);
    let global = TempDir::new("install-cross-region-global");
    global.write("NTEGlobal/Config/Config.ini", "[General]\nGameID=3000001\n");
    assert_eq!(normalize(&global.join("NTEGlobal"), Region::Cn), None);
}

#[test]
fn configured_root_is_authoritative() {
    let home = TempDir::new("install-config-home");
    let install = TempDir::new("install-config-env");
    install.write("NTELauncher/Config/Config.ini", "[VERSION]\n");
    let mut probe = probe_for(&home);
    probe.vars.push(var("NTE_ROOT", &install));

    // 配置目录无效时不能改用环境变量里的其他安装.
    assert_eq!(
        detect(&probe, &configured("cn", PathBuf::from("/nope"))),
        None
    );
}

#[test]
fn detects_from_environment_variables() {
    let home = TempDir::new("install-env-home");
    let install = TempDir::new("install-env-root");
    install.write("NTELauncher/Config/Config.ini", "[VERSION]\n");

    let mut probe = probe_for(&home);
    probe.vars.push(var("NTE_ROOT", &install));
    assert_eq!(
        detect(&probe, &query("cn")),
        Some(install.join("NTELauncher"))
    );

    // NTE_ROOT 无效时继续尝试 NTE_INSTALL.
    probe.vars.clear();
    probe.vars.push(("NTE_ROOT", OsString::from("/nope")));
    probe.vars.push(var("NTE_INSTALL", &install));
    assert_eq!(
        detect(&probe, &query("cn")),
        Some(install.join("NTELauncher"))
    );
}

#[test]
fn detects_the_home_and_chinese_seeds() {
    let home = TempDir::new("install-home-seeds");
    home.write(
        "Games/Neverness To Everness/NTELauncher/Config/Config.ini",
        "[VERSION]\n",
    );
    assert_eq!(
        detect(&probe_for(&home), &query("cn")),
        Some(home.join("Games/Neverness To Everness/NTELauncher"))
    );

    let chinese = TempDir::new("install-home-chinese");
    chinese.write("Games/异环/NTELauncher/Config/Config.ini", "[VERSION]\n");
    assert_eq!(
        detect(&probe_for(&chinese), &query("cn")),
        Some(chinese.join("Games/异环/NTELauncher"))
    );
}

#[test]
fn detects_legacy_install_name_spellings() {
    let home = TempDir::new("install-legacy-home");
    home.write(
        "Games/Neverness to Everness/NTELauncher/Config/Config.ini",
        "[VERSION]\n",
    );
    assert_eq!(
        detect(&probe_for(&home), &query("cn")),
        Some(home.join("Games/Neverness to Everness/NTELauncher"))
    );
}

#[test]
fn global_install_uses_its_own_launcher_and_game_id() {
    let home = TempDir::new("install-global-home");
    home.write(
        "Games/Neverness To Everness Global/NTEGlobal/Config/Config.ini",
        "[General]\nGameID=3000001\n",
    );
    assert_eq!(
        detect(&probe_for(&home), &query("global")),
        Some(home.join("Games/Neverness To Everness Global/NTEGlobal"))
    );

    // 国服的配置不能被国际服探测选中.
    let other = TempDir::new("install-global-other");
    other.write(
        "Games/Neverness To Everness/NTELauncher/Config/Config.ini",
        "[General]\nGameID=1289\n",
    );
    assert_eq!(detect(&probe_for(&other), &query("global")), None);
}

#[test]
fn detects_from_the_working_directory_and_mount_roots() {
    let home = TempDir::new("install-scan-home");

    let tree = TempDir::new("install-seed-tree");
    tree.write("child/install/NTELauncher/Config/Config.ini", "[VERSION]\n");
    let mut probe = probe_for(&home);
    probe.cwd = Some(tree.join("child/install"));
    assert_eq!(
        detect(&probe, &query("cn")),
        Some(tree.join("child/install/NTELauncher"))
    );

    let mounts = TempDir::new("install-mounts");
    mounts.write(
        "user/drive/Neverness To Everness/NTELauncher/Config/Config.ini",
        "[VERSION]\n",
    );
    let mut probe = probe_for(&home);
    probe.mounts = vec![mounts.path().to_path_buf()];
    assert_eq!(
        detect(&probe, &query("cn")),
        Some(mounts.join("user/drive/Neverness To Everness/NTELauncher"))
    );
}

#[test]
fn unknown_distributions_have_no_install_knowledge() {
    let home = TempDir::new("install-unknown-home");
    let probe = probe_for(&home);
    assert_eq!(detect(&probe, &query("eu")), None);
    assert_eq!(launcher_dir(&query("eu")), None);
    assert_eq!(launcher_dir(&query("cn")), Some("NTELauncher"));
    assert_eq!(launcher_dir(&query("global")), Some("NTEGlobal"));
    assert_eq!(
        selected_install_root(&probe, &query("eu")),
        home.join("Games")
    );
    assert_eq!(
        effective_install_root(&probe, &query("eu"), None),
        home.join("Games")
    );
}

#[test]
fn selects_the_configured_or_default_install_root() {
    let home = TempDir::new("install-selected-home");
    let probe = probe_for(&home);
    assert_eq!(
        selected_install_root(&probe, &query("cn")),
        home.join("Games/Neverness To Everness")
    );
    assert_eq!(
        selected_install_root(&probe, &query("global")),
        home.join("Games/Neverness To Everness Global")
    );

    let custom = configured("cn", home.join("custom"));
    assert_eq!(selected_install_root(&probe, &custom), home.join("custom"));

    // 选中了启动器目录本身时取父目录作为安装根.
    let launcher = configured("cn", home.join("custom/NTELauncher"));
    assert_eq!(
        selected_install_root(&probe, &launcher),
        home.join("custom")
    );
    assert_eq!(
        effective_install_root(&probe, &launcher, None),
        home.join("custom")
    );
}

#[test]
fn effective_install_root_prefers_the_launcher_parent() {
    let home = TempDir::new("install-effective-home");
    let probe = probe_for(&home);
    assert_eq!(
        effective_install_root(
            &probe,
            &query("cn"),
            Some(Path::new("/games/nte/NTELauncher"))
        ),
        PathBuf::from("/games/nte")
    );
    assert_eq!(
        effective_install_root(&probe, &query("cn"), Some(Path::new("/"))),
        home.join("Games/Neverness To Everness")
    );
    assert_eq!(
        effective_install_root(&probe, &query("cn"), None),
        home.join("Games/Neverness To Everness")
    );
}

#[test]
fn reads_ini_values_with_bom_comments_and_duplicates() {
    let text = "\u{feff}; 注释\n[General]\nGameID=1289\n[Game]\nVersion_=1.4.20\nVersion_=1.4.21\n";
    assert_eq!(ini_value(text, "General", "GameID"), Some("1289"));
    assert_eq!(ini_value(text, "Game", "Version_"), Some("1.4.21"));
    assert_eq!(ini_value(text, "Game", "Missing"), None);
    assert_eq!(
        ini_value("[VERSION]\nVersion=1\n", "VERSION", "Version"),
        Some("1")
    );
    // 第一个 section 之前的键属于 General.
    assert_eq!(ini_value("GameID=7\n", "General", "GameID"), Some("7"));
    assert_eq!(ini_value("# 注释\n; 注释\n", "General", "GameID"), None);
}

#[test]
fn reads_the_game_version_from_user_config() {
    let dir = TempDir::new("install-version");
    let launcher = dir.join("NTELauncher");
    assert_eq!(game_version(&launcher), "");

    dir.write(
        "NTELauncher/UserData/Config/Config.ini",
        "[Game]\nVersion_=1.4.20\n",
    );
    assert_eq!(game_version(&launcher), "1.4.20");

    // 空值等同于没有版本.
    dir.write(
        "NTELauncher/UserData/Config/Config.ini",
        "[Game]\nVersion_=\n",
    );
    assert_eq!(game_version(&launcher), "");
}

#[test]
fn reads_the_launcher_version_from_config() {
    let dir = TempDir::new("install-launcher-info");
    let launcher = dir.join("NTELauncher");
    assert_eq!(launcher_info(&launcher), None);

    dir.write(
        "NTELauncher/Config/Config.ini",
        "[General]\nGameID=1289\n[VERSION]\nVersion=1.4.16\nBuild=20240101\n",
    );
    assert_eq!(
        launcher_info(&launcher),
        Some(LauncherInfo {
            version: "1.4.16".into(),
            build: "20240101".into(),
        })
    );

    // 配置存在但没有 [VERSION] 段时给空值, 与旧配置读取一致.
    dir.write("NTELauncher/Config/Config.ini", "[General]\nGameID=1289\n");
    assert_eq!(
        launcher_info(&launcher),
        Some(LauncherInfo {
            version: String::new(),
            build: String::new(),
        })
    );
}

#[test]
fn locates_the_game_executable_case_insensitively() {
    let dir = TempDir::new("install-game-exe");
    dir.write(
        "install/client/windowsnoeditor/ht/binaries/win64/htgame.exe",
        b"MZ",
    );
    assert_eq!(
        game_exe(&dir.join("install/NTELauncher")),
        Some(dir.join("install/client/windowsnoeditor/ht/binaries/win64/htgame.exe"))
    );

    // 没有父目录时无法推断安装位置.
    assert_eq!(game_exe(Path::new("/")), None);
    let empty = TempDir::new("install-game-exe-empty");
    assert_eq!(game_exe(&empty.join("install/NTELauncher")), None);
    assert!(game_exe_hint().contains("HTGame.exe"));
}

#[test]
fn locator_reads_the_process_environment() {
    let mut env = EnvGuard::new();
    let home = TempDir::new("install-process-home");
    let install = TempDir::new("install-process-install");
    install.write("NTELauncher/Config/Config.ini", "[VERSION]\n");
    install.write(
        "NTELauncher/UserData/Config/Config.ini",
        "[Game]\nVersion_=1.4.20\n",
    );
    env.set("HOME", home.path());
    env.set("NTE_ROOT", install.path());

    let locator = locator();
    let query = query("cn");
    assert_eq!(
        locator.launcher_root(&query),
        Some(install.join("NTELauncher"))
    );
    assert_eq!(locator.launcher_dir(&query), Some("NTELauncher"));
    assert_eq!(
        locator.selected_install_root(&query),
        home.join("Games/Neverness To Everness")
    );
    assert_eq!(
        locator.effective_install_root(&query, None),
        home.join("Games/Neverness To Everness")
    );
    assert_eq!(
        locator.effective_install_root(&query, Some(&install.join("NTELauncher"))),
        install.path()
    );
    assert_eq!(locator.game_exe(&install.join("NTELauncher")), None);
    assert_eq!(locator.game_version(&install.join("NTELauncher")), "1.4.20");
    assert_eq!(
        locator.launcher_info(&install.join("NTELauncher")),
        Some(LauncherInfo {
            version: String::new(),
            build: String::new(),
        })
    );
    assert!(locator.game_exe_hint().contains("HTGame.exe"));
}

#[test]
fn process_probe_falls_back_to_tmp_without_home() {
    let mut env = EnvGuard::new();
    env.remove("HOME");
    assert_eq!(Probe::process().home, PathBuf::from("/tmp"));
}

#[test]
fn finds_case_insensitive_components() {
    let dir = TempDir::new("find-ci");
    dir.write("Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe", b"MZ");
    assert_eq!(
        find_ci(
            dir.path(),
            &[
                "client",
                "windowsnoeditor",
                "ht",
                "binaries",
                "win64",
                "htgame.exe"
            ]
        ),
        Some(dir.join("Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe"))
    );
    assert_eq!(find_ci(dir.path(), &["missing"]), None);
    assert_eq!(find_ci(&dir.join("nope"), &["Client"]), None);
}

#[test]
fn locates_sdk_dir_and_launcher_config() {
    let dir = TempDir::new("install-sdk");
    dir.write("NTELauncher/WPGameClientSDK_64.dll", b"MZ");
    assert_eq!(
        locator().sdk_dir(&dir.join("NTELauncher")),
        Some(dir.join("NTELauncher"))
    );
    let config = dir.write(
        "NTELauncher/Client/WindowsNoEditor/HT/Binaries/Win64/GameLauncher.config",
        b"Z:\\sdk",
    );
    assert_eq!(
        locator().launcher_config_path(&dir.join("NTELauncher")),
        Some(config)
    );
    assert!(locator().sdk_dir(&dir.join("missing")).is_none());
}

#[test]
fn exposes_official_client_and_prefix_link() {
    let cn = Path::new("/games/install/NTELauncher");
    assert_eq!(
        INSTALL.official_client_exe(cn),
        Some(cn.join("NTEGame.exe"))
    );
    let global = Path::new("/games/install/NTEGlobal");
    assert_eq!(
        INSTALL.official_client_exe(global),
        Some(global.join("NTEGlobalGame.exe"))
    );
    assert_eq!(
        INSTALL.official_client_exe(Path::new("/games/install/unknown")),
        None
    );

    // 官方目录名之外, Config.ini 的 GameID 也能定位客户端.
    let custom = TempDir::new("install-official-client");
    custom.write("Config/Config.ini", "[General]\nGameID=3000001\n");
    assert_eq!(
        INSTALL.official_client_exe(custom.path()),
        Some(custom.join("NTEGlobalGame.exe"))
    );
    assert_eq!(
        INSTALL.prefix_link_name(),
        Some("Program Files (x86)/Neverness To Everness")
    );
}

#[test]
fn launcher_config_is_optional() {
    let dir = TempDir::new("install-sdk-missing");
    assert!(locator()
        .launcher_config_path(&dir.join("NTELauncher"))
        .is_none());
}
