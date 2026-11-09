import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  archVersion,
  debFileName,
  debVersion,
  gentooVersion,
  isPrerelease,
  parseVersion,
  rpmMetadata,
} from "./release-version.mjs";

test("稳定版保持原版本号", () => {
  assert.equal(debVersion("0.1.0"), "0.1.0");
  assert.deepEqual(rpmMetadata("0.1.0"), { version: "0.1.0", release: "1" });
  assert.equal(archVersion("0.1.0"), "0.1.0");
  assert.equal(isPrerelease("0.1.0"), false);
});

test("预发布版本按 Debian 规则转换", () => {
  assert.equal(debVersion("0.1.0-alpha.2"), "0.1.0~alpha.2");
  assert.equal(debVersion("0.1.0-rc.1"), "0.1.0~rc.1");
  assert.equal(isPrerelease("0.1.0-alpha.2"), true);
});

test("deb 资产文件名使用 GitHub 规范化后的形式", () => {
  assert.equal(debFileName("0.1.0"), "oddity_0.1.0_amd64.deb");
  assert.equal(
    debFileName("0.1.0-alpha.2"),
    "oddity_0.1.0.alpha.2_amd64.deb",
  );
  assert.equal(
    debFileName("0.1.0-alpha-2"),
    "oddity_0.1.0.alpha.2_amd64.deb",
  );
  assert.equal(
    debFileName("0.1.0+build.7"),
    "oddity_0.1.0+build.7_amd64.deb",
  );
});

test("预发布版本按 RPM 规则拆分 Version 和 Release", () => {
  assert.deepEqual(rpmMetadata("0.1.0-alpha.2"), {
    version: "0.1.0",
    release: "0.alpha.2",
  });
  assert.deepEqual(rpmMetadata("0.1.0-beta.10"), {
    version: "0.1.0",
    release: "0.beta.10",
  });
});

test("预发布版本按 Arch 规则保留可排序分段", () => {
  assert.equal(archVersion("0.1.0-alpha.2"), "0.1.0alpha.2");
  assert.equal(archVersion("0.1.0-rc.1"), "0.1.0rc.1");
});

test("预发布版本按 Gentoo 规则转换", () => {
  assert.equal(gentooVersion("0.1.0-alpha.2"), "0.1.0_alpha2");
  assert.equal(gentooVersion("0.1.0-beta.10"), "0.1.0_beta10");
  assert.equal(gentooVersion("0.1.0-rc.1"), "0.1.0_rc1");
  assert.equal(gentooVersion("0.1.0"), "0.1.0");
});

test("预发布标识中的连字符会被替换", () => {
  assert.equal(debVersion("0.1.0-alpha-2"), "0.1.0~alpha.2");
  assert.deepEqual(rpmMetadata("0.1.0-alpha-2"), {
    version: "0.1.0",
    release: "0.alpha.2",
  });
  assert.equal(archVersion("0.1.0-alpha-2"), "0.1.0alpha.2");
});

test("构建元数据不会破坏包版本", () => {
  assert.deepEqual(parseVersion("0.1.0+build.7"), {
    core: "0.1.0",
    prerelease: "",
    build: "build.7",
  });
  assert.equal(debVersion("0.1.0+build.7"), "0.1.0+build.7");
  assert.equal(archVersion("0.1.0+build.7"), "0.1.0+build.7");
});

test("非法版本直接报错", () => {
  assert.throws(() => parseVersion("0.1"), /SemVer/);
  assert.throws(() => parseVersion("v0.1.0"), /SemVer/);
});

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function releaseVerificationScript() {
  const workflow = readFileSync(join(repoRoot, ".gitea/workflows/release.yml"), "utf8");
  const step = workflow.match(/      - name: 校验标签与版本\r?\n[\s\S]*?        run: \|\r?\n((?:(?: {10}.*)?\r?\n)+)/);
  assert.ok(step, "找不到发布 workflow 的 tag 校验步骤");
  return step[1].replace(/^ {10}/gm, "");
}

function verifyRelease({ version, target, releaseBranches = [] }) {
  const root = mkdtempSync(join(tmpdir(), "oddity-release-test-"));
  const origin = join(root, "origin.git");
  const source = join(root, "source");
  const checkout = join(root, "checkout");
  const env = {
    ...process.env,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_TERMINAL_PROMPT: "0",
  };
  const git = (cwd, ...args) => execFileSync("git", args, { cwd, env, stdio: "pipe", encoding: "utf8" }).trim();
  try {
    git(root, "init", "--bare", "-b", "main", origin);
    git(root, "init", "-b", "main", source);
    git(source, "config", "user.name", "Release Test");
    git(source, "config", "user.email", "test@example.invalid");
    mkdirSync(join(source, "scripts"));
    mkdirSync(join(source, "launcher/ui"), { recursive: true });
    copyFileSync(join(repoRoot, "scripts/check-release-version.mjs"), join(source, "scripts/check-release-version.mjs"));
    writeFileSync(join(source, "launcher/Cargo.toml"), `[workspace.package]\nversion = "${version}"\n`);
    writeFileSync(join(source, "launcher/ui/package.json"), JSON.stringify({ version }));
    git(source, "add", ".");
    git(source, "commit", "-m", "base");
    const base = git(source, "rev-parse", "HEAD");
    git(source, "commit", "--allow-empty", "-m", "main update");
    const commits = { "main/ancestor": base, main: git(source, "rev-parse", "HEAD") };

    for (const branch of releaseBranches) {
      git(source, "switch", "-c", `release/${branch}`, base);
      git(source, "commit", "--allow-empty", "-m", `release ${branch}`);
      commits[`release/${branch}/ancestor`] = git(source, "rev-parse", "HEAD");
      git(source, "commit", "--allow-empty", "-m", `release ${branch} update`);
      commits[`release/${branch}`] = git(source, "rev-parse", "HEAD");
    }
    git(source, "switch", "-c", "unrelated", base);
    git(source, "commit", "--allow-empty", "-m", "unrelated update");
    commits.unrelated = git(source, "rev-parse", "HEAD");

    assert.ok(commits[target], `未知的测试提交: ${target}`);
    const tag = `oddity-v${version}`;
    git(source, "tag", "-a", tag, "-m", tag, commits[target]);
    git(source, "remote", "add", "origin", origin);
    git(source, "push", "origin", "--all");
    git(source, "push", "origin", "--tags");
    git(root, "clone", "--single-branch", "--branch", "main", origin, checkout);
    git(checkout, "checkout", "--detach", tag);
    const output = join(root, "output");
    writeFileSync(output, "");
    const result = spawnSync("bash", ["-c", releaseVerificationScript()], {
      cwd: checkout,
      encoding: "utf8",
      env: { ...env, TAG: tag, INPUT_TAG: "", GITHUB_REF_TYPE: "tag", GITHUB_REF_NAME: tag, GITHUB_OUTPUT: output },
    });
    return { ...result, output: readFileSync(output, "utf8"), commit: commits[target] };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

for (const scenario of [
  { name: "main 上的稳定版", version: "0.1.1", target: "main", prerelease: false },
  { name: "main 祖先提交上的稳定版", version: "0.1.1", target: "main/ancestor", prerelease: false },
  { name: "main 上的预发布版", version: "0.1.1-rc.1+build.2", target: "main", prerelease: true },
  { name: "main 上构建元数据含连字符的稳定版", version: "0.1.1+build-2", target: "main", prerelease: false },
  { name: "对应维护分支末端的稳定版", version: "0.1.1", target: "release/0.1", releaseBranches: ["0.1"], prerelease: false },
  { name: "对应维护分支祖先提交上的稳定版", version: "0.1.2", target: "release/0.1/ancestor", releaseBranches: ["0.1"], prerelease: false },
  { name: "维护分支上构建元数据含连字符的稳定版", version: "0.1.2+build-3", target: "release/0.1", releaseBranches: ["0.1"], prerelease: false },
  { name: "其他 major 和 minor 对应的维护分支", version: "2.10.0", target: "release/2.10", releaseBranches: ["2.10"], prerelease: false },
  { name: "维护分支上的预发布版", version: "0.1.2-rc.1+build.3", target: "release/0.1", releaseBranches: ["0.1"], error: /预发布版本必须从 main 发布/ },
  { name: "没有维护分支且提交不在 main", version: "0.1.2", target: "unrelated", error: /维护分支 release\/0\.1 不存在/ },
  { name: "缺少对应维护分支", version: "0.1.2", target: "release/0.2", releaseBranches: ["0.2"], error: /维护分支 release\/0\.1 不存在/ },
  { name: "提交只在其他维护分支", version: "0.1.2", target: "release/0.2", releaseBranches: ["0.1", "0.2"], error: /tag .* 不在 release\/0\.1 上/ },
  { name: "提交不在 main 或对应维护分支", version: "0.1.2", target: "unrelated", releaseBranches: ["0.1"], error: /tag .* 不在 release\/0\.1 上/ },
  { name: "维护分支上的非法版本", version: "0.1.bad", target: "release/0.1", releaseBranches: ["0.1"], error: /版本号不是合法的 SemVer/ },
]) {
  test(`发布校验: ${scenario.name}`, () => {
    const result = verifyRelease(scenario);
    const diagnostics = `${result.stdout}\n${result.stderr}`;
    if (scenario.error) {
      assert.equal(result.status, 1, diagnostics);
      assert.match(diagnostics, scenario.error);
      assert.equal(result.output, "");
    } else {
      assert.equal(result.status, 0, diagnostics);
      const outputs = Object.fromEntries(result.output.trim().split("\n").map((line) => {
        const separator = line.indexOf("=");
        return [line.slice(0, separator), line.slice(separator + 1)];
      }));
      assert.equal(outputs.version, scenario.version);
      assert.equal(outputs.prerelease, String(scenario.prerelease));
      assert.equal(outputs.commit, result.commit);
    }
  });
}
