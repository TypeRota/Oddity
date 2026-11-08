import { beforeEach, expect, it, vi } from "vitest";
const native = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("../../src/core/desktop", () => native);
vi.mock("../../src/core/gamepad", () => ({ gamepad: { active: false } }));
beforeEach(() => {
  native.open.mockReset();
  native.save.mockReset();
});
it("鼠标选择使用原生对话框, 强制单选并返回取消或路径", async () => {
  const picker = await import("../../src/core/filePicker");
  native.open.mockResolvedValueOnce("/game").mockResolvedValueOnce(null);
  expect(
    await picker.open({ directory: true, multiple: true, title: "目录" }),
  ).toBe("/game");
  expect(native.open).toHaveBeenCalledWith({
    directory: true,
    multiple: false,
    title: "目录",
  });
  expect(await picker.open({})).toBeNull();
  native.save.mockResolvedValue("/export.json");
  expect(await picker.save({ defaultPath: "/export.json" })).toBe(
    "/export.json",
  );
  expect(picker.fileRequest.value).toBeNull();
});
it("手柄使用内置文件选择器, 合并过滤器并将确认和取消返回调用方", async () => {
  const { gamepad } = await import("../../src/core/gamepad");
  const picker = await import("../../src/core/filePicker");
  gamepad.active = true;
  const selected = picker.open({
    directory: true,
    defaultPath: "/home",
    filters: [
      { name: "图片", extensions: ["png", "jpg"] },
      { name: "文档", extensions: ["json"] },
    ],
  });
  expect(picker.fileRequest.value).toMatchObject({
    title: "选择文件",
    directory: true,
    save: false,
    defaultPath: "/home",
    extensions: ["png", "jpg", "json"],
  });
  picker.fileRequest.value!.resolve("/home/game");
  expect(await selected).toBe("/home/game");
  const cancelled = picker.save({});
  expect(picker.fileRequest.value).toMatchObject({
    title: "保存文件",
    directory: false,
    save: true,
    extensions: [],
  });
  picker.fileRequest.value!.resolve(null);
  expect(await cancelled).toBeNull();
  const named = picker.save({ title: "保存配置" });
  expect(picker.fileRequest.value!.title).toBe("保存配置");
  picker.fileRequest.value!.resolve("/profile.json");
  expect(await named).toBe("/profile.json");
  expect(native.open).not.toHaveBeenCalled();
  expect(native.save).not.toHaveBeenCalled();
});
