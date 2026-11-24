import { pauseClock } from "./clock";
import { settingsCategory } from "./settings-helpers";
import { test, expect, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

async function addAccount(page: Page, phone: string, first = false) {
  await settingsCategory(page, "nte-account");
  if (first) await page.locator("#attendance-action").click();
  else {
    await settingsCategory(page, "nte-account");
    await page.locator("#attendance-manage").click();
    await page.getByRole("button", { name: "添加账号", exact: true }).click();
  }
  await page.locator("#attendance-phone").fill(phone);
  await page.locator("#attendance-password").fill("preview-only");
  await page.locator("#attendance-login").click();
  await expect(page.locator(".account-row")).toContainText([
    phone.slice(0, 3) + "****" + phone.slice(7),
  ]);
  await page.getByRole("button", { name: "关闭弹窗" }).click();
  await expect(page.locator(".modal")).toHaveCount(0);
}
async function screenshot(page: Page, name: string) {
  const directory = process.env.ODDITY_SCREENSHOT_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  await page.screenshot({
    path: join(directory, name),
    animations: "disabled",
  });
}

async function expectCenteredIcons(page: Page) {
  const offsets = await page
    .locator(".game-records__icon-button")
    .evaluateAll((buttons) =>
      buttons.map((button) => {
        const circle = button.getBoundingClientRect();
        const icon = button.querySelector("svg")!.getBoundingClientRect();
        return {
          label: button.getAttribute("aria-label"),
          x: Math.abs(icon.x + icon.width / 2 - circle.x - circle.width / 2),
          y: Math.abs(icon.y + icon.height / 2 - circle.y - circle.height / 2),
        };
      }),
    );
  expect(offsets).toHaveLength(3);
  for (const offset of offsets) {
    expect(offset.x, offset.label!).toBeLessThanOrEqual(0.5);
    expect(offset.y, offset.label!).toBeLessThanOrEqual(0.5);
  }
}

test("胶囊开合、账号切换、双向无滚动和移除同步", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  const capsule = page.locator(".game-records__capsule");
  const panel = page.locator(".game-records__panel");
  await expect(capsule).toHaveAttribute("aria-expanded", "false");
  await screenshot(page, "home-capsule-light.png");
  await capsule.click();
  await expect(page.locator(".game-records__empty")).toContainText(
    "登录塔吉多",
  );
  await screenshot(page, "home-empty-light.png");
  await page.getByRole("button", { name: /前往设置登录/ }).click();
  await expect(page.locator("#attendance-action")).toHaveText("立即登录");
  await settingsCategory(page, "appearance");
  await page.locator("#set-hero-media-trigger").click();
  await page.getByRole("option", { name: "静态照片", exact: true }).click();
  await addAccount(page, "13800138000", true);
  await addAccount(page, "13900139000");
  await expect(page.locator("#attendance-enabled")).not.toBeChecked();
  await page.locator('[data-page="home"]').click();
  await capsule.click();
  await expect(page.locator(".game-records__account")).toHaveCount(1);
  await expect(page.locator(".game-records__primary")).toContainText(
    "168 / 360",
  );
  await expectCenteredIcons(page);
  await screenshot(page, "home-panel-light.png");
  await page.getByRole("button", { name: "下一个账号" }).click();
  await expect(panel).toContainText("UID 100000002");
  await expect(page.locator(".game-records__primary")).toContainText(
    "周本剩余0 次",
  );
  await screenshot(page, "home-second-account-light.png");
  await page.getByRole("button", { name: "上一个账号" }).click();
  await expect(panel).toContainText("UID 100000001");
  await page.getByRole("button", { name: "刷新游戏状态" }).click();
  await expect(panel).toHaveAttribute("aria-busy", "false");
  await page.locator('[data-page="settings"]').click();
  await settingsCategory(page, "appearance");
  await page.locator("#set-theme-trigger").click();
  await page.getByRole("option", { name: "暗色", exact: true }).click();
  await page.locator('[data-page="home"]').click();
  await capsule.click();
  await expect(panel).toBeVisible();
  await screenshot(page, "home-panel-dark.png");
  await page.setViewportSize({ width: 980, height: 640 });
  // 视口缩小时面板高度异步归位, 等布局稳定后再测量。
  await expect
    .poll(async () => {
      const gap = await page.evaluate(() => {
        const stats = document
          .querySelector(".game-records__account")!
          .getBoundingClientRect();
        const footer = document
          .querySelector(".game-records__foot")!
          .getBoundingClientRect();
        return footer.top - stats.bottom;
      });
      return gap >= 12 && gap <= 20;
    })
    .toBe(true);
  const positions = await page.evaluate(() => {
    const panel = document
      .querySelector(".game-records__panel")!
      .getBoundingClientRect();
    const stats = document
      .querySelector(".game-records__account")!
      .getBoundingClientRect();
    const footer = document
      .querySelector(".game-records__foot")!
      .getBoundingClientRect();
    const launch = document
      .querySelector("#launch-btn")!
      .getBoundingClientRect();
    const overflow = [
      ...document.querySelectorAll<HTMLElement>(
        ".game-records__panel, .game-records__account, #page-home",
      ),
    ].map((el) => ({
      x: el.scrollWidth - el.clientWidth,
      y: el.scrollHeight - el.clientHeight,
      overflowX: getComputedStyle(el).overflowX,
      overflowY: getComputedStyle(el).overflowY,
    }));
    return {
      contentGap: footer.top - stats.bottom,
      panelRight: panel.right,
      panelBottom: panel.bottom,
      panelLeft: panel.left,
      launchRight: launch.right,
      launchBottom: launch.bottom,
      overflow,
    };
  });
  expect(positions.contentGap).toBeGreaterThanOrEqual(12);
  expect(positions.contentGap).toBeLessThanOrEqual(20);
  expect(positions.panelRight).toBeLessThanOrEqual(980);
  expect(positions.panelBottom).toBeLessThanOrEqual(640);
  expect(positions.launchBottom).toBeLessThanOrEqual(640);
  expect(positions.launchRight).toBeLessThan(positions.panelLeft);
  for (const item of positions.overflow) {
    expect(item.x).toBeLessThanOrEqual(1);
    expect(item.y).toBeLessThanOrEqual(1);
    expect(["hidden", "clip"]).toContain(item.overflowX);
    expect(["hidden", "clip"]).toContain(item.overflowY);
  }
  await expectCenteredIcons(page);
  await screenshot(page, "home-panel-compact-dark.png");
  await page.getByRole("button", { name: "下一个账号" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(panel).toContainText("UID 100000002");
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(capsule).toBeFocused();
  await capsule.click();
  await capsule.click();
  await expect(panel).toHaveCount(0);
  await capsule.click();
  await page.locator("#home-title").click();
  await expect(panel).toHaveCount(0);
  await page.locator('[data-page="settings"]').click();
  await settingsCategory(page, "nte-account");
  await page.locator("#attendance-manage").click();
  await page.getByRole("button", { name: "移除", exact: true }).first().click();
  await page.getByRole("button", { name: "确认移除", exact: true }).click();
  await expect(page.locator(".account-row")).toHaveCount(1);
  await page.getByRole("button", { name: "关闭弹窗" }).click();
  await page.locator('[data-page="home"]').click();
  await capsule.click();
  await expect(panel).not.toContainText("UID 100000001");
  await expect(panel).toContainText("UID 100000002");
  expect(errors).toEqual([]);
});

test("玩家圆形收束、展开归位、快速开合与减少动态效果", async ({ page }) => {
  await pauseClock(page);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/");
  await page.clock.runFor(1500);
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  await page.evaluate(() => {
    const probe = {
      widths: [] as number[],
      heights: [] as number[],
      xs: [] as number[],
      ys: [] as number[],
      done: false,
    };
    (window as unknown as { recordMotion: typeof probe }).recordMotion = probe;
    const start = performance.now();
    function sample() {
      const el = document.querySelector(".game-records__surface");
      if (el) {
        const box = el.getBoundingClientRect();
        probe.widths.push(box.width);
        probe.heights.push(box.height);
        const matrix = new DOMMatrixReadOnly(getComputedStyle(el).transform);
        probe.xs.push(matrix.m41);
        probe.ys.push(matrix.m42);
      }
      if (performance.now() - start < 1400) requestAnimationFrame(sample);
      else probe.done = true;
    }
    requestAnimationFrame(sample);
  });
  const capsule = page.locator(".game-records__capsule");
  const panel = page.locator(".game-records__panel");
  await capsule.click();
  await page.clock.runFor(1500);
  await page.waitForFunction(
    () =>
      (window as unknown as { recordMotion: { done: boolean } }).recordMotion
        .done,
  );
  const frames = await page.evaluate(
    () =>
      (
        window as unknown as {
          recordMotion: {
            widths: number[];
            heights: number[];
            xs: number[];
            ys: number[];
          };
        }
      ).recordMotion,
  );
  expect(frames.widths.length).toBeGreaterThan(10);
  expect(Math.min(...frames.widths)).toBeLessThanOrEqual(65);
  expect(frames.widths.at(-1)).toBeCloseTo(280, 0);
  expect(
    Math.max(...frames.heights) - Math.min(...frames.heights),
  ).toBeGreaterThan(50);
  expect(Math.min(...frames.xs)).toBeLessThan(-110);
  expect(Math.max(...frames.ys)).toBeGreaterThan(40);
  expect(frames.xs.at(-1)).toBeCloseTo(0, 0);
  expect(frames.ys.at(-1)).toBeCloseTo(0, 0);
  await page.clock.resume();
  for (let i = 0; i < 7; i++) await capsule.click();
  await expect(capsule).toHaveAttribute("aria-expanded", "false");
  await expect(panel).toHaveCount(0);
  await capsule.click();
  await expect(panel).toHaveCount(1);
  await expect(panel).not.toHaveAttribute("inert");
  await capsule.click();
  await expect(panel).toHaveCount(0);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await capsule.click();
  await expect(panel).toHaveCount(1);
  await expect
    .poll(() =>
      panel.evaluate(
        (el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).a,
      ),
    )
    .toBe(1);
});

test("玩家组件各阶段中断与窗口缩放后精确归位", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  const capsule = page.locator(".game-records__capsule");
  const surface = page.locator(".game-records__surface");
  const root = page.locator(".game-records");
  await capsule.focus();
  for (const stage of ["circle", "expand", "return"]) {
    await capsule.press("Enter");
    await page.waitForFunction((stage) => {
      const el = document.querySelector<HTMLElement>(".game-records__surface")!;
      const matrix = new DOMMatrixReadOnly(getComputedStyle(el).transform);
      return stage === "circle"
        ? el.offsetWidth < 100
        : stage === "expand"
          ? matrix.m42 > 30
          : el.offsetWidth >= 278 && matrix.m42 > 2 && matrix.m42 < 30;
    }, stage);
    await capsule.press("Enter");
    await expect(root).toHaveAttribute("data-moving", "false");
    await expect(capsule).toHaveAttribute("aria-expanded", "false");
    const box = await surface.evaluate((el) => ({
      width: el.getBoundingClientRect().width,
      height: el.getBoundingClientRect().height,
      x: new DOMMatrixReadOnly(getComputedStyle(el).transform).m41,
      y: new DOMMatrixReadOnly(getComputedStyle(el).transform).m42,
    }));
    expect(box.width).toBeCloseTo(204, 0);
    expect(box.height).toBeCloseTo(44, 0);
    expect(box.x).toBeCloseTo(0, 1);
    expect(box.y).toBeCloseTo(0, 1);
  }
  await capsule.press("Enter");
  await page.setViewportSize({ width: 980, height: 640 });
  await expect(root).toHaveAttribute("data-moving", "false");
  const bounds = await surface.boundingBox();
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(980);
  expect(bounds!.y + bounds!.height).toBeLessThan(640);
  await page.keyboard.press("Escape");
  await expect(root).toHaveAttribute("data-moving", "false");
  await expect(capsule).toHaveAttribute("aria-expanded", "false");
});

test("游戏同步独立于社区账号并保留停止后的角色状态", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?playerState=capturing");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  await page.locator(".game-records__capsule").click();
  const panel = page.locator(".game-records__panel");
  await expect(panel).toContainText("127 / 360");
  await expect(panel).toContainText("190 / 700");
  await expect(panel).toContainText("周本剩余2 次");
  await expect(panel).toContainText("169 / 2400");
  await expect(panel.locator(".game-records__sync")).toHaveCount(0);
  await page.locator('[data-page="settings"]').click();
  await settingsCategory(page, "nte-game");
  const toggle = page.locator("#set-player-state-sync");
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await page.locator('[data-page="home"]').click();
  await page.locator(".game-records__capsule").click();
  await expect(panel).toContainText("127 / 360");
  await page.setViewportSize({ width: 980, height: 640 });
  const box = await panel.boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(980);
  expect(box!.y + box!.height).toBeLessThanOrEqual(640);
  expect(errors).toEqual([]);
});

test("展开时外壳高度与按展开宽度排版的内容一致", async ({ page }) => {
  await pauseClock(page);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/?playerState=capturing");
  await page.clock.runFor(1500);
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  await page.evaluate(() => {
    const gaps: number[] = [];
    (window as unknown as { playerPanelGaps: number[] }).playerPanelGaps = gaps;
    const started = performance.now();
    function sample() {
      const surface = document.querySelector<HTMLElement>(
        ".game-records__surface",
      )!;
      const details = document.querySelector<HTMLElement>(
        ".game-records__details",
      );
      if (details && surface.offsetWidth >= 279)
        gaps.push(surface.clientHeight - 42 - details.offsetHeight);
      if (performance.now() - started < 1200) requestAnimationFrame(sample);
    }
    requestAnimationFrame(sample);
  });
  await page.locator(".game-records__capsule").click();
  await page.clock.runFor(1400);
  const gaps = await page.evaluate(
    () => (window as unknown as { playerPanelGaps: number[] }).playerPanelGaps,
  );
  expect(gaps.length).toBeGreaterThan(10);
  expect(Math.max(...gaps)).toBeLessThanOrEqual(4);
  expect(Math.min(...gaps)).toBeGreaterThanOrEqual(-4);
});

test("四项次级指标对齐且满额数值不被裁切", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?playerState=capturing");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  await page.locator(".game-records__capsule").click();
  const secondary = page.locator(".game-records__secondary");
  await secondary.evaluate((element) => {
    const values = ["700 / 700", "100", "2400 / 2400", "1000 / 1000"];
    [...element.children].forEach((item, index) => {
      item.querySelector("dd")!.textContent = values[index];
    });
  });
  for (const size of [12, 18]) {
    await secondary.locator("dd").evaluateAll((values, size) => {
      for (const value of values)
        (value as HTMLElement).style.fontSize = `${size}px`;
    }, size);
    await expect
      .poll(async () =>
        secondary.evaluate((element) => {
          const panel = element
            .closest(".game-records__surface")!
            .getBoundingClientRect();
          return [...element.querySelectorAll("dt, dd")].every((item) => {
            const box = item.getBoundingClientRect();
            return (
              item.scrollWidth <= item.clientWidth + 1 &&
              box.left >= panel.left &&
              box.right <= panel.right &&
              box.bottom <= panel.bottom
            );
          });
        }),
      )
      .toBe(true);
    const labels = await secondary.locator("dt").evaluateAll((items) =>
      items.map((item) => {
        const box = item.getBoundingClientRect();
        return { x: box.x, y: box.y };
      }),
    );
    expect(labels[0].x).toBeCloseTo(labels[2].x, 1);
    expect(labels[1].x).toBeCloseTo(labels[3].x, 1);
    expect(labels[0].y).toBeCloseTo(labels[1].y, 1);
    expect(labels[2].y).toBeCloseTo(labels[3].y, 1);
  }
});

test("游戏资源跨采集状态保留, 社区刷新仅补角色名称", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?playerState=unidentified");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  await page.locator(".game-records__capsule").click();
  await expect(page.locator(".game-records__name")).toHaveText("当前游戏");
  await expect(page.locator(".game-records__primary")).toContainText(
    "127 / 360",
  );
  await expect(page.locator(".game-records__identity")).not.toContainText(
    "UID",
  );
  await page.goto("/?playerState=quiet");
  await expect(page.locator("#app")).toHaveClass(/is-intro-done/);
  await expect(page.locator(".game-records__stamina")).toContainText(
    "体力 127 / 360",
  );
  await page.locator('[data-page="settings"]').click();
  await addAccount(page, "13800138000", true);
  await page.locator('[data-page="home"]').click();
  await page.locator(".game-records__capsule").click();
  await expect(page.locator(".game-records__primary")).toContainText(
    "127 / 360",
  );
  await expect(page.locator(".game-records__primary")).not.toContainText(
    "168 / 360",
  );
});
