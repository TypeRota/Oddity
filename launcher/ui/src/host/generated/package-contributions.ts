// 由 launcher/tools/ui-registry-gen 生成, 请勿手改.
// 声明源: launcher/packages/*/ui/src/contribution.ts.
import { contribution as gameFixtureContribution } from "@packages/game-fixture/ui/src/contribution";
import { contribution as gameNteContribution } from "@packages/game-nte/ui/src/contribution";

export const packageContributions = [
  { dir: "game-fixture", contribution: gameFixtureContribution },
  { dir: "game-nte", contribution: gameNteContribution },
] as const;
