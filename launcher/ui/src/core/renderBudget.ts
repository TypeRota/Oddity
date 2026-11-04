// 主视觉视频的渲染预算: 按实测帧间隔自动分档, 过载时先降绘制频率, 最后退到静帧。
// 只调节绘制节奏, 画布分辨率保持源画面到后备尺寸的 1:1 纹理直拷。
//
// 唯一的输入是视频帧到达间隔 (rVFC 在主线程上投递, 间隔同时反映绘制与整机主线程负载)。
// 不常驻 rAF 探测: 常驻回调本身就与本模块要降低的开销相冲突。

export type Tier = 0 | 1 | 2 | 3;

/// 静帧档: 停掉绘制并暂停解码, 画布保留最后一帧。
export const STILL_TIER: Tier = 3;

// 各档的绘制间隔: 每 N 个视频帧画一帧, 0 表示停掉绘制并暂停解码。
const CADENCE: readonly number[] = [1, 2, 3, 0];

// 预热期只采基线: 首帧与素材加载期的间隔不可比, 不能拿来判定。
const WARMUP_MS = 2000;
const WARMUP_SAMPLES = 8;
// 一个判定窗口取 30 个视频帧或 1 秒, 先到为准。30fps 下两者相当。
const WINDOW_SAMPLES = 30;
const WINDOW_MS = 1000;
// 快降慢升: 降级 3 个窗口内响应, 升级要 12 个窗口且一次只升一档。
const DEGRADE_WINDOWS = 3;
const RECOVER_WINDOWS = 12;
// 判好与判坏之间留死区, 避免在阈值附近来回抖动。
const BAD_RATIO = 1.45;
const GOOD_RATIO = 1.1;
// 换档后至少驻留这么久, 给新档位留出可比的采样窗口。
const MIN_DWELL_MS = 5000;
// 静帧档观感变化最大, 试探恢复的起始间隔更长, 反复失败时翻倍退避。
const STILL_RETRY_MS = 20000;
const MAX_STILL_RETRY_MS = 120000;
// 升级后这段时间内又降级, 说明档位判断在抖, 升级门槛翻倍。
const FLAP_MS = 30000;
const MAX_FLAP_FACTOR = 4;
// 超过这个长度的间隔来自暂停或离页, 不是帧率问题, 直接丢弃。
const MAX_INTERVAL_MS = 500;

export interface RenderBudgetSnapshot {
  tier: Tier;
  cadence: number;
  ratio: number;
  baseline: number;
  degraded: number;
  recovered: number;
  flapFactor: number;
  stillRetryMs: number;
}

export interface RenderBudgetOptions {
  /// 时钟可注入, 便于测试按帧推进。
  clock?: () => number;
  /// 档位变化回调, 由调用方接回响应式状态与解码器控制。
  onChange?: (tier: Tier) => void;
}

export interface RenderBudget {
  reset(now?: number): void;
  /// 锚点: 源的名义帧间隔 (毫秒), 由调用方从解码计数推出。
  /// 基线与设备性能无关, 低性能设备从第一帧起就能判定过载。
  setNominalInterval(ms: number): void;
  /// 记录一个到达的视频帧, 返回这一帧要不要绘制。
  recordPaint(now?: number): boolean;
  /// 静帧档没有视频帧到达, 由调用方定时试探: 退回低速档重新实测。
  attemptStillRecovery(now?: number): void;
  get tier(): Tier;
  get cadence(): number;
  get warming(): boolean;
  get stillRetryMs(): number;
  snapshot(): RenderBudgetSnapshot;
}

function median(values: number[]) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[sorted.length >> 1];
}

export function createRenderBudget(
  options: RenderBudgetOptions = {},
): RenderBudget {
  const clock = options.clock ?? (() => performance.now());
  const onChange = options.onChange;

  let tier: Tier = 0;
  let tierSince = clock();
  let startedAt = tierSince;
  // -1 表示还没升级过, 抖动退避只在真的升级之后才可能触发。
  let lastUpgrade = -1;
  let flapFactor = 1;
  let degraded = 0;
  let recovered = 0;
  let stillRetryMs = STILL_RETRY_MS;
  let seen = 0;

  let warm = true;
  let warmSamples: number[] = [];
  let nominal = 0;
  let baseline = 0;
  let windowIntervals: number[] = [];
  let windowStartedAt = tierSince;
  let ratio = 1;
  let badWindows = 0;
  let goodWindows = 0;
  let lastPaintAt = 0;

  function publish(next: Tier) {
    if (next === tier) return;
    tier = next;
    onChange?.(tier);
  }

  function upgrade(now: number, next: Tier) {
    publish(next);
    tierSince = now;
    lastUpgrade = now;
    goodWindows = 0;
    recovered += 1;
    // 一路升回满速说明机器确实缓过来了, 试探间隔复位。
    if (next === 0) stillRetryMs = STILL_RETRY_MS;
  }

  function degrade(now: number) {
    if (tier === STILL_TIER) {
      badWindows = 0;
      return;
    }
    // 驻留期内先不动, 坏窗口计数保留, 下一次判定再试。
    if (now - tierSince < MIN_DWELL_MS) return;
    // 刚升级又立刻降级, 说明这一档判断在抖, 收紧后续升级门槛。
    if (lastUpgrade >= 0 && now - lastUpgrade <= FLAP_MS)
      flapFactor = Math.min(MAX_FLAP_FACTOR, flapFactor * 2);
    const next = (tier + 1) as Tier;
    publish(next);
    tierSince = now;
    badWindows = 0;
    degraded += 1;
  }

  function recover(now: number) {
    if (tier === 0) {
      // 稳定在满速一段时间后清掉抖动退避。
      if (now - tierSince >= FLAP_MS) flapFactor = 1;
      goodWindows = 0;
      return;
    }
    if (now - tierSince < MIN_DWELL_MS) return;
    if (goodWindows < RECOVER_WINDOWS * flapFactor) return;
    upgrade(now, (tier - 1) as Tier);
  }

  function decide(now: number) {
    // 视频帧的到达节奏由解码与合成决定, 与画不画无关, 各档共用同一基线。
    ratio =
      windowIntervals.length && baseline
        ? median(windowIntervals) / baseline
        : 1;
    if (ratio >= BAD_RATIO) {
      goodWindows = 0;
      badWindows += 1;
      if (badWindows >= DEGRADE_WINDOWS) degrade(now);
      return;
    }
    // 死区里的窗口打断降级连击, 但不清升级进度, 免得噪声把档位卡死。
    if (ratio > GOOD_RATIO) {
      badWindows = 0;
      return;
    }
    badWindows = 0;
    goodWindows += 1;
    recover(now);
  }

  function closeWindow(now: number) {
    if (
      windowIntervals.length < WINDOW_SAMPLES &&
      now - windowStartedAt < WINDOW_MS
    )
      return;
    decide(now);
    windowIntervals = [];
    windowStartedAt = now;
  }

  function finishWarmup(now: number) {
    warm = false;
    // 名义帧间隔优先; 拿不到时退回自测中位数, 总比完全不判定强。
    baseline = nominal || median(warmSamples);
    warmSamples = [];
    windowStartedAt = now;
  }

  function record(interval: number, now: number) {
    // 暂停或离页后的第一帧间隔是整段静止时长, 不能当成过载。
    if (interval > MAX_INTERVAL_MS) return;
    if (warm) {
      warmSamples.push(interval);
      // 时长与样本数两个条件都要满足, 缺一个就继续采。
      if (now - startedAt >= WARMUP_MS && warmSamples.length >= WARMUP_SAMPLES)
        finishWarmup(now);
      return;
    }
    windowIntervals.push(interval);
    closeWindow(now);
  }

  return {
    reset(now = clock()) {
      const previous = tier;
      tier = 0;
      tierSince = now;
      startedAt = now;
      windowStartedAt = now;
      lastUpgrade = -1;
      flapFactor = 1;
      degraded = 0;
      recovered = 0;
      stillRetryMs = STILL_RETRY_MS;
      seen = 0;
      warm = true;
      warmSamples = [];
      nominal = 0;
      baseline = 0;
      windowIntervals = [];
      ratio = 1;
      badWindows = 0;
      goodWindows = 0;
      lastPaintAt = 0;
      // 只有真的换过档才需要通知, 免得预热重置产生多余的播放请求。
      if (previous !== 0) onChange?.(tier);
    },
    setNominalInterval(ms: number) {
      if (!Number.isFinite(ms) || ms <= 0) return;
      nominal = ms;
      // 预热还没结束就只记下锚点; 已经在判定时立即改用新基线。
      if (!warm) baseline = ms;
    },
    recordPaint(now = clock()) {
      if (lastPaintAt) record(now - lastPaintAt, now);
      lastPaintAt = now;
      seen += 1;
      // 首帧必画: ready 只在成功绘制后置位, 跳过首帧画布就永远不会显示。
      if (seen === 1) return true;
      const cadence = CADENCE[tier];
      return cadence > 0 && seen % cadence === 0;
    },
    attemptStillRecovery(now = clock()) {
      if (tier !== STILL_TIER) return;
      // 试探性恢复只退一档: 重新拿到视频帧后由实测间隔决定继续升还是退回。
      // 连续试探失败就把间隔翻倍, 避免在持续过载的机器上反复全速绘制。
      stillRetryMs = Math.min(MAX_STILL_RETRY_MS, stillRetryMs * 2);
      windowIntervals = [];
      windowStartedAt = now;
      upgrade(now, (STILL_TIER - 1) as Tier);
    },
    get tier() {
      return tier;
    },
    get cadence() {
      return CADENCE[tier];
    },
    get warming() {
      return warm;
    },
    get stillRetryMs() {
      return stillRetryMs;
    },
    snapshot() {
      return {
        tier,
        cadence: CADENCE[tier],
        ratio: Number(ratio.toFixed(3)),
        baseline: Number(baseline.toFixed(2)),
        degraded,
        recovered,
        flapFactor,
        stillRetryMs,
      };
    },
  };
}
