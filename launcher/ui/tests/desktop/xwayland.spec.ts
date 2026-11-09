import { test, expect, _electron as electron } from "@playwright/test";
import { mkdtemp, mkdir, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("发行入口的 XWayland 动作统一 Electron 与后台显示环境", async () => {
  const executablePath = process.env.ODDITY_TEST_EXECUTABLE;
  test.skip(!executablePath, "此用例验证发行版 shell 包装入口");
  const temporary = await mkdtemp(join(tmpdir(), "oddity-xwayland-"));
  await mkdir(join(temporary, "config"));
  await mkdir(join(temporary, "data"));
  const app = await electron.launch({
    executablePath,
    args: ["--xwayland"],
    env: {
      ...process.env,
      HOME: temporary,
      XDG_CONFIG_HOME: join(temporary, "config"),
      XDG_DATA_HOME: join(temporary, "data"),
      WAYLAND_DISPLAY: "nonexistent-wayland-display",
    },
  });
  try {
    await app.firstWindow();
    expect(
      await app.evaluate(({ app }) =>
        app.commandLine.getSwitchValue("ozone-platform"),
      ),
    ).toBe("x11");
    expect(
      await app.evaluate(() => process.env.WAYLAND_DISPLAY),
    ).toBeUndefined();
    const pid = app.process().pid!;
    const children = (
      await readFile(`/proc/${pid}/task/${pid}/children`, "utf8")
    )
      .trim()
      .split(/\s+/);
    let backendEnvironment: string[] | undefined;
    for (const child of children) {
      if (
        (await readFile(`/proc/${child}/comm`, "utf8")).trim() ===
        "oddity-backend"
      ) {
        backendEnvironment = (
          await readFile(`/proc/${child}/environ`, "utf8")
        ).split("\0");
        break;
      }
    }
    expect(backendEnvironment).toBeDefined();
    expect(
      backendEnvironment!.some((entry) => entry.startsWith("WAYLAND_DISPLAY=")),
    ).toBe(false);
  } finally {
    await app.close();
    await rm(temporary, { recursive: true, force: true });
  }
});
