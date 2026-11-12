//! 文件安全原语: 同目录临时文件与符号链接检查.

use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Error, Result};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// 独占创建同目录临时文件, 失败或取消时自动清理。
pub struct TemporaryFile {
    pub file: fs::File,
    pub path: PathBuf,
}

impl TemporaryFile {
    pub fn new(destination: &Path) -> Result<Self> {
        Self::create(destination, None)
    }

    /// 创建临时文件时立即设置权限, 避免写入期间以默认权限暴露内容.
    pub fn with_mode(destination: &Path, mode: u32) -> Result<Self> {
        Self::create(destination, Some(mode))
    }

    fn create(destination: &Path, mode: Option<u32>) -> Result<Self> {
        for _ in 0..64 {
            let mut name = destination.as_os_str().to_os_string();
            name.push(format!(
                ".oddity-new-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            let path = PathBuf::from(name);
            let mut options = fs::OpenOptions::new();
            options.write(true).read(true).create_new(true);
            if let Some(mode) = mode {
                options.mode(mode);
            }
            match options.open(&path) {
                Ok(file) => return Ok(Self { file, path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::Integrity("无法创建独立的安装临时文件".into()))
    }

    pub fn commit(self, destination: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.file.sync_all()?;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        fs::rename(&self.path, destination)?;
        if let Some(parent) = destination.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 逐段检查安装路径中已存在的符号链接, 防止写入逃逸安装目录。
pub fn ensure_no_symlink(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(Error::Integrity("安装路径超出安装目录".into()));
        }
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::Integrity(format!(
                    "安装路径包含符号链接: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Ok(())
}

/// 校验并规范化清单里的相对路径, 拒绝空段, 上下级跳转与盘符.
pub fn relative_path(value: &str) -> Result<PathBuf> {
    let normalized = value.trim_start_matches('/').replace('\\', "/");
    let mut path = PathBuf::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return Err(Error::Parse(format!("无效的清单路径: {value}")));
        }
        path.push(part);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn relative_path_normalizes_separators_and_rejects_escapes() {
        assert_eq!(relative_path("/a/b").unwrap(), PathBuf::from("a/b"));
        assert_eq!(relative_path("a\\b").unwrap(), PathBuf::from("a/b"));
        for value in ["", "/", "a//b", "a/./b", "a/../b", "C:/x", "a:b"] {
            let error = relative_path(value).unwrap_err();
            assert!(matches!(error, Error::Parse(_)), "{value}");
            assert!(
                error
                    .to_string()
                    .contains(&format!("无效的清单路径: {value}")),
                "{value}"
            );
        }
    }

    #[test]
    fn private_temporary_file_is_created_with_restricted_mode() {
        let dir = TempDir::new("temporary-file-mode");
        let target = dir.join("snapshot.json");
        let temp = TemporaryFile::with_mode(&target, 0o600).unwrap();
        let mode = fs::metadata(&temp.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

/// Convert a Unix absolute path to a Wine Z: path.
///
/// Z: maps / in the standard prefix and avoids drive-letter assumptions for
/// removable mounts.
pub fn windows_path(path: &Path) -> String {
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

#[cfg(test)]
mod windows_path_tests {
    use super::windows_path;
    use std::path::Path;

    #[test]
    fn windows_path_uses_z_drive() {
        assert_eq!(
            windows_path(Path::new("/mnt/windows/Program Files (x86)/NTE/HTGame.exe")),
            "Z:\\mnt\\windows\\Program Files (x86)\\NTE\\HTGame.exe"
        );
        assert_eq!(windows_path(Path::new("/")), "Z:\\");
        assert_eq!(
            windows_path(Path::new("relative/path")),
            "Z:\\relative\\path"
        );
    }
}
