import { afterEach, describe, expect, it, vi } from "vitest";

const { handlers, showErrorBox } = vi.hoisted(() => ({
  handlers: new Map<string, (...args: unknown[]) => unknown>(),
  showErrorBox: vi.fn(),
}));
vi.mock("electron", () => ({
  app: {
    isPackaged: false,
    setName: vi.fn(),
    setDesktopName: vi.fn(),
    setPath: vi.fn(),
    disableHardwareAcceleration: vi.fn(),
    requestSingleInstanceLock: () => true,
    whenReady: () => new Promise(() => {}),
    on: (name: string, handler: (...args: unknown[]) => unknown) =>
      handlers.set(name, handler),
  },
  BrowserWindow: vi.fn(),
  dialog: { showErrorBox },
  ipcMain: {},
  Menu: { setApplicationMenu: vi.fn() },
  net: {},
  protocol: { registerSchemesAsPrivileged: vi.fn() },
  Tray: vi.fn(),
}));
afterEach(() => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
  vi.resetModules();
  handlers.clear();
});

describe("XWayland 单实例交接", () => {
  it("现有 Wayland 实例明确提示退出, 不伪装切换成功", async () => {
    vi.stubEnv("WAYLAND_DISPLAY", "wayland-test");
    await import("../../electron/main");
    handlers.get("second-instance")!(null, ["electron", "entry", "--xwayland"]);
    expect(showErrorBox).toHaveBeenCalledWith(
      "无法切换显示模式",
      "请先从托盘完全退出 Oddity, 再选择 XWayland 模式.",
    );
  });
  it("X11 实例重复打开动作不弹出切换错误", async () => {
    vi.stubEnv("WAYLAND_DISPLAY", "");
    await import("../../electron/main");
    handlers.get("second-instance")!(null, ["electron", "entry", "--xwayland"]);
    expect(showErrorBox).not.toHaveBeenCalled();
  });
});
