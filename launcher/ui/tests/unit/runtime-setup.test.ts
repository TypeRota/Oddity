import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { bootstrap } from "./fixtures";
import type { RuntimeEnvironment } from "../../src/types/launcher";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  notify: vi.fn(),
  controller: {} as Record<string, unknown>,
}));
vi.mock("../../src/core/api", () => ({
  invoke: mocks.invoke,
  isDesktop: () => false,
}));
vi.mock("../../src/core/controller", () => mocks.controller);
vi.mock("../../src/core/notifications", async () => {
  const { errorText } = await import("../../src/core/notifications.ts");
  return { notify: mocks.notify, errorText };
});
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
const environment = (name: string): RuntimeEnvironment => ({
  id: "a".repeat(32),
  name,
  game: "nte",
  installRoot: "/game",
  proton: { path: "/proton", identity: "pin" },
  createdAt: 1,
});
// 可变的预览后端状态; 每个用例按需调整.
const setup = {
  components: false,
  installed: true,
  prefixReady: false,
  value: null as RuntimeEnvironment | null,
  systemUmu: false,
  name: "默认运行环境",
  planFails: false,
  initFails: false,
  applyInitFails: false,
  pendingName: null as string | null,
};
const planCalls: unknown[] = [];
beforeEach(() => {
  setup.components = false;
  setup.installed = true;
  setup.prefixReady = false;
  setup.value = null;
  setup.systemUmu = false;
  setup.name = "默认运行环境";
  setup.planFails = false;
  setup.initFails = false;
  setup.applyInitFails = false;
  setup.pendingName = null;
  planCalls.length = 0;
  Object.assign(mocks.controller, {
    state: reactive({ page: "runtime-setup" }),
    form: reactive({ theme: "light", reduceMotion: true }),
    formatSize: (n: number) => `${n} B`,
    runtimeEnvironmentBusy: ref(false),
    runtimeEnvironmentLocked: ref(false),
    runtimeSetupAutomatic: ref(true),
    applyBootstrap: vi.fn(),
    openSettings: vi.fn(),
    showPage: vi.fn(),
  });
  mocks.notify.mockReset();
  mocks.invoke
    .mockReset()
    .mockImplementation(
      async (command: string, args?: Record<string, unknown>) => {
        if (command === "runtime_setup_info")
          return {
            needsSetup: !setup.value,
            environment: setup.value,
            ready: !!setup.value && setup.prefixReady,
            installed: setup.installed,
            channel: "国服",
            installRoot: "/game",
            suggestedName: setup.name,
            umu: setup.components ? "/usr/bin/umu-run" : null,
            proton: setup.components ? "/proton" : null,
            protonDirectory: "/proton-directory",
            systemUmu: setup.systemUmu,
          };
        if (command === "runtime_setup_plan") {
          planCalls.push(args);
          if (setup.planFails) throw new Error("下载源暂时不可用");
          setup.pendingName =
            (args?.environmentName as string | null | undefined) ?? null;
          return { log: "/tmp/setup.log", systemCommand: null };
        }
        if (command === "runtime_setup_apply") {
          setup.components = true;
          if (setup.pendingName) {
            setup.value = environment(setup.pendingName);
            setup.prefixReady = !setup.applyInitFails;
            setup.pendingName = null;
            if (setup.applyInitFails) throw new Error("prefix 初始化失败");
          }
          return null;
        }
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
        if (command === "runtime_setup_environment") {
          if (!setup.value)
            throw new Error("请先在设置中创建并选择 Proton 运行环境");
          if (setup.initFails) throw new Error("prefix 初始化失败");
          setup.prefixReady = true;
          return bootstrap();
        }
        if (command === "bootstrap") return bootstrap();
        return null;
      },
    );
});
async function render() {
  const { default: Page } =
    await import("../../src/pages/RuntimeSetupPage.vue");
  const wrapper = mount(Page, {
    attachTo: document.body,
    global: { stubs: { teleport: true } },
  });
  await flushPromises();
  return wrapper;
}
const button = (wrapper: Awaited<ReturnType<typeof render>>, text: string) =>
  wrapper.findAll("button").find((item) => item.text() === text);

it("自动流程只准备缺失组件, 不创建运行环境", async () => {
  const wrapper = await render();
  expect(planCalls).toEqual([
    { allUsers: false, environmentName: null, proton: null },
  ]);
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_apply");
  expect(mocks.invoke).not.toHaveBeenCalledWith(
    "environment_create",
    expect.anything(),
  );
  // 组件就绪后停在确认场景, 名称默认取建议值.
  expect(wrapper.text()).toContain("创建运行环境");
  expect(wrapper.text()).toContain("国服");
  expect(wrapper.text()).toContain("/game");
  expect(
    (wrapper.get("#runtime-setup-name").element as HTMLInputElement).value,
  ).toBe("默认运行环境");
  wrapper.unmount();
});

it("组件齐备时自动流程不重复安装, 直接进入确认场景", async () => {
  setup.components = true;
  const wrapper = await render();
  expect(planCalls).toEqual([]);
  expect(mocks.invoke).not.toHaveBeenCalledWith("runtime_setup_apply");
  expect(wrapper.text()).toContain("创建运行环境");
  wrapper.unmount();
});

it("自动准备组件失败用通知报告原因并保留重试", async () => {
  setup.planFails = true;
  const wrapper = await render();
  expect(mocks.notify).toHaveBeenCalledWith(
    "启动环境配置失败: 下载源暂时不可用",
    "error",
    6500,
    { key: "runtime-setup" },
  );
  expect(wrapper.text()).toContain("配置未完成");
  expect(planCalls).toHaveLength(1);
  await button(wrapper, "重试")!.trigger("click");
  await flushPromises();
  expect(planCalls).toHaveLength(2);
  wrapper.unmount();
});

it("已有可用环境展示摘要, 继续创建先进入确认场景", async () => {
  setup.components = true;
  setup.value = environment("稳定环境");
  setup.prefixReady = true;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  expect(wrapper.text()).toContain("已有可用运行环境");
  expect(wrapper.text()).toContain("稳定环境");
  expect(planCalls).toEqual([]);
  await button(wrapper, "继续创建")!.trigger("click");
  await flushPromises();
  expect(wrapper.text()).toContain("创建运行环境");
  expect(wrapper.find("#runtime-setup-name").exists()).toBe(true);
  expect(planCalls).toEqual([]);
  wrapper.unmount();
});

it("确认场景按填写的名称创建并初始化环境", async () => {
  setup.components = true;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  await wrapper.get("#runtime-setup-name").setValue("新环境");
  await button(wrapper, "创建并初始化")!.trigger("click");
  await flushPromises();
  expect(planCalls).toEqual([
    { allUsers: false, environmentName: "新环境", proton: null },
  ]);
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_apply");
  expect(wrapper.text()).toContain("启动环境已就绪");
  wrapper.unmount();
});

it("创建环境失败时保留名称并回到确认场景", async () => {
  setup.components = true;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  mocks.invoke.mockImplementationOnce(async () => ({
    log: "",
    systemCommand: null,
  }));
  mocks.invoke.mockImplementationOnce(async () => {
    throw new Error("运行环境名称已存在");
  });
  await wrapper.get("#runtime-setup-name").setValue("重复名称");
  await button(wrapper, "创建并初始化")!.trigger("click");
  await flushPromises();
  expect(mocks.notify).toHaveBeenCalledWith(
    "启动环境配置失败: 运行环境名称已存在",
    "error",
    6500,
    { key: "runtime-setup" },
  );
  expect(wrapper.get("#runtime-setup-name").element).toHaveProperty(
    "value",
    "重复名称",
  );
  wrapper.unmount();
});

it("未安装游戏时引导前往游戏更新页且不创建环境", async () => {
  setup.installed = false;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  expect(wrapper.text()).toContain("未找到游戏安装");
  expect(wrapper.text()).toContain("请先在游戏更新页完成安装.");
  expect(wrapper.text()).not.toContain("创建并初始化");
  expect(planCalls).toEqual([]);
  await button(wrapper, "前往游戏更新页")!.trigger("click");
  expect(mocks.controller.showPage).toHaveBeenCalledWith("nte-update");
  await button(wrapper, "重新检测")!.trigger("click");
  await flushPromises();
  expect(
    mocks.invoke.mock.calls.filter(
      ([command]) => command === "runtime_setup_info",
    ),
  ).toHaveLength(2);
  wrapper.unmount();
});

it("环境 prefix 失效时只初始化当前环境", async () => {
  setup.components = true;
  setup.value = environment("待修复环境");
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  expect(wrapper.text()).toContain("初始化运行环境");
  expect(wrapper.text()).toContain("待修复环境");
  expect(wrapper.text()).toContain("/proton");
  await button(wrapper, "初始化运行环境")!.trigger("click");
  await flushPromises();
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_environment");
  expect(wrapper.text()).toContain("启动环境已就绪");
  wrapper.unmount();
});

it("初始化失败保留错误并允许改为创建新环境", async () => {
  setup.components = true;
  setup.value = environment("待修复环境");
  setup.initFails = true;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  await button(wrapper, "初始化运行环境")!.trigger("click");
  await flushPromises();
  expect(mocks.notify).toHaveBeenCalledWith(
    "启动环境配置失败: prefix 初始化失败",
    "error",
    6500,
    { key: "runtime-setup" },
  );
  expect(wrapper.text()).toContain("配置未完成");
  expect(wrapper.text()).toContain("仅移除失效环境的登记");
  await button(wrapper, "创建新的运行环境")!.trigger("click");
  await flushPromises();
  expect(wrapper.find("#runtime-setup-name").exists()).toBe(true);
  wrapper.unmount();
});

it("系统 umu 已安装时, 范围开关显示已勾选且不能取消", async () => {
  setup.components = true;
  setup.systemUmu = true;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  await button(wrapper, "安装选项")!.trigger("click");
  await flushPromises();
  const scope = wrapper.get('input[type="checkbox"]')
    .element as HTMLInputElement;
  expect(scope.checked).toBe(true);
  expect(scope.disabled).toBe(true);
  expect(wrapper.text()).toContain("系统 umu 已安装, 将直接复用.");
  wrapper.unmount();
});

it("环境已就绪但缺少启动组件时, 自动流程仍准备组件", async () => {
  setup.value = environment("现成环境");
  setup.prefixReady = true;
  const wrapper = await render();
  expect(planCalls).toEqual([
    { allUsers: false, environmentName: null, proton: null },
  ]);
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_apply");
  expect(mocks.invoke).not.toHaveBeenCalledWith(
    "environment_create",
    expect.anything(),
  );
  expect(wrapper.text()).toContain("启动环境已就绪");
  wrapper.unmount();
});

it("游戏未安装但存在旧环境时先引导完成安装", async () => {
  setup.installed = false;
  setup.value = environment("旧环境");
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  expect(wrapper.text()).toContain("未找到游戏安装");
  expect(wrapper.text()).toContain("请先在游戏更新页完成安装.");
  expect(wrapper.text()).not.toContain("初始化运行环境");
  expect(planCalls).toEqual([]);
  await button(wrapper, "前往游戏更新页")!.trigger("click");
  expect(mocks.controller.showPage).toHaveBeenCalledWith("nte-update");
  wrapper.unmount();
});

it("创建后 prefix 初始化失败时刷新到初始化场景, 不再重复创建", async () => {
  setup.components = true;
  setup.applyInitFails = true;
  mocks.controller.runtimeSetupAutomatic = ref(false);
  const wrapper = await render();
  await wrapper.get("#runtime-setup-name").setValue("失败环境");
  await button(wrapper, "创建并初始化")!.trigger("click");
  await flushPromises();
  expect(mocks.notify).toHaveBeenCalledWith(
    "启动环境配置失败: prefix 初始化失败",
    "error",
    6500,
    { key: "runtime-setup" },
  );
  expect(planCalls).toHaveLength(1);
  expect(wrapper.text()).toContain("初始化运行环境");
  expect(wrapper.text()).not.toContain("配置未完成");
  expect(wrapper.text()).toContain("当前环境的 prefix 不可用");
  await button(wrapper, "初始化运行环境")!.trigger("click");
  await flushPromises();
  expect(mocks.invoke).toHaveBeenCalledWith("runtime_setup_environment");
  expect(wrapper.text()).toContain("启动环境已就绪");
  expect(planCalls).toHaveLength(1);
  wrapper.unmount();
});
