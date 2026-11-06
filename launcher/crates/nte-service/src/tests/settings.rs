use super::*;
use crate::*;

#[test]
fn settings_default_matches_the_ui_contract() {
    let settings = Settings::default();
    assert!(settings.current().install_root.is_none());
    assert!(settings.current().global_install_root.is_none());
    assert_eq!(settings.current().distribution, Distribution::Cn);
    assert_eq!(settings.active_game, nte_core::game_profile::GameId::Nte);
    assert!(settings
        .games
        .contains_key(&nte_core::game_profile::GameId::Nte));
    assert_eq!(settings.current().launch_mode, "background");
    assert_eq!(settings.language, "zh-CN");
    assert!(!settings.reduce_motion);
    assert!(!settings.stack_bulletins);
    assert_eq!(settings.theme, "light");
    assert_eq!(settings.hero_media, "video");
    assert!(settings.relative_heatmap);
    assert!(settings.pause_hero_on_blur);
    assert!(settings.minimize_to_tray_on_close);
    assert!(settings.show_launcher_after_game_exit);
    assert_eq!(settings.snapshot_success_minutes, 1);
    assert!(!settings.current().updates.auto_download);
    assert_eq!(
        settings.current().updates.metered,
        nte_core::updates::MeteredPolicy::Ask
    );
    assert_eq!(settings.current().updates.limit_bytes_per_second, 0);
    assert!(settings.current().updates.window.is_none());
}

#[test]
fn download_only_blocks_region_and_install_paths() {
    let current = Settings::default();
    let mut next = current.clone();
    next.current_mut().launch_mode = "direct".into();
    next.language = "en-US".into();
    next.reduce_motion = true;
    next.theme = "dark".into();
    next.hero_media = "photo".into();
    next.relative_heatmap = false;
    next.pause_hero_on_blur = false;
    next.minimize_to_tray_on_close = false;
    next.show_launcher_after_game_exit = false;
    assert!(!download_paths_changed(&current, &next));

    let mut next = current.clone();
    next.current_mut().install_root = Some("/games/nte".into());
    assert!(download_paths_changed(&current, &next));

    let mut next = current.clone();
    next.current_mut().global_install_root = Some("/games/nte-global".into());
    assert!(download_paths_changed(&current, &next));

    let mut next = current.clone();
    next.current_mut().distribution = Distribution::Global;
    assert!(download_paths_changed(&current, &next));
}

#[test]
fn settings_round_trip_through_the_config_dir() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("settings");
    env.set("XDG_CONFIG_HOME", dir.path());

    // 没有配置文件时使用默认值
    assert_eq!(load_settings().current().launch_mode, "background");
    assert_eq!(settings_path(), dir.join("oddity/settings.json"));

    let mut settings = crate::tests::settings_with_game(
        Distribution::Global,
        Some(PathBuf::from("/games/nte")),
        Some(PathBuf::from("/games/nte-global")),
    );
    settings.current_mut().launch_mode = "direct".to_string();
    settings.language = "en-US".to_string();
    settings.reduce_motion = true;
    settings.stack_bulletins = true;
    settings.theme = "dark".to_string();
    settings.hero_media = "photo".to_string();
    settings.relative_heatmap = false;
    settings.pause_hero_on_blur = false;
    settings.minimize_to_tray_on_close = false;
    settings.show_launcher_after_game_exit = false;
    settings.snapshot_success_minutes = 10;
    persist_settings(&settings).expect("persist settings");

    let text = dir.read_text("oddity/settings.json");
    assert!(text.contains("\"activeGame\""), "{text}");
    assert!(text.contains("\"games\""), "{text}");
    assert!(text.contains("\"installRoot\""), "{text}");
    assert!(text.contains("\"reduceMotion\""), "{text}");
    assert!(text.contains("\"relativeHeatmap\": false"), "{text}");
    assert!(text.contains("\"pauseHeroOnBlur\": false"), "{text}");
    assert!(text.contains("\"minimizeToTrayOnClose\""), "{text}");
    assert!(text.contains("\"showLauncherAfterGameExit\""), "{text}");
    assert!(text.contains("\"snapshotSuccessMinutes\": 10"), "{text}");
    assert!(!dir.join("oddity/settings.json.tmp").exists());

    let loaded = load_settings();
    assert_eq!(loaded.current().install_root.as_deref(), Some("/games/nte"));
    assert_eq!(
        loaded.current().global_install_root.as_deref(),
        Some("/games/nte-global")
    );
    assert_eq!(loaded.current().distribution, Distribution::Global);
    assert_eq!(loaded.current().launch_mode, "direct");
    assert_eq!(loaded.language, "en-US");
    assert!(loaded.reduce_motion);
    assert!(loaded.stack_bulletins);
    assert_eq!(loaded.theme, "dark");
    assert_eq!(loaded.hero_media, "photo");
    assert!(!loaded.relative_heatmap);
    assert!(!loaded.pause_hero_on_blur);
    assert!(!loaded.minimize_to_tray_on_close);
    assert_eq!(loaded.snapshot_success_minutes, 10);
    assert!(!loaded.show_launcher_after_game_exit);
    assert!(!loaded.current().updates.auto_download);
    assert_eq!(
        loaded.current().updates.metered,
        nte_core::updates::MeteredPolicy::Ask
    );
}

#[test]
fn unsupported_snapshot_duration_falls_back_to_one_minute() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("settings-snapshot-duration");
    env.set("XDG_CONFIG_HOME", dir.path());
    persist_settings(&Settings {
        snapshot_success_minutes: 7,
        ..Default::default()
    })
    .expect("persist settings");
    assert_eq!(load_settings().snapshot_success_minutes, 1);
}

#[test]
fn settings_fall_back_when_the_file_is_broken_or_partial() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("settings-broken");
    env.set("XDG_CONFIG_HOME", dir.path());
    persist_settings(&Settings::default()).expect("persist settings");

    dir.write_text("oddity/settings.json", "{ not json");
    assert_eq!(load_settings().current().launch_mode, "background");

    // 缺字段时用 serde 默认值补齐
    dir.write_text("oddity/settings.json", "{\"theme\":\"dark\"}");
    let partial = load_settings();
    assert_eq!(partial.theme, "dark");
    assert!(partial.relative_heatmap);
    assert!(!partial.stack_bulletins);
    assert_eq!(partial.current().launch_mode, "background");
    assert_eq!(partial.hero_media, "video");
    assert!(partial.pause_hero_on_blur);
    assert!(partial.minimize_to_tray_on_close);
    assert!(partial.show_launcher_after_game_exit);
    assert!(!partial.current().updates.auto_download);
    assert!(partial.current().updates.window.is_none());
}

#[test]
fn settings_minimize_to_tray_round_trip() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("settings-tray");
    env.set("XDG_CONFIG_HOME", dir.path());

    let settings = Settings {
        minimize_to_tray_on_close: true,
        ..Settings::default()
    };
    persist_settings(&settings).expect("persist settings");
    let loaded = load_settings();
    assert!(loaded.minimize_to_tray_on_close);

    let settings_disabled = Settings {
        minimize_to_tray_on_close: false,
        ..Settings::default()
    };
    persist_settings(&settings_disabled).expect("persist settings");
    let loaded_disabled = load_settings();
    assert!(!loaded_disabled.minimize_to_tray_on_close);
}

#[test]
fn settings_report_write_failures() {
    let mut env = EnvGuard::new();
    let dir = TempDir::new("settings-blocked");
    // XDG_CONFIG_HOME 是一个文件：创建目录必然失败
    let blocked = dir.write("blocked", b"x");
    env.set("XDG_CONFIG_HOME", &blocked);
    let error = persist_settings(&Settings::default()).expect_err("cannot create config dir");
    assert!(error.contains("create "), "{error}");
    assert_eq!(load_settings().current().launch_mode, "background");
}

#[test]
fn resolves_data_and_config_directories_from_xdg() {
    let mut env = EnvGuard::new();
    let home = TempDir::new("xdg-home");
    env.set("HOME", home.path());
    env.remove("XDG_DATA_HOME");
    env.remove("XDG_CONFIG_HOME");

    assert_eq!(home_dir(), home.path());
    assert_eq!(data_base(), home.join(".local/share"));
    assert_eq!(data_dir(), home.join(".local/share/oddity"));
    assert_eq!(legacy_data_dir(), home.join(".local/share/TheBetterNTE"));
    assert_eq!(config_base(), home.join(".config"));
    assert_eq!(config_dir(), home.join(".config/oddity"));
    assert_eq!(legacy_config_dir(), home.join(".config/TheBetterNTE"));

    let xdg = TempDir::new("xdg-root");
    env.set("XDG_DATA_HOME", xdg.join("data"));
    env.set("XDG_CONFIG_HOME", xdg.join("cfg"));
    assert_eq!(data_base(), xdg.join("data"));
    assert_eq!(data_dir(), xdg.join("data/oddity"));
    assert_eq!(config_dir(), xdg.join("cfg/oddity"));

    // HOME 也不可用时退回 /tmp
    env.remove("HOME");
    env.remove("XDG_DATA_HOME");
    assert_eq!(home_dir(), PathBuf::from("/tmp"));
    assert_eq!(data_base(), PathBuf::from("/tmp/.local/share"));
}

#[test]
fn migrates_the_legacy_data_and_config_directories() {
    let mut env = EnvGuard::new();
    let data = TempDir::new("migrate-data");
    let config = TempDir::new("migrate-config");
    env.set("XDG_DATA_HOME", data.path());
    env.set("XDG_CONFIG_HOME", config.path());
    data.write("TheBetterNTE/marker.txt", b"data");
    config.write_text("TheBetterNTE/settings.json", "{\"theme\":\"dark\"}");

    migrate_legacy_dirs();

    assert_eq!(data.read("oddity/marker.txt"), b"data");
    assert_eq!(
        std::fs::read_link(data.join("TheBetterNTE")).expect("data symlink"),
        data.join("oddity")
    );
    assert_eq!(
        config.read_text("oddity/settings.json"),
        "{\"theme\":\"dark\"}"
    );
    assert_eq!(
        std::fs::read_link(config.join("TheBetterNTE")).expect("config symlink"),
        config.join("oddity")
    );

    // 再跑一次是幂等的，不会把符号链接再搬一次
    migrate_legacy_dirs();
    assert_eq!(data.read("oddity/marker.txt"), b"data");
    assert_eq!(
        config.read_text("oddity/settings.json"),
        "{\"theme\":\"dark\"}"
    );
}

#[test]
fn keeps_the_legacy_directories_when_targets_exist() {
    let mut env = EnvGuard::new();
    let data = TempDir::new("migrate-keep-data");
    let config = TempDir::new("migrate-keep-config");
    env.set("XDG_DATA_HOME", data.path());
    env.set("XDG_CONFIG_HOME", config.path());
    data.write("TheBetterNTE/marker.txt", b"old");
    data.write("oddity/current.txt", b"new");
    config.write("TheBetterNTE/settings.json", b"old");
    config.write("oddity/settings.json", b"new");

    migrate_legacy_dirs();

    assert_eq!(data.read("TheBetterNTE/marker.txt"), b"old");
    assert_eq!(data.read("oddity/current.txt"), b"new");
    assert!(!std::fs::symlink_metadata(data.join("TheBetterNTE"))
        .expect("legacy data dir")
        .file_type()
        .is_symlink());
    assert_eq!(config.read_text("oddity/settings.json"), "new");
}

#[test]
fn reports_migration_failures_without_panicking() {
    // root 不受目录权限限制，这个用例只在普通用户下有意义
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let mut env = EnvGuard::new();
    let data = TempDir::new("migrate-blocked-data");
    let config = TempDir::new("migrate-blocked-config");
    env.set("XDG_DATA_HOME", data.path());
    env.set("XDG_CONFIG_HOME", config.path());
    data.write("TheBetterNTE/marker.txt", b"data");
    config.write_text("TheBetterNTE/settings.json", "{}");
    std::fs::set_permissions(data.path(), std::fs::Permissions::from_mode(0o555))
        .expect("chmod data");
    std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o555))
        .expect("chmod config");

    migrate_legacy_dirs();

    std::fs::set_permissions(data.path(), std::fs::Permissions::from_mode(0o755)).ok();
    std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o755)).ok();
    // 迁移失败后原目录保持不动
    assert_eq!(data.read("TheBetterNTE/marker.txt"), b"data");
    assert_eq!(config.read_text("TheBetterNTE/settings.json"), "{}");
    assert!(!data.join("oddity").exists());
    assert!(!config.join("oddity").exists());
}

#[test]
fn global_settings_preserve_background_launch_mode() {
    let mut env = EnvGuard::new();
    let directory = TempDir::new("global-settings-background");
    env.set("XDG_CONFIG_HOME", directory.path());
    let mut settings = Settings::default();
    settings.current_mut().distribution = Distribution::Global;
    persist_settings(&settings).unwrap();
    let loaded = load_settings();
    assert_eq!(loaded.current().distribution, Distribution::Global);
    assert_eq!(loaded.current().launch_mode, "background");
}

#[test]
fn legacy_settings_file_is_migrated_on_load() {
    let mut env = EnvGuard::new();
    let directory = TempDir::new("settings-legacy-file");
    env.set("XDG_CONFIG_HOME", directory.path());
    let original = r#"{"distribution":"global","installRoot":"/games/cn","globalInstallRoot":"/games/global","theme":"dark"}"#;
    directory.write_text("oddity/settings.json", original);

    let loaded = load_settings();
    let game = loaded.current();
    assert_eq!(game.distribution, Distribution::Global);
    assert_eq!(game.install_root.as_deref(), Some("/games/cn"));
    assert_eq!(game.global_install_root.as_deref(), Some("/games/global"));
    assert_eq!(loaded.theme, "dark");
    assert_eq!(directory.read_text("oddity/settings.json.bak"), original);
    let migrated: serde_json::Value =
        serde_json::from_str(&directory.read_text("oddity/settings.json")).unwrap();
    assert!(migrated.get("installRoot").is_none());
    assert_eq!(migrated["games"]["nte"]["installRoot"], "/games/cn");
}

#[test]
fn saving_after_a_failed_migration_backs_up_the_legacy_file() {
    let mut env = EnvGuard::new();
    let directory = TempDir::new("settings-legacy-save");
    env.set("XDG_CONFIG_HOME", directory.path());
    let original = r#"{"installRoot":"/games/cn","theme":"dark"}"#;
    directory.write_text("oddity/settings.json", original);

    persist_settings(&Settings::default()).expect("persist settings");

    assert_eq!(directory.read_text("oddity/settings.json.bak"), original);
    let migrated: serde_json::Value =
        serde_json::from_str(&directory.read_text("oddity/settings.json")).unwrap();
    assert!(migrated.get("installRoot").is_none());
}

#[test]
fn failed_backup_keeps_the_legacy_file_untouched() {
    let mut env = EnvGuard::new();
    let directory = TempDir::new("settings-backup-failure");
    env.set("XDG_CONFIG_HOME", directory.path());
    let original = r#"{"installRoot":"/games/cn","theme":"dark"}"#;
    directory.write_text("oddity/settings.json", original);
    std::fs::set_permissions(
        directory.join("oddity"),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();

    let error = persist_settings(&Settings::default()).unwrap_err();
    assert!(error.contains("备份旧设置失败"), "{error}");
    assert_eq!(directory.read_text("oddity/settings.json"), original);

    std::fs::set_permissions(
        directory.join("oddity"),
        std::fs::Permissions::from_mode(0o755),
    )
    .ok();
}
