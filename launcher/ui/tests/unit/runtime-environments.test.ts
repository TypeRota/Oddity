import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { bootstrap } from "./fixtures";
import type { EnvironmentAudit } from "../../src/types/launcher";

const controller = vi.hoisted(() => ({
  state: {} as { bootstrap: ReturnType<typeof bootstrap> },
  form: { theme: "light", reduceMotion: true },
  runtimeEnvironmentBusy: {} as ReturnType<typeof ref<boolean>>,
  runtimeEnvironmentLocked: {} as ReturnType<typeof ref<boolean>>,
  formatSize: (bytes: number) =>
    bytes >= 1024 ? `${(bytes / 1024).toFixed(1)} KiB` : `${bytes} B`,
  createRuntimeEnvironment: vi.fn(),
  runtimeEnvironmentAction: vi.fn(),
  renameRuntimeEnvironment: vi.fn(),
  loadEnvironmentAudit: vi.fn(),
  importRuntimeEnvironment: vi.fn(),
  deleteEnvironmentOrphan: vi.fn(),
}));
vi.mock("../../src/core/controller", () => controller);
const a = "a".repeat(32),
  b = "b".repeat(32);
const audit = (): EnvironmentAudit => ({
  environments: [
    {
      id: a,
      name: "稳定 A",
      state: "ready",
      installRoot: "/game",
      installRootCurrent: "/game",
    },
    {
      id: b,
      name: "实验 B",
      state: "ready",
      installRoot: "/game",
      installRootCurrent: "/game",
    },
  ],
  orphans: [],
});
beforeEach(() => {
  controller.state = reactive({ bootstrap: bootstrap() });
  controller.state.bootstrap.settings.runtimeEnvironments = [
    {
      id: a,
      name: "稳定 A",
      proton: { path: "/proton-a", identity: "pin" },
      game: "nte",
      installRoot: "/game",
      createdAt: 1,
    },
    {
      id: b,
      name: "实验 B",
      proton: { path: "/proton-b", identity: "pin-b" },
      game: "nte",
      installRoot: "/game",
      createdAt: 2,
    },
  ];
  controller.state.bootstrap.settings.games.nte!.activeEnvironments = {
    cn: b,
  };
  controller.state.bootstrap.settings.games.nte!.stableEnvironments = {
    cn: a,
  };
  controller.runtimeEnvironmentBusy = ref(false);
  controller.runtimeEnvironmentLocked = ref(false);
  controller.createRuntimeEnvironment.mockReset().mockResolvedValue(true);
  controller.runtimeEnvironmentAction.mockReset().mockResolvedValue(true);
  controller.renameRuntimeEnvironment.mockReset().mockResolvedValue(true);
  controller.loadEnvironmentAudit.mockReset().mockResolvedValue(audit());
  controller.importRuntimeEnvironment.mockReset().mockResolvedValue(true);
  controller.deleteEnvironmentOrphan.mockReset().mockResolvedValue(true);
});
/** 弹窗内容被 Teleport 到 #app, 读数与点击都直接走 document. */
const field = (id: string) =>
  document.getElementById(id) as HTMLInputElement | null;
async function setField(id: string, value: string) {
  const input = field(id)!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await flushPromises();
}
const dialog = () =>
  document.querySelector<HTMLElement>('[role="dialog"]') ?? null;
async function clickButton(scope: ParentNode, name: string) {
  Array.from(scope.querySelectorAll("button"))
    .find((button) => button.textContent?.trim() === name)!
    .click();
  await flushPromises();
}
async function chooseSelect(triggerId: string, label: string) {
  document.getElementById(triggerId)!.click();
  await flushPromises();
  Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
    .find((option) => option.textContent?.trim() === label)!
    .click();
  await flushPromises();
}
async function render() {
  // 弹窗通过 Teleport 挂到 #app; 测试提供真实容器以便检查弹窗内容.
  const app = document.createElement("div");
  app.id = "app";
  document.body.append(app);
  const { default: Component } =
    await import("../../src/components/RuntimeEnvironments.vue");
  const wrapper = mount(Component, {
    attachTo: document.body,
    global: { stubs: { transition: false } },
  });
  await flushPromises();
  return wrapper;
}
it("首次使用时创建全新运行环境", async () => {
  controller.state.bootstrap.settings.runtimeEnvironments = [];
  controller.state.bootstrap.settings.games.nte!.activeEnvironments = {};
  const wrapper = await render();
  expect(wrapper.text()).toContain("请先创建 Proton 运行环境");
  expect(wrapper.find("#runtime-environment").exists()).toBe(false);
  await wrapper.get("button").trigger("click");
  await setField("runtime-environment-name", "首个环境");
  await setField("runtime-environment-proton", "/proton-a");
  await clickButton(document, "创建环境");
  expect(controller.createRuntimeEnvironment).toHaveBeenCalledWith(
    "首个环境",
    "/proton-a",
    null,
  );
  await flushPromises();
  expect(field("runtime-environment-name")).toBeNull();
  expect(controller.loadEnvironmentAudit).toHaveBeenCalledTimes(2);
  wrapper.unmount();
});

it("使用 B 时选中稳定 A, 副本请求使用 A 且不切换当前环境", async () => {
  const wrapper = await render();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", a);
  await flushPromises();
  const button = (name: string) =>
    wrapper.findAll("button").find((button) => button.text() === name)!;
  await button("创建环境或副本").trigger("click");
  expect(document.body.textContent).toContain("源环境: 稳定 A");
  expect(field("runtime-environment-proton")!.value).toBe("/proton-a");
  await setField("runtime-environment-name", "A 副本");
  controller.createRuntimeEnvironment.mockResolvedValueOnce(false);
  await clickButton(dialog()!, "创建副本");
  expect(controller.createRuntimeEnvironment).toHaveBeenCalledWith(
    "A 副本",
    "/proton-a",
    a,
  );
  expect(
    controller.state.bootstrap.settings.games.nte!.activeEnvironments.cn,
  ).toBe(b);
  expect(field("runtime-environment-name")).not.toBeNull();
  await chooseSelect("runtime-environment-source-trigger", "全新环境");
  expect(document.body.textContent).toContain("创建独立 prefix");
  expect(field("runtime-environment-proton")!.value).toBe("");
  await clickButton(dialog()!, "取消创建");
  wrapper.unmount();
});

it("切换和保留指定环境, 当前和稳定环境不可删除", async () => {
  const wrapper = await render();
  const button = (name: string) =>
    wrapper.findAll("button").find((button) => button.text() === name)!;
  expect(button("删除环境").attributes("disabled")).toBeDefined();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", a);
  await flushPromises();
  await button("切换到此环境").trigger("click");
  expect(controller.runtimeEnvironmentAction).toHaveBeenCalledWith("switch", a);
  expect(button("删除环境").attributes("disabled")).toBeDefined();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", b);
  await flushPromises();
  await button("保留为稳定环境").trigger("click");
  expect(controller.runtimeEnvironmentAction).toHaveBeenCalledWith("keep", b);
  wrapper.unmount();
});

it("删除必须在弹窗中确认, 失败保留弹窗并允许取消和重试", async () => {
  controller.state.bootstrap.settings.games.nte!.activeEnvironments = {
    cn: a,
  };
  const wrapper = await render();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", b);
  await flushPromises();
  const remove = wrapper
    .findAll("button")
    .find((button) => button.text() === "删除环境")!;
  await remove.trigger("click");
  await flushPromises();
  const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')!;
  const click = async (name: string) => {
    Array.from(dialog().querySelectorAll("button"))
      .find((button) => button.textContent?.trim() === name)!
      .click();
    await flushPromises();
  };
  expect(dialog().textContent).toContain("确定删除“实验 B”的 prefix?");
  expect(controller.runtimeEnvironmentAction).not.toHaveBeenCalled();
  await click("取消");
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  await remove.trigger("click");
  await flushPromises();
  // 命令失败由全局通知报告, 弹窗保留供再试, 页内不再重复一行错误.
  controller.runtimeEnvironmentAction.mockResolvedValueOnce(false);
  await click("确定删除");
  expect(dialog()).not.toBeNull();
  expect(dialog().textContent).not.toContain("磁盘写入失败");
  await click("确定删除");
  expect(controller.runtimeEnvironmentAction).toHaveBeenCalledWith(
    "discard",
    b,
  );
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  wrapper.unmount();
});

it("失效环境禁用切换和保留, 删除改为仅移除登记", async () => {
  controller.loadEnvironmentAudit.mockResolvedValue({
    environments: [
      ...audit().environments.slice(0, 1),
      {
        id: b,
        name: "实验 B",
        state: "identity-invalid",
        installRoot: "/game",
        installRootCurrent: "/game",
      },
    ],
    orphans: [],
  });
  const wrapper = await render();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", b);
  await flushPromises();
  const button = (name: string) =>
    wrapper.findAll("button").find((button) => button.text() === name)!;
  expect(wrapper.text()).toContain("环境目录不可用, 无法启动.");
  expect(wrapper.text()).toContain("不可用");
  expect(button("切换到此环境").attributes("disabled")).toBeDefined();
  expect(button("保留为稳定环境").attributes("disabled")).toBeDefined();
  await button("仅移除登记").trigger("click");
  await flushPromises();
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
  expect(dialog.textContent).toContain("移除运行环境登记");
  expect(dialog.textContent).toContain("磁盘目录保留");
  Array.from(dialog.querySelectorAll("button"))
    .find((item) => item.textContent?.trim() === "移除登记")!
    .click();
  await flushPromises();
  expect(controller.runtimeEnvironmentAction).toHaveBeenCalledWith("forget", b);
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  wrapper.unmount();
});

it("固定 Proton 失效同样可仅移除登记", async () => {
  controller.loadEnvironmentAudit.mockResolvedValue({
    environments: [
      {
        id: b,
        name: "实验 B",
        state: "proton-invalid",
        installRoot: "/game",
        installRootCurrent: "/game",
      },
    ],
    orphans: [],
  });
  const wrapper = await render();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", b);
  await flushPromises();
  expect(wrapper.text()).toContain("固定的 Proton 已变化, 请创建新环境.");
  expect(
    wrapper.findAll("button").find((button) => button.text() === "仅移除登记"),
  ).toBeTruthy();
  wrapper.unmount();
});

it("安装目录变化时提供更新绑定入口", async () => {
  controller.loadEnvironmentAudit.mockResolvedValue({
    environments: [
      audit().environments[0],
      {
        id: b,
        name: "实验 B",
        state: "ready",
        installRoot: "/old-game",
        installRootCurrent: "/game",
      },
    ],
    orphans: [],
  });
  const wrapper = await render();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", b);
  await flushPromises();
  expect(wrapper.text()).toContain("上次使用的游戏目录与当前安装目录不同.");
  const rebind = wrapper
    .findAll("button")
    .find((button) => button.text() === "更新绑定")!;
  expect(rebind).toBeTruthy();
  await rebind.trigger("click");
  expect(controller.runtimeEnvironmentAction).toHaveBeenCalledWith("rebind", b);
  wrapper.unmount();
});

it("处理期间禁用操作且不在页内重复后端错误", async () => {
  controller.runtimeEnvironmentBusy.value = true;
  controller.runtimeEnvironmentLocked.value = true;
  const wrapper = await render();
  expect(wrapper.get('[role="status"]').text()).toContain("正在处理运行环境");
  expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  expect(
    wrapper
      .findAll(".control-button")
      .every((button) => button.attributes("disabled") !== undefined),
  ).toBe(true);
  wrapper.unmount();
});

it("重命名当前环境并保留选择, 失败时留在弹窗", async () => {
  const wrapper = await render();
  wrapper
    .findComponent({ name: "BaseSelect" })
    .vm.$emit("update:modelValue", b);
  await flushPromises();
  const button = (name: string) =>
    wrapper.findAll("button").find((button) => button.text() === name)!;
  const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')!;
  const save = () =>
    Array.from(dialog().querySelectorAll("button")).find(
      (button) => button.textContent?.trim() === "保存名称",
    )!;
  await button("重命名").trigger("click");
  await flushPromises();
  expect(dialog().textContent).toContain("重命名运行环境");
  expect(field("runtime-environment-rename")!.value).toBe("实验 B");
  await setField("runtime-environment-rename", "  ");
  expect(save().disabled).toBe(true);
  await setField("runtime-environment-rename", "实验 B 改名");
  expect(save().disabled).toBe(false);
  save().click();
  await flushPromises();
  expect(controller.renameRuntimeEnvironment).toHaveBeenCalledWith(
    "实验 B 改名",
    b,
  );
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  await button("重命名").trigger("click");
  await flushPromises();
  // 重名由后端拒绝, 失败原因走全局通知, 弹窗保持打开供改名重试.
  controller.renameRuntimeEnvironment.mockResolvedValueOnce(false);
  await setField("runtime-environment-rename", "稳定 A");
  save().click();
  await flushPromises();
  expect(dialog()).not.toBeNull();
  expect(dialog().textContent).not.toContain("运行环境名称已存在");
  wrapper.unmount();
});

it("未登记数据列出类型与体积, 可导入或删除", async () => {
  const orphanId = "c".repeat(32);
  controller.loadEnvironmentAudit.mockResolvedValue({
    environments: audit().environments,
    orphans: [
      {
        kind: "environment",
        name: orphanId,
        id: orphanId,
        bytes: 2048,
        modifiedAt: 1_700_000_000,
        identityValid: true,
        prefixValid: true,
      },
      {
        kind: "logs",
        name: "d".repeat(32),
        id: "d".repeat(32),
        bytes: 512,
        modifiedAt: 1_700_000_000,
        identityValid: false,
        prefixValid: false,
      },
      {
        kind: "pending",
        name: `.creating-${"e".repeat(32)}`,
        id: "e".repeat(32),
        bytes: 0,
        modifiedAt: 1_700_000_000,
        identityValid: false,
        prefixValid: false,
      },
    ],
  });
  const wrapper = await render();
  expect(wrapper.text()).toContain("环境数据维护");
  expect(wrapper.text()).toContain("环境目录 · cccccccc");
  expect(wrapper.text()).toContain("日志目录");
  expect(wrapper.text()).toContain("未完成操作");
  expect(wrapper.text()).toContain("2.0 KiB");
  const button = (wrapper: Awaited<ReturnType<typeof render>>, name: string) =>
    wrapper.findAll("button").find((button) => button.text() === name);
  await button(wrapper, "导入")!.trigger("click");
  await flushPromises();
  expect(dialog()!.textContent).toContain("导入运行环境");
  expect(dialog()!.textContent).toContain("绑定当前区服的安装目录");
  expect(field("runtime-environment-import-name")!.value).toBe("导入的环境");
  await setField("runtime-environment-import-name", "恢复的环境");
  await setField("runtime-environment-import-proton", "/proton-import");
  Array.from(dialog()!.querySelectorAll("button"))
    .find((item) => item.textContent?.trim() === "导入环境")!
    .click();
  await flushPromises();
  expect(controller.importRuntimeEnvironment).toHaveBeenCalledWith(
    orphanId,
    "恢复的环境",
    "/proton-import",
  );
  // 日志目录只提供删除, 二次确认后调用清理命令.
  const logRow = wrapper
    .findAll(".runtime-orphan")
    .find((row) => row.text().includes("日志目录"))!;
  await logRow
    .findAll("button")
    .find((item) => item.text() === "删除")!
    .trigger("click");
  await flushPromises();
  expect(dialog()!.textContent).toContain("确定删除未登记的日志目录");
  Array.from(dialog()!.querySelectorAll("button"))
    .find((item) => item.textContent?.trim() === "确定删除")!
    .click();
  await flushPromises();
  expect(controller.deleteEnvironmentOrphan).toHaveBeenCalledWith(
    "logs",
    "d".repeat(32),
  );
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  wrapper.unmount();
});

it("没有未登记数据时给出空列表提示", async () => {
  const wrapper = await render();
  expect(wrapper.text()).toContain("未发现未登记的环境目录或日志.");
  wrapper.unmount();
});

it("审计失败显示错误与重新检测, 重试成功后恢复列表", async () => {
  const orphanId = "c".repeat(32);
  controller.loadEnvironmentAudit
    .mockResolvedValueOnce(null)
    .mockResolvedValueOnce({
      environments: audit().environments,
      orphans: [
        {
          kind: "environment",
          name: orphanId,
          id: orphanId,
          bytes: 2048,
          modifiedAt: 1_700_000_000,
          identityValid: true,
          prefixValid: true,
        },
      ],
    });
  const wrapper = await render();
  expect(wrapper.text()).toContain("读取环境状态失败");
  expect(wrapper.text()).not.toContain("正在检查未登记的环境与日志.");
  const retry = wrapper
    .findAll("button")
    .find((item) => item.text() === "重新检测")!;
  await retry.trigger("click");
  await flushPromises();
  expect(controller.loadEnvironmentAudit).toHaveBeenCalledTimes(2);
  expect(wrapper.text()).not.toContain("读取环境状态失败");
  expect(wrapper.text()).toContain("环境目录 · cccccccc");
  // 再次刷新失败时保留已加载的列表, 只提示状态错误.
  controller.loadEnvironmentAudit.mockResolvedValueOnce(null);
  controller.state.bootstrap = bootstrap();
  await flushPromises();
  expect(wrapper.text()).toContain("读取环境状态失败");
  expect(wrapper.text()).toContain("环境目录 · cccccccc");
  wrapper.unmount();
});
