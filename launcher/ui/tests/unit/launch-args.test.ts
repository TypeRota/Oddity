import { describe, expect, it } from "vitest";
import {
  isLaunchRequest,
  secondInstanceArgs,
} from "../../electron/launch-args";

describe("单实例启动参数转发", () => {
  it("使用第二实例的原始参数, 不读取被 Chromium 重排的事件 argv", () => {
    const original = [
      "--launch",
      "--game",
      "nte",
      "--region",
      "cn",
      "--launch-mode",
      "direct",
      "--environment",
      "GE Proton 9",
    ];
    const reordered = [
      "oddity-shell",
      "--launch",
      "--game",
      "--region",
      "--launch-mode",
      "--environment",
      "--allow-file-access-from-files",
      "--ozone-platform=wayland",
      "nte",
      "cn",
      "direct",
      "GE Proton 9",
    ];

    expect(secondInstanceArgs({ argv: original }, reordered, true)).toEqual(
      original,
    );
    expect(isLaunchRequest(original)).toBe(true);
  });

  it("缺少 additionalData 时退回事件 argv", () => {
    expect(
      secondInstanceArgs(undefined, ["oddity-shell", "--launch"], true),
    ).toEqual(["--launch"]);
    expect(
      secondInstanceArgs(
        { argv: ["--launch", 1] },
        ["oddity-shell", "--launch"],
        true,
      ),
    ).toEqual(["--launch"]);
  });

  it("开发模式跳过 Electron 与应用路径", () => {
    expect(
      secondInstanceArgs({}, ["electron", "app", "--launch"], false),
    ).toEqual(["--launch"]);
  });

  it("等号形式与独立形式都识别为启动请求", () => {
    expect(isLaunchRequest(["--launch"])).toBe(true);
    expect(isLaunchRequest(["--game=nte"])).toBe(true);
    expect(isLaunchRequest(["--environment", "GE Proton 9"])).toBe(true);
    expect(isLaunchRequest(["--display", ":0"])).toBe(false);
  });
});
