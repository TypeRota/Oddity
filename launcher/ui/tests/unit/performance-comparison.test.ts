import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { bootstrap } from "./fixtures";
import type { PerformanceOverview } from "../../src/types/launcher";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  save: vi.fn(),
}));
const controller = vi.hoisted(() => ({
  state: {} as { bootstrap: ReturnType<typeof bootstrap> | null; game: string },
  runtimeEnvironmentLocked: {} as ReturnType<typeof ref<boolean>>,
  runtimeEnvironmentAction: vi.fn(),
  openSettings: vi.fn(),
}));
vi.mock("../../src/core/api", () => ({
  invoke: mock.invoke,
  isDesktop: () => true,
}));
vi.mock("../../src/core/desktop", () => ({ listen: mock.listen }));
vi.mock("../../src/core/filePicker", () => ({ save: mock.save }));
vi.mock("../../src/core/controller", () => controller);
let overview: PerformanceOverview;
let updated: () => void;
beforeEach(() => {
  controller.state = reactive({ bootstrap: bootstrap(), game: "idle" });
  controller.runtimeEnvironmentLocked = ref(false);
  const environment = (id: string) => ({
    id,
    name: `环境 ${id}`,
    proton: { path: `/proton/${id}`, identity: id },
    game: "nte" as const,
    distribution: "cn" as const,
    installRoot: "/game",
    createdAt: 1,
  });
  controller.state.bootstrap!.settings.runtimeEnvironments = [
    environment("a"),
    environment("b"),
  ];
  controller.state.bootstrap!.settings.games.nte!.stableEnvironment = "a";
  controller.state.bootstrap!.settings.games.nte!.activeEnvironment = "a";
  overview = { active: null, recording: null, comparisons: [] };
  mock.invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "performance_overview")
      return JSON.parse(JSON.stringify(overview));
    if (command === "performance_report")
      return '<svg xmlns="http://www.w3.org/2000/svg"><text>report</text></svg>';
    if (command === "performance_export") return "/tmp/report.png";
    return null;
  });
  mock.listen
    .mockReset()
    .mockImplementation(async (_event: string, callback: () => void) => {
      updated = callback;
      return vi.fn();
    });
  mock.save.mockReset().mockResolvedValue("/tmp/report.png");
  controller.runtimeEnvironmentAction.mockReset();
  controller.openSettings.mockReset();
});
async function render(plainButtons = false) {
  const { default: Component } =
    await import("../../src/components/PerformanceComparison.vue");
  const wrapper = mount(Component, {
    global: {
      stubs: {
        ...(plainButtons
          ? {
              BaseButton: {
                props: ["disabled", "busy"],
                template:
                  '<button :disabled="disabled || busy"><slot /></button>',
              },
            }
          : {}),
        BaseSelect: {
          props: ["modelValue", "options", "disabled"],
          template:
            '<select :value="modelValue" :disabled="disabled" @change="$emit(\'update:modelValue\', $event.target.value)"><option v-for="o in options" :value="o.value">{{ o.label }}</option></select>',
        },
        BaseDisclosure: { template: "<div><slot /></div>" },
      },
    },
  });
  await flushPromises();
  return wrapper;
}
function withComparison() {
  const [a, b] = controller.state.bootstrap!.settings.runtimeEnvironments;
  overview.comparisons = [
    {
      id: "comparison",
      name: "A/B",
      createdAt: 1,
      a,
      b,
      error: null,
      sessions: [],
    },
  ];
  overview.active = "comparison";
}

it("默认选择稳定 A 和另一环境, 创建后自动读取比较", async () => {
  const wrapper = await render();
  expect(wrapper.findAll("select").map((s) => s.element.value)).toEqual([
    "a",
    "b",
  ]);
  await wrapper.get('input[aria-label="比较名称"]').setValue("版本测试");
  const create = wrapper
    .findAll("button")
    .find((b) => b.text() === "创建比较")!;
  withComparison();
  await create.trigger("click");
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_create", {
    name: "版本测试",
    a: "a",
    b: "b",
  });
  expect(wrapper.text()).toContain("A: 环境 a (0 次)");
  expect(wrapper.text()).toContain("下次启动将采样 A 环境");
  wrapper.unmount();
});

it("采样完成事件自动加载图表, 支持暂停、切换环境和 PNG 导出", async () => {
  withComparison();
  const wrapper = await render();
  overview.comparisons[0].sessions.push({
    id: "run",
    side: "A",
    startedAt: 1,
    durationSeconds: 60,
    frames: 3600,
    metrics: {
      fps: 60,
      low1: 45,
      p95: 20,
      p99: 25,
      stallPercent: 0,
      vramMeanMib: null,
      rssMeanMib: null,
      gpuLoadMean: null,
      powerMeanW: null,
    },
    facts: {
      gameExecutable: "HTGame",
      gameVersion: "1.0",
      launchMode: "background",
      displayBackend: "XWayland",
      gameMode: bootstrap().settings.games.nte!.gameMode,
      vkd3dConfig: null,
      protonIdentity: "pin",
      kernel: null,
      cpu: null,
      gpu: null,
    },
    warnings: ["GPU 指标不可用"],
  });
  updated();
  await flushPromises();
  expect(wrapper.get("img").attributes("src")).toContain("data:image/svg+xml");
  const button = (label: string) =>
    wrapper.findAll("button").find((b) => b.text() === label)!;
  await button("切换到 B 环境").trigger("click");
  expect(controller.runtimeEnvironmentAction).toHaveBeenCalledWith(
    "switch",
    "b",
  );
  await button("导出 PNG").trigger("click");
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_export", {
    id: "comparison",
    destination: "/tmp/report.png",
  });
  expect(wrapper.text()).toContain("已保存: /tmp/report.png");
  await button("暂停采样").trigger("click");
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_activate", {
    id: null,
  });
  wrapper.unmount();
});

it("运行期间禁止改比较, 采样失败显示原因", async () => {
  withComparison();
  overview.comparisons[0].error = "本次采样失败: 未找到游戏帧日志";
  overview.recording = "comparison";
  controller.state.game = "running";
  const wrapper = await render();
  expect(wrapper.text()).toContain("正在采样 A 环境");
  expect(wrapper.text()).toContain("本次采样失败: 未找到游戏帧日志");
  expect(
    wrapper
      .findAll("button")
      .find((b) => b.text() === "暂停采样")!
      .attributes("disabled"),
  ).toBeDefined();
  wrapper.unmount();
});

it("手柄暂停采样和切换环境时保留当前操作及滚动位置", async () => {
  withComparison();
  const wrapper = await render(true);
  const { createGamepadNavigation, gamepad, gamepadTarget } =
    await import("../../src/core/gamepad");
  const page = document.createElement("section");
  page.className = "page is-active";
  const scroller = document.createElement("div");
  scroller.className = "settings-panel is-current";
  scroller.style.overflowY = "auto";
  page.append(scroller);
  scroller.append(wrapper.element);
  document.body.append(page);
  Object.defineProperties(scroller, {
    clientHeight: { value: 300 },
    scrollHeight: { value: 1100 },
  });
  scroller.scrollBy = ((options: ScrollToOptions) => {
    scroller.scrollTop = Math.max(
      0,
      Math.min(800, scroller.scrollTop + (options.top ?? 0)),
    );
  }) as typeof scroller.scrollBy;
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockImplementation(
    function (this: HTMLElement) {
      return [this.getBoundingClientRect()] as unknown as DOMRectList;
    },
  );
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      if (this === scroller) return new DOMRect(0, 0, 800, 300);
      const text = this.textContent?.trim();
      const y =
        this.tagName === "INPUT"
          ? 100
          : text === "管理运行环境"
            ? 0
            : text?.startsWith("切换到")
              ? 750
              : text === "刷新报告"
                ? 850
                : 650;
      return new DOMRect(400, y - scroller.scrollTop, 160, 40);
    },
  );
  gamepad.navigation = false;
  gamepad.active = true;
  const navigation = createGamepadNavigation();
  try {
    navigation.refresh();
    expect(gamepadTarget.value?.textContent).toBe("管理运行环境");
    await navigation.press(12);
    await navigation.press(12);
    const sampling = gamepadTarget.value!;
    expect(sampling.textContent).toBe("暂停采样");
    expect(scroller.scrollTop).toBeGreaterThan(0);
    const samplingScroll = scroller.scrollTop;
    let finishSampling!: () => void;
    const samplingPending = new Promise<void>((resolve) => {
      finishSampling = resolve;
    });
    mock.invoke.mockImplementation(async (command: string) => {
      if (command === "performance_activate") {
        await samplingPending;
        overview.active = null;
        return null;
      }
      if (command === "performance_overview")
        return JSON.parse(JSON.stringify(overview));
      return null;
    });
    await navigation.press(0);
    await flushPromises();
    navigation.refresh();
    expect(gamepadTarget.value).toBe(sampling);
    expect(scroller.scrollTop).toBe(samplingScroll);
    finishSampling();
    await flushPromises();
    navigation.refresh();
    expect(gamepadTarget.value).toBe(sampling);
    expect(sampling.textContent).toBe("启用此比较");
    expect(scroller.scrollTop).toBe(samplingScroll);

    await navigation.press(12);
    const switchB = gamepadTarget.value!;
    expect(switchB.textContent).toBe("切换到 B 环境");
    const switchingScroll = scroller.scrollTop;
    let finishSwitch!: () => void;
    controller.runtimeEnvironmentAction.mockImplementation(async () => {
      controller.runtimeEnvironmentLocked.value = true;
      await new Promise<void>((resolve) => {
        finishSwitch = resolve;
      });
      controller.state.bootstrap!.settings.games.nte!.activeEnvironment = "b";
      controller.runtimeEnvironmentLocked.value = false;
      return true;
    });
    await navigation.press(0);
    await flushPromises();
    navigation.refresh();
    expect(gamepadTarget.value).toBe(switchB);
    expect(scroller.scrollTop).toBe(switchingScroll);
    finishSwitch();
    await flushPromises();
    navigation.refresh();
    expect(gamepadTarget.value?.textContent).toBe("切换到 A 环境");
    expect(scroller.scrollTop).toBe(switchingScroll);
  } finally {
    gamepad.active = false;
    navigation.reset();
    navigation.dispose();
    wrapper.unmount();
    page.remove();
  }
});

it("启动信息就绪后才请求性能比较记录", async () => {
  const data = controller.state.bootstrap;
  controller.state.bootstrap = null;
  const wrapper = await render();
  expect(mock.invoke).not.toHaveBeenCalledWith("performance_overview");
  controller.state.bootstrap = data;
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_overview");
  wrapper.unmount();
});
