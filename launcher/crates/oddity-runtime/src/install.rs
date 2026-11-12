//! 安装位置探测的宿主-包接口.
//!
//! 安装目录候选、环境变量与官方启动器的文件位置都属于游戏包; 宿主只把
//! 只读设置整理成 [`InstallQuery`], 再消费 [`InstallLocator`] 返回的路径.

use std::path::{Path, PathBuf};

/// 安装探测的只读输入.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallQuery {
    /// 当前发行渠道, 与设置里的 distribution 一致, 例如 cn、global.
    pub distribution: String,
    /// 设置里显式选择的安装根目录, 调用方已经展开 `~`.
    ///
    /// 有值时探测只认这个目录, 找不到就是未安装, 不回退到其他候选.
    pub configured_install_root: Option<PathBuf>,
}

/// 官方启动器记录的版本信息.
///
/// 字段来自启动器自己的配置文件; 没有填写的项是空字符串.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherInfo {
    /// 启动器版本.
    pub version: String,
    /// 启动器构建号.
    pub build: String,
}

/// 游戏包提供的安装位置知识.
///
/// 宿主按当前游戏从 [`crate::Registry`] 取到实现, 不在宿主保留目录候选
/// 或回退规则.
pub trait InstallLocator: Send + Sync + 'static {
    /// 探测官方启动器根目录; 未安装时返回 None.
    fn launcher_root(&self, query: &InstallQuery) -> Option<PathBuf>;

    /// 安装根目录下的官方启动器目录名; 该地区没有安装布局时返回 None.
    fn launcher_dir(&self, query: &InstallQuery) -> Option<&'static str>;

    /// 尚未安装时界面展示与下载规划使用的安装根目录.
    fn selected_install_root(&self, query: &InstallQuery) -> PathBuf;

    /// 有效安装根目录.
    ///
    /// 配置了安装目录时用它本身; 否则由已探测到的启动器根目录取父目录;
    /// 两者都没有时退回该地区的默认安装位置.
    fn effective_install_root(&self, query: &InstallQuery, launcher_root: Option<&Path>)
        -> PathBuf;

    /// 游戏可执行文件; 未安装或该游戏没有安装布局时返回 None.
    fn game_exe(&self, launcher_root: &Path) -> Option<PathBuf>;

    /// 官方启动器记录的游戏版本; 没有记录时为空字符串.
    fn game_version(&self, launcher_root: &Path) -> String;

    /// 官方启动器记录的版本信息; 配置读不到时返回 None.
    fn launcher_info(&self, launcher_root: &Path) -> Option<LauncherInfo>;

    /// 游戏可执行文件缺失时给用户看的提示.
    fn game_exe_hint(&self) -> String;
    /// SDK 组件所在目录; 未安装或找不到时返回 None.
    ///
    /// 启动器与游戏运行需要把 SDK DLL 放在这个目录; 宿主只展示与诊断.
    fn sdk_dir(&self, _launcher_root: &Path) -> Option<PathBuf> {
        None
    }

    /// 官方启动器写入的启动器配置文件路径; 找不到时返回 None.
    fn launcher_config_path(&self, _install_root: &Path) -> Option<PathBuf> {
        None
    }

    /// 官方客户端可执行文件, 与启动器根目录同区服; 找不到时返回 None.
    fn official_client_exe(&self, _launcher_root: &Path) -> Option<PathBuf> {
        None
    }

    /// Wine prefix 内游戏安装链接的相对路径; 没有迁移链接时返回 None.
    fn prefix_link_name(&self) -> Option<&'static str> {
        None
    }
}
