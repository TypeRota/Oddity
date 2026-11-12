// 只取生成物里的包预览声明; 这段路径不含组件, 不会把包内组件拉进 api.ts 的依赖图.
import { packagePreviews } from "./generated/page-entries";
import type { UiCommand } from "../core/generated/command-ids";
import type { PreviewContribution, PreviewHandler } from "../types/preview";

/**
 * 包预览注册表.
 *
 * 游戏包按命名空间声明自己负责的命令的浏览器演示与模拟数据; 宿主按当前游戏
 * 命名空间取表, 同名命令各自命中本包的演示. 没有包声明的命令仍走宿主
 * previewInvoke 的默认分支.
 */
export const packagePreviewContributions: readonly PreviewContribution[] =
  packagePreviews.map(({ preview }) => preview);

type ContributionCommands<T> = T extends { readonly commands: infer C }
  ? Extract<keyof C, UiCommand>
  : never;

type PackagePreview = (typeof packagePreviews)[number]["preview"];

/** 已由包提供演示的命令. */
export type PackagePreviewCommand = ContributionCommands<PackagePreview>;

/** 仍需宿主提供演示的命令; 新命令既没进包表也没进宿主分支时这里编译失败. */
export type HostPreviewCommand = Exclude<UiCommand, PackagePreviewCommand>;

/** 按当前游戏命名空间查找包预览; 该命名空间没有声明该命令时返回 undefined. */
export function previewHandler<C extends UiCommand>(
  command: C,
  namespace: string,
): PreviewHandler<C> | undefined {
  const contribution = packagePreviewContributions.find(
    (entry) => entry.namespace === namespace,
  );
  return contribution?.commands[command];
}
