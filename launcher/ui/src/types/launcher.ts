import type { GamepadOptions } from "./gamepad";
import type { CapabilityFact } from "../host/capability";
export interface PerformanceMetrics {
  fps: number;
  low1: number;
  p95: number;
  p99: number;
  stallPercent: number;
  vramMeanMib: number | null;
  rssMeanMib: number | null;
  gpuLoadMean: number | null;
  powerMeanW: number | null;
}
export interface PerformanceFacts {
  gameExecutable: string;
  gameVersion: string | null;
  launchMode: string;
  displayBackend: string;
  gameMode: GameModeOptions;
  vkd3dConfig: string | null;
  protonIdentity: string;
  kernel: string | null;
  cpu: string | null;
  gpu: { uuid: string; name: string; driver: string } | null;
}
export interface PerformanceEnvironmentSnapshot {
  id: string;
  name: string;
  proton: string;
}
export interface PerformanceRecord {
  id: string;
  startedAt: number;
  durationSeconds: number;
  frames: number;
  metrics: PerformanceMetrics;
  facts: PerformanceFacts;
  environment: PerformanceEnvironmentSnapshot;
  warnings: string[];
  pinned: boolean;
}
export interface PerformanceOverview {
  recordingEnabled: boolean;
  records: PerformanceRecord[];
  recording: string | null;
}
export type AttendanceTrigger = "off" | "launch" | "boot" | "daily" | "manual";
export interface AttendanceTasks {
  communitySign: boolean;
  roleSign: boolean;
  browsePosts: boolean;
  likePosts: boolean;
  sharePosts: boolean;
  cloudDuration: boolean;
}
export interface AttendancePreferences {
  trigger: AttendanceTrigger;
  time: string;
  tasks: AttendanceTasks;
}
export interface AttendanceFailure {
  kind:
    | "network"
    | "auth"
    | "verification"
    | "keyring"
    | "storage"
    | "busy"
    | "protocol"
    | "service";
  message: string;
}
export interface AttendanceReport {
  at: number;
  ok: boolean;
  steps: {
    label: string;
    ok: boolean;
    message: string;
    kind: AttendanceFailure["kind"] | null;
    optional?: boolean;
  }[];
}
export interface AttendanceAccount {
  id: string;
  maskedPhone: string;
  rememberPassword: boolean;
  signedToday: boolean;
  failedToday: boolean;
  lastReport: AttendanceReport | null;
  attention: AttendanceFailure | null;
  gameRecord?: GameRecord | null;
  gameRecordError?: AttendanceFailure | null;
}
export interface GameRecord {
  roleId: string;
  roleName: string;
  serverName: string;
  stamina: number | null;
  staminaMax: number | null;
  cityStamina: number | null;
  cityStaminaMax: number | null;
  activity: number | null;
  weeklyRemaining: number | null;
  updatedAt: number;
}
export interface AttendanceStatus {
  accounts: AttendanceAccount[];
  preferences: AttendancePreferences;
}
export interface GamescopeOptions {
  enabled: boolean;
  windowMode: "fullscreen" | "borderless" | "windowed";
  gameWidth: number;
  gameHeight: number;
  outputWidth: number;
  outputHeight: number;
  fps: number;
  scaling: "linear" | "nearest" | "fsr" | "nis" | "integer";
}
export interface GpuConfig {
  enabled: boolean;
  device: number;
  vendor: "nvidia" | "amd";
  coreOffset: number;
  memoryOffset: number;
  powerMode: number;
  amdLevel: "auto" | "high" | "low" | "profileStandard" | "profilePeak";
}
export interface GpuInfo {
  config: GpuConfig;
  devices: { id: number; vendor: GpuConfig["vendor"] }[];
}
export interface GameModeTuning {
  pinCores: string | null;
  parkCores: string | null;
  renice: number | null;
  ioprio: string | null;
}
export interface GameModeOptions {
  performance: boolean;
  inhibit: boolean;
  notifications: boolean;
  overlay: boolean;
  gamescope: GamescopeOptions;
}
export interface GameModeFeatureStatus {
  id: keyof GameModeOptions;
  available: boolean;
  state:
    | "ready"
    | "unavailable"
    | "disabled"
    | "waiting"
    | "active"
    | "requested"
    | "error"
    | "finished";
  detail: string;
}
export interface GameModeStatus {
  capabilities: GameModeFeatureStatus[];
  session: GameModeFeatureStatus[] | null;
}
export type MeteredPolicy = "ask" | "pause" | "allow";
export interface UpdateWindow {
  start: string;
  end: string;
}
export interface UpdateSettings {
  autoDownload: boolean;
  metered: MeteredPolicy;
  limitBytesPerSecond: number;
  window: UpdateWindow | null;
}
export type GameId = "nte" | "hsr" | "fixture";
export interface AudioOptions {
  device: string | null;
  volume: number;
}
export interface AudioStatus {
  devices: { index: number; name: string; label: string }[];
  defaultDevice: string | null;
  streams: {
    index: number;
    pid: number | null;
    device: number;
    volume: number;
    muted: boolean;
  }[];
  error: string | null;
}
export interface GameSettings {
  audio: AudioOptions;
  /** 当前渠道; null 表示该游戏没有渠道或尚未选择. */
  distribution: string | null;
  /** 按安装根键保存的安装目录. */
  installRoots: Record<string, string>;
  /** 自定义预下载缓存目录, 空值使用安装目录内的默认路径. */
  preloadRoot: string | null;
  launchMode: string;
  gameAccountId: string | null;
  gameMode: GameModeOptions;
  updates: UpdateSettings;
  activeEnvironment: string | null;
  stableEnvironment: string | null;
}
export type SettingsFieldValue = string | number | boolean;
/** 包声明的一个设置字段及其当前值. */
export interface SettingsFieldView {
  key: string;
  label: string;
  description: string;
  order: number;
  type: "boolean" | "integer" | "text" | "select";
  default: SettingsFieldValue;
  min?: number;
  max?: number;
  maxChars?: number;
  options?: string[];
  value: SettingsFieldValue;
}
/** 一个 owner 的设置 section 与字段. */
export interface SettingsSchemaView {
  owner: string;
  version: number;
  fields: SettingsFieldView[];
}

export interface SettingsSectionData {
  version: number;
  values: Record<string, unknown>;
}

export interface Settings {
  activeGame: GameId;
  games: Partial<Record<GameId, GameSettings>>;
  /** 按 owner 保存的声明设置 section; 没有声明 section 时省略. */
  sections?: Record<string, SettingsSectionData>;
  runtimeEnvironments: RuntimeEnvironment[];
  gamepad: GamepadOptions;
  language: string;
  reduceMotion: boolean;
  stackBulletins: boolean;
  theme: string;
  heroMedia: string;
  pauseHeroOnBlur: boolean;
  adaptiveTheme?: boolean;
  relativeHeatmap?: boolean;
  minimizeToTrayOnClose: boolean;
  showLauncherAfterGameExit: boolean;
  snapshotSuccessMinutes: number;
}
/** 按 owner 更新的设置字段; 只有该 owner 的字段会写入. */
export type SettingsPatch = Record<string, unknown>;
/** 设置 owner 更新后的视图; 游戏 owner 的 fields 形状为 GameSettings. */
export interface SettingsSection {
  owner: string;
  fields: SettingsPatch;
}
export interface GameAccount {
  accountId: string;
  maskedPhone: string;
}
export interface PlaytimeSession {
  startedAt: number;
  endedAt: number;
  mode: string;
  distribution: string;
}
export interface ActivePlaytime {
  startedAt: number;
  lastSeenAt: number;
  mode: string;
  distribution: string;
}
export interface PlaytimeHistory {
  sessions: PlaytimeSession[];
  active: ActivePlaytime | null;
}
export interface ProtonPin {
  path: string;
  identity: string;
}
export interface RuntimeEnvironment {
  id: string;
  name: string;
  proton: ProtonPin;
  installRoot: string;
  game: GameId;
  distribution: string;
  createdAt: number;
}
export interface RuntimeInstallation {
  distribution: string;
  installRoot: string;
  installed: boolean;
}
export interface CheckItem {
  label: string;
  ok: boolean;
  detail: string;
}
export type IntegrityState =
  | "checking"
  | "verified"
  | "baseline"
  | "unverified"
  | "changed"
  | "unsupported";
export type IntegritySource = "online" | "bundled";
export type IntegrityNotice = "success" | "changed";
export interface IntegrityStatus {
  state: IntegrityState;
  source: IntegritySource | null;
  checkedAt: number;
  changed: string[];
  changedTotal: number;
  notice: IntegrityNotice | null;
  appVersion: string;
  releaseUrl: string;
}
/** 当前游戏声明的一条渠道. */
export interface ChannelView {
  id: string;
  label: string;
  default: boolean;
  active: boolean;
  installRootKey: string;
}

export interface Bootstrap {
  ok: boolean;
  error: string | null;
  installRoot: string;
  launcherRoot: string;
  prefix: string;
  prefixExists: boolean;
  protonName: string;
  protonPath: string;
  umuAvailable: boolean;
  launcherVersion: string;
  launcherBuild: string;
  gameVersion: string;
  gameExe: string;
  gameExists: boolean;
  sdkPath: string;
  checks: CheckItem[];
  /** 当前游戏的能力事实, 与 bootstrap 设置同源. */
  capabilities: CapabilityFact[];
  /** 当前游戏声明的发行渠道. */
  channels: ChannelView[];
  /** 已装配包声明的设置 section 与当前值. */
  settingsSchemas?: SettingsSchemaView[];
  runtimeInstallations: RuntimeInstallation[];
  settings: Settings;
  integrity: IntegrityStatus;
  appVersion: string;
  os: string;
}
export interface CheckResult {
  installed: boolean;
  distribution: string;
  installRoot: string;
  remoteVersion: string;
  localVersion: string;
  productVersion: string;
  files: number;
  ok: number;
  missing: number;
  changed: number;
  bytes: number;
  durationMs: number;
  missingPreview: string[];
  changedPreview: string[];
}
export interface LaunchResult {
  mode: string;
  pid: number;
  exe: string;
  prefix: string;
  log: string;
  startedAt: number;
}
export type LaunchStage =
  "preparing" | "helper" | "resources" | "game" | "running";
export interface LaunchProgress {
  stage: LaunchStage;
  startedAt: number;
  cancelling: boolean;
}
export interface GameStatus {
  error?: string | null;
  running: boolean;
  closing: boolean;
  pid: number;
  startedAt: number;
  exe: string;
  log: string;
  launch?: LaunchProgress | null;
}
export type DownloadPhase =
  | "idle"
  | "checking"
  | "needs-install"
  | "needs-update"
  | "needs-verify"
  | "launcher"
  | "game"
  | "ready"
  | "preloaded"
  | "preload-unavailable"
  | "cancelled"
  | "failed";
export interface OptionalResource {
  tag: string;
  name: string;
  version: string;
  size: number;
  state: "absent" | "partial" | "installed";
  selected: boolean;
  needsUpdate: boolean;
}
export interface ResourceCatalog {
  distribution: string;
  installRoot: string;
  version: string;
  /** 公告中的未来版本, 空字符串表示当前没有资源预下载. */
  preloadVersion: string;
  preloadAvailable: boolean;
  preloadCompleted: boolean;
  preloadCached: boolean;
  preloadCachedVersion: string;
  preloadCachedBytes: number;
  preloadResourceBytes: Record<string, number>;
  resources: OptionalResource[];
}
export type MigrationPhase =
  | "idle"
  | "planning"
  | "moving"
  | "copying"
  | "verifying"
  | "committing"
  | "rebinding"
  | "cleanup"
  | "repair"
  | "done"
  | "failed"
  | "cancelled";
export interface MigrationEnvironment {
  id: string;
  name: string;
  prefix: string;
}
export interface MigrationPlan {
  distribution: string;
  mode: "move" | "copy" | "repair";
  source: string;
  target: string;
  sameFilesystem: boolean;
  files: number;
  bytes: number;
  requiredBytes: number;
  availableBytes: number;
  environments: MigrationEnvironment[];
  preloadRoot: string | null;
  warnings: string[];
}
export interface MigrationStatus {
  phase: MigrationPhase;
  distribution: string | null;
  mode: "move" | "copy" | "repair" | "";
  source: string;
  target: string;
  files: number;
  totalFiles: number;
  bytes: number;
  totalBytes: number;
  current: string;
  error: string;
  cleanupPending: boolean;
  cleanupSource: string;
  canCancel: boolean;
}
export interface ResourceTask {
  phase: "checking" | "downloading" | "ready" | "failed" | "cancelled";
  downloadedBytes: number;
  totalBytes: number;
  error: string;
}
export interface DownloadStatus {
  resourceTasks?: Record<string, ResourceTask>;
  mode: "check" | "apply" | "launcher" | "preload" | "resources";
  phase: DownloadPhase;
  /** manual 或 auto, 用于区分手动与自动任务. */
  origin: "manual" | "auto";
  /** 自动下载的等待原因, 空字符串表示没有等待. */
  autoWaiting: string;
  /** 最近一次资源检查完成时间 (Unix 毫秒), 0 表示尚未检查. */
  checkedAt: number;
  distribution: string;
  installRoot: string;
  /** 官方本地清单存在时视为已安装。 */
  installed: boolean;
  full: boolean;
  unverifiedFiles: number;
  currentFile: string;
  currentFileBytes: number;
  completedFiles: number;
  totalFiles: number;
  downloadedBytes: number;
  totalBytes: number;
  patchFiles: number;
  patchBytes: number;
  fullFiles: number;
  fullBytes: number;
  stage: string;
  remoteVersion: string;
  optionalTags: string[];
  /** 当前预下载任务是在校验已完成的缓存. */
  verify: boolean;
  pendingFiles: number;
  /** 检查时安装目录所在文件系统的可用空间, 读取失败时为 null. */
  diskAvailableBytes: number | null;
  /** 按当前资源计划估算的额外空间需求. */
  diskRequiredBytes: number;
  error: string;
}
export type BulletinKind = "bilibili" | "x" | "preview" | "maintenance";
export interface BulletinItem {
  kind: BulletinKind;
  title: string;
  summary: string;
  url: string;
  imageUrl: string | null;
  publishedAt: string;
  source: string;
}
export interface BulletinFeed {
  items: BulletinItem[];
  errors: string[];
}
export interface LauncherBackgroundMedia {
  video: string | null;
  image: string | null;
}
export interface LogPreview {
  id: string;
  fileCount: number;
  redactions: { home: number; phone: number; credentials: number; jwt: number };
  samples: { file: string; line: number; text: string }[];
  normalizedFiles: number;
}

export type SnapshotStatus =
  "starting" | "running" | "success" | "failed" | "interrupted";
export type SnapshotFailureReason = "spawn" | "earlyExit" | "interrupted";
export interface SnapshotFailure {
  reason: SnapshotFailureReason;
  exitCode: number | null;
  detail: string | null;
}
export interface SnapshotSummary {
  id: string;
  capturedAt: number;
  status: SnapshotStatus;
  durationMs: number | null;
  failure: SnapshotFailure | null;
  environment: string | null;
  proton: string | null;
  distribution: string;
  mode: string;
}
export interface SnapshotReference {
  label: string;
  savedAt: number;
  capturedAt: number;
  environment: string | null;
  proton: string | null;
  distribution: string;
  mode: string;
}
export interface SnapshotOverview {
  baseline: SnapshotSummary | null;
  reference: SnapshotReference | null;
  latest: SnapshotSummary | null;
  attempts: number;
}
export type SnapshotDiffStatus =
  "same" | "changed" | "unknown" | "added" | "removed";
export interface SnapshotDiffRow {
  key: string;
  section: string;
  label: string;
  baseline: string | null;
  current: string | null;
  status: SnapshotDiffStatus;
}
export interface SnapshotComparison {
  target: "baseline" | "reference";
  targetLabel: string;
  targetTime: number;
  targetEnvironment: string | null;
  currentEnvironment: string | null;
  rows: SnapshotDiffRow[];
  changed: number;
  unknown: number;
  unchanged: number;
}

// 命令与参数类型由契约生成, 这里转出以保持既有引用路径.
export type { Commands, CommandArgs } from "./generated/commands";

export type PreviewPaths = Partial<
  Record<
    "home" | "dataHome" | "steamRoot" | "installRoot" | "protonName",
    string
  >
>;
declare global {
  const __APP_VERSION__: string;
  interface Navigator {
    readonly userAgentData?: { readonly platform: string };
  }
  interface Window {
    __NTE_PREVIEW__?: PreviewPaths;
  }
}

export interface RuntimeSetupInfo {
  environment: RuntimeEnvironment | null;
  needsSetup: boolean;
  umu: string | null;
  proton: string | null;
  protonDirectory: string;
  systemUmu: boolean;
}
export interface RuntimeSetupPlan {
  umu: string;
  proton: string;
  installUmu: boolean;
  installProton: boolean;
  systemCommand: string[] | null;
  umuVersion: string;
  log: string;
}
export interface RuntimeSetupStatus {
  busy: boolean;
  progress: {
    component: "umu" | "proton" | "runtime";
    phase: string;
    detail: string;
    downloaded: number;
    total: number;
  };
  error: string | null;
  done: boolean;
}
