import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { computed, defineComponent, h, nextTick } from "vue";
import * as c from "../../src/core/controller";
import { pagesForSlot } from "../../src/host/contributions";
import { factsFrom } from "../../src/host/facts";
import HostSlot from "../../src/host/HostSlot.vue";
import NavigationRail from "../../src/components/layout/NavigationRail.vue";
import { bootstrap, settings as defaultSettings } from "./fixtures";
import type { Settings } from "../../src/types/launcher";

/** 与 App.vue 相同的宿主装配: 主导航加两个页面槽, 可见性由贡献自己判断. */
const Shell = defineComponent({
  setup() {
    const facts = computed(() => factsFrom(c.state.bootstrap));
    return () =>
      h("div", { class: "app", id: "app" }, [
        h(NavigationRail),
        h(HostSlot, {
          contributions: pagesForSlot("home", facts.value),
          area: "home",
        }),
        h(HostSlot, {
          contributions: pagesForSlot("drawer", facts.value),
          area: "drawer",
        }),
      ]);
  },
});

/** fixture 没有对应的设置枚举, 这里模拟后端把 activeGame 报成包 id. */
function settingsFor(game: string): Settings {
  return { ...defaultSettings, activeGame: game as Settings["activeGame"] };
}

const mounted: VueWrapper[] = [];
async function render(game: string): Promise<VueWrapper> {
  c.state.bootstrap = bootstrap({ settings: settingsFor(game) });
  const wrapper = mount(Shell, { attachTo: document.body });
  mounted.push(wrapper);
  await nextTick();
  return wrapper;
}
function navIds(wrapper: VueWrapper): (string | undefined)[] {
  return wrapper
    .findAll("[data-page]")
    .map((node) => node.attributes("data-page"));
}
function categoryIds(wrapper: VueWrapper): (string | undefined)[] {
  // 游戏模式页也有 data-category, 这里只看设置页的分区导航.
  return wrapper
    .findAll("#page-settings [data-category]")
    .map((node) => node.attributes("data-category"));
}

beforeEach(() => {
  c.dispose();
});
afterEach(() => {
  mounted.splice(0).forEach((wrapper) => wrapper.unmount());
  c.dispose();
});

describe("游戏级可见性", () => {
  it("fixture 激活时异环的页面、卡片与设置分区都不挂 DOM", async () => {
    const wrapper = await render("fixture");
    expect(wrapper.find("#page-fixture-fixture").exists()).toBe(true);
    expect(wrapper.find("[data-fixture-page]").exists()).toBe(true);
    // 宿主页面照常挂载: 只有异环贡献的内容消失.
    expect(wrapper.find("#page-home").exists()).toBe(true);
    // 更新页由异环包贡献, fixture 激活时随包卸载.
    expect(wrapper.find("#page-nte-update").exists()).toBe(false);
    expect(wrapper.find("#page-settings").exists()).toBe(true);
    // 异环的公告页、玩家卡与设置分区都不在 DOM 里, fixture 分区出现.
    expect(wrapper.find("#page-nte-bulletin").exists()).toBe(false);
    expect(wrapper.find(".game-records").exists()).toBe(false);
    expect(categoryIds(wrapper)).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "runtime",
      "maintenance",
      "fixture-settings",
    ]);
    expect(navIds(wrapper)).toEqual([
      "home",
      "game-mode",
      "records",
      "settings",
      "fixture-fixture",
    ]);
    // fixture 激活时比宿主页面表少一个由异环包贡献的更新页.
    expect(wrapper.findAll(".page")).toHaveLength(6);
  });

  it("异环激活时 fixture 的页面不挂 DOM", async () => {
    const wrapper = await render("nte");
    expect(wrapper.find("#page-nte-bulletin").exists()).toBe(true);
    expect(wrapper.find(".game-records").exists()).toBe(true);
    expect(wrapper.find("#page-fixture-fixture").exists()).toBe(false);
    // 更新页由异环包贡献, nte 激活时出现在导航里.
    expect(wrapper.find("#page-nte-update").exists()).toBe(true);
    expect(navIds(wrapper)).toEqual([
      "home",
      "nte-update",
      "game-mode",
      "nte-bulletin",
      "records",
      "settings",
    ]);
    expect(categoryIds(wrapper)).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "nte-game",
      "runtime",
      "nte-account",
      "maintenance",
    ]);
  });

  it("切换当前游戏后按新游戏重新挂载", async () => {
    const wrapper = await render("nte");
    expect(wrapper.find("#page-nte-bulletin").exists()).toBe(true);
    c.state.bootstrap = bootstrap({ settings: settingsFor("fixture") });
    await nextTick();
    expect(wrapper.find("#page-nte-bulletin").exists()).toBe(false);
    expect(wrapper.find("#page-fixture-fixture").exists()).toBe(true);
    expect(wrapper.find(".game-records").exists()).toBe(false);
    expect(categoryIds(wrapper)).not.toContain("nte-game");
    c.state.bootstrap = bootstrap({ settings: settingsFor("nte") });
    await nextTick();
    expect(wrapper.find("#page-nte-bulletin").exists()).toBe(true);
    expect(wrapper.find("#page-fixture-fixture").exists()).toBe(false);
    expect(wrapper.find(".game-records").exists()).toBe(true);
  });
});

describe("当前页与默认页", () => {
  it("切换游戏后当前页不可见时回退到第一个可见页", () => {
    c.state.bootstrap = bootstrap();
    c.showPage("nte-bulletin");
    expect(c.state.page).toBe("nte-bulletin");
    c.applyBootstrap(bootstrap({ settings: settingsFor("fixture") }));
    expect(c.state.page).toBe("home");
    expect(window.location.hash).toBe("#home");
  });

  it("请求不可见的页面时回退, 不进入空白页", () => {
    c.state.bootstrap = bootstrap({ settings: settingsFor("fixture") });
    c.showPage("nte-bulletin");
    expect(c.state.page).toBe("home");
    c.showPage("fixture-fixture");
    expect(c.state.page).toBe("fixture-fixture");
    // 白名单之外的 id 保持当前页.
    c.showPage("unknown");
    expect(c.state.page).toBe("fixture-fixture");
    c.state.bootstrap = bootstrap();
    c.showPage("nte-bulletin");
    c.showPage("fixture-fixture");
    expect(c.state.page).toBe("home");
  });
});
