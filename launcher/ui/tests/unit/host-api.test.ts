import { beforeEach, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  invoke: vi.fn(),
  isDesktop: vi.fn(() => true),
}));
vi.mock("../../src/core/api", () => ({
  invoke: api.invoke,
  isDesktop: api.isDesktop,
}));

import * as c from "../../src/core/controller";
import { bootstrap, settings, withGame } from "./fixtures";
import {
  hostFacts,
  invoke,
  notify,
  openUrl,
  playerActions,
  playerCard,
  playerFacts,
} from "../../src/host-api/api";
import { defaultAttendanceTasks } from "../../src/core/attendanceTasks";
import { settingsNavigation } from "../../src/core/settingsCatalog";
import { notify as hostNotify } from "../../src/core/notifications";

beforeEach(() => {
  c.dispose();
  api.invoke.mockReset().mockResolvedValue(null);
  api.isDesktop.mockReturnValue(true);
});

it("命令入口沿用宿主类型化 invoke", async () => {
  expect(invoke).toBe(api.invoke);
  api.invoke.mockResolvedValueOnce({ items: [], errors: [] });
  await expect(
    invoke("fetch_bulletins", { distribution: "cn" }),
  ).resolves.toEqual({ items: [], errors: [] });
});

it("事实随 bootstrap 与宿主表单更新", () => {
  expect(hostFacts.game.value).toBe("nte");
  expect(hostFacts.distribution.value).toBeNull();
  expect(hostFacts.ready.value).toBe(false);
  expect(hostFacts.page.value).toBe("home");
  expect(hostFacts.gamePhase.value).toBe("idle");
  expect(hostFacts.stackBulletins.value).toBe(false);

  c.state.bootstrap = bootstrap({
    settings: withGame({ distribution: "global" }, settings),
    channels: bootstrap().channels.map((channel) => ({
      ...channel,
      active: channel.id === "global",
    })),
  });
  c.state.page = "nte-bulletin";
  c.state.game = "running";
  c.form.stackBulletins = true;
  expect(hostFacts.game.value).toBe("nte");
  expect(hostFacts.distribution.value).toBe("global");
  expect(hostFacts.ready.value).toBe(true);
  expect(hostFacts.page.value).toBe("nte-bulletin");
  expect(hostFacts.gamePhase.value).toBe("running");
  expect(hostFacts.stackBulletins.value).toBe(true);

  c.state.bootstrap = null;
  c.ui.setupNeeded = true;
  expect(hostFacts.ready.value).toBe(true);
});

it("打开链接走宿主 open_url 命令, 通知复用宿主通知中心", async () => {
  expect(notify).toBe(hostNotify);
  await openUrl("https://example.com/news");
  expect(api.invoke).toHaveBeenCalledWith("open_url", {
    url: "https://example.com/news",
  });
  expect(c.notifications.at(-1)?.message).toBe("已在浏览器打开。");
});

it("玩家卡事实跟随账号与游戏记录状态, 不向包内暴露写入入口", () => {
  const tasks = { ...defaultAttendanceTasks };
  c.syncAttendance({
    accounts: [
      {
        id: "account-1",
        maskedPhone: "138****8000",
        rememberPassword: false,
        signedToday: false,
        failedToday: false,
        lastReport: null,
        attention: null,
      },
    ],
    preferences: { trigger: "off", time: "08:00", tasks },
  });
  expect(playerFacts.accounts.value.map((account) => account.id)).toEqual([
    "account-1",
  ]);
  expect(playerFacts.attendanceLoaded.value).toBe(false);
  c.gameRecords.loading = true;
  c.gameRecords.waiting = true;
  c.gameRecords.error = "网络暂时不可用";
  expect(playerFacts.recordsLoading.value).toBe(true);
  expect(playerFacts.recordsWaiting.value).toBe(true);
  expect(playerFacts.recordsError.value).toBe("网络暂时不可用");
});

it("玩家卡动作复用宿主账号入口与设置跳转", async () => {
  api.invoke.mockResolvedValueOnce({
    accounts: [],
    preferences: {
      trigger: "off",
      time: "08:00",
      tasks: { ...defaultAttendanceTasks },
    },
  });
  await playerActions.loadAccounts();
  expect(api.invoke).toHaveBeenCalledWith("attendance_status");
  expect(playerFacts.attendanceLoaded.value).toBe(true);

  playerCard.expanded.value = true;
  expect(playerCard.expanded.value).toBe(true);
  playerCard.expanded.value = false;
  expect(playerCard.gamepad.value).toBe(false);

  playerActions.openAccountSettings();
  expect(c.state.page).toBe("settings");
  expect(settingsNavigation.category).toBe("nte-account");
});
