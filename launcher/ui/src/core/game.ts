import type {
  ChannelView,
  GameId,
  GameSettings,
  Settings,
} from "../types/launcher";
import { defaultGamescope } from "./gameMode";

/** 新游戏与启动方式失效时使用的默认启动方式. */
export const DEFAULT_LAUNCH_MODE = "background";

const DEFAULT_GAME_SETTINGS: GameSettings = {
  audio: { device: null, volume: 100 },
  distribution: null,
  installRoots: {},
  preloadRoot: null,
  launchMode: DEFAULT_LAUNCH_MODE,
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
  activeEnvironments: {},
  stableEnvironments: {},
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

/**
 * 按区服记住运行环境时使用的键.
 *
 * 当前区服取标记为 active 的一条, 其次回落到默认区服; 游戏没有声明渠道时为空串.
 * 与宿主 channel::effective_id 的取值一致.
 */
export function channelKey(channels: readonly ChannelView[]): string {
  return (
    channels.find((channel) => channel.active)?.id ??
    channels.find((channel) => channel.default)?.id ??
    ""
  );
}

/** 当前区服记住的当前运行环境 id; 没有选择时为 null. */
export function activeEnvironmentId(
  settings: Settings | null | undefined,
  channels: readonly ChannelView[],
): string | null {
  return (
    gameSettings(settings).activeEnvironments?.[channelKey(channels)] ?? null
  );
}

/** 当前区服记住的稳定运行环境 id; 没有标记时为 null. */
export function stableEnvironmentId(
  settings: Settings | null | undefined,
  channels: readonly ChannelView[],
): string | null {
  return (
    gameSettings(settings).stableEnvironments?.[channelKey(channels)] ?? null
  );
}

/** 该环境是否被任一区服记为当前或稳定环境. */
export function environmentInUse(
  settings: Settings | null | undefined,
  id: string,
): boolean {
  const game = gameSettings(settings);
  return [game.activeEnvironments, game.stableEnvironments].some((slots) =>
    Object.values(slots ?? {}).includes(id),
  );
}
