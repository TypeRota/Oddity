import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export async function findReusableRun({
  root,
  workflow,
  ref,
  runId,
  baseRef = "main",
  now = new Date(),
  getRuns,
}) {
  if (!/^(frontend|rust|generated)-quality$/.test(workflow)) {
    throw new Error("不支持的质量检查工作流.");
  }
  if (!/^refs\/pull\/\d+\/head$/.test(ref) || !Number.isSafeInteger(runId)) {
    throw new Error("缺少有效的 PR 运行上下文.");
  }
  const source = readFileSync(
    join(root, `.gitea/workflows/${workflow}.yml`),
    "utf8",
  );
  // 路径列表与触发条件共用, 仅接受每行一个 JSON 字符串的列表.
  const section = source.match(/^    paths:\r?\n((?:      - .+\r?\n)+)/m);
  if (!section) throw new Error("工作流缺少 paths 列表.");
  const paths = section[1]
    .trimEnd()
    .split(/\r?\n/)
    .map((line) => {
      const pattern = JSON.parse(line.slice(8));
      if (typeof pattern !== "string" || !pattern) {
        throw new Error("工作流 paths 必须是非空字符串.");
      }
      return pattern.startsWith("!")
        ? `:(top,glob,exclude)${pattern.slice(1)}`
        : `:(top,glob)${pattern}`;
    });
  const day = now.toISOString().slice(0, 10);
  const base =
    workflow === "rust-quality"
      ? execFileSync("git", ["merge-base", `origin/${baseRef}`, "HEAD"], {
          cwd: root,
          encoding: "utf8",
        })
      : null;
  for (let page = 1; ; page++) {
    const { workflow_runs: runs } = await getRuns(page);
    if (!Array.isArray(runs)) throw new Error("Gitea 返回了无效的运行列表.");
    for (const run of runs) {
      if (
        run.path !== `${workflow}.yml@${ref}` ||
        run.event !== "pull_request" ||
        run.status !== "completed" ||
        run.conclusion !== "success" ||
        run.id === runId
      )
        continue;
      if (
        !Number.isSafeInteger(run.id) ||
        typeof run.head_sha !== "string" ||
        !/^[a-f0-9]{40}$/.test(run.head_sha) ||
        typeof run.completed_at !== "string" ||
        !Number.isFinite(Date.parse(run.completed_at))
      )
        throw new Error("Gitea 返回了无效的成功运行记录.");
      if (new Date(run.completed_at).toISOString().slice(0, 10) !== day)
        continue;
      const present = spawnSync(
        "git",
        ["cat-file", "-e", `${run.head_sha}^{commit}`],
        { cwd: root, stdio: "ignore" },
      );
      if (present.status !== 0) {
        // 浅检出可能没有成功运行对应的提交, 只补取比对所需对象.
        const args = ["fetch", "--no-tags", "--filter=blob:none"];
        if (!base) args.push("--depth=1");
        execFileSync("git", [...args, "origin", run.head_sha], {
          cwd: root,
          stdio: "inherit",
        });
      }
      const diff = spawnSync(
        "git",
        [
          "diff",
          "--quiet",
          "--no-renames",
          run.head_sha,
          "HEAD",
          "--",
          ...paths,
        ],
        { cwd: root, stdio: "inherit" },
      );
      if (diff.status === 1) continue;
      if (diff.status !== 0) throw new Error("无法比对检查输入.");
      // 增量覆盖率还取决于 PR 的共同祖先.
      if (
        base &&
        execFileSync("git", ["merge-base", `origin/${baseRef}`, run.head_sha], {
          cwd: root,
          encoding: "utf8",
        }) !== base
      )
        continue;
      return run.id;
    }
    if (runs.length < 50) return null;
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  appendFileSync(process.env.GITHUB_OUTPUT, "reused=false\n");
  const workflow = process.argv[2];
  const run = await findReusableRun({
    root: fileURLToPath(new URL("../", import.meta.url)),
    workflow,
    ref: process.env.GITHUB_REF,
    runId: Number(process.env.GITHUB_RUN_ID),
    baseRef: process.env.GITHUB_BASE_REF || "main",
    getRuns: async (page) => {
      const url = new URL(
        `${process.env.GITHUB_API_URL}/repos/${process.env.GITHUB_REPOSITORY}/actions/workflows/${workflow}.yml/runs`,
      );
      url.search = new URLSearchParams({
        event: "pull_request",
        status: "success",
        page,
        limit: 50,
      });
      const response = await fetch(url, {
        headers: { Authorization: `token ${process.env.GITEA_TOKEN}` },
        signal: AbortSignal.timeout(20_000),
      });
      if (!response.ok)
        throw new Error(`查询成功运行失败: HTTP ${response.status}.`);
      return response.json();
    },
  });
  appendFileSync(process.env.GITHUB_OUTPUT, `reused=${run !== null}\n`);
  console.log(
    run === null
      ? `${workflow}: 未找到可复用成功运行.`
      : `${workflow}: 复用成功运行 #${run}.`,
  );
}
