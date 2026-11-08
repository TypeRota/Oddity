import { test, expect } from "@playwright/test";
import { settingsCategory } from "./settings-helpers";

test("音频分类支持音量增强、静音、恢复和重新打开", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/#settings");
  await settingsCategory(page, "audio");
  await expect(page.locator("#set-audio-device-trigger")).toBeVisible();
  await page.locator("#set-audio-device-trigger").click();
  await page.getByRole("option", { name: "系统输出设备" }).click();
  const volume = page.locator("#set-audio-volume");
  await volume.fill("150");
  await volume.dispatchEvent("change");
  await expect(page.getByText("音量超过 100%, 可能产生失真.")).toBeVisible();
  await expect(page.locator("#settings-form")).toHaveAttribute(
    "data-save-state",
    "saved",
  );
  await settingsCategory(page, "appearance");
  await settingsCategory(page, "audio");
  await expect(volume).toHaveValue("150");
  await expect(page.locator("#set-audio-device-trigger")).toContainText(
    "系统输出设备",
  );
  await page.setViewportSize({ width: 640, height: 520 });
  expect(
    await page
      .locator("#settings-panel-audio")
      .evaluate((element) => element.scrollWidth - element.clientWidth),
  ).toBeLessThanOrEqual(1);
  await volume.fill("0");
  await volume.dispatchEvent("change");
  await expect(page.locator("output[for=set-audio-volume]")).toHaveText("0%");
  await page.getByRole("button", { name: "恢复为 100%" }).click();
  await expect(volume).toHaveValue("100");
  await settingsCategory(page, "appearance");
  await page.getByRole("searchbox", { name: "搜索设置" }).fill("增益");
  await page.locator(".settings-result").click();
  await expect(volume).toBeFocused();
  expect(errors).toEqual([]);
});
