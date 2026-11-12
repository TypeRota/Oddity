import { mount } from "@vue/test-utils";
import { expect, it, vi } from "vitest";
import * as c from "../../src/core/controller";

// 空表用于验证没有卡片贡献时首页不保留任何占位.
vi.mock("../../src/host/home-cards", () => ({
  homeCardContributions: [],
  selectHomeCards: () => [],
}));

import HomePage from "../../src/pages/HomePage.vue";

it("首页卡片表为空时不渲染占位", () => {
  c.dispose();
  const wrapper = mount(HomePage);
  expect(wrapper.find(".hero__inner").exists()).toBe(true);
  expect(wrapper.find(".game-records").exists()).toBe(false);
  wrapper.unmount();
});
