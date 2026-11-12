use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    name: String,
    path: String,
    directory: bool,
}
#[derive(Serialize)]
pub struct Listing {
    path: String,
    parent: Option<String>,
    home: String,
    entries: Vec<Entry>,
}

pub fn browse_files(path: Option<String>) -> Result<Listing, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("无法定位主目录.")?;
    let requested = path
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| home.display().to_string());
    let requested = if requested == "~" {
        home.clone()
    } else if let Some(suffix) = requested.strip_prefix("~/") {
        home.join(suffix)
    } else {
        PathBuf::from(requested)
    };
    if !requested.is_absolute() {
        return Err("请选择绝对路径.".into());
    }
    let path = requested.canonicalize().map_err(|e| e.to_string())?;
    let mut entries = std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let metadata = entry.path().metadata().ok()?;
            (metadata.is_dir() || metadata.is_file()).then(|| Entry {
                name,
                path: entry.path().display().to_string(),
                directory: metadata.is_dir(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing {
        path: path.display().to_string(),
        parent: path.parent().map(|p| p.display().to_string()),
        home: home.display().to_string(),
        entries,
    })
}

/// 只允许打开 http(s) 链接.
pub(crate) fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只允许打开 http(s) 链接".into());
    }
    Command::new("xdg-open")
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("打开链接失败：{e}"))?;
    Ok(())
}

pub(crate) fn open_directory_impl(path: &Path) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("读取文件夹失败: {error}"))?;
    if !path.is_dir() {
        return Err("指定路径不是文件夹".into());
    }
    let status = Command::new("xdg-open")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("打开文件夹失败: {error}"))?;
    if !status.success() {
        return Err(format!("系统默认文件管理器返回 {status}"));
    }
    Ok(())
}

pub(crate) async fn open_directory(path: String) -> Result<(), String> {
    crate::service::spawn_blocking(move || open_directory_impl(Path::new(&path)))
        .await
        .map_err(|error| format!("打开文件夹任务失败: {error}"))?
}

#[cfg(test)]
#[path = "tests/gamepad_files.rs"]
mod tests;
