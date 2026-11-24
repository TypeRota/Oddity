import { defaultAttendanceTasks } from "../../src/core/attendanceTasks";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type {
  AttendanceAccount,
  AttendanceStatus,
  GameRecord,
  PlayerStateStatus,
} from "../../src/types/launcher";
const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../src/core/api", () => ({
  invoke: api.invoke,
  isDesktop: () => false,
}));
import * as c from "../../src/core/controller";
import { notifications } from "../../src/core/notifications";
import { PlayerInfoCard as Component } from "@packages/game-nte/ui";
import { bootstrap } from "./fixtures";
import GameSettingsSection from "../../../packages/game-nte/ui/src/domains/settings/GameSettingsSection.vue";

let wrapper: VueWrapper;
let status: AttendanceStatus;
let liveStatus: PlayerStateStatus;
const record = (overrides: Partial<GameRecord> = {}): GameRecord => ({
  roleId: "100000001",
  roleName: "调查员甲",
  serverName: "国服",
  stamina: 0,
  staminaMax: 240,
  cityStamina: null,
  cityStaminaMax: 700,
  activity: 60,
  weeklyRemaining: 0,
  incentiveObtained: 350000,
  incentiveMaximum: 1150000,
  incentivePeriodDays: 14,
  incentiveRemaining: 13 * 86_400_000,
  updatedAt: 1700000000,
  ...overrides,
});
const account = (id: string): AttendanceAccount => ({
  id,
  maskedPhone: `138****800${id}`,
  rememberPassword: false,
  signedToday: false,
  failedToday: false,
  lastReport: null,
  attention: null,
});
beforeEach(() => {
  c.dispose();
  c.form.reduceMotion = true;
  Object.assign(c.attendance, {
    status: null,
    loaded: false,
    loading: false,
    busy: false,
    running: false,
    error: "",
  });
  status = {
    accounts: [account("1"), account("2")],
    preferences: {
      trigger: "off",
      time: "08:00",
      tasks: { ...defaultAttendanceTasks },
    },
  };
  liveStatus = {
    observation: null,
    enabled: false,
    state: "disabled",
    message: null,
    record: null,
  };
  api.invoke.mockReset().mockImplementation(async (command) => {
    if (command === "game_state_start") {
      liveStatus.state = "capturing";
      liveStatus.enabled = true;
    }
    if (command === "game_state_stop") {
      liveStatus.state = "disabled";
      liveStatus.enabled = false;
    }
    if (command.startsWith("game_state_")) return structuredClone(liveStatus);
    if (command === "attendance_refresh_game_records")
      status.accounts.forEach((a, i) => {
        a.gameRecord = record({ roleName: i ? "调查员乙" : "调查员甲" });
      });
    return structuredClone(status);
  });
});
afterEach(() => {
  wrapper?.unmount();
  c.dispose();
});
async function boot() {
  wrapper = mount(Component);
  await flushPromises();
  await wrapper.get(".game-records__capsule").trigger("click");
  await flushPromises();
}
const requests = () =>
  api.invoke.mock.calls.filter(
    ([cmd]) => cmd === "attendance_refresh_game_records",
  ).length;

it("切换查看全部账号、保留零值和未知字段, 手动刷新不执行签到", async () => {
  await boot();
  expect(wrapper.findAll(".game-records__account")).toHaveLength(1);
  expect(wrapper.text()).toContain("调查员甲");
  expect(wrapper.text()).not.toContain("调查员乙");
  await wrapper.get('[aria-label="下一个账号"]').trigger("click");
  await flushPromises();
  expect(wrapper.text()).toContain("调查员乙");
  expect(wrapper.text()).toContain("0 / 240");
  expect(wrapper.text()).toContain("-- / 700");
  expect(wrapper.text()).toContain("周本剩余0 次");
  expect(wrapper.get(".game-records__name").text()).toBe("调查员乙");
  expect(wrapper.get(".game-records__stamina").text()).toBe("体力 0 / 240");
  expect(requests()).toBe(1);
  await wrapper.get('[aria-label="刷新游戏状态"]').trigger("click");
  await flushPromises();
  expect(requests()).toBe(2);
  expect(api.invoke).not.toHaveBeenCalledWith("attendance_run");
});

it("空账号前往设置, 隐藏首页时停止网络刷新, 回到首页按间隔更新", async () => {
  status.accounts = [];
  await boot();
  expect(requests()).toBe(0);
  await wrapper.get(".game-records__link").trigger("click");
  expect(c.state.page).toBe("settings");
  status.accounts = [account("1")];
  c.syncAttendance(structuredClone(status));
  await flushPromises();
  expect(requests()).toBe(0);
  c.state.page = "home";
  await flushPromises();
  expect(requests()).toBe(1);
  await vi.advanceTimersByTimeAsync(29 * 60_000);
  expect(requests()).toBe(1);
  c.state.page = "settings";
  await vi.advanceTimersByTimeAsync(60_000);
  expect(requests()).toBe(1);
  c.state.page = "home";
  await flushPromises();
  expect(requests()).toBe(2);
});

it("单账号失效仍显示其他账号和上次数据, 提供重新登录入口", async () => {
  status.accounts[0].gameRecord = record();
  status.accounts[0].gameRecordError = { kind: "auth", message: "请重新登录" };
  api.invoke.mockImplementation(async (command) =>
    structuredClone(command.startsWith("game_state_") ? liveStatus : status),
  );
  await boot();
  expect(wrapper.text()).toContain("0 / 240");
  expect(wrapper.findAll(".game-records__account")).toHaveLength(1);
  await wrapper.get(".game-records__link").trigger("click");
  expect(c.state.page).toBe("settings");
});

it("网络失败可重试, 恢复后清除错误, 忙碌时不重复查询", async () => {
  await boot();
  api.invoke.mockRejectedValueOnce({ message: "网络暂时不可用" });
  await c.refreshGameRecords(true);
  await flushPromises();
  expect(notifications.some((item) => item.message === "网络暂时不可用")).toBe(
    true,
  );
  expect(wrapper.find(".game-records__error").exists()).toBe(false);
  expect(wrapper.text()).toContain("调查员甲");
  const failedRequests = requests();
  await vi.advanceTimersByTimeAsync(29 * 60_000);
  expect(requests()).toBe(failedRequests);
  await vi.advanceTimersByTimeAsync(60_000);
  expect(requests()).toBe(failedRequests + 1);
  await c.refreshGameRecords(true);
  await flushPromises();
  expect(c.gameRecords.error).toBe("");
  c.attendance.busy = true;
  const before = requests();
  await c.refreshGameRecords(true);
  expect(requests()).toBe(before);
});

it("账号移除和组件卸载后丢弃旧请求, 防止已移除账号重新出现", async () => {
  await boot();
  let resolve!: (value: AttendanceStatus) => void;
  const old = structuredClone(status);
  api.invoke.mockImplementationOnce(
    () =>
      new Promise<AttendanceStatus>((done) => {
        resolve = done;
      }),
  );
  const pending = c.refreshGameRecords(true);
  c.syncAttendance({ ...status, accounts: [] });
  resolve(old);
  await pending;
  await flushPromises();
  expect(c.attendance.status?.accounts).toHaveLength(0);
  c.syncAttendance(old);
  await flushPromises();
  api.invoke.mockImplementationOnce(
    () =>
      new Promise<AttendanceStatus>((done) => {
        resolve = done;
      }),
  );
  const disposed = c.refreshGameRecords(true);
  wrapper.unmount();
  c.dispose();
  resolve({ ...status, accounts: [] });
  await disposed;
  expect(c.gameRecords.loading).toBe(false);
});

it("默认收起为胶囊, 胶囊与 Esc 收起, 选中账号移除后显示剩余账号", async () => {
  wrapper = mount(Component);
  await flushPromises();
  const capsule = wrapper.get(".game-records__capsule");
  expect(capsule.attributes("aria-expanded")).toBe("false");
  expect(wrapper.find(".game-records__panel").exists()).toBe(false);
  await capsule.trigger("click");
  await flushPromises();
  expect(capsule.attributes("aria-expanded")).toBe("true");
  await wrapper.get('[aria-label="下一个账号"]').trigger("click");
  await flushPromises();
  expect(wrapper.text()).toContain("调查员乙");
  status.accounts = [status.accounts[0]];
  c.syncAttendance(structuredClone(status));
  await flushPromises();
  expect(wrapper.text()).toContain("调查员甲");
  expect(wrapper.find('[aria-label="切换账号"]').exists()).toBe(false);
  await capsule.trigger("click");
  expect(capsule.attributes("aria-expanded")).toBe("false");
  await capsule.trigger("click");
  await wrapper.get(".game-records").trigger("keydown", { key: "Escape" });
  expect(capsule.attributes("aria-expanded")).toBe("false");
});

it("启动时已签到遇到临时忙碌会自动重试, 不留下错误提示", async () => {
  status.accounts[0].signedToday = true;
  status.accounts[0].gameRecord = record();
  api.invoke.mockImplementation(async (command) => {
    if (command.startsWith("game_state_")) return structuredClone(liveStatus);
    if (command === "attendance_refresh_game_records" && requests() === 1)
      throw { kind: "busy", message: "账号或签到任务正在处理, 请稍后重试" };
    return structuredClone(status);
  });
  await boot();
  expect(requests()).toBe(1);
  expect(wrapper.text()).toContain("稍后自动刷新");
  expect(wrapper.find(".game-records__error").exists()).toBe(false);
  await vi.advanceTimersByTimeAsync(2000);
  await flushPromises();
  expect(requests()).toBe(2);
  expect(c.gameRecords.waiting).toBe(false);
  expect(wrapper.text()).not.toContain("稍后自动刷新");
  expect(c.gameRecords.error).toBe("");
});

it.each([true, false])(
  "签到和查询并发完成时分别保留结果, 查询先返回: %s",
  async (recordsFirst) => {
    await boot();
    let resolveSign!: (value: AttendanceStatus) => void;
    let resolveRecord!: (value: AttendanceStatus) => void;
    const signed = structuredClone(status);
    signed.accounts[0].signedToday = true;
    const records = structuredClone(status);
    records.accounts[0].gameRecord = record({ stamina: 120 });
    api.invoke.mockImplementation(
      (command) =>
        new Promise<AttendanceStatus>((done) => {
          if (command === "attendance_run") resolveSign = done;
          if (command === "attendance_refresh_game_records")
            resolveRecord = done;
        }),
    );
    const sign = c.runAttendance();
    await flushPromises();
    expect(
      wrapper.get('[aria-label="刷新游戏状态"]').attributes("disabled"),
    ).toBeUndefined();
    const query = c.refreshGameRecords(true);
    await flushPromises();
    expect(resolveRecord).toBeTypeOf("function");
    if (recordsFirst) {
      resolveRecord(records);
      await query;
      resolveSign(signed);
    } else {
      resolveSign(signed);
      await sign;
      resolveRecord(records);
    }
    await Promise.all([sign, query]);
    await flushPromises();
    expect(c.attendance.status?.accounts[0].signedToday).toBe(true);
    expect(c.attendance.status?.accounts[0].gameRecord?.stamina).toBe(120);
    expect(wrapper.text()).toContain("120 / 240");
  },
);

it("退出后清理临时忙碌重试, 旧账号查询失败不会污染新状态", async () => {
  await boot();
  let reject!: (cause: unknown) => void;
  api.invoke.mockImplementationOnce(
    () =>
      new Promise((_done, fail) => {
        reject = fail;
      }),
  );
  const pending = c.refreshGameRecords(true);
  c.syncAttendance({ ...status, accounts: [] });
  reject({ kind: "busy", message: "旧任务忙碌" });
  await pending;
  expect(c.gameRecords.waiting).toBe(false);
  expect(c.gameRecords.error).toBe("");
  c.syncAttendance(structuredClone(status));
  api.invoke.mockRejectedValue({ kind: "busy", message: "忙碌" });
  await c.refreshGameRecords(true);
  const before = requests();
  wrapper.unmount();
  c.dispose();
  await vi.advanceTimersByTimeAsync(5000);
  expect(requests()).toBe(before);
});

it("较早的状态读取不能覆盖已经完成的签到", async () => {
  await boot();
  const old = structuredClone(status);
  const signed = structuredClone(status);
  signed.accounts[0].signedToday = true;
  let resolveStatus!: (value: AttendanceStatus) => void;
  api.invoke.mockImplementation((command) => {
    if (command === "attendance_status")
      return new Promise<AttendanceStatus>((done) => {
        resolveStatus = done;
      });
    return Promise.resolve(signed);
  });
  const loading = c.loadAttendance();
  await c.runAttendance();
  resolveStatus(old);
  await loading;
  expect(c.attendance.status?.accounts[0].signedToday).toBe(true);
});

it("游戏同步无需社区登录, 同时显示两种体力与周本并保留停止后的时间", async () => {
  status.accounts = [];
  liveStatus = {
    observation: null,
    enabled: true,
    state: "capturing",
    message: null,
    record: record({
      stamina: 127,
      staminaMax: 360,
      staminaBackup: 169,
      cityStaminaBackup: 0,
      cityStamina: 190,
      weeklyRemaining: 2,
      activity: null,
    }),
  };
  await boot();
  expect(wrapper.text()).toContain("调查员甲");
  expect(wrapper.text()).toContain("127 / 360");
  expect(wrapper.text()).toContain("190 / 700");
  expect(wrapper.text()).toContain("周本剩余2 次");
  expect(wrapper.text()).toContain("备用本性像素169 / 2400");
  expect(wrapper.text()).toContain("备用都市活力0 / 1000");
  expect(wrapper.text()).toContain("大亨激励金350,000 / 1,150,000");
  expect(wrapper.text()).toContain("13 天后刷新");
  expect(wrapper.find(".game-records__sync").exists()).toBe(false);
  await c.stopPlayerState();
  await flushPromises();
  expect(wrapper.text()).toContain("127 / 360");
  expect(wrapper.text()).toContain("调查员甲");
});

it("同步角色按 UID 独立显示, 切换其他社区账号不混用体力", async () => {
  liveStatus = {
    observation: null,
    enabled: true,
    state: "capturing",
    message: null,
    record: record({
      roleId: "200000002",
      roleName: "游戏角色",
      stamina: 129,
      staminaMax: 360,
      cityStamina: 424,
    }),
  };
  await boot();
  expect(wrapper.get(".game-records__name").text()).toBe("游戏角色");
  expect(wrapper.text()).toContain("129 / 360");
  await wrapper.get('[aria-label="下一个账号"]').trigger("click");
  expect(wrapper.get(".game-records__name").text()).toBe("调查员甲");
  expect(wrapper.text()).toContain("0 / 240");
  expect(wrapper.text()).not.toContain("129 / 360");
  expect(wrapper.text()).not.toContain("129 / 360");
});

it("等待读取、权限错误和空周本都保持可见未知值", async () => {
  status.accounts = [];
  liveStatus = {
    observation: null,
    enabled: false,
    state: "waiting",
    message: "请启动异环.",
    record: null,
  };
  await boot();
  expect(wrapper.text()).toContain("登录塔吉多");
  expect(wrapper.find(".game-records__sync").exists()).toBe(false);
  liveStatus = {
    observation: null,
    enabled: false,
    state: "error",
    message: "同步权限未授予",
    record: null,
  };
  await c.refreshPlayerState();
  await flushPromises();
  expect(wrapper.text()).not.toContain("同步权限未授予");
  expect(
    notifications.filter((item) => item.key === "player-state"),
  ).toHaveLength(1);
  expect(
    notifications.find((item) => item.key === "player-state")?.message,
  ).toBe("同步权限未授予");
  await c.refreshPlayerState();
  expect(notifications.find((item) => item.key === "player-state")?.count).toBe(
    1,
  );
  expect(wrapper.find(".game-records__sync-button").exists()).toBe(false);
  expect(wrapper.text()).not.toContain("估算");
});

it("读取新会话时保留已知数据并显示上次同步时间", async () => {
  status.accounts = [];
  liveStatus = {
    observation: null,
    enabled: false,
    state: "reading",
    message: "等待读取角色.",
    record: record(),
  };
  await boot();
  expect(wrapper.text()).toContain("0 / 240");
  expect(wrapper.get(".game-records__name").text()).toBe("调查员甲");
});

it("采集故障保留游戏读数, 并通过全局通知提示", async () => {
  liveStatus = {
    observation: null,
    enabled: true,
    state: "capturing",
    message: null,
    record: record({ stamina: 127 }),
  };
  await boot();
  expect(wrapper.text()).toContain("127 / 240");
  const before = requests();
  liveStatus.state = "error";
  liveStatus.message = "未安装 dumpcap, 将在安装后自动重试.";
  await c.refreshPlayerState();
  await flushPromises();
  expect(requests()).toBe(before);
  expect(wrapper.text()).toContain("127 / 240");
  expect(wrapper.get(".game-records__name").text()).toBe("调查员甲");
  expect(wrapper.text()).not.toContain("未安装 dumpcap");
  expect(
    notifications.find((item) => item.key === "player-state")?.message,
  ).toBe(liveStatus.message);
  await c.refreshPlayerState();
  expect(requests()).toBe(before);
  expect(notifications.find((item) => item.key === "player-state")?.count).toBe(
    1,
  );
});

it("同 UID 社区信息仅补角色名, 游戏资源未知值保持未知", async () => {
  liveStatus = {
    observation: null,
    enabled: true,
    state: "capturing",
    message: null,
    record: record({
      roleName: "",
      stamina: 24,
      staminaMax: 360,
      cityStamina: null,
      cityStaminaMax: null,
    }),
  };
  api.invoke.mockImplementation(async (command) => {
    if (command.startsWith("game_state_")) return structuredClone(liveStatus);
    status.accounts[0].gameRecord = record({
      roleName: "测试角色名",
      cityStamina: 424,
      cityStaminaMax: 700,
    });
    return structuredClone(status);
  });
  await boot();
  expect(wrapper.get(".game-records__name").text()).toBe("测试角色名");
  expect(wrapper.get(".game-records__identity").text()).toContain("测试角色名");
  expect(wrapper.get(".game-records__identity").text()).toContain(
    "UID 100000001",
  );
  expect(wrapper.text()).toContain("-- / --");
  expect(wrapper.text()).toContain("24 / 360");
  liveStatus.record!.stamina = null;
  liveStatus.record!.weeklyRemaining = null;
  liveStatus.record!.cityStamina = 190;
  await c.refreshPlayerState();
  await flushPromises();
  expect(wrapper.text()).toContain("-- / 360");
  expect(wrapper.text()).toContain("190 / --");
  expect(wrapper.text()).toContain("周本剩余-- 次");
});

it("同步开关位于游戏设置, 开关调用同步命令并展示宿主状态", async () => {
  c.state.bootstrap = bootstrap();
  c.form.distribution = "cn";
  wrapper = mount(GameSettingsSection, {
    global: { stubs: { Teleport: true } },
  });
  await flushPromises();
  const toggle = wrapper.get("#set-player-state-sync");
  expect((toggle.element as HTMLInputElement).checked).toBe(false);
  await toggle.setValue(!(toggle.element as HTMLInputElement).checked);
  await flushPromises();
  expect(api.invoke).toHaveBeenCalledWith("game_state_start");
  expect((toggle.element as HTMLInputElement).checked).toBe(true);
  await toggle.setValue(!(toggle.element as HTMLInputElement).checked);
  await flushPromises();
  expect(api.invoke).toHaveBeenCalledWith("game_state_stop");
  expect((toggle.element as HTMLInputElement).checked).toBe(false);
});

it("能力不满足时卸载能力相关的子区块, 不留下禁用占位", async () => {
  // 用真实的 bootstrap 形状投射能力事实: 国际服下两项能力都报告 unsupported.
  c.state.bootstrap = bootstrap({
    capabilities: [
      {
        id: "oddity.account.game",
        revision: 1,
        state: "unsupported",
        code: null,
        message: null,
      },
      {
        id: "oddity.profile.player",
        revision: 1,
        state: "unsupported",
        code: null,
        message: null,
      },
    ],
  });
  wrapper = mount(GameSettingsSection, {
    global: { stubs: { Teleport: true } },
  });
  await flushPromises();
  // 玩家信息与游戏账号整块卸载, 直接启动方式不再出现在选项里.
  expect(wrapper.find("#set-player-state-sync").exists()).toBe(false);
  expect(wrapper.find('[data-setting="game-account"]').exists()).toBe(false);
  expect(wrapper.text()).not.toContain("直接启动");
});

it("未识别角色的资源单独显示, 不与旧账号身份混合", async () => {
  liveStatus = {
    enabled: true,
    observation: record({
      roleId: "",
      roleName: "",
      stamina: 129,
      staminaMax: 360,
      weeklyRemaining: null,
    }),
    state: "unidentified",
    message: "已读取当前游戏资源, 正在等待角色身份.",
    record: record({ stamina: 24 }),
  };
  await boot();
  expect(wrapper.get(".game-records__name").text()).toBe("当前游戏");
  expect(wrapper.text()).toContain("129 / 360");
  expect(wrapper.get(".game-records__identity").text()).not.toContain("UID");
  await wrapper.get('[aria-label="下一个账号"]').trigger("click");
  expect(wrapper.get(".game-records__name").text()).toBe("调查员甲");
  expect(wrapper.text()).toContain("24 / 240");
  expect(wrapper.text()).not.toContain("129 / 360");
});

it("游戏退出, 停止同步, 缺包和后端异常时持续展示计算值, 社区刷新不覆盖", async () => {
  liveStatus = {
    enabled: true,
    observation: null,
    state: "capturing",
    message: null,
    record: record({
      stamina: 129,
      staminaBackup: 169,
      cityStamina: 190,
      cityStaminaBackup: 60,
    }),
  };
  await boot();
  const before = requests();
  for (const state of [
    "waiting",
    "disabled",
    "starting",
    "reading",
    "stale",
    "error",
  ] as const) {
    liveStatus.state = state;
    liveStatus.enabled = state !== "disabled";
    liveStatus.record!.stamina = 130;
    await c.refreshPlayerState();
    await flushPromises();
    await c.refreshGameRecords(true);
    await flushPromises();
    expect(wrapper.text()).toContain("130 / 240");
    expect(wrapper.text()).toContain("190 / 700");
    expect(wrapper.text()).toContain("备用都市活力60 / 1000");
    expect(wrapper.text()).toContain("备用本性像素169 / 2400");
  }
  const after = requests();
  expect(after).toBe(before + 6);
  await vi.advanceTimersByTimeAsync(30_000);
  expect(requests()).toBe(after);
  api.invoke.mockRejectedValueOnce("后端连接断开");
  await c.refreshPlayerState();
  await flushPromises();
  expect(wrapper.text()).toContain("130 / 240");
  wrapper.unmount();
  c.dispose();
  await boot();
  expect(wrapper.text()).toContain("130 / 240");
});

it("同一抓包故障自动重试期间只通知一次", async () => {
  liveStatus = {
    enabled: true,
    observation: null,
    state: "error",
    message: "抓包进程已退出, 正在重试.",
    record: null,
  };
  await boot();
  expect(notifications.find((item) => item.key === "player-state")?.count).toBe(
    1,
  );
  liveStatus.state = "starting";
  await c.refreshPlayerState();
  liveStatus.state = "error";
  await c.refreshPlayerState();
  expect(notifications.find((item) => item.key === "player-state")?.count).toBe(
    1,
  );
});
