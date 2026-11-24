# 设置分区域 (UI)

"游戏与启动" 与 "账号与签到" 两个设置分区, 以及签到账号弹窗与任务胶囊.
分区只负责渲染与交互, 设置保存、签到命令与锁状态仍由宿主 controller 调度.

## 职责

- 声明两个分区的 id、标题、图标、顺序、检索项与可见条件, 由包入口导出;
  宿主设置页按贡献表装配, 只在异环激活时挂载.
- `GameSettingsSection` 绑定 `form` 里的游戏字段 (地区、安装目录、启动方式、
  背景素材、游戏账号、资源更新), 改动通过 `queueSettingsSave("game")` 交给宿主保存队列.
- `BackgroundLibrary` 管理启动页背景素材: 归档开关、归档目录、保留套数与缩略图切换.
  偏好与条目由 `launcher_background_archive` / `launcher_background_select` /
  `launcher_background_configure` 读写, 切换后调用 `reloadHeroBackground`
  让主视觉重新解析.
- `AccountSettingsSection` / `TaygedoSettings` / `TaygedoAccountDialog` 管理
  签到账号、自动签到与任务授权; 账号状态与 `attendance_*` 命令仍由宿主提供.
- `attendanceOptions.ts` 保存任务授权的标签与完成判定; 宿主只保留默认授权字段.

## 依赖

- `@host/api`: `form`, `hostFacts`, 锁状态 (`settingsPathsLocked`、
  `runtimePathsLocked`、`migrationBusy`、`runtimeEnvironmentBusy`)、
  `queueSettingsSave`, `queueAttendanceSave`, `attendance`, `attendanceForm`,
  `loadAttendance`, `runAttendance`, `settingsNavigation`, `isDesktop`,
  `invoke`, `notify`, `reloadHeroBackground` 与契约类型.
- `@host/ui-kit`: 基础控件 (`BaseInput`, `BaseSelect`, `BaseSwitch`,
  `BaseDisclosure`, `BaseModal`, `BaseTabs`, `BaseView`), 表单容器
  (`SettingGroup`, `SettingRow`), `DirectoryField`, `GameAccountDialog` 与
  `InstallMigration`.
- 不引用宿主内部实现, 边界由 `scripts/check-ui-boundaries.mjs` 检查.

## 样式

分区沿用宿主全局样式表 (`launcher/ui/src/styles/account.css`,
`settings.css` 等) 里的类名, 包内不重复定义视觉规范, 迁移前后 DOM 与外观一致.

## 测试入口

```
cd launcher/ui && NODE_ENV=test pnpm test -- tests/unit/settings-sections.test.ts tests/unit/background-library.test.ts tests/unit/attendance.test.ts
```

## 限制

- 签到账号与偏好仍由宿主 controller 持久化, 包内不缓存状态.
- 生效中的签到任务名由后端返回, 任务完成判定按这些名字匹配.
- 背景素材的归档结果由后端持有, 包内只保留当前弹窗的展示副本.
