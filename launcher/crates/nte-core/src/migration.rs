//! 游戏安装目录迁移: 文件复制、校验与绝对路径重绑定.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::game_profile::GameProfile;
use crate::{Error, Result};

/// 源目录中不随迁移复制到其他文件系统的 NTFS 兼容属性.
const NON_PORTABLE_XATTRS: [&str; 3] = ["security.NTACL", "system.ntfs_acl", "user.DOSATTRIB"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub files: u64,
    pub directories: u64,
    pub symlinks: u64,
    pub bytes: u64,
    pub hardlinks: u64,
    pub special: u64,
    pub foreign_owners: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub current: String,
    pub files: u64,
    pub total_files: u64,
    pub bytes: u64,
    pub total_bytes: u64,
}

/// 统计安装树; 不跟随符号链接.
pub fn scan(root: &Path) -> Result<Scan> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Integrity(format!(
            "安装目录不是普通目录: {}",
            root.display()
        )));
    }
    let mut scan = Scan::default();
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.gid() != unsafe { libc::getegid() }
    {
        scan.foreign_owners = 1;
    }
    scan_dir(root, &mut scan)?;
    Ok(scan)
}

fn scan_dir(path: &Path, scan: &mut Scan) -> Result<()> {
    let mut entries = fs::read_dir(path)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            scan.symlinks = scan.symlinks.saturating_add(1);
        } else if metadata.is_dir() {
            scan.directories = scan.directories.saturating_add(1);
            scan_dir(&path, scan)?;
        } else if metadata.is_file() {
            scan.files = scan.files.saturating_add(1);
            scan.bytes = scan.bytes.saturating_add(metadata.len());
            if metadata.nlink() > 1 {
                scan.hardlinks = scan.hardlinks.saturating_add(1);
            }
        } else {
            scan.special = scan.special.saturating_add(1);
        }
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.gid() != unsafe { libc::getegid() }
        {
            scan.foreign_owners = scan.foreign_owners.saturating_add(1);
        }
    }
    Ok(())
}

/// 同文件系统直接改名, 跨文件系统复制到目标旁的临时目录后再改名.
pub fn copy_tree(
    source: &Path,
    target: &Path,
    scan: &Scan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    copy_tree_to(source, target, target, scan, cancel, progress)
}

/// `write_root` 是实际写入目录, `link_root` 是发布后的最终目录.
pub fn copy_tree_to(
    source: &Path,
    write_root: &Path,
    link_root: &Path,
    scan: &Scan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let mut state = CopyState {
        source_root: source.to_path_buf(),
        target_root: link_root.to_path_buf(),
        progress: Progress {
            total_files: scan.files.saturating_add(scan.symlinks),
            total_bytes: scan.bytes,
            ..Progress::default()
        },
        cancel,
        progress_callback: progress,
    };
    state.copy_dir(source, write_root)?;
    Ok(())
}

struct CopyState<'a> {
    source_root: PathBuf,
    target_root: PathBuf,
    progress: Progress,
    cancel: &'a AtomicBool,
    progress_callback: &'a mut dyn FnMut(Progress),
}

impl CopyState<'_> {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn emit(&mut self, current: &Path) {
        self.progress.current = current.display().to_string();
        (self.progress_callback)(self.progress.clone());
    }

    fn copy_dir(&mut self, source: &Path, target: &Path) -> Result<()> {
        self.check()?;
        let metadata = fs::symlink_metadata(source)?;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(target)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(error)
                }
            })?;
        let mut entries = fs::read_dir(source)?.collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            self.check()?;
            let from = entry.path();
            let to = target.join(entry.file_name());
            let metadata = fs::symlink_metadata(&from)?;
            if metadata.file_type().is_symlink() {
                self.copy_link(&from, &to)?;
            } else if metadata.is_dir() {
                self.copy_dir(&from, &to)?;
            } else if metadata.is_file() {
                self.copy_file(&from, &to)?;
            } else {
                return Err(Error::Unsupported(format!(
                    "安装目录包含特殊文件: {}",
                    from.display()
                )));
            }
        }
        copy_xattrs(source, target)?;
        set_mode_and_time(target, &metadata)?;
        sync_dir(target)?;
        Ok(())
    }

    fn copy_file(&mut self, source: &Path, target: &Path) -> Result<()> {
        self.check()?;
        let metadata = fs::symlink_metadata(source)?;
        if metadata.nlink() > 1 {
            return Err(Error::Unsupported(format!(
                "安装目录包含硬链接: {}",
                source.display()
            )));
        }
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(target)?;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            self.check()?;
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            self.progress.bytes = self.progress.bytes.saturating_add(read as u64);
            self.emit(source);
        }
        output.sync_all()?;
        copy_xattrs(source, target)?;
        set_mode_and_time(target, &metadata)?;
        self.progress.files = self.progress.files.saturating_add(1);
        self.emit(source);
        Ok(())
    }

    fn copy_link(&mut self, source: &Path, target: &Path) -> Result<()> {
        self.check()?;
        let metadata = fs::symlink_metadata(source)?;
        let link = fs::read_link(source)?;
        let link = if link.is_absolute() && link.starts_with(&self.source_root) {
            let relative = link
                .strip_prefix(&self.source_root)
                .map_err(|_| Error::Integrity("链接目标无效".into()))?;
            self.target_root.join(relative)
        } else {
            link
        };
        symlink(&link, target)?;
        set_time(target, &metadata)?;
        self.progress.files = self.progress.files.saturating_add(1);
        self.emit(source);
        Ok(())
    }
}

/// 逐字节比较两棵目录树; 用于跨文件系统复制后的读回校验.
pub fn verify_tree(
    source: &Path,
    target: &Path,
    scan: &Scan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    verify_tree_to(source, target, target, scan, cancel, progress)
}

pub fn verify_tree_to(
    source: &Path,
    target: &Path,
    link_root: &Path,
    scan: &Scan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let mut state = VerifyState {
        source_root: source.to_path_buf(),
        target_root: link_root.to_path_buf(),
        progress: Progress {
            total_files: scan.files.saturating_add(scan.symlinks),
            total_bytes: scan.bytes,
            ..Progress::default()
        },
        cancel,
        progress_callback: progress,
    };
    state.verify_dir(source, target)
}

struct VerifyState<'a> {
    source_root: PathBuf,
    target_root: PathBuf,
    progress: Progress,
    cancel: &'a AtomicBool,
    progress_callback: &'a mut dyn FnMut(Progress),
}

impl VerifyState<'_> {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn emit(&mut self, current: &Path) {
        self.progress.current = current.display().to_string();
        (self.progress_callback)(self.progress.clone());
    }

    fn verify_dir(&mut self, source: &Path, target: &Path) -> Result<()> {
        self.check()?;
        let source_meta = fs::symlink_metadata(source)?;
        let target_meta = fs::symlink_metadata(target)?;
        if !target_meta.is_dir() || target_meta.file_type().is_symlink() {
            return Err(mismatch(source, "目标不是目录"));
        }
        same_mode_time(source, &source_meta, &target_meta)?;
        let names = |path: &Path| -> Result<Vec<std::ffi::OsString>> {
            let mut names = fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            names.sort();
            Ok(names)
        };
        let source_names = names(source)?;
        let target_names = names(target)?;
        if source_names != target_names {
            return Err(mismatch(source, "目录内容不一致"));
        }
        for name in source_names {
            self.check()?;
            let from = source.join(&name);
            let to = target.join(&name);
            let from_meta = fs::symlink_metadata(&from)?;
            if from_meta.file_type().is_symlink() {
                self.verify_link(&from, &to, &from_meta)?;
            } else if from_meta.is_dir() {
                self.verify_dir(&from, &to)?;
            } else if from_meta.is_file() {
                self.verify_file(&from, &to, &from_meta)?;
            } else {
                return Err(Error::Unsupported(format!(
                    "安装目录包含特殊文件: {}",
                    from.display()
                )));
            }
        }
        same_xattrs(source, target)?;
        Ok(())
    }

    fn verify_file(&mut self, source: &Path, target: &Path, metadata: &fs::Metadata) -> Result<()> {
        let target_meta = fs::symlink_metadata(target)?;
        if !target_meta.is_file() {
            return Err(mismatch(source, "目标不是普通文件"));
        }
        if metadata.len() != target_meta.len() {
            return Err(mismatch(source, "文件大小不一致"));
        }
        same_mode_time(source, metadata, &target_meta)?;
        let source_hash = hash_file(source, self.cancel)?;
        let target_hash = hash_file(target, self.cancel)?;
        if source_hash != target_hash {
            return Err(mismatch(source, "文件内容不一致"));
        }
        same_xattrs(source, target)?;
        self.progress.files = self.progress.files.saturating_add(1);
        self.progress.bytes = self.progress.bytes.saturating_add(metadata.len());
        self.emit(source);
        Ok(())
    }

    fn verify_link(&mut self, source: &Path, target: &Path, metadata: &fs::Metadata) -> Result<()> {
        let target_meta = fs::symlink_metadata(target)?;
        if !target_meta.file_type().is_symlink() {
            return Err(mismatch(source, "目标不是符号链接"));
        }
        same_mode_time(source, metadata, &target_meta)?;
        let source_link = fs::read_link(source)?;
        let target_link = fs::read_link(target)?;
        let expected = if source_link.is_absolute() && source_link.starts_with(&self.source_root) {
            let relative = source_link
                .strip_prefix(&self.source_root)
                .map_err(|_| Error::Integrity("链接目标无效".into()))?;
            self.target_root.join(relative)
        } else {
            source_link.clone()
        };
        if expected != target_link {
            return Err(mismatch(source, "符号链接目标不一致"));
        }
        self.progress.files = self.progress.files.saturating_add(1);
        self.emit(source);
        Ok(())
    }
}

fn mismatch(path: &Path, message: &str) -> Error {
    Error::Integrity(format!("{message}: {}", path.display()))
}

fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().into())
}

fn same_mode_time(source: &Path, source_meta: &fs::Metadata, target: &fs::Metadata) -> Result<()> {
    if source_meta.mode() & 0o7777 != target.mode() & 0o7777 {
        return Err(mismatch(source, "权限不一致"));
    }
    if source_meta.mtime() != target.mtime() || source_meta.mtime_nsec() != target.mtime_nsec() {
        return Err(mismatch(source, "修改时间不一致"));
    }
    Ok(())
}

fn set_mode_and_time(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(metadata.mode() & 0o7777))?;
    set_time(path, metadata)
}

fn set_time(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::Integrity("路径包含空字节".into()))?;
    let times = [
        libc::timespec {
            tv_sec: metadata.atime(),
            tv_nsec: metadata.atime_nsec(),
        },
        libc::timespec {
            tv_sec: metadata.mtime(),
            tv_nsec: metadata.mtime_nsec(),
        },
    ];
    if unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn xattr_names(path: &Path) -> Result<Vec<CString>> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::Integrity("路径包含空字节".into()))?;
    let size = unsafe { libc::llistxattr(path.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::ENOTSUP) => Ok(Vec::new()),
            _ => Err(error.into()),
        };
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut buffer = vec![0u8; size as usize];
    let read = unsafe { libc::llistxattr(path.as_ptr(), buffer.as_mut_ptr().cast(), buffer.len()) };
    if read < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    buffer.truncate(read as usize);
    Ok(buffer
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| CString::new(name).expect("xattr 名称不含空字节"))
        .collect())
}

fn xattr_value(path: &Path, name: &CString) -> Result<Vec<u8>> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::Integrity("路径包含空字节".into()))?;
    let size = unsafe { libc::lgetxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut buffer = vec![0u8; size as usize];
    if size == 0 {
        return Ok(buffer);
    }
    let read = unsafe {
        libc::lgetxattr(
            path.as_ptr(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if read < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    buffer.truncate(read as usize);
    Ok(buffer)
}

fn set_xattr_value(path: &Path, name: &CString, value: &[u8]) -> Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::Integrity("路径包含空字节".into()))?;
    if unsafe {
        libc::lsetxattr(
            path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

/// 除 NTFS 专属属性外保留全部扩展属性; 目标不支持时失败, 不静默丢失.
fn copy_xattrs(source: &Path, target: &Path) -> Result<()> {
    for name in xattr_names(source)? {
        let name_text = name.to_string_lossy();
        if NON_PORTABLE_XATTRS.contains(&name_text.as_ref()) {
            continue;
        }
        let value = xattr_value(source, &name)?;
        set_xattr_value(target, &name, &value)?;
    }
    Ok(())
}

fn same_xattrs(source: &Path, target: &Path) -> Result<()> {
    let mut source_names = xattr_names(source)?;
    source_names.retain(|name| !NON_PORTABLE_XATTRS.contains(&name.to_string_lossy().as_ref()));
    let mut target_names = xattr_names(target)?;
    target_names.retain(|name| !NON_PORTABLE_XATTRS.contains(&name.to_string_lossy().as_ref()));
    source_names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    target_names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    if source_names != target_names {
        return Err(mismatch(source, "扩展属性不一致"));
    }
    for name in source_names {
        if xattr_value(source, &name)? != xattr_value(target, &name)? {
            return Err(mismatch(source, "扩展属性内容不一致"));
        }
    }
    Ok(())
}

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

/// 更新 prefix 内的游戏目录链接.
pub fn rewrite_prefix(prefix: &Path, old: &Path, new: &Path) -> Result<bool> {
    let link = prefix
        .join("drive_c")
        .join("Program Files (x86)")
        .join("Neverness To Everness");
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
    profile: &GameProfile,
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
    if let (Some(layout), Some(recipe)) = (profile.layout(), profile.launch()) {
        let game_config = layout
            .game_exe_path(install_root)
            .with_file_name(recipe.launcher_config);
        if game_config.is_file() {
            let text = fs::read_to_string(&game_config)?;
            if text.contains(&old_win) {
                write_private(&game_config, text.replace(&old_win, &new_win).as_bytes())?;
            }
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

/// 返回挂载在 root 之下的挂载点; 用于拒绝重命名或删除时跨越文件系统.
pub fn mount_points_under(root: &Path) -> Result<Vec<PathBuf>> {
    let text = match fs::read_to_string("/proc/self/mountinfo") {
        Ok(text) => text,
        Err(_) => return Ok(Vec::new()),
    };
    let mut found = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(raw) = fields.get(4) else {
            continue;
        };
        let mount = decode_mount_path(raw);
        if mount.starts_with(root) {
            found.push(mount);
        }
    }
    Ok(found)
}

fn decode_mount_path(raw: &str) -> PathBuf {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let digits: String = chars.by_ref().take(3).collect();
        if let Ok(value) = u8::from_str_radix(&digits, 8) {
            out.push(value as char);
        }
    }
    PathBuf::from(out)
}

/// 删除旧安装目录; 调用方需先确认根目录身份.
pub fn remove_tree(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Integrity("旧目录不是普通目录".into()));
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

/// 比较两个路径, 优先使用规范化结果; 目标不存在时退回绝对路径比较.
pub fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => absolute(left) == absolute(right),
    }
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// 检查目标路径是否包含 Windows 不接受的字符.
pub fn valid_windows_root(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    if text.is_empty() || text.contains('\0') {
        return false;
    }
    text.trim_start_matches('/').split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.chars().any(|ch| {
                ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
            })
            && !part.ends_with(' ')
            && !part.ends_with('.')
    })
}

/// 列出目录下的所有符号链接; 仅用于迁移后的残留检查.
pub fn symlink_targets(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut links = Vec::new();
    collect_links(root, &mut links)?;
    Ok(links)
}

fn collect_links(root: &Path, links: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            links.push((entry.path(), fs::read_link(entry.path())?));
        } else if metadata.is_dir() {
            collect_links(&entry.path(), links)?;
        }
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

    #[test]
    fn scan_counts_files_and_directories() {
        let dir = fixture("migration-scan");
        let scan = scan(&dir.join("source")).unwrap();
        assert_eq!(scan.files, 3);
        assert_eq!(scan.directories, 2);
        assert_eq!(scan.bytes, 5 + 4 + 20);
        assert_eq!(scan.hardlinks, 0);
        assert_eq!(scan.special, 0);
        assert_eq!(scan.foreign_owners, 0);
    }

    #[test]
    fn copy_verify_and_remove_round_trip() {
        let dir = fixture("migration-copy");
        let source = dir.join("source");
        let target = dir.join("target");
        let scan = scan(&source).unwrap();
        let cancel = AtomicBool::new(false);
        copy_tree(&source, &target, &scan, &cancel, &mut |_| {}).unwrap();
        verify_tree(&source, &target, &scan, &cancel, &mut |_| {}).unwrap();
        assert_eq!(dir.read_text("target/game/a.pak"), "alpha");
        remove_tree(&target).unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn symlinks_are_preserved_and_internal_targets_are_rewritten() {
        let dir = fixture("migration-links");
        let source = dir.join("source");
        let target = dir.join("target");
        symlink(source.join("game/a.pak"), source.join("game/link")).unwrap();
        let scan = scan(&source).unwrap();
        let cancel = AtomicBool::new(false);
        copy_tree(&source, &target, &scan, &cancel, &mut |_| {}).unwrap();
        verify_tree(&source, &target, &scan, &cancel, &mut |_| {}).unwrap();
        assert_eq!(
            fs::read_link(target.join("game/link")).unwrap(),
            target.join("game/a.pak")
        );
    }

    #[test]
    fn read_only_files_keep_user_xattrs() {
        let dir = fixture("migration-xattrs-readonly");
        let source = dir.join("source");
        let file = source.join("game/a.pak");
        let name = CString::new("user.oddity.test").unwrap();
        set_xattr_value(&file, &name, b"kept").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
        let scan = scan(&source).unwrap();
        let target = dir.join("target");
        copy_tree(
            &source,
            &target,
            &scan,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        verify_tree(
            &source,
            &target,
            &scan,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            xattr_value(&target.join("game/a.pak"), &name).unwrap(),
            b"kept"
        );
    }

    #[test]
    fn staged_copies_link_to_the_final_root() {
        let dir = fixture("migration-staging-links");
        let source = dir.join("source");
        let staging = dir.join("staging");
        let target = dir.join("target");
        symlink(source.join("game/a.pak"), source.join("game/link")).unwrap();
        let scan = scan(&source).unwrap();
        let cancel = AtomicBool::new(false);
        copy_tree_to(&source, &staging, &target, &scan, &cancel, &mut |_| {}).unwrap();
        verify_tree_to(&source, &staging, &target, &scan, &cancel, &mut |_| {}).unwrap();
        assert_eq!(
            fs::read_link(staging.join("game/link")).unwrap(),
            target.join("game/a.pak")
        );
        fs::rename(&staging, &target).unwrap();
        assert!(target.join("game/a.pak").is_file());
    }

    #[test]
    fn registry_and_prefix_are_rebound() {
        let dir = fixture("migration-rebind");
        let prefix = dir.join("prefix");
        fs::create_dir_all(prefix.join("drive_c/Program Files (x86)")).unwrap();
        symlink(
            dir.join("source"),
            prefix.join("drive_c/Program Files (x86)/Neverness To Everness"),
        )
        .unwrap();
        let old_wine = wine_path(&dir.join("source")).replace('\\', "\\\\");
        dir.write(
            "prefix/system.reg",
            format!("\"ImagePath\"=str(2):\"{old_wine}\\\\NTELauncher\\\\driver\\\\x.sys\""),
        );
        rewrite_prefix(&prefix, &dir.join("source"), &dir.join("target")).unwrap();
        rewrite_registry(&prefix, &dir.join("source"), &dir.join("target")).unwrap();
        assert_eq!(
            fs::read_link(prefix.join("drive_c/Program Files (x86)/Neverness To Everness"))
                .unwrap(),
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
            "source/NTELauncher/UserData/Config/Config.ini",
            format!("[Game]\nACE={encoded}\nName=value\n"),
        );
        rewrite_official(
            &dir.join("source"),
            "NTELauncher",
            crate::game_profile::GameId::Nte.profile(),
            Path::new(old),
            Path::new(new),
        )
        .unwrap();
        let text = dir.read_text("source/NTELauncher/UserData/Config/Config.ini");
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
    fn windows_path_rejects_invalid_components() {
        assert!(valid_windows_root(Path::new("/games/NTE")));
        assert!(!valid_windows_root(Path::new("/games/NTE:backup")));
        assert!(!valid_windows_root(Path::new("/games/NTE/bad?")));
    }

    #[test]
    fn scan_rejects_non_directories_and_counts_special_entries() {
        let dir = fixture("migration-scan-errors");
        let file = dir.join("source/game/a.pak");
        assert!(scan(&file).is_err());
        let link = dir.join("source/game/link");
        symlink("a.pak", &link).unwrap();
        assert!(scan(&link).is_err());
        let hard = dir.join("source/game/hard.pak");
        std::fs::hard_link(&file, &hard).unwrap();
        let fifo = dir.join("source/game/pipe");
        let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        let result = scan(&dir.join("source"));
        assert_eq!(result.unwrap().special, 1);
    }

    #[test]
    fn copy_rejects_cancel_hardlinks_and_special_files() {
        let dir = fixture("migration-copy-errors");
        let source = dir.join("source");
        let scan_value = scan(&source).unwrap();
        let target = dir.join("target");
        assert!(matches!(
            copy_tree(
                &source,
                &target,
                &scan_value,
                &AtomicBool::new(true),
                &mut |_| {}
            ),
            Err(Error::Cancelled)
        ));
        let hard = source.join("game/hard.pak");
        std::fs::hard_link(source.join("game/a.pak"), &hard).unwrap();
        let scan_value = scan(&source).unwrap();
        let target = dir.join("target-hard");
        assert!(copy_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        let fifo = source.join("game/pipe");
        let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        let scan_value = scan(&source).unwrap();
        let target = dir.join("target-special");
        assert!(copy_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
    }

    #[test]
    fn verify_detects_file_tree_link_and_mode_mismatches() {
        let dir = fixture("migration-verify-errors");
        let source = dir.join("source");
        let target = dir.join("target");
        let scan_value = scan(&source).unwrap();
        copy_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        std::fs::write(target.join("game/a.pak"), "changed").unwrap();
        assert!(verify_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        std::fs::write(target.join("extra"), "x").unwrap();
        assert!(verify_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        std::fs::remove_file(target.join("extra")).unwrap();
        std::fs::set_permissions(
            target.join("game/a.pak"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(verify_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        assert!(matches!(
            verify_tree(
                &source,
                &target,
                &scan_value,
                &AtomicBool::new(true),
                &mut |_| {}
            ),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn verify_detects_directory_and_link_mismatches() {
        let dir = fixture("migration-verify-links");
        let source = dir.join("source");
        let target = dir.join("target");
        symlink("a.pak", source.join("game/link")).unwrap();
        let scan_value = scan(&source).unwrap();
        copy_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        std::fs::remove_file(target.join("game/link")).unwrap();
        symlink("other", target.join("game/link")).unwrap();
        assert!(verify_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        std::fs::remove_file(target.join("game/link")).unwrap();
        std::fs::write(target.join("game/link"), "not a link").unwrap();
        assert!(verify_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
        std::fs::remove_file(target.join("game/link")).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
        std::fs::write(&target, "file").unwrap();
        assert!(verify_tree(
            &source,
            &target,
            &scan_value,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err());
    }

    #[test]
    fn remove_tree_rejects_files_and_links() {
        let dir = fixture("migration-remove-errors");
        let file = dir.join("source/game/a.pak");
        assert!(remove_tree(&file).is_err());
        let link = dir.join("source/game/link");
        symlink("a.pak", &link).unwrap();
        assert!(remove_tree(&link).is_err());
    }

    #[test]
    fn rewrite_prefix_handles_relative_unknown_and_already_new_links() {
        let dir = fixture("migration-prefix-errors");
        let prefix = dir.join("prefix");
        let link = prefix.join("drive_c/Program Files (x86)/Neverness To Everness");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::write(&link, "file").unwrap();
        assert!(rewrite_prefix(&prefix, Path::new("/old"), Path::new("/new")).is_err());
        std::fs::remove_file(&link).unwrap();
        symlink("/unknown", &link).unwrap();
        assert!(rewrite_prefix(&prefix, Path::new("/old"), Path::new("/new")).is_err());
        std::fs::remove_file(&link).unwrap();
        symlink("../old", &link).unwrap();
        assert!(rewrite_prefix(&prefix, Path::new("/old"), Path::new("/new")).is_err());
        std::fs::remove_file(&link).unwrap();
        symlink("/new", &link).unwrap();
        assert!(!rewrite_prefix(&prefix, Path::new("/old"), Path::new("/new")).unwrap());
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
        let path = root.join("Client/WindowsNoEditor/HT/Binaries/Win64/GameLauncher.config");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("path={}", wine_path(Path::new(old)))).unwrap();
        rewrite_official(
            &root,
            "NTELauncher",
            crate::game_profile::GameId::Nte.profile(),
            Path::new(old),
            Path::new(new),
        )
        .unwrap();
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains(&wine_path(Path::new(new))));
    }

    #[test]
    fn path_helpers_cover_invalid_and_relative_cases() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(
            absolute(Path::new("relative")),
            std::env::current_dir().unwrap().join("relative")
        );
        assert!(!valid_windows_root(Path::new("")));
        assert!(!valid_windows_root(Path::new("/bad:name")));
        assert!(!valid_windows_root(Path::new("/bad ")));
        assert!(!valid_windows_root(Path::new("/bad.")));
        assert!(!valid_windows_root(Path::new("/bad//name")));
        assert!(!valid_windows_root(&PathBuf::from(OsString::from_vec(
            vec![0xff]
        ))));
        assert_eq!(decode_mount_path(r"\999"), PathBuf::from(""));
    }

    #[test]
    fn mount_paths_and_symlink_targets_are_readable() {
        let dir = fixture("migration-mount-links");
        let link = dir.join("source/game/link");
        symlink("a.pak", &link).unwrap();
        assert_eq!(symlink_targets(&dir.join("source")).unwrap().len(), 1);
        assert_eq!(
            decode_mount_path(r"/tmp/ space"),
            PathBuf::from("/tmp/ space")
        );
        assert!(!mount_points_under(Path::new("/")).unwrap().is_empty());
    }
}
