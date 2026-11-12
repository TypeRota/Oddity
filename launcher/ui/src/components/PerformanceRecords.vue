<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import BaseButton from "./BaseButton.vue";
import BaseModal from "./BaseModal.vue";
import BaseSelect from "./BaseSelect.vue";
import BaseSwitch from "./BaseSwitch.vue";
import SettingGroup from "./settings/SettingGroup.vue";
import SettingRow from "./settings/SettingRow.vue";
import { invoke, isDesktop } from "../core/api";
import { listen } from "../core/desktop";
import {
  migrationBusy,
  runtimeEnvironmentBusy,
  state,
  openSettings,
} from "../core/controller";
import { save } from "../core/filePicker";
import { gameSettings } from "../core/game";
import type { PerformanceOverview, PerformanceRecord } from "../types/launcher";

const overview = ref<PerformanceOverview>({
  recordingEnabled: false,
  records: [],
  recording: null,
});
const a = ref("");
const b = ref("");
const error = ref("");
const deleteError = ref("");
const report = ref("");
const reporting = ref(false);
const saved = ref("");
const originalSize = ref(false);
const switching = ref(false);
const pinning = ref("");
const removing = ref(false);
const pending = ref<PerformanceRecord | null>(null);
const records = computed(() => overview.value.records);
// 后端只在启动中、运行环境任务进行中或会话存活时拒绝固定和删除.
const locked = computed(
  () =>
    state.game !== "idle" ||
    migrationBusy.value ||
    runtimeEnvironmentBusy.value,
);
// 采样需要当前运行环境; 没有环境时启动不会产生记录.
const hasEnvironment = computed(() =>
  Boolean(gameSettings(state.bootstrap?.settings).activeEnvironment),
);
const recordOptions = computed(() =>
  records.value.map((record) => ({
    value: record.id,
    label: `${record.environment.name} · ${formatTime(record.startedAt)} · ${record.metrics.fps.toFixed(1)} FPS`,
  })),
);
const recordA = computed(() =>
  records.value.find((record) => record.id === a.value),
);
const recordB = computed(() =>
  records.value.find((record) => record.id === b.value),
);
const canCompare = computed(
  () => records.value.length >= 2 && a.value !== b.value,
);
const reportUrl = computed(() =>
  report.value
    ? "data:image/svg+xml;charset=utf-8," + encodeURIComponent(report.value)
    : "",
);
const pendingLabel = computed(() =>
  pending.value
    ? `${pending.value.environment.name} · ${formatTime(pending.value.startedAt)}`
    : "",
);
interface ComparisonRow {
  label: string;
  a: string;
  b: string;
  delta: number | null;
  // 帧时间越低越好, 其余指标越高越好.
  better: "up" | "down" | null;
}
const rows = computed<ComparisonRow[]>(() => {
  const base = recordA.value;
  const target = recordB.value;
  if (!base || !target || base.id === target.id) return [];
  const percent = (from: number, to: number) =>
    from > 0 ? ((to - from) / from) * 100 : null;
  return [
    {
      label: "环境",
      a: base.environment.name,
      b: target.environment.name,
      delta: null,
      better: null,
    },
    {
      label: "记录时间",
      a: formatTime(base.startedAt),
      b: formatTime(target.startedAt),
      delta: null,
      better: null,
    },
    {
      label: "时长",
      a: formatDuration(base.durationSeconds),
      b: formatDuration(target.durationSeconds),
      delta: null,
      better: null,
    },
    {
      label: "平均 FPS",
      a: base.metrics.fps.toFixed(1),
      b: target.metrics.fps.toFixed(1),
      delta: percent(base.metrics.fps, target.metrics.fps),
      better: "up",
    },
    {
      label: "1% Low",
      a: base.metrics.low1.toFixed(1),
      b: target.metrics.low1.toFixed(1),
      delta: percent(base.metrics.low1, target.metrics.low1),
      better: "up",
    },
    {
      label: "P99 帧时间",
      a: `${base.metrics.p99.toFixed(1)} ms`,
      b: `${target.metrics.p99.toFixed(1)} ms`,
      delta: percent(base.metrics.p99, target.metrics.p99),
      better: "down",
    },
  ];
});

// 时间精确到分钟; 跨年的记录补上年份, 否则选项会重名.
function formatTime(ms: number) {
  const date = new Date(ms);
  return date.toLocaleString("zh-CN", {
    ...(date.getFullYear() === new Date().getFullYear()
      ? {}
      : { year: "numeric" }),
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}
function formatDuration(seconds: number) {
  const minutes = Math.max(0, Math.round(seconds / 60));
  if (minutes < 1) return "不足 1 分钟";
  if (minutes < 60) return `${minutes} 分钟`;
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分钟`;
}
// 列表里的固定和删除按钮同名, 用环境名与时间补出可区分的可访问名称.
function recordLabel(record: PerformanceRecord, action?: string) {
  const name = `${record.environment.name} ${formatTime(record.startedAt)}`;
  return action ? `${action} ${name} 的记录` : name;
}
function formatDelta(delta: number) {
  return `${delta < 0 ? "-" : "+"}${Math.abs(delta).toFixed(1)}%`;
}
function deltaClass(row: ComparisonRow) {
  if (row.delta === null || row.delta === 0) return "";
  const improved = row.better === "down" ? row.delta < 0 : row.delta > 0;
  return improved ? "is-better" : "is-worse";
}
const message = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);

let disposed = false;
let generation = 0;
let reportRun = 0;
let unlisten: (() => void) | undefined;

// 选中的记录被删除后只重置失效的一侧, 另一侧保持用户的选择.
function selectDefault() {
  const known = (id: string) =>
    records.value.some((record) => record.id === id);
  if (known(a.value) && known(b.value)) return;
  if (!known(a.value)) a.value = pickAway(b.value, 1);
  if (!known(b.value)) b.value = pickAway(a.value, 0);
  report.value = "";
}

// 回到最近的记录: 先按偏移取一条, 与另一侧重复时再往后找.
function pickAway(taken: string, prefer: number) {
  const first = records.value[prefer];
  if (first && first.id !== taken) return first.id;
  return records.value.find((record) => record.id !== taken)?.id ?? "";
}

async function loadReport() {
  const run = ++reportRun;
  saved.value = "";
  if (!canCompare.value) {
    report.value = "";
    reporting.value = false;
    return;
  }
  reporting.value = true;
  report.value = "";
  await invoke("performance_report", { a: a.value, b: b.value }).then(
    (svg) => {
      if (!disposed && run === reportRun) report.value = svg;
    },
    (reason) => {
      if (!disposed && run === reportRun) error.value = message(reason);
    },
  );
  if (!disposed && run === reportRun) reporting.value = false;
}

async function reload() {
  if (!state.bootstrap) return;
  const run = ++generation;
  await invoke("performance_overview").then(
    (result) => {
      if (disposed || run !== generation) return;
      overview.value = result;
      selectDefault();
      error.value = "";
    },
    (reason) => {
      if (!disposed && run === generation) error.value = message(reason);
    },
  );
  if (disposed || run !== generation) return;
  await loadReport();
}

async function toggleRecording(enabled: boolean) {
  if (switching.value) return;
  switching.value = true;
  error.value = "";
  await invoke("performance_recording", { enabled }).then(
    reload,
    async (reason) => {
      // 请求失败时以存储里的开关状态为准, 让开关回到实际值.
      await reload();
      error.value = message(reason);
    },
  );
  switching.value = false;
}

function selectSide(side: "a" | "b", id: string) {
  if (side === "a") a.value = id;
  else b.value = id;
  void loadReport();
}

async function togglePin(record: PerformanceRecord) {
  if (locked.value || pinning.value || removing.value) return;
  pinning.value = record.id;
  error.value = "";
  await invoke("performance_pin", {
    id: record.id,
    pinned: !record.pinned,
  }).then(reload, (reason) => {
    error.value = message(reason);
  });
  pinning.value = "";
}

function askDelete(record: PerformanceRecord) {
  if (locked.value || removing.value) return;
  deleteError.value = "";
  pending.value = record;
}

async function confirmDelete() {
  const record = pending.value;
  if (!record || removing.value) return;
  removing.value = true;
  deleteError.value = "";
  await invoke("performance_delete", { id: record.id }).then(
    async () => {
      pending.value = null;
      report.value = "";
      await reload();
    },
    (reason) => {
      deleteError.value = message(reason);
    },
  );
  removing.value = false;
}

async function exportPng() {
  if (!canCompare.value || !reportUrl.value) return;
  if (!isDesktop()) {
    error.value = "导出 PNG 需要在桌面应用中使用.";
    return;
  }
  const destination = await save({
    title: "导出性能记录对比图",
    defaultPath: "oddity-performance.png",
    filters: [{ name: "PNG 图片", extensions: ["png"] }],
  });
  if (!destination) return;
  error.value = "";
  await invoke("performance_export", {
    a: a.value,
    b: b.value,
    destination,
  }).then(
    (path) => {
      saved.value = path;
    },
    (reason) => {
      error.value = message(reason);
    },
  );
}

watch(
  () => Boolean(state.bootstrap),
  (ready) => {
    if (ready) void reload();
  },
  { immediate: true },
);
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
  reportRun++;
  unlisten?.();
});
</script>

<template>
  <div class="performance-records">
    <SettingGroup
      title="性能记录"
      description="开启后每次启动游戏都会保存一条记录, 可以挑选任意两条对比."
    >
      <template #action
        ><BaseButton variant="ghost" @click="openSettings('runtime')"
          >管理运行环境</BaseButton
        ></template
      >
      <SettingRow
        for="performance-recording"
        label="记录每次启动"
        description="需要系统中已安装 MangoHud; 关闭时不产生记录."
        ><template #default="{ describedby }"
          ><BaseSwitch
            id="performance-recording"
            v-model="overview.recordingEnabled"
            :describedby="describedby"
            :disabled="switching"
            @update:model-value="toggleRecording" /></template
      ></SettingRow>
      <p v-if="!hasEnvironment" role="status">
        请先在运行环境里选择一个环境, 否则启动不会产生记录.
      </p>
      <p v-if="overview.recording" role="status">
        正在记录本次启动, 退出游戏后, 采样有效的记录会出现在列表里.
      </p>
      <p v-if="!isDesktop()" role="status">
        浏览器预览使用示例数据, 采样与导出仅在桌面应用中使用.
      </p>
    </SettingGroup>
    <SettingGroup v-if="records.length" title="记录对比">
      <p v-if="records.length < 2">再完成一次游戏后可比较.</p>
      <template v-else>
        <SettingRow label="基准记录 A" description="对比的基准.">
          <BaseSelect
            :model-value="a"
            label="基准记录 A"
            :options="recordOptions"
            @update:model-value="selectSide('a', $event)"
          />
        </SettingRow>
        <SettingRow label="对比记录 B" description="差值为 B - A.">
          <BaseSelect
            :model-value="b"
            label="对比记录 B"
            :options="recordOptions"
            @update:model-value="selectSide('b', $event)"
          />
        </SettingRow>
        <p v-if="!canCompare" role="alert">请选择两条不同的记录.</p>
        <table v-else class="performance-records__summary">
          <thead>
            <tr>
              <th scope="col">指标</th>
              <th scope="col">基准 A</th>
              <th scope="col">对比 B</th>
              <th scope="col">差值</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.label">
              <th scope="row">{{ row.label }}</th>
              <td>{{ row.a }}</td>
              <td>{{ row.b }}</td>
              <td :class="deltaClass(row)">
                {{ row.delta === null ? "" : formatDelta(row.delta) }}
              </td>
            </tr>
          </tbody>
        </table>
        <div class="performance-records__actions" data-pad-actions>
          <BaseButton
            variant="ghost"
            :disabled="!canCompare || reporting"
            @click="loadReport"
            >刷新报告</BaseButton
          >
          <BaseButton :disabled="!canCompare || !reportUrl" @click="exportPng"
            >导出 PNG</BaseButton
          >
          <BaseButton
            v-if="reportUrl"
            variant="ghost"
            @click="originalSize = !originalSize"
            >{{ originalSize ? "适应宽度" : "原始尺寸" }}</BaseButton
          >
        </div>
        <p v-if="saved" role="status">已保存: {{ saved }}</p>
        <p v-if="reporting" role="status">正在生成对比图表.</p>
        <div
          v-else-if="reportUrl"
          class="performance-records__viewport"
          :class="{ 'is-original': originalSize }"
          tabindex="0"
          role="img"
          aria-label="性能记录对比图表"
        >
          <img
            :src="reportUrl"
            alt="两条性能记录的平均帧率、1% Low、帧时间分布和时间线对比"
            class="performance-records__report"
          />
        </div>
      </template>
    </SettingGroup>
    <SettingGroup
      title="记录列表"
      description="每个游戏保留最近 10 条未固定记录, 固定记录不受限制."
    >
      <p v-if="!records.length">开启记录后完成一次游戏即可产生第一条记录.</p>
      <article
        v-for="record in records"
        :key="record.id"
        class="performance-records__record"
        :class="{ 'is-pinned': record.pinned }"
        :aria-label="recordLabel(record)"
      >
        <div class="performance-records__record-head">
          <strong>{{ record.environment.name }}</strong>
          <span v-if="record.pinned" class="performance-records__pin"
            >已固定</span
          >
        </div>
        <p class="performance-records__meta">
          {{ record.environment.proton }} · {{ formatTime(record.startedAt) }} ·
          {{ formatDuration(record.durationSeconds) }}
        </p>
        <p class="performance-records__meta">
          平均 {{ record.metrics.fps.toFixed(1) }} FPS · 1% Low
          {{ record.metrics.low1.toFixed(1) }} FPS
        </p>
        <p
          v-for="warning in record.warnings"
          :key="warning"
          class="performance-records__meta is-warning"
        >
          {{ warning }}
        </p>
        <div class="performance-records__actions" data-pad-actions>
          <BaseButton
            variant="ghost"
            size="sm"
            :busy="pinning === record.id"
            :disabled="locked"
            :aria-label="
              recordLabel(record, record.pinned ? '取消固定' : '固定')
            "
            @click="togglePin(record)"
            >{{ record.pinned ? "取消固定" : "固定" }}</BaseButton
          >
          <BaseButton
            variant="ghost"
            size="sm"
            :disabled="locked || removing"
            :aria-label="recordLabel(record, '删除')"
            @click="askDelete(record)"
            >删除</BaseButton
          >
        </div>
      </article>
    </SettingGroup>
    <p v-if="error" role="alert">{{ error }}</p>
    <BaseModal
      :open="pending !== null"
      title="删除性能记录"
      @update:open="
        (value) => {
          if (!value && !removing) pending = null;
        }
      "
    >
      <p>确定删除 {{ pendingLabel }}? 此操作无法撤销.</p>
      <p class="performance-records__meta">原始采样文件会一并删除.</p>
      <p v-if="deleteError" role="alert">{{ deleteError }}</p>
      <div class="performance-records__actions">
        <BaseButton variant="danger" :busy="removing" @click="confirmDelete"
          >确定删除</BaseButton
        >
        <BaseButton variant="ghost" :disabled="removing" @click="pending = null"
          >取消</BaseButton
        >
      </div>
    </BaseModal>
  </div>
</template>

<style scoped>
.performance-records {
  display: grid;
  gap: 24px;
}
.performance-records__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
.performance-records__summary {
  width: 100%;
  margin-top: 12px;
  border-collapse: collapse;
  color: var(--bone);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}
.performance-records__summary th,
.performance-records__summary td {
  padding: 8px 12px 8px 0;
  border-bottom: 1px solid var(--line);
  text-align: left;
  white-space: nowrap;
}
.performance-records__summary thead th {
  color: var(--muted);
  font-weight: var(--weight-medium);
}
.performance-records__summary tbody th {
  color: var(--muted);
  font-weight: var(--weight-medium);
}
.performance-records__summary .is-better {
  color: var(--accent-text);
  font-weight: var(--weight-medium);
}
.performance-records__summary .is-worse {
  color: var(--danger);
  font-weight: var(--weight-medium);
}
.performance-records__viewport {
  overflow: auto;
  max-height: 720px;
}
.performance-records__viewport.is-original .performance-records__report {
  width: 1440px;
  max-width: none;
}
.performance-records__report {
  display: block;
  width: 100%;
  height: auto;
}
.performance-records__record {
  padding-block: 14px;
  border-bottom: 1px solid var(--line);
}
.performance-records__record:last-child {
  border-bottom: 0;
}
.performance-records__record-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.performance-records__record-head strong {
  color: var(--bone);
  font-weight: var(--weight-medium);
}
.performance-records__pin {
  padding: 2px 8px;
  border-radius: var(--r-control);
  background: var(--ghost-bg);
  color: var(--accent-text);
  font-size: var(--text-xs);
}
.performance-records__meta {
  margin: 4px 0 0;
  color: var(--muted);
  font-size: var(--text-sm);
}
.performance-records__meta.is-warning {
  color: var(--warn);
}
</style>
