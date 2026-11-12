import type {
  BulletinFeed,
  PreviewContribution,
  PreviewHandlers,
} from "@host/api";

/**
 * fixture 包的命令预览.
 *
 * 命令由包实现, 浏览器演示也由包给出; 宿主不认识 fixture 的命令与返回形状.
 */
export const previewCommands = {
  // 与包内 chars().count() 一致, 按 Unicode 码点计数.
  fixture_echo: ({ message }) => ({ message, length: [...message].length }),
  fixture_info: () => ({
    game: "fixture",
    name: "测试游戏",
    capabilities: ["info", "echo", "failure-injection"],
  }),
  fixture_fail: ({ message }) => {
    throw new Error(message);
  },
  // 与异环同名; 宿主按当前游戏命名空间取用, 两份演示互不串用.
  fetch_bulletins: (): BulletinFeed => ({
    items: [
      {
        kind: "preview",
        title: "测试游戏公告",
        summary: "fixture 包的浏览器演示数据.",
        url: "https://example.com/fixture",
        imageUrl: null,
        publishedAt: "预览",
        source: "fixture",
      },
    ],
    errors: [],
  }),
} as const satisfies Partial<PreviewHandlers>;

export const preview = {
  namespace: "fixture",
  commands: previewCommands,
} as const satisfies PreviewContribution;
