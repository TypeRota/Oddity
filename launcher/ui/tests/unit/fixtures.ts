import { defaultGamepadOptions } from "../../src/types/gamepad";
import { defaultGamescope } from "../../src/core/gameMode";
import type {
  Bootstrap,
  CheckResult,
  GameSettings,
  GameStatus,
  Settings,
} from "../../src/types/launcher";

export const defaultGame = (
  overrides: Partial<GameSettings> = {},
): GameSettings => ({
  distribution: "cn",
  installRoot: null,
  globalInstallRoot: null,
  preloadRoot: null,
  launchMode: "background",
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
  ...overrides,
});

/** 返回当前游戏字段被覆盖的设置副本. */
export function withGame(
  overrides: Partial<GameSettings>,
  base: Settings = settings,
): Settings {
  const game = base.activeGame;
  return {
    ...base,
    games: {
      ...base.games,
      [game]: { ...defaultGame(base.games[game]), ...overrides },
    },
  };
}

export const settings: Settings = {
  activeGame: "nte",
  games: { nte: defaultGame() },
  runtimeEnvironments: [],
  gamepad: defaultGamepadOptions(),
  language: "zh-CN",
  reduceMotion: true,
  stackBulletins: false,
  theme: "light",
  heroMedia: "photo",
  adaptiveTheme: true,
  minimizeToTrayOnClose: true,
  showLauncherAfterGameExit: true,
  snapshotSuccessMinutes: 1,
};
export const bootstrap = (overrides: Partial<Bootstrap> = {}): Bootstrap => ({
  ok: true,
  error: null,
  installRoot: "/game",
  launcherRoot: "/game/launcher",
  prefix: "/prefix",
  prefixExists: true,
  protonName: "GE-Proton",
  protonPath: "/proton",
  umuAvailable: true,
  launcherVersion: "1.0",
  launcherBuild: "build",
  gameVersion: "1.0",
  gameExe: "/game/game.exe",
  gameExists: true,
  sdkPath: "/game/sdk.dll",
  checks: [{ label: "Proton", ok: true, detail: "/proton" }],
  runtimeInstallations: [
    { distribution: "cn", installRoot: "/game", installed: true },
    { distribution: "global", installRoot: "/global-game", installed: false },
  ],
  settings: structuredClone(settings),
  integrity: {
    state: "verified",
    source: "online",
    checkedAt: 1,
    changed: [],
    changedTotal: 0,
    notice: null,
    appVersion: "0.1.0",
    releaseUrl: "https://github.com/LaT-SKY/Oddity/releases/tag/oddity-v0.1.0",
  },
  appVersion: "0.1.0",
  os: "linux",
  ...overrides,
});
export const check = (overrides: Partial<CheckResult> = {}): CheckResult => ({
  installed: true,
  distribution: "cn",
  installRoot: "/game",
  remoteVersion: "1.0",
  localVersion: "1.0",
  productVersion: "1.0",
  files: 3,
  ok: 3,
  missing: 0,
  changed: 0,
  bytes: 0,
  durationMs: 10,
  missingPreview: [],
  changedPreview: [],
  ...overrides,
});
export const session = (overrides: Partial<GameStatus> = {}): GameStatus => ({
  running: false,
  closing: false,
  pid: 0,
  startedAt: 0,
  exe: "/game/game.exe",
  log: "",
  ...overrides,
});
