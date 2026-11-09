import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

for (const scenario of [
  {
    name: "普通入口保留显示环境",
    args: [],
    target: "shell",
    wayland: "wayland-test",
    expected: [],
  },
  {
    name: "XWayland 同时选择 Electron X11 与游戏显示环境",
    args: ["--xwayland", "--launch", "--environment", "GE Proton test"],
    target: "shell",
    wayland: null,
    expected: [
      "--ozone-platform=x11",
      "--xwayland",
      "--launch",
      "--environment",
      "GE Proton test",
    ],
  },
  {
    name: "外部启动帮助不启动 Electron",
    args: ["--xwayland", "--launch", "--help"],
    target: "backend",
    wayland: "wayland-test",
  },
  {
    name: "应急恢复仍进入恢复程序",
    args: ["--recovery"],
    target: "recovery",
    wayland: "wayland-test",
  },
]) {
  test(scenario.name, () => {
    const directory = mkdtempSync(join(tmpdir(), "oddity desktop "));
    try {
      mkdirSync(join(directory, "resources/bin"), { recursive: true });
      writeFileSync(
        join(directory, "oddity"),
        readFileSync(new URL("../launcher/packaging/oddity", import.meta.url)),
        { mode: 0o755 },
      );
      for (const [path, target] of [
        ["oddity-shell", "shell"],
        ["resources/bin/oddity-backend", "backend"],
        ["resources/bin/oddity-recovery", "recovery"],
      ]) {
        writeFileSync(
          join(directory, path),
          `#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify({target:${JSON.stringify(target)},args:process.argv.slice(2),wayland:process.env.WAYLAND_DISPLAY??null,proxy:process.env.HTTP_PROXY}));\n`,
          { mode: 0o755 },
        );
      }
      const result = JSON.parse(
        execFileSync(join(directory, "oddity"), scenario.args, {
          env: {
            ...process.env,
            WAYLAND_DISPLAY: "wayland-test",
            HTTP_PROXY: "http://proxy.invalid:8080",
          },
          encoding: "utf8",
        }),
      );
      assert.equal(result.target, scenario.target);
      assert.equal(result.wayland, scenario.wayland);
      assert.deepEqual(result.args, scenario.expected ?? scenario.args);
      assert.equal(result.proxy, "http://proxy.invalid:8080");
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
}
