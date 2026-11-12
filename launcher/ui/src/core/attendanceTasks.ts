import type { AttendanceTasks } from "../types/launcher";

/**
 * 签到任务的默认授权.
 *
 * 字段来自宿主契约; 任务的标签与说明属于游戏包 UI, 不在这里维护.
 */
export const defaultAttendanceTasks: AttendanceTasks = {
  communitySign: true,
  roleSign: true,
  browsePosts: true,
  likePosts: true,
  sharePosts: true,
  cloudDuration: true,
};
