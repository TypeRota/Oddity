// 由 launcher/tools/ui-registry-gen 生成, 请勿手改.
// 声明源: launcher/packages/*/ui/src/pages.ts, game.ts, preview.ts.
import { pages as gameFixturePages } from "@packages/game-fixture/ui/src/pages";
import { preview as gameFixturePreview } from "@packages/game-fixture/ui/src/preview";
import { pages as gameNtePages } from "@packages/game-nte/ui/src/pages";
import { game as gameNteGame } from "@packages/game-nte/ui/src/game";
import { preview as gameNtePreview } from "@packages/game-nte/ui/src/preview";

export const packagePages = [
  { dir: "game-fixture", pages: gameFixturePages },
  { dir: "game-nte", pages: gameNtePages },
] as const;

export const packageGames = [{ dir: "game-nte", game: gameNteGame }] as const;

export const packagePreviews = [
  { dir: "game-fixture", preview: gameFixturePreview },
  { dir: "game-nte", preview: gameNtePreview },
] as const;
