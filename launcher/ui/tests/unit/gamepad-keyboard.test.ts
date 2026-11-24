import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import type { PinyinCandidate } from "../../src/core/pinyin";

const loadPinyinEngine = vi.hoisted(() => vi.fn());
const notify = vi.hoisted(() => vi.fn());

vi.mock("../../src/core/notifications", async () => {
  const { errorText } = await import("../../src/core/notifications.ts");
  return { notify, errorText };
});

vi.mock("../../src/core/controller", async () => {
  const { reactive } = await import("vue");
  const { defaultGamepadOptions } = await import("../../src/types/gamepad");
  return { form: reactive({ gamepad: defaultGamepadOptions() }) };
});

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
    gamepadGlyph: (button: string) => `xbox-${button}`,
  };
});

import GamepadKeyboard from "../../src/components/GamepadKeyboard.vue";
import { gamepad } from "../../src/core/gamepad";
import { form } from "../../src/core/controller";
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

// 小写 -> 大写 -> 符号 -> 拼音 -> 小写.
async function cycle(wrapper: VueWrapper, times = 1) {
  for (let index = 0; index < times; index++)
    await button(wrapper, "键盘").trigger("click");
  await nextTick();
}

beforeEach(() => {
  form.gamepad.keyboardLayout = "qwerty";
  loadPinyinEngine.mockReset();
  notify.mockReset();
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
  gamepad.active = false;
  gamepadActions.keyboard = null;
});

afterEach(() => {
  for (const wrapper of wrappers) wrapper.unmount();
  wrappers = [];
});

describe("手柄拼音键盘", () => {
  it("输入拼音、选择候选并写入字段", async () => {
    const { wrapper, input } = await openKeyboard();
    await cycle(wrapper, 3);
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
    await cycle(wrapper, 3);
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
    await cycle(wrapper, 3);
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
    await cycle(wrapper, 3);
    await type(wrapper, ["n", "i"]);
    await button(wrapper, "空格").trigger("click");
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("你");
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
  });

  it("没有候选时可以原样上屏", async () => {
    const { wrapper, input } = await openKeyboard();
    await cycle(wrapper, 3);
    await type(wrapper, ["z", "z"]);
    await button(wrapper, "原样上屏").trigger("click");
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("zz");
    await button(wrapper, "完成").trigger("click");
    expect(input.value).toBe("zz");
  });

  it("退格优先删除拼音, 清空会收起候选", async () => {
    const { wrapper } = await openKeyboard();
    await cycle(wrapper, 3);
    await type(wrapper, ["n", "i"]);
    await button(wrapper, "删除").trigger("click");
    await flushPromises();
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("n");
    gamepadActions.keyboard?.clear();
    await nextTick();
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
    expect(wrapper.get(".pad-keyboard__display").text()).not.toContain("n");
  });

  it("密码字段不进入拼音", async () => {
    const { wrapper } = await openKeyboard("password");
    await cycle(wrapper, 3);
    expect(gamepadActions.keyboard?.pinyin).toBe(false);
    expect(button(wrapper, "键盘").exists()).toBe(true);
    expect(wrapper.get(".pad-keyboard__display").text()).toBe("输入密码");
  });

  it("拼音模式不显示数字键", async () => {
    const { wrapper } = await openKeyboard();
    expect(key(wrapper, "1").exists()).toBe(true);
    await cycle(wrapper, 3);
    expect(
      wrapper
        .findAll(".pad-keyboard__row:not(.pad-keyboard__actions) > button")
        .map((item) => item.text()),
    ).not.toContain("1");
    await type(wrapper, ["n", "i"]);
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("ni");
    await cycle(wrapper);
    expect(key(wrapper, "1").exists()).toBe(true);
  });

  it("切回字母模式会收起候选", async () => {
    const { wrapper } = await openKeyboard();
    await cycle(wrapper, 3);
    await type(wrapper, ["n", "i"]);
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(true);
    await cycle(wrapper);
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
    expect(gamepadActions.keyboard?.pinyin).toBe(false);
  });

  it("词库加载失败通过通知报告, 键盘内不再显示", async () => {
    loadPinyinEngine.mockRejectedValueOnce(new Error("HTTP 503"));
    const { wrapper } = await openKeyboard();
    await cycle(wrapper, 3);
    await flushPromises();
    expect(notify).toHaveBeenCalledWith(
      "拼音词库加载失败: HTTP 503",
      "error",
      6500,
    );
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    // 失败后仍可继续输入, 只是没有候选.
    await type(wrapper, ["z"]);
    expect(wrapper.get(".pad-keyboard__display").text()).toContain("z");
  });
});

describe("手柄键盘快捷键提示", () => {
  async function glyphs(wrapper: VueWrapper, label: string) {
    gamepad.active = true;
    await nextTick();
    return button(wrapper, label)
      .findAll(".gamepad-button-hint use")
      .map((glyph) => glyph.attributes("href")?.split("#")[1]);
  }

  it("动作按钮在右侧标明对应按键", async () => {
    const { wrapper } = await openKeyboard();
    expect(await glyphs(wrapper, "键盘")).toEqual(["xbox-select"]);
    expect(await glyphs(wrapper, "删除")).toEqual(["xbox-north"]);
    expect(await glyphs(wrapper, "完成")).toEqual(["xbox-start"]);
    await button(wrapper, "键盘").trigger("click");
    expect(await glyphs(wrapper, "键盘")).toEqual(["xbox-select"]);
  });

  it("候选翻页与时间键盘显示各自按键", async () => {
    const { wrapper } = await openKeyboard();
    await cycle(wrapper, 3);
    await type(wrapper, ["n", "i"]);
    expect(await glyphs(wrapper, "下一页")).toEqual(["xbox-rt"]);
    await button(wrapper, "下一页").trigger("click");
    expect(await glyphs(wrapper, "上一页")).toEqual(["xbox-lt"]);
    const time = await openKeyboard("time");
    expect(await glyphs(time.wrapper, "清空")).toEqual(["xbox-l3", "xbox-r3"]);
  });
});

describe("手柄键盘布局", () => {
  it.each(["qwerty", "alphabetical"] as const)(
    "%s 布局的字母、大小写和符号键区均为矩形",
    async (layout) => {
      form.gamepad.keyboardLayout = layout;
      const { wrapper, input } = await openKeyboard();
      const rows = () =>
        wrapper
          .findAll(".pad-keyboard__row:not(.pad-keyboard__actions)")
          .map((row) =>
            row
              .findAll("button")
              .map((item) => item.text())
              .join(""),
          );
      const letters =
        layout === "qwerty"
          ? ["qwertyuiop", "asdfghjkl;", "zxcvbnm,./"]
          : ["abcdefghij", "klmnopqrst", "uvwxyz,.;/"];
      expect(rows()).toEqual(["1234567890", ...letters]);
      await type(wrapper, ["a", ";", "/"]);
      await cycle(wrapper);
      expect(rows()).toEqual([
        "1234567890",
        ...letters.map((row) => row.toUpperCase()),
      ]);
      await key(wrapper, "Z").trigger("click");
      await cycle(wrapper);
      expect(rows().map((row) => row.length)).toEqual([8, 8, 8, 8]);
      await key(wrapper, "@").trigger("click");
      await button(wrapper, "完成").trigger("click");
      expect(input.value).toBe("a;/Z@");
    },
  );

  it("拼音使用所选布局, 标点将当前拼音上屏", async () => {
    form.gamepad.keyboardLayout = "alphabetical";
    const { wrapper, input } = await openKeyboard();
    await cycle(wrapper, 3);
    expect(
      wrapper
        .findAll(".pad-keyboard__row:not(.pad-keyboard__actions) button")
        .slice(0, 10)
        .map((item) => item.text())
        .join(""),
    ).toBe("abcdefghij");
    await type(wrapper, ["n", "i", ","]);
    expect(wrapper.get(".pad-keyboard__display").text()).toBe("ni,");
    expect(wrapper.find(".pad-keyboard__candidates").exists()).toBe(false);
    await button(wrapper, "完成").trigger("click");
    expect(input.value).toBe("ni,");
  });

  it("时间格式错误留在键盘内, 不弹通知", async () => {
    const { wrapper, input } = await openKeyboard("time");
    await button(wrapper, "完成").trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe(
      "请输入 24 小时时间 (HH:mm).",
    );
    expect(notify).not.toHaveBeenCalled();
    expect(gamepad.editing).not.toBeNull();
    for (const label of ["2", "1", ":", "3", "0"])
      await key(wrapper, label).trigger("click");
    await button(wrapper, "完成").trigger("click");
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(input.value).toBe("21:30");
  });

  it.each(["number", "tel", "time"])("%s 字段使用数字键盘", async (type) => {
    form.gamepad.keyboardLayout = "alphabetical";
    const { wrapper } = await openKeyboard(type);
    const rows = wrapper.findAll(
      ".pad-keyboard__row:not(.pad-keyboard__actions)",
    );
    expect(rows.map((row) => row.text())).toEqual([
      "123",
      "456",
      "789",
      type === "time" ? "0:" : "0.-",
    ]);
  });
});
