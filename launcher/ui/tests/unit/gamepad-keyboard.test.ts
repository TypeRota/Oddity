import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import type { PinyinCandidate } from "../../src/core/pinyin";

const loadPinyinEngine = vi.hoisted(() => vi.fn());

vi.mock("../../src/core/pinyin", () => ({ loadPinyinEngine }));

vi.mock("../../src/core/gamepad", async () => {
  const { reactive, shallowRef } = await import("vue");
  return {
    gamepad: reactive({
      editing: null as HTMLInputElement | null,
      active: false,
      pressing: false,
    }),
    gamepadTarget: shallowRef(null),
    gamepadGlyph: () => "xbox-a",
  };
});

import GamepadKeyboard from "../../src/components/GamepadKeyboard.vue";
import { gamepad } from "../../src/core/gamepad";
import { gamepadActions } from "../../src/core/gamepadActions";

let wrappers: VueWrapper[] = [];

function engineFor(search: (input: string) => PinyinCandidate[]) {
  return { search };
}

async function openKeyboard(type = "text") {
  const input = document.createElement("input");
  input.type = type;
  document.body.append(input);
  const wrapper = mount(GamepadKeyboard, { attachTo: document.body });
  wrappers.push(wrapper);
  gamepad.editing = input;
  await nextTick();
  await flushPromises();
  return { wrapper, input };
}

function button(wrapper: VueWrapper, label: string) {
  const found = wrapper.findAll("button").find((item) => item.text() === label);
  if (!found) throw new Error(`缺少按钮 ${label}`);
  return found;
}

function key(wrapper: VueWrapper, label: string) {
  const found = wrapper
    .findAll(".pad-keyboard__row:not(.pad-keyboard__actions) > button")
    .find((item) => item.text() === label);
  if (!found) throw new Error(`缺少按键 ${label}`);
  return found;
}

async function type(wrapper: VueWrapper, letters: string[]) {
  for (const letter of letters) await key(wrapper, letter).trigger("click");
  await flushPromises();
}

beforeEach(() => {
  loadPinyinEngine.mockReset();
  loadPinyinEngine.mockResolvedValue(
    engineFor((input) => {
      if (input === "nihao")
        return [
          { word: "你好", matchedLength: 5, frequency: 100 },
          { word: "你", matchedLength: 2, frequency: 90 },
        ];
      if (input === "ni")
        return [
          { word: "你", matchedLength: 2, frequency: 90 },
          { word: "泥", matchedLength: 2, frequency: 80 },
          { word: "尼", matchedLength: 2, frequency: 70 },
          { word: "拟", matchedLength: 2, frequency: 60 },
          { word: "逆", matchedLength: 2, frequency: 50 },
          { word: "腻", matchedLength: 2, frequency: 40 },
        ];
      return [];
    }),
  );
  gamepad.editing = null;
  gamepadActions.keyboard = null;
});

afterEach(() => {
  for (const wrapper of wrappers) wrapper.unmount();
  wrappers = [];
});

describe("手柄拼音键盘", () => {
  it("输入拼音、选择候选并写入字段", async () => {
    const { wrapper, input } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    expect(gamepadActions.keyboard?.pinyin).toBe(true);
    await type(wrapper, ["n", "i", "h", "a", "o"]);
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("nihao");
    const candidate = wrapper
      .findAll(".pad-keyboard__candidates button")
      .find((item) => item.text().includes("你好"));
    expect(candidate).toBeTruthy();
    await candidate!.trigger("click");
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("你好");
    await button(wrapper, "完成").trigger("click");
    expect(input.value).toBe("你好");
    expect(gamepad.editing).toBeNull();
  });

  it("候选超过一页时翻页", async () => {
    const { wrapper } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    await type(wrapper, ["n", "i"]);
    expect(
      wrapper
        .findAll(".pad-keyboard__candidates button")
        .map((item) => item.text()),
    ).toEqual([
      "1. 你",
      "2. 泥",
      "3. 尼",
      "4. 拟",
      "5. 逆",
      "上一页",
      "下一页",
    ]);
    await button(wrapper, "下一页").trigger("click");
    expect(
      wrapper
        .findAll(".pad-keyboard__candidates button")
        .map((item) => item.text()),
    ).toContain("1. 腻");
    expect(gamepadActions.keyboard?.page?.(1)).toBe(false);
    expect(gamepadActions.keyboard?.page?.(-1)).toBe(true);
  });

  it("选词只消耗匹配的拼音", async () => {
    const { wrapper } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    await type(wrapper, ["n", "i", "h", "a", "o"]);
    const partial = wrapper
      .findAll(".pad-keyboard__candidates button")
      .find(
        (item) => item.text().includes("你") && !item.text().includes("你好"),
      );
    expect(partial).toBeTruthy();
    await partial!.trigger("click");
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("你hao");
  });

  it("空格选择第一个候选", async () => {
    const { wrapper } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    await type(wrapper, ["n", "i"]);
    await button(wrapper, "空格").trigger("click");
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("你");
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
  });

  it("没有候选时可以原样上屏", async () => {
    const { wrapper, input } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    await type(wrapper, ["z", "z"]);
    await button(wrapper, "原样上屏").trigger("click");
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("zz");
    await button(wrapper, "完成").trigger("click");
    expect(input.value).toBe("zz");
  });

  it("退格优先删除拼音, 清空会收起候选", async () => {
    const { wrapper } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    await type(wrapper, ["n", "i"]);
    await button(wrapper, "删除").trigger("click");
    await flushPromises();
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("n");
    gamepadActions.keyboard?.clear();
    await nextTick();
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
    expect(wrapper.get(".pad-keyboard__display").text()).not.toContain("n");
  });

  it("密码字段不提供拼音入口", async () => {
    const { wrapper } = await openKeyboard("password");
    expect(
      wrapper.findAll("button").some((item) => item.text() === "拼音"),
    ).toBe(false);
    expect(
      wrapper.findAll("button").some((item) => item.text() === "大写"),
    ).toBe(true);
    expect(wrapper.get(".pad-keyboard__display").text()).toBe("输入密码");
  });

  it("拼音模式不显示数字键", async () => {
    const { wrapper } = await openKeyboard();
    expect(key(wrapper, "1").exists()).toBe(true);
    await button(wrapper, "拼音").trigger("click");
    expect(
      wrapper
        .findAll(".pad-keyboard__row:not(.pad-keyboard__actions) > button")
        .map((item) => item.text()),
    ).not.toContain("1");
    await type(wrapper, ["n", "i"]);
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("ni");
    await button(wrapper, "字母").trigger("click");
    expect(key(wrapper, "1").exists()).toBe(true);
  });

  it("切回字母模式会收起候选", async () => {
    const { wrapper } = await openKeyboard();
    await button(wrapper, "拼音").trigger("click");
    await type(wrapper, ["n", "i"]);
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(true);
    await button(wrapper, "字母").trigger("click");
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
    expect(gamepadActions.keyboard?.pinyin).toBe(false);
  });
});
