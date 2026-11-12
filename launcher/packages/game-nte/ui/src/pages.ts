import type { HostFacts, PageEntry } from "@host/api";

/**
 * 异环的页面声明.
 *
 * 只描述位置与可见性, 组件绑定在 contribution.ts; 宿主把这份元数据并进
 * 页面登记表, 只在异环激活时把它放进导航与页面白名单.
 */
export const pages = [
  {
    id: "nte-update",
    slot: "drawer",
    order: 2,
    nav: { label: "更新", icon: "download-simple" },
    visible: (facts: HostFacts) => facts.activeGame === "nte",
  },
  {
    id: "nte-bulletin",
    slot: "drawer",
    order: 4,
    nav: { label: "公告", icon: "newspaper-clipping" },
    visible: (facts: HostFacts) => facts.activeGame === "nte",
  },
] as const satisfies readonly PageEntry[];
