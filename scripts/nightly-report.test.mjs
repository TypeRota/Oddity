import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  dayOf,
  failedChecks,
  nightlyChecks,
  renderReport,
  runNightly,
  selectRuns,
  unverifiedChecks,
} from "./nightly-report.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));

const checks = [
  { file: "rust-quality.yml", name: "Rust 质量检查" },
  { file: "frontend-quality.yml", name: "前端质量检查" },
];

function scheduledRun(id, file, createdAt, extra = {}) {
  return {
    id,
    path: `.gitea/workflows/${file}@refs/heads/main`,
    event: "schedule",
    status: "completed",
    conclusion: "success",
    created_at: createdAt,
    html_url: `https://gitea.example/Owner/Repo/actions/runs/${id}`,
    ...extra,
  };
}

// 固定基准时间, 用例不依赖运行当天。
const NOW = new Date("2026-11-20T03:00:00Z");
const DAY = dayOf(NOW);

// 北京时间 5:00 的定时运行落在 UTC 的前一天。
function fixtureRuns() {
  return [
    scheduledRun(901, "rust-quality.yml", "2026-11-19T21:00:54Z"),
    scheduledRun(902, "frontend-quality.yml", "2026-11-19T21:00:56Z"),
    scheduledRun(900, "rust-quality.yml", "2026-11-18T21:00:54Z", {
      conclusion: "failure",
    }),
  ];
}

test("按北京时间归集当天最新的定时运行", () => {
  const { results, pending } = selectRuns(checks, fixtureRuns(), DAY);
  assert.equal(pending, false);
  assert.deepEqual(
    results.map(({ file, run }) => [file, run?.id]),
    [
      ["rust-quality.yml", 901],
      ["frontend-quality.yml", 902],
    ],
  );
});

test("忽略其它分支, 其它事件和手动触发", () => {
  const runs = [
    scheduledRun(1, "rust-quality.yml", "2026-11-19T21:00:00Z", {
      path: ".gitea/workflows/rust-quality.yml@refs/pull/1/head",
    }),
    scheduledRun(2, "rust-quality.yml", "2026-11-19T21:00:00Z", {
      event: "workflow_dispatch",
    }),
    scheduledRun(3, "frontend-quality.yml", "2026-11-18T21:00:00Z"),
  ];
  const { results } = selectRuns(checks, runs, DAY);
  assert.equal(results[0].run, null);
  assert.equal(results[1].run, null);
});

test("运行尚未结束时保持等待", () => {
  const runs = fixtureRuns();
  runs[0] = { ...runs[0], status: "running", conclusion: null };
  assert.equal(selectRuns(checks, runs, DAY).pending, true);
});

test("未完成的运行不算失败, 也不算通过", () => {
  const results = [
    { name: "A", run: { status: "completed", conclusion: "success" } },
    { name: "B", run: { status: "completed", conclusion: "cancelled" } },
    { name: "C", run: { status: "queued", conclusion: null } },
    { name: "D", run: null },
  ];
  assert.deepEqual(
    failedChecks(results).map(({ name }) => name),
    ["B"],
  );
  assert.deepEqual(
    unverifiedChecks(results).map(({ name }) => name),
    ["C", "D"],
  );
});

test("报告列出失败检查与运行链接", () => {
  const body = renderReport({
    commitUrl: COMMIT_URL,
    results: [
      {
        name: "Rust 质量检查",
        run: {
          id: 7,
          status: "completed",
          conclusion: "failure",
          html_url: `${PUBLIC_RUN}/7`,
        },
      },
      { name: "前端质量检查", run: null },
    ],
  });
  assert.match(body, /\/commit\/abc123/);
  assert.match(body, /\[Rust 质量检查\]\(\S+\/runs\/7\): 失败/);
  assert.match(body, /前端质量检查: 当天没有运行/);
});

const PUBLIC_RUN = "https://gitea.example/Owner/Repo/actions/runs";
const COMMIT_URL = "https://gitea.example/Owner/Repo/commit/abc123";

function fakeClient(overrides = {}) {
  const calls = [];
  const client = {
    commitUrl: async () => COMMIT_URL,
    listScheduledRuns: async () => fixtureRuns(),
    findOpenIssue: async () => null,
    comment: async (number, body) => calls.push(["comment", number, body]),
    close: async (number) => calls.push(["close", number]),
    createIssue: async ({ title, body, label }) => {
      calls.push(["create", title, body, label]);
      return { number: 12 };
    },
    ...overrides,
  };
  return { client, calls };
}

test("检查全部通过时不创建 Issue", async () => {
  const { client, calls } = fakeClient();
  const outcome = await runNightly({
    client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 0,
  });
  assert.equal(outcome.action, "pass");
  assert.deepEqual(calls, []);
});

test("检查失败时创建 Issue 并带上 ci 标签", async () => {
  const runs = fixtureRuns();
  runs[1] = { ...runs[1], conclusion: "failure" };
  const { client, calls } = fakeClient({ listScheduledRuns: async () => runs });
  const outcome = await runNightly({
    client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 0,
  });
  assert.equal(outcome.action, "create");
  assert.equal(calls[0][0], "create");
  assert.equal(calls[0][1], "[NIGHTLY] 每日检查未通过");
  assert.equal(calls[0][3], "ci");
  assert.match(calls[0][2], /前端质量检查/);
});

test("已有 Issue 时追加评论, 恢复后关闭", async () => {
  const runs = fixtureRuns();
  runs[0] = { ...runs[0], conclusion: "failure" };
  const issue = { number: 34 };
  const failing = fakeClient({
    listScheduledRuns: async () => runs,
    findOpenIssue: async () => issue,
  });
  const outcome = await runNightly({
    client: failing.client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 0,
  });
  assert.equal(outcome.action, "comment");
  assert.deepEqual(failing.calls[0].slice(0, 2), ["comment", 34]);

  const passing = fakeClient({ findOpenIssue: async () => issue });
  const recovered = await runNightly({
    client: passing.client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 0,
  });
  assert.equal(recovered.action, "close");
  assert.deepEqual(
    passing.calls.map(([kind, number]) => [kind, number]),
    [
      ["comment", 34],
      ["close", 34],
    ],
  );
});

test("等待期间轮询直到运行结束", async () => {
  let polls = 0;
  const { client } = fakeClient({
    listScheduledRuns: async () => {
      polls += 1;
      if (polls === 1) {
        const runs = fixtureRuns();
        runs[0] = { ...runs[0], status: "running", conclusion: null };
        return runs;
      }
      return fixtureRuns();
    },
  });
  const outcome = await runNightly({
    client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 5,
    wait: async () => {},
  });
  assert.equal(outcome.action, "pass");
  assert.equal(polls, 2);
});

test("仓库中的定时检查都被纳入汇总", () => {
  const files = nightlyChecks().map(({ file }) => file);
  assert.ok(files.includes("rust-quality.yml"));
  assert.ok(files.includes("frontend-quality.yml"));
  assert.ok(!files.includes("nightly-alert.yml"));
});

test("命令行入口按运行结果创建 Issue 并写入摘要", async (t) => {
  const requests = [];
  const runs = fixtureRuns();
  runs[0] = { ...runs[0], conclusion: "failure" };
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      requests.push({ method: req.method, url: req.url, body });
      const send = (payload) => {
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify(payload));
      };
      const path = req.url.split("?")[0];
      if (path === "/api/v1/repos/Owner/Repo")
        send({ html_url: "https://gitea.example/Owner/Repo" });
      // Gitea 把运行列表包在 workflow_runs 里, 其余列表接口返回裸数组。
      else if (path === "/api/v1/repos/Owner/Repo/actions/runs")
        send({ total_count: runs.length, workflow_runs: runs });
      else if (path === "/api/v1/repos/Owner/Repo/issues") {
        if (req.method === "POST") send({ number: 77 });
        else send([]);
      } else if (path === "/api/v1/repos/Owner/Repo/labels")
        send([{ id: 9, name: "ci" }]);
      else send({});
    });
  });
  server.listen(0);
  t.after(() => server.close());
  await once(server, "listening");
  const summary = join(tmpdir(), `nightly-summary-${process.pid}-fixture.txt`);
  t.after(() => rmSync(summary, { force: true }));
  const child = spawn(
    process.execPath,
    [join(repository, "scripts/nightly-report.mjs")],
    {
      cwd: repository,
      env: {
        ...process.env,
        GITHUB_API_URL: `http://127.0.0.1:${server.address().port}/api/v1`,
        GITHUB_REPOSITORY: "Owner/Repo",
        GITHUB_SERVER_URL: "https://gitea.example",
        GITHUB_SHA: "abc123",
        GITHUB_STEP_SUMMARY: summary,
        GITEA_TOKEN: "fixture",
        NIGHTLY_WAIT_MINUTES: "0",
        NIGHTLY_NOW: NOW.toISOString(),
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let output = "";
  child.stdout.on("data", (chunk) => (output += chunk));
  child.stderr.on("data", (chunk) => (output += chunk));
  const [code] = await once(child, "close");
  assert.equal(code, 0, output);
  const created = requests.find(
    (request) =>
      request.method === "POST" &&
      request.url === "/api/v1/repos/Owner/Repo/issues",
  );
  assert.ok(created, "应创建 Issue");
  const payload = JSON.parse(created.body);
  assert.equal(payload.title, "[NIGHTLY] 每日检查未通过");
  assert.deepEqual(payload.labels, [9]);
  assert.match(payload.body, /Rust 质量检查/);
  assert.match(
    readFileSync(summary, "utf8"),
    /每日检查未通过, 已创建 Issue #77/,
  );
});

test("运行列表缺少 workflow_runs 时报错而不是当作空列表", async (t) => {
  const server = createServer((req, res) => {
    res.writeHead(200, { "Content-Type": "application/json" });
    const path = req.url.split("?")[0];
    res.end(
      JSON.stringify(
        path === "/api/v1/repos/Owner/Repo"
          ? { html_url: "https://gitea.example/Owner/Repo" }
          : { total_count: 0 },
      ),
    );
  });
  server.listen(0);
  t.after(() => server.close());
  await once(server, "listening");
  const child = spawn(
    process.execPath,
    [join(repository, "scripts/nightly-report.mjs")],
    {
      cwd: repository,
      env: {
        ...process.env,
        GITHUB_API_URL: `http://127.0.0.1:${server.address().port}/api/v1`,
        GITHUB_REPOSITORY: "Owner/Repo",
        GITHUB_SERVER_URL: "https://gitea.example",
        GITHUB_SHA: "abc123",
        GITHUB_STEP_SUMMARY: join(tmpdir(), "nightly-summary-unused.txt"),
        GITEA_TOKEN: "fixture",
        NIGHTLY_WAIT_MINUTES: "0",
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let output = "";
  child.stdout.on("data", (chunk) => (output += chunk));
  child.stderr.on("data", (chunk) => (output += chunk));
  const [code] = await once(child, "close");
  assert.notEqual(code, 0);
  assert.match(output, /没有返回列表/);
});

test("检查被排队卡住时不登记 Issue", async () => {
  const runs = fixtureRuns().map((run) => ({
    ...run,
    status: "queued",
    conclusion: null,
  }));
  const { client, calls } = fakeClient({ listScheduledRuns: async () => runs });
  const outcome = await runNightly({
    client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 0,
  });
  assert.equal(outcome.action, "incomplete");
  assert.deepEqual(calls, []);
  assert.deepEqual(
    outcome.unverified.map(({ name }) => name),
    ["Rust 质量检查", "前端质量检查"],
  );
});

test("只有排队卡住时保留既有 Issue 而不关闭", async () => {
  const runs = fixtureRuns().map((run) => ({
    ...run,
    status: "queued",
    conclusion: null,
  }));
  const { client, calls } = fakeClient({
    listScheduledRuns: async () => runs,
    findOpenIssue: async () => ({ number: 34 }),
  });
  const outcome = await runNightly({
    client,
    checks,
    sha: "abc123",
    now: NOW,
    waitMinutes: 0,
  });
  assert.equal(outcome.action, "incomplete");
  assert.equal(outcome.number, 34);
  assert.deepEqual(calls, []);
});

test("部分失败时不为未完成的检查编造结论", () => {
  const body = renderReport({
    commitUrl: COMMIT_URL,
    results: [
      {
        name: "Rust 质量检查",
        run: { id: 7, status: "completed", conclusion: "failure" },
      },
      { name: "前端质量检查", run: { id: 8, status: "queued" } },
    ],
  });
  assert.match(body, /Rust 质量检查.*失败/);
  assert.match(body, /前端质量检查.*未完成/);
  assert.doesNotMatch(body, /状态未知/);
});

test("Issue 正文只使用公开链接, 不暴露内部地址", () => {
  const body = renderReport({
    commitUrl: COMMIT_URL,
    results: [
      {
        name: "Rust 质量检查",
        run: {
          id: 7,
          status: "completed",
          conclusion: "failure",
          html_url: `${PUBLIC_RUN}/7`,
        },
      },
    ],
  });
  assert.match(body, /https:\/\/gitea\.example\/Owner\/Repo\/commit\/abc123/);
  assert.doesNotMatch(body, /127\.0\.0\.1|localhost/);
});

test("缺少仓库 html_url 时拒绝生成链接", async () => {
  const { client } = fakeClient({
    commitUrl: async () => {
      throw new Error("Gitea API: 仓库信息缺少 html_url.");
    },
  });
  await assert.rejects(
    () =>
      runNightly({ client, checks, sha: "abc123", now: NOW, waitMinutes: 0 }),
    /缺少 html_url/,
  );
});
