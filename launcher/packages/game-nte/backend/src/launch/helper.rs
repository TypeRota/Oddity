//! 内嵌 helper 的安装与刷新.
//!
//! helper 是 NTE 启动配方的一部分, 宿主只提供数据目录.

use std::fs;
use std::path::{Path, PathBuf};

/// 窗口守卫 DLL; 阻止官方 UI 线程首次映射主窗口.
pub const WINDOW_GUARD_BYTES: &[u8] = include_bytes!("../../helpers/nte-window-guard.dll");
/// 官方启动器前台包装 helper.
pub const LAUNCH_HELPER_BYTES: &[u8] = include_bytes!("../../helpers/nte-launch-helper.exe");
/// 后台静默启动 helper.
pub const SESSION_HELPER_BYTES: &[u8] = include_bytes!("../../helpers/nte-session-helper.exe");
/// 直启模式的独立 SDK 宿主.
pub const SDK_HOST_BYTES: &[u8] = include_bytes!("../../helpers/nte-sdk-host.exe");

/// 把内嵌文件写成指定名字; 内容变化时原子替换.
pub fn ensure_helper(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let path = dir.join(name);
    let current = fs::read(&path).ok();
    if current.as_deref() != Some(bytes) {
        let temp = dir.join(format!("{name}.new"));
        fs::write(&temp, bytes).map_err(|error| format!("write {}: {error}", temp.display()))?;
        fs::rename(&temp, &path).map_err(|error| format!("install {}: {error}", path.display()))?;
    }
    Ok(path)
}

/// 前台官方模式使用的 helper 组合.
pub fn ensure_launch_helper(dir: &Path) -> Result<PathBuf, String> {
    ensure_helper(dir, "nte-window-guard.dll", WINDOW_GUARD_BYTES)?;
    ensure_helper(dir, "nte-launch-helper.exe", LAUNCH_HELPER_BYTES)
}

/// 后台静默模式使用的 helper.
pub fn ensure_session_helper(dir: &Path) -> Result<PathBuf, String> {
    ensure_helper(dir, "nte-session-helper.exe", SESSION_HELPER_BYTES)
}

/// 直启模式使用的 SDK 宿主.
pub fn ensure_sdk_host(dir: &Path) -> Result<PathBuf, String> {
    ensure_helper(dir, "nte-sdk-host.exe", SDK_HOST_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn bundled_helpers_are_pe_files() {
        assert!(LAUNCH_HELPER_BYTES.starts_with(b"MZ") && LAUNCH_HELPER_BYTES.len() > 1024);
        assert!(WINDOW_GUARD_BYTES.starts_with(b"MZ") && WINDOW_GUARD_BYTES.len() > 1024);
        assert!(SESSION_HELPER_BYTES.starts_with(b"MZ") && SESSION_HELPER_BYTES.len() > 1024);
        assert!(SDK_HOST_BYTES.starts_with(b"MZ") && SDK_HOST_BYTES.len() > 1024);
    }

    #[test]
    fn installs_and_refreshes_helpers_atomically() {
        let dir = TempDir::new("helpers");
        let helper_dir = dir.join("helpers");
        let path = ensure_launch_helper(&helper_dir).expect("install helper");
        assert_eq!(fs::read(&path).unwrap(), LAUNCH_HELPER_BYTES);
        assert_eq!(
            fs::read(helper_dir.join("nte-window-guard.dll")).unwrap(),
            WINDOW_GUARD_BYTES
        );
        let stamp = fs::metadata(&path).unwrap().modified().unwrap();
        ensure_launch_helper(&helper_dir).expect("reinstall helper");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp);

        let guard = helper_dir.join("nte-window-guard.dll");
        fs::write(&path, b"MZ stale").unwrap();
        fs::write(&guard, b"MZ stale guard").unwrap();
        ensure_launch_helper(&helper_dir).expect("refresh helper");
        assert_eq!(fs::read(&path).unwrap(), LAUNCH_HELPER_BYTES);
        assert_eq!(fs::read(&guard).unwrap(), WINDOW_GUARD_BYTES);
        assert!(!helper_dir.join("nte-window-guard.dll.new").exists());
        assert!(!helper_dir.join("nte-launch-helper.exe.new").exists());
        assert!(ensure_session_helper(&helper_dir).is_ok());
        assert!(ensure_sdk_host(&helper_dir).is_ok());
    }
}
