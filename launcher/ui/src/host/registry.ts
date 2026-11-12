// 宿主 UI 的组件装配入口: 生成物加宿主表在这里合并, 涉及组件的宿主模块只取表.
// 页面元数据与预览声明走 generated/page-entries.ts, 避免把 .vue 拉进 controller.
import { packageContributions } from "./generated/package-contributions";
import type { HomeCardContribution } from "./home-cards";
import { compareByOrderAndId, type PageContribution } from "./pages";
import type { SettingsSectionContribution } from "./settings";

/** 各游戏包声明的页面组件表; 宿主页面组件在 contributions.ts 绑定. */
export const packagePageContributions: readonly PageContribution[] =
  packageContributions.flatMap(
    ({ contribution }) =>
      contribution.pageContributions as readonly PageContribution[],
  );

/** 首页卡片表, 由各包清单汇总并按 (order, id) 排序. */
export const homeCardContributions: readonly HomeCardContribution[] =
  sortByOrderAndId(
    packageContributions.flatMap(
      ({ contribution }) =>
        contribution.homeCards as readonly HomeCardContribution[],
    ),
  );

/** 设置分区表, 由各包清单汇总并按 (order, id) 排序. */
export const settingsSectionContributions: readonly SettingsSectionContribution[] =
  sortByOrderAndId(
    packageContributions.flatMap(
      ({ contribution }) =>
        contribution.settingsSections as readonly SettingsSectionContribution[],
    ),
  );

/** 贡献表统一按 order 升序, 同序按 id 字符序, 与目录扫描顺序无关. */
export function sortByOrderAndId<
  T extends { readonly id: string; readonly order: number },
>(contributions: readonly T[]): T[] {
  return [...contributions].sort(compareByOrderAndId);
}
