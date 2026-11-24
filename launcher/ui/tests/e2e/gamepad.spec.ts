import { test, expect, type Page } from "@playwright/test";
import { settingsCategory } from "./settings-helpers";

type PadWindow = Window & {
  __hold: (index: number, pressed: boolean) => void;
};

function injectGamepad(page: Page) {
  return page.addInitScript(() => {
    const buttons = Array.from({ length: 17 }, () => ({
      pressed: false,
      touched: false,
      value: 0,
    }));
    const pad = {
      id: "Xbox Wireless Controller (STANDARD GAMEPAD Vendor: 045e Product: 0b13)",
      index: 0,
      connected: true,
      mapping: "standard",
      timestamp: 0,
      axes: [0, 0, 0, 0],
      buttons,
      vibrationActuator: {
        playEffect: () => Promise.resolve("complete"),
        reset: () => Promise.resolve("complete"),
      },
    };
    Object.defineProperty(navigator, "getGamepads", {
      configurable: true,
      value: () => [pad],
    });
    (window as unknown as { __hold: PadWindow["__hold"] }).__hold = (
      index,
      pressed,
    ) => {
      const button = pad.buttons[index]!;
      button.pressed = pressed;
      button.touched = pressed;
      button.value = pressed ? 1 : 0;
    };
  });
}

const hold = (page: Page, index: number, pressed: boolean) =>
  page.evaluate(
    ([buttonIndex, down]) =>
      (window as unknown as PadWindow).__hold(buttonIndex, down),
    [index, pressed] as const,
  );

test("迁移按钮在手柄模式和不同窗口宽度下保持右对齐", async ({ page }) => {
  await injectGamepad(page);
  await page.goto("/#settings");
  await settingsCategory(page, "nte-game");
  const group = page.locator(".settings-migration");
  const buttons = group.getByRole("button");
  await expect(buttons).toHaveCount(2);

  for (const gamepadMode of [false, true]) {
    if (gamepadMode) {
      await hold(page, 15, true);
      await expect(page.locator("#app")).toHaveClass(/gamepad-mode/);
      await hold(page, 15, false);
      await expect(group.locator(".gamepad-button-hint")).toHaveCount(2);
    }
    for (const width of [1440, 1180, 960, 640]) {
      await page.setViewportSize({ width, height: 760 });
      await expect(async () => {
        const control = await group.locator("..").boundingBox();
        const first = await buttons.nth(0).boundingBox();
        const second = await buttons.nth(1).boundingBox();
        expect(control).not.toBeNull();
        expect(first).not.toBeNull();
        expect(second).not.toBeNull();
        expect(second!.x + second!.width).toBeCloseTo(
          control!.x + control!.width,
          0,
        );
        if (Math.abs(first!.y - second!.y) > 1) {
          expect(first!.x + first!.width).toBeCloseTo(
            second!.x + second!.width,
            0,
          );
        } else {
          expect(first!.x + first!.width).toBeLessThan(second!.x);
        }
        expect(first!.x).toBeGreaterThanOrEqual(control!.x - 1);
      }).toPass();
      if (gamepadMode && width === 1180) {
        await group.screenshot({
          path: "test-results/migration-gamepad-alignment.png",
        });
      }
    }
  }
});

test("浏览器预览识别标准手柄并支持导航", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await injectGamepad(page);
  await page.goto("/");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);

  // D-pad 右: 进入手柄操作模式并出现按键提示.
  await hold(page, 15, true);
  await expect(page.locator(".gamepad-hints")).toBeVisible();
  await expect(page.locator("#app")).toHaveClass(/gamepad-mode/);
  await hold(page, 15, false);

  // Start: 直接打开手柄设置, 并显示已连接设备.
  await hold(page, 9, true);
  await expect(page.locator("#page-settings")).toBeVisible();
  await hold(page, 9, false);
  const settings = page.locator('[data-setting="gamepad"]');
  await expect(settings).toContainText("Xbox Wireless Controller");
  await expect(settings).toContainText("已连接");
  await page.screenshot({ path: "test-results/gamepad-browser.png" });

  expect(errors).toEqual([]);
});

for (const alphabetical of [false, true]) {
  test(`键盘布局设置与手柄纵向移动 (${alphabetical ? "A-Z" : "QWERTY"})`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await injectGamepad(page);
    await page.goto("/#settings");
    await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
    await page.locator('[data-category="gamepad"]').click();
    const layout = page.locator("#set-gamepad-keyboard-layout-trigger");
    await expect(layout).toContainText("美式键盘");
    await layout.click();
    await page
      .getByRole("option", { name: "字母表 (A-Z)", exact: true })
      .click();
    await expect(page.locator("#settings-form")).toHaveAttribute(
      "data-save-state",
      "saved",
    );
    await page.locator('[data-category="appearance"]').click();
    await page.locator('[data-category="gamepad"]').click();
    await expect(layout).toContainText("字母表 (A-Z)");
    if (!alphabetical) {
      await layout.click();
      await page.getByRole("option", { name: "美式键盘", exact: true }).click();
    }
    await page.locator('[data-category="nte-game"]').click();
    const press = async (index: number) => {
      await hold(page, index, true);
      await page.waitForTimeout(80);
      await hold(page, index, false);
      await page.waitForTimeout(80);
    };
    await press(15);
    await press(0);
    await expect(page.locator("#set-distribution-trigger")).toHaveAttribute(
      "data-pad-selected",
      "true",
    );
    await press(13);
    await expect(
      page.getByRole("button", { name: "浏览国服安装目录", exact: true }),
    ).toHaveAttribute("data-pad-selected", "true");
    await press(14);
    await expect(page.locator("#set-install")).toHaveAttribute(
      "data-pad-selected",
      "true",
    );
    await press(0);
    const keyboard = page.getByRole("dialog", {
      name: "屏幕键盘",
      exact: true,
    });
    await expect(keyboard).toBeVisible();
    const keys = keyboard.locator(
      ".pad-keyboard__row:not(.pad-keyboard__actions)",
    );
    await expect(keys).toHaveCount(4);
    for (const row of await keys.all())
      await expect(row.locator("button")).toHaveCount(10);
    // 动作按钮右侧标明手柄快捷键, 用鼠标时也要看得到.
    const actions = keyboard.locator(".pad-keyboard__actions");
    for (const [label, keys] of [
      ["键盘", 1],
      ["删除", 1],
      ["完成", 1],
    ] as const) {
      const hint = actions
        .getByRole("button", { name: label, exact: true })
        .locator(".gamepad-button-hint");
      await expect(hint).toHaveCSS("opacity", "1");
      await expect(hint.locator("use")).toHaveCount(keys);
    }
    await expect(keyboard.locator("[data-pad-selected]")).toHaveText("1");
    await press(15);
    await press(13);
    await expect(keyboard.locator("[data-pad-selected]")).toHaveText(
      alphabetical ? "b" : "w",
    );
    await press(13);
    await expect(keyboard.locator("[data-pad-selected]")).toHaveText(
      alphabetical ? "l" : "s",
    );
    await page.screenshot({
      path: `test-results/gamepad-keyboard-${alphabetical ? "alphabetical" : "qwerty"}.png`,
    });
    await press(12);
    await expect(keyboard.locator("[data-pad-selected]")).toHaveText(
      alphabetical ? "b" : "w",
    );
    expect(errors).toEqual([]);
  });
}
