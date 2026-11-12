import { contribution } from "./contribution";

export { default as BulletinPage } from "./domains/bulletin/BulletinPage.vue";
export { default as UpdatePage } from "./domains/update/UpdatePage.vue";
export { default as OptionalResources } from "./domains/update/OptionalResources.vue";
export { default as PlayerInfoCard } from "./domains/player-info/PlayerInfoCard.vue";
export { contribution };
export { game, game as nteGame } from "./game";
export { preview, preview as ntePreviewContribution } from "./preview";
export { pages, pages as ntePageEntries } from "./pages";

/** 包公共出口; 宿主装配从生成注册表取表, 不直接引用本模块. */
export const ntePageContributions = contribution.pageContributions;
export const nteSettingsSections = contribution.settingsSections;
