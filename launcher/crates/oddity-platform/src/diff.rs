//! HDiffPatch 差分应用.
//!
//! 只封装库调用与补丁自身的一致性检查; 补丁链, 回退与失败重试属于游戏更新流水线.

use std::fs;
use std::path::Path;

use hdiffpatch_rs::patchers::{DiffFormat, HDiff, PatchOptions};

use crate::{Error, Result};

pub fn apply_hdiff(old: &Path, patch: &Path, out: &Path) -> Result<()> {
    let old_size = fs::metadata(old)?.len();
    if !patch.is_file() {
        return Err(Error::Parse(format!(
            "patch file not found: {}",
            patch.display()
        )));
    }
    let mut patcher = HDiff::new(
        old.to_string_lossy().into_owned(),
        patch.to_string_lossy().into_owned(),
        out.to_string_lossy().into_owned(),
    );
    let info = patcher
        .info()
        .map_err(|error| Error::Parse(format!("read HDIFF13 patch: {error}")))?;
    if info.is_directory
        || info.old_size != old_size
        || !matches!(
            info.format,
            DiffFormat::HDiff13 | DiffFormat::HDiffSf20 | DiffFormat::HDiffW26
        )
    {
        return Err(Error::Integrity(format!(
            "HDiff patch does not match {}",
            old.display()
        )));
    }
    // 大文件启用读/写流水线；小文件单线程更省内存。
    if info.new_size > 1024 * 1024 * 1024 {
        patcher = patcher.with_options(
            PatchOptions::default()
                .with_threads(2)
                .with_memory_budget(128 * 1024 * 1024),
        );
    }
    if out.exists() {
        fs::remove_file(out)?;
    }
    if !patcher.apply() {
        fs::remove_file(out).ok();
        return Err(Error::Parse(format!(
            "HDIFF13 patch failed for {}",
            patch.display()
        )));
    }
    if fs::metadata(out)?.len() != info.new_size {
        fs::remove_file(out).ok();
        return Err(Error::Integrity(format!(
            "HDIFF13 output size mismatch for {}",
            out.display()
        )));
    }
    Ok(())
}
