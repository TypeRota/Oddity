import type { HostFacts, PageEntry } from "@host/api";

/**
 * 测试包的页面声明.
 *
 * 可见条件写成 activeGame 判断, 宿主只在 fixture 激活时把它放进导航与
 * 页面白名单; 演示与正式构建里 fixture 不会被激活.
 */
export const pages = [
  {
    id: "fixture-fixture",
    slot: "drawer",
    order: 8,
    nav: { label: "测试页", icon: "terminal-window" },
    visible: (facts: HostFacts) => facts.activeGame === "fixture",
  },
] as const satisfies readonly PageEntry[];
