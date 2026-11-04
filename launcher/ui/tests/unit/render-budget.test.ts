import { describe, expect, it, vi } from "vitest";
import {
  createRenderBudget,
  STILL_TIER,
  type RenderBudget,
  type Tier,
} from "../../src/core/renderBudget";

// 与被测模块保持一致的时间常量。
const WARMUP_MS = 2000;
const WARMUP_SAMPLES = 8;
const WINDOW_SAMPLES = 30;
const DEGRADE_WINDOWS = 3;
const RECOVER_WINDOWS = 12;
const STILL_RETRY_MS = 20000;
const MAX_STILL_RETRY_MS = 120000;
const FLAP_MS = 30000;
const MAX_INTERVAL_MS = 500;

// 基线取 20ms, 这样各档间隔下 30 个样本都短于 1 秒窗口, 窗口一律按样本数关闭,
// 判定次数在测试里是确定的。
const BASE = 20;
const HEALTHY = BASE; // 比值 1.0, 判好
const DEAD_ZONE = BASE * 1.25; // 比值 1.25, 落在 1.10 与 1.45 之间
const OVERLOAD = BASE * 1.5; // 比值 1.5, 判坏
/// 极低帧率: 样本数很少但已越过 1 秒窗口时长。
const CRAWL = MAX_INTERVAL_MS - 1;

function makeClock() {
  const clock = { now: 0 };
  return { clock, time: () => clock.now };
}

/// 第一次调用只建立时间基准, 不产生间隔样本。
function prime(budget: RenderBudget, clock: { now: number }) {
  budget.recordPaint(clock.now);
}

/// 按固定帧间隔推进假时钟, 每次调用产生恰好一个间隔样本。
function feedIntervals(
  budget: RenderBudget,
  clock: { now: number },
  count: number,
  intervalMs: number,
) {
  const drawn: boolean[] = [];
  for (let i = 0; i < count; i += 1) {
    clock.now += intervalMs;
    drawn.push(budget.recordPaint(clock.now));
  }
  return drawn;
}

const intervals = (windows: number) => windows * WINDOW_SAMPLES;

/// 走完预热期。帧数刚好卡在时长与样本数同时满足的那一帧上,
/// 判定窗口从干净边界开始, 不会混入预热期的尾巴。
function warmup(budget: RenderBudget, clock: { now: number }) {
  prime(budget, clock);
  feedIntervals(budget, clock, Math.ceil(WARMUP_MS / BASE) - 1, BASE);
  expect(budget.warming).toBe(true);
  feedIntervals(budget, clock, 1, BASE);
  expect(budget.warming).toBe(false);
  const baseline = budget.snapshot().baseline;
  expect(baseline).toBe(BASE);
  expect(budget.snapshot().ratio).toBe(1);
  return baseline;
}

/// 持续投喂坏窗口直到降到目标档, 驻留期由时钟自然推进跨过。
function degradeTo(budget: RenderBudget, clock: { now: number }, target: Tier) {
  for (let guard = 0; budget.tier < target && guard < 80; guard += 1)
    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS), OVERLOAD);
  expect(budget.tier).toBe(target);
}

/// 持续投喂好窗口直到升回目标档。
function recoverTo(budget: RenderBudget, clock: { now: number }, target: Tier) {
  for (let guard = 0; budget.tier > target && guard < 80; guard += 1)
    feedIntervals(budget, clock, intervals(RECOVER_WINDOWS + 2), HEALTHY);
  expect(budget.tier).toBe(target);
}

describe("渲染预算分档", () => {
  it("预热期只采基线, 不做任何判定", () => {
    // 样本够了但时长不足: 这两个窗口的坏数据不能触发降级。
    const first = makeClock();
    const changes: Tier[] = [];
    const budget = createRenderBudget({
      clock: first.time,
      onChange: (tier) => changes.push(tier),
    });
    prime(budget, first.clock);
    feedIntervals(budget, first.clock, 60, OVERLOAD);
    expect(budget.warming).toBe(true);
    expect(budget.snapshot().baseline).toBe(0);
    expect(budget.tier).toBe(0);
    expect(changes).toEqual([]);

    // 时长够了但样本不足: 仍然不结束预热。
    const second = makeClock();
    const other = createRenderBudget({ clock: second.time });
    prime(other, second.clock);
    second.clock.now += WARMUP_MS + 100;
    // 跨度过长的第一个间隔被丢弃, 有效样本只剩 7 个。
    feedIntervals(other, second.clock, WARMUP_SAMPLES, HEALTHY);
    expect(other.warming).toBe(true);
    expect(other.snapshot().baseline).toBe(0);
    expect(other.tier).toBe(0);
  });

  it("健康节奏下保持满速, 首帧必画", () => {
    const { clock, time } = makeClock();
    const changes: Tier[] = [];
    const budget = createRenderBudget({
      clock: time,
      onChange: (tier) => changes.push(tier),
    });

    expect(budget.recordPaint(clock.now)).toBe(true);
    expect(budget.tier).toBe(0);
    expect(budget.cadence).toBe(1);

    warmup(budget, clock);
    feedIntervals(budget, clock, intervals(6), HEALTHY);
    expect(budget.tier).toBe(0);
    expect(changes).toEqual([]);
    expect(budget.snapshot().ratio).toBe(1);
  });

  it("节奏变坏时逐级降到静帧, 到底后不再通知", () => {
    const { clock, time } = makeClock();
    const changes: Tier[] = [];
    const budget = createRenderBudget({
      clock: time,
      onChange: (tier) => changes.push(tier),
    });
    warmup(budget, clock);

    degradeTo(budget, clock, 1);
    degradeTo(budget, clock, 2);
    degradeTo(budget, clock, STILL_TIER);
    expect(changes).toEqual([1, 2, STILL_TIER]);
    expect(budget.snapshot().degraded).toBe(3);

    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS * 3), OVERLOAD);
    expect(changes).toEqual([1, 2, STILL_TIER]);
    expect(budget.snapshot().degraded).toBe(3);
  });

  it("坏窗口被好窗口打断时不降级, 连续三个才降", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);

    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS - 1), OVERLOAD);
    feedIntervals(budget, clock, intervals(1), HEALTHY);
    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS - 1), OVERLOAD);
    expect(budget.tier).toBe(0);

    // 不被打断的三个坏窗口就会降级, 说明上面确实是连击被清掉了。
    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS), OVERLOAD);
    expect(budget.tier).toBe(1);
  });

  it("节奏档按间隔跳帧, 静帧档完全不画", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);

    degradeTo(budget, clock, 1);
    expect(budget.cadence).toBe(2);
    // 用健康间隔观察绘制模式, 避免观测本身推动档位变化。
    const half = feedIntervals(budget, clock, 12, HEALTHY);
    expect(half.filter(Boolean)).toHaveLength(6);
    for (let i = 1; i < half.length; i += 1)
      expect(half[i] && half[i - 1]).toBe(false);

    degradeTo(budget, clock, 2);
    expect(budget.cadence).toBe(3);
    const slow = feedIntervals(budget, clock, 12, HEALTHY);
    expect(slow.filter(Boolean)).toHaveLength(4);

    degradeTo(budget, clock, STILL_TIER);
    expect(budget.cadence).toBe(0);
    expect(
      feedIntervals(budget, clock, 10, HEALTHY).every((drew) => !drew),
    ).toBe(true);
  });

  it("静帧档只能靠试探退回低速档, 再由实测升回满速", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, STILL_TIER);

    budget.attemptStillRecovery(clock.now);
    expect(budget.tier).toBe(STILL_TIER - 1);
    // 非静帧档调用无效。
    budget.attemptStillRecovery(clock.now);
    expect(budget.tier).toBe(STILL_TIER - 1);

    recoverTo(budget, clock, 0);
    expect(budget.snapshot().recovered).toBeGreaterThan(0);
    expect(budget.snapshot().flapFactor).toBe(1);
  });

  it("好窗口不足时不升级", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, 1);
    // 11 个好窗口跨过驻留期但仍不足 12 个。
    feedIntervals(budget, clock, intervals(RECOVER_WINDOWS - 1), HEALTHY);
    expect(budget.tier).toBe(1);
  });

  it("驻留期内不升级也不降级", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, 1);
    const degraded = budget.snapshot().degraded;

    // 三个坏窗口只占 2.7 秒, 短于 5 秒驻留期。
    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS), OVERLOAD);
    expect(budget.tier).toBe(1);
    expect(budget.snapshot().degraded).toBe(degraded);
    // 继续投喂跨过驻留期后才降, 坏窗口计数在驻留期内被保留。
    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS), OVERLOAD);
    expect(budget.tier).toBe(2);
    expect(budget.snapshot().degraded).toBe(degraded + 1);

    // 升级同样受驻留期约束。
    feedIntervals(budget, clock, intervals(5), HEALTHY);
    expect(budget.tier).toBe(2);
    feedIntervals(budget, clock, intervals(RECOVER_WINDOWS + 2), HEALTHY);
    expect(budget.tier).toBe(1);
  });

  it("死区窗口打断降级连击但不清升级进度", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, 1);

    // 攒 10 个好窗口, 跨过驻留期。
    feedIntervals(budget, clock, intervals(10), HEALTHY);
    // 死区窗口既不算好也不算坏。
    feedIntervals(budget, clock, intervals(1), DEAD_ZONE);
    expect(budget.tier).toBe(1);
    expect(budget.snapshot().ratio).toBeGreaterThan(1.1);
    expect(budget.snapshot().ratio).toBeLessThan(1.45);
    // 再补两个好窗口就够 12 个, 说明死区没有清掉升级进度。
    feedIntervals(budget, clock, intervals(2), HEALTHY);
    expect(budget.tier).toBe(0);

    // 死区同样打断降级连击: 两个坏窗口加死区再加两个坏窗口不降级。
    feedIntervals(budget, clock, intervals(3), HEALTHY);
    feedIntervals(budget, clock, intervals(2), OVERLOAD);
    feedIntervals(budget, clock, intervals(1), DEAD_ZONE);
    feedIntervals(budget, clock, intervals(2), OVERLOAD);
    expect(budget.tier).toBe(0);
    // 不被打断的三个坏窗口就会降级。
    feedIntervals(budget, clock, intervals(DEGRADE_WINDOWS), OVERLOAD);
    expect(budget.tier).toBe(1);
  });

  it("暂停后的超长间隔不计入判定", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    clock.now += MAX_INTERVAL_MS * 4;
    budget.recordPaint(clock.now);
    feedIntervals(budget, clock, intervals(6), HEALTHY);
    expect(budget.snapshot().ratio).toBe(1);
    expect(budget.tier).toBe(0);
  });

  it("静帧档试探间隔随连续失败翻倍并封顶", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, STILL_TIER);
    expect(budget.stillRetryMs).toBe(STILL_RETRY_MS);

    let expected = STILL_RETRY_MS;
    for (let retry = 0; retry < 4; retry += 1) {
      budget.attemptStillRecovery(clock.now);
      expected = Math.min(MAX_STILL_RETRY_MS, expected * 2);
      expect(budget.stillRetryMs).toBe(expected);
      degradeTo(budget, clock, STILL_TIER);
    }
    expect(budget.stillRetryMs).toBe(MAX_STILL_RETRY_MS);

    // 升回满速说明机器缓过来了, 试探间隔复位。
    recoverTo(budget, clock, 0);
    expect(budget.stillRetryMs).toBe(STILL_RETRY_MS);
  });

  it("升级后很快又降级会收紧升级门槛", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, 1);
    recoverTo(budget, clock, 0);
    expect(budget.snapshot().flapFactor).toBe(1);

    // FLAP_MS 内再次降级, 门槛翻倍。
    degradeTo(budget, clock, 1);
    expect(budget.snapshot().flapFactor).toBe(2);

    // 翻倍后需要 24 个好窗口, 一轮 14 个不够。
    feedIntervals(budget, clock, intervals(RECOVER_WINDOWS + 2), HEALTHY);
    expect(budget.tier).toBe(1);
    feedIntervals(budget, clock, intervals(RECOVER_WINDOWS + 2), HEALTHY);
    expect(budget.tier).toBe(0);
  });

  it("满速稳定足够久后清掉抖动退避", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    degradeTo(budget, clock, 1);
    recoverTo(budget, clock, 0);
    degradeTo(budget, clock, 1);
    expect(budget.snapshot().flapFactor).toBe(2);

    recoverTo(budget, clock, 0);
    // 停在满速超过 FLAP_MS 后门槛回到初始值。
    feedIntervals(
      budget,
      clock,
      Math.ceil(FLAP_MS / HEALTHY) + WINDOW_SAMPLES,
      HEALTHY,
    );
    expect(budget.snapshot().flapFactor).toBe(1);
  });

  it("抖动退避封顶在四倍", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    for (let round = 0; round < 3; round += 1) {
      degradeTo(budget, clock, 1);
      recoverTo(budget, clock, 0);
    }
    expect(budget.snapshot().flapFactor).toBe(4);
  });

  it("帧率极低时窗口按时长而非样本数关闭", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    // 12 个样本远不足 90 个 (三个窗口), 但时长已越过三个 1 秒窗口。
    feedIntervals(budget, clock, 12, CRAWL);
    expect(budget.tier).toBeGreaterThan(0);
    expect(budget.snapshot().degraded).toBeGreaterThan(0);
    expect(budget.snapshot().ratio).toBeGreaterThan(1.45);
  });

  it("样本与时长都不足时不判定, 满样本即关闭", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    warmup(budget, clock);
    feedIntervals(budget, clock, WINDOW_SAMPLES - 1, 5);
    expect(budget.snapshot().ratio).toBe(1);
    expect(budget.snapshot().degraded).toBe(0);
    expect(budget.tier).toBe(0);
    // 第 30 个样本立即关闭窗口, 比值随之更新。
    feedIntervals(budget, clock, 1, 5);
    expect(budget.snapshot().ratio).toBeLessThan(1);
  });

  it("reset 清空统计, 只在换过档时通知", () => {
    const { clock, time } = makeClock();
    const changes: Tier[] = [];
    const budget = createRenderBudget({
      clock: time,
      onChange: (tier) => changes.push(tier),
    });
    warmup(budget, clock);
    degradeTo(budget, clock, 1);
    expect(changes).toEqual([1]);

    budget.reset(clock.now);
    expect(changes).toEqual([1, 0]);
    expect(budget.snapshot()).toMatchObject({
      tier: 0,
      cadence: 1,
      ratio: 1,
      baseline: 0,
      degraded: 0,
      recovered: 0,
      flapFactor: 1,
      stillRetryMs: STILL_RETRY_MS,
    });
    expect(budget.warming).toBe(true);

    // 满速下的 reset 不产生多余通知。
    budget.reset(clock.now);
    expect(changes).toEqual([1, 0]);
  });

  it("名义帧间隔作为基线, 从启动就过载也能降级", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    // 源名义 20ms, 但机器从第一帧就只跑到 30ms: 自测基线也是 30ms,
    // 比值恒为 1。用名义间隔当锚点才能看出这是过载。
    budget.setNominalInterval(BASE);
    prime(budget, clock);
    feedIntervals(budget, clock, Math.ceil(WARMUP_MS / OVERLOAD) + 1, OVERLOAD);
    expect(budget.warming).toBe(false);
    expect(budget.snapshot().baseline).toBe(BASE);
    degradeTo(budget, clock, 1);
    expect(budget.snapshot().ratio).toBeCloseTo(OVERLOAD / BASE, 2);
  });

  it("拿不到名义间隔时退回自测中位数", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    prime(budget, clock);
    feedIntervals(budget, clock, Math.ceil(WARMUP_MS / BASE) + 1, BASE);
    expect(budget.warming).toBe(false);
    expect(budget.snapshot().baseline).toBe(BASE);
    expect(budget.snapshot().ratio).toBe(1);
  });

  it("非法名义间隔被忽略, 判定开始后调整立即生效", () => {
    const { clock, time } = makeClock();
    const budget = createRenderBudget({ clock: time });
    for (const bad of [0, -1, Number.NaN, Number.POSITIVE_INFINITY])
      budget.setNominalInterval(bad);
    expect(budget.snapshot().baseline).toBe(0);

    budget.setNominalInterval(BASE);
    prime(budget, clock);
    feedIntervals(budget, clock, Math.ceil(WARMUP_MS / BASE) + 1, BASE);
    expect(budget.snapshot().baseline).toBe(BASE);

    // 判定开始后把锚点放大一倍, 基线立刻跟着变。
    const doubled = BASE * 2;
    budget.setNominalInterval(doubled);
    feedIntervals(budget, clock, WINDOW_SAMPLES, BASE);
    expect(budget.snapshot().baseline).toBe(doubled);
    // 相对新锚点只有一半间隔, 属于判好, 不会降级。
    expect(budget.tier).toBe(0);
  });

  it("默认时钟取 performance.now 且行为一致", () => {
    const step = 100;
    let value = 0;
    const now = vi
      .spyOn(performance, "now")
      .mockImplementation(() => (value += step));
    const budget = createRenderBudget();
    expect(budget.warming).toBe(true);
    expect(budget.recordPaint()).toBe(true);
    for (let i = 0; i < Math.ceil(WARMUP_MS / step) + WARMUP_SAMPLES; i += 1)
      budget.recordPaint();
    expect(budget.warming).toBe(false);
    expect(budget.snapshot().baseline).toBe(step);
    // 节奏稳定在同一个间隔, 不应降级。
    for (let i = 0; i < intervals(DEGRADE_WINDOWS); i += 1)
      budget.recordPaint();
    expect(budget.tier).toBe(0);
    now.mockRestore();
  });
});
