import { contribution } from "./contribution";

export { greeting } from "./greeting";
export { default as FixtureHomeCard } from "./fixture-home-card.vue";
export { default as FixturePage } from "./fixture-page.vue";
export { default as FixtureSettingsSection } from "./fixture-settings-section.vue";
export { default as HelloCard } from "./hello-card.vue";
export { contribution };
export { preview, preview as fixturePreviewContribution } from "./preview";
export { pages, pages as fixturePageEntries } from "./pages";

/** 包公共出口; 宿主装配从生成注册表取表, 不直接引用本模块. */
export const fixturePageContributions = contribution.pageContributions;
