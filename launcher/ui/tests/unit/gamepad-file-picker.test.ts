import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";

const invoke = vi.hoisted(() => vi.fn());
const notify = vi.hoisted(() => vi.fn());
vi.mock("../../src/core/api", () => ({ invoke }));
vi.mock("../../src/core/notifications", async () => {
  const { errorText } = await import("../../src/core/notifications.ts");
  return { notify, errorText };
});
vi.mock("../../src/components/BaseModal.vue", () => ({
  default: {
    props: ["open", "title", "theme", "reduceMotion"],
    template: '<div v-if="open" role="dialog"><slot /></div>',
  },
}));
vi.mock("../../src/core/controller", async () => {
  const { reactive } = await import("vue");
  return {
    form: reactive({ theme: "dark", reduceMotion: false }),
  };
});
vi.mock("../../src/core/gamepad", async () => {
  const { reactive, shallowRef } = await import("vue");
  return {
    gamepad: reactive({ active: false, pressing: false }),
    gamepadTarget: shallowRef(null),
    gamepadGlyph: (button: string) => `xbox-${button}`,
  };
});

import GamepadFilePicker from "../../src/components/GamepadFilePicker.vue";
import { fileRequest } from "../../src/core/filePicker";
import { gamepad } from "../../src/core/gamepad";

let wrapper: VueWrapper | undefined;

function open(directory: boolean, save = false) {
  wrapper = mount(GamepadFilePicker, { attachTo: document.body });
  fileRequest.value = {
    title: "选择目录",
    directory,
    save,
    defaultPath: "/home/test",
    extensions: [],
    resolve: vi.fn(),
  };
  return wrapper;
}

async function hints(label: string) {
  await nextTick();
  gamepad.active = true;
  await nextTick();
  const found = wrapper!
    .findAll("button")
    .find((item) => item.text() === label);
  if (!found) throw new Error(`缺少按钮 ${label}`);
  return found
    .findAll(".gamepad-button-hint use")
    .map((glyph) => glyph.attributes("href")?.split("#")[1]);
}

beforeEach(() => {
  invoke.mockReset().mockResolvedValue({
    path: "/home/test",
    parent: "/home",
    home: "/home/test",
    entries: [],
  });
  notify.mockReset();
  gamepad.active = false;
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  fileRequest.value = null;
});

it("选择目录时工具栏与操作按钮标明对应手柄按键", async () => {
  open(true);
  await flushPromises();
  expect(await hints("上一级")).toEqual(["xbox-west"]);
  expect(await hints("取消")).toEqual(["xbox-back"]);
  expect(await hints("选择此文件夹")).toEqual(["xbox-start"]);
});

it("保存文件时保存按钮使用开始键提示", async () => {
  open(false, true);
  await flushPromises();
  expect(await hints("保存")).toEqual(["xbox-start"]);
  await wrapper!.get("#pad-file-name").setValue("config.json");
  await flushPromises();
  expect(await hints("保存")).toEqual(["xbox-start"]);
});

it("读取文件夹失败通过通知报告, 允许返回主目录重试", async () => {
  invoke.mockRejectedValueOnce(new Error("目录不存在"));
  open(true);
  await flushPromises();
  expect(notify).toHaveBeenCalledWith(
    "读取文件夹失败: 目录不存在",
    "error",
    6500,
  );
  expect(wrapper!.find('[role="alert"]').exists()).toBe(false);
  // 失败后不再占用忙碌状态, 可以换个目录重试.
  await wrapper!.get("#pad-file-path").setValue("/home/test");
  await wrapper!
    .findAll("button")
    .find((item) => item.text() === "前往")!
    .trigger("click");
  await flushPromises();
  expect(invoke).toHaveBeenLastCalledWith("browse_files", {
    path: "/home/test",
  });
  // 重试成功后不再显示通知占位.
  expect(wrapper!.find('[role="alert"]').exists()).toBe(false);
});

it("文件名无效时保留页内提示, 不发送通知", async () => {
  open(false, true);
  await flushPromises();
  await wrapper!.get("#pad-file-name").setValue("../escape");
  await wrapper!
    .findAll("button")
    .find((item) => item.text() === "保存")!
    .trigger("click");
  await flushPromises();
  expect(wrapper!.get('[role="alert"]').text()).toBe("请输入有效文件名.");
  expect(notify).not.toHaveBeenCalled();
});
