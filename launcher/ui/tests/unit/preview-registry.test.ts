import { describe, expect, it, vi } from "vitest";
import { invoke } from "../../src/core/api";
import {
  packagePreviewContributions,
  previewHandler,
} from "../../src/host/preview";

const commandsOf = (namespace: string) =>
  Object.keys(
    packagePreviewContributions.find((entry) => entry.namespace === namespace)
      ?.commands ?? {},
  ).sort();

describe("包预览注册表", () => {
  it("各包按命名空间声明自己的演示命令", () => {
    expect(commandsOf("nte")).toEqual([
      "attendance_configure",
      "attendance_keyring_check",
      "attendance_login",
      "attendance_login_sms",
      "attendance_logout",
      "attendance_refresh_game_records",
      "attendance_run",
      "attendance_send_code",
      "attendance_status",
      "fetch_bulletins",
    ]);
    expect(commandsOf("fixture")).toEqual([
      "fetch_bulletins",
      "fixture_echo",
      "fixture_fail",
      "fixture_info",
    ]);
  });

  it("同名命令按当前游戏命名空间命中各自的演示", async () => {
    const nte = previewHandler("fetch_bulletins", "nte");
    const fixture = previewHandler("fetch_bulletins", "fixture");
    expect(nte).toBeTypeOf("function");
    expect(fixture).toBeTypeOf("function");
    expect(fixture).not.toBe(nte);
    // 异环的演示会等一段模拟网络延迟, 手动推进计时器.
    const pending = nte!({ distribution: "cn" });
    await vi.advanceTimersByTimeAsync(300);
    const nteFeed = await pending;
    const fixtureFeed = await fixture!({ distribution: "cn" });
    expect(nteFeed.items[0].title).toContain("异环");
    expect(fixtureFeed.items[0].title).toBe("测试游戏公告");
  });

  it("只查当前游戏命名空间, 不跨包回落", () => {
    expect(previewHandler("fixture_info", "fixture")).toBeTypeOf("function");
    expect(previewHandler("fixture_info", "nte")).toBeUndefined();
    expect(previewHandler("attendance_status", "fixture")).toBeUndefined();
    expect(previewHandler("fetch_bulletins", "hsr")).toBeUndefined();
    expect(previewHandler("bootstrap", "nte")).toBeUndefined();
    expect(previewHandler("game_download_status", "nte")).toBeUndefined();
  });

  it("浏览器演示先走包处理, 未知命令给出明确错误", async () => {
    await expect(invoke("attendance_status")).resolves.toMatchObject({
      accounts: [],
    });
    const pending = invoke("fetch_bulletins", { distribution: "cn" });
    await vi.advanceTimersByTimeAsync(300);
    await expect(pending).resolves.toMatchObject({ errors: [] });
    await expect(invoke("fixture_info")).rejects.toThrow(
      "浏览器演示未实现该命令",
    );
    await expect(invoke("not-a-command" as never)).rejects.toThrow(
      "浏览器演示未实现该命令",
    );
  });
});
