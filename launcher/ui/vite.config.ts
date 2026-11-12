import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// 游戏包 UI 在构建期静态打进宿主 bundle, 不引入 pnpm workspace.
const packages = fileURLToPath(new URL("../packages", import.meta.url));
// 包 UI 访问宿主的唯一公共入口, 与 src/host-api 一一对应.
const hostApi = fileURLToPath(new URL("./src/host-api", import.meta.url));

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(
      JSON.parse(
        readFileSync(new URL("./package.json", import.meta.url), "utf8"),
      ).version,
    ),
  },
  plugins: [vue({ template: { transformAssetUrls: false } })],
  resolve: {
    alias: { "@packages": packages, "@host": hostApi },
  },
  server: {
    // 开发服务器默认只放行项目根, 包 UI 在上一级目录.
    fs: { allow: [fileURLToPath(new URL("..", import.meta.url))] },
  },
  base: "./",
  publicDir: "public",
  clearScreen: false,
  build: {
    target: "es2022",
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            {
              name: "motion",
              test: /node_modules\/.*(?:motion-dom|motion-v|motion-utils|framer-motion)/,
            },
            { name: "vue", test: /node_modules\/.*(?:@vue|vue@)/ },
            { name: "animation", test: /node_modules\/.*animejs/ },
          ],
        },
      },
    },
  },
});
