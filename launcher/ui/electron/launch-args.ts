// 第二实例事件里的 argv 会被 Chromium 重排, 原始参数改用 additionalData 传递.
const launchOptions = ["--game", "--region", "--launch-mode", "--environment"];

/// 这些参数只能配合 --launch 使用; 只要出现就交给后台校验组合是否合法.
export function isLaunchRequest(argv: string[]): boolean {
  return (
    argv.includes("--launch") ||
    argv.some((arg) =>
      launchOptions.some((name) => arg === name || arg.startsWith(`${name}=`)),
    )
  );
}

/// additionalData 保持第二实例的原始参数顺序; 事件 argv 只在缺失时兜底.
export function secondInstanceArgs(
  additionalData: unknown,
  argv: string[],
  packaged: boolean,
): string[] {
  if (additionalData && typeof additionalData === "object") {
    const forwarded = (additionalData as { argv?: unknown }).argv;
    if (
      Array.isArray(forwarded) &&
      forwarded.every((arg: unknown) => typeof arg === "string")
    )
      return forwarded as string[];
  }
  return argv.slice(packaged ? 1 : 2);
}
