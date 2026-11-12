import type { Component } from "vue";
// 页面登记表只取游戏包的元数据, 不把包内组件拉进控制器的依赖图.
import { packageGames, packagePages } from "./generated/page-entries";
import { contributionVisible, type CapabilityRequirement } from "./capability";
import type { HostFacts } from "./facts";

/** 页面挂载的宿主区域: home 常驻, drawer 随导航抽屉切换. */
export type PageSlot = "home" | "drawer";

export interface PageNavItem {
  readonly label: string;
  readonly icon: string;
}

/** 页面声明: 只描述位置与可见性, 不引入组件, 供导航、白名单与默认页使用. */
export interface PageEntry {
  readonly id: string;
  readonly slot: PageSlot;
  readonly order: number;
  /** false 表示页面只由流程进入, 不进入主导航. */
  readonly nav: PageNavItem | false;
  /** 页面所需能力; required 不满足时页面不挂载. */
  readonly requires?: readonly CapabilityRequirement[];
  /** 页面可见条件; 条件不成立的页面不挂载, 也不进入导航与页面白名单. */
  readonly visible: (facts: HostFacts) => boolean;
}

/** 页面贡献: 声明加组件绑定; 游戏包导出的页面也是这个形状. */
export interface PageContribution extends PageEntry {
  readonly component: Component;
}

/**
 * 游戏展示声明: 首页标题、安装目录提示与游玩海报文件名等宿主通用位置使用的
 * 展示信息由游戏包提供.
 *
 * 宿主不按发行渠道拼游戏名, 也不枚举具体游戏; 没有声明的游戏回退到
 * activeGame 本身或通用文案.
 */
export interface GameContribution {
  readonly id: string;
  readonly displayName: (facts: HostFacts) => string;
  /** 安装目录输入框的默认路径提示; 空串表示不提示. */
  readonly installRootHint: (facts: HostFacts) => string;
  /** 游玩记录海报的文件名前缀, 宿主追加日期. */
  readonly playtimePosterPrefix: (facts: HostFacts) => string;
}

// 宿主页面; 由游戏包提供内容的页面在包内声明.
export const hostPageEntries = [
  {
    id: "home",
    slot: "home",
    order: 1,
    nav: { label: "启动", icon: "play" },
    visible: () => true,
  },
  {
    id: "game-mode",
    slot: "drawer",
    order: 3,
    nav: { label: "游戏模式", icon: "game-controller" },
    visible: () => true,
  },
  {
    id: "records",
    slot: "drawer",
    order: 5,
    nav: { label: "记录", icon: "clock" },
    visible: () => true,
  },
  {
    id: "settings",
    slot: "drawer",
    order: 6,
    nav: { label: "设置", icon: "gear-six" },
    visible: () => true,
  },
  {
    // 安装向导由启动流程进入, 当前不出现在主导航.
    id: "runtime-setup",
    slot: "drawer",
    order: 7,
    nav: false,
    visible: () => true,
  },
] as const satisfies readonly PageEntry[];

type PackagePage = (typeof packagePages)[number]["pages"][number];

const packagePageEntries: readonly PackagePage[] = packagePages
  .flatMap((entry) => entry.pages as readonly PackagePage[])
  .sort(compareByOrderAndId);

/** 页面登记表: 宿主页面加游戏包声明的页面, 导航、白名单与默认页都由它派生. */
export const pageEntries = [
  ...hostPageEntries,
  ...packagePageEntries,
] as const satisfies readonly PageEntry[];

export type PageId = (typeof pageEntries)[number]["id"];

export const DEFAULT_PAGE_ID = "home" satisfies PageId;

/** 页面与贡献共用的排序: order 升序, 同序按 id 的字符序, 与目录扫描顺序无关. */
export function compareByOrderAndId(
  a: { readonly id: string; readonly order: number },
  b: { readonly id: string; readonly order: number },
): number {
  return a.order - b.order || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** 游戏展示声明表: 与页面登记表一样由包声明, 宿主只负责合并. */
export const gameContributions: readonly GameContribution[] = packageGames.map(
  (entry) => entry.game,
);

/** 当前游戏的展示声明; 宿主只按 id 查表, 不枚举具体游戏. */
export function gameContribution(
  facts: HostFacts,
): GameContribution | undefined {
  return gameContributions.find((item) => item.id === facts.activeGame);
}

/** 当前游戏的展示名; 没有声明的游戏回退到 activeGame, 宿主不猜文案. */
export function gameDisplayName(facts: HostFacts): string {
  return gameContribution(facts)?.displayName(facts) ?? facts.activeGame;
}

/** 有展示声明的游戏名; 法律声明等指名位置用它, 没有声明时返回 null. */
export function declaredGameName(facts: HostFacts): string | null {
  return gameContribution(facts)?.displayName(facts) ?? null;
}

/** 安装目录输入框的默认路径; 没有声明时不提示. */
export function gameInstallRootHint(facts: HostFacts): string {
  return gameContribution(facts)?.installRootHint(facts) ?? "";
}

/** 游玩记录海报的文件名前缀; 没有声明时用通用名. */
export function playtimePosterPrefix(facts: HostFacts): string {
  return gameContribution(facts)?.playtimePosterPrefix(facts) ?? "游玩记录";
}

export interface NavigationItem<
  Id extends string = PageId,
> extends PageNavItem {
  readonly id: Id;
}

/** 从给定条目派生导航项; 可见条件不成立的页面不进入导航. */
export function selectNavigationItems<Id extends string>(
  entries: readonly (PageEntry & { readonly id: Id })[],
  facts: HostFacts,
): NavigationItem<Id>[] {
  return [...entries]
    .sort(compareByOrderAndId)
    .filter((page) => contributionVisible(page, facts))
    .flatMap((page) =>
      page.nav
        ? [{ id: page.id, label: page.nav.label, icon: page.nav.icon }]
        : [],
    );
}

/** 导航项与页面表同源: 未进导航或当前不可见的页面不会出现在这里. */
export function navigationItems(facts: HostFacts): NavigationItem[] {
  return selectNavigationItems(pageEntries, facts);
}

/** 页面白名单: 表里的 id 才允许成为当前页, 是否可见另行判断. */
export function isPageId(value: string): value is PageId {
  return pageEntries.some((page) => page.id === value);
}

/** 页面在当前事实下是否可见; 不在白名单里的 id 一律不可见. */
export function isPageVisible(id: string, facts: HostFacts): boolean {
  return pageEntries.some(
    (page) => page.id === id && contributionVisible(page, facts),
  );
}

/** 第一个可见页面; 没有可用页面时退回默认页, 避免停在空面板. */
export function firstVisiblePage(facts: HostFacts): PageId {
  const first = [...pageEntries]
    .sort(compareByOrderAndId)
    .find((page) => contributionVisible(page, facts));
  return first ? first.id : DEFAULT_PAGE_ID;
}

/** 解析一次页面请求: 白名单之外返回 null, 当前不可见的页面回退到第一个可见页. */
export function resolvePage(id: string, facts: HostFacts): PageId | null {
  if (!isPageId(id)) return null;
  return isPageVisible(id, facts) ? id : firstVisiblePage(facts);
}
