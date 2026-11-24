import { expect, it } from "vitest";
import {
  homeCardContributions,
  selectHomeCards,
} from "../../src/host/home-cards";
import { factsFrom } from "../../src/host/facts";
import { PlayerInfoCard } from "@packages/game-nte/ui";
import { bootstrap, capabilities } from "./fixtures";
import type { Bootstrap } from "../../src/types/launcher";

it("首页卡片表绑定游戏包提供的玩家信息卡", () => {
  expect(homeCardContributions.map((card) => card.id)).toEqual([
    "nte-player-info",
    "fixture-home",
  ]);
  expect(homeCardContributions[0].component).toBe(PlayerInfoCard);
});

it("玩家信息卡只在异环激活时挂载", () => {
  expect(
    selectHomeCards(factsFrom(bootstrap({ capabilities }))).map(
      (card) => card.id,
    ),
  ).toEqual(["nte-player-info"]);
  expect(
    selectHomeCards({
      activeGame: "fixture",
      distribution: "cn",
      capabilities,
      channels: [],
    }).map((card) => card.id),
  ).toEqual(["fixture-home"]);
  expect(
    selectHomeCards({
      activeGame: "hsr",
      distribution: "cn",
      capabilities,
      channels: [],
    }),
  ).toEqual([]);
});

it("玩家档案能力不支持时卸载玩家信息卡, 不留占位", () => {
  // 国际服立场: 后端把该能力报告为 unsupported, 卡片整块不挂载.
  const unsupported = capabilities.map((fact) =>
    fact.id === "oddity.profile.player"
      ? { ...fact, state: "unsupported" as const }
      : fact,
  );
  const facts = factsFrom(
    bootstrap({
      capabilities: unsupported as Bootstrap["capabilities"],
      channels: [
        {
          id: "cn",
          label: "国服",
          default: true,
          active: false,
          installRootKey: "cn",
        },
        {
          id: "global",
          label: "国际服",
          default: false,
          active: true,
          installRootKey: "global",
        },
      ],
    }),
  );
  expect(facts.distribution).toBe("global");
  expect(selectHomeCards(facts)).toEqual([]);
  // 能力恢复到可用后重新挂载.
  expect(
    selectHomeCards(
      factsFrom(
        bootstrap({
          capabilities,
          channels: [
            {
              id: "cn",
              label: "国服",
              default: true,
              active: true,
              installRootKey: "cn",
            },
            {
              id: "global",
              label: "国际服",
              default: false,
              active: false,
              installRootKey: "global",
            },
          ],
        }),
      ),
    ).map((card) => card.id),
  ).toEqual(["nte-player-info"]);
});
