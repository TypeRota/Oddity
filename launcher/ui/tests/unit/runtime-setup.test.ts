import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { bootstrap } from "./fixtures";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  controller: {} as Record<string, unknown>,
}));
vi.mock("../../src/core/api", () => ({
  invoke: mocks.invoke,
  isDesktop: () => false,
}));
vi.mock("../../src/core/controller", () => mocks.controller);
vi.mock("canvas-confetti", () => ({
  default: {
    create: () =>
      Object.assign(
        vi.fn(() => Promise.resolve()),
        { reset: vi.fn() },
      ),
  },
}));
vi.mock("animejs", () => ({
  createTimeline: () => ({
    add() {
      return this;
    },
    call() {
      return this;
    },
    revert() {},
    cancel() {},
  }),
  spring: () => "linear",
  stagger: () => 0,
}));
beforeEach(() => {
  Object.assign(mocks.controller, {
    state: reactive({ page: "runtime-setup" }),
    form: reactive({ theme: "light", reduceMotion: true }),
    runtimeEnvironmentBusy: ref(false),
    runtimeEnvironmentLocked: ref(false),
    runtimeSetupAutomatic: ref(true),
    applyBootstrap: vi.fn(),
    showPage: vi.fn(),
    formatSize: (n: number) => `${n} B`,
  });
  mocks.invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "runtime_setup_info")
      return { needsSetup: true, systemUmu: false };
    if (command === "runtime_setup_plan")
      return { log: "/tmp/setup.log", systemCommand: null };
    if (command === "runtime_setup_status")
      return {
        busy: false,
        done: true,
        progress: {
          component: "runtime",
          phase: "runtime",
          detail: "",
          downloaded: 0,
          total: 0,
        },
      };
    if (command === "bootstrap") return bootstrap();
    return null;
  });
});
it("首次检测缺少环境后自动开始配置, 无需点击按钮", async () => {
  const { default: Page } =
    await import("../../src/pages/RuntimeSetupPage.vue");
  const wrapper = mount(Page, { global: { stubs: { teleport: true } } });
  await flushPromises();
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_plan", {
    allUsers: false,
    createEnvironment: false,
    proton: null,
  });
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_apply");
  expect(wrapper.text()).toContain("启动环境已就绪");
  wrapper.unmount();
});
it.each(["下载源暂时不可用", "缺少 Python 3", "缺少 pkexec"])(
  "配置失败显示可用的重试: %s",
  async (message) => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "runtime_setup_info")
        return { needsSetup: true, systemUmu: false };
      if (command === "runtime_setup_plan") throw new Error(message);
      return null;
    });
    const { default: Page } =
      await import("../../src/pages/RuntimeSetupPage.vue");
    const wrapper = mount(Page, { global: { stubs: { teleport: true } } });
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain(message);
    expect(
      wrapper
        .findAll("button")
        .find((button) => button.text() === "重试")!
        .attributes("disabled"),
    ).toBeUndefined();
    expect(
      mocks.invoke.mock.calls.filter(
        ([command]) => command === "runtime_setup_plan",
      ),
    ).toHaveLength(1);
    wrapper.unmount();
  },
);

it("已有环境展示摘要, 继续创建发送独立创建请求", async () => {
  const environment = {
    id: "a".repeat(32),
    name: "稳定环境",
    game: "nte",
    distribution: "cn",
    installRoot: "/game",
    proton: { path: "/Proton", identity: "pin" },
    createdAt: 1,
  };
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const original = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command: string) =>
    command === "runtime_setup_info"
      ? {
          needsSetup: false,
          environment,
          systemUmu: false,
        }
      : original(command),
  );
  const { default: Page } =
    await import("../../src/pages/RuntimeSetupPage.vue");
  const wrapper = mount(Page, { global: { stubs: { teleport: true } } });
  await flushPromises();
  expect(wrapper.text()).toContain("已有可用运行环境");
  expect(wrapper.text()).toContain("稳定环境");
  expect(mocks.invoke).not.toHaveBeenCalledWith("runtime_setup_apply");
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "继续创建")!
    .trigger("click");
  await flushPromises();
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_plan", {
    allUsers: false,
    createEnvironment: true,
    proton: null,
  });
  wrapper.unmount();
});

it("系统 umu 已安装时, 范围开关显示已勾选且不能取消", async () => {
  mocks.controller.runtimeSetupAutomatic = ref(false);
  mocks.invoke.mockResolvedValue({
    needsSetup: false,
    environment: null,
    umu: "/usr/bin/umu-run",
    systemUmu: true,
  });
  const { default: Page } =
    await import("../../src/pages/RuntimeSetupPage.vue");
  const wrapper = mount(Page, { global: { stubs: { teleport: true } } });
  await flushPromises();
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "安装选项")!
    .trigger("click");
  await flushPromises();
  const scope = wrapper.get('input[type="checkbox"]')
    .element as HTMLInputElement;
  expect(scope.checked).toBe(true);
  expect(scope.disabled).toBe(true);
  expect(wrapper.text()).toContain("系统 umu 已安装, 将直接复用.");
  wrapper.unmount();
});
