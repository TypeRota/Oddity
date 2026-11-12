import { defaultGamepadOptions } from "../types/gamepad";
import { defaultGamescope } from "./gameMode";
import { distributions } from "./distribution";
import { factsFrom } from "../host/facts";
import { gameInstallRootHint } from "../host/pages";
import { previewHandler, type HostPreviewCommand } from "../host/preview";
import { APP_VERSION } from "./version";
import { nativeInvoke, isDesktop, listen } from "./desktop";
import { applyDemoPreview, isDemo } from "./demo";
import type {
  Bootstrap,
  CheckResult,
  GameStatus,
  LaunchProgress,
  LaunchStage,
  DownloadStatus,
  MigrationStatus,
  ResourceCatalog,
  OptionalResource,
  ResourceTask,
  Commands,
  CommandArgs,
  PerformanceOverview,
  PerformanceRecord,
  PreviewPaths,
  PlaytimeHistory,
  PlaytimeSession,
  SnapshotComparison,
  SnapshotDiffRow,
  SnapshotOverview,
  SnapshotReference,
  SnapshotSummary,
  IntegrityStatus,
  GameId,
  GameSettings,
  RuntimeSetupPlan,
  RuntimeSetupStatus,
} from "../types/launcher";
import { activeGame, gameSettings } from "./game";
import type { UiCommand } from "./generated/command-ids";
export { isDesktop };
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));

const PREVIEW_KEYS = [
  "home",
  "dataHome",
  "steamRoot",
  "installRoot",
  "protonName",
] as const;

function readStoredPreview(): PreviewPaths {
  try {
    return (
      JSON.parse(window.localStorage.getItem("nte.preview.paths") || "{}") || {}
    );
  } catch {
    return {};
  }
}

function readQueryPreview() {
  const params = new URLSearchParams(window.location.search);
  const query: PreviewPaths = {};
  for (const key of PREVIEW_KEYS) {
    const value = params.get(key);
    if (value && value.trim()) query[key] = value.trim();
  }
  return query;
}

// 预览路径按查询参数、本地存储、窗口配置的顺序覆盖默认值。
function previewOverrides() {
  const sources = [
    window.__NTE_PREVIEW__,
    readStoredPreview(),
    readQueryPreview(),
  ];
  const overrides: PreviewPaths = {};
  for (const source of sources) {
    if (!source || typeof source !== "object") continue;
    for (const key of PREVIEW_KEYS) {
      if (typeof source[key] === "string" && source[key].trim()) {
        overrides[key] = source[key].trim();
      }
    }
  }
  return overrides;
}

const joinPath = (...parts: string[]) =>
  parts
    .filter((part) => part != null && part !== "")
    .map((part, index) =>
      index === 0
        ? String(part).replace(/\/+$/, "")
        : String(part).replace(/^\/+|\/+$/g, ""),
    )
    .join("/");

const previewFacts = factsFrom(null);

// 默认安装路径取游戏包声明; 未声明的游戏退回通用目录, 宿主不猜游戏名.
function previewInstallRoot(distribution: "cn" | "global", home = "~"): string {
  const hint = gameInstallRootHint({ ...previewFacts, distribution });
  if (!hint) return joinPath(home, "Games");
  return home === "~" || !hint.startsWith("~")
    ? hint
    : joinPath(home, hint.replace(/^~\/?/, ""));
}

function previewPaths() {
  const overrides = previewOverrides();
  const home = overrides.home || "~";
  const dataHome = overrides.dataHome || joinPath(home, ".local/share");
  const steamRoot = overrides.steamRoot || joinPath(dataHome, "Steam");
  const installRoot = overrides.installRoot || previewInstallRoot("cn", home);
  const protonName = overrides.protonName || "GE-Proton11-5-x86_64";
  return {
    dataHome,
    steamRoot,
    installRoot,
    launcherRoot: joinPath(installRoot, distributions.cn.launcherDir),
    prefix: joinPath(dataHome, "oddity/prefix"),
    protonName,
    protonPath: joinPath(
      steamRoot,
      "compatibilitytools.d",
      protonName,
      "proton",
    ),
    gameExe: joinPath(
      installRoot,
      "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
    ),
    sdkPath: joinPath(
      installRoot,
      distributions.cn.launcherDir,
      "WPGameClientSDK_64.dll",
    ),
  };
}

function previewOs() {
  const platform =
    navigator.userAgentData?.platform || navigator.platform || "";
  const family = /win/i.test(platform)
    ? "windows"
    : /mac/i.test(platform)
      ? "macos"
      : "linux";
  const arch = /arm64|aarch64/i.test(navigator.userAgent)
    ? "aarch64"
    : "x86_64";
  return `${family} / ${arch} / browser preview`;
}

const preview = previewPaths();

// 浏览器预览固定为异环国服, 能力事实与后端返回同形.
const previewCapabilities: Bootstrap["capabilities"] = [
  {
    id: "oddity.account.game",
    revision: 1,
    state: "available",
    code: null,
    message: null,
  },
  {
    id: "oddity.profile.player",
    revision: 1,
    state: "available",
    code: null,
    message: null,
  },
];

const mock: Bootstrap = {
  ok: false,
  error: "请先在设置中创建并选择 Proton 运行环境",
  installRoot: preview.installRoot,
  launcherRoot: preview.launcherRoot,
  prefix: "",
  prefixExists: false,
  protonName: "",
  protonPath: "",
  umuAvailable: true,
  launcherVersion: "1.0.7.0924",
  launcherBuild: "1fba17f8",
  gameVersion: "1.4.16",
  gameExe: preview.gameExe,
  gameExists: true,
  sdkPath: preview.sdkPath,
  capabilities: previewCapabilities,
  checks: [
    { label: "启动器配置", ok: true, detail: preview.launcherRoot },
    { label: "游戏可执行文件", ok: true, detail: preview.gameExe },
    { label: "umu-launcher", ok: true, detail: "umu-run（PATH）" },
    { label: "Proton", ok: false, detail: "未选择运行环境" },
    { label: "Wine prefix", ok: false, detail: "" },
    {
      label: "官方启动器",
      ok: true,
      detail: joinPath(preview.launcherRoot, distributions.cn.clientExe),
    },
  ],
  channels: [
    {
      id: "cn",
      label: "国服",
      default: true,
      active: true,
      installRootKey: "cn",
    },
    {
      id: "global",
      label: "国际服",
      default: false,
      active: false,
      installRootKey: "global",
    },
  ],
  runtimeInstallations: [
    { distribution: "cn", installRoot: preview.installRoot, installed: true },
    {
      distribution: "global",
      installRoot: previewInstallRoot("global"),
      installed: false,
    },
  ],
  settings: {
    activeGame: "nte",
    games: {
      nte: {
        audio: { device: null, volume: 100 },
        distribution: "cn",
        installRoots: {},
        preloadRoot: null,
        launchMode: "background",
        gameAccountId: null,
        gameMode: {
          performance: false,
          inhibit: false,
          notifications: false,
          overlay: false,
          gamescope: { ...defaultGamescope },
        },
        updates: {
          autoDownload: false,
          metered: "ask",
          limitBytesPerSecond: 0,
          window: null,
        },
        activeEnvironment: null,
        stableEnvironment: null,
      },
    },
    gamepad: defaultGamepadOptions(),
    runtimeEnvironments: [],
    language: "zh-CN",
    reduceMotion: false,
    stackBulletins: false,
    theme: "light",
    heroMedia: "video",
    pauseHeroOnBlur: true,
    relativeHeatmap: true,
    minimizeToTrayOnClose: true,
    showLauncherAfterGameExit: true,
    snapshotSuccessMinutes: 1,
  },
  integrity: {
    state: "verified",
    source: "online",
    checkedAt: Date.now() - 60_000,
    changed: [],
    changedTotal: 0,
    notice: null,
    appVersion: APP_VERSION,
    releaseUrl: `https://github.com/LaT-SKY/Oddity/releases/tag/oddity-v${APP_VERSION}`,
  },
  appVersion: APP_VERSION,
  os: previewOs(),
};

if (isDemo)
  applyDemoPreview(mock, {
    protonPath: preview.protonPath,
    protonName: preview.protonName,
    prefix: preview.prefix,
  });

const previewSnapshotNow = Date.now();
let previewSnapshotAttempts = 2;
let previewSnapshotBaseline: SnapshotSummary | null = {
  id: "preview-baseline",
  capturedAt: previewSnapshotNow - 86_400_000,
  status: "success",
  durationMs: 42 * 60_000,
  failure: null,
  environment: null,
  proton: "GE-Proton9-20",
  distribution: "cn",
  mode: "background",
};
let previewSnapshotLatest: SnapshotSummary | null = {
  id: "preview-latest",
  capturedAt: previewSnapshotNow - 3_600_000,
  status: "failed",
  durationMs: 4_000,
  failure: { reason: "earlyExit", exitCode: 3, detail: null },
  environment: null,
  proton: "GE-Proton9-21",
  distribution: "cn",
  mode: "background",
};
let previewSnapshotReference: SnapshotReference | null = {
  label: "更新驱动前",
  savedAt: previewSnapshotNow - 172_800_000,
  capturedAt: previewSnapshotNow - 172_800_000,
  environment: null,
  proton: "GE-Proton9-20",
  distribution: "cn",
  mode: "background",
};

function previewSnapshotEnvironment() {
  const active = mock.settings.runtimeEnvironments.find(
    (environment) =>
      environment.id === gameSettings(mock.settings).activeEnvironment,
  );
  return active?.name ?? null;
}

function previewSnapshotProton() {
  return mock.protonName || previewSnapshotBaseline?.proton || null;
}

function previewSnapshotOverview(): SnapshotOverview {
  return {
    baseline: previewSnapshotBaseline ? { ...previewSnapshotBaseline } : null,
    reference: previewSnapshotReference,
    latest: previewSnapshotLatest ? { ...previewSnapshotLatest } : null,
    attempts: previewSnapshotAttempts,
  };
}

function previewSnapshotComparison(
  target: "baseline" | "reference",
): SnapshotComparison {
  if (target === "reference" && !previewSnapshotReference)
    throw new Error("尚未保存参考快照");
  if (target === "baseline" && !previewSnapshotBaseline)
    throw new Error("还没有成功启动记录");
  const rows: SnapshotDiffRow[] = [
    {
      key: "launcher.version",
      section: "启动器",
      label: "Oddity 版本",
      baseline: "0.2.0-alpha.2",
      current: "0.2.0-alpha.2",
      status: "same",
    },
    {
      key: "system.session",
      section: "系统",
      label: "会话类型",
      baseline: "Wayland",
      current: "Wayland",
      status: "same",
    },
    {
      key: "graphics.gpu.10DE:2C58",
      section: "图形",
      label: "显卡 10DE:2C58",
      baseline: "nvidia 615.78.08",
      current: "nvidia 615.78.10",
      status: "changed",
    },
    {
      key: "graphics.opengl",
      section: "图形",
      label: "OpenGL 驱动",
      baseline: "NVIDIA GeForce RTX 5060 · 4.6.0 NVIDIA 615.78.08",
      current: "NVIDIA GeForce RTX 5060 · 4.6.0 NVIDIA 615.78.10",
      status: "changed",
    },
    {
      key: "graphics.vulkan",
      section: "图形",
      label: "Vulkan 驱动",
      baseline: null,
      current: "Mesa 26.2.5",
      status: "unknown",
    },
    {
      key: "game.version",
      section: "游戏",
      label: "游戏版本",
      baseline: "1.0.7",
      current: "1.0.7",
      status: "same",
    },
    {
      key: "runtime.proton",
      section: "运行环境",
      label: "Proton",
      baseline: "GE-Proton9-20",
      current: "GE-Proton9-21",
      status: "changed",
    },
    {
      key: "runtime.gamescope",
      section: "运行环境",
      label: "Gamescope",
      baseline: "关闭",
      current: "关闭",
      status: "same",
    },
  ];
  const targetTime =
    target === "reference"
      ? previewSnapshotReference!.capturedAt
      : previewSnapshotBaseline!.capturedAt;
  return {
    target,
    targetLabel: target === "reference" ? "参考快照" : "上次成功启动",
    targetTime,
    targetEnvironment: previewSnapshotEnvironment(),
    currentEnvironment: previewSnapshotEnvironment(),
    rows,
    changed: rows.filter(
      (row) =>
        row.status === "changed" ||
        row.status === "added" ||
        row.status === "removed",
    ).length,
    unknown: rows.filter((row) => row.status === "unknown").length,
    unchanged: rows.filter((row) => row.status === "same").length,
  };
}

function previewSnapshotSaveReference(label: string | null): SnapshotReference {
  const now = Date.now();
  const reference: SnapshotReference = {
    label: label?.trim() || "参考快照",
    savedAt: now,
    capturedAt: now,
    environment: previewSnapshotEnvironment(),
    proton: previewSnapshotProton(),
    distribution: gameSettings(mock.settings).distribution ?? "cn",
    mode: gameSettings(mock.settings).launchMode,
  };
  previewSnapshotReference = reference;
  return reference;
}

function updateGameSettings(patch: Partial<GameSettings>) {
  const game = activeGame(mock.settings);
  mock.settings.games[game] = { ...gameSettings(mock.settings), ...patch };
}

/** 演示数据按安装根键取目录; 未知键回落到当前安装. */
function demoInstallRoot(
  game: GameSettings,
  distribution: string | null = game.distribution,
): string | null {
  const key =
    distribution === "global" ? "global" : distribution === "cn" ? "cn" : null;
  return key ? (game.installRoots?.[key] ?? null) : null;
}

function applyPreviewDistribution() {
  const state =
    gameSettings(mock.settings).distribution === "cn"
      ? "available"
      : "unsupported";
  mock.capabilities = previewCapabilities.map((fact) => ({ ...fact, state }));
  const game = gameSettings(mock.settings);
  if (game.distribution === "global")
    mock.installRoot =
      demoInstallRoot(game, "global") || previewInstallRoot("global");
  else mock.installRoot = demoInstallRoot(game, "cn") || preview.installRoot;
  mock.launcherRoot = joinPath(
    mock.installRoot,
    game.distribution === "global"
      ? distributions.global.launcherDir
      : distributions.cn.launcherDir,
  );
  mock.sdkPath = joinPath(mock.launcherRoot, "WPGameClientSDK_64.dll");
  mock.gameExe = joinPath(
    mock.installRoot,
    "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe",
  );
  for (const item of mock.checks) {
    if (item.label === "启动器配置") item.detail = mock.launcherRoot;
    if (item.label === "游戏可执行文件") item.detail = mock.gameExe;
    if (
      item.detail.endsWith("NTEGame.exe") ||
      item.detail.endsWith("NTEGlobalGame.exe")
    ) {
      item.detail = joinPath(
        mock.launcherRoot,
        gameSettings(mock.settings).distribution === "global"
          ? distributions.global.clientExe
          : distributions.cn.clientExe,
      );
    }
  }
  mockCheck.distribution = gameSettings(mock.settings).distribution ?? "cn";
  mockCheck.installRoot = mock.installRoot;
}

function selectPreviewEnvironment(
  profile: Bootstrap["settings"]["runtimeEnvironments"][number],
) {
  updateGameSettings({ distribution: profile.distribution });
  applyPreviewDistribution();
  updateGameSettings({ activeEnvironment: profile.id });
  mock.prefix = joinPath(
    preview.prefix.replace(/\/prefix$/, ""),
    "environments",
    profile.id,
    "prefix",
  );
  mock.prefixExists = true;
  mock.protonPath = profile.proton.path;
  mock.protonName = mock.protonPath.split("/").pop() || preview.protonName;
  mock.ok = true;
  mock.error = null;
  mock.checks[3] = { label: "Proton", ok: true, detail: mock.protonName };
  mock.checks[4] = { label: "Wine prefix", ok: true, detail: mock.prefix };
}

const mockCheck: CheckResult = {
  installed: true,
  distribution: "cn",
  installRoot: preview.installRoot,
  remoteVersion: "1.0.7.0924",
  localVersion: "1.0.7.0924",
  productVersion: "1.0.7.0924_1",
  files: 667,
  ok: 667,
  missing: 0,
  changed: 0,
  bytes: 0,
  durationMs: 342,
  missingPreview: [],
  changedPreview: [],
};

// 浏览器预览模拟会话, 原生窗口的状态由 Rust 维护。
const previewGame = {
  running: false,
  closing: false,
  pid: 0,
  startedAt: 0,
  exe: preview.gameExe,
  log: "",
  launch: null as LaunchProgress | null,
};
const previewGameListeners = new Set<(status: GameStatus) => void>();
let previewLaunchTimers: number[] = [];

function previewGameStatus() {
  return clone(previewGame);
}

function emitPreviewGameStatus() {
  const status = previewGameStatus();
  for (const handler of previewGameListeners) handler(status);
}

function clearPreviewLaunchTimers() {
  for (const timer of previewLaunchTimers) window.clearTimeout(timer);
  previewLaunchTimers = [];
}

function startPreviewLaunch() {
  clearPreviewLaunchTimers();
  const startedAt = Date.now();
  previewGame.running = true;
  previewGame.closing = false;
  previewGame.pid = 23140;
  previewGame.startedAt = startedAt;
  previewGame.launch = { stage: "preparing", startedAt, cancelling: false };
  emitPreviewGameStatus();
  const stages: { stage: LaunchStage; delay: number }[] = [
    { stage: "helper", delay: 600 },
    { stage: "resources", delay: 1300 },
    { stage: "game", delay: 3000 },
  ];
  for (const item of stages) {
    previewLaunchTimers.push(
      window.setTimeout(() => {
        if (!previewGame.launch || previewGame.launch.cancelling) return;
        previewGame.launch = { ...previewGame.launch, stage: item.stage };
        emitPreviewGameStatus();
      }, item.delay),
    );
  }
  previewLaunchTimers.push(
    window.setTimeout(() => {
      if (!previewGame.launch || previewGame.launch.cancelling) return;
      previewGame.launch = null;
      emitPreviewGameStatus();
    }, 3700),
  );
}

function previewSessions(): PlaytimeSession[] {
  const sessions: PlaytimeSession[] = [];
  const now = new Date();
  for (let offset = 11; offset >= 0; offset -= 1) {
    const month = new Date(now.getFullYear(), now.getMonth() - offset, 1);
    const count = 2 + ((offset * 3) % 4);
    for (let index = 0; index < count; index += 1) {
      const day = 3 + ((offset * 5 + index * 6) % 22);
      const startedAt = new Date(
        month.getFullYear(),
        month.getMonth(),
        day,
        19 + (index % 3),
        (index * 17) % 60,
      ).getTime();
      const endedAt =
        startedAt + (35 + ((offset * 11 + index * 23) % 150)) * 60_000;
      if (endedAt > Date.now()) continue;
      sessions.push({
        startedAt,
        endedAt,
        mode: index % 3 ? "background" : "official",
        distribution: "cn",
      });
    }
  }
  return sessions.sort((left, right) => left.startedAt - right.startedAt);
}

const previewPlaytime = previewSessions();

function previewHistory(): PlaytimeHistory {
  return {
    sessions: clone(previewPlaytime),
    active: previewGame.running
      ? {
          startedAt: previewGame.startedAt,
          lastSeenAt: Date.now(),
          mode: "background",
          distribution: "cn",
        }
      : null,
  };
}

// 浏览器预览固定的三条记录, 开关、固定和删除只改内存里的这份数据.
function previewPerformanceRecord(
  id: string,
  startedAt: number,
  environment: { name: string; proton: string },
  metrics: { fps: number; low1: number; p99: number; vramMeanMib: number },
  pinned: boolean,
): PerformanceRecord {
  const durationSeconds = 1920;
  return {
    id,
    startedAt,
    durationSeconds,
    frames: Math.round(metrics.fps * durationSeconds),
    metrics: {
      fps: metrics.fps,
      low1: metrics.low1,
      p95: metrics.p99 - 1.4,
      p99: metrics.p99,
      stallPercent: 0.8,
      vramMeanMib: metrics.vramMeanMib,
      rssMeanMib: 5240,
      gpuLoadMean: 73,
      powerMeanW: 179,
    },
    facts: {
      gameExecutable: "HTGame",
      gameVersion: "1.4.17",
      launchMode: "background",
      displayBackend: "Wayland",
      gameMode: {
        performance: false,
        inhibit: false,
        notifications: false,
        overlay: false,
        gamescope: { ...defaultGamescope },
      },
      vkd3dConfig: null,
      protonIdentity: environment.proton,
      kernel: "6.12.9",
      cpu: "AMD Ryzen 7 7800X3D 8-Core Processor",
      gpu: {
        uuid: "GPU-demo",
        name: "NVIDIA GeForce RTX 4070",
        driver: "565.77",
      },
    },
    environment: {
      id: `${id}-environment`,
      name: environment.name,
      proton: environment.proton,
    },
    warnings: [],
    pinned,
  };
}

const previewPerformance: PerformanceOverview = {
  recordingEnabled: true,
  recording: null,
  records: [
    previewPerformanceRecord(
      "preview-record-3",
      Date.now() - 40 * 60_000,
      { name: "GE-Proton11-5", proton: "GE-Proton11-5" },
      { fps: 61.8, low1: 48.3, p99: 21.9, vramMeanMib: 6380 },
      true,
    ),
    previewPerformanceRecord(
      "preview-record-2",
      Date.now() - 26 * 60 * 60_000,
      { name: "Proton Experimental", proton: "Proton-Experimental" },
      { fps: 59.2, low1: 43.6, p99: 24.2, vramMeanMib: 6420 },
      false,
    ),
    previewPerformanceRecord(
      "preview-record-1",
      Date.now() - 50 * 60 * 60_000,
      { name: "GE-Proton11-5", proton: "GE-Proton11-5" },
      { fps: 60.4, low1: 46.1, p99: 23.5, vramMeanMib: 6410 },
      false,
    ),
  ],
};

// 浏览器演示用的示意图表, 与 Rust 侧的真实报告渲染不共享实现.
function previewPerformanceReport(a: string, b: string): string {
  const base = previewPerformance.records.find((record) => record.id === a);
  const target = previewPerformance.records.find((record) => record.id === b);
  if (!base || !target) return "";
  const scale = (value: number) => Math.round(value * 5);
  const rows = [
    { label: "平均 FPS", left: base.metrics.fps, right: target.metrics.fps },
    { label: "1% Low", left: base.metrics.low1, right: target.metrics.low1 },
  ];
  const bars = rows
    .flatMap((row, index) => {
      const top = 180 + index * 140;
      return [
        `<text x="64" y="${top}" font-size="24" fill="#edf2f8">${row.label}</text>`,
        `<rect x="64" y="${top + 20}" width="${scale(row.left)}" height="34" rx="8" fill="#6b7a8c" />`,
        `<text x="${72 + scale(row.left)}" y="${top + 44}" font-size="20" fill="#aebccc">${row.left.toFixed(1)}</text>`,
        `<rect x="64" y="${top + 62}" width="${scale(row.right)}" height="34" rx="8" fill="#d5e0ec" />`,
        `<text x="${72 + scale(row.right)}" y="${top + 86}" font-size="20" fill="#aebccc">${row.right.toFixed(1)}</text>`,
      ];
    })
    .join("");
  return [
    '<svg xmlns="http://www.w3.org/2000/svg" width="960" height="480" viewBox="0 0 960 480">',
    '<rect width="960" height="480" fill="#172434" />',
    '<text x="64" y="72" font-size="30" fill="#edf2f8">性能记录对比</text>',
    `<text x="64" y="112" font-size="20" fill="#aebccc">A ${base.environment.name} · B ${target.environment.name}</text>`,
    '<text x="64" y="150" font-size="18" fill="#aebccc">灰色为 A, 浅色为 B</text>',
    bars,
    "</svg>",
  ].join("");
}

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export function listenGameState(handler: (status: GameStatus) => void) {
  if (!isDesktop()) {
    previewGameListeners.add(handler);
    return () => {
      previewGameListeners.delete(handler);
    };
  }
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen<GameStatus>("game-state", (event) => handler(event.payload))
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    if (unlisten) unlisten();
  };
}

/// 外部启动切换地区或运行环境后, 后端推送新的 bootstrap 让界面同步.
export function listenSettingsChanged(handler: (data: Bootstrap) => void) {
  if (!isDesktop()) return () => {};
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen<Bootstrap>("settings-changed", (event) => handler(event.payload))
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    if (unlisten) unlisten();
  };
}

/// 启动器安全验证在后台完成或状态变化时推送.
export function listenIntegrityState(
  handler: (status: IntegrityStatus) => void,
) {
  if (!isDesktop()) return () => {};
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen<IntegrityStatus>("integrity-state", (event) => handler(event.payload))
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    if (unlisten) unlisten();
  };
}

/// 后端创建游玩会话后通知前端补读记录, 避免 running 先于会话创建。
export function listenPlaytimeStarted(handler: () => void) {
  if (!isDesktop()) return () => {};
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen("playtime-started", () => handler())
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    if (unlisten) unlisten();
  };
}

export function listenDownloadState(handler: (status: DownloadStatus) => void) {
  if (!isDesktop()) return () => {};
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen<DownloadStatus>("game-download-state", (event) =>
    handler(event.payload),
  )
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    if (unlisten) unlisten();
  };
}

export function listenMigrationState(
  handler: (status: MigrationStatus) => void,
) {
  if (!isDesktop()) return () => {};
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen<MigrationStatus>("migration-state", (event) => handler(event.payload))
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    if (unlisten) unlisten();
  };
}

const mockDownload: DownloadStatus = {
  mode: "check",
  phase: "idle",
  origin: "manual",
  autoWaiting: "",
  checkedAt: 0,
  distribution: "cn",
  installRoot: preview.installRoot,
  installed: true,
  full: false,
  unverifiedFiles: 0,
  currentFile: "",
  currentFileBytes: 0,
  completedFiles: 0,
  totalFiles: 0,
  downloadedBytes: 0,
  totalBytes: 0,
  patchFiles: 0,
  patchBytes: 0,
  fullFiles: 0,
  fullBytes: 0,
  stage: "",
  remoteVersion: "",
  optionalTags: [],
  verify: false,
  pendingFiles: 0,
  diskAvailableBytes: 256 * 1024 ** 3,
  diskRequiredBytes: 0,
  error: "",
};

const previewOptionalResources: OptionalResource[] = [
  {
    tag: "pakchunk101",
    name: "中文语音",
    version: "0.101.18",
    size: 1_793_007_011,
    state: "installed",
    selected: true,
    needsUpdate: false,
  },
  {
    tag: "pakchunk102",
    name: "日语语音",
    version: "0.102.18",
    size: 1_752_625_939,
    state: "installed",
    selected: true,
    needsUpdate: false,
  },
  {
    tag: "pakchunk103",
    name: "英语语音",
    version: "0.103.18",
    size: 1_836_921_904,
    state: "absent",
    selected: false,
    needsUpdate: false,
  },
  {
    tag: "pakchunk104",
    name: "韩语语音",
    version: "0.104.18",
    size: 2_008_643_453,
    state: "absent",
    selected: false,
    needsUpdate: false,
  },
];

let previewPreloadCompleted = false;
let previewPreloadCached = false;
let previewPreloadResourceBytes: Record<string, number> = {};

function previewCatalog(): ResourceCatalog {
  return clone({
    distribution: gameSettings(mock.settings).distribution ?? "cn",
    installRoot:
      demoInstallRoot(
        gameSettings(mock.settings),
        gameSettings(mock.settings).distribution,
      ) ||
      (gameSettings(mock.settings).distribution === "global"
        ? previewInstallRoot("global")
        : preview.installRoot),
    version: "1.4.16",
    preloadVersion: "1.5.0",
    preloadAvailable: true,
    preloadCompleted: previewPreloadCompleted,
    preloadCached: previewPreloadCached,
    preloadCachedVersion: previewPreloadCached ? "1.5.0" : "",
    preloadCachedBytes: previewPreloadCached
      ? 5 * 1024 ** 3 +
        Object.values(previewPreloadResourceBytes).reduce(
          (total, bytes) => total + bytes,
          0,
        )
      : 0,
    preloadResourceBytes: previewPreloadResourceBytes,
    resources: previewOptionalResources,
  });
}

export async function invoke<C extends keyof Commands>(
  command: C,
  ...params: CommandArgs[C] extends undefined
    ? [args?: undefined]
    : [args: CommandArgs[C]]
): Promise<Commands[C]> {
  if (isDesktop()) return nativeInvoke<C>(command, params[0]);
  return previewInvoke(command, params[0]) as Promise<Commands[C]>;
}

const idleMigration: MigrationStatus = {
  phase: "idle",
  distribution: null,
  mode: "",
  source: "",
  target: "",
  files: 0,
  totalFiles: 0,
  bytes: 0,
  totalBytes: 0,
  current: "",
  error: "",
  cleanupPending: false,
  cleanupSource: "",
  canCancel: false,
};
let previewMigration: MigrationStatus = clone(idleMigration);

let previewSetupPlan: RuntimeSetupPlan | null = null;
let previewSetupStatus: RuntimeSetupStatus = {
  busy: false,
  progress: {
    component: "umu",
    phase: "",
    detail: "",
    downloaded: 0,
    total: 0,
  },
  error: null,
  done: false,
};
let previewSetupProton: string | null = null;
let previewSetupCreate = false;
let previewSystemUmu = false;

async function previewInvoke(
  command: UiCommand,
  args?: CommandArgs[UiCommand],
): Promise<Commands[UiCommand]> {
  // 包声明自己负责的命令, 先按当前游戏命名空间查包预览; 其余命令走宿主默认演示.
  const handler = previewHandler(command, activeGame(mock.settings));
  if (handler) return handler(args);
  return hostPreviewInvoke(command as HostPreviewCommand, args);
}

async function hostPreviewInvoke(
  command: HostPreviewCommand,
  args?: CommandArgs[UiCommand],
): Promise<Commands[UiCommand]> {
  switch (command) {
    case "runtime_setup_info":
      return {
        needsSetup: false,
        environment:
          mock.settings.runtimeEnvironments.find(
            (entry) =>
              entry.id === gameSettings(mock.settings).activeEnvironment,
          ) ?? null,
        umu: mock.umuAvailable ? "/usr/bin/umu-run" : null,
        proton: previewSetupProton,
        protonDirectory: joinPath(preview.steamRoot, "compatibilitytools.d"),
        systemUmu: previewSystemUmu,
      };
    case "runtime_setup_plan": {
      const { allUsers, proton, createEnvironment } =
        args as CommandArgs["runtime_setup_plan"];
      previewSetupCreate = createEnvironment;
      previewSetupPlan = {
        umu: allUsers
          ? "/usr/bin/umu-run"
          : joinPath(preview.dataHome, "oddity/runtime/umu/umu-run"),
        proton: proton || preview.protonPath.replace(/\/proton$/, ""),
        installUmu: allUsers ? !previewSystemUmu : !mock.umuAvailable,
        installProton: !proton,
        systemCommand:
          allUsers && !previewSystemUmu
            ? [
                "/usr/bin/pacman",
                "-S",
                "--needed",
                "--noconfirm",
                "umu-launcher",
              ]
            : null,
        umuVersion: "1.4.4",
        log: joinPath(preview.dataHome, "oddity/logs/runtime-setup.log"),
      };
      return clone(previewSetupPlan);
    }
    case "runtime_setup_apply":
      if (!previewSetupPlan) throw new Error("请先查看安装计划");
      previewSetupProton = previewSetupPlan.proton;
      if (previewSetupPlan.systemCommand) previewSystemUmu = true;
      for (const component of ["umu", "proton", "runtime"] as const) {
        previewSetupStatus = {
          busy: true,
          done: false,
          error: null,
          progress: {
            component,
            phase: "prepare",
            detail: `正在准备 ${component}`,
            downloaded: 0,
            total: 0,
          },
        };
        await new Promise((resolve) => window.setTimeout(resolve, 1400));
      }
      mock.umuAvailable = true;
      previewSetupStatus = {
        busy: false,
        progress: {
          component: "runtime",
          phase: "runtime",
          detail: "运行时准备完成",
          downloaded: 0,
          total: 0,
        },
        error: null,
        done: true,
      };
      if (
        previewSetupCreate ||
        !mock.settings.runtimeEnvironments.some(
          (entry) =>
            entry.game === activeGame(mock.settings) &&
            entry.distribution === gameSettings(mock.settings).distribution,
        )
      ) {
        await previewInvoke("environment_create", {
          name: mock.settings.runtimeEnvironments.length
            ? `运行环境 ${mock.settings.runtimeEnvironments.length + 1}`
            : "默认运行环境",
          protonPath: previewSetupProton,
          sourceId: null,
          distribution: gameSettings(mock.settings).distribution ?? "cn",
        });
      }
      const created = mock.settings.runtimeEnvironments.at(-1);
      if (previewSetupCreate && created)
        await previewInvoke("environment_action", {
          action: "switch",
          id: created.id,
        });
      previewSetupPlan = null;
      return null;
    case "runtime_setup_status":
      return clone(previewSetupStatus);
    case "runtime_setup_dismiss":
      return null;
    case "runtime_setup_environment":
      if (!mock.prefixExists) throw new Error("请先配置启动环境");
      return clone(mock);
    case "game_accounts":
      return [];
    case "game_send_code":
    case "game_login":
      throw new Error("游戏账号登录需要在桌面启动器中操作.");
    case "game_account_remove":
      return null;
    case "launcher_background":
      return { video: null, image: null };
    case "bootstrap":
      return clone(mock);
    case "environment_create": {
      const { name, protonPath, sourceId, distribution } =
        args as CommandArgs["environment_create"];
      const source = mock.settings.runtimeEnvironments.find(
        (profile) => profile.id === sourceId,
      );
      if (sourceId && !source) throw new Error("源运行环境不存在");
      const target = mock.runtimeInstallations.find(
        (entry) => entry.distribution === distribution,
      );
      if (!target?.installed) throw new Error("尚未安装所选区服, 请先安装游戏");
      if (!name.trim() || !protonPath.trim())
        throw new Error("请填写环境名称和 Proton 目录");
      if (
        mock.settings.runtimeEnvironments.some(
          (profile) => profile.name === name.trim(),
        )
      )
        throw new Error("运行环境名称已存在");
      const profile = {
        id: String(mock.settings.runtimeEnvironments.length + 1).padStart(
          32,
          "0",
        ),
        name: name.trim(),
        proton: { path: protonPath.trim(), identity: "browser-preview" },
        installRoot: target.installRoot,
        game: activeGame(mock.settings),
        distribution,
        createdAt: Math.floor(Date.now() / 1000),
      };
      mock.settings.runtimeEnvironments.push(profile);
      if (!gameSettings(mock.settings).activeEnvironment) {
        updateGameSettings({ stableEnvironment: profile.id });
        selectPreviewEnvironment(profile);
      }
      return clone(mock);
    }
    case "environment_action": {
      const { action, id } = args as CommandArgs["environment_action"];
      const profile = mock.settings.runtimeEnvironments.find(
        (entry) => entry.id === id,
      );
      if (!profile) throw new Error("运行环境不存在");
      if (action === "switch") {
        const target = mock.runtimeInstallations.find(
          (entry) => entry.distribution === profile.distribution,
        );
        if (target?.installRoot !== profile.installRoot)
          throw new Error("运行环境与绑定的游戏目录不匹配");
        selectPreviewEnvironment(profile);
      } else if (action === "keep") {
        updateGameSettings({ stableEnvironment: id });
      } else {
        if (
          !id ||
          id === gameSettings(mock.settings).activeEnvironment ||
          id === gameSettings(mock.settings).stableEnvironment
        )
          throw new Error("不能删除当前环境或稳定环境");
        mock.settings.runtimeEnvironments =
          mock.settings.runtimeEnvironments.filter((entry) => entry.id !== id);
      }
      return clone(mock);
    }
    case "check_update":
      await new Promise((resolve) => setTimeout(resolve, 620));
      return clone(mockCheck);
    case "update_settings_section": {
      const { owner, patch } = args as CommandArgs["update_settings_section"];
      const fields = { ...patch };
      if (owner === "host") {
        const allowed = [
          "activeGame",
          "runtimeEnvironments",
          "gamepad",
          "language",
          "reduceMotion",
          "stackBulletins",
          "theme",
          "heroMedia",
          "pauseHeroOnBlur",
          "adaptiveTheme",
          "relativeHeatmap",
          "minimizeToTrayOnClose",
          "showLauncherAfterGameExit",
          "snapshotSuccessMinutes",
        ];
        for (const key of Object.keys(fields)) {
          if (!allowed.includes(key))
            throw new Error(`设置 owner ${owner} 不能修改字段: ${key}`);
        }
        Object.assign(mock.settings, fields);
        const view: Record<string, unknown> = { ...mock.settings };
        delete view.games;
        return clone({ owner, fields: view });
      }
      if (!owner.startsWith("game:")) {
        throw new Error(`无法识别的设置 owner: ${owner}`);
      }
      const gameId = owner.slice("game:".length) as GameId;
      const game = mock.settings.games[gameId];
      if (!game) throw new Error(`设置 owner 不存在: ${owner}`);
      const allowed = [
        "audio",
        "distribution",
        "installRoots",
        "preloadRoot",
        "launchMode",
        "gameAccountId",
        "gameMode",
        "updates",
        "activeEnvironment",
        "stableEnvironment",
      ];
      for (const key of Object.keys(fields)) {
        if (!allowed.includes(key))
          throw new Error(`设置 owner ${owner} 不能修改字段: ${key}`);
      }
      Object.assign(game, fields);
      const roots = gameSettings(mock.settings);
      mock.runtimeInstallations = [
        {
          distribution: "cn",
          installRoot: demoInstallRoot(roots, "cn") || preview.installRoot,
          installed: true,
        },
        {
          distribution: "global",
          installRoot:
            demoInstallRoot(roots, "global") || previewInstallRoot("global"),
          installed: Boolean(demoInstallRoot(roots, "global")),
        },
      ];
      applyPreviewDistribution();
      const view: Record<string, unknown> = { ...game };
      return clone({ owner, fields: view });
    }
    case "gamepad_poll":
      return {
        idle: { requested: false, error: null },
        devices: [],
        device: null,
        buttons: [],
        pressed: [],
        axes: [0, 0, 0, 0, 0, 0],
        focused: false,
        error: null,
      };
    case "gamepad_release_idle":
    case "gamepad_feedback":
      return null;
    case "browse_files":
      throw new Error("请在 Oddity 桌面应用中浏览文件.");
    case "gamepad_profile_validate":
      throw new Error("请在 Oddity 桌面应用中导入反馈配置.");
    case "gamepad_profile_load":
    case "gamepad_profile_save":
      throw new Error("请在 Oddity 桌面应用中导入或导出反馈配置.");
    case "game_resource_catalog":
      await new Promise((resolve) => setTimeout(resolve, 180));
      return previewCatalog();
    case "game_preload_delete":
      previewPreloadCompleted = false;
      previewPreloadCached = false;
      previewPreloadResourceBytes = {};
      Object.assign(mockDownload, {
        mode: "check",
        phase: "idle",
        verify: false,
        remoteVersion: "",
        optionalTags: [],
        currentFile: "",
        currentFileBytes: 0,
        completedFiles: 0,
        totalFiles: 0,
        downloadedBytes: 0,
        totalBytes: 0,
        patchFiles: 0,
        patchBytes: 0,
        fullFiles: 0,
        fullBytes: 0,
        pendingFiles: 0,
        diskRequiredBytes: 0,
        stage: "",
        error: "",
      });
      return previewCatalog();
    case "game_resource_remove": {
      const { tag, preload } = args as CommandArgs["game_resource_remove"];
      const resource = previewOptionalResources.find(
        (item) => item.tag === tag,
      );
      if (!resource) throw new Error(`未知的可选资源: ${tag}`);
      if (preload) {
        if (!previewPreloadResourceBytes[tag])
          throw new Error("该语种没有可删除的预下载缓存");
        delete previewPreloadResourceBytes[tag];
        previewPreloadCompleted = false;
      } else resource.state = "absent";
      resource.selected = preload && resource.state !== "absent";
      return previewCatalog();
    }
    case "game_resource_download": {
      const { tag } = args as CommandArgs["game_resource_download"];
      const resource = previewOptionalResources.find(
        (item) => item.tag === tag,
      );
      if (!resource) throw new Error(`未知的语音资源: ${tag}`);
      resource.selected = true;
      if (mockDownload.mode !== "resources") {
        Object.assign(mockDownload, {
          mode: "resources",
          resourceTasks: {},
          downloadedBytes: 0,
          totalBytes: 0,
        });
      }
      mockDownload.phase = "game";
      const tasks = mockDownload.resourceTasks!;
      if (["checking", "downloading"].includes(tasks[tag]?.phase || ""))
        return clone(mockDownload);
      const task: ResourceTask = {
        phase: "downloading",
        downloadedBytes: 0,
        totalBytes: resource.size,
        error: "",
      };
      tasks[tag] = task;
      mockDownload.totalBytes += resource.size;
      window.setTimeout(() => {
        if (task.phase !== "downloading") return;
        task.downloadedBytes = resource.size;
        task.phase = "ready";
        resource.state = "installed";
        resource.needsUpdate = false;
        mockDownload.downloadedBytes += resource.size;
        if (Object.values(tasks).every((task) => task.phase === "ready"))
          mockDownload.phase = "ready";
      }, 2200);
      return clone(mockDownload);
    }
    case "migration_plan": {
      const { distribution, target } = args as CommandArgs["migration_plan"];
      const game = gameSettings(mock.settings);
      const source = demoInstallRoot(game, distribution) || mock.installRoot;
      return {
        distribution,
        mode: "copy",
        source,
        target,
        sameFilesystem: false,
        files: 128,
        bytes: 8 * 1024 ** 3,
        requiredBytes: 8 * 1024 ** 3,
        availableBytes: 128 * 1024 ** 3,
        environments: mock.settings.runtimeEnvironments
          .filter(
            (profile) =>
              profile.game === activeGame(mock.settings) &&
              profile.distribution === distribution,
          )
          .map((profile) => ({
            id: profile.id,
            name: profile.name,
            prefix: `preview/${profile.id}/prefix`,
          })),
        preloadRoot: null,
        warnings: [],
      };
    }
    case "migration_start": {
      const { distribution, target } = args as CommandArgs["migration_start"];
      const game = gameSettings(mock.settings);
      previewMigration = {
        ...idleMigration,
        phase: "done",
        distribution,
        mode: "copy",
        source: demoInstallRoot(game, distribution) || mock.installRoot,
        target,
      };
      return clone(previewMigration);
    }
    case "migration_status":
      return clone(previewMigration);
    case "migration_cancel":
      previewMigration.phase = "cancelled";
      return clone(previewMigration);
    case "migration_cleanup":
      previewMigration.cleanupPending = false;
      previewMigration.cleanupSource = "";
      return clone(previewMigration);
    case "migration_abandon":
      previewMigration = clone(idleMigration);
      return clone(previewMigration);
    case "migration_dismiss":
      previewMigration = clone(idleMigration);
      return clone(previewMigration);
    case "migration_retry":
      return clone(previewMigration);
    case "game_download_status":
      return clone(mockDownload);
    case "game_download_start": {
      const { mode, full } = args as CommandArgs["game_download_start"];
      // 预览资源大小一致, 完整校验才能发现模拟的内容损坏。
      if (mode === "check" && !full) {
        Object.assign(mockDownload, {
          mode,
          phase: "ready",
          installed: true,
          full: false,
          unverifiedFiles: 0,
          currentFile: "",
          currentFileBytes: 0,
          completedFiles: 0,
          totalFiles: 0,
          downloadedBytes: 0,
          totalBytes: 0,
          patchFiles: 0,
          patchBytes: 0,
          fullFiles: 0,
          fullBytes: 0,
          stage: "",
          remoteVersion: "",
          optionalTags: [],
          pendingFiles: 0,
          diskRequiredBytes: 0,
          error: "",
          distribution: gameSettings(mock.settings).distribution,
          installRoot: mock.installRoot,
        });
        return clone(mockDownload);
      }
      Object.assign(mockDownload, {
        mode,
        verify: mode === "preload" && previewPreloadCompleted,
        full: full || (mockDownload.full && mockDownload.phase !== "ready"),
        unverifiedFiles: 0,
        phase: mode === "check" ? "needs-update" : "game",
        distribution: gameSettings(mock.settings).distribution,
        installRoot: mock.installRoot,
        currentFile:
          mode === "check" ? "" : "Client/WindowsNoEditor/example.pak",
        currentFileBytes: mode === "check" ? 0 : 6_000_000,
        optionalTags: previewOptionalResources
          .filter((resource) => resource.selected)
          .map((resource) => resource.tag),
        completedFiles: 0,
        totalFiles: 2,
        downloadedBytes: 0,
        totalBytes: 12_000_000,
        patchFiles: 1,
        patchBytes: 10_000_000,
        fullFiles: 1,
        fullBytes: 2_000_000,
        stage: mode === "check" ? "" : "download",
        remoteVersion: mode === "preload" ? "1.5.0" : "1.4.17",
        pendingFiles: 2,
        diskRequiredBytes: 18_000_000,
        error: "",
      });
      if (mode !== "check") {
        window.setTimeout(() => {
          if (mode === "preload") {
            previewPreloadCompleted = true;
            previewPreloadCached = true;
            previewPreloadResourceBytes = Object.fromEntries(
              previewOptionalResources
                .filter((resource) => resource.selected)
                .map((resource) => [resource.tag, resource.size]),
            );
          }
          if (mode === "apply")
            previewOptionalResources.forEach((resource) => {
              if (resource.selected) resource.state = "installed";
            });
          Object.assign(mockDownload, {
            phase: mode === "preload" ? "preloaded" : "ready",
            completedFiles: 2,
            downloadedBytes: 12_000_000,
            pendingFiles: 0,
            currentFile: "",
            currentFileBytes: 0,
            stage: "",
          });
        }, 1400);
      }
      return clone(mockDownload);
    }
    case "game_download_cancel":
      Object.values(mockDownload.resourceTasks || {}).forEach((task) => {
        if (["checking", "downloading"].includes(task.phase))
          task.phase = "cancelled";
      });
      mockDownload.phase = "cancelled";
      return clone(mockDownload);
    case "environment_snapshot_overview":
      return clone(previewSnapshotOverview());
    case "environment_snapshot_compare": {
      const { target } = args as CommandArgs["environment_snapshot_compare"];
      return clone(previewSnapshotComparison(target));
    }
    case "environment_snapshot_save_reference": {
      const { label } =
        args as CommandArgs["environment_snapshot_save_reference"];
      return clone(previewSnapshotSaveReference(label));
    }
    case "environment_snapshot_delete_reference":
      previewSnapshotReference = null;
      return null;
    case "environment_snapshot_clear_history":
      previewSnapshotBaseline = null;
      previewSnapshotLatest = null;
      previewSnapshotAttempts = 0;
      return null;
    case "open_url":
      return null;
    case "integrity_acknowledge":
      return null;
    case "prepare_log_export":
    case "save_log_export":
    case "discard_log_export":
    case "open_directory":
      throw new Error("请在 Oddity 桌面应用中导出日志");
    case "launch_official":
      if (!gameSettings(mock.settings).activeEnvironment)
        throw new Error("请先创建 Proton 运行环境");
      startPreviewLaunch();
      return {
        mode: "background",
        pid: previewGame.pid,
        exe: previewGame.exe,
        prefix: mock.prefix,
        log: "",
        startedAt: previewGame.startedAt,
      };
    case "cancel_launch":
      clearPreviewLaunchTimers();
      if (previewGame.launch) {
        previewGame.launch = {
          ...previewGame.launch,
          cancelling: true,
        };
        emitPreviewGameStatus();
      }
      await delay(450);
      previewGame.running = false;
      previewGame.closing = false;
      previewGame.pid = 0;
      previewGame.launch = null;
      emitPreviewGameStatus();
      return previewGameStatus();
    case "game_mode_tuning_info":
      return { pinCores: null, parkCores: null, renice: null, ioprio: null };
    case "game_mode_tuning_apply":
      throw new Error("请在原生启动器中应用 GameMode 配置");
    case "game_mode_gpu_info":
      return {
        config: {
          enabled: false,
          device: 0,
          vendor: "nvidia",
          coreOffset: -1,
          memoryOffset: -1,
          powerMode: -1,
          amdLevel: "auto",
        },
        devices: [],
      };
    case "game_mode_gpu_apply":
      throw new Error("请在原生启动器中应用 GPU 配置");
    case "audio_status":
      return {
        devices: [{ index: 1, name: "demo-output", label: "系统输出设备" }],
        defaultDevice: "demo-output",
        streams: [],
        error: null,
      };
    case "game_mode_status":
      return {
        capabilities: (
          [
            "performance",
            "inhibit",
            "notifications",
            "overlay",
            "gamescope",
          ] as const
        ).map((id) => ({
          id,
          available: false,
          state: "unavailable" as const,
          detail: "浏览器预览无法检测系统能力, 请在原生启动器中检测",
        })),
        session: null,
      };
    case "game_status":
      return previewGameStatus();
    case "take_launch_error":
      return null;
    case "performance_overview":
      return clone(previewPerformance);
    case "performance_recording": {
      const { enabled } = args as CommandArgs["performance_recording"];
      previewPerformance.recordingEnabled = enabled;
      return null;
    }
    case "performance_pin": {
      const { id, pinned } = args as CommandArgs["performance_pin"];
      const record = previewPerformance.records.find((item) => item.id === id);
      if (record) record.pinned = pinned;
      return null;
    }
    case "performance_delete": {
      const { id } = args as CommandArgs["performance_delete"];
      previewPerformance.records = previewPerformance.records.filter(
        (item) => item.id !== id,
      );
      return null;
    }
    case "performance_report": {
      const { a, b } = args as CommandArgs["performance_report"];
      return previewPerformanceReport(a, b);
    }
    case "performance_export":
      throw new Error("请在原生启动器中导出性能对比图");
    case "playtime_history":
      return previewHistory();
    case "playtime_export_poster": {
      const { destination } = args as CommandArgs["playtime_export_poster"];
      return destination;
    }
    case "close_game":
      clearPreviewLaunchTimers();
      previewGame.closing = true;
      previewGame.launch = null;
      emitPreviewGameStatus();
      await delay(850);
      previewGame.running = false;
      previewGame.closing = false;
      previewGame.pid = 0;
      previewPlaytime.push({
        startedAt: Date.now() - 90 * 60_000,
        endedAt: Date.now(),
        mode: "background",
        distribution: "cn",
      });
      emitPreviewGameStatus();
      return previewGameStatus();
    case "win_minimize":
    case "win_toggle_maximize":
    case "win_close":
    case "win_hide":
      return null;
    default: {
      // 新命令既没进包预览表也没补宿主分支时这里会编译失败.
      const unhandled: never = command;
      throw new Error("浏览器演示未实现该命令: " + String(unhandled));
    }
  }
}
