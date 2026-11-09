# 第三方组件

Oddity 的代码使用 [GPL-3.0-or-later](LICENSE). 下列组件保留各自许可证.

## 字体与图标

| 组件                   | 用途     | 许可证                                                          |
| ---------------------- | -------- | --------------------------------------------------------------- |
| Space Grotesk          | 界面字体 | [OFL 1.1](launcher/ui/public/assets/fonts/OFL.txt)              |
| Noto Serif CJK SC 子集 | 标题字体 | [OFL 1.1](launcher/ui/public/assets/fonts/OFL-NotoSerifCJK.txt) |
| Phosphor Icons         | 图标     | [MIT](launcher/ui/public/assets/icons/LICENSE)                  |

## 依赖

前端依赖见 [package.json](launcher/ui/package.json), 锁定版本见 [pnpm-lock.yaml](launcher/ui/pnpm-lock.yaml).

Rust 依赖见 [Cargo.lock](launcher/Cargo.lock), 许可证检查配置见 [deny.toml](launcher/deny.toml).

游戏差分补丁使用 [hdiffpatch-rs](https://github.com/TwintailTeam/hdiffpatch-rs), 许可证为 MIT.

音频设备与游戏播放流控制使用 [libpulse-binding](https://github.com/jnqnfe/pulse-binding-rust), 许可证为 MIT / Apache-2.0; 通过系统 libpulse 连接 PulseAudio 或 pipewire-pulse. 设备和播放流分别管理的设计参考 [pulsemixer](https://github.com/GeorgeFilipkin/pulsemixer) (MIT), 未复制其代码.

音频启动环境中的 `target.object` 属性用于让 Easy Effects 保留游戏的独立路由, 依据其 [播放流目标识别逻辑](https://github.com/wwmm/easyeffects/blob/master/src/pw_node_manager.cpp); 未复制其代码.

## 颜色计算

| 用途           | 来源                                                                                                             | 许可证        |
| -------------- | ---------------------------------------------------------------------------------------------------------------- | ------------- |
| Oklab 颜色空间 | [Björn Ottosson, A perceptual color space for image processing (2020)](https://bottosson.github.io/posts/oklab/) | Public domain |

主视觉取色在 `launcher/ui/src/core/palette.ts` 中实现, 只使用 Oklab 的公开转换矩阵, 不引入额外依赖.

## 图像与素材

| 素材                         | 位置                                           | 授权                         |
| ---------------------------- | ---------------------------------------------- | ---------------------------- |
| 项目自制背景, 应用图标与标志 | `launcher/ui/public/assets/`                   | [GPL-3.0-or-later](LICENSE)  |
| 官方游戏美术素材             | 文档截图, 公告预览, 运行时读取的官方客户端素材 | 版权归权利人所有, 不适用 GPL |

内置背景为项目自制素材. Oddity 会读取本机官方客户端的主页素材, 文档截图与公告预览中也包含官方游戏美术素材. 这些素材仅用于识别游戏和展示界面, 不属于 GPL 授权范围.

官方素材的授权范围与移除方式见[美术素材声明](ASSETS.md).

## 塔吉多协议参考

| 用途                         | 来源与参考提交                                                                                                                | 许可证                                          |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| 登录, 签到, 金币与云异环任务 | [taygedo-auto-attendance](https://github.com/zzstar101/taygedo-auto-attendance/tree/cc8c036d0a66802a39b4ecc10f7095e38f695151) | [MIT](licenses/taygedo-auto-attendance-MIT.txt) |
| 游戏状态查询与字段解析       | [NTEUID](https://github.com/tyql688/NTEUID/tree/8fc59ff35b7fdbd1e31bf6b15ec4ba0c8d36b7cb)                                     | [GPL-3.0](licenses/NTEUID-GPL-3.0.txt)          |

## 启动器实现参考

| 用途                                           | 来源                                                              | 许可证                                                                |
| ---------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------- |
| 预下载与补丁应用分离, 选择可选资源后下载并校验 | [Collapse Launcher](https://github.com/CollapseLauncher/Collapse) | [MIT](https://github.com/CollapseLauncher/Collapse/blob/main/LICENSE) |

只参考阶段划分和交互边界, 未引入其 Sophon 依赖, 也未复制代码.

## Electron 运行时

图形界面使用 Electron 44 (MIT), 其中的 Chromium、Node.js 与 FFmpeg 使用各自许可. 发行目录保留 Electron 的 `LICENSE` 与完整 `LICENSES.chromium.html`; Oddity 的 GPL-3.0-or-later 许可证位于 `LICENSE.oddity`.

## 启动环境准备

| 组件 | 用途 | 许可证 |
| --- | --- | --- |
| [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher) | 下载官方 zipapp 或通过系统软件包安装, 调用它准备 Steam Linux Runtime 和 prefix | [GPL-3.0](https://github.com/Open-Wine-Components/umu-launcher/blob/main/LICENSE) |
| [GE-Proton](https://github.com/GloriousEggroll/proton-ge-custom) | 下载并安装官方兼容工具分发包 | [上游各组件许可证](https://github.com/GloriousEggroll/proton-ge-custom#license) |
| [Heroic Games Launcher](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher) | 参考兼容层下载管理与 umu 集成的职责划分, 未复制代码 | [GPL-3.0](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher/blob/main/LICENSE) |

下载的 umu、Proton 和 Steam Linux Runtime 保留各自分发包中的许可证. Rust tar 解包依赖使用 MIT/Apache-2.0, 版本见 Cargo.lock.

## 完成礼花

启动环境向导使用 [canvas-confetti](https://github.com/catdad/canvas-confetti) 的粒子物理与 Canvas 渲染, 许可证为 [ISC](https://github.com/catdad/canvas-confetti/blob/master/LICENSE). 发射时序由 Anime.js 编排, 尊重减少动态效果设置, 离开向导时清理粒子.

## 性能采样与图表

性能比较调用系统 [MangoHud](https://github.com/flightlessmango/MangoHud) (MIT), 按其官方 `logging.cpp` 定义读取逐帧 CSV, 参考随附 mangoplot 的本地日志出图方式; 未复制代码, 不上传采样数据. 硬件补充指标来自 `/proc` 和系统 `nvidia-smi`, 无需安装 Python 分析环境.

SVG 转 PNG 使用 [resvg](https://github.com/linebender/resvg) (MIT / Apache-2.0), CSV 解析使用 [rust-csv](https://github.com/BurntSushi/rust-csv) (MIT / Unlicense). 报告图表与统计逻辑为项目实现.

resvg 的 tiny-skia 依赖 [arrayref](https://github.com/droundy/arrayref) 0.3.9 (BSD-2-Clause), 许可证文本随该 crate 发布; `deny.toml` 为此版本设置限定许可.

GPU 进程分类参考 [gpustat](https://github.com/wookayin/gpustat) (MIT) 区分图形与计算上下文的方式; 使用系统 `nvidia-smi` XML, 未复制代码或引入 NVML 绑定.
