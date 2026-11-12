import type { HomeCardContribution } from "./home-cards";
import type { PageContribution } from "./pages";
import type { SettingsSectionContribution } from "./settings";

/**
 * 一个游戏包对外声明的组件类 UI 贡献.
 *
 * 页面元数据, 游戏展示与预览命令由生成的 page-entries 聚合; 这里只放需要
 * 绑定 Vue 组件的贡献, 避免把 .vue 拉进纯声明模块的依赖图.
 */
export interface PackageContribution<N extends string = string> {
  /** 包命名空间; 页面, 卡片与分区的 id 由它派生前缀. */
  readonly namespace: N;
  readonly pageContributions: readonly PageContribution[];
  readonly homeCards: readonly HomeCardContribution[];
  readonly settingsSections: readonly SettingsSectionContribution[];
}
