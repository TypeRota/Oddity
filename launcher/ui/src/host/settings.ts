import type { Component } from "vue";
import { settingsCategories, settingsEntries } from "../core/settingsCatalog";
import { contributionVisible, type CapabilityRequirement } from "./capability";
import type { HostFacts } from "./facts";
import { settingsSectionContributions, sortByOrderAndId } from "./registry";

/** 设置项的检索元数据; 随声明它的分区一起注册, 供设置页搜索与定位. */
export interface SettingsEntryDeclaration {
  readonly id: string;
  readonly label: string;
  readonly description: string;
  readonly keywords: string;
}

/** 设置分区贡献: 分区内容、顺序与可见条件由声明方提供, 宿主只负责外壳与导航. */
export interface SettingsSectionContribution {
  readonly id: string;
  readonly label: string;
  readonly icon: string;
  readonly order: number;
  readonly component: Component;
  /** 分区内设置项的检索索引, 与分区同源. */
  readonly entries: readonly SettingsEntryDeclaration[];
  /** 分区所需能力; required 不满足时分区不挂载. */
  readonly requires?: readonly CapabilityRequirement[];
  /** 可见条件; 条件不成立的分区不进入导航、搜索与渲染. */
  readonly visible: (facts: HostFacts) => boolean;
}

export { settingsSectionContributions };

/** 分区 id 由声明方给出, 宿主不枚举具体游戏分区. */
export type SettingsSectionId = string;

/** 设置页实际渲染的分区: 宿主通用分区与当前可见的贡献分区合并后的形状. */
export interface SettingsSection {
  readonly id: SettingsSectionId;
  readonly label: string;
  readonly icon: string;
  readonly order: number;
}

export interface SettingsSearchEntry {
  readonly id: string;
  readonly category: SettingsSectionId;
  readonly label: string;
  readonly description: string;
  readonly keywords: string;
}

/** 按 order 取出当前事实下可见的分区贡献. */
export function selectSettingsSections(
  facts: HostFacts,
  contributions: readonly SettingsSectionContribution[] = settingsSectionContributions,
): SettingsSectionContribution[] {
  return sortByOrderAndId(contributions).filter((section) =>
    contributionVisible(section, facts),
  );
}

/** 宿主通用分区加当前可见的贡献分区, 按 order 排序. */
export function settingsSections(
  facts: HostFacts,
  contributions: readonly SettingsSectionContribution[] = settingsSectionContributions,
): SettingsSection[] {
  const contributed: SettingsSection[] = selectSettingsSections(
    facts,
    contributions,
  ).map(({ id, label, icon, order }) => ({ id, label, icon, order }));
  return sortByOrderAndId([...settingsCategories, ...contributed]);
}

/** 搜索索引: 宿主分区条目与可见贡献分区条目合并, 供设置页过滤与定位. */
export function settingsSearchEntries(
  facts: HostFacts,
  contributions: readonly SettingsSectionContribution[] = settingsSectionContributions,
): SettingsSearchEntry[] {
  return [
    ...settingsEntries,
    ...selectSettingsSections(facts, contributions).flatMap((section) =>
      section.entries.map((entry) => ({ ...entry, category: section.id })),
    ),
  ];
}
