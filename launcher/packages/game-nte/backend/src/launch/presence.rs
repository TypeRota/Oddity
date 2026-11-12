//! 官方启动器的 presence 调优与 Wine 托盘隐藏.
//!
//! 官方启动器默认在游戏退出后继续驻留并检查更新; 这里在启动前把它改写成
//! "随游戏退出"的薄壳. 配置缺失或写入失败都不阻塞启动, 由调用方记录日志.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// 保留官方资源初始化, 游戏退出后关闭官方后台.
const PRESENCE_SETTINGS: [(&str, &str); 4] = [
    ("autoUpdate", "1"),
    ("quitWithGame", "1"),
    ("showAfterGameQuit", "0"),
    ("closeAction", "0"),
];
const PRESENCE_UPDATER: [(&str, &str); 1] = [("NeedUpdate", "0")];

fn line_ending(line: &str, fallback: &str) -> &'static str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        // 最后一行没有换行符时, 不能吞掉随后追加的键.
        if fallback == "\r\n" {
            "\r\n"
        } else {
            "\n"
        }
    }
}

fn append_presence_keys(
    out: &mut String,
    section: &str,
    seen_setting: &mut Vec<&'static str>,
    seen_updater: &mut Vec<&'static str>,
    eol: &str,
) {
    if section.eq_ignore_ascii_case("setting") {
        for (key, value) in PRESENCE_SETTINGS {
            if !seen_setting.contains(&key) {
                out.push_str(&format!("{key}={value}{eol}"));
                seen_setting.push(key);
            }
        }
    } else if section.eq_ignore_ascii_case("updater") {
        for (key, value) in PRESENCE_UPDATER {
            if !seen_updater.contains(&key) {
                out.push_str(&format!("{key}={value}{eol}"));
                seen_updater.push(key);
            }
        }
    }
}

/// 保留原有段落与其它键, 只覆盖 presence 需要的项.
fn apply_presence_settings(text: &str) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(text.len() + 128);
    let mut section = String::new();
    let mut setting_seen = false;
    let mut updater_seen = false;
    let mut seen_setting: Vec<&'static str> = Vec::new();
    let mut seen_updater: Vec<&'static str> = Vec::new();

    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']).trim();
        if body.starts_with('[') && body.ends_with(']') {
            append_presence_keys(
                &mut out,
                &section,
                &mut seen_setting,
                &mut seen_updater,
                eol,
            );
            section = body[1..body.len() - 1].trim().to_string();
            if section.eq_ignore_ascii_case("setting") {
                setting_seen = true;
            }
            if section.eq_ignore_ascii_case("updater") {
                updater_seen = true;
            }
            out.push_str(line);
            continue;
        }

        if let Some((key, _)) = body.split_once('=') {
            let key_trim = key.trim();
            if section.eq_ignore_ascii_case("setting") {
                if let Some((matched, value)) = PRESENCE_SETTINGS
                    .iter()
                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key_trim))
                {
                    let ending = line_ending(line, eol);
                    out.push_str(&format!("{key_trim}={value}{ending}"));
                    if !seen_setting.contains(matched) {
                        seen_setting.push(matched);
                    }
                    continue;
                }
            } else if section.eq_ignore_ascii_case("updater") {
                if let Some((matched, value)) = PRESENCE_UPDATER
                    .iter()
                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key_trim))
                {
                    let ending = line_ending(line, eol);
                    out.push_str(&format!("{key_trim}={value}{ending}"));
                    if !seen_updater.contains(matched) {
                        seen_updater.push(matched);
                    }
                    continue;
                }
            }
        }

        out.push_str(line);
    }

    append_presence_keys(
        &mut out,
        &section,
        &mut seen_setting,
        &mut seen_updater,
        eol,
    );

    if !setting_seen {
        out.push_str(&format!("[Setting]{eol}"));
        for (key, value) in PRESENCE_SETTINGS {
            out.push_str(&format!("{key}={value}{eol}"));
        }
    }
    if !updater_seen {
        out.push_str(&format!("[UPDATER]{eol}"));
        for (key, value) in PRESENCE_UPDATER {
            out.push_str(&format!("{key}={value}{eol}"));
        }
    }
    out
}

/// 改写即将运行的官方启动器配置.
///
/// 配置缺失时保持原样 (官方启动器会生成默认值); 写入失败只上报, 不阻塞启动.
pub(crate) fn prepare_launcher(launcher_dir: &Path) -> Result<(), String> {
    let config = launcher_dir
        .join("UserData")
        .join("Config")
        .join("Config.ini");
    let original = match fs::read_to_string(&config) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read {}: {error}", config.display())),
    };
    let updated = apply_presence_settings(&original);
    if updated == original {
        return Ok(());
    }

    let backup = config.with_file_name("Config.ini.nte-presence.bak");
    if !backup.exists() {
        fs::write(&backup, &original)
            .map_err(|error| format!("write {}: {error}", backup.display()))?;
        // 配置里可能有账号信息, 备份保持私有.
        let _ = fs::set_permissions(&backup, fs::Permissions::from_mode(0o600));
    }
    let tmp = config.with_file_name("Config.ini.nte-presence.tmp");
    fs::write(&tmp, updated).map_err(|error| format!("write {}: {error}", tmp.display()))?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    fs::rename(&tmp, &config).map_err(|error| format!("rename {}: {error}", config.display()))?;
    Ok(())
}

/// 隐藏 Wine 系统托盘里的宿主项, 不结束 explorer.exe.
///
/// Wine 在 shell 启动时读取 NoTrayItemsDisplay 策略; 只为当前 prefix 设置,
/// 自动登录与游戏窗口不受影响. 已在 GE-Proton 11-5 / Wine 11.0 验证.
pub(crate) fn hide_wine_tray(proton_root: &Path, prefix: &Path) -> Result<(), String> {
    let wine = proton_root.join("files").join("bin").join("wine");
    if !wine.is_file() {
        return Err(format!("未找到 {}", wine.display()));
    }
    let status = Command::new(&wine)
        .args([
            "reg",
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer",
            "/v",
            "NoTrayItemsDisplay",
            "/t",
            "REG_DWORD",
            "/d",
            "1",
            "/f",
        ])
        .env("WINEPREFIX", prefix)
        .env("WINEDEBUG", "-all")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("执行 wine reg 失败：{error}"))?;
    if !status.success() {
        return Err(format!("wine reg 返回 {status}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{write_script, TempDir};

    #[test]
    fn prepare_writes_backup_and_keeps_credentials() {
        let root = TempDir::new("presence");
        let config_dir = root.join("UserData").join("Config");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let config = config_dir.join("Config.ini");
        fs::write(
            &config,
            "[Setting]\r\nautoLogin=1\r\n[User]\r\nuser_t92=deadbeef\r\n",
        )
        .expect("write config");

        prepare_launcher(root.path()).expect("apply presence");
        let updated = fs::read_to_string(&config).expect("read updated");
        assert!(updated.contains("autoLogin=1"));
        assert!(updated.contains("user_t92=deadbeef"));
        assert!(updated.contains("autoUpdate=1"));
        assert!(updated.contains("quitWithGame=1"));
        assert!(updated.contains("showAfterGameQuit=0"));
        assert!(updated.contains("closeAction=0"));
        assert!(updated.contains("NeedUpdate=0"));

        let backup = config_dir.join("Config.ini.nte-presence.bak");
        assert!(backup.is_file());
        assert_eq!(
            fs::read_to_string(&backup).expect("read backup"),
            "[Setting]\r\nautoLogin=1\r\n[User]\r\nuser_t92=deadbeef\r\n"
        );
        // 配置里可能有账号信息, 备份必须保持私有.
        let mode = fs::metadata(&backup)
            .expect("backup metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!config_dir.join("Config.ini.nte-presence.tmp").exists());

        // 第二次执行是空操作, 不会重写备份.
        let backup_before = fs::read(&backup).expect("read backup");
        prepare_launcher(root.path()).expect("apply presence again");
        let backup_after = fs::read(&backup).expect("read backup again");
        assert_eq!(backup_before, backup_after);
    }

    #[test]
    fn presence_is_a_noop_without_a_config() {
        let dir = TempDir::new("presence-missing");
        prepare_launcher(dir.path()).expect("missing config is fine");
        assert!(!dir.join("UserData/Config").exists());
    }

    #[test]
    fn presence_reports_unreadable_configs() {
        let dir = TempDir::new("presence-unreadable");
        fs::create_dir_all(dir.join("UserData/Config/Config.ini")).expect("create directory");
        let error = prepare_launcher(dir.path()).expect_err("config is a directory");
        assert!(error.contains("read "), "{error}");
    }

    #[test]
    fn presence_leaves_an_up_to_date_config_alone() {
        let dir = TempDir::new("presence-stable");
        let original = "[Setting]\nautoUpdate=1\nquitWithGame=1\nshowAfterGameQuit=0\ncloseAction=0\n[UPDATER]\nNeedUpdate=0\n";
        dir.write("UserData/Config/Config.ini", original);

        prepare_launcher(dir.path()).expect("apply presence");

        assert_eq!(dir.read_text("UserData/Config/Config.ini"), original);
        assert!(!dir
            .join("UserData/Config/Config.ini.nte-presence.bak")
            .exists());
        assert!(!dir
            .join("UserData/Config/Config.ini.nte-presence.tmp")
            .exists());
    }

    #[test]
    fn presence_settings_preserve_other_sections_and_crlf() {
        let input =
            "[Setting]\r\nautoLogin=1\r\n\r\n[UPDATER]\r\nNeedUpdate=1\r\n[Game]\r\nloginType=92\r\n";
        let output = apply_presence_settings(input);
        assert!(output.contains("autoLogin=1\r\n"));
        assert!(output.contains("autoUpdate=1\r\n"));
        assert!(output.contains("quitWithGame=1\r\n"));
        assert!(output.contains("showAfterGameQuit=0\r\n"));
        assert!(output.contains("closeAction=0\r\n"));
        assert!(output.contains("NeedUpdate=0\r\n"));
        assert!(!output.contains("NeedUpdate=1"));
        assert!(output.contains("loginType=92\r\n"));
        assert_eq!(apply_presence_settings(&output), output);
    }

    #[test]
    fn presence_settings_add_missing_sections() {
        let output = apply_presence_settings("[GENERAL]\npreVersion=1\n");
        assert!(output.contains("[Setting]\n"));
        assert!(output.contains("autoUpdate=1\n"));
        assert!(output.contains("quitWithGame=1\n"));
        assert!(output.contains("showAfterGameQuit=0\n"));
        assert!(output.contains("closeAction=0\n"));
        assert!(output.contains("[UPDATER]\n"));
        assert!(output.contains("NeedUpdate=0\n"));
        assert!(output.contains("preVersion=1"));
    }

    #[test]
    fn presence_settings_cover_an_empty_file() {
        assert_eq!(
            apply_presence_settings(""),
            "[Setting]\nautoUpdate=1\nquitWithGame=1\nshowAfterGameQuit=0\ncloseAction=0\n[UPDATER]\nNeedUpdate=0\n"
        );
    }

    #[test]
    fn presence_settings_handle_section_and_key_case() {
        let output = apply_presence_settings("[SETTING]\nAUTOUPDATE=0\n[updater]\nneedupdate=1\n");
        assert!(output.contains("AUTOUPDATE=1"), "{output}");
        assert!(output.contains("quitWithGame=1"), "{output}");
        assert!(output.contains("needupdate=0"), "{output}");
        assert!(!output.contains("AUTOUPDATE=0"), "{output}");
        assert!(!output.contains("needupdate=1"), "{output}");
        // 两个段都存在, 不再追加新段.
        assert_eq!(output.matches("[SETTING]").count(), 1);
        assert_eq!(output.matches("[updater]").count(), 1);
    }

    #[test]
    fn presence_settings_keep_a_last_line_without_newline() {
        let output = apply_presence_settings("[setting]\nautoUpdate=1");
        assert!(output.contains("autoUpdate=1"), "{output}");
        assert!(output.contains("quitWithGame=1\n"), "{output}");
        assert!(output.contains("[UPDATER]\n"), "{output}");

        // 使用 CRLF 的文件里补出来的键也要是 CRLF.
        let output = apply_presence_settings("[General]\r\nkey=1");
        assert!(output.contains("key=1"), "{output}");
        assert!(output.ends_with("NeedUpdate=0\r\n"), "{output}");
    }

    #[test]
    fn presence_settings_append_to_an_empty_section() {
        let output = apply_presence_settings("[Setting]\n[UPDATER]\n");
        assert_eq!(
            output,
            "[Setting]\nautoUpdate=1\nquitWithGame=1\nshowAfterGameQuit=0\ncloseAction=0\n[UPDATER]\nNeedUpdate=0\n"
        );
    }

    #[test]
    fn line_endings_follow_the_file_style() {
        assert_eq!(line_ending("a\r\n", "\n"), "\r\n");
        assert_eq!(line_ending("a\n", "\r\n"), "\n");
        assert_eq!(line_ending("a", "\r\n"), "\r\n");
        assert_eq!(line_ending("a", "\n"), "\n");
    }

    #[test]
    fn hide_wine_tray_items_invokes_wine_reg() {
        let dir = TempDir::new("wine-tray");
        let log = dir.join("wine-args.txt");
        let proton_root = dir.join("proton");
        fs::create_dir_all(proton_root.join("files/bin")).expect("create wine dir");
        write_script(
            &proton_root.join("files/bin"),
            "wine",
            &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display()),
        );
        let prefix = dir.join("prefix");

        hide_wine_tray(&proton_root, &prefix).expect("hide tray items");
        let args = fs::read_to_string(&log).expect("read wine log");
        assert!(args.contains("NoTrayItemsDisplay"), "{args}");
        assert!(args.contains("REG_DWORD"), "{args}");
        assert!(
            args.contains(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer"),
            "{args}"
        );

        // 没有 wine 时明确报错.
        let error = hide_wine_tray(&dir.join("empty-proton"), &prefix).expect_err("missing wine");
        assert!(error.contains("未找到"), "{error}");

        // wine 返回非零时把状态带出来.
        let failing_root = dir.join("failing-proton");
        fs::create_dir_all(failing_root.join("files/bin")).expect("create wine dir");
        write_script(
            &failing_root.join("files/bin"),
            "wine",
            "#!/bin/sh\nexit 7\n",
        );
        let error = hide_wine_tray(&failing_root, &prefix).expect_err("wine failed");
        assert!(error.contains("wine reg 返回"), "{error}");
    }
}
