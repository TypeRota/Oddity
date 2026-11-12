# 更新页 (UI)

游戏资源更新页: 检查/下载/修复启动器与游戏资源, 预下载, 可选语音资源.
页面与宿主设置里的更新策略 (自动下载/计费/时段/限速) 同属一个领域.

## 职责

- `UpdatePage` 声明为包页面 (drawer, order 2), 只在异环激活时进入导航与页面
  白名单; 切到 fixture 时随包贡献一起卸载.
- 页面读取宿主只读更新状态与动作 (`@host/api` 的 `update` 命名空间):
  下载状态/进度/磁盘预算/资源目录, 以及检查、开始/取消下载、删除预下载与
  保存设置等动作; 宿主 controller 仍是状态机与命令调度 owner.
- `OptionalResources` 是页面内的可选资源区, 资源目录与下载任务由宿主提供.

## 依赖

- `@host/api`: `update` 命名空间 (状态、视图与动作) 与契约类型
  (`DownloadPhase`, `OptionalResource`).
- `@host/ui-kit`: 基础控件与动效工具.
- 不引用宿主内部实现; 边界由 `scripts/check-ui-boundaries.mjs` 检查.

## 测试入口

```
cd launcher/ui && pnpm exec vitest run tests/unit/update-page.test.ts
```

页面声明可见性与装载由宿主 vitest 入口覆盖, 覆盖率报告包含包内文件.
