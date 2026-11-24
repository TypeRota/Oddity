# 贡献指南

感谢你愿意改进 Oddity。提交前请先读本页，能省掉很多来回沟通。

## 先看这些

- 第一次参与贡献，先看[贡献者入门](https://github.com/LaT-SKY/Oddity/discussions/156)：项目结构、本地开发、第一个 PR 的流程。
- 使用与安装问题请移步 [README](README.md)、[安装指南](docs/install.md) 和[排障指南](docs/troubleshooting.md)。
- 安全漏洞不要提交公开 Issue，按 [SECURITY.md](SECURITY.md) 的方式私下报告。

## 工作方式

- 动手前先读懂相关代码，再决定怎么改。
- 关注要达成的结果，不要机械照搬流程。
- 优先选择简单、可维护、与现有架构一致的方案；可复用的逻辑放进 `oddity-host`，不要复制一份。
- 遇到不熟悉的功能，先在 GitHub 上找维护活跃、许可证明确的同类开源项目，比较实现方式、依赖和已知问题。优先采用经过验证的设计，不要直接复制许可证不兼容或来源不明的代码。

## 代码风格

### 注释

- **注释写中文**，说明意图即可，不要逐行翻译代码。
- 不要在注释里写实现状态、协作进度、里程碑、决策、反思或备忘。
- 用户可见的文字一律使用正式中文、统一术语和半角符号。

### Rust / 脚本 / 前端

- Rust 使用 `rustfmt` 默认配置，并且必须通过 Clippy。
- Shell 脚本必须通过 ShellCheck。
- 前端使用 Vite + TypeScript + Vue 3，TypeScript 开启严格模式。
- Prettier 负责格式，Stylelint 负责 CSS 的错误写法和无效声明，两者职责不重叠。
- 所有动画使用 Motion For Vue，涉及动画编排时使用 Anime.js，不要用 CSS 手写动画。
- 写组件前先看 `launcher/ui/src/components/` 有没有相同或相近的实现，不要重复造。
- 不要顺手格式化无关文件，也不要把重构和新功能混在同一个 PR。

## 前端目录

`launcher/ui` 根目录保留工具配置和 HTML 入口，源码在 `src/` 下按职责组织：

- `main.ts`、`App.vue`：应用入口和整体布局组装。
- `components/`：基础组件和各功能面板。
- `pages/`：启动、更新、公告和设置页面。
- `core/`：命令桥接、状态与交互控制。
- `host/`：宿主页面、槽位和贡献注册表；其中 `host/generated/` 由生成器产出，不要手改。
- `host-api/`：游戏包可用的宿主接口（`api.ts`、`ui-kit.ts`）。
- `composables/`：跨组件复用的组合式函数。
- `styles/`：按职责拆分的全局样式，入口为 `main.css`，设计变量集中在 `tokens.css`。
- `types/`：类型定义。

游戏包位于 `launcher/packages/<game>/`，后端和 UI 各自独立：

- `backend/`：Rust 后端，声明该游戏的命令组、安装域和启动实现。
- `ui/src/`：该游戏的页面、设置和预览，通过 `contribution.ts` 与 `pages.ts` 向宿主注册。

游戏包 UI 只能依赖包内代码、约定的宿主公共入口和包自己的依赖，不要引用宿主内部实现，也不要直接使用 Node API；宿主侧只有 `launcher/ui/src/host/generated/` 可以引用游戏包。这条边界由 `node scripts/check-ui-boundaries.mjs` 检查。

静态资源放在 `launcher/ui/public/assets/`，单元测试放在 `launcher/ui/tests/unit/`，浏览器测试放在 `launcher/ui/tests/e2e/`，桌面端测试放在 `tests/desktop/`，演示站测试放在 `tests/demo/`。文档截图放在 `docs/assets/`，只保留文档实际引用的文件；PR 的前后对比图直接上传到 PR 描述，不提交进仓库。

界面文字、类名、显示状态和按钮状态通过 Vue 响应式绑定更新。DOM 引用用于视频播放等原生 API，全局事件和订阅随组件卸载释放。应用版本由 `package.json` 注入构建，后台提供的游戏和启动器版本通过快照显示，不在 Vue 模板中固定版本号。

## 测试与覆盖率

- 测试要断言实际行为，不要只把代码跑一遍。
- 值得测的是行为，不是实现细节；补哪些测试自行判断。
- 核心 TS 逐文件要求行覆盖率不低于 80%、分支覆盖率不低于 70%，由 Vitest 强制。
- UI 组件与动效的行覆盖率参考目标是 60%，未达到不阻断检查。
- Rust 总体行覆盖率不低于 80%，PR 改动的生产代码同样按 80% 判断，由 `scripts/check-rust-coverage.mjs` 判定。
- 不要通过排除文件、空测试、只调用不断言、修改门槛等方式绕过覆盖率要求。确实无法测试的代码，在 PR 中说明原因，由维护者判断。

## 提交 PR 前

在 Open PR 之前，先在自己的机器上完整检查一遍，不要只依赖 CI。

Rust 与生成物：

```bash
cd launcher
cargo run --locked -p contract-gen -- --check
cargo run --locked -p ui-registry-gen -- --check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
```

覆盖率：

```bash
cd launcher
cargo llvm-cov --workspace --locked --lcov --output-path lcov.info
cd ..
node scripts/check-rust-coverage.mjs
```

前端：

```bash
cd launcher/ui
pnpm install --frozen-lockfile
pnpm run format:check
pnpm run lint:css
node ../../scripts/check-ui-boundaries.mjs
pnpm run build
pnpm run test:coverage
pnpm run test:e2e
```

在仓库根目录执行仓库与打包检查：

```bash
node scripts/check-repo.mjs
node scripts/check-packaging.mjs
```

如果改动涉及 Electron、桌面入口或打包，本地至少构建一次 Debian 包：

```bash
cd launcher/ui
pnpm exec install-electron
pnpm run desktop:package
```

完整发布流程会在 Ubuntu 22.04 上构建 `.deb`、`.rpm`、`.pkg.tar.zst` 和 `.AppImage`，并校验包版本、依赖、桌面入口和图标。

至少运行与改动相关的检查，如果不确定，那就把全部检查跑完。

## Commit 约定

Commit 标题使用模块前缀：

```text
module: change
```

例如：

```text
ui: 调整设置页的间距
core: 修复资源清单解析失败
ci: 添加覆盖率下限检查
docs: 补充首次启动说明
```

- 冒号使用半角 `:`。
- 一次提交尽量只做一件相关的事，不要把格式化、重构和功能混在一起。
- 如果一次提交包含多个变更，标题写影响最大的那个，其余变更放在提交描述里。
- 描述里写清楚改了什么、为什么改、有什么影响。

## Issue 与 PR 标题

### Issue 标题

Issue 标题必须带类型前缀，并与类型标签保持一致：

```text
[BUG] 启动游戏后窗口没有出现
[CRASH] 切换账号时闪退
[FEATURE] 希望支持自定义游戏目录
[ENHANCEMENT] 调整设置界面的布局间距
[QUESTION] Proton 版本应该怎么选
```

一个 Issue 只保留一个类型标签。

### PR 标题

PR 标题使用以下两种写法之一：

```text
ui: 调整设置页的间距
docs: 补充首次启动说明
feat(ui): 增加启动器托盘菜单
fix(updater): 修复断点续传进度计算
```

- 模块式写法使用 `module: change`。
- 语义化写法使用 `type(scope): change`，type 表示变更类别，scope 表示受影响的模块。
- 冒号使用半角 `:`。
- 标题描述不要以大写字母开头。
- 破坏性变更在冒号前加 `!`，例如 `refactor(config)!: 调整设置文件结构`。

## Issue 与 PR 描述

### Issue

- 只写现象、复现步骤和期望结果，简洁清楚。
- 不要给别人派活，也不要写与问题无关的内容。
- 回复保持简短直接，不要堆砌冗长的分析。
- 不要泄露个人信息，包括目录结构、账号和未脱敏的日志。

### PR

- 必须用 `Closes`、`Refs` 或 `Fixed` 关联 Issue，不要用其他前缀。
- 只写这个 PR 做了什么、为什么需要、用户或开发者怎么使用，以及涉及界面时的截图或录屏。
- 描述里不要提自己的覆盖测试或单元测试。
- 不要只写 `fix bug`、`update`、`优化一下` 这种没有信息量的描述。
- 不要使用 PR 当作无关内容的堆放处。

### 不要破坏已有功能

PR 不能破坏已经存在的功能或内容。确实需要不兼容变更时：

- 在 PR 描述中写明影响范围、迁移方式和回滚方式。
- 在标题或描述中标注破坏性变更。
- 必要时先开 Issue 讨论，不要直接合并。

## AI

可以使用 AI 写代码和文档，但必须自己先看一遍、跑一遍再提交。

- 不要让 AI 把文档当成代码编写，也不要只改几个词就当成人工写的说明。
- 提交前核对事实、命令、路径、链接和版本号是否真的正确。
- 不要伪造测试结果或覆盖率数据。
- 不要提交模型生成的、与本次改动无关的大段内容。

## 不要动维护者文件

仓库根目录的维护者文件由维护者管理。

除非 Issue 或 PR 明确要求，否则不要修改、删除、重命名或顺手整理这些文件，也不要让 AI 工具把它们当成可自由编辑的普通文档。

## 许可

项目以 [GPL-3.0-or-later](LICENSE) 发布，提交 PR 即表示你同意以同一许可证授权自己的贡献。

## 安全

发现安全问题时，请不要公开细节，按 [SECURITY.md](SECURITY.md) 的方式，通过邮件发送到 me@miprota.cc。

---

提交前如果有不确定的地方，先开 Issue 问，比提交一个方向错误的 PR 更省时间。
