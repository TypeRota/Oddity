import { describe, expect, it } from "vitest";
// 页面模块先于贡献表求值: 表里的组件不能因此变成 undefined.
import PlaytimePage from "../../src/pages/PlaytimePage.vue";
import { pageContributions } from "../../src/host/contributions";

describe("页面贡献表与页面模块的求值顺序", () => {
  it("页面模块先求值时, 每条贡献仍有组件", () => {
    expect(PlaytimePage).toBeTruthy();
    expect(pageContributions.every((page) => page.component)).toBe(true);
    expect(
      pageContributions.find((page) => page.id === "records")?.component,
    ).toBe(PlaytimePage);
  });
});
