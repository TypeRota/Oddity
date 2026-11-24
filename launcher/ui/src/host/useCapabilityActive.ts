import { computed, type ComputedRef } from "vue";
import { contributionActive, type CapabilityRequirement } from "./capability";
import { factsFrom } from "./facts";
import { state } from "../core/controller";

/**
 * 这组能力要求此刻是否满足, 作为响应式真假.
 *
 * 用于筛选选项, 列表等不是 DOM 挂载点的数据; 需要决定是否渲染时用
 * CapabilityGate, 两者共用 contributionActive, 不对同一份声明各判一次.
 */
export function useCapabilityActive(
  requires: readonly CapabilityRequirement[],
): ComputedRef<boolean> {
  return computed(() =>
    contributionActive(requires, factsFrom(state.bootstrap)),
  );
}
