/**
 * 贡献所需能力的类型与判定.
 *
 * 与 oddity-runtime 的 CapabilityRequirement 逐字段对应; UI 侧只关心能力 id,
 * revision 与必要性. 事实由宿主按当前上下文投影, 贡献不读取宿主状态.
 */

/** 能力要求的必要性: required 不满足则不挂载, optional 只交给 visible 与组件自行降级. */
export type Necessity = "required" | "optional";

/** 一条能力要求. */
export interface CapabilityRequirement {
  readonly id: string;
  /** 要求的能力版本; 只做精确匹配. */
  readonly revision: number;
  readonly necessity: Necessity;
}

/** 能力事实的可用状态; unsupported 表示平台不支持, unavailable 表示当前上下文不可用. */
export type CapabilityState = "available" | "unsupported" | "unavailable";

/** 宿主投影的一条能力事实. */
export interface CapabilityFact {
  readonly id: string;
  readonly revision: number;
  readonly state: CapabilityState;
  /** 不可用时的稳定原因码; 其它状态为 null. */
  readonly code: string | null;
  /** 不可用时的包内文案; 其它状态为 null. */
  readonly message: string | null;
}

/**
 * 能力事实的形状.
 *
 * 判定函数只依赖窄的 facts 接口, 不依赖宿主设置是否已投影能力.
 */
export interface CapabilityFacts {
  readonly capabilities: readonly CapabilityFact[];
}

/** 按能力 id 取事实; 没有投影时返回 undefined. */
export function capabilityOf(
  facts: CapabilityFacts,
  id: string,
): CapabilityFact | undefined {
  return facts.capabilities.find((fact) => fact.id === id);
}

/**
 * required 能力全部满足时返回 true.
 *
 * optional 不参与挂载判定; 缺少事实等同于不满足, 不做版本区间或降级匹配.
 */
export function contributionActive(
  requires: readonly CapabilityRequirement[] | undefined,
  facts: CapabilityFacts,
): boolean {
  return (requires ?? []).every((requirement) => {
    if (requirement.necessity !== "required") return true;
    const fact = capabilityOf(facts, requirement.id);
    return (
      fact?.state === "available" && fact.revision === requirement.revision
    );
  });
}

/**
 * 贡献是否挂载: required 能力全部可用, 且声明方的 visible 成立.
 *
 * optional 不参与这里的判定, 由声明方在 visible 中读取事实后决定降级.
 */
export function contributionVisible<F extends CapabilityFacts>(
  contribution: {
    readonly requires?: readonly CapabilityRequirement[];
    readonly visible: (facts: F) => boolean;
  },
  facts: F,
): boolean {
  return (
    contributionActive(contribution.requires, facts) &&
    contribution.visible(facts)
  );
}
