import { describe, expect, it } from "vitest";
import {
  capabilityOf,
  contributionActive,
  type CapabilityFact,
  type CapabilityRequirement,
} from "../../src/host/capability";

const facts: CapabilityFact[] = [
  { id: "login", revision: 2, state: "available", code: null, message: null },
  {
    id: "signin",
    revision: 1,
    state: "unavailable",
    code: "offline",
    message: "离线",
  },
];

const required = (id: string, revision: number): CapabilityRequirement => ({
  id,
  revision,
  necessity: "required",
});

describe("能力判定", () => {
  it("按 id 取事实", () => {
    expect(capabilityOf({ capabilities: facts }, "login")).toMatchObject({
      revision: 2,
      state: "available",
    });
    expect(capabilityOf({ capabilities: facts }, "missing")).toBeUndefined();
  });

  it("required 要求事实存在, 可用且 revision 精确匹配", () => {
    expect(contributionActive(undefined, { capabilities: facts })).toBe(true);
    expect(
      contributionActive([required("login", 2)], { capabilities: facts }),
    ).toBe(true);
    expect(
      contributionActive([required("login", 1)], { capabilities: facts }),
    ).toBe(false);
    expect(
      contributionActive([required("missing", 1)], { capabilities: facts }),
    ).toBe(false);
    expect(
      contributionActive([required("signin", 1)], { capabilities: facts }),
    ).toBe(false);
  });

  it("optional 不参与挂载判定", () => {
    expect(
      contributionActive(
        [{ id: "missing", revision: 1, necessity: "optional" }],
        { capabilities: facts },
      ),
    ).toBe(true);
  });
});
