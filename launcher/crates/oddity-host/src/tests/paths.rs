use super::*;
use crate::channel::ChannelId;
use crate::identity::GameId;
use crate::install;
use crate::paths::expand_path;
use crate::runtime_setup::executable as which;
use crate::settings::Settings;

#[test]
fn expands_home_paths_only() {
    let mut env = EnvGuard::new();
    let home = TempDir::new("expand-home");
    env.set("HOME", home.path());
    assert_eq!(expand_path("~"), home.path());
    assert_eq!(expand_path("~/games"), home.join("games"));
    assert_eq!(expand_path("/abs/path"), PathBuf::from("/abs/path"));
    assert_eq!(expand_path("rel/path"), PathBuf::from("rel/path"));
    assert_eq!(expand_path("~other"), PathBuf::from("~other"));
}

#[test]
fn which_only_accepts_executable_files() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("which");
    let exe = write_script(dir.path(), "umu-run", "#!/bin/sh\nexit 0\n");
    dir.write("plain-file", b"x");
    std::fs::create_dir_all(dir.join("a-directory")).expect("create directory");

    env.set("PATH", dir.path());
    assert_eq!(which("umu-run"), Some(exe));
    assert_eq!(which("plain-file"), None);
    assert_eq!(which("a-directory"), None);
    assert_eq!(which("missing"), None);

    env.remove("PATH");
    assert_eq!(which("umu-run"), None);
}

#[test]
fn detection_routes_through_the_current_install_domain() {
    let install_dir = TempDir::new("detect-package");
    install_dir.write_text("NTELauncher/Config/Config.ini", "[VERSION]\n");
    let settings = settings_with_game(
        ChannelId::new("cn"),
        Some(install_dir.path().to_path_buf()),
        None,
    );
    let registry = crate::testing::packages();

    assert_eq!(
        install::launcher_root(&registry, &settings),
        Some(install_dir.join("NTELauncher"))
    );
    assert_eq!(
        install::selected_install_root(&registry, &settings),
        install_dir.path()
    );
    assert_eq!(
        install::launcher_dir(&registry, &settings),
        Some("NTELauncher")
    );
    assert_eq!(
        install::effective_install_root(
            &registry,
            &settings,
            Some(&install_dir.join("NTELauncher"))
        ),
        install_dir.path()
    );

    // 配置目录无效时只认配置, 不回退到环境变量里的其他安装.
    let mut env = EnvGuard::new();
    let stale = settings_with_game(ChannelId::new("cn"), Some(PathBuf::from("/nope")), None);
    env.set("NTE_ROOT", install_dir.path());
    assert_eq!(install::launcher_root(&registry, &stale), None);
}

#[test]
fn games_without_an_install_domain_have_no_guessed_paths() {
    let registry = crate::testing::packages();
    let settings = Settings {
        active_game: GameId::from("hsr"),
        ..Settings::default()
    };
    let launcher = Path::new("/games/nte/NTELauncher");

    assert_eq!(install::launcher_root(&registry, &settings), None);
    assert_eq!(
        install::selected_install_root(&registry, &settings),
        PathBuf::new()
    );
    assert_eq!(
        install::effective_install_root(&registry, &settings, None),
        PathBuf::new()
    );
    assert_eq!(install::launcher_dir(&registry, &settings), None);
    assert_eq!(
        install::game_exe(&registry, &GameId::from("hsr"), launcher),
        None
    );
    assert_eq!(
        install::game_version(&registry, &GameId::from("hsr"), launcher),
        ""
    );
    assert_eq!(
        install::game_exe_hint(&registry, &GameId::from("hsr")),
        "未找到游戏可执行文件"
    );
}
