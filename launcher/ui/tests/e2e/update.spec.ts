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

test("可选资源切换即时生效并同步预下载版本", async ({ page }) => {
  await page.goto("/#update");
  const update = page.locator("#page-update");
  const summary = update.locator(".resource-options__summary");
  await expect(summary).toHaveText("2/4 项 · 3.3 GiB");
  await expect(update.locator("#game-preload")).toHaveText("资源预下载 1.5.0");
  await expect(update.locator(".resource-row")).toHaveCount(0);
  await update.locator(".resource-options__toggle").click();
  await expect(update.locator(".resource-row__tag")).toHaveText([
    "中文语音",
    "日语语音",
    "英语语音",
    "韩语语音",
  ]);
  await expect(update).not.toContainText("pakchunk101");
  await expect(update.locator("#optional-resource-pakchunk101")).toBeDisabled();
  await update.locator("#optional-resource-pakchunk103").check();
  await expect(summary).toHaveText("3/4 项 · 5.0 GiB");
  await update.locator("#optional-resources-all").click();
  await expect(summary).toHaveText("4/4 项 · 6.9 GiB");
  await expect(update.locator("#optional-resources-all")).toHaveText(
    "取消全选",
  );
  await update.locator("#optional-resources-all").click();
  await expect(summary).toHaveText("2/4 项 · 3.3 GiB");
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
