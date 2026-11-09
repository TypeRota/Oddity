<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import BaseButton from "./BaseButton.vue";
import BaseInput from "./BaseInput.vue";
import BaseSelect from "./BaseSelect.vue";
import BaseDisclosure from "./BaseDisclosure.vue";
import SettingGroup from "./settings/SettingGroup.vue";
import SettingRow from "./settings/SettingRow.vue";
import { invoke, isDesktop } from "../core/api";
import { listen } from "../core/desktop";
import {
  state,
  openSettings,
  runtimeEnvironmentAction,
  runtimeEnvironmentLocked,
} from "../core/controller";
import { activeGame, gameSettings } from "../core/game";
import { save } from "../core/filePicker";
import type { PerformanceOverview } from "../types/launcher";

const overview = ref<PerformanceOverview>({
  active: null,
  recording: null,
  comparisons: [],
});
const selected = ref("");
const a = ref("");
const b = ref("");
const name = ref("");
const error = ref("");
const busy = ref(false);
const report = ref("");
const originalSize = ref(false);
const saved = ref("");
const environments = computed(() =>
  (state.bootstrap?.settings.runtimeEnvironments ?? []).filter(
    (e) => e.game === activeGame(state.bootstrap?.settings),
  ),
);
const environmentOptions = computed(() =>
  environments.value.map((e) => ({ value: e.id, label: e.name })),
);
const comparisonOptions = computed(() =>
  overview.value.comparisons.map((c) => ({
    value: c.id,
    label: c.name + (c.id === overview.value.active ? " · 已启用" : ""),
  })),
);
const comparison = computed(() =>
  overview.value.comparisons.find((c) => c.id === selected.value),
);
const reportUrl = computed(() =>
  report.value
    ? "data:image/svg+xml;charset=utf-8," + encodeURIComponent(report.value)
    : "",
);
const locked = computed(
  () => state.game !== "idle" || busy.value || runtimeEnvironmentLocked.value,
);
const runningEnvironment = computed(
  () => gameSettings(state.bootstrap?.settings).activeEnvironment,
);
const samplingSide = computed(() => {
  const c = overview.value.comparisons.find(
    (c) => c.id === overview.value.active,
  );
  if (!c) return "已暂停采样";
  const side =
    c.a.id === runningEnvironment.value
      ? "A"
      : c.b.id === runningEnvironment.value
        ? "B"
        : null;
  if (overview.value.recording === c.id)
    return `正在采样 ${side ?? "所选"} 环境, 游戏退出后自动生成报告.`;
  return side
    ? `下次启动将采样 ${side} 环境; 游戏退出后自动生成报告.`
    : "当前环境不属于启用的比较, 本次不会采样.";
});
let disposed = false;
let generation = 0;
let unlisten: (() => void) | undefined;
const message = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);

async function reload() {
  if (!state.bootstrap) return;
  const run = ++generation;
  await invoke("performance_overview").then(
    (result) => {
      if (disposed || run !== generation) return;
      overview.value = result;
      if (!result.comparisons.some((c) => c.id === selected.value))
        selected.value = result.active ?? result.comparisons[0]?.id ?? "";
      error.value = "";
    },
    (reason) => {
      if (!disposed && run === generation) error.value = message(reason);
    },
  );
  if (disposed || run !== generation) return;
  if (comparison.value?.sessions.length || comparison.value?.error) {
    await invoke("performance_report", { id: selected.value }).then(
      (svg) => {
        if (!disposed && run === generation) report.value = svg;
      },
      (reason) => {
        if (!disposed && run === generation) error.value = message(reason);
      },
    );
  } else report.value = "";
}

async function create() {
  busy.value = true;
  await invoke("performance_create", {
    name: name.value,
    a: a.value,
    b: b.value,
  }).then(
    async () => {
      selected.value = "";
      name.value = "";
      await reload();
    },
    (reason) => {
      error.value = message(reason);
    },
  );
  busy.value = false;
}
async function activate() {
  busy.value = true;
  await invoke("performance_activate", {
    id: overview.value.active === selected.value ? null : selected.value,
  }).then(reload, (reason) => {
    error.value = message(reason);
  });
  busy.value = false;
}
async function exportPng() {
  const id = selected.value;
  const destination = await save({
    title: "导出性能比较图",
    defaultPath: `oddity-performance-${id}.png`,
    filters: [{ name: "PNG 图片", extensions: ["png"] }],
  });
  if (!destination) return;
  busy.value = true;
  await invoke("performance_export", { id, destination }).then(
    (path) => {
      saved.value = path;
    },
    (reason) => {
      error.value = message(reason);
    },
  );
  busy.value = false;
}
watch(
  () =>
    state.bootstrap
      ? `${activeGame(state.bootstrap.settings)}:${environments.value.map((e) => e.id).join(",")}`
      : "",
  () => {
    selected.value = "";
    report.value = "";
    const stable = gameSettings(state.bootstrap?.settings).stableEnvironment;
    a.value =
      environments.value.find((e) => e.id === stable)?.id ??
      environments.value[0]?.id ??
      "";
    b.value = environments.value.find((e) => e.id !== a.value)?.id ?? "";
    void reload();
  },
  { immediate: true },
);
watch(selected, () => {
  report.value = "";
  saved.value = "";
  void reload();
});
watch(
  () => state.game,
  () => {
    void reload();
  },
);
if (isDesktop())
  void listen("performance-updated", () => {
    void reload();
  }).then((stop) => {
    if (disposed) stop();
    else unlisten = stop;
  });
onBeforeUnmount(() => {
  disposed = true;
  generation++;
  unlisten?.();
});
</script>

<template>
  <div class="performance-comparison">
    <SettingGroup
      title="性能比较"
      description="选择两个运行环境, 正常游玩后自动比较并生成本地图表."
    >
      <template #action
        ><BaseButton variant="ghost" @click="openSettings('runtime')"
          >管理运行环境</BaseButton
        ></template
      >
      <p v-if="!isDesktop()" role="status">性能采样需要在桌面应用中使用.</p>
      <SettingRow label="比较名称"
        ><BaseInput
          v-model="name"
          aria-label="比较名称"
          placeholder="例如: Proton 版本比较"
          :maxlength="80"
          :disabled="locked"
      /></SettingRow>
      <SettingRow label="A 环境"
        ><BaseSelect
          v-model="a"
          aria-label="A 环境"
          :options="environmentOptions"
          :disabled="locked"
      /></SettingRow>
      <SettingRow label="B 环境"
        ><BaseSelect
          v-model="b"
          aria-label="B 环境"
          :options="environmentOptions"
          :disabled="locked"
      /></SettingRow>
      <SettingRow
        data-pad-actions
        label="创建并启用"
        description="需要安装 MangoHud. 按当前选择的环境启动游戏, 退出后自动保存报告."
        ><BaseButton
          :busy="busy"
          :disabled="
            locked || !name.trim() || !a || !b || a === b || !isDesktop()
          "
          @click="create"
          >创建比较</BaseButton
        ></SettingRow
      >
    </SettingGroup>
    <SettingGroup v-if="overview.comparisons.length" title="比较记录">
      <SettingRow label="查看比较"
        ><BaseSelect
          v-model="selected"
          aria-label="查看比较"
          :options="comparisonOptions"
          :disabled="busy"
      /></SettingRow>
      <SettingRow data-pad-actions label="自动采样" :description="samplingSide"
        ><BaseButton :disabled="locked || !selected" @click="activate">{{
          overview.active === selected ? "暂停采样" : "启用此比较"
        }}</BaseButton></SettingRow
      >
      <template v-if="comparison">
        <p v-if="comparison.error" role="alert">{{ comparison.error }}</p>
        <div class="performance-comparison__actions" data-pad-actions>
          <BaseButton
            variant="ghost"
            :disabled="locked || runningEnvironment === comparison.a.id"
            @click="runtimeEnvironmentAction('switch', comparison.a.id)"
            >切换到 A 环境</BaseButton
          ><BaseButton
            variant="ghost"
            :disabled="locked || runningEnvironment === comparison.b.id"
            @click="runtimeEnvironmentAction('switch', comparison.b.id)"
            >切换到 B 环境</BaseButton
          >
        </div>
        <p role="status">
          A: {{ comparison.a.name }} ({{
            comparison.sessions.filter((s) => s.side === "A").length
          }}
          次) · B: {{ comparison.b.name }} ({{
            comparison.sessions.filter((s) => s.side === "B").length
          }}
          次)
        </p>
        <div class="performance-comparison__actions" data-pad-actions>
          <BaseButton variant="ghost" :disabled="busy" @click="reload"
            >刷新报告</BaseButton
          ><BaseButton :disabled="busy || !reportUrl" @click="exportPng"
            >导出 PNG</BaseButton
          >
        </div>
        <p v-if="saved" role="status">已保存: {{ saved }}</p>
        <template v-if="reportUrl">
          <BaseButton variant="ghost" @click="originalSize = !originalSize">{{
            originalSize ? "适应宽度" : "原始尺寸"
          }}</BaseButton>
          <div
            class="performance-comparison__viewport"
            tabindex="0"
            aria-label="性能比较图表"
            :class="{ 'is-original': originalSize }"
          >
            <img
              :src="reportUrl"
              alt="A/B 性能指标差值、帧时间分布和会话时间线"
              class="performance-comparison__report"
            />
          </div>
        </template>
        <p v-else>等待游戏采样. 两侧都有记录后自动生成比较结果.</p>
        <BaseDisclosure v-if="comparison.sessions.length" title="每次会话">
          <article
            v-for="session in [...comparison.sessions].reverse()"
            :key="session.id"
            class="performance-comparison__session"
          >
            <strong
              >{{ session.side }} ·
              {{ new Date(session.startedAt).toLocaleString() }}</strong
            >
            <p>
              {{ Math.round(session.durationSeconds) }} 秒 · 平均
              {{ session.metrics.fps.toFixed(1) }} FPS · 1% Low
              {{ session.metrics.low1.toFixed(1) }} FPS · P99
              {{ session.metrics.p99.toFixed(1) }} ms
            </p>
            <p v-for="warning in session.warnings" :key="warning">
              {{ warning }}
            </p>
            <p>
              游戏版本: {{ session.facts.gameVersion ?? "未知" }} · 显示模式:
              {{ session.facts.displayBackend ?? "未知" }}
            </p>
            <BaseDisclosure title="采样配置">
              <pre>{{ JSON.stringify(session.facts, null, 2) }}</pre>
            </BaseDisclosure>
          </article>
        </BaseDisclosure>
      </template>
    </SettingGroup>
    <p v-if="error" role="alert">{{ error }}</p>
  </div>
</template>

<style scoped>
.performance-comparison {
  display: grid;
  gap: 24px;
}
.performance-comparison__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
.performance-comparison__viewport {
  overflow: auto;
  max-height: 720px;
}
.performance-comparison__viewport.is-original .performance-comparison__report {
  width: 1440px;
  max-width: none;
}
.performance-comparison__report {
  display: block;
  width: 100%;
  height: auto;
}
.performance-comparison__session {
  padding-block: 12px;
  border-bottom: 1px solid var(--line);
}
pre {
  overflow: auto;
  max-height: 280px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
