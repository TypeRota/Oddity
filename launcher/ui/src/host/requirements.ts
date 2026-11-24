import type { CapabilityRequirement } from "./capability";

/**
 * 宿主页面依赖的能力要求.
 *
 * id 与 revision 对应宿主硬件提供者声明的能力; 页面只声明需要什么,
 * 由宿主按当前上下文求值, 不读探测结果. 游戏包的要求在包内声明.
 */

/** CPU 调优: 核心绑定, 优先级与 I/O 优先级经 GameMode 下发. */
export const cpuTuning = {
  id: "oddity.tuning.cpu",
  revision: 1,
  necessity: "required",
} as const satisfies CapabilityRequirement;

/** GPU 调优: 超频与功耗上限经 GameMode 下发, 需要可调优的显卡. */
export const gpuTuning = {
  id: "oddity.tuning.gpu",
  revision: 1,
  necessity: "required",
} as const satisfies CapabilityRequirement;
