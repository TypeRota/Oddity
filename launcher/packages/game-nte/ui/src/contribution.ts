import type { Component } from "vue";
import type { HostFacts, PackageContribution } from "@host/api";
import BulletinPage from "./domains/bulletin/BulletinPage.vue";
import PlayerInfoCard from "./domains/player-info/PlayerInfoCard.vue";
import AccountSettingsSection from "./domains/settings/AccountSettingsSection.vue";
import GameSettingsSection from "./domains/settings/GameSettingsSection.vue";
import UpdatePage from "./domains/update/UpdatePage.vue";
import { pages } from "./pages";

const whenNte = (facts: HostFacts) => facts.activeGame === "nte";

const playerProfile = {
  id: "oddity.profile.player",
  revision: 1,
  necessity: "required",
} as const;

/** 页面组件绑定随页面声明一起由包提供, 宿主不再维护页面 id 到组件的映射. */
const pageComponents: Record<(typeof pages)[number]["id"], Component> = {
  "nte-update": UpdatePage,
  "nte-bulletin": BulletinPage,
};

/** 异环的全部 UI 贡献; 宿主经生成物装配, 不直接引用这里. */
export const contribution = {
  namespace: "nte",
  pageContributions: pages.map((page) => ({
    ...page,
    component: pageComponents[page.id],
  })),
  homeCards: [
    {
      id: "nte-player-info",
      order: 1,
      component: PlayerInfoCard,
      visible: whenNte,
    },
  ],
  settingsSections: [
    {
      id: "nte-game",
      label: "游戏与启动",
      icon: "i-game-controller",
      order: 4,
      component: GameSettingsSection,
      entries: [
        {
          id: "distribution",
          label: "游戏地区",
          description: "选择国服或国际服.",
          keywords: "区服 国际服 国服 NTE 异环",
        },
        {
          id: "install",
          label: "国服安装目录",
          description: "指定国服游戏的安装位置.",
          keywords: "路径 文件夹 磁盘 位置",
        },
        {
          id: "global-install",
          label: "国际服安装目录",
          description: "指定国际服游戏的安装位置.",
          keywords: "路径 文件夹 磁盘 位置 NTE",
        },
        {
          id: "migration",
          label: "迁移安装目录",
          description: "把当前安装移动到其他位置, 保留运行环境和预下载资源.",
          keywords: "迁移 移动 安装目录 路径 磁盘 空间",
        },
        {
          id: "launch",
          label: "启动方式",
          description: "选择后台启动、前台启动或直接启动.",
          keywords: "官方 登录 后台 前台 直接",
        },
        {
          id: "game-account",
          label: "游戏账号",
          description: "选择直接启动使用的游戏账号.",
          keywords: "登录 手机号 验证码 密码 账号切换 记住账号",
        },
        {
          id: "preload-dir",
          label: "预下载资源目录",
          description:
            "指定预下载缓存的存放位置, 留空使用安装目录内的默认位置.",
          keywords: "预下载 缓存 目录 磁盘 位置",
        },
        {
          id: "updates",
          label: "资源自动下载",
          description: "发现更新后自动下载, 并设置限速、计费网络与时段.",
          keywords: "自动 更新 下载 限速 带宽 计费 流量 时段 后台",
        },
      ],
      visible: whenNte,
    },
    {
      id: "nte-account",
      label: "账号与签到",
      icon: "i-users",
      order: 6,
      component: AccountSettingsSection,
      entries: [
        {
          id: "account",
          label: "塔吉多签到",
          description: "管理账号、自动签到、签到时机与每日时间.",
          keywords: "账号 登录 密码 验证码 签到 账号管理 社区",
        },
      ],
      requires: [playerProfile],
      visible: whenNte,
    },
  ],
} as const satisfies PackageContribution<"nte">;
