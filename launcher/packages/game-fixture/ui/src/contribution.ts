import type { Component } from "vue";
import type { HostFacts, PackageContribution } from "@host/api";
import FixtureHomeCard from "./fixture-home-card.vue";
import FixturePage from "./fixture-page.vue";
import FixtureSettingsSection from "./fixture-settings-section.vue";
import { pages } from "./pages";

const whenFixture = (facts: HostFacts) => facts.activeGame === "fixture";

const settingsCapability = {
  id: "oddity.fixture.settings",
  revision: 1,
  necessity: "required",
} as const;

/** 页面组件绑定随页面声明一起由包提供, 宿主不再维护页面 id 到组件的映射. */
const pageComponents: Record<(typeof pages)[number]["id"], Component> = {
  "fixture-fixture": FixturePage,
};

/** fixture 包的全部 UI 贡献; 宿主经生成物装配, 不直接引用这里. */
export const contribution = {
  namespace: "fixture",
  pageContributions: pages.map((page) => ({
    ...page,
    component: pageComponents[page.id],
  })),
  homeCards: [
    {
      id: "fixture-home",
      order: 2,
      component: FixtureHomeCard,
      visible: whenFixture,
    },
  ],
  settingsSections: [
    {
      id: "fixture-settings",
      label: "测试设置",
      icon: "i-terminal-window",
      order: 8,
      component: FixtureSettingsSection,
      entries: [
        {
          id: "nickname",
          label: "测试昵称",
          description: "用于验证文本字段与默认值.",
          keywords: "fixture 昵称 文本 默认值",
        },
        {
          id: "showGreeting",
          label: "显示问候",
          description: "关闭后页面只显示标题.",
          keywords: "fixture 问候 开关",
        },
        {
          id: "greetingStyle",
          label: "问候样式",
          description: "选择页面上问候语的写法.",
          keywords: "fixture 问候 样式 选择",
        },
        {
          id: "maxRecords",
          label: "记录数量",
          description: "限制演示页展示的记录条数.",
          keywords: "fixture 记录 数量 整数",
        },
      ],
      requires: [settingsCapability],
      visible: whenFixture,
    },
  ],
} as const satisfies PackageContribution<"fixture">;
