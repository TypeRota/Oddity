# fixture 游戏包

装配、契约、设置 schema 与 UI 注册表的测试包, 不面向玩家. 它同时是新增游戏包时
可以直接对照的最小实现.

## 结构

```text
game-fixture/
├── backend/                  Rust 包
│   └── src/
│       ├── lib.rs            包身份, 命令组, 能力与设置声明
│       ├── fixture.rs        三条命令的实现
│       ├── settings.rs       设置字段, 默认值, 校验与版本迁移
│       └── tests.rs          装载, 契约, 能力与设置测试
└── ui/                       Vue 包
    └── src/
        ├── pages.ts          页面声明与可见条件
        ├── contribution.ts   页面组件, 首页卡片与设置分区清单
        ├── preview.ts        三条命令的浏览器演示
        ├── index.ts          包公共出口
        ├── fixture-page.vue  页面组件
        ├── fixture-home-card.vue       首页卡片
        ├── fixture-settings-section.vue 设置分区
        ├── hello-card.vue    卡片组件
        └── greeting.ts       纯函数示例
```

## 后端

- 包 id 为 `fixture`, 显示名为 "测试游戏".
- 两个命令组: `fixture` 声明 `fixture_echo` / `fixture_fail`; `fixture-info` 声明 `fixture_info`.
- 三条命令的契约声明在 `oddity-contract`, `current_game: false`, 由命令组而不是宿主 arm 处理.
- 能力 `oddity.fixture.settings` 由本包提供并依赖; 能力只在 fixture 游戏上下文内可用.
- 不提供安装域, 启动实现与长驻域, 因此不会成为可选游戏, 也没有启动流程.

### 设置 schema

`settings.rs` 声明 owner 为 `game:fixture` 的 section:

- 字段键, 展示名, 排序, 类型, 默认值都在 `FIELDS` 中.
- 字段类型支持布尔, 整数范围, 文本长度与固定选项; 宿主按类型与范围校验.
- `validate` 做昵称非空这类跨字段校验.
- `version` 为 2; `migrate` 把版本 1 的 `displayName` 升级为 `nickname`.
- 宿主只保存 `sections["game:fixture"]` 的版本与字段值, 默认值, 校验与迁移都来自这里.

新增字段只改本包: 在 `FIELDS` 加一项, 需要时同步 `contribution.ts` 的检索项; 宿主设置页
会按 bootstrap 里的 `settingsSchemas` 自动渲染控件. 修改既有字段键或语义时提升
`version` 并在 `migrate` 里处理旧值.

## 前端

- `pages.ts` 声明 `fixture-fixture` 页面, 可见条件为 `activeGame === "fixture"`.
- `contribution.ts` 以 `namespace: "fixture"` 用 `pages.map(...)` 派生 `pageContributions`,
  并汇总首页卡片, 设置分区与检索项.
- 首页卡片和设置分区只在 fixture 激活时可见; 设置分区还要求 `oddity.fixture.settings` 能力可用.
- `fixture-settings-section.vue` 复用宿主 `DeclaredSettings` 表单, 不手写字段控件.
- `preview.ts` 声明三条包内命令与一条与异环同名的 `fetch_bulletins` 演示; 生成物把预览表
  聚合进宿主注册表, 宿主按当前游戏命名空间取用.

## 生产装配

`oddity-backend` 默认只启用 `game-nte`, 生产构建不包含 fixture 包. 需要 fixture 的测试与
演示构建显式加 `--features game-fixture`; 契约里的命令声明与 Electron 白名单保持同一份,
包未装配时命令由宿主报告为不可达.

## 新增内容

1. 命令: 在 `oddity-contract` 的 `COMMANDS` 加声明并运行 `cargo run -p contract-gen`,
   然后在包内新增 `CommandGroup` 与分发函数; 由包处理的命令不写宿主 dispatch arm.
2. 能力: 在包内定义 `CapabilityDescriptor`, 通过 `capabilities()` 提供, 用 `requires()`
   声明依赖; 装配期会校验缺失 provider, revision 与 scope.
3. 设置: 在 `settings.rs` 声明字段与可选的 `validate` / `migrate`, 在
   `GamePackage::settings()` 返回 schema; 设置值仍保存在现有 `settings.json` 的
   `sections` 映射里.
4. UI 贡献: 页面写在 `pages.ts`, 组件, 卡片与设置分区写在 `contribution.ts`; 贡献 id
   必须是 `<namespace>-<name>`. 新页面或分区的检索项与可见条件也由包声明.
5. 生成: 运行 `cargo run -p ui-registry-gen` 更新 `launcher/ui/src/host/generated/`;
   CI 用 `--check` 校验.
6. 校验: 运行包内测试, 并确认 `cargo test -p oddity-backend --features game-fixture` 与
   `cargo run -p contract-gen -- --check` 通过.
