import type { Commands } from "@host/api";

/** 公告数据完全来自契约命令, 包内不重复声明字段. */
export type BulletinFeed = Commands["fetch_bulletins"];
export type BulletinItem = BulletinFeed["items"][number];
