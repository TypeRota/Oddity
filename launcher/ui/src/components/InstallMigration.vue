<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { AnimatePresence, motion, useSpring } from "motion-v";
import { createTimeline } from "animejs";
import BaseButton from "./BaseButton.vue";
import BaseModal from "./BaseModal.vue";
import DirectoryField from "./DirectoryField.vue";
import {
  useControlMotion,
  controlSpring,
} from "../composables/useControlMotion";
import { gameSettings } from "../core/game";
import { channelLabel } from "../host/facts";
import {
  state,
  migration,
  planInstallMigration,
  startInstallMigration,
  cancelInstallMigration,
  abandonInstallMigration,
  cleanupInstallMigration,
  dismissInstallMigration,
  retryInstallMigration,
  refreshMigrationStatus,
  formatSize,
} from "../core/controller";

const props = defineProps<{
  distribution: string;
  disabled?: boolean;
}>();

const { reduced, surface } = useControlMotion();
const open = ref(false);
const target = ref("");
const localError = ref("");
const summary = ref<HTMLElement | null>(null);
const progress = useSpring(0, controlSpring);
let timeline: ReturnType<typeof createTimeline> | undefined;

const installed = computed(
  () =>
    state.bootstrap?.runtimeInstallations.find(
      (entry) => entry.distribution === props.distribution,
    )?.installed ?? false,
);
const channel = computed(() =>
  state.bootstrap?.channels.find((entry) => entry.id === props.distribution),
);
const configured = computed(() => {
  const saved = gameSettings(state.bootstrap?.settings);
  const key = channel.value?.installRootKey;
  return key ? saved.installRoots?.[key] : undefined;
});
const available = computed(() => installed.value || Boolean(configured.value));
const channelName = computed(
  () =>
    channelLabel(
      { channels: state.bootstrap?.channels ?? [] },
      props.distribution,
    ) || props.distribution,
);
const label = computed(() => `迁移${channelName.value}`);
const status = computed(() => migration.status);
const plan = computed(() => migration.plan);
const locked = computed(
  () => Boolean(props.disabled) && status.value?.phase !== "repair",
);
const running = computed(() =>
  [
    "planning",
    "moving",
    "copying",
    "verifying",
    "committing",
    "rebinding",
    "cleanup",
  ].includes(status.value?.phase ?? ""),
);
const busy = computed(() => running.value);
const showForm = computed(
  () => !plan.value && (!status.value || status.value.phase === "idle"),
);
const showPlan = computed(
  () => Boolean(plan.value) && (!status.value || status.value.phase === "idle"),
);
const showProgress = computed(
  () =>
    Boolean(status.value) &&
    [
      "planning",
      "moving",
      "copying",
      "verifying",
      "committing",
      "rebinding",
      "cleanup",
    ].includes(status.value!.phase),
);
const showResult = computed(
  () =>
    Boolean(status.value) &&
    ["done", "failed", "cancelled", "repair"].includes(status.value!.phase),
);
const modeLabel = computed(
  () =>
    ({
      move: "移动",
      copy: "复制",
      repair: "重新绑定",
    })[plan.value?.mode ?? "move"] ?? "",
);
const phaseLabel = computed(() => {
  const map: Record<string, string> = {
    planning: "检查中",
    moving: "移动中",
    copying: "复制中",
    verifying: "校验中",
    committing: "保存设置",
    rebinding: "更新绑定",
    cleanup: "清理中",
    repair: "迁移未完成",
    done: "迁移完成",
    failed: "迁移失败",
    cancelled: "已取消",
  };
  return map[status.value?.phase ?? ""] ?? "";
});
const percent = computed(() => {
  const value = status.value;
  if (!value) return 0;
  if (value.totalBytes > 0)
    return Math.min(100, Math.round((value.bytes / value.totalBytes) * 100));
  if (value.totalFiles > 0)
    return Math.min(100, Math.round((value.files / value.totalFiles) * 100));
  if (value.phase === "done") return 100;
  if (value.phase === "cleanup" && value.totalBytes === 0) return 100;
  return 0;
});
const error = computed(
  () => localError.value || migration.error || status.value?.error || "",
);
const requiresCleanup = computed(
  () => status.value?.phase === "done" && status.value.cleanupPending,
);
const spaceText = computed(() => {
  if (!plan.value) return "";
  if (plan.value.requiredBytes <= 0) return "无需额外空间";
  return (
    "需要 " +
    formatSize(plan.value.requiredBytes) +
    " · 可用 " +
    formatSize(plan.value.availableBytes)
  );
});

watch(percent, (value) => {
  if (reduced.value) progress.jump(value / 100);
  else progress.set(value / 100);
});

watch(
  () => plan.value,
  async (value) => {
    timeline?.cancel();
    if (!value) return;
    await nextTick();
    const items = summary.value?.querySelectorAll<HTMLElement>(
      ".migration-summary__item",
    );
    if (!items?.length || reduced.value) return;
    timeline = createTimeline({ defaults: { ease: "outQuart" } });
    items.forEach((item, index) => {
      timeline!.add(
        item,
        {
          opacity: [0, 1],
          translateY: [8, 0],
          duration: 260,
        },
        index * 40,
      );
    });
  },
);

onBeforeUnmount(() => timeline?.cancel());

async function openDialog() {
  if (locked.value || running.value || !available.value) return;
  open.value = true;
  target.value = "";
  localError.value = "";
  if (
    status.value &&
    (["failed", "cancelled"].includes(status.value.phase) ||
      (status.value.phase === "done" && !status.value.cleanupPending))
  ) {
    await dismissInstallMigration();
  }
  migration.plan = null;
  migration.error = "";
  await refreshMigrationStatus();
}

function closeDialog() {
  if (busy.value) return;
  open.value = false;
  if (status.value?.phase === "done" && !status.value.cleanupPending) {
    void dismissInstallMigration();
  }
}

function updateOpen(value: boolean) {
  if (!value) closeDialog();
}

function back() {
  migration.plan = null;
  migration.error = "";
  localError.value = "";
}

async function check() {
  localError.value = "";
  if (!target.value.trim()) {
    localError.value = "请选择目标目录";
    return;
  }
  await planInstallMigration(props.distribution, target.value);
}

async function start() {
  localError.value = "";
  await startInstallMigration(
    props.distribution,
    plan.value?.target ?? target.value,
  );
}

async function cancel() {
  await cancelInstallMigration();
}

async function cleanup() {
  await cleanupInstallMigration();
}

async function retry() {
  await retryInstallMigration();
}

async function abandon() {
  const ok = await abandonInstallMigration();
  if (!ok || migration.status?.phase !== "idle") return;
  migration.plan = null;
  open.value = false;
}

async function keep() {
  await dismissInstallMigration();
  open.value = false;
}

async function finish() {
  await dismissInstallMigration();
  open.value = false;
}
</script>

<template>
  <BaseButton
    variant="secondary"
    :disabled="locked || busy || !available"
    :aria-label="label"
    @click="openDialog"
    >{{ label }}</BaseButton
  >
  <BaseModal
    :open="open"
    title="迁移安装目录"
    :theme="state.bootstrap?.settings.theme"
    :reduce-motion="state.bootstrap?.settings.reduceMotion"
    @update:open="updateOpen"
  >
    <AnimatePresence :initial="false" mode="wait">
      <motion.div
        v-if="showForm"
        key="form"
        :initial="{ opacity: 0, y: 8 }"
        :animate="{ opacity: 1, y: 0 }"
        :exit="{ opacity: 0, y: -6 }"
        :transition="surface"
      >
        <DirectoryField
          id="set-migration-target"
          v-model="target"
          :label="`${channelName}目标目录`"
          placeholder="/path/to/game"
          @change="localError = ''"
        />
        <p class="field__hint">
          目标目录非空时自动创建子目录. 迁移期间不能启动游戏.
        </p>
        <p v-if="error" role="alert" class="migration-error">{{ error }}</p>
        <div class="modal__foot">
          <BaseButton
            variant="primary"
            :busy="migration.planning"
            :disabled="!target.trim() || busy"
            @click="check"
            >检查目标</BaseButton
          >
          <BaseButton variant="ghost" :disabled="busy" @click="closeDialog"
            >取消</BaseButton
          >
        </div>
      </motion.div>

      <motion.div
        v-else-if="showPlan && plan"
        key="plan"
        :initial="{ opacity: 0, y: 8 }"
        :animate="{ opacity: 1, y: 0 }"
        :exit="{ opacity: 0, y: -6 }"
        :transition="surface"
      >
        <div ref="summary" class="migration-summary">
          <div class="migration-summary__item">
            <span>方式</span><strong>{{ modeLabel }}</strong>
          </div>
          <div class="migration-summary__item">
            <span>位置</span>
            <strong class="migration-summary__path" :title="plan.target">{{
              plan.target
            }}</strong>
          </div>
          <div class="migration-summary__item">
            <span>文件</span><strong>{{ plan.files }} 个</strong>
          </div>
          <div class="migration-summary__item">
            <span>大小</span><strong>{{ formatSize(plan.bytes) }}</strong>
          </div>
          <div class="migration-summary__item">
            <span>空间</span><strong>{{ spaceText }}</strong>
          </div>
          <div class="migration-summary__item">
            <span>运行环境</span
            ><strong>{{ plan.environments.length }} 个</strong>
          </div>
        </div>
        <p v-if="plan.mode === 'repair'" class="field__hint">
          旧目录不存在, 只更新绑定.
        </p>
        <p v-if="error" role="alert" class="migration-error">{{ error }}</p>
        <div class="modal__foot">
          <BaseButton variant="primary" :disabled="busy" @click="start">{{
            plan.mode === "repair" ? "更新绑定" : "开始迁移"
          }}</BaseButton>
          <BaseButton variant="ghost" :disabled="busy" @click="back"
            >重新选择</BaseButton
          >
        </div>
      </motion.div>

      <motion.div
        v-else-if="showProgress && status"
        key="progress"
        :initial="{ opacity: 0, y: 8 }"
        :animate="{ opacity: 1, y: 0 }"
        :exit="{ opacity: 0, y: -6 }"
        :transition="surface"
      >
        <div class="migration-progress__head" role="status" aria-live="polite">
          <strong>{{ phaseLabel }}</strong>
          <span>{{ percent }}<small>%</small></span>
        </div>
        <div class="download-meter">
          <progress
            :value="percent"
            max="100"
            :aria-label="'迁移进度 ' + percent + '%'"
          />
          <div class="download-meter__track" aria-hidden="true">
            <motion.div
              class="download-meter__fill"
              :style="{ scaleX: progress }"
            />
          </div>
        </div>
        <p
          v-if="status.current"
          class="migration-progress__file"
          :title="status.current"
        >
          {{ status.current }}
        </p>
        <p v-if="status.totalFiles">
          {{ status.files }} / {{ status.totalFiles }} 个文件
        </p>
        <p v-if="error" role="alert" class="migration-error">{{ error }}</p>
        <div class="modal__foot">
          <BaseButton
            variant="ghost"
            :disabled="!status.canCancel"
            @click="cancel"
            >取消迁移</BaseButton
          >
        </div>
      </motion.div>

      <motion.div
        v-else-if="showResult && status"
        key="result"
        :initial="{ opacity: 0, y: 8 }"
        :animate="{ opacity: 1, y: 0 }"
        :exit="{ opacity: 0, y: -6 }"
        :transition="surface"
      >
        <p class="migration-result__title" role="status" aria-live="polite">
          {{ phaseLabel }}
        </p>
        <p v-if="status.phase === 'done'" class="migration-result__path">
          {{ status.target }}
        </p>
        <p v-if="status.phase === 'repair'" class="field__hint">
          检查 prefix 链接和配置文件权限后重试, 或放弃修复.
        </p>
        <p v-if="requiresCleanup" class="field__hint">旧目录仍保留.</p>
        <p v-if="error" role="alert" class="migration-error">{{ error }}</p>
        <div class="modal__foot">
          <template v-if="status.phase === 'repair'">
            <BaseButton variant="primary" @click="retry">重试</BaseButton>
            <BaseButton variant="ghost" @click="abandon">放弃修复</BaseButton>
          </template>
          <template v-else-if="requiresCleanup">
            <BaseButton variant="danger" :disabled="running" @click="cleanup"
              >删除旧目录</BaseButton
            >
            <BaseButton variant="ghost" :disabled="running" @click="keep"
              >保留旧目录</BaseButton
            >
          </template>
          <BaseButton
            v-else
            variant="secondary"
            :disabled="running"
            @click="finish"
            >关闭</BaseButton
          >
        </div>
      </motion.div>
    </AnimatePresence>
  </BaseModal>
</template>

<style scoped>
.migration-summary {
  display: grid;
  gap: 10px;
  padding-block: 4px 12px;
}

.migration-summary__item {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 20px;
  padding-bottom: 10px;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  font-size: var(--text-sm);
}

.migration-summary__item strong {
  color: var(--bone);
  font-weight: var(--weight-medium);
  text-align: right;
  overflow-wrap: anywhere;
}

.migration-progress__head {
  display: flex;
  align-items: end;
  justify-content: space-between;
  gap: 20px;
  padding-top: 8px;
}

.migration-progress__head strong {
  color: var(--bone);
  font-size: var(--text-md);
  font-weight: var(--weight-medium);
}

.migration-progress__head span {
  color: var(--bone);
  font-size: 30px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
}

.migration-progress__head small {
  padding-left: 2px;
  color: var(--muted);
  font-size: var(--text-sm);
}

.migration-progress__file {
  overflow: hidden;
  color: var(--bone);
  font-size: var(--text-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.migration-progress p {
  margin: 6px 0 0;
  color: var(--muted);
  font-size: var(--text-sm);
}

.migration-result__title {
  margin: 0 0 6px;
  color: var(--bone);
  font-size: var(--text-md);
  font-weight: var(--weight-medium);
}

.migration-result__path {
  margin: 0;
  color: var(--muted);
  font-size: var(--text-sm);
  overflow-wrap: anywhere;
}

.migration-error {
  margin: 12px 0 0;
  color: var(--danger);
  font-size: var(--text-sm);
  line-height: 1.6;
  overflow-wrap: anywhere;
}
</style>
