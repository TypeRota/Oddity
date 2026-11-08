use std::path::{Path, PathBuf};
use std::process::Command;

use crate::game_profile::{GameId, LaunchRecipe};
use crate::{Error, Result};

pub mod background;

pub const OFFICIAL_ARGS: [&str; 2] = ["/launcher", "/directly"];

/// 桌面 Wayland 使用原生驱动, Gamescope 使用它管理的 Xwayland.
pub fn configure_display(command: &mut Command, wayland_available: bool, gamescope: bool) {
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
pub fn configure_network(command: &mut Command) {
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

#[derive(Debug, Clone)]
pub struct ProtonLaunch {
    pub runner: PathBuf,
    pub prefix: PathBuf,
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl ProtonLaunch {
    pub fn umu(game: GameId, prefix: &Path, exe: &Path) -> Result<Self> {
        let recipe = game.profile().launch().ok_or_else(|| {
            Error::Unsupported(format!("内置的《{}》配置缺少启动配方", game.profile().name))
        })?;
        Ok(Self {
            runner: PathBuf::from("umu-run"),
            prefix: prefix.to_path_buf(),
            exe: exe.to_path_buf(),
            args: recipe.args.iter().map(|s| s.to_string()).collect(),
            env: vec![
                ("WINEPREFIX".into(), prefix.display().to_string()),
                ("PROTON_ENABLE_WAYLAND".into(), "1".into()),
            ],
        })
    }

    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.runner);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        configure_network(&mut cmd);
        cmd.arg(&self.exe).args(&self.args);
        cmd
    }

    /// 官方启动器在拉起游戏前把 SDK DLL 路径写成单行配置文件.
    pub fn write_game_launcher_config(
        recipe: &LaunchRecipe,
        game_bin_dir: &Path,
        sdk_dll: &Path,
    ) -> std::io::Result<()> {
        std::fs::write(
            game_bin_dir.join(recipe.launcher_config),
            sdk_dll.display().to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

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
    fn umu_launch_carries_prefix_and_game_args() {
        let launch = ProtonLaunch::umu(
            GameId::Nte,
            Path::new("/prefix"),
            Path::new("/tmp/NTEGame.exe"),
        )
        .expect("NTE launch");
        assert_eq!(launch.runner, PathBuf::from("umu-run"));
        assert_eq!(launch.prefix, PathBuf::from("/prefix"));
        assert_eq!(launch.exe, PathBuf::from("/tmp/NTEGame.exe"));
        assert_eq!(
            launch.args,
            GameId::Nte.profile().launch().unwrap().args.to_vec()
        );
        assert!(launch
            .env
            .contains(&("WINEPREFIX".to_string(), "/prefix".to_string())));
        assert!(launch
            .env
            .contains(&("PROTON_ENABLE_WAYLAND".to_string(), "1".to_string())));

        let cmd = launch.command();
        assert_eq!(cmd.get_program().to_string_lossy(), "umu-run");
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "/tmp/NTEGame.exe".to_string(),
                GameId::Nte.profile().launch().unwrap().args[0].to_string(),
                GameId::Nte.profile().launch().unwrap().args[1].to_string()
            ]
        );
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|value| value.to_string_lossy().to_string()),
                )
            })
            .collect();
        assert!(envs.contains(&("WINEPREFIX".to_string(), Some("/prefix".to_string()))));
        assert!(envs.contains(&("PROTON_ENABLE_WAYLAND".to_string(), Some("1".to_string()))));
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
    fn game_without_a_launch_recipe_cannot_launch() {
        let error = ProtonLaunch::umu(GameId::Hsr, Path::new("/prefix"), Path::new("/hsr.exe"))
            .unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)), "{error:?}");
    }

    #[test]
    fn writes_a_single_line_game_launcher_config() {
        let dir = TempDir::new("launch");
        let bin_dir = dir.join("Binaries/Win64");
        std::fs::create_dir_all(&bin_dir).expect("create bin dir");
        ProtonLaunch::write_game_launcher_config(
            GameId::Nte.profile().launch().unwrap(),
            &bin_dir,
            Path::new("Z:\\game\\WPGameClientSDK_64.dll"),
        )
        .expect("write config");
        assert_eq!(
            dir.read_text("Binaries/Win64/GameLauncher.config"),
            "Z:\\game\\WPGameClientSDK_64.dll"
        );
    }
}
