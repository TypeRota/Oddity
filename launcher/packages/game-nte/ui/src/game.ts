import type { GameContribution, HostFacts } from "@host/api";

/**
 * 异环的展示声明.
 *
 * 宿主不按发行渠道拼游戏名: 首页标题等通用位置取包声明的展示名, 国服与
 * 国际服各自使用发行名. 安装目录提示与游玩海报文件名也由包给出, 宿主不写死
 * 游戏文案.
 */
export const game = {
  id: "nte",
  displayName: (facts: HostFacts) =>
    facts.distribution === "global" ? "NTE" : "异环",
  installRootHint: (facts: HostFacts) =>
    facts.distribution === "global"
      ? "~/Games/Neverness To Everness Global"
      : "~/Games/Neverness To Everness",
  playtimePosterPrefix: () => "异环游玩记录",
} as const satisfies GameContribution;

/** 包公共出口; 宿主装配从生成注册表取表, 不直接引用本模块. */
export const nteGame = game;
