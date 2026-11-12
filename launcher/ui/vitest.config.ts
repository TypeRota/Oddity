import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config.ts";

export default mergeConfig(
  viteConfig,
  defineConfig({
    resolve: {
      dedupe: ["vue", "vitest", "@vue/test-utils", "animejs"],
    },
    server: { fs: { allow: [new URL("../..", import.meta.url).pathname] } },
    test: {
      environment: "jsdom",
      include: ["tests/unit/**/*.test.ts"],
      setupFiles: ["./tests/unit/setup.ts"],
      coverage: {
        provider: "v8",
        // 包 UI 位于项目根之外, 默认不统计; 打开后由下面的跨 root glob 决定范围.
        allowExternal: true,
        include: [
          "src/**/*.ts",
          "src/**/*.vue",
          "../packages/*/ui/src/**/*.ts",
          "../packages/*/ui/src/**/*.vue",
        ],
        // 生成物由对应生成器的 --check 与 Rust 测试覆盖, 不计入手写覆盖率.
        exclude: [
          "src/types/**",
          "src/core/generated/**",
          "src/host/generated/**",
        ],
        reporter: ["text", "html", "lcov", "json-summary"],
        reportOnFailure: true,
        thresholds: {
          // DOM 动效按 UI 参考目标统计, 核心逻辑仍逐文件强制检查。
          "src/**/!(useNotificationArrival|usePlayerMotion|useScenePalette).ts":
            {
              lines: 80,
              branches: 70,
              perFile: true,
            },
        },
      },
    },
  }),
);
