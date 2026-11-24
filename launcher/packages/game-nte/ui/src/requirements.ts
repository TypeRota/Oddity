import type { CapabilityRequirement } from "@host/api";

/**
 * 异环界面依赖的能力要求.
 *
 * id 与 revision 对应后端 `oddity-game-nte` 的 `CapabilityDescriptor`; 界面只声明
 * 自己需要什么, 由宿主按当前上下文求值, 不读宿主状态.
 */

/** 官方客户端账号: 短信, 登录, 账号列表与直接启动凭证; 仅国服提供. */
export const accountGame = {
  id: "oddity.account.game",
  revision: 1,
  necessity: "required",
} as const satisfies CapabilityRequirement;

/** 塔吉多社区账号与角色面板; 仅国服提供. */
export const playerProfile = {
  id: "oddity.profile.player",
  revision: 1,
  necessity: "required",
} as const satisfies CapabilityRequirement;
