import { test, expect } from "@playwright/test";

test("预下载完成保留独立状态", async ({ page }) => {
  await page.goto("/#update");
  const update = page.locator("#page-update");
  const preload = update.locator("#game-preload");
  await expect(update.locator("#game-download-primary")).toHaveText("检查通过");
  await preload.click();
  await expect(
    update.getByRole("heading", { name: "正在预下载游戏资源" }),
  ).toBeVisible();
  await expect(preload).toBeDisabled();
  await expect(
    update.getByRole("heading", { name: "资源预下载已完成" }),
  ).toBeVisible();
  await expect(update).toContainText("版本更新开放后将复用缓存完成安装");
  await expect(update.locator("#game-download-primary")).toHaveText("检查资源");
  await expect(preload).toBeEnabled();
});

test("按语种下载可并发提交, 名称使用正文字体", async ({ page }) => {
  await page.goto("/#update");
  const update = page.locator("#page-update");
  await update.locator(".resource-options__toggle").click();
  const english = update.locator("#resource-download-pakchunk103");
  const korean = update.locator("#resource-download-pakchunk104");
  await expect(english).toHaveText("下载");
  await english.click();
  await expect(english).toBeDisabled();
  await expect(korean).toBeEnabled();
  await korean.click();
  await expect(korean).toBeDisabled();
  await expect(english).toHaveText("更新");
  await expect(korean).toHaveText("更新");
  await expect(update.locator(".resource-options__summary")).toHaveText(
    "4/4 项 · 6.9 GiB",
  );
  const typography = await update
    .locator(".resource-row__tag")
    .first()
    .evaluate((node) => ({
      name: getComputedStyle(node).fontFamily,
      body: getComputedStyle(document.body).fontFamily,
      spacing: getComputedStyle(node).letterSpacing,
    }));
  expect(typography.name).toBe(typography.body);
  expect(typography.spacing).toBe("normal");
});

test("预下载完成后可校验并删除缓存", async ({ page }) => {
  await page.goto("/#update");
  const update = page.locator("#page-update");
  await expect(update.locator("#game-preload")).toHaveText("资源预下载 1.5.0");
  await update.locator("#game-preload").click();
  await expect(
    update.getByRole("heading", { name: "资源预下载已完成" }),
  ).toBeVisible();
  await expect(update.locator("#game-preload")).toHaveText("校验预下载");
  await update.locator("#game-preload-delete").click();
  await expect(page.getByText("删除 1.5.0 的预下载缓存")).toBeVisible();
  await page.getByRole("button", { name: "删除", exact: true }).click();
  await expect(update.locator("#game-preload")).toHaveText("资源预下载 1.5.0");
  await expect(update.locator("#game-preload-delete")).toHaveCount(0);
});

test("更新摘要、暂停继续与窄面板布局", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/#update");
  const update = page.locator("#page-update");
  const primary = page.locator("#game-download-primary");
  const budget = update.locator(".disk-budget");
  const homeStatus = page.locator("#home-status");
  await expect(primary).toHaveText("检查通过");
  await expect(budget).toHaveCount(0);
  await expect(homeStatus).toHaveCSS("visibility", "hidden");
  await update.getByRole("button", { name: "完整校验", exact: true }).click();
  await expect(primary).toHaveText("更新游戏");
  await expect(update.locator("progress")).toHaveCount(0);
  await expect(budget).toContainText("预计需要 17.2 MiB");
  await expect(budget).toContainText("可用 256 GiB");
  await expect(budget).not.toHaveClass(/is-short/);
  await primary.click();
  await expect(budget).toHaveCount(0);
  await update.getByRole("button", { name: "暂停下载", exact: true }).click();
  await expect(
    update.getByRole("heading", { name: "下载已暂停" }),
  ).toBeVisible();
  await expect(homeStatus).toHaveCSS("visibility", "visible");
  await expect(homeStatus).toHaveCSS("opacity", "1");
  await expect(homeStatus).toContainText("下载已暂停, 打开更新页继续");
  await expect(primary).toHaveText("继续更新游戏");
  await expect(update.locator("progress")).toHaveCount(1);
  await expect(update.locator("#download-install-root")).toBeEnabled();
  await expect(
    update.locator(
      ".update-status-view > .view-stack__page:not([aria-hidden])",
    ),
  ).toHaveCSS("opacity", "1");
  await expect(page.locator(".notification-stack")).toHaveAttribute(
    "data-phase",
    "0",
  );
  for (
    let remaining = await page.locator(".notification-slot").count();
    remaining > 0;
    remaining--
  ) {
    await page
      .getByRole("button", { name: "关闭通知", exact: true })
      .first()
      .click();
    await expect(page.locator(".notification-slot")).toHaveCount(remaining - 1);
  }
  await page.screenshot({ path: "test-results/update-paused.png" });
  await primary.click();
  await expect(primary).toHaveText("检查通过");
  await expect(
    update.locator(
      ".update-status-view > .view-stack__page:not([aria-hidden])",
    ),
  ).toHaveCSS("opacity", "1");
  await update.locator("#update-quick").hover();
  await page.screenshot({ path: "test-results/update-ready.png" });
  await page.setViewportSize({ width: 640, height: 520 });
  await expect(update.locator("#download-install-root")).toBeVisible();
  const overflow = await update.locator(".update-content").evaluate((el) => ({
    gutter: (el as HTMLElement).offsetWidth - el.clientWidth,
    horizontal: el.scrollWidth - el.clientWidth,
    scrollbar: getComputedStyle(el).scrollbarWidth,
  }));
  expect(overflow.gutter).toBe(0);
  expect(overflow.horizontal).toBeLessThanOrEqual(1);
  expect(overflow.scrollbar).toBe("none");
  await page.screenshot({ path: "test-results/update-compact.png" });
  expect(errors).toEqual([]);
});

test("语音资源支持卸载, 重新下载和按语种清理预下载", async ({ page }) => {
  await page.goto("/#update");
  const update = page.locator("#page-update");
  await update.locator(".resource-options__toggle").click();
  await expect(update.locator("#resource-options-title")).toHaveText(
    "语音资源管理",
  );
  await update.locator("#resource-remove-pakchunk101").click();
  await expect(
    page.getByRole("heading", { name: "卸载语音资源", exact: true }),
  ).toBeVisible();
  await page.locator("#voice-resource-remove-confirm").click();
  const chinese = update.locator("#resource-download-pakchunk101");
  await expect(chinese).toHaveText("下载");
  await chinese.click();
  await expect(chinese).toBeDisabled();
  await expect(chinese).toHaveText("更新");
  await expect(update.locator("#resource-remove-pakchunk101")).toBeVisible();
  await update.locator("#game-preload").click();
  await expect(update.locator("#game-preload")).toHaveText("校验预下载");
  await update.locator("#resource-preload-remove-pakchunk101").click();
  await page.locator("#voice-resource-remove-confirm").click();
  await expect(
    update.locator("#resource-preload-remove-pakchunk101"),
  ).toHaveCount(0);
  await expect(
    update.locator("#resource-preload-remove-pakchunk102"),
  ).toBeVisible();
  await expect(chinese).toHaveText("更新");
  await expect(chinese).toBeDisabled();
  await expect(update.locator("#game-preload")).toHaveText("资源预下载 1.5.0");
  await page.setViewportSize({ width: 860, height: 760 });
  await expect(update.locator("#resource-options-title")).toBeVisible();
  await page.screenshot({
    path: "/tmp/oddity-75-voice-buttons.png",
    fullPage: true,
  });
});
