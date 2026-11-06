import { resizeElement } from "./setup";
import { defaultGamescope } from "../../src/core/gameMode";
import { defaultAttendanceTasks } from "../../src/core/attendanceTasks";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { APP_VERSION } from "../../src/core/version";
import { bootstrap, check, session, settings, withGame } from "./fixtures";
import { gameSettings } from "../../src/core/game";

/** save_settings 里当前游戏字段的匹配器. */
const savedGame = (overrides: Record<string, unknown>) =>
  expect.objectContaining({
    games: expect.objectContaining({
      nte: expect.objectContaining(overrides),
    }),
  });

const savedSettings = (
  game: Record<string, unknown>,
  rest: Record<string, unknown> = {},
) =>
  expect.objectContaining({
    ...rest,
    games: expect.objectContaining({
      nte: expect.objectContaining(game),
    }),
  });
import type {
  Bootstrap,
  CheckResult,
  GameStatus,
  DownloadStatus,
  ResourceCatalog,
  LauncherBackgroundMedia,
  PlaytimeHistory,
  Settings,
  IntegrityStatus,
  MigrationStatus,
} from "../../src/types/launcher";

const backend = vi.hoisted(() => ({
  native: true,
  unlisten: vi.fn(),
  downloadUnlisten: vi.fn(),
  settingsUnlisten: vi.fn(),
  integrityUnlisten: vi.fn(),
  migrationUnlisten: vi.fn(),
  stopResize: vi.fn(),
  invoke: vi.fn(),
  handler: (_status: GameStatus) => {},
  settingsHandler: (_data: Bootstrap) => {},
  integrityHandler: (_status: IntegrityStatus) => {},
  migrationHandler: (_status: MigrationStatus) => {},
  playtimeStarted: () => {},
  playtimeUnlisten: vi.fn(),
  downloadHandler: (_status: DownloadStatus) => {},
  downloadStatus: {} as DownloadStatus,
  catalog: {} as ResourceCatalog,
  startStatus: null as DownloadStatus | null,
  startFail: false,
  data: {} as Bootstrap,
  result: {} as CheckResult,
  status: {} as GameStatus,
  playtimeHistory: { sessions: [], active: null } as PlaytimeHistory,
  background: { video: null, image: null } as LauncherBackgroundMedia,
  focused: true,
  visible: true,
  isMaximized: vi.fn(),
  onResized: vi.fn(),
  resize: async () => {},
}));
vi.mock("../../src/core/api", () => ({
  isDesktop: () => backend.native,
  invoke: backend.invoke,
  listenGameState: (handler: (status: GameStatus) => void) => {
    backend.handler = handler;
    return backend.unlisten;
  },
  listenPlaytimeStarted: (handler: () => void) => {
    backend.playtimeStarted = handler;
    return backend.playtimeUnlisten;
  },
  listenDownloadState: (handler: (status: DownloadStatus) => void) => {
    backend.downloadHandler = handler;
    return backend.downloadUnlisten;
  },
  listenSettingsChanged: (handler: (data: Bootstrap) => void) => {
    backend.settingsHandler = handler;
    return backend.settingsUnlisten;
  },
  listenIntegrityState: (handler: (status: IntegrityStatus) => void) => {
    backend.integrityHandler = handler;
    return backend.integrityUnlisten;
  },
  listenMigrationState: (handler: (status: MigrationStatus) => void) => {
    backend.migrationHandler = handler;
    return backend.migrationUnlisten;
  },
}));
vi.mock("../../src/core/gamepad", async (original) => ({
  ...(await original<typeof import("../../src/core/gamepad")>()),
  startGamepad: () => () => {},
}));
// 控制器测试不推进手柄选择层的逐帧布局测量。
vi.mock("../../src/components/GamepadOverlay.vue", () => ({
  default: { render: () => null },
}));
const notificationProjection = vi.hoisted(() => ({
  items: [] as import("../../src/core/notifications").NotificationItem[],
}));
// 控制器测试检查通知数据, 运动轨迹由浏览器用例验证。
vi.mock("../../src/composables/useNotificationArrival", async () => {
  const { computed, ref } = await import("vue");
  return {
    useNotificationArrival: () => ({
      displayed: computed(() => notificationProjection.items),
      retiring: ref([]),
      landed: computed(() =>
        notificationProjection.items.map((item) => item.id),
      ),
      incoming: ref(null),
      revealed: ref(null),
      phase: ref(0),
      settled: computed(() => notificationProjection.items),
    }),
  };
});
vi.mock("../../src/core/desktop", () => ({
  desktopWindow: {
    getState: async () => {
      const focused = backend.focused;
      const visible = backend.visible;
      return {
        maximized: await backend.isMaximized(),
        visible,
        focused,
      };
    },
    onResized: backend.onResized,
  },
}));
let wrapper: VueWrapper | undefined;
let listeners: [
  EventTarget,
  string,
  EventListenerOrEventListenerObject | null,
  boolean | AddEventListenerOptions | undefined,
][];

function catalogFixture(): ResourceCatalog {
  return {
    distribution: "cn",
    installRoot: "/game",
    version: "1.4.16",
    preloadVersion: "1.5.0",
    preloadAvailable: true,
    preloadCompleted: false,
    preloadCached: false,
    preloadCachedVersion: "",
    preloadCachedBytes: 0,
    resources: [
      {
        tag: "pakchunk101",
        name: "pakchunk101",
        version: "0.101.18",
        size: 1_000_000,
        state: "installed",
        selected: true,
      },
      {
        tag: "pakchunk102",
        name: "pakchunk102",
        version: "0.102.18",
        size: 2_000_000,
        state: "absent",
        selected: false,
      },
      {
        tag: "pakchunk103",
        name: "pakchunk103",
        version: "0.103.18",
        size: 3_000_000,
        state: "partial",
        selected: true,
      },
    ],
  };
}

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    drawImage: vi.fn(),
  } as unknown as CanvasRenderingContext2D);
  backend.native = true;
  backend.unlisten.mockClear();
  backend.downloadUnlisten.mockClear();
  backend.settingsUnlisten.mockClear();
  backend.integrityUnlisten.mockClear();
  backend.migrationUnlisten.mockClear();
  backend.stopResize.mockClear();
  backend.settingsHandler = () => {};
  backend.integrityHandler = () => {};
  backend.migrationHandler = () => {};
  backend.data = bootstrap();
  backend.result = check();
  backend.status = session();
  backend.playtimeHistory = { sessions: [], active: null };
  backend.background = { video: null, image: null };
  // 后台预下载提示默认不打扰常规用例, 由专门的用例验证.
  localStorage.setItem("nte.preload.dismissed", "cn:1.5.0");
  backend.catalog = catalogFixture();
  backend.downloadStatus = {
    mode: "check",
    phase: "idle",
    origin: "manual",
    autoWaiting: "",
    checkedAt: 0,
    distribution: "cn",
    installRoot: "/game",
    installed: true,
    full: false,
    unverifiedFiles: 0,
    currentFile: "",
    currentFileBytes: 0,
    completedFiles: 0,
    totalFiles: 0,
    downloadedBytes: 0,
    totalBytes: 0,
    patchFiles: 0,
    patchBytes: 0,
    fullFiles: 0,
    fullBytes: 0,
    stage: "",
    remoteVersion: "",
    optionalTags: [],
    verify: false,
    pendingFiles: 0,
    diskAvailableBytes: 1024 * 1024 * 1024,
    diskRequiredBytes: 0,
    error: "",
  };
  backend.startStatus = structuredClone(backend.downloadStatus);
  backend.startFail = false;
  backend.invoke.mockReset().mockImplementation(
    async (
      command: string,
      args?: {
        settings: Settings;
        mode: "check" | "apply" | "launcher" | "preload";
      },
    ) => {
      switch (command) {
        case "attendance_status":
          return {
            accounts: [],
            preferences: {
              trigger: "off",
              time: "08:00",
              tasks: { ...defaultAttendanceTasks },
            },
          };
        case "launcher_background":
          return structuredClone(backend.background);
        case "bootstrap":
          return structuredClone(backend.data);
        case "check_update":
          return structuredClone(backend.result);
        case "game_status":
          return structuredClone(backend.status);
        case "playtime_history":
          return structuredClone(backend.playtimeHistory);
        case "game_download_status":
          return structuredClone(backend.downloadStatus);
        case "game_download_start": {
          if (backend.startFail) throw new Error("quick check offline");
          if (backend.startStatus) {
            backend.downloadStatus = structuredClone(backend.startStatus);
            return structuredClone(backend.downloadStatus);
          }
          backend.downloadStatus = {
            ...backend.downloadStatus,
            mode: args!.mode,
            phase: "checking",
            distribution: gameSettings(backend.data.settings).distribution,
            installRoot:
              gameSettings(backend.data.settings).distribution === "global"
                ? gameSettings(backend.data.settings).globalInstallRoot ||
                  backend.data.installRoot
                : gameSettings(backend.data.settings).installRoot ||
                  backend.data.installRoot,
          };
          return structuredClone(backend.downloadStatus);
        }
        case "game_download_cancel":
          backend.downloadStatus = {
            ...backend.downloadStatus,
            phase: "cancelled",
          };
          return structuredClone(backend.downloadStatus);
        case "game_resource_catalog":
          return structuredClone(backend.catalog);
        case "game_preload_delete":
          backend.catalog = {
            ...backend.catalog,
            preloadCompleted: false,
            preloadCached: false,
            preloadCachedVersion: "",
            preloadCachedBytes: 0,
          };
          return structuredClone(backend.catalog);
        case "game_resource_select": {
          const { tags } = args as unknown as { tags: string[] };
          backend.catalog.resources.forEach((resource) => {
            if (resource.state === "absent")
              resource.selected = tags.includes(resource.tag);
          });
          return structuredClone(backend.catalog);
        }
        case "save_settings":
          backend.data.settings = { ...args!.settings };
          backend.data.installRoot =
            gameSettings(args!.settings).distribution === "global"
              ? gameSettings(args!.settings).globalInstallRoot ||
                "/games/global"
              : gameSettings(args!.settings).installRoot || "/game";
          return structuredClone(backend.data);
        case "launch_official":
          backend.status = session({ running: true, pid: 123 });
          return { pid: 123 };
        case "close_game":
          backend.status = session();
          return structuredClone(backend.status);
        default:
          return null;
      }
    },
  );
  backend.isMaximized.mockReset().mockResolvedValue(false);
  backend.focused = true;
  backend.visible = true;
  backend.onResized
    .mockReset()
    .mockImplementation(async (callback: () => Promise<void>) => {
      backend.resize = callback;
      return backend.stopResize;
    });
  listeners = [];
  const add = EventTarget.prototype.addEventListener;
  vi.spyOn(EventTarget.prototype, "addEventListener").mockImplementation(
    function (this: EventTarget, type, callback, options) {
      listeners.push([this, type, callback, options]);
      add.call(this, type, callback, options);
    },
  );
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  for (const [target, type, callback, options] of listeners)
    target.removeEventListener(type, callback, options);
});

async function boot(
  beforeCheck?: (
    controller: typeof import("../../src/core/controller"),
  ) => void,
) {
  const controller = await import("../../src/core/controller");
  const { default: App } = await import("../../src/App.vue");
  notificationProjection.items = controller.notifications;
  wrapper = mount(App, { attachTo: document.body });
  await flushPromises();
  beforeCheck?.(controller);
  await vi.advanceTimersByTimeAsync(1000);
  await flushPromises();
  backend.startStatus = null;
  backend.startFail = false;
  return controller;
}
const text = (id: string) => document.getElementById(id)?.textContent;
const noticeMessage = () =>
  document.querySelector(".notification-message")?.textContent;
const button = (id: string) => document.getElementById(id) as HTMLButtonElement;
const primary = () => wrapper!.find("#game-download-primary");
const primaryText = () => primary().text();
it("环境操作先保存修改, 处理期间拒绝重入并同步新登记", async () => {
  const c = await boot();
  const base = backend.invoke.getMockImplementation()!;
  let finish!: (data: Bootstrap) => void;
  let releaseSave!: () => void;
  const saving = new Promise<void>((resolve) => {
    releaseSave = resolve;
  });
  backend.invoke.mockClear();
  backend.invoke.mockImplementation((command: string, args: never) =>
    command === "save_settings"
      ? saving.then(() => base(command, args))
      : command === "environment_create"
        ? new Promise<Bootstrap>((resolve) => {
            finish = resolve;
          })
        : base(command, args),
  );
  c.form.theme = "dark";
  c.form.globalInstallRoot = "/new-install";
  const task = c.createRuntimeEnvironment(
    "测试环境",
    "/new-proton",
    null,
    "cn",
  );
  await flushPromises();
  expect(c.runtimeEnvironmentBusy.value).toBe(true);
  await c.checkUpdate();
  await c.startGameDownload("check");
  expect(backend.invoke.mock.calls.map(([command]) => command)).toEqual([
    "save_settings",
  ]);
  expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
    settings: savedGame({ globalInstallRoot: "/new-install" }),
  });
  releaseSave();
  await flushPromises();
  expect(backend.invoke).toHaveBeenCalledWith("environment_create", {
    distribution: "cn",
    name: "测试环境",
    protonPath: "/new-proton",
    sourceId: null,
  });
  expect(c.runtimeEnvironmentBusy.value).toBe(true);
  expect(c.settingsPathsLocked.value).toBe(true);
  expect(
    await c.createRuntimeEnvironment("重复", "/new-proton", null, "cn"),
  ).toBe(false);
  const next = bootstrap({
    settings: {
      ...backend.data.settings,
      runtimeEnvironments: [
        {
          id: "a".repeat(32),
          name: "测试环境",
          proton: { path: "/new-proton", identity: "pin" },
          installRoot: "/game",
          game: "nte",
          distribution: "cn",
          createdAt: 1,
        },
      ],
    },
  });
  finish(next);
  expect(await task).toBe(true);
  expect(c.runtimeEnvironmentBusy.value).toBe(false);
  expect(c.state.bootstrap?.settings.runtimeEnvironments).toHaveLength(1);
  c.form.reduceMotion = false;
  await c.saveSettings();
  expect(backend.data.settings.runtimeEnvironments[0].name).toBe("测试环境");
});

it("环境操作失败时保留原选择并释放等待状态", async () => {
  const c = await boot();
  backend.invoke.mockRejectedValueOnce(new Error("固定的 Proton 已变化"));
  expect(await c.runtimeEnvironmentAction("switch", "missing")).toBe(false);
  expect(c.runtimeEnvironmentError.value).toContain("Proton 已变化");
  expect(c.runtimeEnvironmentBusy.value).toBe(false);
  expect(
    gameSettings(c.state.bootstrap?.settings).activeEnvironment,
  ).toBeNull();
});
const key = (value: string, options: KeyboardEventInit = {}) =>
  document.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: value,
      bubbles: true,
      cancelable: true,
      ...options,
    }),
  );

describe("启动与设置", () => {
  it("加载配置、诊断和版本, 并自动检查更新", async () => {
    const c = await boot();
    c.openSettings("maintenance");
    await nextTick();
    expect(c.attendance.loaded).toBe(true);
    expect(c.attendance.loading).toBe(false);
    expect(c.form.installRoot).toBe("/game");
    expect(wrapper!.findAll(".check-row")).toHaveLength(2);
    expect(wrapper!.findAll(".check-row")[0].text()).toContain(
      "Oddity 安全性验证",
    );
    expect(c.displayVersion.value).toBe("0.1.0");
    expect(text("update-state-title")).toBe("已是最新");
    expect(button("app").classList.contains("is-intro-done")).toBe(true);
    backend.isMaximized.mockResolvedValueOnce(true);
    await backend.resize();
    expect(button("app").classList.contains("is-maximized")).toBe(true);
  });

  it("设置分类按访问挂载, 已访问的面板保持挂载", async () => {
    const c = await boot();
    expect(document.querySelector("#set-install")).toBeNull();
    expect(document.querySelector("#set-theme")).toBeNull();
    c.openSettings("game");
    await nextTick();
    expect(document.querySelector("#set-install")).not.toBeNull();
    c.openSettings("appearance");
    await nextTick();
    expect(document.querySelector("#set-install")).not.toBeNull();
    expect(document.querySelector("#set-theme")).not.toBeNull();
  });

  it("缺少环境和版本时显示配置提示和默认值", async () => {
    backend.data = bootstrap({
      ok: false,
      error: "找不到游戏",
      gameExists: false,
      appVersion: "",
      launcherVersion: "",
      installRoot: "",
      prefix: "",
      protonPath: "",
      settings: withGame(
        { launchMode: "" },
        { ...settings, theme: "", heroMedia: "" },
      ),
    });
    const c = await boot();
    expect(button("setup-chip").hidden).toBe(false);
    expect(noticeMessage()).toBe("找不到游戏");
    expect(c.displayVersion.value).toBe(APP_VERSION);
    expect(text("update-state-text")).toBe("等待检查");
    expect(c.form.launchMode).toBe("background");
    expect(c.form.theme).toBe("light");
  });

  it("未创建运行环境时, 启动操作转到设置并提示创建", async () => {
    backend.data = bootstrap({
      prefix: "",
      prefixExists: false,
      protonPath: "",
    });
    const c = await boot();
    backend.invoke.mockClear();
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.page).toBe("settings");
    expect(noticeMessage()).toContain("请先在设置中创建");
    expect(backend.invoke).not.toHaveBeenCalledWith("launch_official");
  });

  it("启动失败仍完成入场并允许手动重新检测", async () => {
    const invoke = backend.invoke.getMockImplementation()!;
    let failedStatus = false;
    let failedBootstrap = false;
    backend.invoke.mockImplementation(async (command, args) => {
      if (command === "game_status" && !failedStatus) {
        failedStatus = true;
        throw new Error("status unavailable");
      }
      if (command === "bootstrap" && !failedBootstrap) {
        failedBootstrap = true;
        throw new Error("bootstrap failed");
      }
      return invoke(command, args);
    });
    const c = await boot();
    expect(noticeMessage()).toBe("bootstrap failed");
    expect(button("app").classList.contains("is-intro-done")).toBe(true);
    await c.redetect();
    expect(c.state.bootstrap?.ok).toBe(true);
    expect(noticeMessage()).toBe("已重新检测环境。");
    backend.invoke.mockRejectedValueOnce("redetect failed");
    await c.redetect();
    expect(noticeMessage()).toBe("redetect failed");
    expect(button("settings-redetect").disabled).toBe(false);
  });

  it("重新检测前保存失败时保留表单并停止刷新", async () => {
    const c = await boot();
    c.form.globalInstallRoot = "/draft-prefix";
    backend.invoke.mockClear();
    backend.invoke.mockRejectedValueOnce(new Error("disk full"));
    await c.redetect();
    expect(noticeMessage()).toBe("disk full");
    expect(c.form.globalInstallRoot).toBe("/draft-prefix");
    expect(c.ui.saveState).toBe("error");
    expect(backend.invoke.mock.calls.map(([command]) => command)).toEqual([
      "save_settings",
    ]);
    expect(button("settings-redetect").disabled).toBe(false);
  });

  it("无本地存储权限时仍能应用外观", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    const c = await boot();
    expect(c.applyTheme("invalid")).toBe("light");
    expect(c.applyTheme("dark")).toBe("dark");
    await nextTick();
    expect(button("app").dataset.theme).toBe("dark");
    expect(c.applyReduceMotion(false)).toBe(false);
    await nextTick();
    expect(button("app").classList.contains("reduce-motion")).toBe(false);
  });

  it("保存时清理路径, 持久化外观, 失败后保留错误状态", async () => {
    const c = await boot();
    Object.assign(c.form, {
      installRoot: " /new ",
      launchMode: "",
      theme: "dark",
      reduceMotion: false,
      heroMedia: "photo",
      pauseHeroOnBlur: true,
    });
    await c.saveSettings();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: {
        ...withGame({
          installRoot: "/new",
          globalInstallRoot: null,
          launchMode: "background",
        }),
        language: "zh-CN",
        theme: "dark",
        reduceMotion: false,
        heroMedia: "photo",
        pauseHeroOnBlur: true,
        minimizeToTrayOnClose: true,
        showLauncherAfterGameExit: true,
      },
    });
    expect(c.ui.saveState).toBe("saved");
    vi.advanceTimersByTime(2000);
    await flushPromises();
    expect(c.ui.saveState).toBe("idle");
    Object.assign(c.form, { installRoot: "", globalInstallRoot: "" });
    backend.invoke.mockRejectedValueOnce(new Error("disk full"));
    await c.saveSettings();
    expect(noticeMessage()).toBe("disk full");
    expect(c.ui.saveState).toBe("error");
    vi.advanceTimersByTime(6000);
    await flushPromises();
    expect(document.querySelector(".notification-stack")).toBeNull();
  });

  it("下载限速支持自由填写并保留小数", async () => {
    const c = await boot();
    c.openSettings("game");
    await nextTick();
    backend.invoke.mockClear();

    const input = document.querySelector<HTMLInputElement>(
      "#set-download-limit",
    )!;
    input.value = "12.5";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(100);
    await flushPromises();
    const saved = backend.invoke.mock.calls
      .filter(([command]) => command === "save_settings")
      .at(-1);
    expect(saved).toBeDefined();
    expect(
      gameSettings((saved![1] as { settings: Settings }).settings).updates
        .limitBytesPerSecond,
    ).toBe(13107200);

    input.value = "0";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(100);
    await flushPromises();
    const cleared = backend.invoke.mock.calls
      .filter(([command]) => command === "save_settings")
      .at(-1);
    expect(
      gameSettings((cleared![1] as { settings: Settings }).settings).updates
        .limitBytesPerSecond,
    ).toBe(0);
  });

  it("文本输入防抖自动保存, 保存状态随后回到空闲", async () => {
    const c = await boot();
    c.openSettings("game");
    await nextTick();
    backend.invoke.mockClear();
    const input = document.querySelector<HTMLInputElement>("#set-install")!;
    input.value = " /auto-game ";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    expect(c.ui.saveState).toBe("idle");
    await vi.advanceTimersByTimeAsync(499);
    expect(backend.invoke).not.toHaveBeenCalledWith(
      "save_settings",
      expect.anything(),
    );
    await vi.advanceTimersByTimeAsync(1);
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: savedGame({ installRoot: "/auto-game" }),
    });
    expect(c.ui.saveState).toBe("saved");
    await vi.advanceTimersByTimeAsync(2000);
    expect(c.ui.saveState).toBe("idle");
  });

  it("修改安装目录后统一刷新启动器与游戏资源", async () => {
    const c = await boot();
    c.openSettings("game");
    await nextTick();
    backend.invoke.mockClear();
    const input = document.querySelector<HTMLInputElement>("#set-install")!;
    input.value = "/new-game";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(500);
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: savedGame({ installRoot: "/new-game" }),
    });
    expect(backend.invoke).toHaveBeenCalledWith("check_update", {
      full: false,
    });
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: false,
    });
  });

  it("更新页修改安装目录也会统一刷新", async () => {
    const c = await boot();
    c.showPage("update");
    await nextTick();
    backend.invoke.mockClear();
    const input = document.querySelector<HTMLInputElement>(
      "#download-install-root",
    )!;
    input.value = "/update-game";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(500);
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: savedGame({ installRoot: "/update-game" }),
    });
    expect(backend.invoke).toHaveBeenCalledWith("check_update", {
      full: false,
    });
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: false,
    });
  });

  it("切换到没有启动器的目录时两张状态卡同时显示未安装", async () => {
    backend.data = bootstrap({ launcherRoot: "", launcherVersion: "" });
    const c = await boot();
    c.openSettings("game");
    await nextTick();
    backend.invoke.mockClear();
    const input = document.querySelector<HTMLInputElement>("#set-install")!;
    input.value = "/empty-game";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(500);
    await flushPromises();

    expect(c.updateView.value.title).toBe("尚未安装");
    await c.showPage("update");
    await nextTick();
    expect(text("game-download-state-title")).toBe("需要安装启动器文件");
    expect(primaryText()).toBe("安装游戏");
    expect(text("update-state-title")).toBe("尚未安装");
    expect(backend.invoke).not.toHaveBeenCalledWith("check_update", {
      full: false,
    });
    expect(backend.invoke).not.toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: false,
    });
  });

  it("开关修改立即自动保存", async () => {
    const c = await boot();
    c.openSettings("appearance");
    await nextTick();
    const checkbox = document.querySelector<HTMLInputElement>(
      "#set-minimize-to-tray",
    )!;
    checkbox.checked = false;
    checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: expect.objectContaining({ minimizeToTrayOnClose: false }),
    });
    expect(c.ui.saveState).toBe("saved");
  });

  it("游戏模式选项通过统一保存队列持久化并从后端恢复", async () => {
    const c = await boot();
    Object.assign(c.form.gameMode, {
      performance: true,
      inhibit: true,
      notifications: false,
      overlay: true,
      gamescope: { ...defaultGamescope },
    });
    c.queueSettingsSave(0);
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: savedGame({
        gameMode: {
          performance: true,
          inhibit: true,
          notifications: false,
          overlay: true,
          gamescope: { ...defaultGamescope },
        },
      }),
    });
    expect(c.ui.saveState).toBe("saved");
    await c.redetect();
    expect(c.form.gameMode).toEqual({
      performance: true,
      inhibit: true,
      notifications: false,
      overlay: true,
      gamescope: { ...defaultGamescope },
    });
  });

  it("下载写入期间锁住地区和安装目录, 其他设置仍可修改", async () => {
    const c = await boot();
    c.openSettings("game");
    await nextTick();
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "apply",
      phase: "game",
    });
    await nextTick();
    expect(c.settingsPathsLocked.value).toBe(true);
    expect(
      document.querySelector<HTMLSelectElement>("#set-distribution")!.disabled,
    ).toBe(true);
    expect(
      document.querySelector<HTMLButtonElement>("#set-distribution-trigger")!
        .disabled,
    ).toBe(true);
    expect(
      document.querySelector<HTMLInputElement>("#set-install")!.disabled,
    ).toBe(true);
    expect(
      document.querySelector<HTMLInputElement>("#set-global-install")!.disabled,
    ).toBe(true);
    c.openSettings("appearance");
    await nextTick();
    expect(
      document.querySelector<HTMLSelectElement>("#set-theme")!.disabled,
    ).toBe(false);
    expect(
      document.querySelector<HTMLSelectElement>("#set-mode")!.disabled,
    ).toBe(false);
    expect(wrapper!.find(".settings-lock").text()).toContain(
      "暂时无法修改游戏地区和安装目录",
    );
    backend.invoke.mockClear();
    c.applyTheme("dark");
    c.queueSettingsSave(0);
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: savedSettings(
        { installRoot: null, globalInstallRoot: null },
        { theme: "dark" },
      ),
    });
    expect(c.ui.saveState).toBe("saved");
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "check",
      phase: "checking",
    });
    await nextTick();
    expect(c.settingsPathsLocked.value).toBe(false);
    expect(
      document.querySelector<HTMLInputElement>("#set-install")!.disabled,
    ).toBe(false);
    expect(wrapper!.find(".settings-lock").exists()).toBe(false);
  });

  it("自动检查完成不提示下载完成, 自动更新完成才提示", async () => {
    const c = await boot();
    backend.downloadStatus = {
      ...backend.downloadStatus,
      origin: "auto",
      mode: "check",
      phase: "checking",
      autoWaiting: "",
    };
    backend.downloadHandler({ ...backend.downloadStatus, phase: "ready" });
    await nextTick();
    await flushPromises();
    expect(
      c.notifications.some((item) => item.key === "auto-download-done"),
    ).toBe(false);

    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "apply",
      phase: "checking",
    });
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "apply",
      phase: "ready",
    });
    await nextTick();
    await flushPromises();
    expect(
      c.notifications.some((item) => item.key === "auto-download-done"),
    ).toBe(true);
  });

  it("保存中的连续修改串行提交最新快照", async () => {
    const c = await boot();
    let finish!: (data: Bootstrap) => void;
    backend.invoke.mockImplementationOnce(
      () =>
        new Promise<Bootstrap>((resolve) => {
          finish = resolve;
        }),
    );
    c.form.globalInstallRoot = "/first";
    c.queueSettingsSave(0);
    await flushPromises();
    c.form.globalInstallRoot = "/second";
    c.queueSettingsSave(0);
    await flushPromises();
    finish(bootstrap());
    await flushPromises();
    const saves = backend.invoke.mock.calls.filter(
      ([command]) => command === "save_settings",
    );
    expect(saves).toHaveLength(2);
    expect(saves[1][1]).toMatchObject({
      settings: { games: { nte: { globalInstallRoot: "/second" } } },
    });
    expect(c.ui.saveState).toBe("saved");
  });

  it("自动保存失败后在下一次修改时重试", async () => {
    const c = await boot();
    backend.invoke.mockRejectedValueOnce(new Error("disk full"));
    c.form.globalInstallRoot = "/blocked";
    c.queueSettingsSave(0);
    await flushPromises();
    expect(noticeMessage()).toBe("disk full");
    expect(c.ui.saveState).toBe("error");
    c.form.globalInstallRoot = "/retry";
    c.queueSettingsSave(0);
    await flushPromises();
    expect(c.ui.saveState).toBe("saved");
    expect(backend.invoke).toHaveBeenLastCalledWith(
      "save_settings",
      expect.objectContaining({
        settings: savedGame({ globalInstallRoot: "/retry" }),
      }),
    );
  });

  it("关闭窗口时最小化到托盘配置支持切换与保存", async () => {
    const c = await boot();
    c.openSettings("appearance");
    await nextTick();
    expect(c.form.minimizeToTrayOnClose).toBe(true);
    const checkbox = document.querySelector<HTMLInputElement>(
      "#set-minimize-to-tray",
    )!;
    expect(checkbox.checked).toBe(true);
    c.form.minimizeToTrayOnClose = false;
    await c.saveSettings();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: expect.objectContaining({
        minimizeToTrayOnClose: false,
      }),
    });
  });

  it("游戏退出后打开启动器配置支持切换与自动保存", async () => {
    const c = await boot();
    c.openSettings("appearance");
    await nextTick();
    expect(c.form.showLauncherAfterGameExit).toBe(true);
    const checkbox = document.querySelector<HTMLInputElement>(
      "#set-show-after-game-exit",
    )!;
    expect(checkbox.checked).toBe(true);
    checkbox.checked = false;
    checkbox.dispatchEvent(new Event("change", { bubbles: true }));
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: expect.objectContaining({
        showLauncherAfterGameExit: false,
      }),
    });
    expect(c.ui.saveState).toBe("saved");
  });
});

describe("下载大小显示", () => {
  it("按 B、KiB、MiB、GiB 格式化并处理零值", async () => {
    const c = await boot();
    expect(c.formatSize(0)).toBe("0 B");
    expect(c.formatSize(1024)).toBe("1.0 KiB");
    expect(c.formatSize(1536)).toBe("1.5 KiB");
    expect(c.formatSize(330 * 1024 * 1024)).toBe("330 MiB");
    expect(c.formatSize(3.4 * 1024 * 1024 * 1024)).toBe("3.4 GiB");
  });

  it("检查后展示空间需求并标出缺口", async () => {
    const c = await boot();
    await c.showPage("update");
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      pendingFiles: 2,
      totalFiles: 2,
      diskAvailableBytes: 800 * 1024 * 1024,
      diskRequiredBytes: 1024 * 1024 * 1024,
    });
    await nextTick();
    expect(c.diskBudget.value).toEqual({
      availableText: "800 MiB",
      requiredText: "1.0 GiB",
      shortfallText: "224 MiB",
      insufficient: true,
      percent: 100,
    });
    expect(document.body.textContent).toContain("预计需要 1.0 GiB");
    expect(document.body.textContent).toContain("可用 800 MiB");
    expect(document.body.textContent).toContain("还差 224 MiB");
    expect(document.body.textContent).toContain("清理磁盘空间或更换安装目录");
  });

  it("空间充足时按占用比例展示且不提示缺口", async () => {
    const c = await boot();
    await c.showPage("update");
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      pendingFiles: 1,
      totalFiles: 1,
      diskAvailableBytes: 2 * 1024 * 1024 * 1024,
      diskRequiredBytes: 512 * 1024 * 1024,
    });
    await nextTick();
    expect(c.diskBudget.value).toMatchObject({
      insufficient: false,
      percent: 25,
    });
    expect(document.body.textContent).not.toContain("还差");
    expect(document.body.textContent).not.toContain(
      "清理磁盘空间或更换安装目录",
    );
  });

  it("空间读取失败或没有待处理资源时不展示预算", async () => {
    const c = await boot();
    await c.showPage("update");
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      pendingFiles: 1,
      diskAvailableBytes: null,
      diskRequiredBytes: 100,
    });
    await nextTick();
    expect(c.diskBudget.value).toBeNull();

    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      pendingFiles: 1,
      diskAvailableBytes: 0,
      diskRequiredBytes: 100,
    });
    await nextTick();
    expect(c.diskBudget.value).toMatchObject({
      insufficient: true,
      percent: 100,
      shortfallText: "100 B",
    });

    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "ready",
      pendingFiles: 0,
      diskAvailableBytes: 100,
      diskRequiredBytes: 0,
    });
    await nextTick();
    expect(c.diskBudget.value).toBeNull();
  });
});

describe("更新检查", () => {
  it("启动检查不持久化探测路径或尚未保存的表单", async () => {
    const c = await boot((controller) => {
      expect(controller.form.globalInstallRoot).toBe("");
      controller.form.globalInstallRoot = "/draft-prefix";
    });
    expect(backend.invoke).not.toHaveBeenCalledWith(
      "save_settings",
      expect.anything(),
    );
    expect(c.form.globalInstallRoot).toBe("/draft-prefix");
  });

  it.each(["failed", "cancelled", "ready"] as const)(
    "开始响应不能覆盖先收到的 %s 事件",
    async (phase) => {
      const c = await boot();
      const original = backend.invoke.getMockImplementation()!;
      let resolve!: (status: DownloadStatus) => void;
      backend.invoke.mockImplementation((command, args) =>
        command === "game_download_start"
          ? new Promise<DownloadStatus>((done) => {
              resolve = done;
            })
          : original(command, args),
      );
      const pending = c.startGameDownload("check");
      await flushPromises();
      backend.downloadHandler({ ...backend.downloadStatus, phase });
      resolve({ ...backend.downloadStatus, phase: "checking" });
      await pending;
      expect(c.state.download?.phase).toBe(phase);
      expect(c.downloadBusy()).toBe(false);
    },
  );

  it("检查资源完成后刷新预下载目录", async () => {
    const c = await boot();
    await flushPromises();
    backend.invoke.mockClear();
    const start = c.startGameDownload("check", false);
    await flushPromises();
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "check",
      phase: "ready",
    });
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_resource_catalog");
    await start;
  });

  it("取消响应不能覆盖先收到的终止事件", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "apply",
      phase: "game",
    });
    let resolve!: (status: DownloadStatus) => void;
    backend.invoke.mockImplementationOnce(
      () =>
        new Promise<DownloadStatus>((done) => {
          resolve = done;
        }),
    );
    const pending = c.cancelGameDownload();
    backend.downloadHandler({ ...backend.downloadStatus, phase: "cancelled" });
    resolve({ ...backend.downloadStatus, mode: "apply", phase: "game" });
    await pending;
    expect(c.state.download?.phase).toBe("cancelled");
    expect(c.downloadBusy()).toBe(false);
  });

  it("就绪后的环境刷新保留等待期间的设置编辑和主题", async () => {
    const c = await boot();
    let resolve!: (data: Bootstrap) => void;
    backend.invoke.mockImplementationOnce(
      () =>
        new Promise<Bootstrap>((done) => {
          resolve = done;
        }),
    );
    backend.downloadHandler({ ...backend.downloadStatus, phase: "ready" });
    c.form.installRoot = "/unsaved/game";
    c.form.globalInstallRoot = "/unsaved/prefix";
    c.applyTheme("dark");
    resolve(bootstrap({ gameVersion: "2.0" }));
    await flushPromises();
    expect(c.state.bootstrap?.gameVersion).toBe("2.0");
    expect(c.form.installRoot).toBe("/unsaved/game");
    expect(c.form.globalInstallRoot).toBe("/unsaved/prefix");
    expect(c.form.theme).toBe("dark");
    expect(primaryText()).toBe("检查资源");
  });

  it("就绪刷新不能覆盖在请求期间保存的新地区", async () => {
    const c = await boot();
    let resolve!: (data: Bootstrap) => void;
    backend.invoke.mockImplementationOnce(
      () =>
        new Promise<Bootstrap>((done) => {
          resolve = done;
        }),
    );
    backend.downloadHandler({ ...backend.downloadStatus, phase: "ready" });
    c.form.distribution = "global";
    await c.saveSettings();
    resolve(bootstrap());
    await flushPromises();
    expect(gameSettings(c.state.bootstrap?.settings).distribution).toBe(
      "global",
    );
    expect(c.form.distribution).toBe("global");
  });

  it("只读检查可取消且不阻止启动和保存, 安装检查仍阻止启动", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "check",
      phase: "checking",
    });
    await nextTick();
    expect(document.body.textContent).toContain("取消检查");
    expect(c.launchView.value.busy).toBe(false);
    expect(await c.saveSettings()).toBe(true);
    c.handleLaunchAction();
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("launch_official");
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "apply",
      phase: "checking",
    });
    expect(c.launchView.value.busy).toBe(true);
  });

  it("首页更新提示使用已保存地区", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
    });
    c.form.distribution = "global";
    expect(c.homeDownloadNotice.value.visible).toBe(true);
    expect(c.launchView.value.label).toBe("更新游戏");
    backend.downloadHandler({
      ...backend.downloadStatus,
      distribution: "global",
      phase: "needs-update",
    });
    expect(c.homeDownloadNotice.value.visible).toBe(false);
    expect(c.launchView.value.label).toBe("启动游戏");
  });

  it("首页启动提示在固定槽位内显隐", async () => {
    await boot();
    const slot = wrapper!.find(".hero__status-slot");
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "check",
      phase: "cancelled",
    });
    await nextTick();
    expect(slot.classes()).toContain("is-visible");
    expect(wrapper!.find("#home-status").attributes("aria-hidden")).toBe(
      undefined,
    );
    expect(text("home-status-text")).toBe("资源检查已取消, 打开更新页重新检查");
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "check",
      phase: "checking",
    });
    await nextTick();
    expect(slot.classes()).not.toContain("is-visible");
    expect(wrapper!.find("#home-status").attributes("aria-hidden")).toBe(
      "true",
    );
  });

  it("资源预下载独立展示完成与暂停, 不将未来资源视为当前更新", async () => {
    const c = await boot();
    await wrapper!.get("#game-preload").trigger("click");
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "preload",
      full: false,
    });
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "preload",
      phase: "preloaded",
      remoteVersion: "2.0",
      totalFiles: 1,
      completedFiles: 1,
      pendingFiles: 0,
    });
    await nextTick();
    expect(c.downloadView.value.title).toBe("资源预下载已完成");
    expect(c.downloadView.value.progress).toBe(100);
    expect(c.downloadView.value.detail).toContain("版本更新开放后");
    expect(c.launchView.value.label).not.toBe("更新游戏");
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "preload",
      phase: "cancelled",
      pendingFiles: 1,
    });
    await nextTick();
    expect(primaryText()).toBe("继续资源预下载");
    expect(c.downloadView.value.detail).toContain("下次预下载");
  });

  it("更新页读取可选资源并提交选择", async () => {
    const c = await boot();
    c.showPage("update");
    await nextTick();
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_resource_catalog");
    expect(c.state.resourceCatalog?.resources.length).toBe(3);
    expect(c.state.resourceCatalog?.preloadVersion).toBe("1.5.0");
    expect(wrapper!.get("#game-preload").text()).toBe("资源预下载 1.5.0");
    await wrapper!.get(".resource-options__toggle").trigger("click");

    const installed = wrapper!.get("#optional-resource-pakchunk101")
      .element as HTMLInputElement;
    expect(installed.disabled).toBe(true);
    expect(installed.checked).toBe(true);

    await wrapper!.get("#optional-resource-pakchunk102").setValue(true);
    expect(
      c.state.resourceCatalog?.resources.find(
        (resource) => resource.tag === "pakchunk102",
      )?.selected,
    ).toBe(true);
    expect(c.state.resourcesSaving).toBe(true);
    await vi.advanceTimersByTimeAsync(400);
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_resource_select", {
      tags: ["pakchunk101", "pakchunk102", "pakchunk103"],
    });
    expect(c.state.resourcesSaving).toBe(false);
    expect(c.state.resourceCatalog?.resources.length).toBe(3);
  });

  it("已完成预下载显示校验入口并可删除缓存", async () => {
    backend.catalog.preloadCompleted = true;
    backend.catalog.preloadCached = true;
    backend.catalog.preloadCachedBytes = 2048;
    const c = await boot();
    c.showPage("update");
    await nextTick();
    await flushPromises();
    expect(wrapper!.get("#game-preload").text()).toBe("校验预下载");
    await wrapper!.get("#game-preload-delete").trigger("click");
    await nextTick();
    const confirm = [...document.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "删除",
    ) as HTMLButtonElement | undefined;
    expect(confirm).toBeTruthy();
    confirm!.click();
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_preload_delete");
    expect(wrapper!.get("#game-preload").text()).toBe("资源预下载 1.5.0");
    expect(wrapper!.find("#game-preload-delete").exists()).toBe(false);
  });

  it("缺少预下载公告时隐藏预下载入口但保留缓存清理", async () => {
    backend.catalog.preloadAvailable = false;
    backend.catalog.preloadVersion = "";
    backend.catalog.preloadCached = true;
    backend.catalog.preloadCachedVersion = "1.5.0";
    backend.catalog.preloadCachedBytes = 2048;
    const c = await boot();
    c.showPage("update");
    await nextTick();
    await flushPromises();
    expect(wrapper!.find("#game-preload").exists()).toBe(false);
    expect(wrapper!.find("#game-preload-delete").exists()).toBe(true);
    await wrapper!.get("#game-preload-delete").trigger("click");
    await nextTick();
    const confirm = [...document.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "删除",
    ) as HTMLButtonElement | undefined;
    expect(confirm).toBeTruthy();
    confirm!.click();
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_preload_delete");
    expect(wrapper!.find("#game-preload-delete").exists()).toBe(false);
    expect(backend.catalog.preloadCached).toBe(false);
  });

  it("后台发现预下载公告时提示, 可按版本忽略", async () => {
    localStorage.removeItem("nte.preload.dismissed");
    await boot();
    await flushPromises();
    expect(noticeMessage()).toContain("1.5.0 资源可预下载");
    const secondary = document.querySelector(
      ".notification-action--ghost",
    ) as HTMLButtonElement | null;
    expect(secondary).toBeTruthy();
    expect(secondary!.querySelector("svg")).toBeTruthy();
    secondary!.click();
    await flushPromises();
    expect(localStorage.getItem("nte.preload.dismissed")).toBe("cn:1.5.0");
  });

  it("可选资源全选与取消全选只调整未安装项", async () => {
    const c = await boot();
    c.showPage("update");
    await nextTick();
    await flushPromises();
    await wrapper!.get(".resource-options__toggle").trigger("click");
    await wrapper!.get("#optional-resources-all").trigger("click");
    expect(
      c.state.resourceCatalog?.resources.every((resource) => resource.selected),
    ).toBe(true);
    await vi.advanceTimersByTimeAsync(400);
    await flushPromises();
    expect(backend.invoke).toHaveBeenLastCalledWith("game_resource_select", {
      tags: ["pakchunk101", "pakchunk102", "pakchunk103"],
    });
    expect(wrapper!.get("#optional-resources-all").text()).toBe("取消全选");

    await wrapper!.get("#optional-resources-all").trigger("click");
    expect(
      c.state.resourceCatalog?.resources
        .filter((resource) => resource.state === "absent")
        .every((resource) => !resource.selected),
    ).toBe(true);
    await vi.advanceTimersByTimeAsync(400);
    await flushPromises();
    expect(backend.invoke).toHaveBeenLastCalledWith("game_resource_select", {
      tags: ["pakchunk101", "pakchunk103"],
    });
  });

  it("为国际服保存独立目录并展示可暂停的下载任务", async () => {
    const c = await boot();
    c.form.distribution = "global";
    c.form.globalInstallRoot = "/games/global";
    await c.startGameDownload("apply", true);
    expect(backend.invoke).toHaveBeenCalledWith("save_settings", {
      settings: savedGame({
        distribution: "global",
        globalInstallRoot: "/games/global",
      }),
    });
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "apply",
      full: true,
    });
    expect(c.state.download?.phase).toBe("checking");
    expect(document.body.textContent).not.toContain("暂停下载");
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "game",
      currentFile: "Client/data.pak",
      currentFileBytes: 300 * 1024 * 1024,
      downloadedBytes: 4,
      totalBytes: 10,
      patchBytes: 6,
      fullBytes: 4,
      completedFiles: 1,
      totalFiles: 2,
      pendingFiles: 2,
      stage: "patch",
    });
    await nextTick();
    expect(document.body.textContent).toContain("暂停下载");
    expect(c.downloadView.value.progress).toBe(40);
    expect(c.downloadView.value.sizeText).toBe("本次需下载 10 B");
    expect(c.downloadView.value.stageText).toBe("应用补丁");
    expect(c.downloadView.value.splitText).toBe("补丁 6 B / 完整文件 4 B");
    expect(document.body.textContent).toContain("本次需下载 10 B");
    expect(document.body.textContent).toContain("补丁 6 B / 完整文件 4 B");
    expect(document.body.textContent).toContain("正在应用补丁");
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "game",
      downloadedBytes: 8,
      totalBytes: 10,
      completedFiles: 0,
      totalFiles: 2,
      pendingFiles: 2,
    });
    await nextTick();
    expect(c.downloadView.value.progress).toBe(80);
    expect(c.downloadView.value.downloadedText).toBe("8 B / 10 B");
    expect(button("download-install-root").disabled).toBe(true);
    await c.cancelGameDownload();
    expect(backend.invoke).toHaveBeenCalledWith("game_download_cancel");
  });

  it("主按钮按检查结果显示检查通过、安装或更新", async () => {
    const c = await boot();
    expect(primaryText()).toBe("检查资源");
    await primary().trigger("click");
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: false,
    });
    expect(document.body.textContent).not.toContain("暂停下载");

    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      installed: true,
      pendingFiles: 1,
      totalFiles: 1,
    });
    await flushPromises();
    expect(primaryText()).toBe("更新游戏");
    c.state.game = "running";
    await nextTick();
    expect(button("game-download-primary").disabled).toBe(true);
    c.state.game = "idle";
    await nextTick();
    expect(button("game-download-primary").disabled).toBe(false);
    await primary().trigger("click");
    await flushPromises();
    expect(backend.invoke).toHaveBeenLastCalledWith("game_download_start", {
      mode: "apply",
      full: false,
    });
    expect(document.body.textContent).not.toContain("暂停下载");

    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      installed: false,
      pendingFiles: 1,
      totalFiles: 1,
    });
    await flushPromises();
    expect(primaryText()).toBe("安装游戏");

    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "ready",
      installed: true,
      pendingFiles: 0,
      totalFiles: 1,
    });
    await flushPromises();
    expect(primaryText()).toBe("检查通过");
    expect(button("game-download-primary").disabled).toBe(false);
    expect(
      button("game-download-primary").classList.contains("is-passed"),
    ).toBe(true);
    expect(document.body.textContent).not.toContain("暂停下载");

    // 安装目录改变后，旧结果不再适用。
    c.form.installRoot = "/other";
    await flushPromises();
    expect(primaryText()).toBe("检查资源");

    // 新目录开始检查时应立即显示检查中，而不是沿用旧结果。
    await primary().trigger("click");
    await flushPromises();
    expect(primaryText()).toBe("正在检查");
    expect(text("game-download-state-title")).toBe("正在检查游戏资源");
  });

  it("首次安装只显示启动器文件体积并说明后续检查", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-install",
      installed: false,
      pendingFiles: 683,
      totalFiles: 683,
      totalBytes: 274636864,
    });
    await flushPromises();
    expect(c.downloadView.value.title).toBe("需要安装启动器文件");
    expect(c.downloadView.value.detail).toBe(
      "启动器文件尚未安装, 完成后将继续检查游戏资源",
    );
    expect(c.downloadView.value.sizeText).toBe("启动器文件 262 MiB");
    expect(primaryText()).toBe("安装游戏");
    expect(document.body.textContent).toContain("启动器文件 262 MiB");
    expect(document.body.textContent).not.toContain("本次需下载 262 MiB");
  });

  it("启动时静默快速检查并在发现小版本更新时更新入口", async () => {
    backend.startStatus = {
      ...backend.downloadStatus,
      phase: "needs-update",
      installed: true,
      pendingFiles: 2,
      totalFiles: 2,
      totalBytes: 2048,
    };
    const c = await boot();
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: false,
    });
    expect(c.state.download?.phase).toBe("needs-update");
    expect(c.launchView.value.label).toBe("更新游戏");
    expect(primaryText()).toBe("更新游戏");
    expect(document.querySelector(".notification-stack")).toBeNull();
  });

  it("未安装游戏时启动不执行资源检查", async () => {
    backend.data = bootstrap({ gameExists: false });
    const c = await boot();
    expect(backend.invoke).not.toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: false,
    });
    expect(c.state.download?.phase ?? "idle").toBe("idle");
  });

  it("启动时的快速检查失败保持静默", async () => {
    backend.startFail = true;
    const c = await boot();
    expect(c.state.download?.phase ?? "idle").toBe("idle");
    expect(document.querySelector(".notification-stack")).toBeNull();
  });

  it("完整校验发现损坏后，主按钮继续提交完整校验", async () => {
    const c = await boot();
    await wrapper!
      .findAll("#page-update button")
      .find((item) => item.text() === "完整校验")!
      .trigger("click");
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "check",
      full: true,
    });
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      full: true,
      pendingFiles: 1,
      totalFiles: 1,
    });
    await nextTick();
    expect(primaryText()).toBe("更新游戏");
    await primary().trigger("click");
    await flushPromises();
    expect(backend.invoke).toHaveBeenLastCalledWith("game_download_start", {
      mode: "apply",
      full: true,
    });
    expect(c.state.download?.phase).toBe("checking");
  });

  it("未知大文件显示待验证提示，并阻止首页直接启动游戏", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-verify",
      unverifiedFiles: 2,
    });
    await nextTick();
    expect(c.downloadView.value.title).toBe("存在尚未验证的游戏资源");
    expect(c.downloadView.value.detail).toContain("2 个文件尚未验证");
    expect(c.launchView.value.label).toBe("更新游戏");
    c.handleLaunchAction();
    expect(c.state.page).toBe("update");
    expect(backend.invoke).not.toHaveBeenCalledWith("launch_official");
  });

  it("检查通过后主按钮显示通过状态", async () => {
    await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "ready",
      full: true,
    });
    await flushPromises();
    expect(primaryText()).toBe("检查通过");
    expect(button("game-download-primary").disabled).toBe(false);
    expect(
      button("game-download-primary").classList.contains("is-passed"),
    ).toBe(true);
  });

  it("下载进度始终按体积推进, 完成后才显示 100%", async () => {
    const c = await boot();
    const status: DownloadStatus = {
      ...backend.downloadStatus,
      phase: "game",
      completedFiles: 1,
      totalFiles: 2,
      totalBytes: 100,
      downloadedBytes: 100,
      stage: "patch",
    };
    backend.downloadHandler(status);
    expect(c.downloadView.value.progress).toBe(99);
    backend.downloadHandler({ ...status, downloadedBytes: 101 });
    expect(c.downloadView.value.progress).toBe(99);
    backend.downloadHandler({
      ...status,
      downloadedBytes: 80,
      stage: "download",
    });
    expect(c.downloadView.value.progress).toBe(80);
    backend.downloadHandler({
      ...status,
      downloadedBytes: 150,
      stage: "download",
    });
    expect(c.downloadView.value.progress).toBe(99);
    backend.downloadHandler({ ...status, completedFiles: 2, stage: "install" });
    expect(c.downloadView.value.progress).toBe(99);
    backend.downloadHandler({
      ...status,
      phase: "ready",
      completedFiles: 2,
      stage: "",
    });
    expect(c.downloadView.value.progress).toBe(100);
  });

  it("暂停后仍按已下载体积显示进度", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "cancelled",
      completedFiles: 192,
      totalFiles: 334,
      totalBytes: Math.round(76.6 * 1024 ** 3),
      downloadedBytes: 579 * 1024 ** 2,
      stage: "install",
    });
    expect(c.downloadView.value.progress).toBe(1);
  });

  it("未完成时文件数回退也不会显示 100%", async () => {
    const c = await boot();
    backend.downloadHandler({
      ...backend.downloadStatus,
      phase: "needs-update",
      totalBytes: 0,
      downloadedBytes: 0,
      completedFiles: 3,
      totalFiles: 3,
    });
    expect(c.downloadView.value.progress).toBe(99);
  });

  it.each([
    [
      check({
        missing: 2,
        changed: 1,
        missingPreview: ["a"],
        changedPreview: ["b"],
      }),
      "3 个文件待更新",
      false,
    ],
    [
      check({
        changed: 1,
        changedPreview: ["QtQuick/Controls/Styles/Flat/plugins.qmltypes"],
      }),
      "已是最新",
      true,
    ],
    [check({ files: 0, localVersion: "" }), "已是最新", true],
  ])("展示缺失、变更和已知例外", async (result, expected, hidden) => {
    const c = await boot();
    backend.result = result;
    await c.checkUpdate(true);
    await nextTick();
    expect(text("update-state-title")).toBe(expected);
    expect(button("update-files").hidden).toBe(hidden);
    if (!hidden) {
      expect(wrapper!.findAll(".update-file")).toHaveLength(2);
      expect(wrapper!.find(".update-file__more").text()).toBe(
        "另有 1 个文件未展开",
      );
      expect(text("home-status-text")).toBe("3 个文件待更新");
    }
  });

  it("启动器文件待更新时提供更新入口并在完成后刷新", async () => {
    const c = await boot();
    backend.result = check({
      changed: 4,
      changedPreview: [
        "/NTEBrowser.exe",
        "/NTEUpdate.exe",
        "/NTEWebBooster.exe",
        "/ResData/1289.dat",
      ],
    });
    await c.checkUpdate(true);
    await nextTick();
    const entry = wrapper!
      .findAll("#page-update button")
      .find((item) => item.text() === "更新启动器文件");
    expect(entry).toBeTruthy();
    c.state.game = "running";
    await nextTick();
    expect(entry!.attributes("disabled")).toBeDefined();
    c.state.game = "idle";
    c.runtimeEnvironmentBusy.value = true;
    await nextTick();
    expect(entry!.attributes("disabled")).toBeDefined();
    c.runtimeEnvironmentBusy.value = false;
    await nextTick();
    await entry!.trigger("click");
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("game_download_start", {
      mode: "launcher",
      full: false,
    });

    backend.result = check();
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "launcher",
      phase: "ready",
      pendingFiles: 0,
    });
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("check_update", {
      full: false,
    });
    await nextTick();
    expect(text("update-state-title")).toBe("已是最新");
    expect(
      wrapper!
        .findAll("#page-update button")
        .some((item) => item.text() === "更新启动器文件"),
    ).toBe(false);

    // 取消或失败后必须重新完整校验, 否则同尺寸损坏会被快速检查误判为最新.
    backend.downloadHandler({
      ...backend.downloadStatus,
      mode: "launcher",
      phase: "cancelled",
    });
    await flushPromises();
    expect(backend.invoke).toHaveBeenLastCalledWith("check_update", {
      full: true,
    });
  });

  it("未安装启动器时显示尚未安装, 目录变化后旧结果失效", async () => {
    const c = await boot();
    backend.data = bootstrap({
      ok: false,
      error: "所选安装目录尚未安装启动器文件",
      launcherRoot: "",
      launcherVersion: "",
    });
    await c.redetect();
    expect(c.updateView.value.title).toBe("尚未安装");

    backend.result = check({
      installed: false,
      installRoot: "/game",
      files: 0,
      localVersion: "",
    });
    await c.checkUpdate(true);
    await nextTick();
    expect(c.updateView.value.title).toBe("尚未安装");
    expect(c.updateView.value.showFiles).toBe(false);

    // 已安装状态下的旧结果也不能沿用到新目录.
    backend.data = bootstrap();
    await c.redetect();
    backend.result = check({ installRoot: "/game" });
    await c.checkUpdate(true);
    await nextTick();
    expect(c.updateView.value.title).toBe("已是最新");
    c.form.installRoot = "/other";
    await nextTick();
    expect(c.updateView.value.title).toBe("尚未检查");
  });

  it("检查期间阻止重复请求, 失败后恢复按钮并按需提示", async () => {
    const c = await boot();
    let reject!: (error: Error) => void;
    backend.invoke.mockReturnValueOnce(
      new Promise((_resolve, fail) => {
        reject = fail;
      }),
    );
    const pending = c.checkUpdate();
    await nextTick();
    expect(button("update-quick").disabled).toBe(true);
    backend.invoke.mockClear();
    await c.checkUpdate();
    expect(backend.invoke).not.toHaveBeenCalled();
    reject(new Error("CDN offline"));
    await pending;
    expect(text("update-state-title")).toBe("检查失败");
    expect(text("home-status-text")).toBe("检查失败，打开更新页");
    expect(noticeMessage()).toBe("CDN offline");
    expect(button("update-quick").disabled).toBe(false);
    backend.invoke.mockRejectedValueOnce("silent error");
    await c.checkUpdate(false, true);
    expect(noticeMessage()).toBe("CDN offline");
  });
});

describe("游戏会话", () => {
  it.each([5 * 60_000 - 1, 5 * 60_000])(
    "退出记录距今 %i 毫秒时遵守五分钟提示边界",
    async (age) => {
      await boot();
      const endedAt = Date.now() - age;
      backend.playtimeHistory = {
        sessions: [
          {
            startedAt: endedAt - 60_000,
            endedAt,
            mode: "background",
            distribution: "cn",
          },
        ],
        active: null,
      };
      backend.handler(session({ running: true, pid: 123 }));
      backend.handler(session());
      await flushPromises();
      expect(noticeMessage()).toBe(
        age < 5 * 60_000 ? "游戏已退出 · 本次游玩 1 分钟" : "游戏已退出。",
      );
    },
  );

  it("读取游玩记录失败时仍提示游戏退出", async () => {
    await boot();
    backend.handler(session({ running: true, pid: 123 }));
    const original = backend.invoke.getMockImplementation()!;
    backend.invoke.mockImplementation((command, args) =>
      command === "playtime_history"
        ? Promise.reject(new Error("history offline"))
        : original(command, args),
    );
    backend.handler(session());
    await flushPromises();
    expect(noticeMessage()).toBe("游戏已退出。");
  });

  it("记录页打开时启动游戏会刷新活动会话", async () => {
    const c = await boot();
    backend.playtimeHistory = { sessions: [], active: null };
    c.showPage("records");
    await flushPromises();
    const reads = () =>
      backend.invoke.mock.calls.filter(
        ([command]) => command === "playtime_history",
      ).length;
    expect(reads()).toBe(1);
    const active = {
      startedAt: Date.now(),
      lastSeenAt: Date.now(),
      mode: "background",
      distribution: "cn" as const,
    };
    backend.playtimeHistory = { sessions: [], active };
    backend.handler(session({ running: true, pid: 123 }));
    await flushPromises();
    expect(reads()).toBe(2);
    expect(c.playtime.history).not.toBeNull();
    expect(c.playtime.history?.active).toEqual(active);
  });

  it("running 先于会话创建时, 会话创建后补读记录", async () => {
    const c = await boot();
    backend.playtimeHistory = { sessions: [], active: null };
    c.showPage("records");
    await flushPromises();
    const reads = () =>
      backend.invoke.mock.calls.filter(
        ([command]) => command === "playtime_history",
      ).length;
    expect(reads()).toBe(1);
    // 游戏先进入 running, 后端尚未创建活动会话。
    backend.handler(session({ running: true, pid: 123 }));
    await flushPromises();
    expect(c.playtime.history?.active).toBeNull();
    const before = reads();
    const active = {
      startedAt: Date.now(),
      lastSeenAt: Date.now(),
      mode: "background",
      distribution: "cn" as const,
    };
    backend.playtimeHistory = { sessions: [], active };
    backend.playtimeStarted();
    await flushPromises();
    expect(reads()).toBe(before + 1);
    expect(c.playtime.history?.active).toEqual(active);
  });

  it("退出提示选择最后结束的会话并可进入记录页", async () => {
    const c = await boot();
    const endedAt = Date.now() - 1000;
    backend.playtimeHistory = {
      sessions: [
        {
          startedAt: endedAt - 3 * 3_600_000,
          endedAt,
          mode: "background",
          distribution: "cn",
        },
        {
          startedAt: endedAt - 2 * 3_600_000,
          endedAt: endedAt - 3_600_000,
          mode: "official",
          distribution: "cn",
        },
      ],
      active: null,
    };
    backend.handler(session({ running: true, pid: 123 }));
    backend.handler(session());
    await flushPromises();
    expect(noticeMessage()).toBe("游戏已退出 · 本次游玩 3 小时");
    const notice = c.notifications.find((item) =>
      item.message.includes("本次游玩"),
    )!;
    expect(notice.secondary?.label).toBe("查看记录");
    notice.secondary?.run();
    await flushPromises();
    expect(c.state.page).toBe("records");
    expect(window.location.hash).toBe("#records");
  });

  it("启动反馈期间忽略事件和重复操作, 启动成功后收起窗口", async () => {
    // 前台启动没有暂停入口, 反馈期间的重复点击仍应被忽略。
    backend.data.settings.games.nte!.launchMode = "official";
    const c = await boot();
    button("launch-btn").click();
    expect(c.state.game).toBe("launching");
    backend.handler(session());
    button("launch-btn").click();
    window.dispatchEvent(new Event("focus"));
    expect(c.state.game).toBe("launching");
    await vi.advanceTimersByTimeAsync(900);
    expect(c.state.game).toBe("running");
    expect(backend.invoke).toHaveBeenCalledWith("win_hide");
    backend.handler(session());
    expect(c.state.game).toBe("idle");
    await nextTick();
    await flushPromises();
    expect(noticeMessage()).toBe("游戏已退出。");
  });

  it("后台启动时主按钮展示阶段并允许整键暂停", async () => {
    const c = await boot();
    const startedAt = Date.now();
    backend.invoke.mockResolvedValueOnce({ pid: 123, mode: "background" });
    c.handleLaunchAction();
    backend.handler(
      session({
        running: false,
        launch: { stage: "preparing", startedAt, cancelling: false },
      }),
    );
    await flushPromises();
    expect(c.state.game).toBe("launching");
    expect(c.state.launch?.stage).toBe("preparing");
    await nextTick();
    expect(button("launch-btn").getAttribute("aria-label")).toBe(
      "正在准备环境，暂停启动",
    );
    expect(button("launch-btn").getAttribute("aria-disabled")).toBeNull();
    expect(document.getElementById("launch-pause-btn")).toBeNull();
    backend.handler(
      session({
        running: true,
        pid: 123,
        launch: { stage: "resources", startedAt, cancelling: false },
      }),
    );
    await flushPromises();
    expect(c.state.game).toBe("launching");
    expect(c.state.launch?.stage).toBe("resources");
    expect(backend.invoke).not.toHaveBeenCalledWith("win_hide");
    expect(button("launch-btn").getAttribute("aria-label")).toBe(
      "正在等待资源，暂停启动",
    );
    expect(button("launch-btn").getAttribute("aria-disabled")).toBeNull();
  });

  it.each(["official", "direct"])("%s 启动不提供暂停入口", async (mode) => {
    backend.data.settings.games.nte!.launchMode = mode;
    backend.data.settings.games.nte!.gameAccountId = "synthetic-account";
    const c = await boot();
    button("launch-btn").click();
    await nextTick();
    expect(c.state.game).toBe("launching");
    expect(c.launchView.value.pauseAvailable).toBe(false);
    expect(c.launchView.value.label).toBe("正在启动");
    expect(document.getElementById("launch-pause-btn")).toBeNull();
    expect(button("launch-btn").getAttribute("aria-disabled")).toBe("true");
    button("launch-btn").click();
    await c.pauseLaunch();
    c.handleLaunchAction();
    await flushPromises();
    expect(backend.invoke).not.toHaveBeenCalledWith("cancel_launch");
  });

  it("后台阶段完成时收起窗口并只隐藏一次", async () => {
    const c = await boot();
    backend.invoke.mockResolvedValueOnce({ pid: 123, mode: "background" });
    c.handleLaunchAction();
    backend.handler(
      session({
        running: true,
        pid: 123,
        launch: {
          stage: "running",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await flushPromises();
    expect(c.state.game).toBe("running");
    expect(c.state.launch).toBeNull();
    expect(
      backend.invoke.mock.calls.filter(([command]) => command === "win_hide"),
    ).toHaveLength(1);
  });

  it("关闭游戏时保留关闭反馈, 不被已完成阶段覆盖", async () => {
    const c = await boot();
    backend.handler(
      session({
        running: true,
        pid: 123,
        launch: {
          stage: "running",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await flushPromises();
    expect(c.state.game).toBe("running");
    backend.handler(
      session({
        running: true,
        closing: true,
        pid: 123,
        launch: {
          stage: "running",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await flushPromises();
    expect(c.state.game).toBe("closing");
    expect(button("launch-btn").getAttribute("aria-disabled")).toBe("true");
  });

  it("恢复启动中状态时主按钮展示阶段并允许暂停", async () => {
    backend.status = session({
      running: true,
      pid: 123,
      launch: {
        stage: "helper",
        startedAt: Date.now() - 5000,
        cancelling: false,
      },
    });
    const c = await boot();
    expect(c.state.game).toBe("launching");
    expect(c.state.launch?.stage).toBe("helper");
    expect(button("launch-btn").getAttribute("aria-label")).toBe(
      "正在启动后台，暂停启动",
    );
    expect(button("launch-btn").getAttribute("aria-disabled")).toBeNull();
    expect(document.getElementById("launch-pause-btn")).toBeNull();
  });

  it("后台启动失败时清除阶段", async () => {
    const c = await boot();
    backend.invoke.mockResolvedValueOnce({ pid: 123, mode: "background" });
    c.handleLaunchAction();
    backend.handler(
      session({
        running: true,
        pid: 123,
        launch: {
          stage: "resources",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await flushPromises();
    expect(c.state.game).toBe("launching");
    backend.handler(
      session({ error: "官方后台未能在 180 秒内完成资源初始化." }),
    );
    await flushPromises();
    expect(c.state.game).toBe("idle");
    expect(c.state.launch).toBeNull();
    expect(noticeMessage()).toBe("官方后台未能在 180 秒内完成资源初始化.");
  });

  it("点击启动按钮暂停后台启动且不提示错误", async () => {
    const c = await boot();
    let rejectLaunch!: (error: Error) => void;
    backend.invoke.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          rejectLaunch = reject;
        }),
    );
    button("launch-btn").click();
    backend.handler(
      session({
        running: false,
        launch: {
          stage: "preparing",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await nextTick();
    backend.invoke.mockResolvedValueOnce(
      session({
        running: false,
        launch: {
          stage: "preparing",
          startedAt: Date.now(),
          cancelling: true,
        },
      }),
    );
    button("launch-btn").click();
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("cancel_launch");
    expect(c.state.game).toBe("launching");
    expect(c.state.launch?.cancelling).toBe(true);
    expect(button("launch-btn").getAttribute("aria-label")).toBe("正在暂停");
    expect(button("launch-btn").getAttribute("aria-disabled")).toBe("true");
    rejectLaunch(new Error("启动已暂停。"));
    await flushPromises();
    expect(c.state.game).toBe("idle");
    expect(noticeMessage()).toBeUndefined();
  });

  it("首个阶段事件到达前点击暂停也会立即反馈", async () => {
    const c = await boot();
    let resolveLaunch!: (value: { pid: number; mode: string }) => void;
    backend.invoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveLaunch = resolve;
        }),
    );
    button("launch-btn").click();
    await nextTick();
    expect(c.state.game).toBe("launching");
    expect(c.state.launch).toBeNull();
    // 启动命令尚未返回、还没有阶段事件, 此时点击暂停也要立即更新按钮。
    button("launch-btn").click();
    await nextTick();
    expect(button("launch-btn").getAttribute("aria-label")).toBe("正在暂停");
    expect(button("launch-btn").getAttribute("aria-disabled")).toBe("true");
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("cancel_launch");
    resolveLaunch({ pid: 123, mode: "background" });
    await flushPromises();
    expect(c.state.launch?.cancelling).toBe(true);
    expect(button("launch-btn").getAttribute("aria-label")).toBe("正在暂停");
  });

  it("手柄确认键在启动中通过启动区暂停", async () => {
    const c = await boot();
    backend.invoke.mockResolvedValueOnce({ pid: 123, mode: "background" });
    button("launch-btn").click();
    backend.handler(
      session({
        running: false,
        launch: {
          stage: "preparing",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await flushPromises();
    await nextTick();
    expect(c.state.game).toBe("launching");
    // 手柄确认键作用于选中的启动区, 通过启动区转发到主按钮。
    document
      .getElementById("launch-region")!
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    await nextTick();
    expect(button("launch-btn").getAttribute("aria-label")).toBe("正在暂停");
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("cancel_launch");
  });

  it("启动区只转发自身收到的确认键", async () => {
    const c = await boot();
    document
      .getElementById("launch-btn")!
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    await nextTick();
    // 按钮自身的按键激活由浏览器默认行为触发, 启动区不重复处理。
    expect(c.state.game).toBe("idle");
    expect(backend.invoke).not.toHaveBeenCalledWith("launch_official");
  });

  it("手柄西键在启动中暂停启动", async () => {
    const c = await boot();
    backend.invoke.mockResolvedValueOnce({ pid: 123, mode: "background" });
    button("launch-btn").click();
    backend.handler(
      session({
        running: true,
        pid: 123,
        launch: {
          stage: "helper",
          startedAt: Date.now(),
          cancelling: false,
        },
      }),
    );
    await flushPromises();
    backend.invoke.mockResolvedValueOnce(session());
    c.handleLaunchAction();
    await flushPromises();
    expect(backend.invoke).toHaveBeenCalledWith("cancel_launch");
    expect(c.state.game).toBe("idle");
  });

  it("游戏退出后读取本次时长并提示查看记录", async () => {
    const c = await boot();
    const endedAt = Date.now() - 1_000;
    backend.playtimeHistory = {
      sessions: [
        {
          startedAt: endedAt - 2 * 60 * 60_000,
          endedAt,
          mode: "background",
          distribution: "cn",
        },
      ],
      active: null,
    };
    backend.handler(session({ running: true, pid: 123 }));
    backend.handler(session());
    await flushPromises();
    expect(c.state.game).toBe("idle");
    expect(c.playtime.history?.sessions).toHaveLength(1);
    expect(noticeMessage()).toBe("游戏已退出 · 本次游玩 2 小时");
  });

  it("忽略本次启动之前结束的旧会话", async () => {
    const c = await boot();
    c.handleLaunchAction();
    await vi.advanceTimersByTimeAsync(900);
    await flushPromises();
    expect(c.state.game).toBe("running");
    const endedAt = Date.now() - 60_000;
    backend.playtimeHistory = {
      sessions: [
        {
          startedAt: endedAt - 2 * 60 * 60_000,
          endedAt,
          mode: "background",
          distribution: "cn",
        },
      ],
      active: null,
    };
    backend.handler(session());
    await flushPromises();
    expect(noticeMessage()).toBe("游戏已退出。");
  });

  it("dispose 后不再发送退出提示", async () => {
    const c = await boot();
    backend.handler(session({ running: true, pid: 123 }));
    const original = backend.invoke.getMockImplementation()!;
    let resolveHistory!: (history: PlaytimeHistory) => void;
    const pending = new Promise<PlaytimeHistory>((resolve) => {
      resolveHistory = resolve;
    });
    backend.invoke.mockImplementation((command, args) =>
      command === "playtime_history" ? pending : original(command, args),
    );
    backend.handler(session());
    c.dispose();
    resolveHistory({
      sessions: [
        {
          startedAt: Date.now() - 2 * 60 * 60_000,
          endedAt: Date.now() - 1_000,
          mode: "background",
          distribution: "cn",
        },
      ],
      active: null,
    });
    await flushPromises();
    expect(noticeMessage()).toBeUndefined();
  });

  it("进程在反馈期间退出时不收起窗口", async () => {
    const c = await boot();
    c.handleLaunchAction();
    await flushPromises();
    backend.status = session();
    await vi.advanceTimersByTimeAsync(900);
    expect(c.state.game).toBe("idle");
    expect(noticeMessage()).toBe("游戏进程已退出，启动未完成。");
    expect(backend.invoke).not.toHaveBeenCalledWith("win_hide");
  });

  it("启动命令失败时用后端状态恢复按钮", async () => {
    const c = await boot();
    backend.invoke.mockRejectedValueOnce(new Error("spawn failed"));
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.game).toBe("idle");
    expect(noticeMessage()).toBe("spawn failed");
    backend.status = session({ running: true });
    backend.invoke.mockRejectedValueOnce("spawn uncertain");
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.game).toBe("running");
    expect(noticeMessage()).toBe("spawn uncertain");
  });

  it("启动状态查询或隐藏窗口失败仍保持可用的关闭按钮", async () => {
    const c = await boot();
    backend.invoke
      .mockResolvedValueOnce({ pid: 123 })
      .mockRejectedValueOnce(new Error("status failed"))
      .mockRejectedValueOnce(new Error("hide failed"));
    c.handleLaunchAction();
    await vi.advanceTimersByTimeAsync(900);
    expect(c.state.game).toBe("running");
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.game).toBe("idle");
    expect(noticeMessage()).toBe("游戏已关闭。");
  });

  it("关闭命令失败时同步仍运行、已退出和未知状态", async () => {
    const c = await boot();
    backend.status = session({ running: true });
    backend.handler(backend.status);
    backend.invoke.mockRejectedValueOnce(new Error("close failed"));
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.game).toBe("running");
    expect(noticeMessage()).toBe("close failed");
    backend.invoke
      .mockRejectedValueOnce("close unknown")
      .mockRejectedValueOnce(new Error("status failed"));
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.game).toBe("running");
    backend.status = session();
    backend.invoke.mockRejectedValueOnce("already closed");
    c.handleLaunchAction();
    await flushPromises();
    expect(c.state.game).toBe("idle");
  });

  it("托盘恢复同步运行和关闭状态, 查询失败不破坏按钮", async () => {
    const c = await boot();
    backend.status = session({ running: true, closing: true });
    window.dispatchEvent(new Event("focus"));
    await flushPromises();
    expect(c.state.game).toBe("closing");
    window.dispatchEvent(new Event("focus"));
    c.state.game = "idle";
    backend.invoke.mockRejectedValueOnce(new Error("probe failed"));
    window.dispatchEvent(new Event("focus"));
    await flushPromises();
    expect(c.state.game).toBe("idle");
  });

  it("浏览器预览不调用原生窗口命令", async () => {
    backend.native = false;
    const c = await boot();
    c.handleLaunchAction();
    await nextTick();
    expect(noticeMessage()).toBe("浏览器预览：模拟启动流程。");
    await vi.advanceTimersByTimeAsync(900);
    expect(c.state.game).toBe("running");
    expect(backend.invoke).not.toHaveBeenCalledWith("win_hide");
  });
});

describe("导航和窗口交互", () => {
  it("导航校验、深链接、快捷键和表单提交", async () => {
    history.replaceState(null, "", "/#settings");
    const c = await boot();
    expect(c.state.page).toBe("settings");
    c.showPage(undefined);
    c.showPage("invalid");
    expect(c.state.page).toBe("settings");
    key("1", { ctrlKey: true });
    await nextTick();
    expect(wrapper!.find("#page-home").classes()).toContain("is-active");
    key("2", { ctrlKey: true, altKey: true });
    key("2", { ctrlKey: true, metaKey: true });
    key("0", { ctrlKey: true });
    expect(c.state.page).toBe("home");
    key("3", { ctrlKey: true });
    expect(c.state.page).toBe("game-mode");
    key("5", { ctrlKey: true });
    expect(c.state.page).toBe("records");
    key("6", { ctrlKey: true });
    expect(c.state.page).toBe("settings");
    backend.invoke.mockClear();
    key("s", { ctrlKey: true });
    await flushPromises();
    expect(backend.invoke).not.toHaveBeenCalledWith(
      "save_settings",
      expect.anything(),
    );
    history.replaceState(null, "", "/#bulletin");
    window.dispatchEvent(new Event("hashchange"));
    expect(c.state.page).toBe("bulletin");
    vi.spyOn(history, "replaceState").mockImplementation(() => {
      throw new Error("unsupported protocol");
    });
    expect(() => c.showPage("update")).not.toThrow();
  });

  it("保留文本输入行为并阻止页面拖拽和上下文菜单", async () => {
    const c = await boot();
    c.openSettings("game");
    await nextTick();
    for (const type of [
      "dragstart",
      "dragover",
      "drop",
      "selectstart",
      "contextmenu",
    ]) {
      const event = new Event(type, { bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
    }
    for (const type of ["selectstart", "contextmenu"]) {
      const event = new Event(type, { bubbles: true, cancelable: true });
      document.getElementById("set-install")!.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
    expect(key("Tab")).toBe(false);
    expect(key("a")).toBe(true);
  });

  it("窗口按钮、标题栏拖动、外链及原生命令错误", async () => {
    const c = await boot();
    await c.windowAction("minimize");
    await c.windowAction("maximize");
    await c.windowAction("close");
    expect(backend.invoke).toHaveBeenCalledWith("win_minimize");
    expect(backend.invoke).toHaveBeenCalledWith("win_toggle_maximize");
    expect(backend.invoke).toHaveBeenCalledWith("win_close");
    backend.invoke.mockRejectedValueOnce(new Error("window failed"));
    await c.windowAction("close");
    expect(noticeMessage()).toBe("window failed");
    await c.openUrl("https://example.com");
    expect(backend.invoke).toHaveBeenCalledWith("open_url", {
      url: "https://example.com",
    });
    expect(noticeMessage()).toBe("已在浏览器打开。");
    backend.invoke.mockRejectedValueOnce("cannot open");
    await c.openUrl("https://example.com");
    expect(noticeMessage()).toBe("cannot open");
    backend.native = false;
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    await c.openUrl("https://example.com");
    expect(open).toHaveBeenCalledWith(
      "https://example.com",
      "_blank",
      "noopener,noreferrer",
    );
    expect(noticeMessage()).toBe("预览模式：已在新标签页打开。");
    backend.invoke.mockClear();
    await c.windowAction("minimize");
    expect(backend.invoke).not.toHaveBeenCalled();
  });
});

describe("主视觉和资源", () => {
  it("保持循环间隙的画面, 离页暂停并保留当前帧", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    const canvas = document.getElementById("hero-canvas") as HTMLCanvasElement;
    const draw = vi.mocked(canvas.getContext("2d")!.drawImage);
    expect(video.src).toContain("assets/hero-loop.mp4");
    expect(video.play).toHaveBeenCalled();
    expect(canvas.classList.contains("is-visible")).toBe(false);
    expect([canvas.width, canvas.height]).toEqual([0, 0]);
    Object.defineProperties(video, {
      readyState: { value: 2, writable: true },
      paused: { value: false, writable: true },
      seeking: { value: false, writable: true },
      duration: { value: 13 },
      videoWidth: { value: 1920 },
      videoHeight: { value: 1080 },
    });
    Object.defineProperties(canvas, {
      clientWidth: { value: 800 },
      clientHeight: { value: 600 },
    });
    resizeElement(canvas, 800, 600);
    await vi.advanceTimersByTimeAsync(40);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    expect(draw).toHaveBeenLastCalledWith(
      video,
      expect.closeTo(259.2),
      0,
      1440,
      1080,
      0,
      0,
      800,
      600,
    );
    vi.stubGlobal("devicePixelRatio", 4);
    await vi.advanceTimersByTimeAsync(40);
    expect(canvas.width).toBe(1440);
    expect(canvas.height).toBe(1080);
    vi.stubGlobal("devicePixelRatio", 1);
    await vi.advanceTimersByTimeAsync(40);
    video.currentTime = 12;
    backend.invoke.mockClear();
    await vi.advanceTimersByTimeAsync(40);
    expect(video.currentTime).toBe(0);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    Object.defineProperty(video, "seeking", { value: true });
    draw.mockClear();
    await vi.advanceTimersByTimeAsync(100);
    expect(draw).not.toHaveBeenCalled();
    expect(canvas.classList.contains("is-visible")).toBe(true);
    Object.defineProperty(video, "seeking", { value: false });
    Object.defineProperty(video, "readyState", { value: 1 });
    await vi.advanceTimersByTimeAsync(100);
    expect(draw).not.toHaveBeenCalled();
    expect(canvas.classList.contains("is-visible")).toBe(true);
    Object.defineProperty(video, "readyState", { value: 2 });
    video.currentTime = 6;
    c.showPage("settings");
    await nextTick();
    expect(c.media.looping).toBe(false);
    expect(video.currentTime).toBe(6);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    await vi.advanceTimersByTimeAsync(100);
    expect(draw).not.toHaveBeenCalled();
    Object.defineProperty(video, "seeking", { value: true });
    c.showPage("home");
    await flushPromises();
    await vi.advanceTimersByTimeAsync(40);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    Object.defineProperty(video, "seeking", { value: false });
    await vi.advanceTimersByTimeAsync(40);
    expect(video.currentTime).toBe(6);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    c.showPage("settings");
    c.applyHeroMedia("photo");
    await nextTick();
    expect(video.hasAttribute("src")).toBe(false);
    expect(document.getElementById("hero-video-b")!.hasAttribute("src")).toBe(
      false,
    );
    expect(canvas.classList.contains("is-visible")).toBe(false);
    video.currentTime = 0;
    c.applyHeroMedia("video");
    await flushPromises();
    video.dispatchEvent(new Event("loadedmetadata"));
    expect(video.currentTime).toBe(6);
    c.applyReduceMotion(true);
    c.applyHeroMedia("video");
    await nextTick();
    expect(canvas.classList.contains("is-visible")).toBe(false);
    expect(
      vi
        .mocked(fetch)
        .mock.calls.filter(([url]) => String(url).includes("hero-loop")),
    ).toHaveLength(0);
  });

  it("窗口隐藏时停止解码, 恢复可见后从头播放", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    video.currentTime = 6;
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    const visibility = vi
      .spyOn(document, "visibilityState", "get")
      .mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    await nextTick();
    expect(c.media.looping).toBe(false);
    expect(video.currentTime).toBe(0);
    expect(video.hasAttribute("src")).toBe(false);
    visibility.mockReturnValue("visible");
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    await flushPromises();
    expect(c.media.looping).toBe(true);
    expect(video.currentTime).toBe(0);
  });

  it("游戏会话期间停止解码, 游戏退出后恢复播放", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    expect(c.media.looping).toBe(true);
    video.currentTime = 6;

    backend.handler(session({ running: true, pid: 123 }));
    await nextTick();
    await flushPromises();
    expect(c.state.game).toBe("running");
    expect(c.media.looping).toBe(false);
    expect(video.currentTime).toBe(6);
    expect(video.getAttribute("src")).toContain("assets/hero-loop.mp4");

    backend.handler(session());
    await nextTick();
    await flushPromises();
    expect(c.state.game).toBe("idle");
    expect(c.media.looping).toBe(true);
    expect(video.currentTime).toBe(6);
    expect(video.getAttribute("src")).toContain("assets/hero-loop.mp4");
  });

  it("游戏期间隐藏窗口仍保留播放位置", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
      pauseHeroOnBlur: false,
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    video.currentTime = 6;

    backend.handler(session({ running: true, pid: 123 }));
    await nextTick();
    await flushPromises();
    backend.visible = false;
    await backend.resize();
    await nextTick();
    await flushPromises();
    expect(video.hasAttribute("src")).toBe(false);

    backend.handler(session());
    await nextTick();
    await flushPromises();
    expect(c.state.game).toBe("idle");

    backend.visible = true;
    await backend.resize();
    await nextTick();
    await flushPromises();
    video.dispatchEvent(new Event("loadedmetadata"));
    expect(c.media.looping).toBe(true);
    expect(video.currentTime).toBe(6);
  });

  it("失焦暂停设置控制窗口失焦时的视频解码", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
      pauseHeroOnBlur: false,
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    video.currentTime = 6;
    backend.focused = false;
    await backend.resize();
    await nextTick();
    await flushPromises();
    expect(c.media.looping).toBe(true);
    expect(video.getAttribute("src")).toContain("assets/hero-loop.mp4");

    c.form.pauseHeroOnBlur = true;
    c.applyHeroMedia("video");
    await nextTick();
    await flushPromises();
    expect(c.media.looping).toBe(false);
    expect(video.currentTime).toBe(6);
    expect(video.getAttribute("src")).toContain("assets/hero-loop.mp4");

    backend.focused = true;
    await backend.resize();
    await nextTick();
    await flushPromises();
    expect(c.media.looping).toBe(true);
    expect(video.currentTime).toBe(6);
    expect(video.getAttribute("src")).toContain("assets/hero-loop.mp4");
  });

  it("迟到的焦点快照不覆盖新焦点", async () => {
    const c = await boot();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    backend.isMaximized.mockImplementationOnce(async () => {
      await gate;
      return false;
    });
    backend.focused = false;
    const stale = backend.resize();
    backend.focused = true;
    window.dispatchEvent(new Event("focus"));
    release();
    await stale;
    await flushPromises();
    expect(c.ui.windowFocused).toBe(true);
  });

  it("浏览器预览跟随窗口焦点暂停", async () => {
    backend.native = false;
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
      pauseHeroOnBlur: true,
    };
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const c = await boot();
    expect(c.ui.windowFocused).toBe(true);
    expect(c.media.looping).toBe(true);

    window.dispatchEvent(new Event("blur"));
    await nextTick();
    await flushPromises();
    expect(c.ui.windowFocused).toBe(false);
    expect(c.media.looping).toBe(false);

    window.dispatchEvent(new Event("focus"));
    await nextTick();
    await flushPromises();
    expect(c.ui.windowFocused).toBe(true);
    expect(c.media.looping).toBe(true);
  });

  it("视频播放或原生窗口接口失败时仍显示主页", async () => {
    vi.mocked(HTMLMediaElement.prototype.play).mockRejectedValue(
      new DOMException("video unavailable", "NotSupportedError"),
    );
    backend.isMaximized.mockRejectedValue(new Error("window unavailable"));
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    expect(c.state.bootstrap?.ok).toBe(true);
    expect(button("app").classList.contains("is-intro-done")).toBe(true);
    expect(button("hero-canvas").classList.contains("is-visible")).toBe(false);
  });

  it("不支持视频编码时保留照片", async () => {
    vi.mocked(HTMLMediaElement.prototype.canPlayType).mockReturnValue("");
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    await boot();
    expect(button("hero-canvas").classList.contains("is-visible")).toBe(false);
    expect(fetch).not.toHaveBeenCalled();
  });

  it("预算退到静帧档时暂停解码并保留当前帧, 恢复后续播", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    const canvas = document.getElementById("hero-canvas") as HTMLCanvasElement;
    Object.defineProperties(video, {
      readyState: { value: 2, writable: true },
      paused: { value: false, writable: true },
      seeking: { value: false, writable: true },
      duration: { value: 13 },
      videoWidth: { value: 1920 },
      videoHeight: { value: 1080 },
      currentTime: { value: 5, writable: true },
    });
    Object.defineProperties(canvas, {
      clientWidth: { value: 800 },
      clientHeight: { value: 600 },
    });
    resizeElement(canvas, 800, 600);
    await vi.advanceTimersByTimeAsync(40);
    expect(canvas.classList.contains("is-visible")).toBe(true);

    vi.mocked(HTMLMediaElement.prototype.pause).mockClear();
    vi.mocked(HTMLMediaElement.prototype.play).mockClear();
    c.setHeroRenderTier(3);
    expect(c.media.renderTier).toBe(3);
    expect(c.media.looping).toBe(false);
    expect(video.pause).toHaveBeenCalled();
    // 静帧保留在当前帧, 不回跳到首帧。
    expect(video.currentTime).toBe(5);

    // 抽帧动画在静帧档停掉, 画布继续显示最后一帧。
    Object.defineProperty(video, "paused", { value: true, writable: true });
    vi.mocked(HTMLMediaElement.prototype.play).mockClear();
    c.setHeroRenderTier(2);
    expect(c.media.renderTier).toBe(2);
    expect(video.play).toHaveBeenCalled();
    expect(c.media.looping).toBe(true);
    expect(video.currentTime).toBe(5);
  });

  it("预算档位在非静帧档之间变化不触发播放请求", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    Object.defineProperties(video, {
      readyState: { value: 2, writable: true },
      paused: { value: false, writable: true },
      currentTime: { value: 3, writable: true },
    });
    vi.mocked(HTMLMediaElement.prototype.play).mockClear();
    vi.mocked(HTMLMediaElement.prototype.pause).mockClear();
    c.setHeroRenderTier(1);
    c.setHeroRenderTier(2);
    expect(c.media.renderTier).toBe(2);
    expect(video.play).not.toHaveBeenCalled();
    expect(video.pause).not.toHaveBeenCalled();
    expect(video.currentTime).toBe(3);
  });

  it("帧间隔持续变坏时降到静帧档, 暂停解码并保留当前帧", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    // 用可控帧间隔驱动预算: 预热期 200ms 一帧, 之后翻倍到 400ms。
    // 200ms 间隔下 1 秒窗口只装得下 3 个样本, 判定次数在测试里是确定的。
    const frame = { delay: 200 };
    let pending = 0;
    HTMLVideoElement.prototype.requestVideoFrameCallback = function (cb) {
      pending = window.setTimeout(
        () =>
          cb(performance.now(), { mediaTime: 0 } as VideoFrameCallbackMetadata),
        frame.delay,
      );
      return pending;
    };
    HTMLVideoElement.prototype.cancelVideoFrameCallback = (id) =>
      window.clearTimeout(id);

    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    const canvas = document.getElementById("hero-canvas") as HTMLCanvasElement;
    Object.defineProperties(video, {
      readyState: { value: 2, writable: true },
      paused: { value: false, writable: true },
      seeking: { value: false, writable: true },
      duration: { value: 13 },
      videoWidth: { value: 1920 },
      videoHeight: { value: 1080 },
      currentTime: { value: 7, writable: true },
    });
    Object.defineProperties(canvas, {
      clientWidth: { value: 800 },
      clientHeight: { value: 600 },
    });
    resizeElement(canvas, 800, 600);
    // 预热期健康: 跑满 2 秒采基线, 之后画布照常显示。
    await vi.advanceTimersByTimeAsync(2600);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    expect(c.media.renderTier).toBe(0);

    // 帧间隔翻倍越过判坏阈值, 逐级降到静帧档。
    frame.delay = 400;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(c.media.renderTier).toBe(3);
    expect(c.media.looping).toBe(false);
    // 静帧停在当前帧, 不回跳到首帧; 画布保留最后一帧继续显示。
    expect(video.currentTime).toBe(7);
    expect(canvas.classList.contains("is-visible")).toBe(true);

    vi.mocked(HTMLMediaElement.prototype.play).mockClear();
    c.showPage("settings");
    await nextTick();
    // 离页复位档位, 复位本身不抢先拉起播放。
    expect(c.media.renderTier).toBe(0);
    expect(c.media.looping).toBe(false);
    expect(video.play).not.toHaveBeenCalled();
    window.clearTimeout(pending);
  });

  it("静帧档恢复时首帧还没解码也请求播放", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    Object.defineProperties(video, {
      readyState: { value: 0, writable: true },
      paused: { value: false, writable: true },
      currentTime: { value: 4, writable: true },
    });
    vi.mocked(HTMLMediaElement.prototype.play).mockClear();
    vi.mocked(HTMLMediaElement.prototype.pause).mockClear();
    c.setHeroRenderTier(3);
    expect(c.media.renderTier).toBe(3);
    expect(c.media.looping).toBe(false);
    expect(video.pause).toHaveBeenCalled();

    // 首帧未解码就没人再拉起播放, 画布会永久停在上一帧。
    vi.mocked(HTMLMediaElement.prototype.play).mockClear();
    c.setHeroRenderTier(2);
    expect(c.media.renderTier).toBe(2);
    expect(video.play).toHaveBeenCalled();
    expect(c.media.looping).toBe(true);
    // 恢复续播不改写播放位置, 首帧解码出来后从同一帧继续。
    expect(video.currentTime).toBe(4);
  });

  it("恢复播放位置后按新区间重算名义帧间隔, 仍能识别过载", async () => {
    backend.data.settings = {
      ...settings,
      reduceMotion: false,
      heroMedia: "video",
    };
    // 源名义帧率 10fps: 解码计数贴着媒体时间走, 与绘制到达的节奏无关。
    // 过载只让绘制间隔翻倍到 200ms, 名义间隔仍应是 100ms。
    const sourceInterval = 100;
    const frame = { delay: sourceInterval };
    const decoded = { frames: 0 };
    let pending = 0;
    HTMLVideoElement.prototype.requestVideoFrameCallback = function (cb) {
      const source = this as HTMLVideoElement;
      pending = window.setTimeout(() => {
        source.currentTime += frame.delay / 1000;
        decoded.frames += Math.round(frame.delay / sourceInterval);
        cb(performance.now(), {
          mediaTime: source.currentTime,
        } as VideoFrameCallbackMetadata);
      }, frame.delay);
      return pending;
    };
    HTMLVideoElement.prototype.cancelVideoFrameCallback = (id) =>
      window.clearTimeout(id);

    const c = await boot();
    const video = document.getElementById("hero-video") as HTMLVideoElement;
    const canvas = document.getElementById("hero-canvas") as HTMLCanvasElement;
    Object.defineProperties(video, {
      readyState: { value: 2, writable: true },
      paused: { value: false, writable: true },
      seeking: { value: false, writable: true },
      duration: { value: 600 },
      videoWidth: { value: 1920 },
      videoHeight: { value: 1080 },
      currentTime: { value: 0, writable: true },
    });
    // jsdom 没有解码计数, 按测试控制的累计值提供。
    Object.defineProperty(video, "getVideoPlaybackQuality", {
      configurable: true,
      value: () => ({
        totalVideoFrames: decoded.frames,
        droppedVideoFrames: 0,
      }),
    });
    Object.defineProperties(canvas, {
      clientWidth: { value: 800 },
      clientHeight: { value: 600 },
    });
    resizeElement(canvas, 800, 600);

    // 健康设备跑完预热: 名义间隔 100ms 成为基线。
    await vi.advanceTimersByTimeAsync(2600);
    expect(canvas.classList.contains("is-visible")).toBe(true);
    expect(c.media.renderTier).toBe(0);

    // 游戏会话暂停播放并保留位置, 再隐藏窗口才会走重新加载与续播的路径。
    backend.handler(session({ running: true, pid: 123 }));
    await nextTick();
    await flushPromises();
    expect(c.media.looping).toBe(false);
    video.currentTime = 11;

    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    const visibility = vi
      .spyOn(document, "visibilityState", "get")
      .mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    await nextTick();
    await flushPromises();
    expect(video.hasAttribute("src")).toBe(false);

    visibility.mockReturnValue("visible");
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    await nextTick();
    await flushPromises();

    // 重新加载后解码计数仍是累计值, 播放位置却被搬回 11 秒:
    // 两者不再属于同一区间, 用累计比值会把基线放大到不再降档。
    video.dispatchEvent(new Event("loadedmetadata"));
    await nextTick();
    expect(video.currentTime).toBe(11);

    backend.handler(session());
    await nextTick();
    await flushPromises();
    expect(c.media.looping).toBe(true);

    // 恢复后整机过载: 绘制间隔翻倍, 源的名义间隔没变。
    frame.delay = 200;
    // 静帧档的试探恢复会短暂升档, 因此按是否到过静帧判定。
    let reachedStill = false;
    for (let round = 0; round < 12; round += 1) {
      await vi.advanceTimersByTimeAsync(5000);
      if (c.media.renderTier === 3) reachedStill = true;
    }
    expect(reachedStill).toBe(true);
    window.clearTimeout(pending);
  });
});

it("官方启动器视频通过本地流交给双缓冲循环", async () => {
  const official = "http://127.0.0.1:4321/video?token=abc&v=1";
  backend.background = { video: official, image: null };
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "video",
  };
  const c = await boot();
  const video = document.getElementById("hero-video") as HTMLVideoElement;
  expect(video.src).toBe(official);
  expect(video.loop).toBe(false);
  expect(c.media.officialVideo).toBe(true);
  expect(c.media.cropX).toBe(0.5);
  expect(c.media.cropY).toBe(0.5);
  expect(
    vi
      .mocked(fetch)
      .mock.calls.some(([url]) => String(url).includes("hero-loop")),
  ).toBe(false);
});

it("官方静态壁纸用于照片模式并使用居中构图", async () => {
  const official = "http://127.0.0.1:4321/image?token=abc&v=1";
  backend.background = { video: null, image: official };
  backend.data.settings = {
    ...settings,
    reduceMotion: true,
    heroMedia: "photo",
  };
  const c = await boot();
  expect(c.media.officialImage).toBe(true);
  expect(c.media.image).toBe(official);
  const art = document.querySelector(".hero__art") as HTMLElement;
  expect(art.classList.contains("hero__art--official")).toBe(true);
  expect(art.style.backgroundImage).toContain(official);
});

it("没有官方视频时回退内置视频", async () => {
  backend.background = { video: null, image: null };
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "video",
  };
  const c = await boot();
  expect(
    (document.getElementById("hero-video") as HTMLVideoElement).src,
  ).toContain("assets/hero-loop.mp4");
  expect(fetch).not.toHaveBeenCalled();
  expect(c.media.officialVideo).toBe(false);
  expect(c.media.cropX).toBeCloseTo(0.54);
  expect(c.media.cropY).toBeCloseTo(0.38);
});

it("切换地区后重新读取背景素材", async () => {
  const cn = "http://127.0.0.1:4321/video?token=abc&v=1";
  const global = "http://127.0.0.1:4321/video?token=abc&v=2";
  backend.background = { video: cn, image: null };
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "video",
  };
  const c = await boot();
  const video = document.getElementById("hero-video") as HTMLVideoElement;
  expect(video.src).toBe(cn);

  backend.background = { video: global, image: null };
  backend.data.launcherRoot = "/games/global/launcher";
  backend.data.settings = withGame(
    { distribution: "global", globalInstallRoot: "/games/global" },
    backend.data.settings,
  );
  c.form.distribution = "global";
  c.form.globalInstallRoot = "/games/global";
  await c.redetect();
  await flushPromises();

  expect(video.src).toBe(global);
  expect(c.media.officialVideo).toBe(true);
  expect(c.media.cropX).toBe(0.5);
});

it("切换地区时不复用旧地区的在途视频请求", async () => {
  const cn = "http://127.0.0.1:4321/cn.mp4";
  const global = "http://127.0.0.1:4321/global.mp4";
  backend.background = { video: cn, image: null };
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "video",
  };
  const base = backend.invoke.getMockImplementation()!;
  let finishCn!: (source: LauncherBackgroundMedia) => void;
  let calls = 0;
  backend.invoke.mockImplementation((command: string, args: never) => {
    if (command !== "launcher_background") return base(command, args);
    if (calls++ === 0)
      return new Promise<LauncherBackgroundMedia>((resolve) => {
        finishCn = resolve;
      });
    return Promise.resolve<LauncherBackgroundMedia>({
      video: global,
      image: null,
    });
  });
  const c = await boot();
  const video = document.getElementById("hero-video") as HTMLVideoElement;
  expect(c.media.url).toBe("");
  expect(finishCn).toBeTypeOf("function");

  backend.data.launcherRoot = "/games/global/launcher";
  backend.data.settings = withGame(
    { distribution: "global", globalInstallRoot: "/games/global" },
    backend.data.settings,
  );
  c.form.distribution = "global";
  c.form.globalInstallRoot = "/games/global";
  await c.redetect();
  await flushPromises();

  expect(video.src).toBe(global);
  // 旧地区的请求晚到也不能挡住或停掉新地区的视频.
  finishCn({ video: cn, image: null });
  await flushPromises();
  expect(video.src).toBe(global);
  expect(c.media.url).toBe(global);
});

it("在二级界面切换地区时保留旧背景直到新素材就绪", async () => {
  const cn = "http://127.0.0.1:4321/cn-keyart.jpg";
  const global = "http://127.0.0.1:4321/global-keyart.jpg";
  backend.background = { video: null, image: cn };
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "photo",
  };
  const c = await boot();
  expect(c.media.image).toBe(cn);

  c.showPage("update");
  const base = backend.invoke.getMockImplementation()!;
  let finish!: (source: LauncherBackgroundMedia) => void;
  backend.invoke.mockImplementation((command: string, args: never) =>
    command === "launcher_background"
      ? new Promise<LauncherBackgroundMedia>((resolve) => {
          finish = resolve;
        })
      : base(command, args),
  );
  backend.data.launcherRoot = "/games/global/launcher";
  backend.data.settings = withGame(
    { distribution: "global", globalInstallRoot: "/games/global" },
    backend.data.settings,
  );
  c.form.distribution = "global";
  c.form.globalInstallRoot = "/games/global";
  const switching = c.redetect();
  await flushPromises();
  expect(finish).toBeTypeOf("function");
  // 新地区的素材还没回来时, 二级界面仍应显示旧壁纸.
  expect(c.media.image).toBe(cn);

  finish({ video: null, image: global });
  await switching;
  await flushPromises();
  expect(c.media.image).toBe(global);
  expect(c.media.initialized).toBe(true);
});

it("背景读取失败时保留当前背景", async () => {
  const cn = "http://127.0.0.1:4321/cn-keyart.jpg";
  backend.background = { video: null, image: cn };
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "photo",
  };
  const c = await boot();
  c.showPage("update");
  const base = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation((command: string, args: never) =>
    command === "launcher_background"
      ? Promise.reject(new Error("offline"))
      : base(command, args),
  );
  backend.data.launcherRoot = "/games/global/launcher";
  backend.data.settings = withGame(
    { distribution: "global", globalInstallRoot: "/games/global" },
    backend.data.settings,
  );
  c.form.distribution = "global";
  c.form.globalInstallRoot = "/games/global";
  await c.redetect();
  await flushPromises();

  expect(c.media.image).toBe(cn);
  expect(c.media.initialized).toBe(true);
});

it("首次读取背景失败时回退内置壁纸", async () => {
  const base = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation((command: string, args: never) =>
    command === "launcher_background"
      ? Promise.reject(new Error("offline"))
      : base(command, args),
  );
  const c = await boot();

  expect(c.media.initialized).toBe(true);
  expect(c.media.image).toBe("");
  expect(c.media.officialImage).toBe(false);
});

it("在视频结尾切换到预载的备播元素", async () => {
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "video",
  };
  const c = await boot();
  const a = document.getElementById("hero-video") as HTMLVideoElement;
  const b = document.getElementById("hero-video-b") as HTMLVideoElement;
  const canvas = document.getElementById("hero-canvas") as HTMLCanvasElement;
  const draw = vi.mocked(canvas.getContext("2d")!.drawImage);
  Object.defineProperties(a, {
    readyState: { value: 2, writable: true },
    paused: { value: false, writable: true },
    seeking: { value: false, writable: true },
    duration: { value: 13 },
    videoWidth: { value: 1920 },
    videoHeight: { value: 1080 },
    currentTime: { value: 12, writable: true },
  });
  Object.defineProperties(b, {
    readyState: { value: 2, writable: true },
    paused: { value: false, writable: true },
    seeking: { value: false, writable: true },
    duration: { value: 13 },
    videoWidth: { value: 1920 },
    videoHeight: { value: 1080 },
    currentTime: { value: 0, writable: true },
  });
  Object.defineProperties(canvas, {
    clientWidth: { value: 800 },
    clientHeight: { value: 600 },
  });
  resizeElement(canvas, 800, 600);
  await vi.advanceTimersByTimeAsync(40);
  await vi.advanceTimersByTimeAsync(40);
  expect(draw).toHaveBeenLastCalledWith(
    b,
    expect.closeTo(259.2),
    0,
    1440,
    1080,
    0,
    0,
    800,
    600,
  );
  expect(c.media.looping).toBe(true);
});

it("放弃修复后同步安装目录表单", async () => {
  const controller = await boot();
  backend.data.settings.games.nte!.installRoot = "/moved-game";
  backend.invoke.mockImplementation(async (command: string) => {
    if (command === "migration_abandon")
      return { phase: "idle", canCancel: false } as never;
    if (command === "bootstrap") return structuredClone(backend.data);
    return null as never;
  });
  expect(await controller.abandonInstallMigration()).toBe(true);
  expect(controller.form.installRoot).toBe("/moved-game");
});

it("组件事件和双向绑定将用户操作传给控制器", async () => {
  const c = await boot();
  await wrapper!.find('[data-page="settings"]').trigger("click");
  expect(c.state.page).toBe("settings");
  await wrapper!.find('[data-category="game"]').trigger("click");
  await nextTick();
  await wrapper!.find("#set-install").setValue(" /component-game ");
  await wrapper!.find("#set-install").trigger("change");
  await wrapper!.find('[data-category="appearance"]').trigger("click");
  await nextTick();
  await wrapper!.find("#set-theme").setValue("dark");
  expect(localStorage.getItem("nte.theme")).toBe("dark");
  await wrapper!.find("#set-hero-media").setValue("photo");
  await wrapper!.find("#set-motion").setValue(false);
  await flushPromises();
  expect(gameSettings(backend.data.settings).installRoot).toBe(
    "/component-game",
  );
  expect(backend.data.settings.theme).toBe("dark");
  expect(backend.data.settings.reduceMotion).toBe(false);
  // 目录切换触发的统一刷新会启动只读资源检查, 这里模拟它完成.
  backend.downloadHandler({ ...backend.downloadStatus, phase: "ready" });
  await flushPromises();
  await wrapper!.find("#settings-redetect").trigger("click");
  await flushPromises();
  expect(noticeMessage()).toBe("已重新检测环境。");
  for (const id of [
    "open-official",
    "open-contract",
    "open-privacy",
    "open-support",
  ]) {
    await wrapper!.find(`#${id}`).trigger("click");
    await flushPromises();
  }
  expect(
    backend.invoke.mock.calls.filter(([command]) => command === "open_url"),
  ).toHaveLength(4);
  for (const action of ["minimize", "maximize", "close"])
    await wrapper!.find(`[data-win="${action}"]`).trigger("click");
  await wrapper!.find("#setup-chip").trigger("click");
  expect(c.state.page).toBe("settings");
  await wrapper!.find("#home-status").trigger("click");
  expect(c.state.page).toBe("update");
  await wrapper!.find("#update-quick").trigger("click");
  await flushPromises();
  await wrapper!.find("#update-full").trigger("click");
  await flushPromises();
  expect(backend.invoke).toHaveBeenCalledWith("check_update", { full: true });
  await wrapper!.find('[data-page="bulletin"]').trigger("click");
  expect(c.state.page).toBe("bulletin");
  await wrapper!.find('[data-page="home"]').trigger("click");
  await wrapper!.find('[data-page="update"]').trigger("click");
  await flushPromises();
  expect(c.state.page).toBe("update");
  backend.downloadHandler({ ...backend.downloadStatus, phase: "ready" });
  await flushPromises();
  await wrapper!.find('[data-page="update"]').trigger("click");
  await wrapper!.find("#launch-btn").trigger("click");
  await vi.advanceTimersByTimeAsync(900);
  expect(c.state.game).toBe("running");
});

it("版本显示随后台快照刷新, 缺失值显示未知而非固定版本", async () => {
  const c = await boot();
  backend.data = bootstrap({
    appVersion: "9.8.7",
    gameVersion: "6.5.4",
    launcherVersion: "3.2.1",
    launcherBuild: "abcd",
  });
  await c.redetect();
  expect(c.displayVersion.value).toBe("9.8.7");
  backend.data = bootstrap({
    appVersion: "",
    gameVersion: "",
    launcherVersion: "",
    launcherBuild: "",
    gameExists: false,
  });
  await c.redetect();
  expect(c.displayVersion.value).toBe(APP_VERSION);
});

it("后台会话事件通过响应式绑定更新按钮文字、图标和禁用状态", async () => {
  const c = await boot();
  backend.handler(session({ running: true }));
  await nextTick();
  expect(text("launch-btn")).toContain("关闭游戏");
  expect(wrapper!.find("#launch-btn use").attributes("href")).toBe("#i-power");
  expect(wrapper!.find("#launch-btn").classes()).toContain("is-running");
  backend.handler(session({ running: true, closing: true }));
  await nextTick();
  expect(text("launch-btn")).toContain("正在关闭");
  expect(button("launch-btn").getAttribute("aria-disabled")).toBe("true");
  expect(button("launch-btn").getAttribute("aria-busy")).toBe("true");
  backend.handler(session());
  // 关闭期间由命令维护中间状态, 避免后台事件覆盖反馈。
  expect(button("launch-btn").getAttribute("aria-disabled")).toBe("true");
  const calls = backend.invoke.mock.calls.length;
  c.handleLaunchAction();
  expect(c.state.game).toBe("closing");
  expect(backend.invoke.mock.calls.length).toBe(calls);
});

it("组件卸载时释放原生订阅、视频状态、定时器和全局事件", async () => {
  backend.data.settings = {
    ...settings,
    reduceMotion: false,
    heroMedia: "video",
  };
  const c = await boot();

  await c.openUrl("https://example.com");
  expect(vi.getTimerCount()).toBeGreaterThan(0);
  wrapper!.unmount();
  wrapper = undefined;
  expect(backend.unlisten).toHaveBeenCalledOnce();
  expect(backend.downloadUnlisten).toHaveBeenCalledOnce();
  expect(backend.stopResize).toHaveBeenCalledOnce();
  expect(c.media.url).toBe("");
  expect(vi.getTimerCount()).toBe(0);
  backend.invoke.mockClear();
  window.dispatchEvent(new Event("focus"));
  key("s", { ctrlKey: true });
  await flushPromises();
  expect(backend.invoke).not.toHaveBeenCalled();
});

it("卸载时取消尚未触发的自动保存", async () => {
  const c = await boot();
  backend.invoke.mockClear();
  c.form.globalInstallRoot = "/pending";
  c.queueSettingsSave();
  wrapper!.unmount();
  wrapper = undefined;
  await vi.advanceTimersByTimeAsync(1000);
  await flushPromises();
  expect(backend.invoke).not.toHaveBeenCalledWith(
    "save_settings",
    expect.anything(),
  );
  expect(c.ui.saveState).toBe("idle");
});

it("卸载后返回的初始化快照不会重新触发界面或入场", async () => {
  let resolve!: (data: Bootstrap) => void;
  const pending = new Promise<Bootstrap>((done) => {
    resolve = done;
  });
  const invoke = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(
    (
      command: string,
      args?: { settings: Settings; mode: "check" | "apply" },
    ) => (command === "bootstrap" ? pending : invoke(command, args)),
  );
  const c = await boot();
  wrapper!.unmount();
  wrapper = undefined;
  resolve(bootstrap());
  await flushPromises();
  expect(c.state.bootstrap).toBeNull();
  expect(c.ui.introDone).toBe(false);
  expect(vi.getTimerCount()).toBe(0);
});

it.each(["check_update", "save_settings", "bootstrap"] as const)(
  "卸载后忽略延迟的 %s 结果",
  async (command) => {
    const c = await boot();
    let resolve!: (value: unknown) => void;
    backend.invoke.mockReturnValueOnce(
      new Promise((done) => {
        resolve = done;
      }),
    );
    if (command === "save_settings") c.form.globalInstallRoot = "/late";
    const pending =
      command === "check_update"
        ? c.checkUpdate(true)
        : command === "save_settings"
          ? c.saveSettings()
          : c.redetect();
    await flushPromises();
    wrapper!.unmount();
    wrapper = undefined;
    resolve(command === "check_update" ? check({ changed: 2 }) : bootstrap());
    await pending;
    expect(c.state.bootstrap).toBeNull();
    expect(c.state.check).toBeNull();
    expect(Boolean(c.notifications.length)).toBe(false);
    expect(c.ui.saveState).toBe("idle");
    expect(c.ui.redetecting).toBe(false);
  },
);

it("启动反馈期间卸载会取消等待, 不再隐藏窗口", async () => {
  const c = await boot();
  c.handleLaunchAction();
  await flushPromises();
  wrapper!.unmount();
  wrapper = undefined;
  backend.invoke.mockClear();
  await vi.advanceTimersByTimeAsync(900);
  expect(c.state.game).toBe("idle");
  expect(backend.invoke).not.toHaveBeenCalled();
});

it("设置页面标题与按钮图标正确, 且能计算滚动模糊边缘状态", async () => {
  const c = await boot();
  c.showPage("settings");
  await nextTick();
  expect(text("settings-title")).toBe("设置");
  expect(wrapper!.find("#settings-redetect svg use").attributes("href")).toBe(
    "#i-arrows-clockwise",
  );
  expect(wrapper!.find("#settings-save").exists()).toBe(false);
  expect(wrapper!.find("#settings-autosave").exists()).toBe(false);
  expect(wrapper!.find("#page-settings").text()).not.toContain("自动保存");
  expect(wrapper!.find("#settings-form").attributes("data-save-state")).toBe(
    "idle",
  );

  const activePage = document.querySelector<HTMLElement>(".page.is-active")!;
  Object.defineProperty(activePage, "scrollHeight", {
    value: 1000,
    configurable: true,
  });
  Object.defineProperty(activePage, "clientHeight", {
    value: 600,
    configurable: true,
  });
  Object.defineProperty(activePage, "scrollTop", {
    value: 0,
    configurable: true,
    writable: true,
  });

  c.syncScrollEdges();
  expect(c.ui.scrollEdge.top).toBe(false);
  expect(c.ui.scrollEdge.bottom).toBe(true);

  activePage.scrollTop = 100;
  c.syncScrollEdges();
  expect(c.ui.scrollEdge.top).toBe(true);
  expect(c.ui.scrollEdge.bottom).toBe(true);

  activePage.scrollTop = 400;
  c.syncScrollEdges();
  expect(c.ui.scrollEdge.top).toBe(true);
  expect(c.ui.scrollEdge.bottom).toBe(false);

  c.showPage("home");
  await nextTick();
  c.syncScrollEdges();
  expect(c.ui.scrollEdge.top).toBe(false);
  expect(c.ui.scrollEdge.bottom).toBe(false);
});

it("保存先提交通用设置, 再提交已加载的签到偏好", async () => {
  const c = await boot();
  c.syncAttendance({
    accounts: [],
    preferences: {
      trigger: "off",
      time: "08:00",
      tasks: { ...defaultAttendanceTasks },
    },
  });
  c.attendance.loaded = true;
  c.attendanceForm.enabled = true;
  c.attendanceForm.trigger = "daily";
  c.attendanceForm.time = "20:30";
  c.form.globalInstallRoot = "/changed";
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (command, args) =>
    command === "attendance_configure"
      ? {
          accounts: [],
          preferences: {
            trigger: "daily",
            time: "20:30",
            tasks: { ...defaultAttendanceTasks },
          },
        }
      : original(command, args),
  );
  backend.invoke.mockClear();
  await c.saveSettings();
  expect(backend.invoke.mock.calls.map(([command]) => command)).toEqual([
    "save_settings",
    "attendance_configure",
  ]);
  expect(backend.invoke).toHaveBeenLastCalledWith("attendance_configure", {
    preferences: {
      trigger: "daily",
      time: "20:30",
      tasks: { ...defaultAttendanceTasks },
    },
  });
  expect(c.ui.saveState).toBe("saved");
});

it("签到未加载时保存不提交签到偏好", async () => {
  const c = await boot();
  c.attendance.loaded = false;
  c.attendanceForm.enabled = true;
  c.form.globalInstallRoot = "/changed";
  backend.invoke.mockClear();
  await c.saveSettings();
  expect(backend.invoke.mock.calls.map(([command]) => command)).toEqual([
    "save_settings",
  ]);
});

it("签到保存失败明确提示并保留错误状态", async () => {
  const c = await boot();
  c.syncAttendance({
    accounts: [],
    preferences: {
      trigger: "off",
      time: "08:00",
      tasks: { ...defaultAttendanceTasks },
    },
  });
  c.attendance.loaded = true;
  c.attendanceForm.enabled = true;
  c.form.globalInstallRoot = "/changed";
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (command, args) => {
    if (command === "attendance_configure")
      throw { message: "定时任务设置失败" };
    return original(command, args);
  });
  await c.saveSettings();
  expect(c.notifications[0]?.message).toBe(
    "签到设置保存失败: 定时任务设置失败",
  );
  expect(c.notifications[0]?.kind).toBe("error");
  expect(c.ui.saveState).toBe("error");
  expect(c.attendance.busy).toBe(false);
});

it("外部启动切换地区后刷新设置表单", async () => {
  const c = await boot();
  const data = structuredClone(backend.data);
  data.settings = withGame(
    { distribution: "global", globalInstallRoot: "/games/global" },
    data.settings,
  );
  data.installRoot = "/games/global";
  backend.settingsHandler(data);
  await nextTick();
  expect(gameSettings(c.state.bootstrap?.settings).distribution).toBe("global");
  expect(c.form.distribution).toBe("global");
});

it("外部启动失败的错误在初始化后提示", async () => {
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (command, args) => {
    if (command === "take_launch_error") return "异环启动失败: 缺少运行环境";
    return original(command, args);
  });
  const c = await boot();
  expect(
    c.notifications.some((item) => item.message.includes("异环启动失败")),
  ).toBe(true);
});

it("短信控制器保留未保存偏好, 发送失败复位并阻止并发", async () => {
  const c = await import("../../src/core/controller");
  Object.assign(c.attendance, { busy: false, running: false, error: "" });
  c.attendanceForm.enabled = true;
  c.attendanceForm.trigger = "boot";
  c.attendanceForm.time = "21:30";
  let finish!: (value: string) => void;
  backend.invoke.mockImplementationOnce(
    () =>
      new Promise<string>((resolve) => {
        finish = resolve;
      }),
  );
  const sending = c.sendAttendanceCode(" 13800138000 ");
  expect(c.attendance.busy).toBe(true);
  await expect(c.sendAttendanceCode("13800138000")).rejects.toThrow("正在进行");
  finish("test-device");
  await expect(sending).resolves.toBe("test-device");
  expect(backend.invoke).toHaveBeenCalledWith("attendance_send_code", {
    phone: "13800138000",
  });
  expect(c.attendance.busy).toBe(false);
  backend.invoke.mockRejectedValueOnce({ message: "发送失败" });
  await expect(c.sendAttendanceCode("13800138000")).rejects.toEqual({
    message: "发送失败",
  });
  expect(c.attendance.error).toBe("发送失败");
  expect(c.attendance.busy).toBe(false);
  backend.invoke.mockResolvedValueOnce({
    accounts: [],
    preferences: {
      trigger: "off",
      time: "08:00",
      tasks: { ...defaultAttendanceTasks },
    },
  });
  await c.loginAttendanceSms(" 13800138000 ", "123456", "test-device");
  expect(backend.invoke).toHaveBeenLastCalledWith("attendance_login_sms", {
    phone: "13800138000",
    code: "123456",
    deviceId: "test-device",
  });
  expect(c.attendanceForm).toMatchObject({
    enabled: true,
    trigger: "boot",
    time: "21:30",
    tasks: { ...defaultAttendanceTasks },
  });
  expect(c.attendance.error).toBe("");
  expect(c.attendance.busy).toBe(false);
  backend.invoke.mockRejectedValueOnce({ message: "验证码错误" });
  await expect(
    c.loginAttendanceSms("13800138000", "123456", "test-device"),
  ).rejects.toEqual({ message: "验证码错误" });
  expect(c.attendance.error).toBe("验证码错误");
  expect(c.attendance.busy).toBe(false);
});

it("区服切换后隔离公告缓存并丢弃旧请求", async () => {
  const c = await boot();
  localStorage.removeItem("nte.bulletins.v3.cn");
  const original = backend.invoke.getMockImplementation()!;
  let finishCn!: (feed: { items: []; errors: string[] }) => void;
  const globalFeed = { items: [], errors: ["global feed"] };
  backend.invoke.mockImplementation((command: string, args?: unknown) => {
    if (command === "fetch_bulletins") {
      if ((args as { distribution: string }).distribution === "cn")
        return new Promise((resolve) => {
          finishCn = resolve;
        });
      return Promise.resolve(globalFeed);
    }
    return original(command, args as Parameters<typeof original>[1]);
  });
  const pending = c.fetchBulletins(true, true);
  c.form.distribution = "global";
  c.form.globalInstallRoot = "/games/global";
  await c.saveSettings();
  await flushPromises();
  expect(gameSettings(backend.data.settings).launchMode).toBe("background");
  expect(c.state.bulletins).toEqual(globalFeed);
  finishCn({ items: [], errors: ["old cn feed"] });
  await pending;
  expect(c.state.bulletins).toEqual(globalFeed);
  expect(localStorage.getItem("nte.bulletins.v3.global")).toBeNull();
  expect(localStorage.getItem("nte.bulletins.v3.cn")).toBeNull();
  expect(wrapper!.findComponent({ name: "HomeGameRecords" }).exists()).toBe(
    true,
  );
});

it("公告来源失败不缓存, 下次读取会重新请求", async () => {
  const c = await boot();
  localStorage.removeItem("nte.bulletins.v3.cn");
  backend.invoke.mockResolvedValueOnce({
    items: [],
    errors: ["B站动态: 暂时不可用"],
  });
  await c.fetchBulletins(true, true);
  expect(c.state.bulletins?.errors).toHaveLength(1);
  expect(localStorage.getItem("nte.bulletins.v3.cn")).toBeNull();
  backend.invoke.mockResolvedValueOnce({ items: [], errors: [] });
  await c.fetchBulletins();
  expect(c.state.bulletins?.errors).toEqual([]);
  expect(localStorage.getItem("nte.bulletins.v3.cn")).toContain('"errors":[]');
  backend.invoke.mockResolvedValueOnce({ items: [], errors: ["临时失败"] });
  await c.fetchBulletins(true, true);
  backend.invoke.mockResolvedValueOnce({ items: [], errors: [] });
  await c.fetchBulletins();
  expect(c.state.bulletins?.errors).toEqual([]);
});

it("在运行诊断中只读显示安全验证状态", async () => {
  const c = await boot();
  const { settingsNavigation } = await import("../../src/core/settingsCatalog");
  c.showPage("settings");
  settingsNavigation.category = "maintenance";
  await nextTick();
  const row = document.querySelector('[data-setting="integrity"]');
  expect(row?.textContent).toContain("Oddity 安全性验证");
  expect(row?.textContent).toContain("在线已验证");
  expect(row?.textContent).toContain("官方清单比对通过");
  expect(row?.textContent).toContain("来源: 在线官方清单");

  backend.settingsHandler(
    bootstrap({
      integrity: {
        ...backend.data.integrity,
        state: "changed",
        source: "bundled",
        checkedAt: 2,
        changed: ["README.md"],
        changedTotal: 1,
        notice: null,
      },
    }),
  );
  await nextTick();
  expect(row?.textContent).toContain("与内置清单不一致");
  expect(c.ui.integrityDialog).toBe(false);
});

it("安全验证发现改动时提示一次并列出变更文件", async () => {
  const c = await boot();
  backend.invoke.mockClear();
  const changed = {
    ...backend.data.integrity,
    state: "changed" as const,
    source: "bundled" as const,
    checkedAt: 2,
    changed: ["resources/app.asar", "oddity"],
    changedTotal: 3,
    notice: "changed" as const,
  };
  c.applyIntegrityStatus(changed);
  await nextTick();
  expect(c.ui.integrityDialog).toBe(true);
  expect(document.body.textContent).toContain("Oddity 安全性验证未通过");
  // 弹窗刚出现时不能直接确认, 否则窗口崩溃会让这次版本不再提示.
  expect(backend.invoke).not.toHaveBeenCalledWith("integrity_acknowledge", {
    notice: "changed",
  });

  const view = [
    ...document.querySelectorAll<HTMLButtonElement>(".modal button"),
  ].find((item) => item.textContent?.includes("查看变更文件"))!;
  view.click();
  await nextTick();
  expect(document.body.textContent).toContain("resources/app.asar");
  expect(document.body.textContent).toContain("共 3 项");

  const later = [
    ...document.querySelectorAll<HTMLButtonElement>(".modal button"),
  ].find((item) => item.textContent?.includes("稍后处理"))!;
  later.click();
  await flushPromises();
  expect(c.ui.integrityDialog).toBe(false);
  expect(backend.invoke).toHaveBeenCalledWith("integrity_acknowledge", {
    notice: "changed",
  });

  // 同一次运行里重复收到同一个提示不再弹窗.
  c.applyIntegrityStatus({ ...changed, checkedAt: 3 });
  await nextTick();
  expect(c.ui.integrityDialog).toBe(false);
});

it("安全验证弹窗可以打开当前版本 Release", async () => {
  const c = await boot();
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    state: "changed",
    source: "bundled",
    checkedAt: 2,
    changed: ["resources/app.asar"],
    changedTotal: 1,
    notice: "changed",
  });
  await nextTick();
  backend.invoke.mockClear();
  const download = [
    ...document.querySelectorAll<HTMLButtonElement>(".modal button"),
  ].find((item) => item.textContent?.includes("前往下载官方版本"))!;
  download.click();
  await flushPromises();
  expect(backend.invoke).toHaveBeenCalledWith("open_url", {
    url: "https://github.com/LaT-SKY/Oddity/releases/tag/oddity-v0.1.0",
  });
});

it("打开官方下载页失败时给出提示", async () => {
  const c = await boot();
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    state: "changed",
    source: "bundled",
    checkedAt: 2,
    changed: ["resources/app.asar"],
    changedTotal: 1,
    notice: "changed",
  });
  await nextTick();
  const base = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation((command: string, args: never) =>
    command === "open_url"
      ? Promise.reject(new Error("打开链接失败"))
      : base(command, args),
  );
  await c.openIntegrityRelease();
  await flushPromises();
  expect(noticeMessage()).toContain("打开链接失败");
});

it("安全验证从失败恢复时只发一次成功通知", async () => {
  const c = await boot();
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    state: "unverified",
    source: null,
    checkedAt: 1,
    notice: null,
  });
  backend.invoke.mockClear();
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    checkedAt: 2,
    notice: "success",
  });
  await flushPromises();
  expect(noticeMessage()).toContain("Oddity 安全性验证通过");
  expect(backend.invoke).toHaveBeenCalledWith("integrity_acknowledge", {
    notice: "success",
  });

  // 同一版本内再次收到成功提示时不重复通知.
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    state: "unverified",
    source: null,
    checkedAt: 3,
    notice: null,
  });
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    checkedAt: 4,
    notice: "success",
  });
  expect(
    backend.invoke.mock.calls.filter(
      ([command]) => command === "integrity_acknowledge",
    ),
  ).toHaveLength(1);
});

it("安全验证结果先于 bootstrap 到达时不会降级", async () => {
  const c = await boot();
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    checkedAt: 9,
  });
  expect(c.state.bootstrap?.integrity.checkedAt).toBe(9);
  c.applyIntegrityStatus({
    ...backend.data.integrity,
    state: "checking",
    checkedAt: 1,
  });
  expect(c.state.bootstrap?.integrity.state).toBe("verified");
  expect(c.state.bootstrap?.integrity.checkedAt).toBe(9);

  backend.settingsHandler(
    bootstrap({
      integrity: {
        ...backend.data.integrity,
        state: "checking",
        checkedAt: 0,
      },
    }),
  );
  await nextTick();
  expect(c.state.bootstrap?.integrity.state).toBe("verified");
});
