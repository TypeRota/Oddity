//! NTE 启动配方: 游戏参数, 官方组件文件名, prefix 链接与 umu/Proton 启动环境.
//!
//! 配方与区服无关, 描述官方启动器拉起游戏时的约定; 命令构造与进程监视在
//! provider 中消费这份配方.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

/// 官方启动器的前台启动参数.
pub(crate) const OFFICIAL_ARGS: [&str; 2] = ["/launcher", "/directly"];

/// 与地区无关的启动配方.
pub(crate) struct LaunchRecipe {
    /// 传给游戏可执行文件的参数.
    pub(crate) args: &'static [&'static str],
    /// 官方启动器写入 SDK 路径的配置文件名.
    pub(crate) launcher_config: &'static str,
    /// 官方启动器加载的 SDK DLL 文件名.
    pub(crate) sdk_dll: &'static str,
    /// 直接启动所需的 OneSDK DLL 文件名.
    pub(crate) one_sdk_dll: &'static str,
    /// 游戏目录下需要可写的服务器列表缓存名.
    pub(crate) serverlist: &'static str,
    /// Wine prefix 中指向安装目录的链接, 相对 `drive_c`.
    pub(crate) prefix_install_link: &'static str,
    /// `ArcGameSDKSharePath` 指向的目录名, 相对启动器根目录.
    pub(crate) sdk_share_dir: &'static str,
}

impl LaunchRecipe {
    /// 助手进程按单行环境变量接收参数.
    pub(crate) fn args_line(&self) -> String {
        self.args.join(" ")
    }

    /// 让 Wine prefix 里的安装目录链接指向当前安装根.
    pub(crate) fn ensure_install_link(
        &self,
        prefix: &Path,
        install_dir: &Path,
    ) -> Result<(), String> {
        let drive_c = prefix.join("drive_c");
        if !drive_c.is_dir() {
            return Err(format!(
                "Wine prefix 尚未初始化：{}（先运行一次 Proton 以创建 prefix）",
                prefix.display()
            ));
        }
        let link = drive_c.join(self.prefix_install_link);
        if let Ok(meta) = link.symlink_metadata() {
            if meta.file_type().is_symlink() {
                let good = fs::read_link(&link)
                    .map(|target| target == install_dir)
                    .unwrap_or(false);
                if good {
                    return Ok(());
                }
                fs::remove_file(&link)
                    .map_err(|e| format!("remove stale {}: {e}", link.display()))?;
            } else {
                return Ok(());
            }
        }
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        symlink(install_dir, &link).map_err(|e| format!("symlink {}: {e}", link.display()))
    }

    /// 官方组件需要写入 SDK 路径与服务器缓存, 只读文件会让游戏在内部失败.
    pub(crate) fn check_runtime_config_writable(&self, game_exe: &Path) -> Result<(), String> {
        let bin = game_exe
            .parent()
            .ok_or_else(|| "游戏可执行文件目录异常".to_string())?;
        for (path, mode) in [
            (bin.to_path_buf(), libc::W_OK | libc::X_OK),
            (bin.join(self.launcher_config), libc::W_OK),
            (bin.join(self.serverlist), libc::W_OK | libc::X_OK),
        ] {
            match fs::metadata(&path) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!("读取游戏目录权限失败: {}: {error}", path.display()))
                }
            }
            let raw = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| "游戏路径含有无效字符".to_string())?;
            if unsafe { libc::access(raw.as_ptr(), mode) } != 0 {
                return Err(format!(
                    "游戏运行配置不可写: {}. 请检查此路径的所有者和写入权限后重试.",
                    path.display()
                ));
            }
        }
        Ok(())
    }
}

/// 《异环》的启动配方.
pub(crate) static NTE_LAUNCH: LaunchRecipe = LaunchRecipe {
    // UE map 与额外参数, 取自官方启动器拉起游戏时的 NTEGame.log.
    args: &[
        "/Game/LoginAndCreate/Map/Updater/Updater_P",
        "-SAVEWINPOS=1",
    ],
    // 官方启动器在拉起 HTGame.exe 前把 WPGameClientSDK_64.dll 的路径写进这个文件.
    launcher_config: "GameLauncher.config",
    sdk_dll: "WPGameClientSDK_64.dll",
    one_sdk_dll: "WPOneSDK_64.dll",
    serverlist: "patchersdk_serverlist",
    prefix_install_link: "Program Files (x86)/Neverness To Everness",
    sdk_share_dir: "WPGameClientSDK",
};

/// 桌面 Wayland 使用原生驱动, Gamescope 使用它管理的 Xwayland.
pub(crate) fn configure_display(command: &mut Command, wayland_available: bool, gamescope: bool) {
    let enabled = if wayland_available && !gamescope {
        "1"
    } else {
        "0"
    };
    // 两个别名均显式赋值, 防止 Proton 从用户配置中补入相反的值.
    command
        .env("PROTON_ENABLE_WAYLAND", enabled)
        .env("PROTON_USE_WAYLAND", enabled);
}

/// 游戏与官方 SDK 直连, 不继承启动器下载请求使用的代理.
pub(crate) fn configure_network(command: &mut Command) {
    let keys: Vec<_> = std::env::vars_os()
        .map(|(key, _)| key)
        .chain(command.get_envs().map(|(key, _)| key.to_os_string()))
        .collect();
    for key in keys {
        let name = key.to_string_lossy().to_ascii_lowercase();
        if name == "proxy" || name.ends_with("_proxy") {
            command.env_remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn recipe_keeps_the_launcher_contract() {
        assert_eq!(
            NTE_LAUNCH.args,
            [
                "/Game/LoginAndCreate/Map/Updater/Updater_P",
                "-SAVEWINPOS=1"
            ]
        );
        assert_eq!(
            NTE_LAUNCH.args_line(),
            "/Game/LoginAndCreate/Map/Updater/Updater_P -SAVEWINPOS=1"
        );
        assert_eq!(NTE_LAUNCH.launcher_config, "GameLauncher.config");
        assert_eq!(NTE_LAUNCH.sdk_dll, "WPGameClientSDK_64.dll");
        assert_eq!(NTE_LAUNCH.one_sdk_dll, "WPOneSDK_64.dll");
        assert_eq!(NTE_LAUNCH.serverlist, "patchersdk_serverlist");
        assert_eq!(
            NTE_LAUNCH.prefix_install_link,
            "Program Files (x86)/Neverness To Everness"
        );
        assert_eq!(NTE_LAUNCH.sdk_share_dir, "WPGameClientSDK");
        assert_eq!(OFFICIAL_ARGS, ["/launcher", "/directly"]);
    }

    #[test]
    fn display_backend_follows_the_desktop_and_gamescope() {
        for (wayland, gamescope, expected) in [
            (true, false, "1"),
            (false, false, "0"),
            (true, true, "0"),
            (false, true, "0"),
        ] {
            let mut command = Command::new("umu-run");
            command.env("PROTON_ENABLE_WAYLAND", "0");
            command.env("PROTON_USE_WAYLAND", "1");
            configure_display(&mut command, wayland, gamescope);
            let value = |key: &str| command.get_envs().find(|(name, _)| *name == key).unwrap().1;
            assert_eq!(
                value("PROTON_ENABLE_WAYLAND"),
                Some(std::ffi::OsStr::new(expected))
            );
            assert_eq!(
                value("PROTON_USE_WAYLAND"),
                Some(std::ffi::OsStr::new(expected))
            );
        }
    }

    #[test]
    fn game_network_removes_proxy_overrides_and_preserves_runtime_options() {
        let mut command = Command::new("umu-run");
        for key in [
            "proxy",
            "HTTP_PROXY",
            "https_proxy",
            "aLl_PrOxY",
            "NO_PROXY",
            "ftp_proxy",
        ] {
            command.env(key, "http://proxy.invalid:8080");
        }
        command
            .env("PROTONPATH", "/proton")
            .env("VKD3D_CONFIG", "test");
        configure_network(&mut command);
        let variables: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        for key in [
            "proxy",
            "HTTP_PROXY",
            "https_proxy",
            "aLl_PrOxY",
            "NO_PROXY",
            "ftp_proxy",
        ] {
            assert_eq!(variables.get(std::ffi::OsStr::new(key)), Some(&None));
        }
        assert_eq!(
            variables[std::ffi::OsStr::new("PROTONPATH")],
            Some(std::ffi::OsStr::new("/proton"))
        );
        assert_eq!(
            variables[std::ffi::OsStr::new("VKD3D_CONFIG")],
            Some(std::ffi::OsStr::new("test"))
        );
    }

    #[test]
    fn install_link_is_created_and_refreshed() {
        let dir = TempDir::new("sdk-link");
        let prefix = dir.join("prefix");
        let install = dir.join("install");
        fs::create_dir_all(&install).expect("create install dir");

        let error = NTE_LAUNCH
            .ensure_install_link(&prefix, &install)
            .expect_err("no drive_c");
        assert!(error.contains("Wine prefix 尚未初始化"), "{error}");

        fs::create_dir_all(prefix.join("drive_c")).expect("create drive_c");
        NTE_LAUNCH
            .ensure_install_link(&prefix, &install)
            .expect("create link");
        let link = prefix.join("drive_c/Program Files (x86)/Neverness To Everness");
        assert!(fs::symlink_metadata(&link)
            .expect("link metadata")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&link).expect("read link"), install);

        // 已经指向正确目标时保持原样.
        NTE_LAUNCH
            .ensure_install_link(&prefix, &install)
            .expect("keep link");
        assert_eq!(fs::read_link(&link).expect("read link"), install);

        // 指向别处时替换掉.
        fs::remove_file(&link).expect("remove link");
        let other = dir.join("other");
        fs::create_dir_all(&other).expect("create other dir");
        std::os::unix::fs::symlink(&other, &link).expect("create stale link");
        NTE_LAUNCH
            .ensure_install_link(&prefix, &install)
            .expect("refresh link");
        assert_eq!(fs::read_link(&link).expect("read link"), install);

        // 真实目录不会被替换.
        fs::remove_file(&link).expect("remove link");
        fs::create_dir_all(&link).expect("create real dir");
        NTE_LAUNCH
            .ensure_install_link(&prefix, &install)
            .expect("keep real dir");
        assert!(!fs::symlink_metadata(&link)
            .expect("link metadata")
            .file_type()
            .is_symlink());
    }
}
