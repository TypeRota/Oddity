//! 官方启动器自更新引擎: 配置, 清单, 版本, 文件与安装规划.

pub mod config;
pub mod files;
pub mod install;
pub mod manifest;
pub mod version_ini;

pub use config::LauncherConfig;
pub use files::{
    apply_file, apply_file_resumable, apply_file_resumable_limited, check, check_cancellable,
    CheckReport,
};
pub use install::{plan_launcher, plan_launcher_cancellable, LauncherPlan};
pub use manifest::{Manifest, ManifestFile};
pub use version_ini::{fetch as fetch_version, VersionInfo};
