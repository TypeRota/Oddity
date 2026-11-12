import type {
  AttendanceStatus,
  BulletinFeed,
  CommandArgs,
  PreviewContribution,
  PreviewHandlers,
} from "@host/api";

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));

const OFFICIAL = {
  cn: "https://yh.wanmei.com/launcher/launcher_ob2.html?expand=0",
  global: "https://nte.perfectworld.com/en/article/news/index.html",
  maintenance: "https://yh.wanmei.com/news/gamebroad/index.html",
  bilibili: "https://space.bilibili.com/3546636978489848/dynamic",
} as const;

/** 浏览器预览的公告示例; 原生启动器读取真实公告, 不用这份数据. */
const previewBulletins: BulletinFeed = {
  items: [
    {
      kind: "bilibili",
      title: "《异环》官方动态示例",
      summary: "浏览器预览使用本地示例数据，原生启动器会自动读取官方动态。",
      url: OFFICIAL.bilibili,
      imageUrl: null,
      publishedAt: "刚刚",
      source: "B站官方动态",
    },
    {
      kind: "preview",
      title: "《异环》版本前瞻特别节目 | 情报回顾",
      summary: "版本前瞻回顾",
      url: OFFICIAL.cn,
      imageUrl: new URL("assets/hero-keyart.jpg", document.baseURI).href,
      publishedAt: "",
      source: "版本前瞻",
    },
    {
      kind: "maintenance",
      title: "《异环》不停服更新公告",
      summary: "服务器维护时间以官方公告为准。",
      url: OFFICIAL.maintenance,
      imageUrl: null,
      publishedAt: "最新",
      source: "官方公告",
    },
  ],
  errors: [],
};

const DEFAULT_TASKS = {
  communitySign: true,
  roleSign: true,
  browsePosts: true,
  likePosts: true,
  sharePosts: true,
  cloudDuration: true,
};

/** 浏览器预览的签到状态; 与宿主的真实账号状态无关. */
const previewAttendance: AttendanceStatus = {
  accounts: [],
  preferences: {
    trigger: "off",
    time: "08:00",
    tasks: { ...DEFAULT_TASKS },
  },
};

async function fetchBulletins(
  args: CommandArgs["fetch_bulletins"],
): Promise<BulletinFeed> {
  await new Promise((resolve) => setTimeout(resolve, 260));
  if (args.distribution === "global") {
    return {
      items: [
        {
          kind: "preview",
          title: "国际服官方新闻",
          summary: "浏览器预览数据",
          url: OFFICIAL.global,
          imageUrl: null,
          publishedAt: "",
          source: "国际服官网",
        },
      ],
      errors: [],
    };
  }
  return clone(previewBulletins);
}

function refreshGameRecords(): AttendanceStatus {
  previewAttendance.accounts.forEach((account, index) => {
    account.gameRecord = {
      roleId: String(100000001 + index),
      roleName: index ? `预览角色 ${index + 1}` : "海特洛调查员",
      serverName: "预览服务器",
      stamina: index ? 240 : 168,
      staminaMax: 240,
      cityStamina: index ? 180 : 420,
      cityStaminaMax: 700,
      activity: index ? 100 : 60,
      weeklyRemaining: index ? 0 : 2,
      updatedAt: Math.floor(Date.now() / 1000),
    };
    account.gameRecordError = null;
  });
  return clone(previewAttendance);
}

function sendCode(args: CommandArgs["attendance_send_code"]) {
  if (!/^[0-9]{11}$/.test(args.phone)) throw new Error("请输入有效的手机号码");
  return "preview-device-" + args.phone;
}

function login(
  phone: string,
  password: string,
  rememberPassword: boolean,
): AttendanceStatus {
  if (!/^[0-9]{11}$/.test(phone) || !password)
    throw new Error("请输入手机号码和密码");
  let hash = 2166136261;
  for (const char of phone)
    hash = Math.imul(hash ^ char.charCodeAt(0), 16777619);
  const account = {
    id: (hash >>> 0).toString(16),
    maskedPhone: phone.slice(0, 3) + "****" + phone.slice(7),
    rememberPassword,
    signedToday: false,
    failedToday: false,
    lastReport: null,
    attention: null,
  };
  const index = previewAttendance.accounts.findIndex(
    (item) => item.id === account.id,
  );
  if (index < 0) previewAttendance.accounts.push(account);
  else previewAttendance.accounts[index] = account;
  return clone(previewAttendance);
}

function loginSms(args: CommandArgs["attendance_login_sms"]): AttendanceStatus {
  const { phone, code, deviceId } = args;
  if (!/^[0-9]{11}$/.test(phone)) throw new Error("请输入有效的手机号码");
  if (!/^[0-9]{4,8}$/.test(code)) throw new Error("请输入 4-8 位短信验证码");
  if (deviceId !== "preview-device-" + phone)
    throw new Error("请先为当前手机号获取验证码.");
  return login(phone, "preview", false);
}

function configure(
  args: CommandArgs["attendance_configure"],
): AttendanceStatus {
  const { preferences } = args;
  if (preferences.trigger !== "off" && !previewAttendance.accounts.length)
    throw new Error("请先登录塔吉多账号");
  previewAttendance.preferences = clone(preferences);
  return clone(previewAttendance);
}

function run(): AttendanceStatus {
  if (!previewAttendance.accounts.length) throw new Error("请先登录塔吉多账号");
  if (!Object.values(previewAttendance.preferences.tasks).some(Boolean))
    return clone(previewAttendance);
  for (const account of previewAttendance.accounts) {
    if (["auth", "verification"].includes(account.attention?.kind ?? ""))
      continue;
    account.signedToday = true;
    account.failedToday = false;
    account.lastReport = {
      at: Math.floor(Date.now() / 1000),
      ok: true,
      steps: [
        {
          label: "社区签到",
          ok: true,
          message: "模拟签到完成, 未发送真实请求",
          kind: null,
        },
        ...[
          "角色 预览角色",
          "BBS 金币签到",
          "浏览帖子",
          "点赞帖子",
          "分享帖子",
        ].map((label) => ({
          label,
          ok: true,
          message: "模拟任务完成, 未发送真实请求",
          kind: null,
          optional: label !== "角色 预览角色",
        })),
        {
          label: "今日金币",
          ok: true,
          message: "模拟今日金币 110/150, 未发送真实请求",
          kind: null,
          optional: true,
        },
        {
          label: "云异环时长",
          ok: true,
          message: "模拟本次赠送 15 分钟, 剩余 120 分钟, 未发送真实请求",
          kind: null,
          optional: true,
        },
      ].filter((step) => {
        const tasks = previewAttendance.preferences.tasks;
        if (step.label === "社区签到" || step.label === "BBS 金币签到")
          return tasks.communitySign;
        if (step.label.startsWith("角色 ")) return tasks.roleSign;
        if (step.label === "浏览帖子") return tasks.browsePosts;
        if (step.label === "点赞帖子") return tasks.likePosts;
        if (step.label === "分享帖子") return tasks.sharePosts;
        if (step.label === "云异环时长") return tasks.cloudDuration;
        return (
          tasks.communitySign ||
          tasks.browsePosts ||
          tasks.likePosts ||
          tasks.sharePosts
        );
      }),
    };
  }
  return clone(previewAttendance);
}

function logout(args: CommandArgs["attendance_logout"]): AttendanceStatus {
  previewAttendance.accounts = previewAttendance.accounts.filter(
    (account) => account.id !== args.accountId,
  );
  if (!previewAttendance.accounts.length)
    previewAttendance.preferences = {
      trigger: "off",
      time: "08:00",
      tasks: { ...DEFAULT_TASKS },
    };
  return clone(previewAttendance);
}

/**
 * 异环的命令预览.
 *
 * 公告与塔吉多签到的模拟数据随命令一起由包提供, 宿主不再认识这些命令的
 * 返回形状与游戏文案; 宿主 previewInvoke 按当前游戏命名空间查这张表.
 */
export const previewCommands = {
  fetch_bulletins: fetchBulletins,
  attendance_status: () => clone(previewAttendance),
  attendance_refresh_game_records: refreshGameRecords,
  attendance_keyring_check: () => undefined,
  attendance_send_code: sendCode,
  attendance_login_sms: loginSms,
  attendance_login: (args) =>
    login(args.phone, args.password, args.rememberPassword),
  attendance_configure: configure,
  attendance_run: run,
  attendance_logout: logout,
} as const satisfies Partial<PreviewHandlers>;

export const preview = {
  namespace: "nte",
  commands: previewCommands,
} as const satisfies PreviewContribution;

/** 包公共出口; 宿主装配从生成注册表取表, 不直接引用本模块. */
export const ntePreviewContribution = preview;
