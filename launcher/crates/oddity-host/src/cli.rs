//! 外部启动参数. 保持手写解析, 避免为几个固定参数引入完整命令行框架.

use std::ffi::OsString;

use crate::channel::{self, ChannelError, ChannelId};
use oddity_runtime::Registry;

use crate::identity::{known_list, GameId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Official,
    Background,
    Direct,
}

impl LaunchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Background => "background",
            Self::Direct => "direct",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "official" => Ok(Self::Official),
            "background" => Ok(Self::Background),
            "direct" => Ok(Self::Direct),
            _ => Err(format!(
                "无法识别 --launch-mode 参数: {value}. 可选值: official, background, direct."
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// 请求启动的游戏; 省略时使用默认游戏.
    pub game: GameId,
    pub region: Option<ChannelId>,
    pub mode: LaunchMode,
    pub environment: Option<String>,
}

pub const USAGE: &str = "用法: oddity --launch [--game <游戏 id>] [--region <渠道 id>] [--launch-mode official|background|direct] [--environment <名称或 ID>]";

/// --launch 配合 --help 时只打印用法, 避免把帮助当成一次启动请求.
pub fn help_requested(args: &[OsString]) -> bool {
    args.iter().any(|arg| arg == "--launch")
        && args.iter().any(|arg| arg == "--help" || arg == "-h")
}

/// 解析外部启动参数; 没有 --launch 时返回 None.
///
/// 只接受当前装配且有安装域的游戏; 区服支持来自该游戏的安装域.
pub fn parse(args: &[OsString], registry: &Registry) -> Result<Option<LaunchRequest>, String> {
    let mut launch = false;
    let mut game = None;
    let mut region = None;
    let mut mode = None;
    let mut environment = None;
    let mut index = 0;
    while index < args.len() {
        let raw = args[index].to_string_lossy();
        if raw == "--launch" {
            launch = true;
            index += 1;
            continue;
        }
        if let Some(value) = take_value(args, &mut index, "--game")? {
            set_once(&mut game, value, "--game")?;
            continue;
        }
        if let Some(value) = take_value(args, &mut index, "--region")? {
            set_once(&mut region, value, "--region")?;
            continue;
        }
        if let Some(value) = take_value(args, &mut index, "--launch-mode")? {
            set_once(&mut mode, value, "--launch-mode")?;
            continue;
        }
        if let Some(value) = take_value(args, &mut index, "--environment")? {
            set_once(&mut environment, value, "--environment")?;
            continue;
        }
        // 宿主可能追加自己的参数, 未知项保持忽略.
        index += 1;
    }

    if !launch {
        if game.is_some() || region.is_some() || mode.is_some() || environment.is_some() {
            return Err(
                "--game, --region, --launch-mode 和 --environment 只能与 --launch 一起使用.".into(),
            );
        }
        return Ok(None);
    }

    let known = registry.selectable_games();
    let game = match game {
        Some(value) => {
            let game = GameId::new(&value);
            if !known.contains(&game.as_str()) {
                return Err(format!(
                    "无法识别 --game 参数: {value}. 可选值: {}.",
                    known_list(&known)
                ));
            }
            game
        }
        None => {
            let game = GameId::default();
            if !known.contains(&game.as_str()) {
                return Err(format!(
                    "无法识别 --game 参数: {}. 可选值: {}.",
                    game.as_str(),
                    known_list(&known)
                ));
            }
            game
        }
    };
    let region = match region {
        Some(value) => {
            let channel = value
                .parse::<ChannelId>()
                .map_err(|_| format!("无法识别 --region 参数: {value}."))?;
            channel::resolve(registry, game.as_str(), Some(&channel)).map_err(
                |error| match error {
                    ChannelError::Unknown { allowed, .. } => format!(
                        "无法识别 --region 参数: {value}. 可选值: {}.",
                        allowed.join(", ")
                    ),
                    error => format!("无法识别 --region 参数: {value}. {error}"),
                },
            )?;
            Some(channel)
        }
        None => None,
    };
    let mode = match mode {
        Some(value) => LaunchMode::parse(&value)?,
        // 桌面动作不传模式时也走官方启动器与正常登录流程.
        None => LaunchMode::Background,
    };
    Ok(Some(LaunchRequest {
        game,
        region,
        mode,
        environment,
    }))
}

fn take_value(args: &[OsString], index: &mut usize, name: &str) -> Result<Option<String>, String> {
    let raw = args[*index].to_string_lossy();
    let value = if raw == name {
        *index += 1;
        let value = args
            .get(*index)
            .ok_or_else(|| format!("{name} 缺少参数值."))?;
        value.to_string_lossy().to_string()
    } else if let Some(value) = raw.strip_prefix(&format!("{name}=")) {
        *index += 1;
        value.to_string()
    } else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() || value.starts_with("--") {
        return Err(format!("{name} 缺少参数值."));
    }
    Ok(Some(value.to_string()))
}

fn set_once(slot: &mut Option<String>, value: String, name: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{name} 不能重复指定."));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oddity_runtime::{
        GamePackage, InstallLocator, InstallQuery, LauncherInfo, PackageDescriptor,
    };
    use std::path::{Path, PathBuf};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        crate::testing::install(&mut registry);
        registry
    }

    /// 只声明国服安装域的额外游戏, 用来验证区服校验来自 locator.
    struct CnOnly;

    impl InstallLocator for CnOnly {
        fn launcher_root(&self, _query: &InstallQuery) -> Option<PathBuf> {
            None
        }

        fn launcher_dir(&self, _query: &InstallQuery) -> Option<&'static str> {
            None
        }

        fn selected_install_root(&self, _query: &InstallQuery) -> PathBuf {
            PathBuf::new()
        }

        fn effective_install_root(
            &self,
            _query: &InstallQuery,
            _launcher_root: Option<&Path>,
        ) -> PathBuf {
            PathBuf::new()
        }

        fn game_exe(&self, _launcher_root: &Path) -> Option<PathBuf> {
            None
        }

        fn game_version(&self, _launcher_root: &Path) -> String {
            String::new()
        }

        fn launcher_info(&self, _launcher_root: &Path) -> Option<LauncherInfo> {
            None
        }

        fn game_exe_hint(&self) -> String {
            String::new()
        }
    }

    static CN_ONLY: CnOnly = CnOnly;

    static CN_ONLY_CHANNELS: &[oddity_runtime::ChannelDescriptor] =
        &[oddity_runtime::ChannelDescriptor {
            id: "cn",
            label: "国服",
            is_default: true,
            install_root_key: "cn",
        }];

    struct CnOnlyPackage;

    impl GamePackage for CnOnlyPackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "legacy",
                name: "旧游戏",
            }
        }

        fn register(&self, _registry: &mut Registry) {}

        fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
            Some(&CN_ONLY)
        }

        fn channels(&self) -> &'static [oddity_runtime::ChannelDescriptor] {
            CN_ONLY_CHANNELS
        }
    }

    /// 没有安装域的包不能从外部启动, 也不能出现在可选值里.
    struct BarePackage;

    impl GamePackage for BarePackage {
        fn descriptor(&self) -> PackageDescriptor {
            PackageDescriptor {
                id: "fixture",
                name: "夹具",
            }
        }

        fn register(&self, _registry: &mut Registry) {}
    }

    fn registry_with_extra() -> Registry {
        let mut registry = registry();
        registry.install(&CnOnlyPackage);
        registry.install(&BarePackage);
        registry
    }

    fn parse(values: &[&str]) -> Result<Option<LaunchRequest>, String> {
        super::parse(&args(values), &registry())
    }

    fn parse_extra(values: &[&str]) -> Result<Option<LaunchRequest>, String> {
        super::parse(&args(values), &registry_with_extra())
    }

    #[test]
    fn plain_startup_has_no_launch_request() {
        assert!(parse(&[]).unwrap().is_none());
        assert!(parse(&["--recovery"]).unwrap().is_none());
        assert!(parse(&["--some-gtk-flag"]).unwrap().is_none());
    }

    #[test]
    fn launch_defaults_to_background_default_game_and_current_region() {
        let request = parse(&["--launch"]).unwrap().unwrap();
        assert_eq!(request.game, GameId::from("nte"));
        assert_eq!(request.region, None);
        assert_eq!(request.mode, LaunchMode::Background);
        assert_eq!(request.environment, None);
    }

    #[test]
    fn parses_an_assembled_game_and_rejects_the_rest() {
        let request = parse(&["--launch", "--game", "nte"]).unwrap().unwrap();
        assert_eq!(request.game, GameId::from("nte"));
        let request = parse(&["--game=NTE", "--launch"]).unwrap().unwrap();
        assert_eq!(request.game, GameId::from("nte"));
        let request = parse_extra(&["--launch", "--game", "legacy", "--region", "cn"])
            .unwrap()
            .unwrap();
        assert_eq!(request.game, GameId::from("legacy"));
        assert_eq!(request.region, Some(ChannelId::new("cn")));

        let error = parse(&["--launch", "--game", "genshin"]).unwrap_err();
        assert!(error.contains("无法识别 --game 参数: genshin"), "{error}");
        assert!(error.contains("可选值: nte."), "{error}");
        // 没有安装域的包不在可选值里, 也不能外部启动.
        let error = parse_extra(&["--launch", "--game", "fixture"]).unwrap_err();
        assert!(error.contains("无法识别 --game 参数: fixture"), "{error}");
        assert!(error.contains("nte, legacy"), "{error}");

        assert!(parse(&["--game"]).is_err());
        let error = parse(&["--launch", "--game"]).unwrap_err();
        assert!(error.contains("缺少参数值"), "{error}");
        let error =
            parse_extra(&["--launch", "--game", "legacy", "--region", "global"]).unwrap_err();
        assert!(
            error.contains("无法识别 --region 参数: global. 可选值: cn."),
            "{error}"
        );
        let error = parse(&["--launch", "--game", "nte", "--game", "nte"]).unwrap_err();
        assert!(error.contains("不能重复指定"), "{error}");
    }

    #[test]
    fn parses_both_region_forms_and_all_modes() {
        let request = parse(&["--launch", "--region", "CN"]).unwrap().unwrap();
        assert_eq!(request.region, Some(ChannelId::new("cn")));
        let request = parse(&["--region=global", "--launch"]).unwrap().unwrap();
        assert_eq!(request.region, Some(ChannelId::new("global")));
        for (value, expected) in [
            ("official", LaunchMode::Official),
            ("background", LaunchMode::Background),
            ("direct", LaunchMode::Direct),
        ] {
            let request = parse(&["--launch", "--launch-mode", value])
                .unwrap()
                .unwrap();
            assert_eq!(request.mode, expected);
        }
    }

    #[test]
    fn parses_environment_by_name_or_id() {
        let request = parse(&["--launch", "--environment", "GE-Proton 9"])
            .unwrap()
            .unwrap();
        assert_eq!(request.environment.as_deref(), Some("GE-Proton 9"));
        let request = parse(&["--launch", "--environment=abc123"])
            .unwrap()
            .unwrap();
        assert_eq!(request.environment.as_deref(), Some("abc123"));
    }

    #[test]
    fn rejects_options_without_launch_and_bad_values() {
        assert!(parse(&["--region", "cn"]).is_err());
        assert!(parse(&["--game", "nte"]).is_err());
        assert!(parse(&["--launch", "--region", "eu"]).is_err());
        assert!(parse(&["--launch", "--region"]).is_err());
        assert!(parse(&["--launch", "--region", "--launch-mode"]).is_err());
        assert!(parse(&["--launch", "--launch-mode", "debug"]).is_err());
        assert!(parse(&["--launch", "--environment"]).is_err());
        assert!(parse(&["--launch", "--game", "genshin"]).is_err());
        assert!(parse(&["--launch", "--region", "cn", "--region", "global"]).is_err());
        assert!(parse(&[
            "--launch",
            "--launch-mode",
            "direct",
            "--launch-mode",
            "official"
        ])
        .is_err());
    }

    #[test]
    fn help_only_applies_to_launch_requests() {
        assert!(!help_requested(&args(&["--help"])));
        assert!(help_requested(&args(&["--launch", "--help"])));
        assert!(help_requested(&args(&["-h", "--launch"])));
        assert!(!help_requested(&args(&["--launch"])));
    }

    #[test]
    fn ignores_unknown_arguments_between_known_options() {
        let request = parse(&["--display", ":0", "--launch", "--region", "global"])
            .unwrap()
            .unwrap();
        assert_eq!(request.region, Some(ChannelId::new("global")));
    }
}
