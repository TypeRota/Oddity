import type { Component } from "vue";
import HomePage from "../pages/HomePage.vue";
import GameModePage from "../pages/GameModePage.vue";
import PlaytimePage from "../pages/PlaytimePage.vue";
import SettingsPage from "../pages/SettingsPage.vue";
import RuntimeSetupPage from "../pages/RuntimeSetupPage.vue";
import { contributionVisible } from "./capability";
import type { HostFacts } from "./facts";
import { hostPageEntries, type PageContribution, type PageSlot } from "./pages";
import { packagePageContributions, sortByOrderAndId } from "./registry";

export {
  DEFAULT_PAGE_ID,
  gameDisplayName,
  isPageId,
  navigationItems,
  type GameContribution,
  type NavigationItem,
  type PageContribution,
  type PageEntry,
  type PageId,
  type PageNavItem,
  type PageSlot,
} from "./pages";

/** 宿主渲染一个贡献所需的最小形状. */
export interface SlotContribution {
  readonly id: string;
  readonly component: Component;
}

// 宿主页面只在这里绑定组件; 游戏包的页面在包内绑定, 宿主不维护包页面 id 到组件的映射.
type HostPageId = (typeof hostPageEntries)[number]["id"];
const hostPageComponents: Record<HostPageId, Component> = {
  home: HomePage,
  "game-mode": GameModePage,
  records: PlaytimePage,
  settings: SettingsPage,
  "runtime-setup": RuntimeSetupPage,
};

// 页面与导航的唯一来源; 组件同步导入, 不做运行时加载.
export const pageContributions: readonly PageContribution[] = sortByOrderAndId([
  ...hostPageEntries.map((entry) => ({
    ...entry,
    component: hostPageComponents[entry.id],
  })),
  ...packagePageContributions,
]);

/** 按 order 取出某个区域的可见页面, 供 HostSlot 渲染. */
export function selectPageContributions(
  contributions: readonly PageContribution[],
  slot: PageSlot,
  facts: HostFacts,
): PageContribution[] {
  return sortByOrderAndId(contributions).filter(
    (page) => page.slot === slot && contributionVisible(page, facts),
  );
}

export function pagesForSlot(
  slot: PageSlot,
  facts: HostFacts,
): readonly PageContribution[] {
  return selectPageContributions(pageContributions, slot, facts);
}
