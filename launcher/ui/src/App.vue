<script setup lang="ts">
import {
  computed,
  onBeforeUnmount,
  onMounted,
  watch,
  ref,
  nextTick,
  useTemplateRef,
} from "vue";
import { MotionConfig, motion } from "motion-v";
import {
  init,
  ui,
  form,
  state,
  media,
  showPage,
  dispose,
} from "./core/controller";
import { isDemo } from "./core/demo";
import { useSceneMotion, playerExpanded } from "./composables/useSceneMotion";
import { useScenePalette } from "./composables/useScenePalette";
import { navigationScrubbing } from "./composables/navigationGesture";
import NotificationCenter from "./components/NotificationCenter.vue";
import IntegrityDialog from "./components/IntegrityDialog.vue";
import DemoNotice from "./components/DemoNotice.vue";
import TitleBar from "./components/layout/TitleBar.vue";
import HeroMedia from "./components/HeroMedia.vue";
import WallpaperFade from "./components/WallpaperFade.vue";
import HostSlot from "./host/HostSlot.vue";
import { pagesForSlot } from "./host/contributions";
import { factsFrom } from "./host/facts";
import IconSprite from "./components/icons/IconSprite.vue";
import { gamepad, startGamepad } from "./core/gamepad";
import GamepadOverlay from "./components/GamepadOverlay.vue";
import GamepadFilePicker from "./components/GamepadFilePicker.vue";
import GamepadKeyboard from "./components/GamepadKeyboard.vue";
let stopGamepad: (() => void) | undefined;
// 页面贡献按当前游戏过滤; 切换游戏后不可见的页面直接卸载.
const facts = computed(() => factsFrom(state.bootstrap));
const homePages = computed(() => pagesForSlot("home", facts.value));
const drawerPages = computed(() => pagesForSlot("drawer", facts.value));
const scene = useTemplateRef<HTMLElement>("scene");
const drawer = useTemplateRef<HTMLElement>("drawer");
const shade = useTemplateRef<HTMLElement>("shade");
const { opened } = useSceneMotion(scene, drawer, shade);
useScenePalette();
// 二级界面用当前静态主视觉做磨砂底, 不跟随视频帧或页面位移.
const wallpaper = computed(() =>
  media.image
    ? media.image
    : new URL(
        state.heroMedia === "photo"
          ? "assets/hero-keyart.jpg"
          : "assets/hero-poster.jpg",
        document.baseURI,
      ).href,
);
const content = ref<HTMLElement | null>(null);
watch(
  () => state.page,
  async (page, previous) => {
    playerExpanded.value = false;
    const scrubbing = navigationScrubbing.value;
    await nextTick();
    if (scrubbing || navigationScrubbing.value || gamepad.active) return;
    if (page !== "home") {
      content.value
        ?.querySelector<HTMLElement>(".page.is-active h2")
        ?.focus({ preventScroll: true });
    } else if (previous !== "home") {
      document.querySelector<HTMLElement>(`[data-page="${previous}"]`)?.focus();
    }
  },
);
function escape(event: KeyboardEvent) {
  if (event.defaultPrevented || document.querySelector(".modal")) return;
  if (event.key === "Escape" && opened.value) {
    event.preventDefault();
    showPage("home");
  }
}
onMounted(() => {
  void init();
  stopGamepad = startGamepad();
  document.addEventListener("keydown", escape);
});
onBeforeUnmount(() => {
  stopGamepad?.();
  document.removeEventListener("keydown", escape);
  dispose();
});
</script>

<template>
  <IconSprite />
  <MotionConfig :reduced-motion="form.reduceMotion ? 'always' : 'user'">
    <div
      class="app"
      id="app"
      :data-theme="form.theme"
      :class="{
        'is-intro': !ui.introStarted,
        'is-intro-done': ui.introDone,
        'is-maximized': ui.maximized,
        'reduce-motion': form.reduceMotion,
        'has-drawer': opened,
        'gamepad-mode': gamepad.active,
        'is-demo': isDemo,
      }"
    >
      <div class="app__wallpaper" aria-hidden="true">
        <WallpaperFade :src="wallpaper" />
      </div>
      <div ref="scene" class="scene" aria-hidden="true">
        <HeroMedia :wallpaper="wallpaper" />
        <div class="hero__fade"></div>
      </div>
      <HostSlot :contributions="homePages" area="home" />
      <div ref="shade" class="scene-shade" aria-hidden="true"></div>
      <TitleBar />
      <button
        v-if="opened"
        class="drawer-dismiss"
        aria-label="返回主页"
        @click="showPage('home')"
      ></button>
      <div
        ref="drawer"
        class="drawer"
        :inert="!opened"
        :aria-hidden="!opened || undefined"
      >
        <div class="drawer__wallpaper" aria-hidden="true">
          <WallpaperFade :src="wallpaper" />
        </div>
        <div class="drawer__scrim" aria-hidden="true"></div>
        <motion.button
          class="drawer-close"
          type="button"
          aria-label="关闭页面"
          :while-hover="{ scale: 1.08 }"
          :while-press="{ scale: 0.9 }"
          @click="showPage('home')"
        >
          <svg class="icon" aria-hidden="true"><use href="#i-x" /></svg>
        </motion.button>
        <main ref="content" class="main">
          <HostSlot :contributions="drawerPages" area="drawer" />
        </main>
      </div>
      <NotificationCenter />
      <IntegrityDialog />
      <DemoNotice v-if="isDemo" />
      <GamepadFilePicker />
      <GamepadKeyboard />
      <GamepadOverlay />
    </div>
  </MotionConfig>
</template>
