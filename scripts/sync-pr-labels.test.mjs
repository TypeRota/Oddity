import assert from "node:assert/strict";
import test from "node:test";
import { calculateLabels, syncPrLabels } from "./sync-pr-labels.mjs";

test("按文件路径合并 crate、游戏和功能范围", () => {
  assert.deepEqual(
    calculateLabels([
      "launcher/crates/oddity-host/src/environment_commands.rs",
      "launcher/packages/game-nte/backend/src/update/mod.rs",
      "launcher/packages/game-nte/backend/src/auth/mod.rs",
      "launcher/packages/game-arknights/ui/src/main.ts",
      "launcher/packages/game-azurpromilia/backend/src/launch.rs",
      "launcher/packages/game-hsr/backend/src/lib.rs",
    ]),
    [
      "area: arknights",
      "area: azurpromilia",
      "area: host",
      "area: hsr",
      "area: login",
      "area: nte",
      "area: proton",
      "area: ui",
      "area: updater",
    ],
  );
});

test("七个宿主 crate 按各自边界标注", () => {
  const crates = [
    "backend",
    "cli",
    "contract",
    "host",
    "platform",
    "recovery",
    "runtime",
  ];
  for (const name of crates) {
    assert.deepEqual(
      calculateLabels([`launcher/crates/oddity-${name}/Cargo.toml`]),
      [`area: ${name}`],
    );
  }
  assert.deepEqual(calculateLabels(["launcher/ui/electron/commands.ts"]), [
    "area: contract",
    "area: ui",
  ]);
});

test("识别打包、命令行和 Gitea 工作流", () => {
  assert.deepEqual(
    calculateLabels([
      ".gitea/workflows/release.yml",
      "launcher/ui/electron/after-pack.cjs",
      "packaging/gentoo/oddity.ebuild",
      "launcher/crates/oddity-cli/src/main.rs",
    ]),
    ["area: cli", "area: packaging", "area: ui", "ci"],
  );
});

test("混合修改也附加独立的文档和打包标签", () => {
  assert.deepEqual(calculateLabels(["README.md", "docs/assets/demo.png"]), [
    "docs",
  ]);
  assert.deepEqual(
    calculateLabels([
      "docs/assets/demo.png",
      "packaging/arch/PKGBUILD.in",
      "launcher/crates/oddity-cli/src/main.rs",
    ]),
    ["area: cli", "area: packaging", "docs"],
  );
  assert.deepEqual(calculateLabels([]), []);
});

test("同步保留人工标签并计入重命名前后的路径", async () => {
  const available = [
    "area: packaging",
    "area: ui",
    "area: host",
    "status: blocked",
    "feature",
  ].map((name, id) => ({ name, id: id + 1 }));
  let labels = [available[2], available[3], available[4]];
  const writes = [];
  const api = {
    list: async (path) => {
      if (path === "/labels") return available;
      assert.equal(path, "/pulls/76/files");
      return [
        {
          filename: "launcher/ui/electron/after-pack.cjs",
          previous_filename: "packaging/after-pack.cjs",
        },
      ];
    },
    request: async (method, path, body) => {
      if (method === "GET") {
        assert.equal(path, "/issues/76/labels");
        return labels;
      }
      writes.push([method, path, body]);
      if (method === "POST") {
        labels = labels.concat(
          available.filter(({ id }) => body.labels.includes(id)),
        );
      } else {
        labels = labels.filter(({ id }) => path !== `/issues/76/labels/${id}`);
      }
    },
  };

  await syncPrLabels(api, 76);
  await syncPrLabels(api, 76);
  assert.deepEqual(writes, [
    ["POST", "/issues/76/labels", { labels: [1, 2] }],
    ["DELETE", "/issues/76/labels/3", undefined],
  ]);
  assert.deepEqual(labels.map(({ name }) => name).sort(), [
    "area: packaging",
    "area: ui",
    "feature",
    "status: blocked",
  ]);
});

test("文件移出旧范围时，重命名前的路径仍计入改动", () => {
  assert.deepEqual(
    calculateLabels([
      "docs/moved.ts",
      "launcher/packages/game-fixture/ui/src/moved.ts",
    ]),
    ["area: ui", "docs"],
  );
});
