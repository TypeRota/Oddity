import { reactive } from "vue";

export interface KeyboardActions {
  cycle: () => void;
  commit: () => void;
  backspace: () => void;
  clear: () => void;
  /** 拼音模式下翻候选页, 返回是否处理. */
  page?: (delta: number) => boolean;
  /** 当前是否处于拼音模式, 供手柄提示使用. */
  pinyin?: boolean;
}

export interface FilePickerActions {
  primary: () => void;
  parent: () => void;
  reset: () => void;
}

// 屏幕键盘和文件选择框把动作注册进来, 由手柄分发层按当前界面调用.
export const gamepadActions = reactive({
  keyboard: null as KeyboardActions | null,
  picker: null as FilePickerActions | null,
});
