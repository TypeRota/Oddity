import { beforeEach, afterEach, it, expect, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import type { AudioStatus } from "../../src/types/launcher";
const api = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn() }));
vi.mock("../../src/core/api", () => ({ invoke: api.invoke }));
vi.mock("../../src/core/controller", async () => {
  const { reactive } = await import("vue");
  return {
    form: reactive({
      audio: { device: null, volume: 100 },
      theme: "light",
      reduceMotion: true,
    }),
    queueSettingsSave: api.save,
  };
});
let wrapper: ReturnType<typeof mount> | undefined;
let status: AudioStatus;
beforeEach(() => {
  status = {
    devices: [
      { index: 1, name: "speakers", label: "音箱" },
      { index: 2, name: "headphones", label: "耳机" },
    ],
    defaultDevice: "speakers",
    streams: [],
    error: null,
  };
  api.invoke
    .mockReset()
    .mockImplementation(async () => structuredClone(status));
  api.save.mockClear();
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});
it("设备不可用时显示回退, 实际输出不一致时提示, 音量和设备修改自动保存", async () => {
  const { form } = await import("../../src/core/controller");
  const Audio = (await import("../../src/components/AudioSettings.vue"))
    .default;
  const Select = (await import("../../src/components/BaseSelect.vue")).default;
  form.audio.device = "missing";
  status.streams = [{ index: 4, pid: 3, device: 1, volume: 100, muted: false }];
  wrapper = mount(Audio, { props: { active: true } });
  await flushPromises();
  expect(wrapper.text()).toContain("所选设备未连接");
  expect(wrapper.text()).toContain("当前输出: 音箱");
  expect(wrapper.findComponent(Select).props("options")).toContainEqual({
    value: "missing",
    label: "已保存的设备 (未连接)",
  });
  wrapper.findComponent(Select).vm.$emit("update:modelValue", "headphones");
  wrapper.findComponent(Select).vm.$emit("change");
  await flushPromises();
  expect(form.audio.device).toBe("headphones");
  expect(api.save).toHaveBeenCalledWith(0);
  expect(wrapper.text()).toContain("实际输出与所选设备不同");
  await wrapper.get("#set-audio-volume").setValue("150");
  expect(form.audio.volume).toBe(150);
  expect(wrapper.text()).toContain("可能产生失真");
  const reset = wrapper
    .findAll("button")
    .find((button) => button.text() === "恢复为 100%")!;
  await reset.trigger("click");
  expect(form.audio.volume).toBe(100);
  expect(reset.attributes("disabled")).toBeDefined();
  wrapper.findComponent(Select).vm.$emit("update:modelValue", "");
  await flushPromises();
  expect(form.audio.device).toBeNull();
});
it("轮询失败后重试, 隐藏或卸载后忽略迟到响应并停止请求", async () => {
  const Audio = (await import("../../src/components/AudioSettings.vue"))
    .default;
  api.invoke.mockRejectedValueOnce(new Error("连接失败"));
  wrapper = mount(Audio, { props: { active: false } });
  expect(api.invoke).not.toHaveBeenCalled();
  await wrapper.setProps({ active: true });
  await flushPromises();
  expect(wrapper.text()).toContain("连接失败");
  await vi.advanceTimersByTimeAsync(1000);
  await flushPromises();
  expect(wrapper.text()).toContain("等待游戏播放音频");
  expect(wrapper.text()).not.toContain("连接失败");
  status.error = "系统音频不可用";
  await vi.advanceTimersByTimeAsync(1000);
  await flushPromises();
  expect(wrapper.text()).toContain("系统音频不可用");
  let resolve!: (value: AudioStatus) => void;
  api.invoke.mockImplementationOnce(
    () =>
      new Promise<AudioStatus>((done) => {
        resolve = done;
      }),
  );
  await vi.advanceTimersByTimeAsync(1000);
  await wrapper.setProps({ active: false });
  resolve({ ...status, error: "迟到错误" });
  await flushPromises();
  expect(wrapper.text()).not.toContain("迟到错误");
  const count = api.invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(2000);
  expect(api.invoke).toHaveBeenCalledTimes(count);
  await wrapper.setProps({ active: true });
  await flushPromises();
  wrapper.unmount();
  wrapper = undefined;
  const finalCount = api.invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(2000);
  expect(api.invoke).toHaveBeenCalledTimes(finalCount);
});
