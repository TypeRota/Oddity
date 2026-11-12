import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import {
  FixturePage,
  fixturePageContributions,
  fixturePageEntries,
  greeting,
  HelloCard,
} from "@packages/game-fixture/ui";

describe("包 UI 入口", () => {
  it("导出页面声明与按游戏判断的可见条件", () => {
    expect(fixturePageEntries.map((page) => page.id)).toEqual([
      "fixture-fixture",
    ]);
    expect(
      fixturePageEntries[0].visible({
        activeGame: "fixture",
        distribution: "cn",
        capabilities: [],
        channels: [],
      }),
    ).toBe(true);
    expect(
      fixturePageEntries[0].visible({
        activeGame: "nte",
        distribution: "cn",
        capabilities: [],
        channels: [],
      }),
    ).toBe(false);
  });

  it("页面组件绑定随包声明, 宿主不再按页面 id 映射", () => {
    expect(fixturePageContributions.map((page) => page.id)).toEqual([
      "fixture-fixture",
    ]);
    expect(fixturePageContributions[0].component).toBe(FixturePage);
  });

  it("通过 @packages 别名解析到包内纯函数", () => {
    expect(greeting("  旅行者 ")).toBe("旅行者, 欢迎回来");
    expect(greeting(" ")).toBe("欢迎回来");
  });

  it("通过 @packages 别名渲染包内组件", () => {
    const wrapper = mount(HelloCard, { props: { name: "旅行者" } });
    expect(wrapper.get(".hello-card").text()).toBe("旅行者, 欢迎回来");
    wrapper.unmount();
  });

  it("通过宿主构建链路加载包内页面", () => {
    const wrapper = mount(FixturePage, { props: { name: "旅行者" } });
    expect(wrapper.find("[data-fixture-page]").exists()).toBe(true);
    expect(wrapper.get(".hello-card").text()).toBe("旅行者, 欢迎回来");
    expect(wrapper.text()).toContain("仅供开发与契约测试引用");
    wrapper.unmount();
  });

  it("页面未传名字时回退到 fixture", () => {
    const wrapper = mount(FixturePage);
    expect(wrapper.get(".hello-card").text()).toBe("fixture, 欢迎回来");
    wrapper.unmount();
  });
});
