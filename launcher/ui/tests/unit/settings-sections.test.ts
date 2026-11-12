import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import * as c from "../../src/core/controller";
import { settingsNavigation } from "../../src/core/settingsCatalog";
import {
  selectSettingsSections,
  settingsSearchEntries,
  settingsSectionContributions,
  settingsSections,
  type SettingsSectionContribution,
} from "../../src/host/settings";
import SettingsPage from "../../src/pages/SettingsPage.vue";
import { factsFrom } from "../../src/host/facts";
import {
  bootstrap,
  capabilities,
  settings as defaultSettings,
} from "./fixtures";

const mounted: VueWrapper[] = [];
function renderSettings(): VueWrapper {
  const app = document.createElement("div");
  app.id = "app";
  document.body.append(app);
  const wrapper = mount(SettingsPage, { attachTo: document.body });
  mounted.push(wrapper);
  return wrapper;
}
/** 设置分区按当前 bootstrap 派生的事实过滤. */
const facts = () =>
  factsFrom(
    c.state.bootstrap ?? bootstrap({ capabilities, settings: defaultSettings }),
  );
function categoryIds(wrapper: VueWrapper): (string | undefined)[] {
  return wrapper
    .findAll("[data-category]")
    .map((node) => node.attributes("data-category"));
}
beforeEach(() => {
  c.dispose();
  settingsNavigation.category = "appearance";
  settingsNavigation.query = "";
});
afterEach(() => {
  mounted.splice(0).forEach((wrapper) => wrapper.unmount());
  c.dispose();
});

describe("设置分区贡献表", () => {
  it("游戏与账号分区由贡献声明, 与宿主分区按 order 合并", () => {
    expect(settingsSectionContributions.map((section) => section.id)).toEqual([
      "nte-game",
      "nte-account",
      "fixture-settings",
    ]);
    expect(settingsSections(facts()).map((section) => section.id)).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "nte-game",
      "runtime",
      "nte-account",
      "maintenance",
    ]);
    expect(
      settingsSections(facts()).find((section) => section.id === "nte-game"),
    ).toMatchObject({
      label: "游戏与启动",
      icon: "i-game-controller",
      order: 4,
    });
    expect(
      settingsSections(facts()).find((section) => section.id === "nte-account"),
    ).toMatchObject({ label: "账号与签到", icon: "i-users", order: 6 });
  });

  it("每条贡献都绑定组件, 并携带自己的检索项", () => {
    for (const section of settingsSectionContributions) {
      expect(section.component).toBeTruthy();
      expect(section.entries.length).toBeGreaterThan(0);
    }
    const entries = settingsSearchEntries(facts());
    expect(entries).toHaveLength(26);
    expect(
      entries
        .filter((entry) => entry.category === "nte-game")
        .map((entry) => entry.id),
    ).toEqual([
      "distribution",
      "install",
      "global-install",
      "migration",
      "launch",
      "game-account",
      "preload-dir",
      "updates",
    ]);
    expect(entries.at(-1)).toMatchObject({
      id: "account",
      category: "nte-account",
      label: "塔吉多签到",
    });
  });

  it("当前游戏不是异环时不渲染游戏分区, 搜索索引同步剔除", () => {
    expect(selectSettingsSections(facts())).toHaveLength(2);
    c.state.bootstrap = bootstrap({
      settings: { ...defaultSettings, activeGame: "hsr" },
    });
    expect(selectSettingsSections(facts())).toEqual([]);
    expect(settingsSections(facts()).map((section) => section.id)).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "runtime",
      "maintenance",
    ]);
    const entries = settingsSearchEntries(facts());
    expect(entries).toHaveLength(17);
    expect(
      entries.some((entry) =>
        ["nte-game", "nte-account"].includes(entry.category),
      ),
    ).toBe(false);
  });

  it("自定义贡献按自身可见条件过滤后排序", () => {
    const component = settingsSectionContributions[0].component;
    const contributions: SettingsSectionContribution[] = [
      {
        id: "account",
        label: "账号",
        icon: "i-users",
        order: 9,
        component,
        entries: [],
        visible: () => true,
      },
      {
        id: "game",
        label: "游戏",
        icon: "i-game-controller",
        order: 2,
        component,
        entries: [],
        visible: () => false,
      },
    ];
    expect(
      selectSettingsSections(facts(), contributions).map(
        (section) => section.id,
      ),
    ).toEqual(["account"]);
    expect(
      settingsSections(facts(), contributions).map((section) => section.id),
    ).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "runtime",
      "maintenance",
      "account",
    ]);
    expect(settingsSearchEntries(facts(), contributions)).toHaveLength(17);
  });
});

describe("设置页分区渲染", () => {
  it("游戏分区按可见条件进入导航, 访问后挂载贡献组件", async () => {
    c.state.bootstrap = bootstrap();
    c.state.page = "settings";
    const wrapper = renderSettings();
    await nextTick();
    expect(categoryIds(wrapper)).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "nte-game",
      "runtime",
      "nte-account",
      "maintenance",
    ]);
    expect(wrapper.find("#settings-panel-nte-game").exists()).toBe(true);
    await wrapper.get('[data-category="nte-game"]').trigger("click");
    await nextTick();
    expect(wrapper.find("#set-install").exists()).toBe(true);
    expect(wrapper.find("#set-mode").exists()).toBe(true);
    expect(wrapper.find('[data-setting="distribution"]').exists()).toBe(true);
  });

  it("切换游戏后卸载游戏分区, 当前分区回到第一个宿主分区", async () => {
    c.state.bootstrap = bootstrap();
    c.state.page = "settings";
    settingsNavigation.category = "nte-game";
    const wrapper = renderSettings();
    await nextTick();
    expect(wrapper.find("#set-install").exists()).toBe(true);
    c.state.bootstrap = bootstrap({
      settings: { ...defaultSettings, activeGame: "hsr" },
    });
    await nextTick();
    expect(categoryIds(wrapper)).toEqual([
      "appearance",
      "audio",
      "gamepad",
      "runtime",
      "maintenance",
    ]);
    expect(wrapper.find("#settings-panel-nte-game").exists()).toBe(false);
    expect(wrapper.find('[data-category="nte-account"]').exists()).toBe(false);
    expect(settingsNavigation.category).toBe("appearance");
  });

  it("fixture 设置分区按声明字段渲染通用表单", async () => {
    c.state.bootstrap = bootstrap({
      settings: { ...defaultSettings, activeGame: "fixture" },
      settingsSchemas: [
        {
          owner: "game:fixture",
          version: 2,
          fields: [
            {
              key: "nickname",
              label: "测试昵称",
              description: "用于验证文本字段与默认值.",
              order: 1,
              type: "text",
              default: "旅行者",
              maxChars: 24,
              value: "旅行者",
            },
            {
              key: "showGreeting",
              label: "显示问候",
              description: "关闭后页面只显示标题.",
              order: 2,
              type: "boolean",
              default: true,
              value: true,
            },
            {
              key: "greetingStyle",
              label: "问候样式",
              description: "选择页面上问候语的写法.",
              order: 3,
              type: "select",
              default: "plain",
              options: ["plain", "formal"],
              value: "plain",
            },
            {
              key: "maxRecords",
              label: "记录数量",
              description: "限制演示页展示的记录条数.",
              order: 4,
              type: "integer",
              default: 5,
              min: 1,
              max: 20,
              value: 5,
            },
          ],
        },
      ],
    });
    c.state.page = "settings";
    settingsNavigation.category = "fixture-settings";
    const wrapper = renderSettings();
    await nextTick();
    expect(categoryIds(wrapper)).toContain("fixture-settings");
    expect(wrapper.find('[data-setting="nickname"]').exists()).toBe(true);
    expect(wrapper.find('[data-setting="showGreeting"]').exists()).toBe(true);
    expect(wrapper.find('[data-setting="greetingStyle"]').exists()).toBe(true);
    expect(wrapper.find('[data-setting="maxRecords"]').exists()).toBe(true);
  });

  it("搜索名称与关键词覆盖宿主分区和可见的贡献分区", async () => {
    c.state.bootstrap = bootstrap();
    c.state.page = "settings";
    const wrapper = renderSettings();
    await nextTick();
    const search = wrapper.get("#settings-search");
    await search.setValue("路径");
    await nextTick();
    expect(wrapper.findAll(".settings-result")).toHaveLength(3);
    expect(
      wrapper.findAll(".settings-result").map((node) => node.text()),
    ).toEqual([
      expect.stringContaining("国服安装目录"),
      expect.stringContaining("国际服安装目录"),
      expect.stringContaining("迁移安装目录"),
    ]);
    await search.setValue("亮色");
    await nextTick();
    expect(wrapper.findAll(".settings-result")).toHaveLength(1);
    expect(wrapper.get(".settings-result").text()).toContain("外观与交互");
  });

  it("没有可见游戏分区时搜索不再返回游戏设置", async () => {
    c.state.bootstrap = bootstrap({
      settings: { ...defaultSettings, activeGame: "hsr" },
    });
    c.state.page = "settings";
    const wrapper = renderSettings();
    await nextTick();
    const search = wrapper.get("#settings-search");
    await search.setValue("路径");
    await nextTick();
    expect(wrapper.findAll(".settings-result")).toHaveLength(0);
    expect(wrapper.get(".settings-results-count").text()).toBe("未找到“路径”");
  });
});
