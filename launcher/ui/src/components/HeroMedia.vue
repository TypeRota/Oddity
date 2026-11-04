<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useReducedMotion } from "motion-v";
import { useDocumentVisibility } from "@vueuse/core";
import {
  applyHeroMedia,
  heroPlaybackSuspended,
  media,
  setHeroRenderTier,
  setHeroVideo,
  state,
  form,
  ui,
} from "../core/controller";
import {
  createRenderBudget,
  STILL_TIER,
  type Tier,
} from "../core/renderBudget";
import WallpaperFade from "./WallpaperFade.vue";

defineProps<{ wallpaper: string }>();

const videoA = ref<HTMLVideoElement | null>(null);
const videoB = ref<HTMLVideoElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const ready = ref(false);
const reduced = useReducedMotion();
const visibility = useDocumentVisibility();
const videoEnabled = computed(
  () =>
    state.heroMedia === "video" &&
    !form.reduceMotion &&
    !reduced.value &&
    visibility.value === "visible" &&
    ui.windowVisible,
);
const active = ref(0);
const playing = computed(
  () => videoEnabled.value && state.page === "home" && media.looping,
);
// 预算采样的生命周期比播放更长: 静帧档已经停止播放, 仍要按间隔试探恢复。
const budgeting = computed(() => videoEnabled.value && state.page === "home");
const currentVideo = computed(() =>
  active.value === 0 ? videoA.value : videoB.value,
);
const standbyVideo = computed(() =>
  active.value === 0 ? videoB.value : videoA.value,
);
const artStyle = computed(() =>
  media.image ? { backgroundImage: `url("${media.image}")` } : undefined,
);
let context: CanvasRenderingContext2D | null = null;
let frame = 0;
let painting: HTMLVideoElement | null = null;
let resize: ResizeObserver | undefined;
let cssWidth = 0;
let cssHeight = 0;
let switching = false;
let resumeTime = 0;
// 静帧档需要定时器试探恢复, 其余档位没有任何常驻回调。
let retryTimer = 0;
// 名义帧间隔的增量下限: 连续播放区间太短时算出的帧率噪声很大。
const NOMINAL_MIN_FRAMES = 15;
const NOMINAL_MIN_TIME = 0.5;
// 名义帧间隔的采样锚点: 记录同一段连续播放的起点, 用增量推帧率。
let nominalAnchor: {
  source: HTMLVideoElement;
  time: number;
  frames: number;
} | null = null;
// 绘制节奏由实测帧间隔自动分档。
const budget = createRenderBudget({ onChange: handleTierChange });

function handleTierChange(tier: Tier) {
  setHeroRenderTier(tier);
  syncStillRetry();
}

// 两个 video 交替播放, 在结尾前切到已经预载到首帧的备播元素。
// 单元素 loop 会在回跳时等待解码器 seek, 看起来像少了几帧。
function switchLoop() {
  if (switching) return;
  const outgoing = currentVideo.value;
  const incoming = standbyVideo.value;
  if (!outgoing || !incoming) return;
  switching = true;
  try {
    if (incoming.readyState < 2) {
      try {
        outgoing.currentTime = 0;
      } catch {}
      const retry = outgoing.play();
      if (retry && typeof retry.catch === "function") retry.catch(() => {});
      return;
    }
    incoming.pause();
    try {
      incoming.currentTime = 0;
    } catch {}
    const attempt = incoming.play();
    if (attempt && typeof attempt.catch === "function") attempt.catch(() => {});
    active.value = active.value === 0 ? 1 : 0;
    setHeroVideo(incoming);
    outgoing.pause();
    try {
      outgoing.currentTime = 0;
    } catch {}
  } finally {
    switching = false;
  }
}

function drawFrame(
  source: HTMLVideoElement,
  target: HTMLCanvasElement,
  context: CanvasRenderingContext2D,
) {
  // 画布分辨率不超过源画面, 避免高 DPI 下的无效放大。
  const pixelRatio = Math.min(
    window.devicePixelRatio,
    source.videoWidth / cssWidth,
    source.videoHeight / cssHeight,
  );
  const width = Math.round(cssWidth * pixelRatio);
  const height = Math.round(cssHeight * pixelRatio);
  if (width <= 0 || height <= 0) return;
  if (target.width !== width || target.height !== height) {
    target.width = width;
    target.height = height;
  }
  // 直接按源画面比例裁切, 不经过原生视频层的 object-fit 缩放。
  const scale = Math.max(
    width / source.videoWidth,
    height / source.videoHeight,
  );
  const cropWidth = width / scale;
  const cropHeight = height / scale;
  try {
    context.drawImage(
      source,
      (source.videoWidth - cropWidth) * media.cropX,
      (source.videoHeight - cropHeight) * media.cropY,
      cropWidth,
      cropHeight,
      0,
      0,
      width,
      height,
    );
  } catch (error) {
    if (error instanceof DOMException && error.name === "InvalidStateError")
      return;
    throw error;
  }
  ready.value = true;
}

function paint(now: number) {
  if (painting) frame = painting.requestVideoFrameCallback(paint);
  const source = currentVideo.value;
  const target = canvas.value;
  // 回跳与缓冲期间保留上一帧, 不暴露解码器的空画面。
  if (
    source &&
    target &&
    context &&
    !source.paused &&
    !source.seeking &&
    source.readyState >= 2 &&
    source.videoWidth &&
    source.videoHeight
  ) {
    if (budget.warming) syncNominalInterval(source);
    // 预算决定这一帧画不画: 首帧必画, 之后按当前档位的节奏跳帧。
    if (budget.recordPaint(now)) drawFrame(source, target, context);
    // 循环切换与是否绘制解耦: 官方素材的 loopCut 只有 0.02 秒,
    // 低档位下按绘制帧检查会错过切换点。
    if (
      Number.isFinite(source.duration) &&
      source.duration > media.loopCut &&
      source.currentTime >= source.duration - media.loopCut
    )
      switchLoop();
  }
}

/// 用同一段连续播放的时间与解码帧数增量推出源的名义帧间隔, 作为预算的固定锚点。
/// 累计值在恢复播放位置或换元素后跨越了不同的播放区间, 只有增量才反映源帧率。
function syncNominalInterval(source: HTMLVideoElement) {
  if (typeof source.getVideoPlaybackQuality !== "function") return;
  const frames = source.getVideoPlaybackQuality().totalVideoFrames;
  const time = source.currentTime;
  const anchor = nominalAnchor;
  // 换元素或时间、帧数回退都说明区间断了, 从当前位置重新建立锚点。
  if (
    !anchor ||
    anchor.source !== source ||
    time < anchor.time ||
    frames < anchor.frames
  ) {
    nominalAnchor = { source, time, frames };
    return;
  }
  const deltaFrames = frames - anchor.frames;
  const deltaTime = time - anchor.time;
  if (deltaFrames < NOMINAL_MIN_FRAMES || deltaTime < NOMINAL_MIN_TIME) return;
  budget.setNominalInterval((deltaTime / deltaFrames) * 1000);
  nominalAnchor = { source, time, frames };
}

/// 丢弃旧锚点: 播放位置或素材变了, 上一段区间不再可比。
function resetBudget() {
  nominalAnchor = null;
  budget.reset();
}

// 静帧档没有视频帧到达, 只能定时试探: 退回低速档重新实测, 由下一轮判定
// 决定继续升级还是退回静帧。间隔随连续失败翻倍, 持续过载的设备不反复绘制。
function stopStillRetry() {
  if (!retryTimer) return;
  window.clearTimeout(retryTimer);
  retryTimer = 0;
}

function syncStillRetry() {
  stopStillRetry();
  if (!budgeting.value || budget.tier !== STILL_TIER) return;
  retryTimer = window.setTimeout(() => {
    retryTimer = 0;
    if (!budgeting.value) return;
    budget.attemptStillRecovery();
    syncStillRetry();
  }, budget.stillRetryMs);
}

function loadVideos() {
  for (const video of [videoA.value, videoB.value]) {
    if (!video) continue;
    video.pause();
    video.load();
  }
  active.value = 0;
  setHeroVideo(videoA.value);
}

function syncPainting() {
  if (painting) painting.cancelVideoFrameCallback(frame);
  if (playing.value && !context && canvas.value)
    context = canvas.value.getContext("2d", { alpha: false });
  painting = playing.value && context ? currentVideo.value : null;
  if (painting) frame = painting.requestVideoFrameCallback(paint);
}
// 离页、隐藏或游戏/失焦暂停后保留播放位置, 重新解码时从同一帧续播;
// 只有主页因隐藏、静音等普通原因卸载时才从头开始。
watch(
  videoEnabled,
  (enabled) => {
    if (!enabled)
      resumeTime =
        state.page === "home" && !heroPlaybackSuspended()
          ? 0
          : (currentVideo.value?.currentTime ?? 0);
  },
  { flush: "sync" },
);
function restorePosition(event: Event) {
  const video = event.currentTarget as HTMLVideoElement;
  if (video === currentVideo.value && resumeTime > 0) {
    video.currentTime = resumeTime;
    resumeTime = 0;
    // 播放位置被搬走, 锚点里的区间不再连续。
    nominalAnchor = null;
  }
}
watch(playing, syncPainting, { flush: "sync" });
watch(currentVideo, syncPainting, { flush: "post" });
// 离开主页、窗口隐藏或恢复可见都要重新预热: 上一段的基线与场景不再可比。
watch(
  budgeting,
  () => {
    resetBudget();
    syncStillRetry();
  },
  { flush: "sync" },
);
// 只有换素材时才丢弃最后一帧, 页面切换要保留画面直接续播。
watch(
  () => media.videoKey,
  () => {
    ready.value = false;
    if (canvas.value) {
      canvas.value.width = 0;
      canvas.value.height = 0;
    }
    resumeTime = 0;
    active.value = 0;
    // 新素材的解码节奏与旧的没有可比性, 基线要重新预热。
    resetBudget();
    syncStillRetry();
    setHeroVideo(videoA.value);
  },
  { flush: "sync" },
);
watch(
  [() => media.url, videoEnabled],
  () => {
    loadVideos();
    applyHeroMedia(state.heroMedia);
  },
  { flush: "post" },
);
onMounted(() => {
  const target = canvas.value!;
  resize = new ResizeObserver(([entry]) => {
    if (!entry) return;
    cssWidth = entry.contentRect.width;
    cssHeight = entry.contentRect.height;
  });
  resize.observe(target);
  setHeroVideo(videoA.value);
  loadVideos();
  syncPainting();
});
onBeforeUnmount(() => {
  if (painting) painting.cancelVideoFrameCallback(frame);
  stopStillRetry();
  resize?.disconnect();
  // 先摘掉解码器引用再清档位, 避免卸载途中请求播放。
  setHeroVideo(null);
  setHeroRenderTier(0);
});
</script>

<template>
  <div
    class="hero__art"
    :class="{
      'hero__art--video': state.heroMedia === 'video',
      'hero__art--official': media.officialImage,
      'hero__art--pending': !media.initialized,
    }"
    :style="artStyle"
    aria-hidden="true"
  >
    <WallpaperFade
      :src="wallpaper"
      :position="media.officialImage ? 'center' : '54% 38%'"
    />
  </div>
  <video
    id="hero-video"
    ref="videoA"
    class="hero__source"
    :src="videoEnabled ? media.url || undefined : undefined"
    :preload="videoEnabled && media.url ? 'auto' : 'none'"
    :autoplay="playing && active === 0"
    crossorigin="anonymous"
    muted
    @loadedmetadata="restorePosition"
    @ended="switchLoop"
    playsinline
    hidden
    aria-hidden="true"
  ></video>
  <video
    id="hero-video-b"
    ref="videoB"
    class="hero__source"
    :src="videoEnabled ? media.url || undefined : undefined"
    :preload="videoEnabled && media.url ? 'auto' : 'none'"
    :autoplay="playing && active === 1"
    crossorigin="anonymous"
    muted
    @loadedmetadata="restorePosition"
    @ended="switchLoop"
    playsinline
    hidden
    aria-hidden="true"
  ></video>
  <canvas
    id="hero-canvas"
    ref="canvas"
    width="0"
    height="0"
    class="hero__video"
    :class="{
      'is-visible':
        state.heroMedia === 'video' && ready && !form.reduceMotion && !reduced,
    }"
    aria-hidden="true"
  ></canvas>
</template>
