import { beforeEach, afterEach, it, expect, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import type { CrossScreenStatus } from "../../src/types/launcher";
import CrossScreenSettings from "../../src/components/CrossScreenSettings.vue";
const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../src/core/api", () => ({ invoke: api.invoke }));
vi.mock("../../src/core/controller", async () => {
  const { reactive } = await import("vue");
  return {
    form: reactive({ reduceMotion: true }),
    state: reactive({
      bootstrap: { settings: { activeGame: "nte" } },
      game: "idle",
    }),
  };
});
let wrapper: ReturnType<typeof mount> | undefined;
let status: CrossScreenStatus;
beforeEach(() => {
  status = {
    enabled: false,
    supported: true,
    needsRestore: false,
    missing: [],
    message: "开启后在 Moonlight 中选择游戏即可开始",
  };
  api.invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "cross_screen_configure")
      return { ...status, enabled: true, message: "已就绪" };
    return { ...status };
  });
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});
it("一个开关直接开关注册, 不需要账号输入", async () => {
  wrapper = mount(CrossScreenSettings, { props: { active: true } });
  await flushPromises();
  expect(wrapper.find("#sunshine-username").exists()).toBe(false);
  expect(wrapper.find("#sunshine-password").exists()).toBe(false);
  await wrapper.get("#set-cross-screen").setValue(true);
  await flushPromises();
  expect(api.invoke).toHaveBeenCalledWith("cross_screen_configure", {
    enabled: true,
  });
  expect(
    (wrapper.get("#set-cross-screen").element as HTMLInputElement).checked,
  ).toBe(true);
  await wrapper.get("#set-cross-screen").setValue(false);
  await flushPromises();
  expect(api.invoke).toHaveBeenLastCalledWith("cross_screen_configure", {
    enabled: false,
  });
});
it("安装组件后重新检查, 接口失败保持关闭并允许重试", async () => {
  status.missing = ["gamescope"];
  status.supported = false;
  status.message = "需要先安装 Sunshine 与 Gamescope";
  wrapper = mount(CrossScreenSettings, { props: { active: true } });
  await flushPromises();
  const install = wrapper
    .findAll("button")
    .find((button) => button.text() === "安装组件")!;
  await install.trigger("click");
  await flushPromises();
  expect(api.invoke).toHaveBeenCalledWith("cross_screen_install");
  status.missing = [];
  status.supported = true;
  await wrapper.setProps({ active: false });
  await wrapper.setProps({ active: true });
  await flushPromises();
  api.invoke.mockRejectedValueOnce(new Error("显示设置不可用"));
  await wrapper.get("#set-cross-screen").setValue(true);
  await flushPromises();
  expect(wrapper.text()).toContain("显示设置不可用");
  expect(
    (wrapper.get("#set-cross-screen").element as HTMLInputElement).checked,
  ).toBe(false);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "重新检查")!
    .trigger("click");
  await flushPromises();
  expect(wrapper.text()).not.toContain("显示设置不可用");
});
it("隐藏后忽略迟到状态, 仅在有待恢复设置时显示恢复操作", async () => {
  let resolve!: (value: CrossScreenStatus) => void;
  api.invoke.mockImplementationOnce(
    () =>
      new Promise<CrossScreenStatus>((done) => {
        resolve = done;
      }),
  );
  wrapper = mount(CrossScreenSettings, { props: { active: true } });
  await wrapper.setProps({ active: false });
  resolve({ ...status, message: "迟到状态" });
  await flushPromises();
  expect(wrapper.text()).not.toContain("迟到状态");
  status.needsRestore = true;
  await wrapper.setProps({ active: true });
  await flushPromises();
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "恢复显示设置")!
    .trigger("click");
  await flushPromises();
  expect(api.invoke).toHaveBeenCalledWith("cross_screen_restore");
});
