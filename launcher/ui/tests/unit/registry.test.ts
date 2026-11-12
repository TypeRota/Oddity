import { expect, it } from "vitest";
import { sortByOrderAndId } from "../../src/host/registry";

it("贡献按 order 排序, 同序按 id 排序", () => {
  expect(
    sortByOrderAndId([
      { id: "b", order: 2 },
      { id: "c", order: 1 },
      { id: "a", order: 2 },
    ]).map((item) => item.id),
  ).toEqual(["c", "a", "b"]);
});
