<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import canvasConfetti from "canvas-confetti";
import { motion, AnimatePresence } from "motion-v";
import { createTimeline } from "animejs";
import { invoke, isDesktop } from "../core/api";
import { listen } from "../core/desktop";
import {
  formatSize,
  openSettings,
  form,
  applyBootstrap,
  runtimeEnvironmentBusy,
  runtimeEnvironmentLocked,
  runtimeSetupAutomatic,
  showPage,
  state,
} from "../core/controller";
import type {
  RuntimeSetupInfo,
  RuntimeSetupPlan,
  RuntimeSetupStatus,
} from "../types/launcher";
import { useControlMotion } from "../composables/useControlMotion";
import BaseButton from "../components/BaseButton.vue";
import BaseSelect from "../components/BaseSelect.vue";
import BaseSwitch from "../components/BaseSwitch.vue";
import DirectoryField from "../components/DirectoryField.vue";
import BaseView from "../components/BaseView.vue";
import BaseModal from "../components/BaseModal.vue";
import SettingRow from "../components/settings/SettingRow.vue";

const { surface, reduced } = useControlMotion();
const active = computed(() => state.page === "runtime-setup");
const info = ref<RuntimeSetupInfo | null>(null);
const plan = ref<RuntimeSetupPlan | null>(null);
const status = ref<RuntimeSetupStatus | null>(null);
const mode = ref("auto");
const proton = ref("");
const allUsers = ref(false);
const busy = ref(false);
const planning = ref(false);
const error = ref("");
const log = ref("");
const editing = ref(false);
const celebration = ref(false);
const confetti = ref<HTMLCanvasElement | null>(null);
let cannon: ReturnType<typeof canvasConfetti.create> | undefined;
let timer: ReturnType<typeof setInterval> | undefined;
let burst: ReturnType<typeof createTimeline> | undefined;
let celebrated = false;
let stopEvents: (() => void) | undefined;
const transitions = new Map<Element, ReturnType<typeof createTimeline>>();
const options = [
  { label: "自动配置", value: "auto" },
  { label: "使用本地 Proton", value: "manual" },
];
const locked = computed(
  () => busy.value || planning.value || runtimeEnvironmentLocked.value,
);
const complete = computed(
  () => status.value?.done && !error.value && !busy.value,
);
const component = computed(() => status.value?.progress.component ?? "umu");
const scene = computed(() =>
  editing.value
    ? "options"
    : error.value
      ? "error"
      : busy.value || planning.value
        ? component.value
        : complete.value
          ? "complete"
          : !info.value
            ? "checking"
            : runtimeSetupAutomatic.value
              ? "umu"
              : info.value.environment
                ? "existing"
                : "ready",
);
let previousScene = scene.value;
watch(
  scene,
  (_, previous) => {
    previousScene = previous;
  },
  { flush: "sync" },
);
const stages = ["umu", "proton", "runtime"] as const;
const labels = { umu: "umu", proton: "Proton", runtime: "运行时" };
const heading = computed(
  () =>
    ({
      ready: "启动环境向导",
      checking: "检查启动环境",
      existing: "已有可用运行环境",
      options: "安装选项",
      umu: "准备 umu",
      proton: "准备 Proton",
      runtime: "准备运行时",
      error: "配置未完成",
      complete: "启动环境已就绪",
    })[scene.value],
);
const descriptions = {
  ready: "自动准备游戏所需的启动环境.",
  checking: "正在检测已安装的组件和环境",
  existing: "可以继续创建独立环境.",
  options: "",
  umu: "正在安装并检查启动组件",
  proton: "正在准备游戏兼容环境",
  runtime: "正在准备运行时并初始化环境",
  error: "",
  complete: "已添加至 Proton 运行环境.",
};
function cancelTransition(element: Element) {
  transitions.get(element)?.cancel();
  transitions.delete(element);
}
function moveScene(element: Element, done: () => void, entering: boolean) {
  cancelTransition(element);
  const timeline = createTimeline({
    onComplete: () => {
      transitions.delete(element);
      done();
      if (!entering) return;
      if (scene.value === "complete" && !celebrated && !reduced.value) {
        celebrated = true;
        celebration.value = true;
        if (confetti.value) {
          cannon ??= canvasConfetti.create(confetti.value, {
            resize: true,
            disableForReducedMotion: true,
          });
          const fire = (count: number, velocity: number) => {
            const options = {
              particleCount: count,
              spread: 72,
              startVelocity: velocity,
              gravity: 1.05,
              decay: 0.92,
              ticks: 210,
              scalar: 0.9,
              flat: true,
              colors: ["#c7ed40", "#ffb84d", "#72d5ea", "#ea7b8a", "#f6f7fa"],
              disableForReducedMotion: true,
            };
            void cannon!({
              ...options,
              angle: 62,
              origin: { x: 0.08, y: 0.82 },
            });
            void cannon!({
              ...options,
              angle: 118,
              origin: { x: 0.92, y: 0.82 },
            })?.then(() => {
              celebration.value = false;
            });
          };
          burst = createTimeline()
            .call(() => fire(48, 46), 0)
            .call(() => fire(28, 34), 180);
        }
      }
    },
  });
  transitions.set(element, timeline);
  // 两层占据同一固定区域, 同时交叉淡化, 不等待旧画面消失.
  const opacity = Number(getComputedStyle(element).opacity);
  const filter = getComputedStyle(element).filter;
  if (!entering) {
    (element as HTMLElement).inert = true;
    element.setAttribute("aria-hidden", "true");
    element.querySelector("h2")?.removeAttribute("id");
  }
  const sharedTerminal =
    ["checking", "ready", "umu"].includes(previousScene) &&
    ["checking", "ready", "umu"].includes(scene.value);
  const optionsTransition =
    previousScene === "options" || scene.value === "options";
  const blur =
    reduced.value || sharedTerminal
      ? "blur(0px)"
      : optionsTransition
        ? "blur(6px)"
        : "blur(8px)";
  timeline.add(
    element,
    {
      opacity: entering ? [0, 1] : [opacity, 0],
      filter: entering ? [blur, "blur(0px)"] : [filter, blur],
      duration: reduced.value
        ? 100
        : optionsTransition
          ? 160
          : sharedTerminal
            ? 180
            : 240,
      ease: "inOut(3)",
    },
    0,
  );
}
watch(
  active,
  async (value) => {
    if (!value) {
      burst?.cancel();
      cannon?.reset();
      celebration.value = false;
      return;
    }
    if (!busy.value && !planning.value) {
      status.value = null;
      error.value = "";
      editing.value = false;
      info.value = null;
    }
    await refresh();
    if (!busy.value && !planning.value)
      allUsers.value = !!info.value?.systemUmu;
  },
  { immediate: true },
);
watch([allUsers, mode, proton], () => {
  plan.value = null;
  status.value = null;
  error.value = "";
});
function failed(cause: unknown) {
  error.value = cause instanceof Error ? cause.message : String(cause);
}
function refresh() {
  return invoke("runtime_setup_info")
    .then((value) => {
      info.value = value;
    })
    .catch(failed);
}
watch([active, info, runtimeEnvironmentLocked, runtimeSetupAutomatic], () => {
  if (
    !active.value ||
    !info.value ||
    !runtimeSetupAutomatic.value ||
    locked.value
  )
    return;
  runtimeSetupAutomatic.value = false;
  prepare();
});
function prepare() {
  if (locked.value) return;
  editing.value = false;
  celebration.value = false;
  celebrated = false;
  planning.value = true;
  error.value = "";
  status.value = null;
  void invoke("runtime_setup_plan", {
    allUsers: !!info.value?.systemUmu || allUsers.value,
    createEnvironment: !!info.value?.environment,
    proton: mode.value === "manual" ? proton.value.trim() : null,
  })
    .then((value) => {
      log.value = value.log;
      if (value.systemCommand) plan.value = value;
      else apply();
    })
    .catch(failed)
    .finally(() => {
      planning.value = false;
    });
}
function apply() {
  if (busy.value) return;
  plan.value = null;
  busy.value = true;
  runtimeEnvironmentBusy.value = true;
  error.value = "";
  status.value = null;
  timer = setInterval(() => {
    void invoke("runtime_setup_status")
      .then((value) => {
        status.value = value;
      })
      .catch(failed);
  }, 500);
  void invoke("runtime_setup_apply")
    .then(async () => {
      status.value = await invoke("runtime_setup_status");
      await refresh();
      applyBootstrap(await invoke("bootstrap"), false);
    })
    .catch(failed)
    .finally(() => {
      clearInterval(timer);
      busy.value = false;
      runtimeEnvironmentBusy.value = false;
      plan.value = null;
    });
}
function finish() {
  void invoke("runtime_setup_dismiss")
    .then(() => (complete.value ? openSettings("runtime") : showPage("home")))
    .catch(failed);
}
onMounted(() => {
  if (isDesktop())
    void listen<RuntimeSetupStatus>("runtime-setup-state", (event) => {
      status.value = event.payload;
    }).then((stop) => {
      stopEvents = stop;
    });
});
onBeforeUnmount(() => {
  clearInterval(timer);
  stopEvents?.();
  burst?.cancel();
  cannon?.reset();
  transitions.forEach((timeline) => timeline.cancel());
});
</script>

<template>
  <motion.section
    id="page-runtime-setup"
    class="page runtime-setup-page"
    :class="{ 'is-active': active }"
    :inert="!active"
    :aria-hidden="!active || undefined"
    :initial="false"
    :animate="{
      opacity: active ? 1 : 0,
      visibility: active ? 'visible' : 'hidden',
    }"
    :transition="surface"
    aria-labelledby="runtime-setup-title"
    :aria-busy="busy || planning"
  >
    <div class="wizard-shell">
      <header class="wizard-header">
        <span>启动环境向导</span
        ><AnimatePresence
          ><motion.span
            v-if="busy || planning"
            class="wizard-live"
            :initial="{ opacity: 0 }"
            :animate="{ opacity: 1 }"
            :exit="{ opacity: 0 }"
            :transition="surface"
            ><motion.i
              :animate="{ opacity: reduced ? 1 : [0.35, 1, 0.35] }"
              :transition="{ duration: 2, repeat: Infinity }"
            />正在准备</motion.span
          ></AnimatePresence
        >
      </header>
      <nav class="wizard-stages" aria-label="配置阶段">
        <span
          v-for="(stage, index) in stages"
          :key="stage"
          :class="{
            'is-current': scene === stage,
            'is-done':
              complete ||
              scene === 'existing' ||
              stages.indexOf(component) > index,
          }"
          ><svg class="icon" viewBox="0 0 256 256" aria-hidden="true">
            <use
              :href="
                complete ||
                scene === 'existing' ||
                stages.indexOf(component) > index
                  ? '#i-check-circle'
                  : '#i-' + ['terminal-window', 'cpu', 'hard-drives'][index]
              "
            /></svg
          >{{ labels[stage] }}</span
        >
      </nav>
      <div class="wizard-viewport">
        <Transition
          :css="false"
          appear
          @enter="(el, done) => moveScene(el, done, true)"
          @leave="(el, done) => moveScene(el, done, false)"
          @enter-cancelled="cancelTransition"
          @leave-cancelled="cancelTransition"
        >
          <div :key="scene" class="wizard-scene" :data-scene="scene">
            <svg
              v-if="scene !== 'options'"
              class="wizard-art"
              viewBox="0 0 240 180"
              fill="none"
              aria-hidden="true"
            >
              <template
                v-if="
                  scene === 'umu' || scene === 'ready' || scene === 'checking'
                "
              >
                <rect
                  x="40"
                  y="25"
                  width="160"
                  height="125"
                  rx="20"
                  fill="var(--surface-solid)"
                  stroke="currentColor"
                  stroke-width="2"
                />
                <path
                  d="M40 54h160m-141-14h2m10 0h2m10 0h2m-24 38 13 12-13 12"
                  stroke="currentColor"
                  stroke-width="3"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
                <path
                  d="M96 90h55"
                  stroke="var(--signal-strong)"
                  stroke-width="5"
                  stroke-linecap="round"
                />
                <path
                  d="M65 120h112"
                  stroke="currentColor"
                  stroke-width="4"
                  stroke-linecap="round"
                  opacity=".35"
                />
              </template>
              <template v-else-if="scene === 'proton'">
                <rect
                  x="74"
                  y="44"
                  width="92"
                  height="92"
                  rx="20"
                  fill="var(--signal)"
                  stroke="currentColor"
                  stroke-width="2"
                />
                <rect
                  x="99"
                  y="69"
                  width="42"
                  height="42"
                  rx="9"
                  stroke="currentColor"
                  stroke-width="2"
                />
                <path
                  d="M27 70h37m-37 20h37m-37 20h37"
                  stroke="currentColor"
                  stroke-width="5"
                  stroke-linecap="round"
                />
                <path
                  d="M176 70h37m-37 20h37m-37 20h37"
                  stroke="currentColor"
                  stroke-width="5"
                  stroke-linecap="round"
                />
                <path
                  d="M100 17v17m20-17v17m20-17v17m-40 112v17m20-17v17m20-17v17"
                  stroke="var(--signal-strong)"
                  stroke-width="5"
                  stroke-linecap="round"
                />
              </template>
              <template v-else-if="scene === 'runtime'">
                <g v-for="(y, index) in [38, 76, 114]" :key="y">
                  <rect
                    x="46"
                    :y="y"
                    width="148"
                    height="30"
                    rx="9"
                    fill="var(--surface-solid)"
                    stroke="currentColor"
                    stroke-width="2"
                  />
                  <path
                    :d="`M64 ${y + 15}h${60 + index * 15}`"
                    stroke="var(--signal-strong)"
                    stroke-width="4"
                    stroke-linecap="round"
                  />
                  <circle cx="177" :cy="y + 15" r="3" fill="currentColor" />
                </g>
              </template>
              <template v-else>
                <circle cx="120" cy="90" r="54" fill="var(--signal)" />
                <path
                  v-if="scene === 'complete' || scene === 'existing'"
                  d="m94 90 18 18 36-37"
                  stroke="var(--signal-strong)"
                  stroke-width="6"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
                <path
                  v-else
                  d="M120 60v37m0 19v2"
                  stroke="var(--danger)"
                  stroke-width="6"
                  stroke-linecap="round"
                />
              </template>
            </svg>
            <h2 id="runtime-setup-title" class="wizard-copy" tabindex="-1">
              {{ heading }}
            </h2>
            <p
              v-if="scene !== 'options' && scene !== 'error'"
              class="wizard-copy wizard-description"
            >
              {{ descriptions[scene] }}
            </p>
            <div v-if="scene === 'options'" class="wizard-copy wizard-options">
              <div class="field">
                <label for="runtime-setup-mode">配置方式</label
                ><BaseSelect
                  id="runtime-setup-mode"
                  v-model="mode"
                  :options="options"
                  :disabled="locked"
                  label="配置方式"
                />
              </div>
              <BaseView :view-key="mode"
                ><div class="wizard-option-details">
                  <DirectoryField
                    v-if="mode === 'manual'"
                    id="runtime-setup-proton"
                    v-model="proton"
                    label="Proton 目录"
                    :disabled="locked"
                  /><SettingRow
                    label="为全部用户安装 umu"
                    :description="
                      info?.systemUmu
                        ? '系统 umu 已安装, 将直接复用.'
                        : allUsers
                          ? '将安装系统 umu, 需要管理员授权.'
                          : info?.umu
                            ? '复用当前用户的 umu.'
                            : '仅为当前用户安装 umu.'
                    "
                    ><BaseSwitch
                      :model-value="!!info?.systemUmu || allUsers"
                      @update:model-value="allUsers = $event"
                      label="为全部用户安装 umu"
                      :disabled="locked || !!info?.systemUmu"
                  /></SettingRow></div
              ></BaseView>
            </div>
            <div
              v-else-if="busy || planning"
              class="wizard-copy wizard-progress"
              role="status"
            >
              <p>
                {{
                  status?.progress.phase === "download"
                    ? "正在下载"
                    : status?.progress.phase === "extract"
                      ? "正在解压"
                      : "正在检查与初始化"
                }}<span v-if="status?.progress.downloaded"
                  >{{ formatSize(status.progress.downloaded)
                  }}<template v-if="status.progress.total">
                    / {{ formatSize(status.progress.total) }}</template
                  ></span
                >
              </p>
              <div class="wizard-track">
                <motion.span
                  v-if="status?.progress.total"
                  :initial="false"
                  :animate="{
                    width: `${Math.min(100, (status.progress.downloaded / status.progress.total) * 100)}%`,
                  }"
                  :transition="surface"
                /><motion.span
                  v-else
                  class="wizard-indeterminate"
                  :animate="{ x: reduced ? '100%' : ['-100%', '330%'] }"
                  :transition="{
                    duration: 1.9,
                    repeat: Infinity,
                    ease: 'easeInOut',
                  }"
                />
              </div>
            </div>
            <p
              v-if="scene === 'existing'"
              class="wizard-copy wizard-existing"
              :title="info?.environment?.proton.path"
            >
              <strong>{{ info?.environment?.name }}</strong
              ><span>{{ info?.environment?.proton.path }}</span>
            </p>
            <p
              v-if="scene === 'error'"
              class="wizard-copy wizard-error"
              role="alert"
            >
              {{ error }}
            </p>
            <p
              v-if="scene === 'complete'"
              class="wizard-copy wizard-result"
              role="status"
            >
              配置完成.
            </p>
          </div>
        </Transition>
      </div>
      <footer class="wizard-footer">
        <BaseView :view-key="busy || planning ? 'busy' : scene"
          ><div class="wizard-actions">
            <template v-if="scene === 'options'"
              ><BaseButton
                icon="i-download-simple"
                :disabled="locked || (mode === 'manual' && !proton.trim())"
                @click="prepare"
                >开始配置</BaseButton
              ><BaseButton variant="ghost" @click="editing = false"
                >返回</BaseButton
              ></template
            ><template v-else-if="scene === 'existing'"
              ><BaseButton icon="i-plus" :disabled="locked" @click="prepare"
                >继续创建</BaseButton
              ><BaseButton variant="ghost" @click="openSettings('runtime')"
                >查看运行环境</BaseButton
              ><BaseButton variant="ghost" @click="editing = true"
                >安装选项</BaseButton
              ></template
            ><template v-else-if="complete"
              ><BaseButton icon="i-check-circle" @click="finish"
                >查看运行环境</BaseButton
              ><BaseButton variant="ghost" @click="editing = true"
                >安装选项</BaseButton
              ></template
            ><template v-else-if="busy || planning"
              ><BaseButton
                variant="ghost"
                icon="i-folder-open"
                :disabled="!log"
                @click="
                  invoke('open_directory', {
                    path: log.slice(0, log.lastIndexOf('/')),
                  }).catch(failed)
                "
                >查看日志</BaseButton
              ></template
            ><template v-else
              ><BaseButton
                :icon="error ? 'i-arrows-clockwise' : 'i-download-simple'"
                :disabled="locked || !info"
                @click="prepare"
                >{{ error ? "重试" : "自动配置" }}</BaseButton
              ><BaseButton variant="ghost" @click="editing = true"
                >安装选项</BaseButton
              ><BaseButton variant="ghost" @click="finish">稍后</BaseButton
              ><BaseButton
                v-if="error && log"
                variant="ghost"
                icon="i-folder-open"
                @click="
                  invoke('open_directory', {
                    path: log.slice(0, log.lastIndexOf('/')),
                  }).catch(failed)
                "
                >查看日志</BaseButton
              ></template
            >
          </div></BaseView
        >
      </footer>
    </div>
    <Teleport to="body"
      ><canvas
        ref="confetti"
        class="wizard-confetti"
        :data-celebrating="celebration"
        aria-hidden="true"
    /></Teleport>
    <BaseModal
      :open="active && !!plan"
      title="安装系统组件"
      :theme="form.theme"
      :reduce-motion="form.reduceMotion"
      @update:open="plan = null"
      ><div class="wizard-authorization">
        <svg class="icon" viewBox="0 0 256 256" aria-hidden="true">
          <use href="#i-shield-check" />
        </svg>
        <p>将安装 umu-launcher 及其依赖, 需要管理员授权.</p>
      </div>
      <code>{{ ["pkexec", ...(plan?.systemCommand ?? [])].join(" ") }}</code>
      <div class="runtime-environments__actions">
        <BaseButton icon="i-shield-check" @click="apply">授权并安装</BaseButton
        ><BaseButton variant="ghost" @click="plan = null">取消</BaseButton>
      </div></BaseModal
    >
  </motion.section>
</template>
<style scoped>
.runtime-setup-page {
  padding: 36px 40px;
  overflow: hidden;
  scrollbar-width: none;
}
.wizard-shell {
  width: 100%;
  max-width: 620px;
  height: 100%;
  max-height: 600px;
  min-height: 0;
  margin: auto;
  display: grid;
  grid-template-rows: 20px 64px minmax(0, 1fr) 64px;
}
.wizard-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  color: var(--muted);
  font-size: 12px;
  flex: none;
}
.wizard-live {
  display: flex;
  gap: 8px;
  align-items: center;
}
.wizard-live i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--signal-strong);
}
.wizard-stages {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  padding: 22px 0;
  border-bottom: 1px solid var(--line);
  flex: none;
}
.wizard-stages span {
  display: flex;
  gap: 9px;
  align-items: center;
  font-size: 12px;
  color: var(--muted);
}
.wizard-stages .is-current,
.wizard-stages .is-done {
  color: var(--signal-strong);
}
.wizard-viewport {
  position: relative;
  min-height: 0;
  isolation: isolate;
}
.wizard-scene {
  position: absolute;
  inset: 0;
  width: 100%;
  display: grid;
  grid-template-rows: 180px 44px 26px 76px;
  align-content: center;
  place-items: center;
  text-align: center;
  gap: 10px;
}
.wizard-art {
  grid-row: 1;
  width: 240px;
  height: 180px;
  color: var(--bone);
  flex: none;
}
.wizard-scene h2 {
  grid-row: 2;
  margin: 0;
  font-size: clamp(24px, 2.8vw, 34px);
  color: var(--bone);
  letter-spacing: -0.04em;
  line-height: 1.2;
}
.wizard-scene h2:focus {
  outline: none;
}
.wizard-description {
  grid-row: 3;
  margin: 0;
  color: var(--muted);
  font-size: 13px;
}
.wizard-progress {
  grid-row: 4;
  width: min(100%, 380px);
  margin-top: 12px;
}
.wizard-progress p {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--muted);
  margin: 0 0 12px;
}
.wizard-track {
  height: 4px;
  border-radius: 4px;
  background: var(--line);
  overflow: hidden;
}
.wizard-track span {
  display: block;
  height: 100%;
  background: var(--signal-strong);
  border-radius: inherit;
}
.wizard-track .wizard-indeterminate {
  width: 30%;
}
.wizard-options {
  grid-row: 2 / 5;
  width: min(100%, 430px);
  display: flex;
  flex-direction: column;
  gap: 20px;
  text-align: left;
  margin-top: 16px;
}
.wizard-scene[data-scene="options"] {
  grid-template-rows: 44px 250px;
  align-content: center;
}
.wizard-scene[data-scene="options"] h2 {
  grid-row: 1;
}
.wizard-scene[data-scene="options"] .wizard-options {
  grid-row: 2;
  align-self: start;
  margin-top: 0;
}
.wizard-existing {
  grid-row: 4;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
  max-width: 100%;
  margin: 0;
  font-size: 13px;
  color: var(--muted);
}
.wizard-existing span {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.wizard-option-details {
  display: flex;
  flex-direction: column;
  gap: 20px;
}
.wizard-options .setting-row {
  margin: 0;
}
.wizard-footer {
  height: 64px;
  flex: none;
  padding-top: 16px;
  border-top: 1px solid var(--line);
}
.wizard-actions {
  min-height: 42px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  flex-wrap: wrap;
}
.wizard-result {
  grid-row: 4;
  font-size: 12px;
  color: var(--muted);
  margin: 0;
}
.wizard-error {
  grid-row: 4;
  max-width: 100%;
  max-height: 4.8em;
  overflow: hidden;
  overflow-wrap: anywhere;
  font-size: 13px;
  color: var(--danger);
  margin: 0;
}
.wizard-confetti {
  position: fixed;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  z-index: calc(var(--z-modal) + 20);
  overflow: hidden;
}
.wizard-authorization {
  display: flex;
  align-items: center;
  gap: 12px;
}
.wizard-authorization > .icon {
  width: 28px;
  height: 28px;
  color: var(--signal-strong);
  flex: none;
}
@media (max-height: 700px) {
  .runtime-setup-page {
    padding-block: 28px;
  }
  .wizard-stages {
    padding-block: 14px;
  }
  .wizard-art {
    width: 160px;
    height: 120px;
  }
  .wizard-scene {
    grid-template-rows: 120px 36px 22px 64px;
    gap: 8px;
  }
}
@media (max-width: 900px) {
  .runtime-setup-page {
    padding-inline: 28px;
  }
  .wizard-stages {
    gap: 8px;
  }
}
</style>
