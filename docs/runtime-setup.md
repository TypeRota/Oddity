# 启动环境向导

打开 Oddity 时若检测到缺少启动组件或当前游戏的运行环境, 会进入 **启动环境向导** 页面并自动开始准备. 需要管理员授权时显示具体操作, 等待用户确认. 手动配置方式位于 **安装选项**. 设置 → 运行环境中的 **打开向导** 可重新打开此页面.

## 安装范围

默认使用当前用户安装. 已有可用 umu 会直接复用; 缺失时下载官方 umu 1.4.4 zipapp, 安装到 Oddity 数据目录的 `runtime/umu/umu-run`. 只有安装用户级 zipapp 时检查 Python 3.10 或更新版本; 只有安装系统 umu 时检查 `pkexec`. 原生安装包仍声明 Python 和提供 `pkexec` 的系统软件包为必需依赖; AppImage 按实际安装操作提示缺失项.

勾选 **为全部用户安装 umu** 后, 缺失的系统 umu 通过发行版包管理器安装. 当前识别 Arch 及其衍生版、Fedora、Debian/Ubuntu 及其衍生版、openSUSE. 软件包必须存在于用户已配置的软件源中; Oddity 不添加软件源或启用 multilib. 不支持的发行版可使用当前用户安装, 或自行安装系统 umu 后重新检测.

自动准备组件; 需要系统安装时弹窗显示操作和完整 `pkexec` 命令. 点击 **授权并安装** 后请求系统授权, 安装输出保存在操作日志中. 取消授权不会继续下载安装 Proton, 可重试.

## Proton 与运行时

自动配置使用固定的 `GE-Proton11-12-x86_64`, 官方下载包在解压前校验 SHA-256. Proton 安装到检测到的 Steam 根目录下的 `compatibilitytools.d/<版本>`. 检测常规 Steam 链接和 Flatpak Steam 目录; 未安装 Steam 时使用 `~/.local/share/Steam/compatibilitytools.d/`. 不需要安装 Steam 客户端.

已完成并通过摘要校验的下载可在重试时复用. 记录中的 Proton 路径失效时, 自动配置回到当前 Steam 目录下的默认下载目标. Proton 在同一文件系统的临时目录解压, 成功后才放入目标位置. 已有不完整的目标目录会报错, 不覆盖其中的文件. 手动配置直接选择包含 `proton` 和 `toolmanifest.vdf` 的本地目录.

Oddity 使用普通用户身份调用 umu, 让它准备对应的 Steam Linux Runtime. 运行时下载与解压阶段显示阶段信息, 详细输出写入日志; 这一阶段不提供字节进度. 运行时缓存由 umu 管理, 不承诺固定 Steam Linux Runtime 版本.

Proton、运行时、prefix 和游戏启动均以当前用户身份执行. **为全部用户安装 umu** 不会共享 prefix 或登录状态. 显卡驱动及 Vulkan 支持仍需由系统提供.

## 游戏环境

组件准备完成后, 已安装的当前区服会自动创建并初始化独立运行环境. 尚未安装游戏时也会创建并登记运行环境, 绑定当前游戏目录和区服. 已有匹配的环境直接复用; 已固定的 Proton 不会因自动配置而替换.

安装计划和进度由 Rust 服务持有, 通过现有环境任务互斥边界阻止游戏启动及冲突的环境操作. 每次配置由服务生成安装计划, 仅需提权的操作额外确认. 系统安装只执行服务生成的包管理器命令, 不接收前端提供的命令.

配置记录位于 Oddity 数据目录的 `runtime/setup.json`, 日志位于 `logs/runtime-setup.log`. 组件固定版本的实际 NTE 启动兼容性需在发布前进行实机验收.

## 实现参考

- umu-launcher 的官方用户安装使用 zipapp. Oddity 通过 `wineboot -u` 初始化 prefix, 显式设置 `PROTON_VERB=waitforexitandrun`, 检查进程成功退出并确认 Windows 注册表已生成.
- Heroic 将兼容层下载管理和 umu 启动集成分开. Oddity 沿用现有独立运行环境管理, 不引入 Heroic 的运行时框架或代码.

参考项目、用途与许可证见 [THIRD_PARTY.md](../THIRD_PARTY.md).

## 向导界面

umu、Proton、运行时使用独立的准备界面, 核心安装进度携带组件标识, 界面不根据文件名猜测阶段. 下载量持续更新, 未知总量的准备和初始化过程显示活动进度条, 可打开操作日志.

阶段切换使用 Anime.js 同时交叉淡化两层画面, 配合短促的模糊渐入渐出; 安装选项过渡为 160ms, 共用同一图标的首页与 umu 不使用模糊. 图标本身保持静态. 图形、标题、进度和操作区使用固定布局槽位, 不参与过渡重排; 进度轮询不重播入场动画. 安装选项、授权弹窗和完成操作均沿用应用的动画过渡, 向导不显示滚动条. 成功完成后由 Anime.js 编排 canvas-confetti 全窗口双侧礼炮, 系统要求减少动态效果时省略礼炮和循环动效.

重新进入向导时展示已有环境, 可查看或继续创建独立环境. 继续创建会复用已安装的组件, 不覆盖原环境和稳定标记. umu 的系统安装范围与 prefix 创建相互独立, 系统 umu 已安装时锁定范围开关并直接复用.

配置成功前必须完成环境登记和 prefix 初始化. 点击 **查看运行环境** 返回 **设置 → 运行环境**, 可查看已登记的环境.

## 实机初始化检查

可在隔离临时目录运行原生初始化验收, 不启动游戏:

```bash
ODDITY_TEST_UMU=/usr/bin/umu-run ODDITY_TEST_PROTON=/path/to/Proton \
  cargo test --manifest-path launcher/Cargo.toml -p nte-core --test runtime_setup \
  real_runtime_initializes_an_isolated_prefix -- --ignored
```

该检查调用实际安装及初始化代码, 包含带空格路径和已登记 prefix 的初始化. 日常离线测试不依赖本机安装的 Proton.
