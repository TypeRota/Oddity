import { activeGame } from "../core/game";
import type { Bootstrap, ChannelView, Settings } from "../types/launcher";
import type { CapabilityFacts } from "./capability";

/**
 * 贡献求值时可见的宿主事实.
 *
 * 只放宿主已经掌握、与具体游戏无关的事实; 贡献用类型化谓词读它,
 * 不直接接触宿主状态, 也不引入条件表达式或规则引擎.
 */
export interface HostFacts extends CapabilityFacts {
  /**
   * 当前激活游戏的 id, 与设置的 activeGame 同源.
   *
   * 宿主不枚举具体游戏: 取值由游戏包自己判断, 没有对应设置项的游戏包
   * 也能声明可见条件.
   */
  readonly activeGame: string;
  /** 当前有效渠道; 游戏没有声明渠道或尚未选择时为 null. */
  readonly distribution: string | null;
  /** 当前游戏声明的渠道. */
  readonly channels: readonly ChannelView[];
}

/** 渠道展示名; 已不存在的渠道回落到原始 id. */
export function channelLabel(
  facts: Pick<HostFacts, "channels">,
  id: string | null | undefined,
): string {
  if (!id) return "";
  return facts.channels.find((channel) => channel.id === id)?.label ?? id;
}

/**
 * 从 bootstrap 派生当前事实.
 *
 * 渠道以 bootstrap 里标记为 active 的一条为准; 设置尚未到达时沿用默认值,
 * 未传入能力事实时按空表处理.
 */
export function factsFrom(bootstrap: Bootstrap | null | undefined): HostFacts {
  const settings: Settings | null | undefined = bootstrap?.settings;
  const channels = bootstrap?.channels ?? [];
  const active = channels.find((channel) => channel.active)?.id;
  const fallback = channels.find((channel) => channel.default)?.id;
  return {
    activeGame: activeGame(settings),
    distribution: active ?? fallback ?? null,
    channels,
    capabilities: bootstrap?.capabilities ?? [],
  };
}

/** 可见条件: 只有当前激活的是该游戏时才成立. */
export function whenGame(game: string): (facts: HostFacts) => boolean {
  return (facts) => facts.activeGame === game;
}
