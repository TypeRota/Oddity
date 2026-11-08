import { test, expect } from "@playwright/test";

test.use({ video: "on" });

test("交叉模糊过渡无空白帧, 外层位置固定, 完成后可继续创建", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/#runtime-setup");
  const wizard = page.locator("#page-runtime-setup");
  await wizard.getByRole("button", { name: "自动配置", exact: true }).click();
  const umu = wizard.locator('[data-scene="umu"]');
  await expect(umu).toHaveCSS("opacity", "1");
  const samples = page.evaluate(async () => {
    const frames: {
      opacity: number;
      navY: number;
      footerY: number;
      viewportH: number;
      layers: number;
      blur: boolean;
    }[] = [];
    const start = performance.now();
    while (performance.now() - start < 4300) {
      await new Promise(requestAnimationFrame);
      const scenes = [
        ...document.querySelectorAll<HTMLElement>(".wizard-scene"),
      ];
      frames.push({
        opacity: scenes.reduce(
          (sum, el) => sum + Number(getComputedStyle(el).opacity),
          0,
        ),
        navY: document.querySelector(".wizard-stages")!.getBoundingClientRect()
          .y,
        footerY: document
          .querySelector(".wizard-footer")!
          .getBoundingClientRect().y,
        viewportH: document
          .querySelector(".wizard-viewport")!
          .getBoundingClientRect().height,
        layers: scenes.length,
        blur: scenes.some((el) =>
          /blur\((?!0px)/.test(getComputedStyle(el).filter),
        ),
      });
    }
    return frames;
  });
  for (const stage of ["proton", "runtime"]) {
    await expect(wizard.locator(`[data-scene="${stage}"]`)).toHaveCSS(
      "opacity",
      "1",
    );
    await page.screenshot({ path: `test-results/wizard-${stage}.png` });
  }
  await expect(wizard.locator('[data-scene="complete"]')).toHaveCSS(
    "opacity",
    "1",
  );
  await expect(page.locator("canvas.wizard-confetti")).toHaveAttribute(
    "data-celebrating",
    "true",
  );
  await expect
    .poll(() =>
      page
        .locator("canvas.wizard-confetti")
        .evaluate((canvas: HTMLCanvasElement) => {
          const pixels = canvas
            .getContext("2d")!
            .getImageData(0, 0, canvas.width, canvas.height).data;
          return pixels.some((value, index) => index % 4 === 3 && value > 0);
        }),
    )
    .toBe(true);
  const canvasBounds = await page
    .locator("canvas.wizard-confetti")
    .boundingBox();
  expect(canvasBounds).toEqual({ x: 0, y: 0, width: 1180, height: 760 });
  await page.screenshot({ path: "test-results/wizard-complete.png" });
  const frames = await samples;
  expect(Math.min(...frames.map((frame) => frame.opacity))).toBeGreaterThan(
    0.85,
  );
  expect(frames.some((frame) => frame.layers === 2 && frame.blur)).toBe(true);
  for (const key of ["navY", "footerY", "viewportH"] as const) {
    const values = frames.map((frame) => frame[key]);
    expect(Math.max(...values) - Math.min(...values)).toBeLessThan(0.5);
  }
  await wizard
    .getByRole("button", { name: "查看运行环境", exact: true })
    .click();
  await expect(page.locator("#runtime-environment-trigger")).toContainText(
    "默认运行环境",
  );
  await page.getByRole("button", { name: "打开向导", exact: true }).click();
  await expect(
    wizard.getByRole("heading", { name: "已有可用运行环境" }),
  ).toBeVisible();
  await expect(wizard.locator(".wizard-existing")).toContainText(
    "默认运行环境",
  );
  await expect(page.locator("canvas.wizard-confetti")).toHaveAttribute(
    "data-celebrating",
    "false",
  );
  await wizard.getByRole("button", { name: "继续创建", exact: true }).click();
  await expect(wizard.locator('[data-scene="complete"]')).toHaveCSS(
    "opacity",
    "1",
  );
  await wizard
    .getByRole("button", { name: "查看运行环境", exact: true })
    .click();
  await expect(page.locator("#runtime-environment-trigger")).toContainText(
    "运行环境 2",
  );
  expect(errors).toEqual([]);
});

test("umu 安装范围明确, 系统安装成功后锁定并复用, 新环境仍可独立创建", async ({
  page,
}) => {
  await page.goto("/#runtime-setup");
  const wizard = page.locator("#page-runtime-setup");
  await wizard.getByRole("button", { name: "安装选项", exact: true }).click();
  const scope = wizard.getByRole("checkbox", { name: "为全部用户安装 umu" });
  await expect(scope).not.toBeChecked();
  await scope.check();
  await expect(wizard).toContainText("将安装系统 umu, 需要管理员授权.");
  await wizard.getByRole("button", { name: "开始配置", exact: true }).click();
  const authorization = page.getByRole("dialog", {
    name: "安装系统组件",
    exact: true,
  });
  await expect(authorization.locator("code")).toHaveText(
    "pkexec /usr/bin/pacman -S --needed --noconfirm umu-launcher",
  );
  await authorization
    .getByRole("button", { name: "授权并安装", exact: true })
    .click();
  await expect(wizard.locator('[data-scene="complete"]')).toHaveCSS(
    "opacity",
    "1",
  );
  await wizard
    .getByRole("button", { name: "查看运行环境", exact: true })
    .click();
  await page.getByRole("button", { name: "打开向导", exact: true }).click();
  await expect(wizard.locator('[data-scene="existing"]')).toBeVisible();
  await wizard.getByRole("button", { name: "安装选项", exact: true }).click();
  await expect(scope).toBeChecked();
  await expect(scope).toBeDisabled();
  await expect(wizard).toContainText("系统 umu 已安装, 将直接复用.");
  await wizard.getByRole("button", { name: "开始配置", exact: true }).click();
  await expect(authorization).toHaveCount(0);
  await expect(wizard.locator('[data-scene="complete"]')).toHaveCSS(
    "opacity",
    "1",
  );
});

test("小窗口、手动配置与连续阶段过渡不改变布局, 无滚动条", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 640 });
  await page.goto("/#runtime-setup");
  const wizard = page.locator("#page-runtime-setup");
  await wizard.getByRole("button", { name: "安装选项", exact: true }).click();
  await expect(wizard).toHaveCSS("overflow-y", "hidden");
  await expect(
    wizard.getByRole("button", { name: "返回", exact: true }),
  ).toBeInViewport();
  await page.locator("#runtime-setup-mode-trigger").click();
  await page
    .getByRole("option", { name: "使用本地 Proton", exact: true })
    .click();
  await expect(wizard.locator("#runtime-setup-proton")).toBeInViewport();
  await page.screenshot({ path: "test-results/wizard-small-options.png" });
  await wizard.getByRole("button", { name: "返回", exact: true }).click();
  await wizard.getByRole("button", { name: "安装选项", exact: true }).click();
  await page.locator("#runtime-setup-mode-trigger").click();
  await page.getByRole("option", { name: "自动配置", exact: true }).click();
  await wizard.getByRole("button", { name: "开始配置", exact: true }).click();
  await expect(wizard.locator('[data-scene="runtime"]')).toHaveCSS(
    "opacity",
    "1",
  );
  await expect(
    wizard.getByRole("button", { name: "查看日志", exact: true }),
  ).toBeInViewport();
  await page.screenshot({ path: "test-results/wizard-small-runtime.png" });
});
