import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { bootstrap } from "./fixtures";
import type {
  PerformanceOverview,
  PerformanceRecord,
} from "../../src/types/launcher";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  save: vi.fn(),
}));
const controller = vi.hoisted(() => ({
  state: {} as { bootstrap: ReturnType<typeof bootstrap> | null; game: string },
  migrationBusy: {} as ReturnType<typeof ref<boolean>>,
  runtimeEnvironmentBusy: {} as ReturnType<typeof ref<boolean>>,
  openSettings: vi.fn(),
}));
vi.mock("../../src/core/api", () => ({
  invoke: mock.invoke,
  isDesktop: () => true,
}));
vi.mock("../../src/core/desktop", () => ({ listen: mock.listen }));
vi.mock("../../src/core/filePicker", () => ({ save: mock.save }));
vi.mock("../../src/core/controller", () => controller);

const start = Date.UTC(2026, 4, 12, 12);
function record(
  id: string,
  startedAt: number,
  environment: string,
  metrics: { fps: number; low1: number; p99: number },
  pinned = false,
): PerformanceRecord {
  return {
    id,
    startedAt,
    durationSeconds: 1800,
    frames: Math.round(metrics.fps * 1800),
    metrics: {
      ...metrics,
      p95: metrics.p99 - 1,
      stallPercent: 0.5,
      vramMeanMib: null,
      rssMeanMib: null,
      gpuLoadMean: null,
      powerMeanW: null,
    },
    facts: {
      gameExecutable: "HTGame",
      gameVersion: "1.4.17",
      launchMode: "background",
      displayBackend: "Wayland",
      gameMode: bootstrap().settings.games.nte!.gameMode,
      vkd3dConfig: null,
      protonIdentity: environment,
      kernel: null,
      cpu: null,
      gpu: null,
    },
    environment: {
      id: `${id}-environment`,
      name: environment,
      proton: environment,
    },
    warnings: [],
    pinned,
  };
}

let newest: PerformanceRecord;
let middle: PerformanceRecord;
let oldest: PerformanceRecord;
let overview: PerformanceOverview;

beforeEach(() => {
  controller.state = reactive({ bootstrap: bootstrap(), game: "idle" });
  controller.migrationBusy = ref(false);
  controller.runtimeEnvironmentBusy = ref(false);
  controller.openSettings.mockReset();
  newest = record("r3", start + 2 * 86_400_000, "GE-Proton11-5", {
    fps: 56,
    low1: 42,
    p99: 21,
  });
  middle = record("r2", start + 86_400_000, "Proton Experimental", {
    fps: 60,
    low1: 46,
    p99: 25,
  });
  oldest = record("r1", start, "GE-Proton11-5", {
    fps: 59,
    low1: 45,
    p99: 24,
  });
  overview = { recordingEnabled: false, records: [], recording: null };
  mock.invoke
    .mockReset()
    .mockImplementation(async (command: string, args?: unknown) => {
      if (command === "performance_overview")
        return JSON.parse(JSON.stringify(overview));
      if (command === "performance_report") {
        const { a, b } = args as { a: string; b: string };
        return `<svg xmlns="http://www.w3.org/2000/svg"><text>${a} ${b}</text></svg>`;
      }
      if (command === "performance_recording") {
        const { enabled } = args as { enabled: boolean };
        overview.recordingEnabled = enabled;
        return null;
      }
      if (command === "performance_pin") {
        const { id, pinned } = args as { id: string; pinned: boolean };
        const target = overview.records.find((item) => item.id === id);
        if (target) target.pinned = pinned;
        return null;
      }
      if (command === "performance_delete") {
        const { id } = args as { id: string };
        overview.records = overview.records.filter((item) => item.id !== id);
        return null;
      }
      if (command === "performance_export") return "/tmp/report.png";
      return null;
    });
  mock.listen.mockReset().mockResolvedValue(vi.fn());
  mock.save.mockReset().mockResolvedValue("/tmp/report.png");
});

async function render() {
  const { default: Component } =
    await import("../../src/components/PerformanceRecords.vue");
  const wrapper = mount(Component, {
    global: {
      stubs: {
        BaseSwitch: {
          props: ["modelValue", "disabled", "id"],
          emits: ["update:modelValue", "change"],
          template:
            '<input type="checkbox" :id="id" :checked="modelValue" :disabled="disabled" @change="$emit(\'update:modelValue\', $event.target.checked)" />',
        },
        BaseSelect: {
          props: ["modelValue", "options"],
          emits: ["update:modelValue"],
          template:
            '<select :value="modelValue" @change="$emit(\'update:modelValue\', $event.target.value)"><option v-for="option in options" :value="option.value">{{ option.label }}</option></select>',
        },
        BaseModal: {
          props: ["open", "title"],
          emits: ["update:open"],
          template: '<div v-if="open" role="dialog">{{ title }}<slot /></div>',
        },
      },
    },
  });
  await flushPromises();
  return wrapper;
}

function button(wrapper: Awaited<ReturnType<typeof render>>, label: string) {
  return wrapper.findAll("button").find((item) => item.text() === label)!;
}
function reportArgs() {
  return mock.invoke.mock.calls
    .filter(([command]) => command === "performance_report")
    .map(([, args]) => args);
}
function decodedReport(wrapper: Awaited<ReturnType<typeof render>>) {
  const source = wrapper.get("img").attributes("src")!;
  return decodeURIComponent(source.slice(source.indexOf(",") + 1));
}

it("开关记录每次启动, 请求完成前禁止重复切换", async () => {
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  mock.invoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === "performance_recording") {
      await pending;
      overview.recordingEnabled = (args as { enabled: boolean }).enabled;
      return null;
    }
    if (command === "performance_overview")
      return JSON.parse(JSON.stringify(overview));
    return null;
  });
  const wrapper = await render();
  const toggle = wrapper.get('input[type="checkbox"]');
  expect((toggle.element as HTMLInputElement).checked).toBe(false);
  await toggle.setValue(true);
  expect(mock.invoke).toHaveBeenCalledWith("performance_recording", {
    enabled: true,
  });
  expect((toggle.element as HTMLInputElement).checked).toBe(true);
  await flushPromises();
  expect((toggle.element as HTMLInputElement).disabled).toBe(true);
  await toggle.trigger("change");
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "performance_recording",
    ),
  ).toHaveLength(1);
  finish();
  await flushPromises();
  expect((toggle.element as HTMLInputElement).checked).toBe(true);
  expect((toggle.element as HTMLInputElement).disabled).toBe(false);
  wrapper.unmount();
});

it("开关请求失败时回到存储里的状态", async () => {
  mock.invoke.mockImplementation(async (command: string) => {
    if (command === "performance_recording")
      throw new Error("无法写入性能记录索引");
    if (command === "performance_overview")
      return JSON.parse(JSON.stringify(overview));
    return null;
  });
  const wrapper = await render();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_recording", {
    enabled: true,
  });
  expect(wrapper.get('input[type="checkbox"]').element).toHaveProperty(
    "checked",
    false,
  );
  expect(wrapper.text()).toContain("无法写入性能记录索引");
  wrapper.unmount();
});

it("默认对比最近两条记录, 摘要给出两侧指标与差值", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  expect(wrapper.findAll("select").map((item) => item.element.value)).toEqual([
    "r2",
    "r3",
  ]);
  expect(reportArgs()).toEqual([{ a: "r2", b: "r3" }]);
  expect(decodedReport(wrapper)).toContain("r2 r3");
  expect(wrapper.text()).toContain("Proton Experimental");
  expect(wrapper.text()).toContain("平均 FPS");
  expect(wrapper.text()).toContain("1% Low");
  expect(wrapper.text()).toContain("P99 帧时间");
  expect(wrapper.text()).toContain("30 分钟");
  const delta = (value: string) =>
    wrapper.findAll("td").find((cell) => cell.text() === value)!;
  expect(delta("-6.7%").classes()).toContain("is-worse");
  expect(delta("-8.7%").classes()).toContain("is-worse");
  expect(delta("-16.0%").classes()).toContain("is-better");
  wrapper.unmount();
});

it("记录不足两条时提示需要再完成一次游戏", async () => {
  overview.records = [newest];
  const wrapper = await render();
  expect(wrapper.text()).toContain("再完成一次游戏后可比较");
  expect(wrapper.findAll("select")).toHaveLength(0);
  expect(reportArgs()).toEqual([]);
  wrapper.unmount();
});

it("切换基准与对比记录后按新的组合拉取报告", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  await wrapper.findAll("select")[1].setValue("r1");
  await flushPromises();
  expect(reportArgs().at(-1)).toEqual({ a: "r2", b: "r1" });
  expect(decodedReport(wrapper)).toContain("r2 r1");
  await wrapper.findAll("select")[0].setValue("r3");
  await flushPromises();
  expect(reportArgs().at(-1)).toEqual({ a: "r3", b: "r1" });
  await button(wrapper, "刷新报告").trigger("click");
  await flushPromises();
  expect(reportArgs().at(-1)).toEqual({ a: "r3", b: "r1" });
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "performance_report",
    ),
  ).toHaveLength(4);
  wrapper.unmount();
});

it("两侧选择相同时禁用图表与导出", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  await wrapper.findAll("select")[1].setValue("r2");
  await flushPromises();
  expect(wrapper.text()).toContain("请选择两条不同的记录");
  expect(wrapper.find("img").exists()).toBe(false);
  expect(button(wrapper, "导出 PNG").attributes("disabled")).toBeDefined();
  expect(button(wrapper, "刷新报告").attributes("disabled")).toBeDefined();
  wrapper.unmount();
});

it("固定和删除记录调用对应命令, 删除需要二次确认", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  await button(wrapper, "固定").trigger("click");
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_pin", {
    id: "r3",
    pinned: true,
  });
  expect(wrapper.text()).toContain("已固定");
  expect(button(wrapper, "取消固定")).toBeDefined();

  const deletes = () =>
    mock.invoke.mock.calls.filter(
      ([command]) => command === "performance_delete",
    );
  const rows = wrapper.findAll(".performance-records__record");
  await rows[1].findAll("button")[1].trigger("click");
  await flushPromises();
  expect(deletes()).toEqual([]);
  expect(wrapper.get('[role="dialog"]').text()).toContain("删除性能记录");
  await button(wrapper, "确定删除").trigger("click");
  await flushPromises();
  expect(deletes()).toEqual([["performance_delete", { id: "r2" }]]);
  expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  expect(wrapper.findAll(".performance-records__record")).toHaveLength(2);
  expect(wrapper.text()).not.toContain("Proton Experimental");
  wrapper.unmount();
});

it("删除记录后只重置失效的那一侧", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  // 用户把对比侧换成最早的一条.
  await wrapper.findAll("select")[1].setValue("r1");
  await flushPromises();
  expect(reportArgs().at(-1)).toEqual({ a: "r2", b: "r1" });

  // 删除基准侧引用的记录, 对比侧保持用户的选择.
  await wrapper
    .findAll(".performance-records__record")[1]
    .findAll("button")[1]
    .trigger("click");
  await flushPromises();
  await button(wrapper, "确定删除").trigger("click");
  await flushPromises();
  expect(wrapper.findAll("select").map((item) => item.element.value)).toEqual([
    "r3",
    "r1",
  ]);
  wrapper.unmount();
});

it("记录卡片与固定/删除按钮带上环境名和时间的可访问名称", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  const rows = wrapper.findAll(".performance-records__record");
  const labels = rows.map((row) => row.attributes("aria-label"));
  expect(new Set(labels).size).toBe(3);
  expect(labels[0]).toContain("GE-Proton11-5");
  const names = rows.map((row) =>
    row.findAll("button")[1].attributes("aria-label"),
  );
  expect(new Set(names).size).toBe(3);
  expect(names[1]).toMatch(/^删除 Proton Experimental /);
  wrapper.unmount();
});

it("游戏运行期间禁止固定和删除", async () => {
  overview.records = [newest, middle, oldest];
  controller.state.game = "running";
  const wrapper = await render();
  const rows = wrapper.findAll(".performance-records__record");
  for (const item of rows) {
    const [pin, remove] = item.findAll("button");
    expect(pin.attributes("disabled")).toBeDefined();
    expect(remove.attributes("disabled")).toBeDefined();
  }
  wrapper.unmount();
});

it("运行环境任务进行期间禁止固定和删除", async () => {
  overview.records = [newest, middle, oldest];
  controller.runtimeEnvironmentBusy.value = true;
  const wrapper = await render();
  for (const item of wrapper.findAll(".performance-records__record")) {
    const [pin, remove] = item.findAll("button");
    expect(pin.attributes("disabled")).toBeDefined();
    expect(remove.attributes("disabled")).toBeDefined();
  }
  wrapper.unmount();
});

it("迁移进行期间禁止固定和删除", async () => {
  overview.records = [newest, middle, oldest];
  controller.migrationBusy.value = true;
  const wrapper = await render();
  for (const item of wrapper.findAll(".performance-records__record")) {
    const [pin, remove] = item.findAll("button");
    expect(pin.attributes("disabled")).toBeDefined();
    expect(remove.attributes("disabled")).toBeDefined();
  }
  wrapper.unmount();
});

it("游戏运行期间仍可切换记录开关", async () => {
  controller.state.game = "running";
  controller.runtimeEnvironmentBusy.value = true;
  const wrapper = await render();
  const toggle = wrapper.get('input[type="checkbox"]');
  expect((toggle.element as HTMLInputElement).disabled).toBe(false);
  await toggle.setValue(true);
  expect(mock.invoke).toHaveBeenCalledWith("performance_recording", {
    enabled: true,
  });
  wrapper.unmount();
});

it("采样中的记录给出状态提示", async () => {
  overview.records = [newest, middle];
  overview.recording = "r3";
  const wrapper = await render();
  expect(wrapper.text()).toContain(
    "正在记录本次启动, 退出游戏后, 采样有效的记录会出现在列表里",
  );
  wrapper.unmount();
});

it("空列表提示完成一次游戏后产生第一条记录", async () => {
  const wrapper = await render();
  expect(wrapper.text()).toContain("开启记录后完成一次游戏即可产生第一条记录");
  expect(wrapper.text()).not.toContain("再完成一次游戏后可比较");
  wrapper.unmount();
});

it("导出 PNG 传入当前对比的两条记录", async () => {
  overview.records = [newest, middle, oldest];
  const wrapper = await render();
  await button(wrapper, "导出 PNG").trigger("click");
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_export", {
    a: "r2",
    b: "r3",
    destination: "/tmp/report.png",
  });
  expect(wrapper.text()).toContain("已保存: /tmp/report.png");
  wrapper.unmount();
});

it("启动信息就绪后才请求性能记录", async () => {
  const data = controller.state.bootstrap;
  controller.state.bootstrap = null;
  const wrapper = await render();
  expect(mock.invoke).not.toHaveBeenCalledWith("performance_overview");
  controller.state.bootstrap = data;
  await flushPromises();
  expect(mock.invoke).toHaveBeenCalledWith("performance_overview");
  wrapper.unmount();
});
