import type { Component } from "vue";
import { contributionVisible, type CapabilityRequirement } from "./capability";
import type { HostFacts } from "./facts";
import { homeCardContributions, sortByOrderAndId } from "./registry";

/** 宿主挂载一张首页卡片所需的最小形状. */
export interface HomeCardContribution {
  readonly id: string;
  /** 卡片顺序; 同序按 id 排序, 与包目录扫描顺序无关. */
  readonly order: number;
  readonly component: Component;
  /** 卡片所需能力; required 不满足时卡片不挂载. */
  readonly requires?: readonly CapabilityRequirement[];
  /** 卡片可见条件; 条件不成立的卡片不挂载. */
  readonly visible: (facts: HostFacts) => boolean;
}

export { homeCardContributions };

/** 当前事实下要挂载的卡片. */
export function selectHomeCards(
  facts: HostFacts,
  contributions: readonly HomeCardContribution[] = homeCardContributions,
): HomeCardContribution[] {
  return sortByOrderAndId(contributions).filter((card) =>
    contributionVisible(card, facts),
  );
}
