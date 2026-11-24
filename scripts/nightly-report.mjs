import { appendFileSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// Gitea Actions 不会为 schedule 触发的运行发出 workflow_run, 每日结果只能在汇总运行里核对。
const root = fileURLToPath(new URL("../", import.meta.url));
const branch = "main";
const timeZone = "Asia/Shanghai";
const issueTitle = "[NIGHTLY] 每日检查未通过";
const issueLabel = "ci";
const pollIntervalMs = 60_000;

// 质量检查工作流的约定命名, 汇总脚本据此识别当天的定时检查。
const checkSuffix = "-quality.yml";

function readWorkflow(file) {
  return readFileSync(join(root, ".gitea/workflows", file), "utf8");
}

export function nightlyChecks() {
  return readdirSync(join(root, ".gitea/workflows"))
    .filter((file) => file.endsWith(checkSuffix))
    .flatMap((file) => {
      const source = readWorkflow(file);
      const name = source.match(/^name:\s*(.+?)\s*$/m)?.[1];
      if (!name || !/^  schedule:/m.test(source)) return [];
      return [{ file, name }];
    });
}

export function dayOf(date) {
  return new Intl.DateTimeFormat("en-CA", { timeZone }).format(new Date(date));
}

export function selectRuns(checks, runs, day) {
  const latest = new Map();
  for (const run of runs) {
    const [path, ref] = String(run.path ?? "").split("@");
    const file = path?.replace(/^\.gitea\/workflows\//, "");
    if (run.event !== "schedule" || ref !== `refs/heads/${branch}`) continue;
    if (!checks.some((check) => check.file === file)) continue;
    if (dayOf(run.created_at) !== day) continue;
    const previous = latest.get(file);
    if (
      !previous ||
      Date.parse(run.created_at) > Date.parse(previous.created_at)
    )
      latest.set(file, run);
  }
  const results = checks.map(({ file, name }) => ({
    file,
    name,
    run: latest.get(file) ?? null,
  }));
  return {
    results,
    pending: results.some(({ run }) => run?.status !== "completed"),
  };
}

const states = {
  success: "通过",
  failure: "失败",
  cancelled: "已取消",
  skipped: "已跳过",
  timed_out: "超时",
  action_required: "需要处理",
};

// 没有运行或运行尚未结束, 都不能算失败。
export function unverifiedChecks(results) {
  return results.filter(({ run }) => !run || run.status !== "completed");
}

export function failedChecks(results) {
  return results.filter(
    ({ run }) => run?.status === "completed" && run.conclusion !== "success",
  );
}

export function stateOf(run) {
  if (!run) return "未执行";
  if (run.status !== "completed") return "未完成";
  return states[run.conclusion] ?? "未预期的结论";
}

// 链接一律用 Gitea 返回的 html_url, 不能用 GITHUB_SERVER_URL: 它可能是内网回环地址。
function checkLine({ name, run }) {
  if (!run) return `- ${name}: 当天没有运行`;
  const label = run.html_url ? `[${name}](${run.html_url})` : name;
  return `- ${label}: ${stateOf(run)}`;
}

export function renderReport({ commitUrl, results }) {
  return [
    `提交: ${commitUrl}`,
    "",
    ...results.map((result) => checkLine(result)),
  ].join("\n");
}

export function renderPassing({ commitUrl }) {
  return `${commitUrl} 的每日检查全部通过。`;
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

export async function runNightly({
  client,
  checks,
  sha,
  now = new Date(),
  waitMinutes = 30,
  wait = sleep,
}) {
  const commitUrl = await client.commitUrl(sha);
  const day = dayOf(now);
  const deadline = Date.now() + waitMinutes * 60_000;
  let state = selectRuns(checks, await client.listScheduledRuns(), day);
  while (state.pending && Date.now() < deadline) {
    await wait(pollIntervalMs);
    state = selectRuns(checks, await client.listScheduledRuns(), day);
  }
  const failed = failedChecks(state.results);
  const unverified = unverifiedChecks(state.results);
  const issue = await client.findOpenIssue(issueTitle);
  if (failed.length === 0) {
    // 还有检查没跑完时, 全部通过或尚未开 Issue 都不成立。
    if (unverified.length > 0) {
      return {
        action: "incomplete",
        results: state.results,
        unverified,
        commitUrl,
        number: issue?.number ?? null,
      };
    }
    if (!issue) return { action: "pass", results: state.results, commitUrl };
    await client.comment(issue.number, renderPassing({ commitUrl }));
    await client.close(issue.number);
    return {
      action: "close",
      number: issue.number,
      results: state.results,
      commitUrl,
    };
  }
  const body = renderReport({ commitUrl, results: state.results });
  if (issue) {
    await client.comment(issue.number, body);
    return {
      action: "comment",
      number: issue.number,
      results: state.results,
      commitUrl,
    };
  }
  const created = await client.createIssue({
    title: issueTitle,
    body,
    label: issueLabel,
  });
  return {
    action: "create",
    number: created.number,
    results: state.results,
    commitUrl,
  };
}

function createClient(env) {
  const base = `${env.GITHUB_API_URL}/repos/${env.GITHUB_REPOSITORY}`;
  let context;

  async function request(method, path, body) {
    const response = await fetch(base + path, {
      method,
      headers: {
        Authorization: `token ${env.GITEA_TOKEN}`,
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(20_000),
    });
    if (!response.ok)
      throw new Error(
        `Gitea API: ${method} ${path} 返回 HTTP ${response.status}.`,
      );
    if (response.status === 204) return null;
    return response.json();
  }

  // /actions/runs 把结果包在 workflow_runs 里, 其余列表接口直接返回数组。
  async function list(path, query = {}) {
    const limit = 50;
    const items = [];
    for (let page = 1; ; page++) {
      const params = new URLSearchParams({ ...query, page, limit });
      const payload = await request("GET", `${path}?${params}`);
      const batch = Array.isArray(payload) ? payload : payload.workflow_runs;
      if (!Array.isArray(batch)) {
        throw new Error(`Gitea API: ${path} 没有返回列表.`);
      }
      items.push(...batch);
      if (batch.length < limit) return items;
    }
  }

  // 运行链接取自 Gitea 返回的 html_url, 这里只补提交链接。
  // 不能用 GITHUB_SERVER_URL: 实例可能把它设成内网回环地址。
  async function commitUrl(sha) {
    if (!sha) throw new Error("缺少提交号, 无法生成提交链接.");
    const repo = await request("GET", "");
    if (typeof repo?.html_url !== "string") {
      throw new Error("Gitea API: 仓库信息缺少 html_url.");
    }
    return `${repo.html_url}/commit/${sha}`;
  }

  return {
    commitUrl,
    // 事件过滤放在本地: 部分版本会把定时运行的事件写成首次注册的事件。
    listScheduledRuns: async () =>
      (await list("/actions/runs", { branch })).filter(
        (run) => run.event === "schedule",
      ),
    findOpenIssue: async (title) => {
      const issues = await list("/issues", { state: "open", type: "issues" });
      return issues.find((issue) => issue.title === title) ?? null;
    },
    comment: async (number, body) => {
      await request("POST", `/issues/${number}/comments`, { body });
    },
    close: async (number) => {
      await request("PATCH", `/issues/${number}`, { state: "closed" });
    },
    createIssue: async ({ title, body, label }) => {
      const labels = (await list("/labels")).filter(
        ({ name }) => name === label,
      );
      return await request("POST", "/issues", {
        title,
        body,
        labels: labels.map(({ id }) => id),
      });
    },
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const env = process.env;
  if (!env.GITEA_TOKEN) throw new Error("缺少 Gitea 令牌.");
  const client = createClient(env);
  // 手动触发只查看当前结果, 不等待同批检查跑完。
  const waitMinutes =
    env.GITHUB_EVENT_NAME === "schedule"
      ? Number(env.NIGHTLY_WAIT_MINUTES ?? 60)
      : 0;
  const outcome = await runNightly({
    client,
    checks: nightlyChecks(),
    sha: env.GITHUB_SHA,
    // 只在测试里固定"当天", 正常运行时取当前时间。
    now: env.NIGHTLY_NOW ? new Date(env.NIGHTLY_NOW) : new Date(),
    waitMinutes,
  });
  for (const { name, run } of outcome.results) {
    console.log(`${name}: ${stateOf(run)}`);
  }
  const summary = {
    pass: "每日检查全部通过.",
    close: `每日检查全部通过, 已关闭 Issue #${outcome.number}.`,
    comment: `每日检查未通过, 已追加到 Issue #${outcome.number}.`,
    create: `每日检查未通过, 已创建 Issue #${outcome.number}.`,
    incomplete: outcome.number
      ? `检查未跑完, 未登记, 保留 Issue #${outcome.number}.`
      : "检查未跑完, 未登记.",
  }[outcome.action];
  console.log(summary);
  if (env.GITHUB_STEP_SUMMARY) {
    const detail =
      outcome.action === "pass"
        ? renderPassing({ commitUrl: outcome.commitUrl })
        : renderReport({
            commitUrl: outcome.commitUrl,
            results: outcome.results,
          });
    appendFileSync(env.GITHUB_STEP_SUMMARY, `## ${summary}\n\n${detail}\n`);
  }
}
