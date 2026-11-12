import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";

const host = vi.hoisted(() => ({
  invoke: vi.fn(),
  openUrl: vi.fn(),
  notify: vi.fn(),
  state: {
    game: "nte",
    distribution: "cn" as "cn" | "global",
    ready: true,
    page: "home",
    stackBulletins: false,
  },
}));
vi.mock("@host/api", async () => {
  const { computed, reactive } = await import("vue");
  const state = reactive(host.state);
  host.state = state;
  return {
    invoke: host.invoke,
    openUrl: host.openUrl,
    notify: host.notify,
    hostFacts: {
      game: computed(() => state.game),
      distribution: computed(() => state.distribution),
      ready: computed(() => state.ready),
      page: computed(() => state.page),
      stackBulletins: computed(() => state.stackBulletins),
    },
  };
});

import { BulletinPage } from "@packages/game-nte/ui";

type BulletinItem = {
  kind: "bilibili" | "x" | "preview" | "maintenance";
  title: string;
  summary: string;
  url: string;
  imageUrl: string | null;
  publishedAt: string;
  source: string;
};
type BulletinFeed = { items: BulletinItem[]; errors: string[] };

function item(overrides: Partial<BulletinItem> = {}): BulletinItem {
  return {
    kind: "bilibili",
    title: "官方动态",
    summary: "摘要",
    url: "https://example.com/bulletin",
    imageUrl: null,
    publishedAt: "",
    source: "B站官方动态",
    ...overrides,
  };
}

const feed: BulletinFeed = {
  items: [
    item({ title: "第一条", url: "https://example.com/1" }),
    item({
      kind: "x",
      title: "X 动态",
      url: "https://example.com/2",
      source: "X 官方",
    }),
    item({ kind: "preview", title: "版本前瞻", url: "https://example.com/3" }),
    item({
      kind: "maintenance",
      title: "维护公告",
      url: "https://example.com/4",
      source: "官方公告",
    }),
  ],
  errors: [],
};

const cacheKey = (distribution: "cn" | "global") =>
  `nte.bulletins.v3.${distribution}`;

let wrapper: VueWrapper | undefined;

async function render() {
  wrapper = mount(BulletinPage);
  await nextTick();
  return wrapper;
}

beforeEach(() => {
  host.invoke.mockReset().mockResolvedValue(structuredClone(feed));
  host.openUrl.mockReset();
  host.notify.mockReset();
  Object.assign(host.state, {
    game: "nte",
    distribution: "cn",
    ready: true,
    page: "home",
    stackBulletins: false,
  });
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  localStorage.clear();
});

it("加载时显示骨架, 完成后渲染公告并写入当前区服缓存", async () => {
  let resolve!: (value: BulletinFeed) => void;
  host.invoke.mockImplementation(
    () =>
      new Promise<BulletinFeed>((done) => {
        resolve = done;
      }),
  );
  await render();
  expect(wrapper!.get("#page-nte-bulletin").classes()).toContain(
    "bulletin-page",
  );
  expect(wrapper!.find(".bulletin-skeleton").exists()).toBe(true);
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(0);
  expect(host.invoke).toHaveBeenCalledWith("fetch_bulletins", {
    distribution: "cn",
  });

  resolve(structuredClone(feed));
  await flushPromises();
  expect(wrapper!.find(".bulletin-skeleton").exists()).toBe(false);
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(4);
  expect(localStorage.getItem(cacheKey("cn"))).toContain('"errors":[]');
});

it("启动数据就绪前不读取公告", async () => {
  host.state.ready = false;
  await render();
  await flushPromises();
  expect(host.invoke).not.toHaveBeenCalled();

  host.state.ready = true;
  await nextTick();
  await flushPromises();
  expect(host.invoke).toHaveBeenCalledTimes(1);
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(4);
});

it("页面 id 与显隐契约随宿主页面事实变化", async () => {
  host.state.page = "nte-bulletin";
  await render();
  await flushPromises();
  const panel = wrapper!.get("#page-nte-bulletin");
  expect(panel.classes()).toEqual(
    expect.arrayContaining(["page", "bulletin-page", "is-active"]),
  );
  expect(panel.attributes("inert")).toBe("false");
  expect(panel.attributes("aria-hidden")).toBeUndefined();

  host.state.page = "home";
  await nextTick();
  expect(panel.attributes("inert")).toBeDefined();
  expect(panel.attributes("aria-hidden")).toBe("true");
});

it("TTL 内的缓存直接复用, 过期后重新请求", async () => {
  vi.setSystemTime(new Date(2026, 10, 10, 10));
  localStorage.setItem(
    cacheKey("cn"),
    JSON.stringify({ savedAt: Date.now(), feed: structuredClone(feed) }),
  );
  await render();
  await flushPromises();
  expect(host.invoke).not.toHaveBeenCalled();
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(4);

  wrapper!.unmount();
  localStorage.setItem(
    cacheKey("cn"),
    JSON.stringify({
      savedAt: Date.now() - 16 * 60 * 1000,
      feed: structuredClone(feed),
    }),
  );
  await render();
  await flushPromises();
  expect(host.invoke).toHaveBeenCalledTimes(1);
});

it("区服切换丢弃旧区服的在途请求且不串缓存", async () => {
  let finishCn!: (value: BulletinFeed) => void;
  const globalFeed: BulletinFeed = {
    items: [item({ title: "国际服新闻", source: "国际服官网" })],
    errors: [],
  };
  host.invoke.mockImplementation(
    (command: string, args: { distribution: string }) => {
      if (command !== "fetch_bulletins") return Promise.resolve(null);
      return args.distribution === "cn"
        ? new Promise<BulletinFeed>((resolve) => {
            finishCn = resolve;
          })
        : Promise.resolve(structuredClone(globalFeed));
    },
  );
  await render();
  expect(host.invoke).toHaveBeenLastCalledWith("fetch_bulletins", {
    distribution: "cn",
  });

  host.state.distribution = "global";
  await nextTick();
  await flushPromises();
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(1);
  expect(wrapper!.get(".bulletin-entry").text()).toContain("国际服新闻");

  finishCn({
    items: [item({ title: "旧区服公告" })],
    errors: [],
  });
  await flushPromises();
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(1);
  expect(wrapper!.get(".bulletin-entry").text()).toContain("国际服新闻");
  expect(localStorage.getItem(cacheKey("cn"))).toBeNull();
  expect(localStorage.getItem(cacheKey("global"))).toContain("国际服新闻");
});

it("刷新失败时提示并可重试", async () => {
  await render();
  await flushPromises();
  host.invoke.mockResolvedValueOnce({
    items: [],
    errors: ["B站动态: 暂时不可用"],
  });
  await wrapper!.get(".page-head .control-button").trigger("click");
  await flushPromises();
  expect(host.notify).toHaveBeenCalledWith(
    "公告暂时无法更新，请稍后重试。",
    "error",
    6000,
    expect.objectContaining({ key: "bulletins" }),
  );
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(0);
  expect(wrapper!.get(".bulletin-source-error").text()).toContain(
    "B站动态: 暂时不可用",
  );

  const retry = host.notify.mock.calls[0][3].primary.run;
  retry();
  await flushPromises();
  expect(host.invoke).toHaveBeenCalledTimes(3);
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(4);
});

it("首次读取失败显示来源错误, 重新读取后恢复", async () => {
  host.invoke.mockRejectedValueOnce(new Error("公告读取失败"));
  await render();
  await flushPromises();
  expect(wrapper!.get(".bulletin-source-error").text()).toContain(
    "公告读取失败",
  );
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(0);
  expect(host.notify).not.toHaveBeenCalled();

  host.invoke.mockResolvedValueOnce(structuredClone(feed));
  await wrapper!.get(".bulletin-retry .control-button").trigger("click");
  await flushPromises();
  expect(wrapper!.find(".bulletin-source-error").exists()).toBe(false);
  expect(wrapper!.findAll(".bulletin-entry")).toHaveLength(4);
});

it("分类筛选只保留对应来源", async () => {
  await render();
  await flushPromises();
  const tabs = wrapper!.findAll(".control-tabs__tab");
  expect(tabs.length).toBeGreaterThan(2);
  await tabs[1].trigger("click");
  await flushPromises();
  const visible = wrapper!.findAll(
    ".view-stack__page:not([aria-hidden]) .bulletin-entry",
  );
  expect(visible).toHaveLength(2);
  expect(visible.map((entry) => entry.text())).toEqual(
    expect.arrayContaining([expect.stringContaining("第一条")]),
  );
});

it("堆叠开关把同来源公告折叠为一组", async () => {
  host.state.stackBulletins = true;
  const stacked: BulletinFeed = {
    items: [
      item({ title: "一", url: "https://example.com/a1" }),
      item({ title: "二", url: "https://example.com/a2" }),
      item({ title: "三", url: "https://example.com/a3" }),
      item({
        kind: "preview",
        title: "四",
        url: "https://example.com/a4",
        source: "版本前瞻",
      }),
    ],
    errors: [],
  };
  host.invoke.mockResolvedValueOnce(structuredClone(stacked));
  await render();
  await flushPromises();
  const groups = wrapper!.findAll(".bulletin-group");
  expect(groups).toHaveLength(1);
  expect(groups[0].findAll(".bulletin-entry")).toHaveLength(3);
  expect(
    wrapper!.findAll(".view-stack__page:not([aria-hidden]) > .bulletin-entry"),
  ).toHaveLength(1);
  await groups[0].get(".bulletin-group__toggle").trigger("click");
  expect(
    groups[0].get(".bulletin-group__toggle").attributes("aria-expanded"),
  ).toBe("true");
});

it("底部链接按当前区服打开", async () => {
  host.state.distribution = "global";
  await render();
  await flushPromises();
  await wrapper!.get("#open-official").trigger("click");
  expect(host.openUrl).toHaveBeenCalledWith(
    "https://nte.perfectworld.com/en/article/news/index.html",
  );
  await wrapper!.get(".bulletin-entry").trigger("click");
  expect(host.openUrl).toHaveBeenCalledWith("https://example.com/1");
});
