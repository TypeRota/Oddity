import { describe, expect, it } from "vitest";
import { activeGame, gameSettings } from "../../src/core/game";
import { defaultGame, settings } from "./fixtures";
import type { Settings } from "../../src/types/launcher";

describe("当前游戏设置", () => {
  it("读取 activeGame 对应的设置", () => {
    const data = {
      ...settings,
      games: { nte: defaultGame({ installRoot: "/nte" }) },
    };
    expect(activeGame(data)).toBe("nte");
    expect(gameSettings(data).installRoot).toBe("/nte");
  });

  it("缺少 activeGame 条目时使用默认值, 不回退到其他游戏", () => {
    const data: Settings = {
      ...settings,
      activeGame: "hsr",
      games: { nte: defaultGame({ installRoot: "/nte" }) },
    };
    expect(gameSettings(data)).toEqual(defaultGame());
  });

  it("旧设置没有 activeGame 时按《异环》读取", () => {
    const legacy = {
      ...settings,
      activeGame: undefined,
      games: { nte: defaultGame({ installRoot: "/nte" }) },
    } as unknown as Settings;
    expect(activeGame(legacy)).toBe("nte");
    expect(gameSettings(legacy).installRoot).toBe("/nte");
  });
});
