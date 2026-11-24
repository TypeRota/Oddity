import assert from "node:assert/strict";
import { execFile, execFileSync, spawnSync } from "node:child_process";
import { once } from "node:events";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { findReusableRun } from "./reuse-ci-result.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const workflows = ["frontend-quality", "rust-quality", "generated-quality"];

function successfulRun(sha, workflow = "frontend-quality", extra = {}) {
  return {
    id: 1,
    head_sha: sha,
    path: `${workflow}.yml@refs/pull/1/head`,
    event: "pull_request",
    status: "completed",
    conclusion: "success",
    completed_at: "2026-11-18T08:00:00Z",
    ...extra,
  };
}

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "oddity-ci-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
    }).trim();
  git("init", "-q", "-b", "main");
  git("config", "user.name", "CI fixture");
  git("config", "user.email", "ci@example.invalid");
  const write = (file, text) => {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    writeFileSync(join(root, file), text);
    git("add", "--", file);
  };
  for (const file of [
    ...workflows.map((workflow) => `.gitea/workflows/${workflow}.yml`),
    "scripts/reuse-ci-result.mjs",
    ".gitea/actions/ci-result/action.yml",
  ]) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    copyFileSync(join(repository, file), join(root, file));
    git("add", "--", file);
  }
  for (const file of [
    "launcher/crates/oddity-host/src/lib.rs",
    "launcher/ui/src/App.vue",
    "launcher/ui/src/host/pages.ts",
    "launcher/CONTRACTS.md",
    "README.md",
  ])
    write(file, "initial\n");
  const commit = () => {
    git("commit", "-qm", "fixture");
    return git("rev-parse", "HEAD");
  };
  const sha = commit();
  git("update-ref", "refs/remotes/origin/main", "HEAD");
  const find = (workflow, runs, extra = {}) =>
    findReusableRun({
      root,
      workflow,
      ref: "refs/pull/1/head",
      runId: 100,
      now: new Date("2026-11-18T09:00:00Z"),
      getRuns: async () => ({ workflow_runs: runs }),
      ...extra,
    });
  return { root, git, write, commit, sha, find };
}

test("追加说明修改复用服务器上的成功记录", async (t) => {
  const { write, commit, sha, find } = fixture(t);
  write("README.md", "updated\n");
  write("launcher/crates/oddity-host/README.md", "说明\n");
  commit();
  for (const workflow of workflows) {
    assert.equal(await find(workflow, [successfulRun(sha, workflow)]), 1);
  }
});

test("代码和生成物变化使相关检查重新执行", async (t) => {
  for (const [file, affected] of [
    ["launcher/crates/oddity-host/src/lib.rs", ["rust-quality"]],
    ["launcher/ui/src/App.vue", ["frontend-quality"]],
    [
      "launcher/ui/src/host/pages.ts",
      ["frontend-quality", "generated-quality"],
    ],
    ["launcher/CONTRACTS.md", ["generated-quality"]],
    ["scripts/reuse-ci-result.mjs", workflows],
    [".gitea/actions/ci-result/action.yml", workflows],
  ]) {
    const { write, commit, sha, find } = fixture(t);
    write(file, "changed\n");
    commit();
    for (const workflow of workflows) {
      assert.equal(
        await find(workflow, [successfulRun(sha, workflow)]),
        affected.includes(workflow) ? null : 1,
        `${file}: ${workflow}`,
      );
    }
  }
});

test("删除, 重命名和权限变化都会使检查失效", async (t) => {
  const { git, commit, sha, find } = fixture(t);
  let previous = sha;
  for (const args of [
    ["update-index", "--chmod=+x", "launcher/crates/oddity-host/src/lib.rs"],
    [
      "mv",
      "launcher/crates/oddity-host/src/lib.rs",
      "launcher/crates/oddity-host/src/renamed.rs",
    ],
    ["rm", "-f", "launcher/crates/oddity-host/src/renamed.rs"],
  ]) {
    git(...args);
    const current = commit();
    assert.equal(
      await find("rust-quality", [successfulRun(previous, "rust-quality")]),
      null,
    );
    previous = current;
  }
});

test("Rust 增量覆盖率基准变化必须重新检查", async (t) => {
  const { git, write, commit, sha, find } = fixture(t);
  write("README.md", "updated\n");
  commit();
  assert.equal(
    await find("rust-quality", [successfulRun(sha, "rust-quality")]),
    1,
  );
  git("update-ref", "refs/remotes/origin/main", "HEAD");
  assert.equal(
    await find("rust-quality", [successfulRun(sha, "rust-quality")]),
    null,
  );
  assert.equal(await find("frontend-quality", [successfulRun(sha)]), 1);
});

test("只复用相同 PR 和工作流当天已完成的成功运行", async (t) => {
  const { sha, find } = fixture(t);
  const runs = [
    successfulRun(sha, "frontend-quality", {
      path: "frontend-quality.yml@refs/pull/2/head",
    }),
    successfulRun(sha, "generated-quality"),
    successfulRun(sha, "frontend-quality", { event: "workflow_dispatch" }),
    successfulRun(sha, "frontend-quality", { status: "in_progress" }),
    successfulRun(sha, "frontend-quality", { conclusion: "failure" }),
    successfulRun(sha, "frontend-quality", { conclusion: "cancelled" }),
    successfulRun(sha, "frontend-quality", {
      completed_at: "2026-11-17T08:00:00Z",
    }),
    successfulRun(sha, "frontend-quality", { id: 100 }),
  ];
  assert.equal(await find("frontend-quality", runs), null);
  assert.equal(
    await find("frontend-quality", [...runs, successfulRun(sha)]),
    1,
  );
});

test("成功记录跨页查询", async (t) => {
  const { sha, find } = fixture(t);
  const pages = [];
  const result = await find("frontend-quality", [], {
    getRuns: async (page) => {
      pages.push(page);
      return {
        workflow_runs:
          page === 1
            ? Array.from({ length: 50 }, () =>
                successfulRun(sha, "generated-quality"),
              )
            : [successfulRun(sha)],
      };
    },
  });
  assert.equal(result, 1);
  assert.deepEqual(pages, [1, 2]);
});

test("另一台 runner 的浅检出可以补取成功提交并复用", async (t) => {
  const { root, git, write, commit, find } = fixture(t);
  git("config", "uploadpack.allowFilter", "true");
  write("README.md", "previous documentation only\n");
  const sha = commit();
  write("README.md", "updated\n");
  commit();
  const runner = join(root, "another-runner");
  execFileSync("git", [
    "clone",
    "--quiet",
    "--depth=1",
    pathToFileURL(root).href,
    runner,
  ]);
  assert.notEqual(
    spawnSync("git", ["cat-file", "-e", sha], { cwd: runner, stdio: "ignore" })
      .status,
    0,
  );
  assert.equal(
    await find("frontend-quality", [successfulRun(sha)], { root: runner }),
    1,
  );
  assert.equal(
    spawnSync("git", ["cat-file", "-e", sha], { cwd: runner, stdio: "ignore" })
      .status,
    0,
  );
  assert.match(
    execFileSync("git", ["rev-list", "--objects", "--missing=print", sha], {
      cwd: runner,
      encoding: "utf8",
    }),
    /^\?/m,
  );
});

test("查询返回异常时不能复用结果", async (t) => {
  const { sha, find } = fixture(t);
  await assert.rejects(
    find("frontend-quality", [], {
      getRuns: async () => ({ workflow_runs: null }),
    }),
    /无效的运行列表/,
  );
  await assert.rejects(
    find("frontend-quality", [
      successfulRun(sha, "frontend-quality", { head_sha: "invalid" }),
    ]),
    /无效的成功运行记录/,
  );
  await assert.rejects(
    find("frontend-quality", [], {
      getRuns: async () => {
        throw new Error("unavailable");
      },
    }),
    /unavailable/,
  );
});

test("命令行查询 Gitea 并在 API 不可用时保留完整执行", async (t) => {
  const { root, write, commit, sha } = fixture(t);
  write("README.md", "updated\n");
  commit();
  let unavailable = false;
  const server = createServer((request, response) => {
    assert.equal(request.headers.authorization, "token fixture");
    const url = new URL(request.url, "http://localhost");
    assert.equal(
      url.pathname,
      "/api/v1/repos/fixture/repo/actions/workflows/frontend-quality.yml/runs",
    );
    assert.equal(url.searchParams.get("status"), "success");
    assert.equal(url.searchParams.get("event"), "pull_request");
    assert.equal(url.searchParams.get("page"), "1");
    response.writeHead(unavailable ? 503 : 200, {
      "Content-Type": "application/json",
    });
    response.end(
      JSON.stringify({
        workflow_runs: [
          successfulRun(sha, "frontend-quality", {
            completed_at: new Date().toISOString(),
          }),
        ],
      }),
    );
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const output = join(root, "output");
  const options = {
    env: {
      ...process.env,
      GITHUB_API_URL: `http://127.0.0.1:${server.address().port}/api/v1`,
      GITHUB_REPOSITORY: "fixture/repo",
      GITHUB_REF: "refs/pull/1/head",
      GITHUB_RUN_ID: "100",
      GITHUB_OUTPUT: output,
      GITEA_TOKEN: "fixture",
    },
  };
  const args = [join(root, "scripts/reuse-ci-result.mjs"), "frontend-quality"];
  await promisify(execFile)(process.execPath, args, options);
  assert.equal(readFileSync(output, "utf8"), "reused=false\nreused=true\n");
  unavailable = true;
  writeFileSync(output, "");
  await assert.rejects(
    promisify(execFile)(process.execPath, args, options),
    /HTTP 503/,
  );
  assert.equal(readFileSync(output, "utf8"), "reused=false\n");
});
