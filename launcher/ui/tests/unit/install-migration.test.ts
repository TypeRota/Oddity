import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { bootstrap } from "./fixtures";

const controller = vi.hoisted(() => ({
  state: {} as { bootstrap: ReturnType<typeof bootstrap> },
  migration: {} as {
    plan: unknown;
    status: any;
    planning: boolean;
    error: string;
  },
  planInstallMigration: vi.fn(),
  startInstallMigration: vi.fn(),
  cancelInstallMigration: vi.fn(),
  cleanupInstallMigration: vi.fn(),
  dismissInstallMigration: vi.fn(),
  retryInstallMigration: vi.fn(),
  abandonInstallMigration: vi.fn(),
  refreshMigrationStatus: vi.fn(),
  formatSize: (bytes: number) => `${bytes} B`,
}));
vi.mock("../../src/components/BaseModal.vue", () => ({
  default: {
    props: ["open", "title", "theme", "reduceMotion"],
    template: '<div v-if="open" role="dialog"><slot /></div>',
  },
}));
vi.mock("../../src/core/controller", async () => {
  const { reactive } = await import("vue");
  controller.state = reactive({ bootstrap: bootstrap() });
  controller.migration = reactive({
    plan: null,
    status: null,
    planning: false,
    error: "",
  });
  return controller;
});

const status = (overrides: Record<string, unknown> = {}) => ({
  phase: "copying",
  distribution: "cn",
  mode: "copy",
  source: "/old",
  target: "/mnt/games",
  files: 1,
  totalFiles: 2,
  bytes: 5,
  totalBytes: 10,
  current: "game.pak",
  error: "",
  cleanupPending: false,
  cleanupSource: "",
  canCancel: true,
  ...overrides,
});

beforeEach(() => {
  Object.assign(controller.state, { bootstrap: bootstrap() });
  Object.assign(controller.migration, {
    plan: null,
    status: null,
    planning: false,
    error: "",
  });
  controller.planInstallMigration
    .mockReset()
    .mockImplementation(async (distribution, target) => {
      controller.migration.plan = {
        distribution,
        mode: "copy",
        source: "/old",
        target,
        sameFilesystem: false,
        files: 2,
        bytes: 10,
        requiredBytes: 10,
        availableBytes: 100,
        environments: [],
        preloadRoot: null,
        warnings: [],
      };
    });
  controller.startInstallMigration.mockReset().mockImplementation(async () => {
    controller.migration.status = status({
      phase: "done",
      cleanupPending: true,
      cleanupSource: "/old",
    });
  });
  controller.cleanupInstallMigration
    .mockReset()
    .mockImplementation(async () => {
      controller.migration.status = status({ phase: "done" });
    });
  controller.cancelInstallMigration.mockReset().mockImplementation(async () => {
    controller.migration.status = status({
      phase: "cancelled",
      canCancel: false,
    });
  });
  controller.retryInstallMigration.mockReset().mockImplementation(async () => {
    controller.migration.status = status({ phase: "done" });
  });
  controller.abandonInstallMigration
    .mockReset()
    .mockImplementation(async () => {
      Object.assign(controller.migration, {
        status: status({ phase: "idle" }),
        plan: null,
      });
      return true;
    });
  controller.dismissInstallMigration.mockReset().mockResolvedValue?.(undefined);
  controller.refreshMigrationStatus.mockReset().mockResolvedValue(undefined);
});

async function render() {
  const { default: Component } =
    await import("../../src/components/InstallMigration.vue");
  return mount(Component, {
    attachTo: document.body,
    props: { distribution: "cn" },
    global: {
      stubs: {
        teleport: true,
        transition: false,
        BaseModal: {
          props: ["open", "title", "theme", "reduceMotion"],
          template: '<div v-if="open" role="dialog"><slot /></div>',
        },
      },
    },
  });
}

const button = (wrapper: ReturnType<typeof mount>, label: string) =>
  wrapper.findAll("button").find((item) => item.text() === label)!;

it("检查目标, 开始迁移并清理旧目录", async () => {
  const wrapper = await render();
  await button(wrapper, "迁移国服").trigger("click");
  await flushPromises();
  await wrapper.get("#set-migration-target").setValue("/mnt/games");
  await button(wrapper, "检查目标").trigger("click");
  await flushPromises();
  expect(controller.planInstallMigration).toHaveBeenCalledWith(
    "cn",
    "/mnt/games",
  );
  expect(wrapper.text()).toContain("/mnt/games");
  await button(wrapper, "开始迁移").trigger("click");
  await flushPromises();
  expect(controller.startInstallMigration).toHaveBeenCalledWith(
    "cn",
    "/mnt/games",
  );
  expect(wrapper.text()).toContain("迁移完成");
  await button(wrapper, "删除旧目录").trigger("click");
  await flushPromises();
  expect(controller.cleanupInstallMigration).toHaveBeenCalled();
  expect(wrapper.text()).toContain("关闭");
  wrapper.unmount();
});

it("取消进行中的迁移", async () => {
  controller.startInstallMigration.mockImplementation(async () => {
    controller.migration.status = status();
  });
  const wrapper = await render();
  await button(wrapper, "迁移国服").trigger("click");
  await flushPromises();
  await wrapper.get("#set-migration-target").setValue("/mnt/games");
  await button(wrapper, "检查目标").trigger("click");
  await flushPromises();
  await button(wrapper, "开始迁移").trigger("click");
  await flushPromises();
  expect(wrapper.text()).toContain("复制中");
  await button(wrapper, "取消迁移").trigger("click");
  await flushPromises();
  expect(controller.cancelInstallMigration).toHaveBeenCalled();
  expect(wrapper.text()).toContain("已取消");
  wrapper.unmount();
});

it("修复失败时重试或放弃修复", async () => {
  Object.assign(controller.migration, {
    status: status({
      phase: "repair",
      canCancel: false,
      error: "prefix 链接无效",
    }),
  });
  const wrapper = await render();
  await button(wrapper, "迁移国服").trigger("click");
  await flushPromises();
  expect(wrapper.text()).toContain("迁移未完成");
  expect(wrapper.text()).toContain("检查 prefix 链接");
  await button(wrapper, "重试").trigger("click");
  await flushPromises();
  expect(controller.retryInstallMigration).toHaveBeenCalled();
  expect(wrapper.text()).toContain("迁移完成");
  await button(wrapper, "关闭").trigger("click");
  await flushPromises();
  expect(controller.dismissInstallMigration).toHaveBeenCalled();
  wrapper.unmount();
});

it("放弃修复失败时保留弹窗和错误", async () => {
  Object.assign(controller.migration, {
    status: status({ phase: "repair", canCancel: false }),
    error: "",
  });
  controller.abandonInstallMigration.mockImplementation(async () => {
    controller.migration.error = "删除迁移记录失败";
    return false;
  });
  const wrapper = await render();
  await button(wrapper, "迁移国服").trigger("click");
  await flushPromises();
  await button(wrapper, "放弃修复").trigger("click");
  await flushPromises();
  expect(wrapper.find("[role=dialog]").exists()).toBe(true);
  expect(wrapper.text()).toContain("删除迁移记录失败");
  wrapper.unmount();
});

it("放弃无法修复的迁移", async () => {
  Object.assign(controller.migration, {
    status: status({ phase: "repair", canCancel: false }),
  });
  const wrapper = await render();
  await button(wrapper, "迁移国服").trigger("click");
  await flushPromises();
  await button(wrapper, "放弃修复").trigger("click");
  await flushPromises();
  expect(controller.abandonInstallMigration).toHaveBeenCalled();
  expect(wrapper.find("[role=dialog]").exists()).toBe(false);
  wrapper.unmount();
});
