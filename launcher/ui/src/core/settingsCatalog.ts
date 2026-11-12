import { reactive } from "vue";
import type { SettingsSectionId } from "../host/settings";

// 宿主通用分区; 游戏相关分区由游戏包声明, 在 host/settings.ts 合并.
export const settingsCategories = [
  { id: "appearance", label: "外观与交互", icon: "i-monitor", order: 1 },
  { id: "audio", label: "音频", icon: "i-speaker-high", order: 2 },
  { id: "gamepad", label: "手柄", icon: "i-game-controller", order: 3 },
  { id: "runtime", label: "运行环境", icon: "i-hard-drives", order: 5 },
  { id: "maintenance", label: "诊断与维护", icon: "i-shield-check", order: 7 },
] as const;
export type HostSettingsCategory = (typeof settingsCategories)[number]["id"];
/** 设置页分区 id: 宿主分区与当前分区贡献共用同一命名空间. */
export type SettingsCategory = SettingsSectionId;
export const settingsNavigation = reactive<{
  category: SettingsCategory;
  query: string;
}>({ category: "appearance", query: "" });

export const settingsEntries = [
  {
    id: "audio-output",
    category: "audio",
    label: "游戏输出设备",
    description: "为游戏选择音频输出设备.",
    keywords: "声音 音频 耳机 音箱 扬声器 默认 设备",
  },
  {
    id: "audio-volume",
    category: "audio",
    label: "游戏音量",
    description: "调整游戏音量, 支持静音与音量增强.",
    keywords: "声音 音频 增益 减益 放大 静音 音量",
  },
  {
    id: "gamepad",
    category: "gamepad",
    label: "手柄",
    description: "配置手柄操作、按键图标与触觉反馈.",
    keywords: "DS DualSense DualShock Xbox Switch 手柄 振动 反馈 摇杆",
  },
  {
    id: "theme",
    category: "appearance",
    label: "主题",
    description: "选择启动器的亮色或暗色外观.",
    keywords: "亮色 暗色 深色 浅色 颜色",
  },
  {
    id: "media",
    category: "appearance",
    label: "主页主视觉",
    description: "选择动态视频或静态照片.",
    keywords: "壁纸 背景 视频 照片 封面",
  },
  {
    id: "adaptive-theme",
    category: "appearance",
    label: "跟随主视觉",
    description: "用主页主视觉的色相生成界面主题色.",
    keywords: "主题色 自适应 取色 磨砂 颜色 主视觉",
  },
  {
    id: "relative-heatmap",
    category: "appearance",
    label: "热力图相对深浅",
    description: "按最近 12 个月的相对游玩时长着色, 关闭后按固定时长分档.",
    keywords: "记录 热力图 深浅 相对 绝对 游玩 时长",
  },
  {
    id: "bulletins",
    category: "appearance",
    label: "堆叠公告",
    description: "将同一来源的公告合并为卡叠, 点击展开.",
    keywords: "公告 卡片 堆叠 列表 显示",
  },
  {
    id: "motion",
    category: "appearance",
    label: "减少动效",
    description: "关闭入场与页面过渡, 保留状态反馈.",
    keywords: "动画 运动 无障碍",
  },
  {
    id: "focus-pause",
    category: "appearance",
    label: "失焦时暂停主视觉视频",
    description: "窗口失去焦点时停止视频解码, 降低同屏运行游戏时的资源占用.",
    keywords: "焦点 失焦 窗口 视频 解码 性能 资源 主视觉",
  },
  {
    id: "tray",
    category: "appearance",
    label: "关闭窗口时最小化到托盘",
    description: "关闭主窗口后保持后台运行.",
    keywords: "窗口 关闭 后台 最小化 托盘",
  },
  {
    id: "return",
    category: "appearance",
    label: "游戏退出后打开启动器",
    description: "游戏进程结束后显示 Oddity 主界面.",
    keywords: "窗口 返回 退出 启动器",
  },
  {
    id: "runtime",
    category: "runtime",
    label: "Proton 运行环境",
    description: "创建、切换、复制或删除独立的运行环境.",
    keywords: "Proton Wine prefix 副本 稳定 环境名称",
  },
  {
    id: "snapshots",
    category: "maintenance",
    label: "环境快照",
    description: "设置成功判定时长, 对比当前环境与自动基准或参考快照.",
    keywords: "快照 环境 对比 驱动 Proton prefix 历史 运行时长 判定 稳定",
  },
  {
    id: "logs",
    category: "maintenance",
    label: "导出日志",
    description: "导出 Oddity 启动与后台会话日志.",
    keywords: "日志 ZIP 导出 排查",
  },
  {
    id: "diagnostics",
    category: "maintenance",
    label: "运行诊断",
    description: "检查启动器配置、游戏文件与运行依赖.",
    keywords: "检测 检查 umu prefix Proton 故障 配置",
  },
  {
    id: "integrity",
    category: "maintenance",
    label: "Oddity 安全性验证",
    description: "核对启动器文件是否与官方发布一致.",
    keywords: "安全 校验 完整性 文件 篡改 官方 清单 重装",
  },
] as const satisfies readonly {
  id: string;
  category: HostSettingsCategory;
  label: string;
  description: string;
  keywords: string;
}[];
export type SettingsEntry = (typeof settingsEntries)[number];
