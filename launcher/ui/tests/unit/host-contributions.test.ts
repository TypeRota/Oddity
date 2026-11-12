import { describe, expect, it } from "vitest";
import {
  pageContributions,
  pagesForSlot,
  selectPageContributions,
  type PageContribution,
} from "../../src/host/contributions";
import { factsFrom, type HostFacts } from "../../src/host/facts";
import {
  DEFAULT_PAGE_ID,
  declaredGameName,
  firstVisiblePage,
  gameDisplayName,
  gameInstallRootHint,
  isPageId,
  isPageVisible,
  navigationItems,
  playtimePosterPrefix,
  resolvePage,
  selectNavigationItems,
} from "../../src/host/pages";
import { bootstrap, settings } from "./fixtures";
import type { Settings } from "../../src/types/launcher";

const nte = factsFrom(bootstrap());
const fixture: HostFacts = {
  activeGame: "fixture",
  distribution: "cn",
  capabilities: [],
  channels: [],
};
const hsr = factsFrom(
  bootstrap({
    settings: { ...settings, activeGame: "hsr" } as Settings,
    channels: [],
  }),
);

describe("宿主页面贡献表", () => {
  it("页面按区域与顺序派生, 包页面带命名空间前缀", () => {
    expect(pageContributions.map((page) => page.id)).toEqual([
      "home",
      "nte-update",
      "game-mode",
      "nte-bulletin",
      "records",
      "settings",
      "runtime-setup",
      "fixture-fixture",
    ]);
    expect(pagesForSlot("home", nte).map((page) => page.id)).toEqual(["home"]);
    expect(pagesForSlot("drawer", nte).map((page) => page.id)).toEqual([
      "nte-update",
      "game-mode",
      "nte-bulletin",
      "records",
      "settings",
      "runtime-setup",
    ]);
    expect(DEFAULT_PAGE_ID).toBe("home");
  });

  it("每条贡献都绑定到组件", () => {
    expect(pageContributions.every((page) => page.component)).toBe(true);
  });

  it("不可见的页面不进入导航", () => {
    const component = pageContributions[0].component;
    const contributions: PageContribution[] = [
      {
        id: "hidden",
        slot: "drawer",
        component,
        order: 1,
        nav: { label: "隐藏", icon: "x" },
        visible: () => false,
      },
      {
        id: "shown",
        slot: "drawer",
        component,
        order: 2,
        nav: { label: "显示", icon: "y" },
        visible: () => true,
      },
    ];
    expect(
      selectNavigationItems(contributions, nte).map((item) => item.id),
    ).toEqual(["shown"]);
  });

  it("导航项与页面表同源, 安装向导不进入导航", () => {
    expect(navigationItems(nte)).toEqual([
      { id: "home", label: "启动", icon: "play" },
      { id: "nte-update", label: "更新", icon: "download-simple" },
      { id: "game-mode", label: "游戏模式", icon: "game-controller" },
      { id: "nte-bulletin", label: "公告", icon: "newspaper-clipping" },
      { id: "records", label: "记录", icon: "clock" },
      { id: "settings", label: "设置", icon: "gear-six" },
    ]);
  });

  it("页面白名单只接受表里的 id", () => {
    expect(isPageId("home")).toBe(true);
    expect(isPageId("runtime-setup")).toBe(true);
    expect(isPageId("fixture-fixture")).toBe(true);
    expect(isPageId("unknown")).toBe(false);
    expect(isPageId("")).toBe(false);
  });

  it("区域与可见条件在纯函数里过滤", () => {
    const component = pageContributions[0].component;
    const contributions: PageContribution[] = [
      {
        id: "hidden",
        slot: "drawer",
        component,
        order: 10,
        nav: false,
        visible: () => false,
      },
      {
        id: "visible",
        slot: "drawer",
        component,
        order: 20,
        nav: false,
        visible: () => true,
      },
      {
        id: "home-only",
        slot: "home",
        component,
        order: 30,
        nav: false,
        visible: () => true,
      },
    ];
    expect(
      selectPageContributions(contributions, "drawer", nte).map(
        (page) => page.id,
      ),
    ).toEqual(["visible"]);
    expect(
      selectPageContributions(contributions, "home", nte).map(
        (page) => page.id,
      ),
    ).toEqual(["home-only"]);
  });
});

describe("页面可见性按当前游戏过滤", () => {
  it("异环页面只在异环激活时可见, fixture 反之", () => {
    expect(isPageVisible("nte-bulletin", nte)).toBe(true);
    expect(isPageVisible("nte-bulletin", hsr)).toBe(false);
    expect(isPageVisible("nte-bulletin", fixture)).toBe(false);
    expect(isPageVisible("fixture-fixture", fixture)).toBe(true);
    expect(isPageVisible("fixture-fixture", nte)).toBe(false);
    expect(isPageVisible("fixture-fixture", hsr)).toBe(false);
    expect(isPageVisible("unknown", nte)).toBe(false);
  });

  it("导航随当前游戏变化, fixture 页面只在自己激活时出现", () => {
    expect(navigationItems(fixture).map((item) => item.id)).toEqual([
      "home",
      "game-mode",
      "records",
      "settings",
      "fixture-fixture",
    ]);
    expect(navigationItems(hsr).map((item) => item.id)).not.toContain(
      "nte-bulletin",
    );
    expect(navigationItems(hsr).map((item) => item.id)).not.toContain(
      "fixture-fixture",
    );
  });

  it("不可见的页面不挂载", () => {
    expect(pagesForSlot("drawer", fixture).map((page) => page.id)).toEqual([
      "game-mode",
      "records",
      "settings",
      "runtime-setup",
      "fixture-fixture",
    ]);
    expect(pagesForSlot("drawer", hsr).map((page) => page.id)).not.toContain(
      "nte-bulletin",
    );
  });

  it("默认页与当前页回退到第一个可见页", () => {
    expect(firstVisiblePage(nte)).toBe("home");
    expect(firstVisiblePage(fixture)).toBe("home");
    expect(resolvePage("nte-bulletin", nte)).toBe("nte-bulletin");
    expect(resolvePage("nte-bulletin", fixture)).toBe("home");
    expect(resolvePage("fixture-fixture", fixture)).toBe("fixture-fixture");
    expect(resolvePage("fixture-fixture", nte)).toBe("home");
    expect(resolvePage("unknown", nte)).toBeNull();
  });
});

describe("游戏展示名", () => {
  it("由游戏包声明, 宿主不按发行渠道拼文案", () => {
    expect(gameDisplayName(nte)).toBe("异环");
    expect(gameDisplayName({ ...nte, distribution: "global" })).toBe("NTE");
    // 没有声明的游戏回退到 activeGame, 宿主不猜文案.
    expect(gameDisplayName(hsr)).toBe("hsr");
  });

  it("安装目录提示与海报文件名也由游戏包声明", () => {
    expect(gameInstallRootHint(nte)).toBe("~/Games/Neverness To Everness");
    expect(gameInstallRootHint({ ...nte, distribution: "global" })).toBe(
      "~/Games/Neverness To Everness Global",
    );
    expect(playtimePosterPrefix(nte)).toBe("异环游玩记录");
    // 没有声明的游戏不提示路径, 海报用通用名.
    expect(gameInstallRootHint(hsr)).toBe("");
    expect(playtimePosterPrefix(hsr)).toBe("游玩记录");
  });

  it("指名位置只在有展示声明时取游戏名", () => {
    expect(declaredGameName(nte)).toBe("异环");
    expect(declaredGameName({ ...nte, distribution: "global" })).toBe("NTE");
    expect(declaredGameName(hsr)).toBeNull();
  });
});
