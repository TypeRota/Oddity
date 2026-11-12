// 由 launcher/tools/contract-gen 生成, 请勿手改.
// 声明源: launcher/crates/oddity-contract/src/lib.rs.
import type { FileListing } from "../../core/filePicker";
import type { Feedback, GamepadSnapshot, Profile } from "../gamepad";
import type {
  AttendancePreferences,
  AttendanceStatus,
  AudioStatus,
  Bootstrap,
  BulletinFeed,
  CheckResult,
  DownloadStatus,
  GameAccount,
  GameModeStatus,
  GameModeTuning,
  GameStatus,
  GpuConfig,
  GpuInfo,
  IntegrityNotice,
  LaunchResult,
  LauncherBackgroundMedia,
  LogPreview,
  MigrationPlan,
  MigrationStatus,
  PerformanceOverview,
  PlaytimeHistory,
  ResourceCatalog,
  RuntimeSetupInfo,
  RuntimeSetupPlan,
  RuntimeSetupStatus,
  SettingsPatch,
  SettingsSection,
  SnapshotComparison,
  SnapshotOverview,
  SnapshotReference,
} from "../launcher";

export interface Commands {
  runtime_setup_info: RuntimeSetupInfo;
  runtime_setup_plan: RuntimeSetupPlan;
  runtime_setup_apply: null;
  runtime_setup_status: RuntimeSetupStatus;
  runtime_setup_dismiss: null;
  runtime_setup_environment: Bootstrap;
  game_accounts: GameAccount[];
  game_send_code: string;
  game_login: GameAccount;
  game_account_remove: null;
  browse_files: FileListing;
  gamepad_poll: GamepadSnapshot;
  gamepad_release_idle: null;
  gamepad_feedback: null;
  gamepad_profile_validate: Profile;
  gamepad_profile_load: Profile;
  gamepad_profile_save: void;
  attendance_status: AttendanceStatus;
  attendance_refresh_game_records: AttendanceStatus;
  attendance_login: AttendanceStatus;
  attendance_send_code: string;
  attendance_login_sms: AttendanceStatus;
  attendance_configure: AttendanceStatus;
  attendance_run: AttendanceStatus;
  attendance_logout: AttendanceStatus;
  attendance_keyring_check: void;
  launcher_background: LauncherBackgroundMedia;
  bootstrap: Bootstrap;
  integrity_acknowledge: null;
  check_update: CheckResult;
  fetch_bulletins: BulletinFeed;
  update_settings_section: SettingsSection;
  environment_create: Bootstrap;
  environment_action: Bootstrap;
  migration_plan: MigrationPlan;
  migration_start: MigrationStatus;
  migration_status: MigrationStatus;
  migration_cancel: MigrationStatus;
  migration_cleanup: MigrationStatus;
  migration_abandon: MigrationStatus;
  migration_dismiss: MigrationStatus;
  migration_retry: MigrationStatus;
  prepare_log_export: LogPreview;
  save_log_export: string;
  discard_log_export: void;
  environment_snapshot_overview: SnapshotOverview;
  environment_snapshot_compare: SnapshotComparison;
  environment_snapshot_save_reference: SnapshotReference;
  environment_snapshot_delete_reference: void;
  environment_snapshot_clear_history: void;
  open_directory: null;
  open_url: null;
  launch_official: LaunchResult;
  cancel_launch: GameStatus;
  game_status: GameStatus;
  take_launch_error: string | null;
  performance_overview: PerformanceOverview;
  performance_recording: null;
  performance_pin: null;
  performance_delete: null;
  performance_report: string;
  performance_export: string;
  playtime_history: PlaytimeHistory;
  playtime_export_poster: string;
  audio_status: AudioStatus;
  game_mode_status: GameModeStatus;
  game_mode_gpu_info: GpuInfo;
  game_mode_tuning_info: GameModeTuning;
  game_mode_tuning_apply: GameModeTuning;
  game_mode_gpu_apply: GpuInfo;
  game_resource_catalog: ResourceCatalog;
  game_resource_remove: ResourceCatalog;
  game_resource_download: DownloadStatus;
  game_preload_delete: ResourceCatalog;
  game_download_status: DownloadStatus;
  game_download_start: DownloadStatus;
  game_download_cancel: DownloadStatus;
  close_game: GameStatus;
  fixture_echo: { message: string; length: number };
  fixture_info: { game: "fixture"; name: string; capabilities: string[] };
  fixture_fail: null;
  win_minimize: null;
  win_toggle_maximize: null;
  win_close: null;
  win_hide: null;
}

export interface CommandArgs {
  runtime_setup_info: undefined;
  runtime_setup_plan: {
    allUsers: boolean;
    proton: string | null;
    createEnvironment: boolean;
  };
  runtime_setup_apply: undefined;
  runtime_setup_status: undefined;
  runtime_setup_dismiss: undefined;
  runtime_setup_environment: undefined;
  game_accounts: undefined;
  game_send_code: { phone: string };
  game_login: {
    phone: string;
    secret: string;
    password: boolean;
    deviceId: string;
  };
  game_account_remove: { accountId: string };
  browse_files: { path: string | null };
  gamepad_poll: undefined;
  gamepad_release_idle: undefined;
  gamepad_feedback: { deviceId: number; effect: Feedback | "stop" };
  gamepad_profile_validate: { profile: Profile };
  gamepad_profile_load: { path: string };
  gamepad_profile_save: { path: string; profile: Profile };
  attendance_status: undefined;
  attendance_refresh_game_records: undefined;
  attendance_login: {
    phone: string;
    password: string;
    rememberPassword: boolean;
  };
  attendance_send_code: { phone: string };
  attendance_login_sms: { phone: string; code: string; deviceId: string };
  attendance_configure: { preferences: AttendancePreferences };
  attendance_run: undefined;
  attendance_logout: { accountId: string };
  attendance_keyring_check: undefined;
  launcher_background: undefined;
  bootstrap: undefined;
  integrity_acknowledge: { notice: IntegrityNotice };
  check_update: { full: boolean };
  fetch_bulletins: { distribution: "cn" | "global" };
  update_settings_section: { owner: string; patch: SettingsPatch };
  environment_create: {
    distribution: string;
    name: string;
    protonPath: string;
    sourceId: string | null;
  };
  environment_action: {
    action: "switch" | "keep" | "discard";
    id: string | null;
  };
  migration_plan: { distribution: string; target: string };
  migration_start: { distribution: string; target: string };
  migration_status: undefined;
  migration_cancel: undefined;
  migration_cleanup: undefined;
  migration_abandon: undefined;
  migration_dismiss: undefined;
  migration_retry: undefined;
  prepare_log_export: undefined;
  save_log_export: { id: string; destination: string };
  discard_log_export: { id: string };
  environment_snapshot_overview: undefined;
  environment_snapshot_compare: { target: "baseline" | "reference" };
  environment_snapshot_save_reference: {
    label: string | null;
    attemptId: string | null;
  };
  environment_snapshot_delete_reference: undefined;
  environment_snapshot_clear_history: undefined;
  open_directory: { path: string };
  open_url: { url: string };
  launch_official: undefined;
  cancel_launch: undefined;
  game_status: undefined;
  take_launch_error: undefined;
  performance_overview: undefined;
  performance_recording: { enabled: boolean };
  performance_pin: { id: string; pinned: boolean };
  performance_delete: { id: string };
  performance_report: { a: string; b: string };
  performance_export: { a: string; b: string; destination: string };
  playtime_history: undefined;
  playtime_export_poster: { destination: string; data: string };
  audio_status: Record<string, never>;
  game_mode_status: undefined;
  game_mode_gpu_info: undefined;
  game_mode_tuning_info: undefined;
  game_mode_tuning_apply: { config: GameModeTuning };
  game_mode_gpu_apply: { config: GpuConfig };
  game_resource_catalog: undefined;
  game_resource_remove: { tag: string; preload: boolean };
  game_resource_download: { tag: string };
  game_preload_delete: undefined;
  game_download_status: undefined;
  game_download_start: {
    mode: "check" | "apply" | "launcher" | "preload";
    full: boolean;
  };
  game_download_cancel: undefined;
  close_game: undefined;
  fixture_echo: { message: string };
  fixture_info: undefined;
  fixture_fail: { message: string };
  win_minimize: undefined;
  win_toggle_maximize: undefined;
  win_close: undefined;
  win_hide: undefined;
}
