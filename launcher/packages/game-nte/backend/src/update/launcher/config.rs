//! 官方启动器配置: Config/Config.ini 中与自更新相关的字段.

use std::path::{Path, PathBuf};

use oddity_platform::ini::Ini;
use oddity_platform::Result;

use crate::update::region::{self, Region};

/// The subset of the shipped Config/Config.ini that matters for a native launcher.
#[derive(Debug, Clone)]
pub struct LauncherConfig {
    pub root: PathBuf,
    pub launcher: String,
    pub patcher: String,
    pub client: String,
    pub game_id: i64,
    pub launch_type: i64,
    pub version: String,
    pub build: String,
    pub version_url: String,
    pub backup_version_url: String,
    pub branch: String,
    pub patcher_ignored_sections: Vec<String>,
    pub mutex_name: String,
}

impl LauncherConfig {
    /// 读取官方启动器目录下的 Config/Config.ini.
    ///
    /// 区服由 GameID 决定; 没有填写时按安装目录名区分国服与国际服.
    pub fn load(root: &Path) -> Result<Self> {
        let ini = Ini::load(&root.join("Config").join("Config.ini"))?;
        let game_id = ini.get_int("General", "GameID").unwrap_or(0);
        let distribution = region::region_for_app_id(game_id).unwrap_or_else(|| {
            if root
                .file_name()
                .is_some_and(|name| name == region::profile(Region::Global).launcher_dir)
            {
                Region::Global
            } else {
                Region::Cn
            }
        });
        let profile = region::profile(distribution);
        let version = ini.section("VERSION").cloned().unwrap_or_default();
        let ignored = version
            .get("PATCHER_IGNORED_SECTION")
            .cloned()
            .unwrap_or_default();
        Ok(Self {
            root: root.to_path_buf(),
            launcher: ini
                .get("General", "Launcher")
                .filter(|value| !value.is_empty())
                .unwrap_or(profile.launcher_exe)
                .to_string(),
            patcher: ini
                .get("General", "Patcher")
                .filter(|value| !value.is_empty())
                .unwrap_or(profile.patcher_exe)
                .to_string(),
            client: ini
                .get("General", "Client")
                .filter(|value| !value.is_empty())
                .unwrap_or(profile.client_exe)
                .to_string(),
            game_id: if game_id == 0
                && root
                    .file_name()
                    .is_some_and(|name| name == profile.launcher_dir)
            {
                profile.app_id
            } else {
                game_id
            },
            launch_type: ini.get_int("General", "LaunchType").unwrap_or(0),
            version: version.get("Version").cloned().unwrap_or_default(),
            build: version.get("Build").cloned().unwrap_or_default(),
            version_url: version
                .get("VersionInfoFileURL")
                .cloned()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| profile.version_urls[0].to_string()),
            backup_version_url: ini
                .get("VERSION", "BackupVersionURL")
                .or_else(|| ini.get("UPDATE_CONFIG", "BackupVersionURL"))
                .filter(|value| !value.is_empty())
                .unwrap_or(profile.version_urls[1])
                .to_string(),
            branch: ini
                .get("VERSION", "Branch")
                .or_else(|| ini.get("UPDATE_CONFIG", "Branch"))
                .filter(|value| !value.is_empty())
                .unwrap_or(profile.launcher_branch)
                .to_string(),
            patcher_ignored_sections: ignored
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            mutex_name: version.get("MutexName").cloned().unwrap_or_default(),
        })
    }

    /// Path layout of the installed launcher.
    pub fn launcher_dir(&self) -> &Path {
        &self.root
    }
    pub fn user_config(&self) -> PathBuf {
        self.root.join("UserData").join("Config").join("Config.ini")
    }
    pub fn game_dir(&self) -> PathBuf {
        let install_root = self.install_root();
        region::LAYOUT.content_path(&install_root)
    }
    /// 启动器根目录的父目录即安装根目录.
    pub fn install_root(&self) -> PathBuf {
        self.root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn launcher_config_reads_the_ini_values() {
        let dir = TempDir::new("cfg");
        dir.write(
            "Config/Config.ini",
            r#"[General]
Launcher=MyLauncher.exe
Patcher=MyPatcher.exe
Client=MyGame.exe
GameID=1289
LaunchType=3

[VERSION]
Version=1.4.16
Build=20240101
VersionInfoFileURL=https://cdn.test/Version.ini
BackupVersionURL=https://cdn.test/Backup.ini
Branch=publish_test
MutexName=nte-mutex
PATCHER_IGNORED_SECTION= A , ,B 
"#,
        );
        let cfg = LauncherConfig::load(dir.path()).expect("load config");
        assert_eq!(cfg.launcher, "MyLauncher.exe");
        assert_eq!(cfg.patcher, "MyPatcher.exe");
        assert_eq!(cfg.client, "MyGame.exe");
        assert_eq!(cfg.game_id, 1289);
        assert_eq!(cfg.launch_type, 3);
        assert_eq!(cfg.version, "1.4.16");
        assert_eq!(cfg.build, "20240101");
        assert_eq!(cfg.version_url, "https://cdn.test/Version.ini");
        assert_eq!(cfg.backup_version_url, "https://cdn.test/Backup.ini");
        assert_eq!(cfg.branch, "publish_test");
        assert_eq!(cfg.mutex_name, "nte-mutex");
        // 空白项被丢掉，只留下真正的段名
        assert_eq!(cfg.patcher_ignored_sections, vec!["A", "B"]);
        assert_eq!(cfg.launcher_dir(), dir.path());
        assert_eq!(
            cfg.user_config(),
            dir.path()
                .join("UserData")
                .join("Config")
                .join("Config.ini")
        );
        assert_eq!(
            cfg.game_dir(),
            dir.path().parent().unwrap().join("Client/WindowsNoEditor")
        );
    }

    #[test]
    fn launcher_config_falls_back_to_defaults() {
        let dir = TempDir::new("cfg-default");
        dir.write("Config/Config.ini", "[General]\nGameID=not-a-number\n");
        let cfg = LauncherConfig::load(dir.path()).expect("load config");
        assert_eq!(cfg.launcher, "NTELauncher.exe");
        assert_eq!(cfg.patcher, "NTEUpdate.exe");
        assert_eq!(cfg.client, "NTEGame.exe");
        assert_eq!(cfg.game_id, 0);
        assert_eq!(cfg.launch_type, 0);
        assert_eq!(cfg.version, "");
        assert_eq!(cfg.build, "");
        let cn = region::profile(Region::Cn);
        assert_eq!(cfg.version_url, cn.version_urls[0]);
        assert_eq!(cfg.backup_version_url, cn.version_urls[1]);
        assert_eq!(cfg.branch, "publish_ob");
        assert_eq!(cfg.mutex_name, "");
        assert!(cfg.patcher_ignored_sections.is_empty());
    }

    #[test]
    fn global_defaults_and_update_config_remain_in_the_global_distribution() {
        let dir = TempDir::new("cfg-global");
        dir.write("NTEGlobal/Config/Config.ini", "[General]\nGameID=3000001\n[UPDATE_CONFIG]\nBackupVersionURL=https://backup.test/global.ini\nBranch=global_branch\n");
        let cfg = LauncherConfig::load(&dir.join("NTEGlobal")).unwrap();
        assert_eq!(cfg.launcher, "NTEGlobalLauncher.exe");
        assert_eq!(cfg.patcher, "NTEGlobalUpdate.exe");
        assert_eq!(cfg.client, "NTEGlobalGame.exe");
        let global = region::profile(Region::Global);
        assert_eq!(cfg.version_url, global.version_urls[0]);
        assert_eq!(cfg.backup_version_url, "https://backup.test/global.ini");
        assert_eq!(cfg.branch, "global_branch");
        dir.write("NTEGlobal/Config/Config.ini", "[General]\n");
        let cfg = LauncherConfig::load(&dir.join("NTEGlobal")).unwrap();
        assert_eq!(cfg.game_id, global.app_id);
        assert_eq!(cfg.branch, "publish_PC");
        assert_eq!(cfg.backup_version_url, global.version_urls[1]);
    }

    #[test]
    fn launcher_config_requires_config_ini() {
        let dir = TempDir::new("cfg-missing");
        assert!(LauncherConfig::load(dir.path()).is_err());
    }
}
