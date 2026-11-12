# oddity-contract

`oddity-contract` 是宿主命令与事件的唯一声明源. `launcher/tools/contract-gen` 从这里生成界面侧命令表、TypeScript 类型、演示命令键和契约目录; 生成物文件头标明禁止手改.

## 生成物

以下文件都由 `contract-gen` 从 `src/lib.rs` 生成:

- `launcher/ui/electron/commands.ts`: Electron 白名单.
- `launcher/ui/src/types/generated/commands.ts`: `Commands` 与 `CommandArgs` 类型.
- `launcher/ui/src/core/generated/command-ids.ts`: 演示命令键.
- `launcher/CONTRACTS.md`: 命令与事件目录.

## 命令归属

`Command.owner` 声明命令由谁处理:

- `None`: 宿主实现或通用命令. `oddity-host` 的 `dispatch_command_ids` 只从这些非窗口命令派生,
  每条都必须有对应的 `dispatch` arm.
- `Some(id)`: 由包 id 为 `id` 的游戏包命令组处理; `id` 必须在 [`PACKAGE_IDS`](src/lib.rs) 里.
  命令组的 `game` 必须与 owner 相同, 跨包认领会在装配校验时失败.
- 窗口命令由窗口层处理, owner 必须是 `None`.
- 包未装配时, 该包的命令随包缺席, 不要求宿主或其它组认领.

## 新增命令

Rust 侧改三处:

1. 在 `src/lib.rs` 的 `COMMANDS` 里加一条 `Command`, 填好 audience、current_game、owner、args 和 result.
   宿主或通用命令的 owner 为 `None`, 包命令的 owner 取 `PACKAGE_IDS` 里的包 id.
2. 命令由宿主处理时, 在 `launcher/crates/oddity-host/src/protocol.rs` 的 `dispatch` 里加对应 arm;
   由游戏包处理时, 在包的 `CommandGroup` 里声明命令并由注册表路由, 组的 game 与 owner 一致, 不要再加 arm.
3. 实现 handler: 宿主 arm 多委托给各模块函数, 游戏包命令留在包内.

然后运行 `cargo run -p contract-gen` 重新生成并提交. 由游戏包处理的命令, 浏览器演示在包的 `ui/src/preview.ts` 里按命令名声明, 宿主按当前游戏命名空间取用; 由宿主处理的通用命令留在 `launcher/ui/src/core/api.ts` 的 `previewInvoke`. 两条路径都没覆盖时 `never` 检查会让 TypeScript 编译失败.

## 事件

`EVENTS` 只声明事件名与发射领域, 不记录发射点数量. 宿主通过 `oddity_runtime::EventSink` 的 `emit(id, payload)` 发送事件, payload 是 serde 值; 游戏包经宿主能力拿到同一出口, 不自行写管道.

契约测试扫描 `launcher/crates/` 与 `launcher/packages/*/backend/` 下的 Rust 源码, 跳过名为 `tests` 的目录, 注释与字符串里的文本不算. 要求:

- 每个 `.emit("<事件名>", ...)` 使用的事件名都已在 `EVENTS` 里声明;
- 每条声明的事件至少被发射一次;
- 发射调用的第一个参数是字面量事件名, 否则无法核对.

与生产代码同文件的内联 `#[cfg(test)]` 模块按普通代码计入, 新增测试请放进 `tests` 目录. 发射点数量不作断言, 也不写进契约.

## 已知缺口

- 事件 payload 类型仍由渲染层手写; 契约只声明事件名与发射领域, 不声明 payload.
- dispatch 本体未生成, 新增命令仍要手写 arm; 测试只核对 arm 覆盖, 不核对参数与返回的处理.
- `Named` 类型引用 `launcher.ts` 等手写类型定义, 契约不重复定义这些业务类型的字段形状.

## 检查生成物

在 `launcher/` 下运行:

```bash
cargo run -p contract-gen -- --check
```

`--check` 不写盘, 只比对生成物; 不一致时打印第一处差异并以非零码退出. 需要更新生成物时去掉 `--check`.

两条质量流水线都在格式检查之前跑这项检查:

- `.gitea/workflows/frontend-quality.yml`: `pnpm install` 之后、`pnpm run format:check` 之前.
- `.gitea/workflows/rust-quality.yml`: 安装并构建前端之后、`cargo fmt` 与 `cargo clippy` 之前.

CI 里用 `cargo run --locked -p contract-gen -- --check`.
