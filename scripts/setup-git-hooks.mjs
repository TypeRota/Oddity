#!/usr/bin/env node
// 启用仓库自带的 Git 钩子; 每个克隆或工作树初始化后执行一次.
import { execFileSync } from "node:child_process";
import { chmodSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const hooks = join(root, ".githooks");

for (const name of readdirSync(hooks)) chmodSync(join(hooks, name), 0o755);
execFileSync("git", ["config", "core.hooksPath", ".githooks"], {
  cwd: root,
  stdio: "inherit",
});
console.log("已启用仓库钩子: .githooks");
