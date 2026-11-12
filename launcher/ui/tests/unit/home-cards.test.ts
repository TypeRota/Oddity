import { expect, it } from "vitest";
import {
  homeCardContributions,
  selectHomeCards,
} from "../../src/host/home-cards";
import { factsFrom } from "../../src/host/facts";
import { PlayerInfoCard } from "@packages/game-nte/ui";
import { bootstrap, capabilities } from "./fixtures";

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
