<!-- 由 launcher/tools/contract-gen 生成, 请勿手改. -->
<!-- 声明源: launcher/crates/oddity-contract/src/lib.rs. -->

# 契约目录

受众: 渲染层可经 Electron 白名单调用; 仅宿主只由 Electron 主进程调用; 窗口只在 Electron 窗口层处理, 不经过后端.

## 命令

| 命令 | 受众 | 需要当前游戏 | 参数 | 返回 |
| --- | --- | --- | --- | --- |
| `runtime_setup_info` | 渲染层 | 否 | `undefined` | `RuntimeSetupInfo` |
| `runtime_setup_plan` | 渲染层 | 否 | `{ allUsers: boolean; proton: string \| null; createEnvironment: boolean }` | `RuntimeSetupPlan` |
| `runtime_setup_apply` | 渲染层 | 否 | `undefined` | `null` |
| `runtime_setup_status` | 渲染层 | 否 | `undefined` | `RuntimeSetupStatus` |
| `runtime_setup_dismiss` | 渲染层 | 否 | `undefined` | `null` |
| `runtime_setup_environment` | 渲染层 | 是 | `undefined` | `Bootstrap` |
| `game_accounts` | 渲染层 | 是 | `undefined` | `GameAccount[]` |
| `game_send_code` | 渲染层 | 是 | `{ phone: string }` | `string` |
| `game_login` | 渲染层 | 是 | `{ phone: string; secret: string; password: boolean; deviceId: string }` | `GameAccount` |
| `game_account_remove` | 渲染层 | 是 | `{ accountId: string }` | `null` |
| `browse_files` | 渲染层 | 否 | `{ path: string \| null }` | `FileListing` |
| `gamepad_poll` | 渲染层 | 否 | `undefined` | `GamepadSnapshot` |
| `gamepad_release_idle` | 渲染层 | 否 | `undefined` | `null` |
| `gamepad_feedback` | 渲染层 | 否 | `{ deviceId: number; effect: Feedback \| "stop" }` | `null` |
| `gamepad_profile_validate` | 渲染层 | 否 | `{ profile: Profile }` | `Profile` |
| `gamepad_profile_load` | 渲染层 | 否 | `{ path: string }` | `Profile` |
| `gamepad_profile_save` | 渲染层 | 否 | `{ path: string; profile: Profile }` | `void` |
| `attendance_status` | 渲染层 | 否 | `undefined` | `AttendanceStatus` |
| `attendance_refresh_game_records` | 渲染层 | 是 | `undefined` | `AttendanceStatus` |
| `attendance_login` | 渲染层 | 是 | `{ phone: string; password: string; rememberPassword: boolean }` | `AttendanceStatus` |
| `attendance_send_code` | 渲染层 | 是 | `{ phone: string }` | `string` |
| `attendance_login_sms` | 渲染层 | 是 | `{ phone: string; code: string; deviceId: string }` | `AttendanceStatus` |
| `attendance_configure` | 渲染层 | 否 | `{ preferences: AttendancePreferences }` | `AttendanceStatus` |
| `attendance_run` | 渲染层 | 是 | `undefined` | `AttendanceStatus` |
| `attendance_logout` | 渲染层 | 否 | `{ accountId: string }` | `AttendanceStatus` |
| `attendance_keyring_check` | 渲染层 | 否 | `undefined` | `void` |
| `launcher_background` | 渲染层 | 是 | `undefined` | `LauncherBackgroundMedia` |
| `bootstrap` | 渲染层 | 否 | `undefined` | `Bootstrap` |
| `integrity_acknowledge` | 渲染层 | 否 | `{ notice: IntegrityNotice }` | `null` |
| `check_update` | 渲染层 | 是 | `{ full: boolean }` | `CheckResult` |
| `fetch_bulletins` | 渲染层 | 是 | `{ distribution: "cn" \| "global" }` | `BulletinFeed` |
| `update_settings_section` | 渲染层 | 是 | `{ owner: string; patch: SettingsPatch }` | `SettingsSection` |
| `environment_create` | 渲染层 | 是 | `{ distribution: string; name: string; protonPath: string; sourceId: string \| null }` | `Bootstrap` |
| `environment_action` | 渲染层 | 是 | `{ action: "switch" \| "keep" \| "discard"; id: string \| null }` | `Bootstrap` |
| `migration_plan` | 渲染层 | 是 | `{ distribution: string; target: string }` | `MigrationPlan` |
| `migration_start` | 渲染层 | 是 | `{ distribution: string; target: string }` | `MigrationStatus` |
| `migration_status` | 渲染层 | 是 | `undefined` | `MigrationStatus` |
| `migration_cancel` | 渲染层 | 是 | `undefined` | `MigrationStatus` |
| `migration_cleanup` | 渲染层 | 是 | `undefined` | `MigrationStatus` |
| `migration_abandon` | 渲染层 | 是 | `undefined` | `MigrationStatus` |
| `migration_dismiss` | 渲染层 | 是 | `undefined` | `MigrationStatus` |
| `migration_retry` | 渲染层 | 是 | `undefined` | `MigrationStatus` |
| `prepare_log_export` | 渲染层 | 否 | `undefined` | `LogPreview` |
| `save_log_export` | 渲染层 | 否 | `{ id: string; destination: string }` | `string` |
| `discard_log_export` | 渲染层 | 否 | `{ id: string }` | `void` |
| `environment_snapshot_overview` | 渲染层 | 是 | `undefined` | `SnapshotOverview` |
| `environment_snapshot_compare` | 渲染层 | 是 | `{ target: "baseline" \| "reference" }` | `SnapshotComparison` |
| `environment_snapshot_save_reference` | 渲染层 | 是 | `{ label: string \| null; attemptId: string \| null }` | `SnapshotReference` |
| `environment_snapshot_delete_reference` | 渲染层 | 是 | `undefined` | `void` |
| `environment_snapshot_clear_history` | 渲染层 | 是 | `undefined` | `void` |
| `open_directory` | 渲染层 | 否 | `{ path: string }` | `null` |
| `open_url` | 渲染层 | 否 | `{ url: string }` | `null` |
| `launch_official` | 渲染层 | 是 | `undefined` | `LaunchResult` |
| `cancel_launch` | 渲染层 | 是 | `undefined` | `GameStatus` |
| `game_status` | 渲染层 | 是 | `undefined` | `GameStatus` |
| `take_launch_error` | 渲染层 | 是 | `undefined` | `string \| null` |
| `performance_overview` | 渲染层 | 是 | `undefined` | `PerformanceOverview` |
| `performance_recording` | 渲染层 | 是 | `{ enabled: boolean }` | `null` |
| `performance_pin` | 渲染层 | 是 | `{ id: string; pinned: boolean }` | `null` |
| `performance_delete` | 渲染层 | 是 | `{ id: string }` | `null` |
| `performance_report` | 渲染层 | 是 | `{ a: string; b: string }` | `string` |
| `performance_export` | 渲染层 | 是 | `{ a: string; b: string; destination: string }` | `string` |
| `playtime_history` | 渲染层 | 否 | `undefined` | `PlaytimeHistory` |
| `playtime_export_poster` | 渲染层 | 否 | `{ destination: string; data: string }` | `string` |
| `audio_status` | 渲染层 | 是 | `Record<string, never>` | `AudioStatus` |
| `game_mode_status` | 渲染层 | 是 | `undefined` | `GameModeStatus` |
| `game_mode_gpu_info` | 渲染层 | 是 | `undefined` | `GpuInfo` |
| `game_mode_tuning_info` | 渲染层 | 是 | `undefined` | `GameModeTuning` |
| `game_mode_tuning_apply` | 渲染层 | 是 | `{ config: GameModeTuning }` | `GameModeTuning` |
| `game_mode_gpu_apply` | 渲染层 | 是 | `{ config: GpuConfig }` | `GpuInfo` |
| `game_resource_catalog` | 渲染层 | 是 | `undefined` | `ResourceCatalog` |
| `game_resource_remove` | 渲染层 | 是 | `{ tag: string; preload: boolean }` | `ResourceCatalog` |
| `game_resource_download` | 渲染层 | 是 | `{ tag: string }` | `DownloadStatus` |
| `game_preload_delete` | 渲染层 | 是 | `undefined` | `ResourceCatalog` |
| `game_download_status` | 渲染层 | 是 | `undefined` | `DownloadStatus` |
| `game_download_start` | 渲染层 | 是 | `{ mode: "check" \| "apply" \| "launcher" \| "preload"; full: boolean }` | `DownloadStatus` |
| `game_download_cancel` | 渲染层 | 是 | `undefined` | `DownloadStatus` |
| `close_game` | 渲染层 | 是 | `undefined` | `GameStatus` |
| `fixture_echo` | 渲染层 | 否 | `{ message: string }` | `{ message: string; length: number }` |
| `fixture_info` | 渲染层 | 否 | `undefined` | `{ game: "fixture"; name: string; capabilities: string[] }` |
| `fixture_fail` | 渲染层 | 否 | `{ message: string }` | `null` |
| `host_state` | 仅宿主 | 否 | `{ focused: boolean; visible: boolean }` | `null` |
| `external_launch` | 仅宿主 | 否 | `{ argv: string[]; cold: boolean }` | `null` |
| `win_minimize` | 窗口 | 否 | `undefined` | `null` |
| `win_toggle_maximize` | 窗口 | 否 | `undefined` | `null` |
| `win_close` | 窗口 | 否 | `undefined` | `null` |
| `win_hide` | 窗口 | 否 | `undefined` | `null` |

## 事件

| 事件 | 发射领域 |
| --- | --- |
| `game-download-state` | 下载 |
| `game-state` | 游戏进程 |
| `host-exit` | 宿主窗口 |
| `host-hide` | 宿主窗口 |
| `host-show` | 宿主窗口 |
| `integrity-state` | 完整性 |
| `migration-state` | 环境迁移 |
| `performance-updated` | 性能记录 |
| `playtime-started` | 游玩时长 |
| `runtime-setup-state` | 运行时准备 |
| `settings-changed` | 设置 |
| `task-progress` | 后台任务 |

事件只声明名称与发射领域; 发射调用必须使用这里声明的事件名, 且每条声明至少被发射一次.
发送走宿主的事件出口 `oddity_runtime::EventSink`; payload 类型仍由渲染层手写.
