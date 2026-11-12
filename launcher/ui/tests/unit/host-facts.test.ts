import { describe, expect, it } from "vitest";
import { factsFrom, whenGame } from "../../src/host/facts";
import { bootstrap, capabilities, settings } from "./fixtures";
import type { Settings } from "../../src/types/launcher";

/** fixture 这类测试包没有对应的设置枚举, 后端把当前游戏报成包 id. */
function settingsFor(game: string): Settings {
  return { ...settings, activeGame: game as Settings["activeGame"] };
}

describe("宿主事实", () => {
  it("从 bootstrap 派生当前游戏与发行渠道", () => {
    expect(factsFrom(bootstrap())).toMatchObject({
      activeGame: "nte",
      distribution: "cn",
      capabilities,
    });
    expect(factsFrom(bootstrap())).toEqual({
      activeGame: "nte",
      distribution: "cn",
      capabilities,
      channels: bootstrap().channels,
    });
    expect(
      factsFrom(bootstrap({ settings: settingsFor("hsr"), channels: [] })),
    ).toMatchObject({
      activeGame: "hsr",
      distribution: null,
    });
  });

  it("没有声明渠道的游戏不给渠道值", () => {
    expect(factsFrom(bootstrap({ channels: [] }))).toEqual({
      activeGame: "nte",
      distribution: null,
      capabilities,
      channels: [],
    });
  });

  it("设置尚未到达时沿用默认值", () => {
    expect(factsFrom(null)).toEqual({
      activeGame: "nte",
      distribution: null,
      capabilities: [],
      channels: [],
    });
    expect(factsFrom(undefined)).toEqual({
      activeGame: "nte",
      distribution: null,
      capabilities: [],
      channels: [],
    });
  });

  it("能力事实随 bootstrap 一起进入宿主事实", () => {
    expect(factsFrom(bootstrap({ capabilities }))).toMatchObject({
      activeGame: "nte",
      distribution: "cn",
      capabilities,
    });
  });

  it("whenGame 生成按游戏判断的可见条件", () => {
    const visible = whenGame("fixture");
    expect(
      visible({
        activeGame: "fixture",
        distribution: "cn",
        capabilities: [],
        channels: [],
      }),
    ).toBe(true);
    expect(
      visible({
        activeGame: "nte",
        distribution: "cn",
        capabilities: [],
        channels: [],
      }),
    ).toBe(false);
  });
});
