import { readFile } from "node:fs/promises";
import { syncPrLabels } from "./sync-pr-labels.mjs";

const {
  GITHUB_API_URL,
  GITHUB_EVENT_NAME,
  GITHUB_EVENT_PATH,
  GITHUB_REPOSITORY,
  GITEA_TOKEN,
} = process.env;

if (!GITEA_TOKEN) throw new Error("缺少 Gitea Actions 令牌.");

const event = JSON.parse(await readFile(GITHUB_EVENT_PATH, "utf8"));
const base = `${GITHUB_API_URL}/repos/${GITHUB_REPOSITORY}`;

async function request(method, path, body) {
  const response = await fetch(base + path, {
    method,
    headers: {
      Authorization: `token ${GITEA_TOKEN}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) {
    throw new Error(
      `Gitea API: ${method} ${path} 返回 HTTP ${response.status}.`,
    );
  }
  if (response.status !== 204) return response.json();
}

async function list(path, query = {}) {
  const limit = 50;
  const items = [];
  for (let page = 1; ; page++) {
    const params = new URLSearchParams({ ...query, page, limit });
    const batch = await request("GET", `${path}?${params}`);
    items.push(...batch);
    if (batch.length < limit) return items;
  }
}

if (GITHUB_EVENT_NAME !== "pull_request_target") {
  throw new Error(`不支持的事件: ${GITHUB_EVENT_NAME}`);
}
await syncPrLabels({ request, list }, event.pull_request.number);
