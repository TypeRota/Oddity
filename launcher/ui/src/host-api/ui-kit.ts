/** 包 UI 复用的宿主基础组件与图标. */
export { default as BaseButton } from "../components/BaseButton.vue";
export { default as BaseDisclosure } from "../components/BaseDisclosure.vue";
export { default as BaseInput } from "../components/BaseInput.vue";
export { default as BaseModal } from "../components/BaseModal.vue";
export { default as BaseSelect } from "../components/BaseSelect.vue";
export { default as BaseSwitch } from "../components/BaseSwitch.vue";
export { default as BaseTabs } from "../components/BaseTabs.vue";
export { default as BaseView } from "../components/BaseView.vue";
export { default as DeclaredSettings } from "../components/settings/DeclaredSettings.vue";
export { default as DirectoryField } from "../components/DirectoryField.vue";
export { default as GamepadGlyph } from "../components/GamepadGlyph.vue";
export { default as GameAccountDialog } from "../components/GameAccountDialog.vue";
export { default as InstallMigration } from "../components/InstallMigration.vue";
export { default as SettingGroup } from "../components/settings/SettingGroup.vue";
export { default as SettingRow } from "../components/settings/SettingRow.vue";

/** 包 UI 的动效统一使用宿主引入的 Motion for Vue 实例. */
export {
  AnimatePresence,
  MotionConfig,
  motion,
  useReducedMotion,
  useSpring,
} from "motion-v";

/** 包 UI 的编排工具统一使用宿主引入的 Anime.js 实例. */
export { createTimeline, spring, utils } from "animejs";

/** 宿主统一的动效参数与减少动态效果约定. */
export {
  controlSpring,
  useControlMotion,
} from "../composables/useControlMotion";
