//! 宿主用户目录: 数据/配置路径发现与旧项目名的一次性改名.
//!
//! 路径规则只服务宿主自身 (设置, 数据, 日志), 游戏安装位置由包内的安装域
//! 提供.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use crate::settings::UserPaths;

pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// 展开设置里接受的 `~` 形式; 其余路径原样返回, 相对路径保持当前目录语义.
pub(crate) fn expand_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(raw)
}

pub(crate) fn data_base() -> PathBuf {
    UserPaths::discover().data_base
}

pub fn data_dir() -> PathBuf {
    UserPaths::discover().data_dir()
}

pub(crate) fn legacy_data_dir() -> PathBuf {
    data_base().join("TheBetterNTE")
}

pub(crate) fn config_base() -> PathBuf {
    UserPaths::discover().config_base
}

pub(crate) fn config_dir() -> PathBuf {
    UserPaths::discover().config_dir()
}

pub(crate) fn legacy_config_dir() -> PathBuf {
    config_base().join("TheBetterNTE")
}

pub(crate) fn settings_path() -> PathBuf {
    UserPaths::discover().settings()
}

/// 从旧项目名一次性改名. 旧数据目录保留为符号链接, 让写进 Wine prefix 与
/// 官方启动器配置里的绝对路径继续可用.
///
/// 设置里的 `prefix` 故意不改写: 官方 OneSDK 自动登录对 Wine prefix 路径
/// 敏感, 升级用户继续经符号链接使用旧路径, 新安装使用 `oddity` 路径.
pub(crate) fn migrate_legacy_dirs() {
    let old_data = legacy_data_dir();
    let new_data = data_dir();
    if old_data.is_dir() && !new_data.exists() {
        match fs::rename(&old_data, &new_data) {
            Ok(()) => {
                let _ = symlink(&new_data, &old_data);
            }
            Err(error) => {
                eprintln!(
                    "migration: rename {} -> {} failed: {error}",
                    old_data.display(),
                    new_data.display()
                );
            }
        }
    }

    let old_config = legacy_config_dir();
    let new_config = config_dir();
    if old_config.is_dir() && !new_config.exists() {
        match fs::rename(&old_config, &new_config) {
            Ok(()) => {
                let _ = symlink(&new_config, &old_config);
            }
            Err(error) => {
                eprintln!(
                    "migration: rename {} -> {} failed: {error}",
                    old_config.display(),
                    new_config.display()
                );
                if fs::create_dir_all(&new_config).is_ok() {
                    let _ = fs::copy(
                        old_config.join("settings.json"),
                        new_config.join("settings.json"),
                    );
                }
            }
        }
    }
}
