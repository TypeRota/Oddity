//! Wine prefix 内的目录链接与配置路径改写.
//!
//! 链接名与启动器目录名由调用方提供, 平台只负责改写机制.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{symlink, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;

use crate::fs_tree::{same_path, sync_dir};
use crate::{Error, Result};

/// 把 Unix 路径转为 Wine 的 Z: 路径.
pub fn wine_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut out = String::from("Z:");
    if !raw.starts_with('/') {
        out.push('\\');
    }
    for ch in raw.chars() {
        out.push(if ch == '/' { '\\' } else { ch });
    }
    out
}

/// 更新 prefix 内的游戏目录链接; `link_name` 是 drive_c 之下的相对路径.
pub fn rewrite_prefix(prefix: &Path, link_name: &str, old: &Path, new: &Path) -> Result<bool> {
    let link = prefix.join("drive_c").join(link_name);
    let metadata = match fs::symlink_metadata(&link) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(metadata) = &metadata {
        if !metadata.file_type().is_symlink() {
            return Err(Error::Integrity(format!(
                "prefix 游戏目录不是链接: {}",
                link.display()
            )));
        }
        let current = fs::read_link(&link)?;
        let resolved = if current.is_absolute() {
            current
        } else {
            link.parent()
                .ok_or_else(|| Error::Integrity("链接路径无效".into()))?
                .join(current)
        };
        if !same_path(&resolved, old) && !same_path(&resolved, new) {
            return Err(Error::Integrity(format!(
                "prefix 游戏目录指向未知位置: {}",
                link.display()
            )));
        }
        if same_path(&resolved, new) {
            return Ok(false);
        }
    }
    let parent = link
        .parent()
        .ok_or_else(|| Error::Integrity("链接路径无效".into()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".oddity-migration-link-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    symlink(new, &temp)?;
    fs::rename(&temp, &link)?;
    sync_dir(parent)?;
    Ok(true)
}

/// 更新 Wine 注册表文件中指向旧安装目录的绝对路径.
pub fn rewrite_registry(prefix: &Path, old: &Path, new: &Path) -> Result<()> {
    let old = wine_path(old).replace('\\', "\\\\");
    let new = wine_path(new).replace('\\', "\\\\");
    for name in ["system.reg", "user.reg"] {
        let path = prefix.join(name);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !text.contains(&old) {
            continue;
        }
        let updated = text.replace(&old, &new);
        write_private(&path, updated.as_bytes())?;
    }
    Ok(())
}

/// 更新官方启动器配置中的绝对路径; 兼容 base64 编码的路径值.
pub fn rewrite_official(
    install_root: &Path,
    launcher_dir: &str,
    game_config: Option<&Path>,
    old: &Path,
    new: &Path,
) -> Result<()> {
    let old_win = wine_path(old);
    let new_win = wine_path(new);
    let launcher_root = install_root.join(launcher_dir);
    let config = launcher_root
        .join("UserData")
        .join("Config")
        .join("Config.ini");
    if config.is_file() {
        rewrite_ini(&config, &old_win, &new_win)?;
    }
    if let Some(game_config) = game_config.filter(|path| path.is_file()) {
        let text = fs::read_to_string(game_config)?;
        if text.contains(&old_win) {
            write_private(game_config, text.replace(&old_win, &new_win).as_bytes())?;
        }
    }
    Ok(())
}

fn rewrite_ini(path: &Path, old: &str, new: &str) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let mut changed = false;
    let mut output = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, ending) = line
            .strip_suffix("\r\n")
            .map(|body| (body, "\r\n"))
            .or_else(|| line.strip_suffix('\n').map(|body| (body, "\n")))
            .unwrap_or((line, ""));
        let Some((key, value)) = body.split_once('=') else {
            output.push_str(line);
            continue;
        };
        let mut next = value.to_string();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(value.trim()) {
            if let Ok(decoded) = String::from_utf8(bytes) {
                if decoded.contains(old) {
                    let replaced = decoded.replace(old, new);
                    next = base64::engine::general_purpose::STANDARD.encode(replaced);
                    changed = true;
                }
            }
        }
        if next == value && value.contains(old) {
            next = value.replace(old, new);
            changed = true;
        }
        output.push_str(key);
        output.push('=');
        output.push_str(&next);
        output.push_str(ending);
    }
    if changed {
        write_private(path, output.as_bytes())?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mode = fs::metadata(path)
        .map(|metadata| metadata.mode() & 0o777)
        .unwrap_or(0o600);
    let temp = path.with_file_name(format!(
        ".oddity-migration-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn fixture(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        dir.write("source/game/a.pak", "alpha");
        dir.write("source/game/b.txt", "beta");
        dir.write("source/Config/Config.ini", "[General]\nName=test\n");
        dir
    }

    /// prefix 内的安装链接名, 由调用方从安装域提供.
    const LINK_NAME: &str = "Program Files (x86)/Test Game";

    #[test]
    fn registry_and_prefix_are_rebound() {
        let dir = fixture("migration-rebind");
        let prefix = dir.join("prefix");
        fs::create_dir_all(prefix.join("drive_c/Program Files (x86)")).unwrap();
        symlink(dir.join("source"), prefix.join("drive_c").join(LINK_NAME)).unwrap();
        let old_wine = wine_path(&dir.join("source")).replace('\\', "\\\\");
        dir.write(
            "prefix/system.reg",
            format!("\"ImagePath\"=str(2):\"{old_wine}\\\\TestLauncher\\\\driver\\\\x.sys\""),
        );
        rewrite_prefix(&prefix, LINK_NAME, &dir.join("source"), &dir.join("target")).unwrap();
        rewrite_registry(&prefix, &dir.join("source"), &dir.join("target")).unwrap();
        assert_eq!(
            fs::read_link(prefix.join("drive_c").join(LINK_NAME)).unwrap(),
            dir.join("target")
        );
        let new_wine = wine_path(&dir.join("target")).replace('\\', "\\\\");
        assert!(dir.read_text("prefix/system.reg").contains(&new_wine));
    }

    #[test]
    fn official_config_rewrites_base64_paths() {
        let dir = fixture("migration-config");
        let old = "/games/old";
        let new = "/games/new";
        let encoded =
            base64::engine::general_purpose::STANDARD.encode("Z:\\games\\old\\Client\\ACE");
        dir.write(
            "source/TestLauncher/UserData/Config/Config.ini",
            format!("[Game]\nACE={encoded}\nName=value\n"),
        );
        rewrite_official(
            &dir.join("source"),
            "TestLauncher",
            None,
            Path::new(old),
            Path::new(new),
        )
        .unwrap();
        let text = dir.read_text("source/TestLauncher/UserData/Config/Config.ini");
        let value = text
            .lines()
            .find_map(|line| line.strip_prefix("ACE="))
            .unwrap();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .unwrap(),
        )
        .unwrap();
        assert!(decoded.contains("Z:\\games\\new\\Client\\ACE"));
    }

    #[test]
    fn rewrite_prefix_handles_relative_unknown_and_already_new_links() {
        let dir = fixture("migration-prefix-errors");
        let prefix = dir.join("prefix");
        let link = prefix.join("drive_c").join(LINK_NAME);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::write(&link, "file").unwrap();
        assert!(rewrite_prefix(&prefix, LINK_NAME, Path::new("/old"), Path::new("/new")).is_err());
        std::fs::remove_file(&link).unwrap();
        symlink("/unknown", &link).unwrap();
        assert!(rewrite_prefix(&prefix, LINK_NAME, Path::new("/old"), Path::new("/new")).is_err());
        std::fs::remove_file(&link).unwrap();
        symlink("../old", &link).unwrap();
        assert!(rewrite_prefix(&prefix, LINK_NAME, Path::new("/old"), Path::new("/new")).is_err());
        std::fs::remove_file(&link).unwrap();
        symlink("/new", &link).unwrap();
        assert!(!rewrite_prefix(&prefix, LINK_NAME, Path::new("/old"), Path::new("/new")).unwrap());
    }

    #[test]
    fn rewrite_registry_skips_missing_and_unrelated_files() {
        let dir = fixture("migration-registry-errors");
        let prefix = dir.join("prefix");
        std::fs::create_dir_all(&prefix).unwrap();
        assert!(rewrite_registry(&prefix, Path::new("/old"), Path::new("/new")).is_ok());
        std::fs::write(prefix.join("system.reg"), "nothing").unwrap();
        assert!(rewrite_registry(&prefix, Path::new("/old"), Path::new("/new")).is_ok());
    }

    #[test]
    fn rewrite_official_updates_game_launcher_config() {
        let dir = fixture("migration-official-game-config");
        let root = dir.join("source");
        let old = "/games/old";
        let new = "/games/new";
        let path = root.join("Client/Game/GameLauncher.config");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("path={}", wine_path(Path::new(old)))).unwrap();
        rewrite_official(
            &root,
            "TestLauncher",
            Some(&path),
            Path::new(old),
            Path::new(new),
        )
        .unwrap();
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains(&wine_path(Path::new(new))));
    }
}
