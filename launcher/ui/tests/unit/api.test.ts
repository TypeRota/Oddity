import { defaultAttendanceTasks } from "../../src/core/attendanceTasks";
import { APP_VERSION } from "../../src/core/version";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bootstrap, settings, withGame } from "./fixtures";
import { gameSettings } from "../../src/core/game";
import type { Bootstrap, GameStatus } from "../../src/types/launcher";

const native = vi.hoisted(() => ({
  enabled: false,
  invoke: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("../../src/core/desktop", () => ({
  isDesktop: () => native.enabled,
  nativeInvoke: native.invoke,
  listen: native.listen,
}));
beforeEach(() => {
  native.enabled = false;
  native.invoke.mockReset();
  native.listen.mockReset();
  delete window.__NTE_PREVIEW__;
});

describe("浏览器命令桥接", () => {
  it("创建副本验证指定的源环境及其安装范围", async () => {
    const { invoke } = await import("../../src/core/api");
    const first = await invoke("environment_create", {
      distribution: "cn",
      name: "A",
      protonPath: "/proton-a",
      sourceId: null,
    });
    const id = first.settings.runtimeEnvironments[0].id;
    const copied = await invoke("environment_create", {
      distribution: "cn",
      name: "A 副本",
      protonPath: "/proton-b",
      sourceId: id,
    });
    expect(copied.settings.runtimeEnvironments).toHaveLength(2);
    expect(gameSettings(copied.settings).activeEnvironment).toBe(id);
    await expect(
      invoke("environment_create", {
        distribution: "cn",
        name: "缺失源",
        protonPath: "/proton-b",
        sourceId: "missing",
      }),
    ).rejects.toThrow("源运行环境不存在");
    await expect(
      invoke("environment_create", {
        distribution: "global",
        name: "国际服",
        protonPath: "/proton-b",
        sourceId: id,
      }),
    ).rejects.toThrow("尚未安装");
    await invoke("save_settings", {
      settings: withGame(
        { globalInstallRoot: "/global-game" },
        copied.settings,
      ),
    });
    const global = await invoke("environment_create", {
      distribution: "global",
      name: "国际服",
      protonPath: "/proton-b",
      sourceId: id,
    });
    const profile = global.settings.runtimeEnvironments.at(-1)!;
    expect(profile).toMatchObject({
      distribution: "global",
      installRoot: "/global-game",
    });
    expect(gameSettings(global.settings).distribution).toBe("cn");
    const switched = await invoke("environment_action", {
      action: "switch",
      id: profile.id,
    });
    expect(gameSettings(switched.settings).distribution).toBe("global");
    expect(switched.installRoot).toBe("/global-game");
  });
  it("模拟环境创建、切换、保留和删除, 并保护当前和稳定环境", async () => {
    const { invoke } = await import("../../src/core/api");
    await expect(
      invoke("environment_create", {
        distribution: "cn",
        name: "",
        protonPath: "/new",
        sourceId: null,
      }),
    ).rejects.toThrow("请填写");
    const created = await invoke("environment_create", {
      distribution: "cn",
      sourceId: null,
      name: "测试",
      protonPath: "/proton-b",
    });
    const id = created.settings.runtimeEnvironments[0].id;
    await expect(
      invoke("environment_create", {
        distribution: "cn",
        name: "测试",
        protonPath: "/proton-b",
        sourceId: null,
      }),
    ).rejects.toThrow("已存在");
    await expect(
      invoke("environment_action", { action: "switch", id: "missing" }),
    ).rejects.toThrow("不存在");
    const selected = await invoke("environment_action", {
      action: "switch",
      id,
    });
    expect(selected.prefix).toContain(`/environments/${id}/prefix`);
    expect(selected.protonPath).toBe("/proton-b");
    await expect(
      invoke("environment_action", { action: "discard", id }),
    ).rejects.toThrow("不能删除");
    await invoke("environment_action", { action: "keep", id });
    const next = await invoke("environment_create", {
      distribution: "cn",
      name: "其他环境",
      protonPath: "/proton-c",
      sourceId: null,
    });
    const otherId = next.settings.runtimeEnvironments[1].id;
    await invoke("environment_action", { action: "switch", id: otherId });
    await expect(
      invoke("environment_action", { action: "discard", id }),
    ).rejects.toThrow("不能删除");
    await invoke("environment_action", { action: "keep", id: otherId });
    const removed = await invoke("environment_action", {
      action: "discard",
      id,
    });
    expect(
      removed.settings.runtimeEnvironments.map((entry) => entry.id),
    ).toEqual([otherId]);
    await expect(
      invoke("environment_action", { action: "discard", id: null }),
    ).rejects.toThrow("不存在");
  });

  it("预览启动按阶段广播进度并支持暂停", async () => {
    const { invoke, listenGameState } = await import("../../src/core/api");
    const statuses: GameStatus[] = [];
    const dispose = listenGameState((status) => statuses.push(status));
    await invoke("environment_create", {
      distribution: "cn",
      name: "测试",
      protonPath: "/proton",
      sourceId: null,
    });
    await invoke("launch_official");
    expect(statuses.at(-1)?.launch).toMatchObject({
      stage: "preparing",
      cancelling: false,
    });
    await vi.advanceTimersByTimeAsync(600);
    expect(statuses.at(-1)?.launch?.stage).toBe("helper");
    await vi.advanceTimersByTimeAsync(700);
    expect(statuses.at(-1)?.launch?.stage).toBe("resources");
    await vi.advanceTimersByTimeAsync(1700);
    expect(statuses.at(-1)?.launch?.stage).toBe("game");
    await vi.advanceTimersByTimeAsync(700);
    expect(statuses.at(-1)?.launch).toBeNull();
    expect(statuses.at(-1)?.running).toBe(true);

    await invoke("launch_official");
    await vi.advanceTimersByTimeAsync(600);
    expect(statuses.at(-1)?.launch?.stage).toBe("helper");
    const clear = vi.spyOn(window, "clearTimeout");
    const cancelling = invoke("cancel_launch");
    await vi.advanceTimersByTimeAsync(1);
    expect(clear).toHaveBeenCalledTimes(4);
    expect(statuses.at(-1)?.launch).toMatchObject({
      stage: "helper",
      cancelling: true,
    });
    await vi.advanceTimersByTimeAsync(450);
    expect(await cancelling).toMatchObject({ running: false, launch: null });
    expect(statuses.at(-1)?.running).toBe(false);
    clear.mockRestore();
    dispose();

    const before = statuses.length;
    await invoke("launch_official");
    await vi.advanceTimersByTimeAsync(4000);
    expect(statuses.length).toBe(before);
  });

  it("预览资源任务发布预下载目录字段并支持删除", async () => {
    const { invoke } = await import("../../src/core/api");
    await invoke("game_download_start", { mode: "apply", full: false });
    await vi.advanceTimersByTimeAsync(1500);
    const applied = await invoke("game_download_status");
    expect(applied.phase).toBe("ready");
    expect(applied.remoteVersion).toBe("1.4.17");
    expect(applied.verify).toBe(false);

    const initialRequest = invoke("game_resource_catalog");
    await vi.advanceTimersByTimeAsync(200);
    const initial = await initialRequest;
    expect(initial.preloadAvailable).toBe(true);
    expect(initial.preloadCompleted).toBe(false);
    expect(initial.preloadCached).toBe(false);

    await invoke("game_download_start", { mode: "preload", full: false });
    await vi.advanceTimersByTimeAsync(1500);
    const completedRequest = invoke("game_resource_catalog");
    await vi.advanceTimersByTimeAsync(200);
    const completed = await completedRequest;
    expect(completed.preloadCompleted).toBe(true);
    expect(completed.preloadCached).toBe(true);
    expect(completed.preloadCachedVersion).toBe("1.5.0");
    expect(completed.preloadCachedBytes).toBeGreaterThan(0);

    const cleared = await invoke("game_preload_delete");
    expect(cleared.preloadCompleted).toBe(false);
    expect(cleared.preloadCached).toBe(false);
    expect(cleared.preloadCachedVersion).toBe("");
  });

  it("使用默认路径并返回互不共享的启动快照", async () => {
    const { invoke, listenGameState } = await import("../../src/core/api");
    const first = await invoke("bootstrap");
    expect(first.appVersion).toBe(APP_VERSION);
    expect(first.integrity.state).toBe("verified");
    expect(first.integrity.releaseUrl).toContain("oddity-v");
    expect(first.installRoot).toBe("~/Games/Neverness To Everness");
    first.settings.theme = "dark";
    first.checks.length = 0;
    const second = await invoke("bootstrap");
    expect(second.settings.theme).toBe("light");
    expect(second.checks).toHaveLength(6);
    listenGameState(vi.fn())();
    expect(native.listen).not.toHaveBeenCalled();
  });

  it("查询参数优先于本地存储和窗口配置, 并清理路径斜杠", async () => {
    window.__NTE_PREVIEW__ = {
      home: "/window/",
      dataHome: "/data/",
      protonName: "Window",
    };
    localStorage.setItem(
      "nte.preview.paths",
      JSON.stringify({
        home: "/stored/",
        steamRoot: "/steam/",
        protonName: "Stored",
      }),
    );
    history.replaceState(
      null,
      "",
      "/?home=%20/query/%20&installRoot=/game/&protonName=Query",
    );
    const { invoke } = await import("../../src/core/api");
    const data = await invoke("bootstrap");
    expect(data.installRoot).toBe("/game/");
    expect(data.launcherRoot).toBe("/game/NTELauncher");
    expect(data.protonPath).toBe("");
    const created = await invoke("environment_create", {
      distribution: "cn",
      name: "测试",
      protonPath: "/proton",
      sourceId: null,
    });
    expect(created.prefix).toContain("/data/oddity/environments/");
  });

  it.each(["{broken", "null", "42"])("忽略无效本地配置: %s", async (stored) => {
    localStorage.setItem("nte.preview.paths", stored);
    history.replaceState(
      null,
      "",
      "/?home=++&dataHome=&steamRoot=&installRoot=&protonName=",
    );
    const { invoke } = await import("../../src/core/api");
    expect((await invoke("bootstrap")).installRoot).toBe(
      "~/Games/Neverness To Everness",
    );
  });

  it.each([
    ["Win32", "x86_64", "windows / x86_64"],
    ["MacIntel", "arm64", "macos / aarch64"],
    ["Linux", "aarch64", "linux / aarch64"],
    ["", "other", "linux / x86_64"],
  ])("显示预览平台 %s / %s", async (platform, agent, expected) => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(agent);
    const { invoke } = await import("../../src/core/api");
    expect((await invoke("bootstrap")).os).toBe(
      `${expected} / browser preview`,
    );
  });

  it("检查、保存、启动、关闭及窗口命令维持预览状态", async () => {
    const { invoke } = await import("../../src/core/api");
    const update = invoke("check_update", { full: true });
    await vi.advanceTimersByTimeAsync(620);
    expect(await update).toMatchObject({
      installed: true,
      distribution: "cn",
      installRoot: "~/Games/Neverness To Everness",
      missing: 0,
      changed: 0,
      files: 667,
    });
    expect(
      await invoke("save_settings", {
        settings: { ...withGame({ installRoot: "/new" }), theme: "dark" },
      }),
    ).toMatchObject({ installRoot: "/new", settings: { theme: "dark" } });
    await invoke("save_settings", { settings: { ...settings } });
    expect((await invoke("bootstrap")).installRoot).toBe(
      "~/Games/Neverness To Everness",
    );
    await expect(invoke("launch_official")).rejects.toThrow("请先创建");
    await invoke("environment_create", {
      distribution: "cn",
      name: "测试",
      protonPath: "/proton",
      sourceId: null,
    });
    expect(await invoke("launch_official")).toMatchObject({
      mode: "background",
      pid: 23140,
    });
    const snapshot = await invoke("game_status");
    expect(snapshot.running).toBe(true);
    snapshot.running = false;
    expect((await invoke("game_status")).running).toBe(true);
    const clear = vi.spyOn(window, "clearTimeout");
    const closing = invoke("close_game");
    expect((await invoke("game_status")).closing).toBe(true);
    await vi.advanceTimersByTimeAsync(850);
    expect(await closing).toMatchObject({
      running: false,
      closing: false,
      pid: 0,
    });
    expect(clear).toHaveBeenCalledTimes(4);
    clear.mockRestore();
    await vi.advanceTimersByTimeAsync(4000);
    expect((await invoke("game_status")).launch).toBeNull();
    const playtime = await invoke("playtime_history");
    expect(playtime.sessions.length).toBeGreaterThan(0);
    expect(playtime.active).toBeNull();
    expect(
      await invoke("playtime_export_poster", {
        destination: "/tmp/poster.png",
        data: "preview",
      }),
    ).toBe("/tmp/poster.png");
    expect(await invoke("open_url", { url: "https://example.com" })).toBeNull();
    expect(
      await invoke("integrity_acknowledge", { notice: "success" }),
    ).toBeNull();
    for (const command of [
      "win_minimize",
      "win_toggle_maximize",
      "win_close",
      "win_hide",
    ] as const) {
      expect(await invoke(command)).toBeNull();
    }
  });
});

describe("原生命令和事件", () => {
  beforeEach(() => {
    native.enabled = true;
  });
  it("原样转发命令参数、返回值和错误", async () => {
    const { invoke } = await import("../../src/core/api");
    native.invoke
      .mockResolvedValueOnce({ saved: true })
      .mockRejectedValueOnce(new Error("offline"));
    await expect(invoke("save_settings", { settings })).resolves.toEqual({
      saved: true,
    });
    expect(native.invoke).toHaveBeenCalledWith("save_settings", { settings });
    await expect(invoke("bootstrap")).rejects.toThrow("offline");
  });

  it("传递设置变更事件并在订阅完成后解除监听", async () => {
    const { listenSettingsChanged } = await import("../../src/core/api");
    const stop = vi.fn();
    native.listen.mockResolvedValue(stop);
    const handler = vi.fn();
    const dispose = listenSettingsChanged(handler);
    const callback = native.listen.mock.calls[0][1] as (event: {
      payload: Bootstrap;
    }) => void;
    const payload = bootstrap({ installRoot: "/games/global" });
    callback({ payload });
    expect(handler).toHaveBeenCalledWith(payload);
    await Promise.resolve();
    dispose();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("设置变更订阅完成前解除也能释放资源", async () => {
    const { listenSettingsChanged } = await import("../../src/core/api");
    let resolve!: (stop: () => void) => void;
    native.listen.mockReturnValue(
      new Promise<() => void>((done) => {
        resolve = done;
      }),
    );
    const dispose = listenSettingsChanged(vi.fn());
    dispose();
    const stop = vi.fn();
    resolve(stop);
    await Promise.resolve();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("设置变更监听注册失败时不产生未处理的拒绝", async () => {
    const { listenSettingsChanged } = await import("../../src/core/api");
    native.listen.mockRejectedValue(new Error("listener failed"));
    const dispose = listenSettingsChanged(vi.fn());
    await Promise.resolve();
    await Promise.resolve();
    expect(dispose).not.toThrow();
  });

  it("传递安全验证结果并在订阅完成后解除监听", async () => {
    const { listenIntegrityState } = await import("../../src/core/api");
    const stop = vi.fn();
    native.listen.mockResolvedValue(stop);
    const handler = vi.fn();
    const dispose = listenIntegrityState(handler);
    const callback = native.listen.mock.calls[0][1] as (event: {
      payload: Bootstrap["integrity"];
    }) => void;
    const payload = bootstrap().integrity;
    callback({ payload });
    expect(handler).toHaveBeenCalledWith(payload);
    await Promise.resolve();
    dispose();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("安全验证订阅完成前解除也能释放资源", async () => {
    const { listenIntegrityState } = await import("../../src/core/api");
    let resolve!: (stop: () => void) => void;
    native.listen.mockReturnValue(
      new Promise<() => void>((done) => {
        resolve = done;
      }),
    );
    const dispose = listenIntegrityState(vi.fn());
    dispose();
    const stop = vi.fn();
    resolve(stop);
    await Promise.resolve();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("传递事件快照并在订阅完成后解除监听", async () => {
    const { listenGameState } = await import("../../src/core/api");
    const stop = vi.fn();
    native.listen.mockResolvedValue(stop);
    const handler = vi.fn();
    const dispose = listenGameState(handler);
    const callback = native.listen.mock.calls[0][1] as (event: {
      payload: GameStatus;
    }) => void;
    const payload = { running: true } as GameStatus;
    callback({ payload });
    expect(handler).toHaveBeenCalledWith(payload);
    await Promise.resolve();
    dispose();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("在异步订阅完成前解除监听也能释放资源", async () => {
    const { listenGameState } = await import("../../src/core/api");
    let resolve!: (stop: () => void) => void;
    native.listen.mockReturnValue(
      new Promise<() => void>((done) => {
        resolve = done;
      }),
    );
    const dispose = listenGameState(vi.fn());
    dispose();
    const stop = vi.fn();
    resolve(stop);
    await Promise.resolve();
    expect(stop).toHaveBeenCalledOnce();
  });

  it("监听注册失败时不产生未处理的拒绝", async () => {
    const { listenGameState } = await import("../../src/core/api");
    native.listen.mockRejectedValue(new Error("listener failed"));
    const dispose = listenGameState(vi.fn());
    await Promise.resolve();
    await Promise.resolve();
    expect(dispose).not.toThrow();
  });
});

it("游玩记录预览在运行时返回活动会话并隔离快照", async () => {
  const { invoke } = await import("../../src/core/api");
  const before = await invoke("playtime_history");
  expect(before.active).toBeNull();
  expect(
    before.sessions.every(
      (entry) => entry.startedAt < entry.endedAt && entry.endedAt <= Date.now(),
    ),
  ).toBe(true);
  await invoke("environment_create", {
    distribution: "cn",
    name: "测试",
    protonPath: "/proton",
    sourceId: null,
  });
  await invoke("launch_official");
  const running = await invoke("playtime_history");
  expect(running.active).toMatchObject({
    mode: "background",
    distribution: "cn",
    lastSeenAt: Date.now(),
  });
  expect(running.active?.startedAt).toBe(
    (await invoke("game_status")).startedAt,
  );
  running.sessions.length = 0;
  running.active!.mode = "mutated";
  const fresh = await invoke("playtime_history");
  expect(fresh.sessions).toEqual(before.sessions);
  expect(fresh.active?.mode).toBe("background");
  const closing = invoke("close_game");
  await vi.advanceTimersByTimeAsync(850);
  await closing;
  const finished = await invoke("playtime_history");
  expect(finished.active).toBeNull();
  expect(finished.sessions).toHaveLength(before.sessions.length + 1);
  expect(finished.sessions.at(-1)).toMatchObject({
    endedAt: Date.now(),
    mode: "background",
    distribution: "cn",
  });
});

it("游戏状态预览覆盖全部账号, 保留零次周本且不改变签到状态", async () => {
  const { invoke } = await import("../../src/core/api");
  expect((await invoke("attendance_refresh_game_records")).accounts).toEqual(
    [],
  );
  for (const phone of ["13800138000", "13900139000"])
    await invoke("attendance_login", {
      phone,
      password: "preview-only",
      rememberPassword: false,
    });
  const result = await invoke("attendance_refresh_game_records");
  expect(result.accounts.map((a) => a.gameRecord?.weeklyRemaining)).toEqual([
    2, 0,
  ]);
  expect(result.accounts.map((a) => a.gameRecord?.stamina)).toEqual([168, 240]);
  expect(
    result.accounts.every((a) => a.lastReport === null && !a.signedToday),
  ).toBe(true);
  result.accounts.length = 0;
  expect((await invoke("attendance_status")).accounts).toHaveLength(2);
});

it("签到预览默认关闭, 手机号密码登录不保存原始凭据且不会自动开启", async () => {
  const { invoke } = await import("../../src/core/api");
  expect((await invoke("attendance_status")).preferences.trigger).toBe("off");
  await expect(invoke("attendance_run")).rejects.toThrow("请先登录");
  await expect(
    invoke("attendance_login", {
      phone: "bad",
      password: "",
      rememberPassword: false,
    }),
  ).rejects.toThrow();
  await expect(
    invoke("attendance_configure", {
      preferences: {
        trigger: "daily",
        time: "20:30",
        tasks: { ...defaultAttendanceTasks },
      },
    }),
  ).rejects.toThrow("请先登录");
  const logged = await invoke("attendance_login", {
    phone: "13800138000",
    password: "test-password",
    rememberPassword: true,
  });
  expect(logged.accounts).toHaveLength(1);
  expect(logged.accounts[0].rememberPassword).toBe(true);
  expect(logged.preferences.trigger).toBe("off");
  expect(logged.accounts[0].maskedPhone).toBe("138****8000");
  expect(JSON.stringify(logged)).not.toContain("test-password");
  expect(JSON.stringify(logged)).not.toContain("13800138000");
  expect(localStorage.length).toBe(0);
  await invoke("attendance_keyring_check");
  await invoke("attendance_configure", {
    preferences: {
      trigger: "launch",
      time: "08:00",
      tasks: { ...defaultAttendanceTasks },
    },
  });
  expect((await invoke("attendance_run")).accounts[0].lastReport?.ok).toBe(
    true,
  );
  const second = await invoke("attendance_login", {
    phone: "13900139000",
    password: "other-password",
    rememberPassword: false,
  });
  expect(second.accounts).toHaveLength(2);
  const relogged = await invoke("attendance_login", {
    phone: "13800138000",
    password: "new-password",
    rememberPassword: false,
  });
  expect(relogged.accounts.map((account) => account.id)).toEqual(
    second.accounts.map((account) => account.id),
  );
  expect(relogged.accounts[0].lastReport).toBeNull();
  expect(relogged.accounts[0].signedToday).toBe(false);
  const signed = await invoke("attendance_run");
  expect(signed.accounts.every((account) => account.signedToday)).toBe(true);
  const remaining = await invoke("attendance_logout", {
    accountId: second.accounts[1].id,
  });
  expect(remaining.accounts).toHaveLength(1);
  expect(remaining.preferences.trigger).toBe("launch");
  const cleared = await invoke("attendance_logout", {
    accountId: logged.accounts[0].id,
  });
  expect(cleared.accounts).toHaveLength(0);
  expect(cleared.preferences.trigger).toBe("off");
});

it("预览短信登录校验输入并更新同一账号", async () => {
  const { invoke } = await import("../../src/core/api");
  await expect(
    invoke("attendance_send_code", { phone: "123" }),
  ).rejects.toThrow("手机号码");
  const phone = "13700137000";
  const deviceId = await invoke("attendance_send_code", { phone });
  expect(await invoke("attendance_send_code", { phone })).toBe(deviceId);
  await expect(
    invoke("attendance_login_sms", { phone, code: "abc", deviceId }),
  ).rejects.toThrow("验证码");
  await expect(
    invoke("attendance_login_sms", { phone: "1", code: "123456", deviceId }),
  ).rejects.toThrow("手机号码");
  await expect(
    invoke("attendance_login_sms", {
      phone,
      code: "123456",
      deviceId: "wrong",
    }),
  ).rejects.toThrow("获取验证码");
  const status = await invoke("attendance_login_sms", {
    phone,
    code: "123456",
    deviceId,
  });
  const entry = status.accounts.find((a) => a.maskedPhone === "137****7000")!;
  expect(entry.rememberPassword).toBe(false);
  expect(JSON.stringify(status)).not.toContain("123456");
  const again = await invoke("attendance_login_sms", {
    phone,
    code: "123456",
    deviceId,
  });
  expect(again.accounts.filter((a) => a.id === entry.id)).toHaveLength(1);
  await invoke("attendance_logout", { accountId: entry.id });
});

it("模拟环境快照的对比, 参考保存和清理", async () => {
  const { invoke } = await import("../../src/core/api");
  const overview = await invoke("environment_snapshot_overview");
  expect(overview.reference?.label).toBe("更新驱动前");
  expect(overview.baseline?.proton).toBe("GE-Proton9-20");
  expect(overview.latest?.proton).toBe("GE-Proton9-21");

  const comparison = await invoke("environment_snapshot_compare", {
    target: "baseline",
  });
  expect(comparison.changed).toBeGreaterThan(0);
  expect(comparison.unknown).toBeGreaterThan(0);
  expect(comparison.rows.some((row) => row.status === "same")).toBe(true);

  const saved = await invoke("environment_snapshot_save_reference", {
    label: "手动参考",
    attemptId: null,
  });
  expect(saved.label).toBe("手动参考");
  expect((await invoke("environment_snapshot_overview")).reference?.label).toBe(
    "手动参考",
  );

  await invoke("environment_snapshot_delete_reference");
  await invoke("environment_snapshot_clear_history");
  const cleared = await invoke("environment_snapshot_overview");
  expect(cleared.reference).toBeNull();
  expect(cleared.baseline).toBeNull();
  expect(cleared.latest).toBeNull();
  expect(cleared.attempts).toBe(0);
});

it("模拟安装目录迁移命令", async () => {
  const { invoke } = await import("../../src/core/api");
  const plan = await invoke("migration_plan", {
    distribution: "cn",
    target: "/mnt/games",
  });
  expect(plan.target).toBe("/mnt/games");
  expect(plan.environments).toHaveLength(0);
  const started = await invoke("migration_start", {
    distribution: "cn",
    target: plan.target,
  });
  expect(started.phase).toBe("done");
  expect((await invoke("migration_status")).target).toBe("/mnt/games");
  expect((await invoke("migration_cancel")).phase).toBe("cancelled");
  await invoke("migration_cleanup");
  expect((await invoke("migration_retry")).phase).toBe("cancelled");
  await invoke("migration_dismiss");
  expect((await invoke("migration_status")).phase).toBe("idle");
  expect((await invoke("migration_abandon")).phase).toBe("idle");
});

it("桌面会话、下载和迁移订阅转发载荷, 处理提前注销与订阅失败", async () => {
  native.enabled = true;
  const api = await import("../../src/core/api");
  for (const subscribe of [
    api.listenPlaytimeStarted,
    api.listenDownloadState,
    api.listenMigrationState,
  ]) {
    const unlisten = vi.fn();
    native.listen.mockResolvedValueOnce(unlisten);
    const handler = vi.fn();
    const dispose = subscribe(handler);
    await Promise.resolve();
    const [event, callback] = native.listen.mock.calls.at(-1)! as [
      string,
      (event: { payload: { marker: string } }) => void,
    ];
    callback({ payload: { marker: "current-session" } });
    expect(handler).toHaveBeenCalledOnce();
    if (event !== "playtime-started")
      expect(handler).toHaveBeenCalledWith({ marker: "current-session" });
    dispose();
    expect(unlisten).toHaveBeenCalledOnce();
    let complete!: (stop: () => void) => void;
    native.listen.mockReturnValueOnce(
      new Promise<() => void>((resolve) => {
        complete = resolve;
      }),
    );
    const early = subscribe(vi.fn());
    early();
    const lateStop = vi.fn();
    complete(lateStop);
    await Promise.resolve();
    expect(lateStop).toHaveBeenCalledOnce();
    native.listen.mockRejectedValueOnce(new Error("服务已关闭"));
    const failed = subscribe(vi.fn());
    await Promise.resolve();
    await Promise.resolve();
    failed();
  }
  native.enabled = false;
  const calls = native.listen.mock.calls.length;
  api.listenPlaytimeStarted(vi.fn())();
  api.listenDownloadState(vi.fn())();
  api.listenMigrationState(vi.fn())();
  expect(native.listen).toHaveBeenCalledTimes(calls);
});

it("浏览器环境配置计划保留选择, 应用后创建并选中独立运行环境", async () => {
  const { invoke } = await import("../../src/core/api");
  await expect(invoke("runtime_setup_apply")).rejects.toThrow(
    "请先查看安装计划",
  );
  const before = await invoke("runtime_setup_info");
  expect(before.environment).toBeNull();
  const plan = await invoke("runtime_setup_plan", {
    allUsers: true,
    proton: "/test/proton",
    createEnvironment: true,
  });
  expect(plan).toMatchObject({
    proton: "/test/proton",
    installProton: false,
    systemCommand: expect.arrayContaining(["umu-launcher"]),
  });
  const applying = invoke("runtime_setup_apply");
  await vi.advanceTimersByTimeAsync(4200);
  await applying;
  expect(await invoke("runtime_setup_status")).toMatchObject({
    busy: false,
    done: true,
    error: null,
  });
  const configured = await invoke("runtime_setup_environment");
  const current = gameSettings(configured.settings);
  expect(current.activeEnvironment).toBeTruthy();
  expect(configured.settings.runtimeEnvironments).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: current.activeEnvironment,
        proton: expect.objectContaining({ path: "/test/proton" }),
      }),
    ]),
  );
  expect(await invoke("runtime_setup_info")).toMatchObject({
    proton: "/test/proton",
    systemUmu: true,
  });
  const noInstall = await invoke("runtime_setup_plan", {
    allUsers: true,
    proton: "/test/proton",
    createEnvironment: false,
  });
  expect(noInstall.systemCommand).toBeNull();
  const localPlan = await invoke("runtime_setup_plan", {
    allUsers: false,
    proton: null,
    createEnvironment: false,
  });
  expect(localPlan.installProton).toBe(true);
  expect(localPlan.umu).toContain("oddity/runtime/umu");
  await invoke("runtime_setup_dismiss");
  expect((await invoke("runtime_setup_status")).busy).toBe(false);
});

it("浏览器能力信息明确不可用, 原生专用操作不会伪装成功", async () => {
  const { invoke } = await import("../../src/core/api");
  expect(await invoke("game_mode_tuning_info")).toEqual({
    pinCores: null,
    parkCores: null,
    renice: null,
    ioprio: null,
  });
  expect((await invoke("game_mode_gpu_info")).devices).toEqual([]);
  expect(
    (await invoke("game_mode_status")).capabilities.every(
      (entry) => !entry.available && entry.state === "unavailable",
    ),
  ).toBe(true);
  await expect(
    invoke("game_mode_gpu_apply", {
      config: (await invoke("game_mode_gpu_info")).config,
    }),
  ).rejects.toThrow("原生启动器");
  await expect(
    invoke("game_mode_tuning_apply", {
      config: await invoke("game_mode_tuning_info"),
    }),
  ).rejects.toThrow("原生启动器");
  await expect(invoke("browse_files", { path: null })).rejects.toThrow(
    "桌面应用",
  );
  await expect(
    invoke("gamepad_profile_load", { path: "/profile.json" }),
  ).rejects.toThrow("桌面应用");
  await expect(
    invoke("game_send_code", { phone: "13800000000" }),
  ).rejects.toThrow("桌面启动器");
  expect(await invoke("game_accounts")).toEqual([]);
  expect(
    await invoke("game_account_remove", { accountId: "missing" }),
  ).toBeNull();
  expect(await invoke("take_launch_error")).toBeNull();
});
