import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { rpmMetadata } from "./release-version.mjs";
const uiRoot = fileURLToPath(new URL("../launcher/ui/", import.meta.url));
const require = createRequire(
  new URL("../launcher/ui/package.json", import.meta.url),
);
test("RPM 构建配置与发行版本转换保持一致", async () => {
  const pkg = require("./package.json");
  const original = pkg.version;
  const previous = process.cwd();
  process.chdir(uiRoot);
  try {
    for (const version of ["1.2.3", "1.2.3-alpha.1-hotfix+build.5"]) {
      pkg.version = version;
      delete require.cache[require.resolve("./electron-builder.cjs")];
      const config = await require("./electron-builder.cjs")();
      const rpm = rpmMetadata(version);
      assert.equal(
        config.rpm.artifactName,
        `oddity-${rpm.version}-${rpm.release}.x86_64.rpm`,
      );
      assert.deepEqual(config.rpm.fpm.slice(0, 4), [
        "--version",
        rpm.version,
        "--iteration",
        rpm.release,
      ]);
    }
  } finally {
    process.chdir(previous);
    pkg.version = original;
    delete require.cache[require.resolve("./electron-builder.cjs")];
  }
});

test("生成的桌面入口登记全部动作, 使桌面菜单实际展示它们", async () => {
  const previous = process.cwd();
  process.chdir(uiRoot);
  try {
    const config = await require("./electron-builder.cjs")();
    const registered =
      config.linux.desktop.entry.Actions.split(";").filter(Boolean);
    assert.deepEqual(
      registered,
      Object.keys(config.linux.desktop.desktopActions),
    );
    assert.equal(
      config.linux.desktop.desktopActions.XWayland.Exec,
      "oddity --xwayland",
    );
  } finally {
    process.chdir(previous);
  }
});
