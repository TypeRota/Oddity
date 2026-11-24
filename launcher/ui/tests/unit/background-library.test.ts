import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const host = vi.hoisted(() => ({
  invoke: vi.fn(),
  notify: vi.fn(),
  reload: vi.fn(),
}));

vi.mock("@host/api", async () => {
  const { errorText } = await import("../../src/core/notifications.ts");
  return {
    invoke: host.invoke,
    notify: host.notify,
    errorText,
    reloadHeroBackground: host.reload,
  };
});

import BackgroundLibrary from "@packages/game-nte/ui/src/domains/settings/BackgroundLibrary.vue";

type View = {
  available: boolean;
  entries: { id: string; video: string | null; image: string | null }[];
  selected: string | null;
  current: string | null;
  migration: {
    state: "idle" | "moved" | "failed";
    moved: number;
    reason: string | null;
  };
  archive: boolean;
  dir: string;
  retention: number;
};

function view(overrides: Partial<View> = {}): View {
  return {
    available: true,
    entries: [
      { id: "first", video: null, image: "oddity://media/image?v=1" },
      { id: "second", video: null, image: null },
    ],
    selected: null,
    current: "first",
    migration: { state: "idle", moved: 0, reason: null },
    archive: true,
    dir: "",
    retention: 3,
    ...overrides,
  };
}

beforeEach(() => {
  host.invoke.mockReset();
  host.invoke.mockImplementation(() => Promise.resolve(view()));
  host.notify.mockReset();
  host.reload.mockReset();
  // BaseModal 传送到应用根节点, 测试里要先准备好.
  document.body.innerHTML = '<div id="app"></div>';
});

/// 弹窗按"从不打开到打开"加载, 直接挂载 open 不会触发.
async function open() {
  const wrapper = mount(BackgroundLibrary, {
    props: { open: false },
    // 弹窗内容默认传送到应用根节点, 测试里就地渲染.
    global: { stubs: { teleport: true } },
  });
  await wrapper.setProps({ open: true });
  await flushPromises();
  return wrapper;
}

it("列出归档素材并标出使用中的一套", async () => {
  host.invoke.mockResolvedValueOnce(view());
  const wrapper = await open();

  expect(host.invoke).toHaveBeenCalledWith("launcher_background_archive");
  const tiles = wrapper.findAll(".background-library__tile");
  expect(tiles).toHaveLength(2);
  expect(tiles[0].attributes("aria-current")).toBe("true");
  expect(tiles[0].attributes("aria-label")).toBe("第 1 套背景素材");
  expect(tiles[0].text()).toContain("使用中");
  expect(tiles[1].attributes("aria-current")).toBeUndefined();
  // 没有静态图的套显示占位, 不显示文件名.
  expect(tiles[1].find("img").exists()).toBe(false);
});

it("选择历史素材后重新解析主视觉", async () => {
  host.invoke.mockResolvedValueOnce(view());
  const wrapper = await open();

  host.invoke.mockResolvedValueOnce(view({ selected: "second" }));
  await wrapper.findAll(".background-library__tile")[1].trigger("click");
  await flushPromises();

  expect(host.invoke).toHaveBeenLastCalledWith("launcher_background_select", {
    id: "second",
  });
  expect(host.reload).toHaveBeenCalledTimes(1);
});

it("点当前素材时写入跟随当前的选择", async () => {
  host.invoke.mockResolvedValueOnce(view());
  const wrapper = await open();

  host.invoke.mockResolvedValueOnce(view());
  await wrapper.findAll(".background-library__tile")[0].trigger("click");
  await flushPromises();

  expect(host.invoke).toHaveBeenLastCalledWith("launcher_background_select", {
    id: null,
  });
});

it("改动归档设置时整份写回", async () => {
  host.invoke.mockResolvedValueOnce(view({ archive: false, entries: [] }));
  const wrapper = await open();

  host.invoke.mockResolvedValueOnce(view({ retention: 5 }));
  const toggle = wrapper.find('input[type="checkbox"]');
  await toggle.setValue(true);
  await flushPromises();

  expect(host.invoke).toHaveBeenLastCalledWith(
    "launcher_background_configure",
    { archive: true, dir: "", retention: 3 },
  );
  expect(host.reload).toHaveBeenCalledTimes(1);
});

it("索引不可用时只读展示", async () => {
  host.invoke.mockResolvedValueOnce(view({ available: false, archive: false }));
  const wrapper = await open();

  expect(wrapper.text()).toContain("归档索引来自更新版本的启动器");
  // 不可用时不能再出现"开启后会保留最近几套"的空态文案.
  expect(wrapper.text()).not.toContain("开启后会保留最近几套");
  const toggle = wrapper.find('input[type="checkbox"]');
  expect((toggle.element as HTMLInputElement).disabled).toBe(true);
});

it("迁移失败时用全局通知报告并回滚界面", async () => {
  const wrapper = await open();

  host.invoke.mockRejectedValueOnce(new Error("背景素材迁移失败"));
  const toggle = wrapper.find('input[type="checkbox"]');
  await toggle.setValue(false);
  await flushPromises();

  expect(host.notify).toHaveBeenCalledWith("背景素材迁移失败", "error", 6500, {
    key: "launcher-background",
  });
  expect(wrapper.find(".background-library__error").exists()).toBe(false);
  // 后台没有接受这次改动, 界面回到最后一次生效的状态.
  expect(
    (wrapper.find('input[type="checkbox"]').element as HTMLInputElement)
      .checked,
  ).toBe(true);
});
