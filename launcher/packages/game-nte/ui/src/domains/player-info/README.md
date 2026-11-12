# 玩家信息域 (UI)

首页玩家信息卡: 胶囊开合、账号切换、体力与周本读数、错误态与登录引导.
卡片只负责展示与交互, 账号数据和刷新节奏仍由宿主 controller 调度.

## 职责

- 通过 `@host/api` 的 `playerFacts` 只读账号与游戏记录状态, 通过
  `playerActions` 读取账号、刷新游戏记录并跳到账号设置.
- 使用 `@host/ui-kit` 的 Motion 组件、`GamepadGlyph` 与宿主样式变量,
  保持首页既有的 DOM 与视觉.
- 展开状态与宿主场景动画、手柄导航共享 `playerCard.expanded`.

## 依赖

- `@host/api`: `hostFacts` (区服 / 页面 / 减少动态效果), `playerFacts`,
  `playerActions`, `playerCard` 与 `GameRecord` 类型.
- `@host/ui-kit`: `motion`, `AnimatePresence`, `MotionConfig`,
  `useReducedMotion`, `GamepadGlyph`, 以及 Anime.js 的 `createTimeline`,
  `spring`, `utils`; 卡片高度与圆角编排不单独解析包外依赖.
- 不引用宿主内部实现, 边界由 `scripts/check-ui-boundaries.mjs` 检查.

## 测试入口

```
cd launcher/ui && NODE_ENV=test pnpm test -- tests/unit/game-records.test.ts
```

## 限制

- `attendance_*` 命令与签到设置界面都已在游戏包内; 卡片只读状态并调用
  `@host/api` 的固定动作, 账号弹窗在 `domains/settings`.
- 区服以宿主已保存的设置为准, 切换区服后需等保存完成再读取记录.
