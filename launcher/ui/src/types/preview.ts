import type { UiCommand } from "../core/generated/command-ids";
import type { CommandArgs, Commands } from "./generated/commands";

/** 一支命令的浏览器演示处理; 参数与返回值沿用契约类型. */
export type PreviewHandler<C extends UiCommand> = (
  args: CommandArgs[C],
) => Commands[C] | Promise<Commands[C]>;

/** 全部渲染层命令的预览处理表; 包只声明自己负责的命令. */
export type PreviewHandlers = { [C in UiCommand]: PreviewHandler<C> };

/** 包声明的命令预览; 未列出的命令回落到宿主默认预览. */
export interface PreviewContribution {
  /** 包命名空间; 宿主按当前游戏命名空间命中, 同名命令互不串用. */
  readonly namespace: string;
  readonly commands: Partial<PreviewHandlers>;
}
