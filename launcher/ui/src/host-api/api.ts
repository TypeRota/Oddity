import { computed } from "vue";
import { isDesktop } from "../core/api";
import { settingsNavigation } from "../core/settingsCatalog";
import {
  attendance,
  attendanceForm,
  form,
  gameRecords,
  loadAttendance,
  loginAttendance,
  loginAttendanceSms,
  logoutAttendance,
  migrationBusy,
  openSettings,
  queueAttendanceSave,
  queueSettingsSave,
  refreshGameRecords,
  runAttendance,
  runtimeEnvironmentBusy,
  runtimePathsLocked,
  sendAttendanceCode,
  settingsPathsLocked,
  state,
  ui,
  cancelGameDownload,
  checkUpdate,
  deletePreloadResources,
  diskBudget,
  downloadBusy,
  downloadView,
  downloadOptionalResource,
  formatSize,
  loadResourceCatalog,
  resourceCatalogMatches,
  removeOptionalResource,
  resourceDownloadsStarting,
  startGameDownload,
  updateView,
} from "../core/controller";
import { activeGame, gameSettings } from "../core/game";
import { gamepad } from "../core/gamepad";
import { playerExpanded } from "../composables/useSceneMotion";

/** 包 UI 调用宿主命令的唯一入口; 参数与返回值沿用生成的契约类型. */
export { invoke } from "../core/api";
export type { CommandArgs, Commands } from "../types/generated/commands";

/** 包命令预览的公共类型; 每个包用它们声明自己负责的命令与模拟数据. */
export type {
  PreviewContribution,
  PreviewHandler,
  PreviewHandlers,
} from "../types/preview";

/** 贡献声明的公共类型; 游戏包用它们声明页面与可见条件. */
export type { HostFacts } from "../host/facts";
export { channelLabel } from "../host/facts";
export type {
  CapabilityFact,
  CapabilityFacts,
  CapabilityRequirement,
  CapabilityState,
  Necessity,
} from "../host/capability";
/** 能力判定: required 未满足不挂载; optional 由贡献读取事实后自行降级. */
export { capabilityOf, contributionActive } from "../host/capability";
export type { PackageContribution } from "../host/contribution";
export type {
  GameContribution,
  PageContribution,
  PageEntry,
  PageNavItem,
  PageSlot,
} from "../host/pages";
export type {
  SettingsEntryDeclaration,
  SettingsSectionContribution,
} from "../host/settings";

/** 在系统浏览器打开链接, 沿用宿主的成功与失败提示. */
export { openUrl } from "../core/controller";

/** 包 UI 向用户提示瞬时结果, 样式与生命周期由宿主通知中心管理. */
export { notify } from "../core/notifications";

/**
 * 包 UI 可读的宿主事实. 全部是只读响应式值, 跟随 bootstrap 与宿主设置更新;
 * 游戏包只读这些事实, 不直接持有宿主状态.
 */
export const hostFacts = {
  game: computed(() => activeGame(state.bootstrap?.settings)),
  distribution: computed(() => factsFrom(state.bootstrap).distribution),
  /** 当前游戏声明的渠道; 选择器与展示文案都从这里取值. */
  channels: computed(() => factsFrom(state.bootstrap).channels),
  /** 启动数据已落地 (成功或失败), 包内可以开始读取依赖游戏身份的数据. */
  ready: computed(() => state.bootstrap !== null || ui.setupNeeded),
  page: computed(() => state.page),
  /** 游戏进程阶段; 启动、运行或关闭中时设置里的运行路径不可改. */
  gamePhase: computed(() => state.game),
  reduceMotion: computed(() => form.reduceMotion),
  stackBulletins: computed(() => form.stackBulletins),
};

/**
 * 玩家信息卡可读的账号与游戏记录状态. 只读快照, 写入与重试节奏仍由宿主
 * controller 统一调度, 包内不持有宿主状态.
 */
export const playerFacts = {
  accounts: computed(() => attendance.status?.accounts ?? []),
  attendanceLoaded: computed(() => attendance.loaded),
  attendanceLoading: computed(() => attendance.loading),
  attendanceBusy: computed(() => attendance.busy),
  attendanceRunning: computed(() => attendance.running),
  attendanceError: computed(() => attendance.error),
  recordsLoading: computed(() => gameRecords.loading),
  recordsWaiting: computed(() => gameRecords.waiting),
  recordsError: computed(() => gameRecords.error),
};

/** 玩家信息卡可用的宿主动作: 读账号, 刷新游戏记录, 跳到账号设置. */
export const playerActions = {
  loadAccounts: loadAttendance,
  refreshRecords: refreshGameRecords,
  openAccountSettings: () => openSettings("nte-account"),
};

/** 玩家卡与宿主场景共享的展开状态, 以及当前是否由手柄操作. */
export const playerCard = {
  expanded: playerExpanded,
  gamepad: computed(() => gamepad.active),
};

/** 包预览用到的契约类型. */
export type {
  AttendancePreferences,
  AttendanceStatus,
  BulletinFeed,
  BulletinItem,
  GameRecord,
} from "../types/launcher";

/**
 * 设置分区可用的宿主状态与动作.
 *
 * 分区直接双向绑定 form 与 attendanceForm, 保存节奏、锁状态与命令调用仍由
 * 宿主 controller 统一调度; 包内不持有宿主状态.
 */
export {
  attendance,
  attendanceForm,
  form,
  loadAttendance,
  loginAttendance,
  loginAttendanceSms,
  logoutAttendance,
  migrationBusy,
  queueAttendanceSave,
  queueSettingsSave,
  runAttendance,
  runtimeEnvironmentBusy,
  runtimePathsLocked,
  sendAttendanceCode,
  settingsPathsLocked,
  settingsNavigation,
};
export { isDesktop };

/** 设置分区用到的契约类型. */
export type {
  AttendanceAccount,
  AttendanceReport,
  AttendanceTasks,
  MeteredPolicy,
} from "../types/launcher";

import { factsFrom } from "../host/facts";
import { gameInstallRootHint } from "../host/pages";

/**
 * 更新页可读的宿主状态与动作.
 *
 * 页面只通过这个命名空间访问下载状态与命令, 宿主 controller 仍负责状态机与
 * 命令调度; 包内不直接引用宿主内部实现.
 */
export const update = {
  state,
  form,
  downloadBusy,
  downloadView,
  startGameDownload,
  cancelGameDownload,
  checkUpdate,
  updateView,
  formatSize,
  diskBudget,
  queueSettingsSave,
  settingsPathsLocked,
  runtimeEnvironmentBusy,
  resourceCatalogMatches,
  deletePreloadResources,
  loadResourceCatalog,
  downloadOptionalResource,
  resourceDownloadsStarting,
  removeOptionalResource,
  gameSettings,
  factsFrom,
  gameInstallRootHint,
};

/** 更新页用到的契约类型. */
export type {
  DownloadPhase,
  OptionalResource,
  ResourceCatalog,
  ResourceTask,
} from "../types/launcher";
