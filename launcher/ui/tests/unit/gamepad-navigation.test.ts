import { beforeEach, afterEach, expect, it, vi } from "vitest";
import type { GamepadDevice, GamepadSnapshot } from "../../src/types/gamepad";
const mock = vi.hoisted(() => ({
  desktop: true,
  invoke: vi.fn(),
  feedback: vi.fn(),
  poll: vi.fn(),
  pause: vi.fn(),
  launch: vi.fn(),
  show: vi.fn(),
  settings: vi.fn(),
}));
vi.mock("../../src/core/api", () => ({
  invoke: mock.invoke,
  isDesktop: () => mock.desktop,
}));
vi.mock("../../src/core/gamepadBrowser", () => ({
  browserGamepadFeedback: mock.feedback,
  createBrowserGamepadSource: () => ({ poll: mock.poll }),
}));
vi.mock("../../src/core/controller", async () => {
  const { reactive } = await import("vue");
  const { defaultGamepadOptions } = await import("../../src/types/gamepad");
  return {
    form: reactive({ gamepad: defaultGamepadOptions() }),
    state: reactive({
      page: "settings",
      game: "idle",
      download: { phase: "idle" },
    }),
    showPage: mock.show,
    handleLaunchAction: mock.launch,
    openSettings: mock.settings,
  };
});
vi.mock("../../src/composables/useSceneMotion", async () => ({
  playerExpanded: (await import("vue")).ref(false),
}));
vi.mock("../../src/core/notifications", async () => ({
  notifications: (await import("vue")).reactive([]),
  pauseNotifications: mock.pause,
}));
let pad: typeof import("../../src/core/gamepad");
let controller: typeof import("../../src/core/controller");
let actions: typeof import("../../src/core/gamepadActions");
let player: typeof import("../../src/composables/useSceneMotion");
let notification: typeof import("../../src/core/notifications");
let nav:
  | ReturnType<typeof import("../../src/core/gamepad").createGamepadNavigation>
  | undefined;
let stop: (() => void) | undefined;
const device: GamepadDevice = {
  id: 1,
  name: "Xbox",
  family: "xbox",
  rumble: true,
  audio: false,
  triggerRumble: false,
  adaptive: false,
};
function bounds(element: HTMLElement) {
  return new DOMRect(
    Number(element.dataset.x ?? 20),
    Number(element.dataset.y ?? 40),
    Number(element.dataset.w ?? 80),
    Number(element.dataset.h ?? 30),
  );
}
function layout(content: string, navigation = "") {
  document.body.innerHTML = `<nav class="rail__nav"><button data-page="home">首页</button><button data-page="settings">设置</button><button data-page="update">更新</button></nav><div class="win-controls"><button id="minimize">最小化</button></div><section class="page is-active">${navigation}${content}</section>`;
  nav = pad.createGamepadNavigation();
  pad.gamepad.active = true;
  nav.refresh();
  return nav;
}
function target() {
  return pad.gamepadTarget.value?.id;
}
beforeEach(async () => {
  vi.clearAllMocks();
  mock.desktop = true;
  mock.invoke.mockResolvedValue(null);
  mock.feedback.mockResolvedValue(undefined);
  mock.launch.mockResolvedValue(undefined);
  pad = await import("../../src/core/gamepad");
  controller = await import("../../src/core/controller");
  actions = await import("../../src/core/gamepadActions");
  player = await import("../../src/composables/useSceneMotion");
  notification = await import("../../src/core/notifications");
  pad.gamepad.device = { ...device };
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      return bounds(this);
    },
  );
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockImplementation(
    function (this: HTMLElement) {
      return (this.style.display === "none"
        ? []
        : [bounds(this)]) as unknown as DOMRectList;
    },
  );
  HTMLElement.prototype.scrollBy = vi.fn();
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
});
afterEach(() => {
  stop?.();
  stop = undefined;
  nav?.dispose();
  nav?.reset();
  nav = undefined;
});
it("图标和确认键随设备及用户偏好变化, 反馈只在启用时发送并报告设备错误", async () => {
  for (const family of [
    "dualsense",
    "ds4",
    "switch",
    "xbox",
    "generic",
  ] as const) {
    pad.gamepad.device = { ...device, family };
    const prefix =
      family === "dualsense" || family === "ds4"
        ? "ps"
        : family === "switch"
          ? "ns"
          : "xbox";
    expect(pad.gamepadGlyph("confirm")).toBe(
      prefix + (prefix === "ps" ? "-cross" : "-a"),
    );
    expect(pad.gamepadGlyph("back")).toBe(
      prefix + (prefix === "ps" ? "-circle" : "-b"),
    );
    expect(pad.gamepadGlyph("west")).toBe(
      prefix + (prefix === "ps" ? "-square" : prefix === "ns" ? "-y" : "-x"),
    );
    expect(pad.gamepadGlyph("north")).toBe(
      prefix + (prefix === "ps" ? "-triangle" : prefix === "ns" ? "-x" : "-y"),
    );
    expect(pad.gamepadGlyph("start")).toBe(`${prefix}-start`);
  }
  pad.gamepad.device = null;
  expect(pad.gamepadFamily.value).toBe("generic");
  await pad.gamepadFeedback("confirm");
  expect(mock.invoke).not.toHaveBeenCalled();
  pad.gamepad.device = device;
  await pad.gamepadFeedback("confirm");
  expect(mock.invoke).not.toHaveBeenCalled();
  pad.gamepad.active = true;
  controller.form.gamepad.icons = "ds4";
  expect(pad.gamepadGlyph("confirm")).toBe("ps-cross");
  controller.form.gamepad.vibration = false;
  await pad.gamepadFeedback("error");
  expect(mock.invoke).not.toHaveBeenCalled();
  await pad.gamepadFeedback("stop");
  expect(mock.invoke).toHaveBeenCalledWith("gamepad_feedback", {
    deviceId: 1,
    effect: "stop",
  });
  controller.form.gamepad.vibration = true;
  mock.invoke.mockRejectedValueOnce(new Error("设备断开"));
  await pad.gamepadFeedback("error");
  expect(pad.gamepad.feedbackError).toContain("设备断开");
  await pad.gamepadFeedback("complete");
  expect(pad.gamepad.feedbackError).toBe("");
  mock.desktop = false;
  await pad.gamepadFeedback("confirm");
  expect(mock.feedback).toHaveBeenCalledWith(
    1,
    "confirm",
    controller.form.gamepad,
  );
});
it("空间导航跳过隐藏和禁用控件, 调节滑块并拦截视口外确认", async () => {
  const n = layout(
    `<button id="one">一</button><button id="two" data-x="120">二</button><button id="hidden" hidden>隐藏</button><button disabled>禁用</button><button data-pad-skip>跳过</button><button aria-disabled="true">禁用</button><input id="range" type="range" min="0" max="10" value="5" data-y="100"><button id="last" data-x="150" data-y="100">末尾</button>`,
  );
  expect(target()).toBe("one");
  await n.press(14);
  expect(target()).toBe("two");
  await n.press(12);
  expect(target()).toBe("last");
  await n.press(13);
  expect(target()).toBe("range");
  const input = document.querySelector<HTMLInputElement>("#range")!;
  const changed = vi.fn();
  input.addEventListener("change", changed);
  await n.press(14);
  expect(input.value).toBe("6");
  await n.press(13);
  expect(input.value).toBe("5");
  expect(changed).toHaveBeenCalledTimes(2);
  await n.press(11);
  expect(target()).toBe("one");
  await n.press(11);
  expect(target()).toBe("one");
  await n.press(13);
  expect(target()).toBe("one");
  const click = vi.fn();
  const one = document.getElementById("one")!;
  one.addEventListener("click", click);
  await n.press(0);
  expect(click).toHaveBeenCalledTimes(1);
  one.dataset.y = "-50";
  await n.press(0);
  expect(click).toHaveBeenCalledTimes(1);
  one.dataset.y = "1000";
  await n.press(0);
  expect(click).toHaveBeenCalledTimes(1);
  n.reset();
  expect(target()).toBeUndefined();
  expect(pad.gamepad.pressing).toBe(false);
});
it("分类确认进入内容, 返回分类后恢复焦点, 快捷键处理主操作、页面和窗口", async () => {
  const n = layout(
    `<div class="settings-panel is-current"><button id="content" data-pad-primary>应用</button><button id="second" data-x="150">次要</button></div>`,
    `<nav class="settings-categories"><button id="category" data-category="appearance" aria-current="true">外观</button><button id="category2" data-category="game">游戏</button></nav>`,
  );
  expect(target()).toBe("category");
  await n.press(0);
  expect(target()).toBe("content");
  expect(pad.gamepad.navigation).toBe(false);
  await n.press(14);
  expect(target()).toBe("second");
  await n.press(1);
  expect(target()).toBe("category");
  await n.press(0);
  expect(target()).toBe("content");
  await n.press(4);
  expect(pad.gamepad.windowControls).toBe(true);
  expect(target()).toBe("minimize");
  await n.press(1);
  expect(pad.gamepad.windowControls).toBe(false);
  expect(target()).toBe("content");
  const applied = vi.fn();
  document.getElementById("content")!.addEventListener("click", applied);
  await n.press(8);
  expect(applied).toHaveBeenCalledOnce();
  const pageClick = vi.fn();
  document
    .querySelector('[data-page="update"]')!
    .addEventListener("click", pageClick);
  await n.press(10);
  expect(pageClick).toHaveBeenCalledOnce();
  const homeClick = vi.fn();
  document
    .querySelector('[data-page="home"]')!
    .addEventListener("click", homeClick);
  await n.press(9);
  expect(homeClick).toHaveBeenCalledOnce();
  await n.press(6);
  expect(mock.settings).toHaveBeenCalledWith("gamepad");
  expect(pad.gamepad.navigation).toBe(true);
  await n.press(1);
  expect(mock.show).toHaveBeenCalledWith("home");
  controller.state.page = "update";
  controller.state.download!.phase = "game";
  await (await import("vue")).nextTick();
  expect(pad.gamepadTarget.value).not.toBeNull();
});
it("主页快捷启动和玩家面板, 下拉框事件及焦点转移", async () => {
  controller.state.page = "home";
  const n = layout(
    `<button id="launch-btn">启动</button><button id="launch-region" data-y="90">进度</button><section class="game-records"><button id="record">记录</button></section>`,
  );
  expect(target()).toBe("launch-btn");
  await n.press(2);
  expect(mock.launch).toHaveBeenCalledOnce();
  await n.press(3);
  expect(player.playerExpanded.value).toBe(true);
  expect(target()).toBe("record");
  await n.press(1);
  expect(player.playerExpanded.value).toBe(false);
  controller.state.game = "launching";
  n.reset();
  controller.state.page = "update";
  controller.state.page = "home";
  await (await import("vue")).nextTick();
  n.refresh();
  expect(target()).toBe("launch-region");
  controller.state.page = "update";
  await (await import("vue")).nextTick();
  document.querySelector(".page")!.innerHTML =
    `<button id="select" role="combobox" aria-expanded="true">选择</button><button id="focus" data-y="100">下一项</button>`;
  n.refresh();
  const select = document.getElementById("select")!;
  const keys: string[] = [];
  select.addEventListener("keydown", (event) => {
    keys.push(event.key);
    event.preventDefault();
    if (event.key === "Enter") document.getElementById("focus")!.focus();
  });
  for (const button of [11, 12, 1]) await n.press(button);
  expect(keys).toEqual(["ArrowUp", "ArrowDown", "Escape"]);
  n.scroll(40);
  n.scroll(45);
  expect(keys.at(-1)).toBe("ArrowDown");
  n.scroll(-90);
  expect(keys.at(-1)).toBe("ArrowUp");
  await n.press(0);
  expect(target()).toBe("focus");
});
it("弹窗按层级选择, 方向键优先交给控件, 返回关闭弹窗", async () => {
  const n = layout(
    `<button id="base">页面</button><div class="modal" data-modal-layer="bad"><div class="modal__panel"><button>底层</button></div></div><div class="modal" id="top" data-modal-layer="4"><div class="modal__panel"><button data-pad-close>关闭</button><button id="dialog">确认</button><button id="tab" role="tab" data-y="90">选项</button></div></div>`,
  );
  expect(target()).toBe("dialog");
  const tab = document.getElementById("tab")!;
  const clicked = vi.fn();
  tab.addEventListener("click", clicked);
  await n.press(12);
  expect(target()).toBe("tab");
  expect(clicked).toHaveBeenCalledOnce();
  tab.addEventListener("keydown", (event) => event.preventDefault());
  await n.press(11);
  expect(target()).toBe("tab");
  await n.press(1);
  expect(document.getElementById("top")).not.toBeNull();
  tab.replaceWith(tab.cloneNode(true));
  n.refresh();
  document
    .querySelector("#top [data-pad-close]")!
    .addEventListener("click", () => document.getElementById("top")!.remove());
  await n.press(1);
  expect(document.getElementById("top")).toBeNull();
  n.refresh();
  expect(target()).not.toBe("base");
});
it("文本编辑和文件选择快捷操作只作用于当前弹层", async () => {
  const n = layout(
    `<input id="text"><textarea id="area" data-y="90"></textarea><button id="region" data-pad-region data-y="140">区域</button>`,
  );
  await n.press(0);
  expect(pad.gamepad.editing?.id).toBe("text");
  actions.gamepadActions.keyboard = {
    cycle: vi.fn(),
    commit: vi.fn(),
    backspace: vi.fn(),
    clear: vi.fn(),
  };
  for (const button of [4, 6, 2, 7, 8]) await n.press(button);
  expect(actions.gamepadActions.keyboard.cycle).toHaveBeenCalledOnce();
  expect(actions.gamepadActions.keyboard.commit).toHaveBeenCalledOnce();
  expect(actions.gamepadActions.keyboard.backspace).toHaveBeenCalledOnce();
  expect(actions.gamepadActions.keyboard.clear).toHaveBeenCalledTimes(2);
  await n.press(1);
  expect(pad.gamepad.editing).toBeNull();
  await n.press(12);
  await n.press(0);
  expect(pad.gamepad.editing?.id).toBe("area");
  await n.press(1);
  document
    .querySelector(".page")!
    .insertAdjacentHTML(
      "beforeend",
      `<div data-pad-modal><div class="pad-files"><button id="file">文件</button></div></div>`,
    );
  actions.gamepadActions.picker = {
    primary: vi.fn(),
    parent: vi.fn(),
    reset: vi.fn(),
  };
  n.refresh();
  for (const button of [6, 2, 7, 8]) await n.press(button);
  expect(actions.gamepadActions.picker.primary).toHaveBeenCalledOnce();
  expect(actions.gamepadActions.picker.parent).toHaveBeenCalledOnce();
  expect(actions.gamepadActions.picker.reset).toHaveBeenCalledTimes(2);
  document.querySelector("[data-pad-modal]")!.remove();
  n.refresh();
  await n.press(12);
  n.refresh();
  document
    .getElementById("region")!
    .addEventListener("keydown", (event) => event.preventDefault());
  await n.press(12);
  expect(target()).toBe("region");
  await n.press(0);
  expect(pad.gamepad.editing).toBeNull();
  await n.press(1);
  expect(mock.show).not.toHaveBeenCalled();
});
it("通知层和操作按钮替换保留交互范围, 通知消失后恢复页面", async () => {
  const n = layout(
    `<div data-pad-actions><button id="old">下载</button></div>`,
  );
  document.getElementById("old")!.replaceWith(
    Object.assign(document.createElement("button"), {
      id: "new",
      textContent: "暂停",
    }),
  );
  n.refresh();
  expect(target()).toBe("new");
  pad.gamepad.notification = true;
  notification.notifications.push({
    id: 1,
  } as (typeof notification.notifications)[number]);
  document.body.insertAdjacentHTML(
    "beforeend",
    `<div class="notification-stack"><button id="notice">查看</button></div>`,
  );
  n.refresh();
  expect(target()).toBe("notice");
  await n.press(1);
  expect(pad.gamepad.notification).toBe(false);
  expect(mock.pause).toHaveBeenCalledWith("gamepad", false);
  pad.gamepad.notification = true;
  notification.notifications.splice(0);
  n.refresh();
  expect(pad.gamepad.notification).toBe(false);
  expect(target()).toBe("new");
  pad.gamepad.active = false;
  n.refresh();
  expect(target()).toBe("new");
});

it("滚动内容后把焦点留在可见候选项, 视口由滚动祖先裁剪", async () => {
  const n = layout(
    `<div id="scroll" style="overflow-y:auto" data-y="0" data-h="120"><button id="anchor" data-y="40">首项</button><button id="candidate" data-y="140">第二项</button><button id="third" data-x="200" data-y="140">另一列</button></div>`,
  );
  const scroller = document.getElementById("scroll")!;
  Object.defineProperties(scroller, {
    clientHeight: { value: 120 },
    scrollHeight: { value: 500 },
  });
  scroller.scrollBy = vi.fn((value: ScrollToOptions | number = 0) => {
    scroller.scrollTop += typeof value === "number" ? value : (value.top ?? 0);
  });
  for (const id of ["anchor", "candidate", "third"]) {
    const button = document.getElementById(id)!;
    Object.defineProperty(button, "getBoundingClientRect", {
      value: () => {
        const box = bounds(button);
        return new DOMRect(
          box.x,
          box.y - scroller.scrollTop,
          box.width,
          box.height,
        );
      },
    });
  }
  expect(pad.gamepadViewport(document.getElementById("anchor")!)).toEqual({
    top: 0,
    bottom: 120,
  });
  n.scroll(90);
  expect(scroller.scrollTop).toBe(90);
  expect(target()).toBe("candidate");
  n.scroll(5);
  expect(target()).toBe("candidate");
  n.scroll(300);
  expect(target()).toBe("candidate");
});

it("桌面轮询处理按下、摇杆和长按重复, 失焦和鼠标操作释放手柄模式", async () => {
  const snapshot: GamepadSnapshot = {
    idle: { requested: false, error: null },
    devices: [device],
    device,
    buttons: [],
    pressed: [],
    axes: [0, 0, 0, 0, 0, 0],
    focused: true,
    error: null,
  };
  mock.invoke.mockImplementation(async (command: string) =>
    command === "gamepad_poll" ? structuredClone(snapshot) : null,
  );
  layout(
    `<button id="first">一</button><button id="next" data-x="150">二</button><button id="down" data-y="120">三</button>`,
  );
  nav!.dispose();
  nav = undefined;
  pad.gamepad.active = false;
  const click = vi.fn();
  document.getElementById("first")!.addEventListener("click", click);
  stop = pad.startGamepad();
  await vi.advanceTimersByTimeAsync(0);
  expect(pad.gamepad.active).toBe(false);
  snapshot.pressed = [0];
  snapshot.buttons = [0];
  await vi.advanceTimersByTimeAsync(16);
  expect(click).toHaveBeenCalledOnce();
  expect(pad.gamepad.active).toBe(true);
  expect(pad.gamepad.pressing).toBe(true);
  snapshot.pressed = [];
  snapshot.buttons = [];
  snapshot.axes[0] = 1;
  await vi.advanceTimersByTimeAsync(16);
  expect(target()).toBe("next");
  snapshot.axes[0] = 0;
  snapshot.axes[1] = 1;
  await vi.advanceTimersByTimeAsync(16);
  expect(target()).toBe("down");
  await vi.advanceTimersByTimeAsync(340);
  expect(target()).toBe("down");
  snapshot.axes[1] = 0;
  snapshot.axes[0] = -1;
  await vi.advanceTimersByTimeAsync(16);
  snapshot.axes[0] = 0;
  snapshot.axes[1] = -1;
  await vi.advanceTimersByTimeAsync(16);
  expect(target()).toBe("first");
  snapshot.axes[1] = 0;
  snapshot.axes[3] = 0.8;
  await vi.advanceTimersByTimeAsync(16);
  expect(pad.gamepad.active).toBe(true);
  notification.notifications.push({
    id: 7,
    revision: 1,
    kind: "error",
  } as (typeof notification.notifications)[number]);
  await (await import("vue")).nextTick();
  expect(mock.invoke).toHaveBeenCalledWith(
    "gamepad_feedback",
    expect.objectContaining({ effect: "error" }),
  );
  notification.notifications[0]!.revision++;
  notification.notifications[0]!.kind = "ok";
  await (await import("vue")).nextTick();
  expect(mock.invoke).toHaveBeenCalledWith(
    "gamepad_feedback",
    expect.objectContaining({ effect: "complete" }),
  );
  const pointer = new Event("pointermove");
  Object.defineProperties(pointer, {
    movementX: { value: 1 },
    movementY: { value: 0 },
  });
  document.dispatchEvent(pointer);
  expect(pad.gamepad.active).toBe(true);
  document.dispatchEvent(new Event("pointerdown"));
  await (await import("vue")).nextTick();
  expect(pad.gamepad.active).toBe(false);
  expect(mock.invoke).toHaveBeenCalledWith("gamepad_release_idle");
  mock.invoke.mockRejectedValueOnce(new Error("轮询断开"));
  await vi.advanceTimersByTimeAsync(16);
  expect(pad.gamepad.deviceError).toContain("轮询断开");
  snapshot.focused = false;
  snapshot.device = null;
  await vi.advanceTimersByTimeAsync(180);
  expect(pad.gamepad.device).toBeNull();
  expect(pad.gamepad.active).toBe(false);
  window.dispatchEvent(new Event("blur"));
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "A" }));
  const dispose = stop!;
  stop = undefined;
  dispose();
  const calls = mock.invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(500);
  expect(mock.invoke).toHaveBeenCalledTimes(calls);
});

it("浏览器轮询按启用和窗口状态采样, 停止后忽略未完成的桌面响应", async () => {
  mock.desktop = false;
  const snapshot: GamepadSnapshot = {
    idle: { requested: false, error: null },
    devices: [],
    device: null,
    buttons: [],
    pressed: [],
    axes: [0, 0, 0, 0, 0, 0],
    focused: false,
    error: null,
  };
  mock.poll.mockReturnValue(snapshot);
  stop = pad.startGamepad();
  await vi.advanceTimersByTimeAsync(0);
  expect(mock.poll).toHaveBeenCalledWith(true);
  controller.form.gamepad.enabled = false;
  await vi.advanceTimersByTimeAsync(180);
  expect(mock.poll).toHaveBeenLastCalledWith(false);
  stop();
  stop = undefined;
  mock.desktop = true;
  let resolve!: (value: GamepadSnapshot) => void;
  mock.invoke.mockImplementationOnce(
    () =>
      new Promise<GamepadSnapshot>((done) => {
        resolve = done;
      }),
  );
  stop = pad.startGamepad();
  stop();
  stop = undefined;
  resolve({ ...snapshot, device });
  await vi.advanceTimersByTimeAsync(0);
  expect(pad.gamepad.device).toBeNull();
});
