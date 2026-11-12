import type { GameId, GameSettings, Settings } from "../types/launcher";
import { defaultGamescope } from "./gameMode";

const DEFAULT_GAME_SETTINGS: GameSettings = {
  audio: { device: null, volume: 100 },
  distribution: null,
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
};

/** 后端当前展示的游戏; 旧数据缺字段时沿用默认游戏 id. */
export function activeGame(settings: Settings | null | undefined): GameId {
  return settings?.activeGame ?? "nte";
}

/** 当前游戏的地区与安装目录. */
export function gameSettings(
  settings: Settings | null | undefined,
): GameSettings {
  if (!settings) return DEFAULT_GAME_SETTINGS;
  return settings.games?.[activeGame(settings)] ?? DEFAULT_GAME_SETTINGS;
}
