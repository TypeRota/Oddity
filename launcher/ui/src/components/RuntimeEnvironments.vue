<script setup lang="ts">
import BaseButton from "./BaseButton.vue";
import { computed, onMounted, ref, watch } from "vue";
import {
  state,
  form,
  formatSize,
  runtimeEnvironmentBusy,
  runtimeEnvironmentLocked,
  createRuntimeEnvironment,
  deleteEnvironmentOrphan,
  importRuntimeEnvironment,
  loadEnvironmentAudit,
  runtimeEnvironmentAction,
  renameRuntimeEnvironment,
} from "../core/controller";
import type {
  EnvironmentAudit,
  EnvironmentHealth,
  EnvironmentHealthState,
  EnvironmentOrphan,
  RuntimeEnvironment,
} from "../types/launcher";
import {
  activeEnvironmentId,
  activeGame,
  environmentInUse,
  stableEnvironmentId,
} from "../core/game";
import BaseInput from "./BaseInput.vue";
import SettingGroup from "./settings/SettingGroup.vue";
import BaseDisclosure from "./BaseDisclosure.vue";
import BaseModal from "./BaseModal.vue";
import BaseSelect from "./BaseSelect.vue";
import DirectoryField from "./DirectoryField.vue";

type ConfirmTarget =
  | { kind: "discard"; environment: RuntimeEnvironment }
  | { kind: "forget"; environment: RuntimeEnvironment }
  | { kind: "orphan"; orphan: EnvironmentOrphan };

const selected = ref("");
const creating = ref(false);
const name = ref("");
const proton = ref("");
const renaming = ref(false);
const renameName = ref("");
const sourceId = ref("");
const audit = ref<EnvironmentAudit | null>(null);
const auditState = ref<"loading" | "ready" | "error">("loading");
const maintenance = ref(true);
const confirmation = ref<ConfirmTarget | null>(null);
const importing = ref<EnvironmentOrphan | null>(null);
const importName = ref("");
const importProton = ref("");
const settings = computed(() => state.bootstrap?.settings);
const channels = computed(() => state.bootstrap?.channels ?? []);
const profiles = computed(() =>
  (settings.value?.runtimeEnvironments ?? []).filter(
    (entry) => entry.game === activeGame(settings.value),
  ),
);
// 当前和稳定环境按当前区服取值; 切换区服后各自的标记互不影响.
const active = computed(
  () => activeEnvironmentId(settings.value, channels.value) ?? "",
);
const stable = computed(
  () => stableEnvironmentId(settings.value, channels.value) ?? "",
);
const profile = computed(() =>
  profiles.value.find((entry) => entry.id === selected.value),
);
const source = computed(() =>
  profiles.value.find((entry) => entry.id === sourceId.value),
);
const health = computed<EnvironmentHealth | null>(
  () =>
    audit.value?.environments.find((entry) => entry.id === profile.value?.id) ??
    null,
);
const UNUSABLE = "环境目录不可用, 无法启动.";
const HEALTH_LABELS: Record<EnvironmentHealthState, string> = {
  ready: "",
  "prefix-missing": UNUSABLE,
  "identity-invalid": UNUSABLE,
  "directory-missing": UNUSABLE,
  "proton-invalid": "固定的 Proton 已变化, 请创建新环境.",
};
const invalid = computed(
  () => !!health.value && health.value.state !== "ready",
);
// 环境绑定的安装目录与当前区服安装目录不一致时允许更新绑定.
const rebindable = computed(
  () =>
    health.value?.state === "ready" &&
    health.value.installRoot !== health.value.installRootCurrent,
);
const healthHint = computed(() => {
  if (!health.value) return "";
  if (health.value.state !== "ready") return HEALTH_LABELS[health.value.state];
  return health.value.installRoot === health.value.installRootCurrent
    ? ""
    : "上次使用的游戏目录与当前安装目录不同.";
});
const options = computed(() =>
  profiles.value.map((entry) => {
    const entryState = audit.value?.environments.find(
      (item) => item.id === entry.id,
    )?.state;
    return {
      value: entry.id,
      label:
        entry.name +
        (entry.id === active.value ? " · 当前" : "") +
        (entry.id === stable.value ? " · 稳定" : "") +
        (entryState && entryState !== "ready" ? " · 不可用" : ""),
    };
  }),
);
const sourceOptions = computed(() => [
  { value: "", label: "全新环境" },
  ...profiles.value.map((entry) => ({ value: entry.id, label: entry.name })),
]);
// 单个环境可能被多个区服共用, 任一区服标记为当前或稳定时都不能删除.
const removable = computed(
  () => !!profile.value && !environmentInUse(settings.value, profile.value.id),
);
const orphans = computed(() => audit.value?.orphans ?? []);
const confirmationTitle = computed(() => {
  const target = confirmation.value;
  if (!target) return "";
  if (target.kind === "discard") return "删除运行环境";
  if (target.kind === "forget") return "移除运行环境登记";
  return "删除未登记目录";
});
const confirmationAction = computed(() => {
  const target = confirmation.value;
  if (!target) return "";
  if (target.kind === "discard") return "确定删除";
  if (target.kind === "forget") return "移除登记";
  return "确定删除";
});
const orphanKindLabels: Record<EnvironmentOrphan["kind"], string> = {
  environment: "环境目录",
  logs: "日志目录",
  pending: "未完成操作",
};
watch(
  active,
  (value) => {
    selected.value = value;
  },
  { immediate: true },
);
watch(selected, () => {
  confirmation.value = null;
  creating.value = false;
  renaming.value = false;
  importing.value = null;
});
watch(sourceId, () => {
  proton.value = source.value?.proton.path ?? "";
});
// 设置或当前游戏变化后环境健康状态可能变化, 重新扫描.
watch(
  () => state.bootstrap,
  () => {
    void refreshAudit();
  },
);
onMounted(() => {
  void refreshAudit();
});

async function refreshAudit() {
  auditState.value = "loading";
  const next = await loadEnvironmentAudit();
  // 失败时保留最近一次成功的结果, 只标记状态供用户重试.
  if (!next) {
    auditState.value = "error";
    return;
  }
  audit.value = next;
  auditState.value = "ready";
}
function orphanName(orphan: EnvironmentOrphan) {
  if (orphan.kind !== "environment") return orphan.name;
  return (orphan.id ?? orphan.name).slice(0, 8);
}
function orphanTime(orphan: EnvironmentOrphan) {
  const date = new Date(orphan.modifiedAt * 1000);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
function openCreate() {
  confirmation.value = null;
  renaming.value = false;
  sourceId.value = profile.value?.id ?? "";
  proton.value = profile.value?.proton.path ?? "";
  name.value = "";
  creating.value = true;
}
async function create() {
  if (
    await createRuntimeEnvironment(
      name.value,
      proton.value,
      sourceId.value || null,
    )
  ) {
    creating.value = false;
    await refreshAudit();
  }
}
function openRename() {
  if (!profile.value) return;
  creating.value = false;
  confirmation.value = null;
  renameName.value = profile.value.name;
  renaming.value = true;
}
async function rename() {
  const entry = profile.value;
  if (entry && (await renameRuntimeEnvironment(renameName.value, entry.id))) {
    renaming.value = false;
    await refreshAudit();
  }
}
function openRemove() {
  if (!profile.value) return;
  creating.value = false;
  renaming.value = false;
  confirmation.value = invalid.value
    ? { kind: "forget", environment: profile.value }
    : { kind: "discard", environment: profile.value };
}
function confirmOrphan(orphan: EnvironmentOrphan) {
  creating.value = false;
  renaming.value = false;
  confirmation.value = { kind: "orphan", orphan };
}
async function confirmRemoval() {
  const target = confirmation.value;
  if (!target) return;
  if (target.kind === "orphan") {
    if (await deleteEnvironmentOrphan(target.orphan.kind, target.orphan.name)) {
      confirmation.value = null;
      await refreshAudit();
    }
    return;
  }
  const action = target.kind === "forget" ? "forget" : "discard";
  if (await runtimeEnvironmentAction(action, target.environment.id)) {
    confirmation.value = null;
    selected.value = active.value;
    await refreshAudit();
  }
}
function openImport(orphan: EnvironmentOrphan) {
  if (!orphan.id) return;
  creating.value = false;
  renaming.value = false;
  confirmation.value = null;
  importName.value = "导入的环境";
  importProton.value =
    state.bootstrap?.protonPath ||
    state.bootstrap?.settings.runtimeEnvironments.at(-1)?.proton.path ||
    "";
  importing.value = orphan;
}
async function confirmImport() {
  const orphan = importing.value;
  if (!orphan?.id) return;
  if (
    await importRuntimeEnvironment(
      orphan.id,
      importName.value,
      importProton.value,
    )
  ) {
    importing.value = null;
    await refreshAudit();
  }
}
</script>

<template>
  <div class="runtime-environments">
    <SettingGroup title="Proton 运行环境" id="runtime-environments-title">
      <p class="runtime-environments__notice">
        各环境使用独立 prefix, 共享游戏安装.
        游戏更新、共享配置、宿主文件和云端状态不会随环境切换恢复.
      </p>
      <div
        v-if="auditState === 'error'"
        class="runtime-environments__audit"
        role="alert"
      >
        <span>读取环境状态失败, 健康状态和未登记数据可能不是最新.</span>
        <BaseButton
          type="button"
          variant="ghost"
          size="sm"
          icon="i-arrows-clockwise"
          :disabled="runtimeEnvironmentLocked"
          @click="refreshAudit"
          >重新检测</BaseButton
        >
      </div>
      <div v-if="profiles.length" class="field-grid">
        <div class="field field--wide">
          <label for="runtime-environment">运行环境</label>
          <BaseSelect
            id="runtime-environment"
            label="运行环境"
            v-model="selected"
            :options="options"
            :disabled="runtimeEnvironmentLocked"
            placeholder="请选择运行环境"
          />
          <small v-if="profile" class="field__hint">
            Proton: {{ profile.proton.path }}<br />
            上次使用的游戏目录: {{ profile.installRoot }}
            <template v-if="healthHint">
              <br /><span :class="{ 'is-invalid': invalid }">{{
                healthHint
              }}</span>
            </template>
          </small>
        </div>
      </div>
      <p v-else class="field__hint">请先创建 Proton 运行环境, 再启动游戏.</p>
      <div class="runtime-environments__actions">
        <BaseButton
          type="button"
          :variant="profiles.length ? 'secondary' : 'primary'"
          :disabled="runtimeEnvironmentLocked || !state.bootstrap"
          @click="openCreate"
        >
          <svg class="icon" aria-hidden="true"><use href="#i-plus" /></svg>
          {{ profiles.length ? "创建环境或副本" : "创建运行环境" }}
        </BaseButton>
        <template v-if="profiles.length">
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="runtimeEnvironmentLocked || !profile"
            @click="openRename"
          >
            重命名
          </BaseButton>
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="
              runtimeEnvironmentLocked ||
              !profile ||
              selected === active ||
              invalid
            "
            @click="runtimeEnvironmentAction('switch', selected)"
          >
            切换到此环境
          </BaseButton>
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="
              runtimeEnvironmentLocked ||
              !profile ||
              selected === stable ||
              invalid
            "
            @click="runtimeEnvironmentAction('keep', selected)"
          >
            保留为稳定环境
          </BaseButton>
          <BaseButton
            v-if="rebindable"
            type="button"
            variant="ghost"
            :disabled="runtimeEnvironmentLocked"
            @click="runtimeEnvironmentAction('rebind', selected)"
          >
            更新绑定
          </BaseButton>
          <BaseButton
            type="button"
            variant="danger"
            :disabled="
              runtimeEnvironmentLocked || !profile || (!invalid && !removable)
            "
            @click="openRemove"
          >
            {{ invalid ? "仅移除登记" : "删除环境" }}
          </BaseButton>
        </template>
      </div>
      <small v-if="active" class="field__hint"
        >当前 prefix: {{ state.bootstrap?.prefix || "不可用" }}</small
      >
      <BaseModal
        :open="creating"
        title="创建运行环境"
        :theme="form.theme"
        :reduce-motion="form.reduceMotion"
        @update:open="creating = $event"
      >
        <div class="field-grid">
          <div v-if="profiles.length" class="field field--wide">
            <label for="runtime-environment-source">创建来源</label>
            <BaseSelect
              id="runtime-environment-source"
              label="创建来源"
              v-model="sourceId"
              :options="sourceOptions"
              :disabled="runtimeEnvironmentLocked"
            />
          </div>
          <div class="field field--wide">
            <label for="runtime-environment-name">环境名称</label>
            <BaseInput
              id="runtime-environment-name"
              v-model="name"
              maxlength="64"
              :disabled="runtimeEnvironmentLocked"
              placeholder="例如 GE-Proton 稳定版"
            />
          </div>
          <DirectoryField
            id="runtime-environment-proton"
            v-model="proton"
            label="Proton 目录"
            class="field--wide"
            :disabled="runtimeEnvironmentLocked"
          />
        </div>
        <p class="runtime-environments__notice" v-if="source">
          源环境: {{ source.name }}. 复制其 prefix 后使用所选 Proton.
          映射到宿主的 Windows 用户目录将在副本中使用独立空目录,
          首次启动可能需要重新登录.
        </p>
        <p class="runtime-environments__notice" v-else>
          创建独立 prefix, 首次启动时由 Proton 初始化.
          首个环境会自动设为当前和稳定环境.
        </p>
        <p v-if="runtimeEnvironmentBusy" role="status" aria-live="polite">
          正在创建运行环境, 请稍候.
        </p>
        <div class="runtime-environments__actions">
          <BaseButton
            type="button"
            variant="primary"
            :disabled="
              runtimeEnvironmentLocked || !name.trim() || !proton.trim()
            "
            @click="create"
          >
            <svg class="icon" aria-hidden="true">
              <use :href="source ? '#i-copy' : '#i-plus'" />
            </svg>
            {{ source ? "创建副本" : "创建环境" }}
          </BaseButton>
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="runtimeEnvironmentLocked"
            @click="creating = false"
          >
            取消创建
          </BaseButton>
        </div>
      </BaseModal>
      <BaseModal
        :open="renaming"
        title="重命名运行环境"
        :theme="form.theme"
        :reduce-motion="form.reduceMotion"
        @update:open="renaming = $event"
      >
        <div class="field-grid">
          <div class="field field--wide">
            <label for="runtime-environment-rename">环境名称</label>
            <BaseInput
              id="runtime-environment-rename"
              v-model="renameName"
              maxlength="64"
              :disabled="runtimeEnvironmentLocked"
              placeholder="例如 GE-Proton 稳定版"
            />
            <small class="field__hint"
              >名称在全部运行环境中唯一. 当前和稳定环境也可以改名, 不影响
              prefix、游戏目录和标记.</small
            >
          </div>
        </div>
        <p v-if="runtimeEnvironmentBusy" role="status" aria-live="polite">
          正在重命名运行环境, 请稍候.
        </p>
        <div class="runtime-environments__actions">
          <BaseButton
            type="button"
            variant="primary"
            :disabled="runtimeEnvironmentLocked || !renameName.trim()"
            @click="rename"
          >
            保存名称
          </BaseButton>
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="runtimeEnvironmentBusy"
            @click="renaming = false"
          >
            取消
          </BaseButton>
        </div>
      </BaseModal>
      <p
        v-if="
          runtimeEnvironmentBusy &&
          !creating &&
          !confirmation &&
          !renaming &&
          !importing
        "
        role="status"
        aria-live="polite"
      >
        正在处理运行环境, 请稍候.
      </p>
      <BaseModal
        :open="!!confirmation"
        :title="confirmationTitle"
        :theme="form.theme"
        :reduce-motion="form.reduceMotion"
        @update:open="
          (value) => {
            if (!value) confirmation = null;
          }
        "
      >
        <template v-if="confirmation?.kind === 'discard'">
          <p>
            确定删除“{{ confirmation.environment.name }}”的 prefix?
            此操作无法撤销.
          </p>
          <p class="field__hint">游戏安装和其他环境不受影响.</p>
        </template>
        <template v-else-if="confirmation?.kind === 'forget'">
          <p>确定移除“{{ confirmation.environment.name }}”的登记?</p>
          <p class="field__hint">
            仅清除设置中的登记和当前/稳定记忆, 磁盘目录保留.
          </p>
        </template>
        <template v-else-if="confirmation?.kind === 'orphan'">
          <p>
            确定删除未登记的{{ orphanKindLabels[confirmation.orphan.kind] }}“{{
              orphanName(confirmation.orphan)
            }}”? 此操作无法撤销.
          </p>
          <p class="field__hint">磁盘目录会被直接清理.</p>
        </template>
        <p v-if="runtimeEnvironmentBusy" role="status" aria-live="polite">
          {{
            confirmation?.kind === "forget"
              ? "正在移除运行环境登记, 请稍候."
              : "正在删除, 请稍候."
          }}
        </p>
        <div class="runtime-environments__actions">
          <BaseButton
            type="button"
            variant="danger"
            :disabled="runtimeEnvironmentLocked"
            @click="confirmRemoval"
          >
            <svg class="icon" aria-hidden="true"><use href="#i-trash" /></svg>
            {{ confirmationAction }}
          </BaseButton>
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="runtimeEnvironmentBusy"
            @click="confirmation = null"
          >
            取消
          </BaseButton>
        </div>
      </BaseModal>
      <BaseModal
        :open="!!importing"
        title="导入运行环境"
        :theme="form.theme"
        :reduce-motion="form.reduceMotion"
        @update:open="
          (value) => {
            if (!value) importing = null;
          }
        "
      >
        <template v-if="importing">
          <p class="field__hint">
            {{ orphanName(importing) }} · {{ formatSize(importing.bytes) }} ·
            {{ orphanTime(importing) }}
          </p>
          <div class="field-grid">
            <div class="field field--wide">
              <label for="runtime-environment-import-name">环境名称</label>
              <BaseInput
                id="runtime-environment-import-name"
                v-model="importName"
                maxlength="64"
                :disabled="runtimeEnvironmentLocked"
                placeholder="例如 导入的环境"
              />
            </div>
            <DirectoryField
              id="runtime-environment-import-proton"
              v-model="importProton"
              label="Proton 目录"
              class="field--wide"
              :disabled="runtimeEnvironmentLocked"
            />
          </div>
          <p class="field__hint">导入后绑定当前区服的安装目录.</p>
        </template>
        <div class="runtime-environments__actions">
          <BaseButton
            type="button"
            variant="primary"
            :disabled="
              runtimeEnvironmentLocked ||
              !importName.trim() ||
              !importProton.trim()
            "
            @click="confirmImport"
          >
            导入环境
          </BaseButton>
          <BaseButton
            type="button"
            variant="ghost"
            :disabled="runtimeEnvironmentBusy"
            @click="importing = null"
          >
            取消导入
          </BaseButton>
        </div>
      </BaseModal>
    </SettingGroup>
    <SettingGroup
      title="环境数据维护"
      description="未登记的环境目录和日志不会被自动删除."
    >
      <BaseDisclosure v-model="maintenance" title="未登记的环境与日志">
        <p v-if="!audit && auditState === 'loading'" class="field__hint">
          正在检查未登记的环境与日志.
        </p>
        <div v-else-if="orphans.length" class="runtime-orphans">
          <div
            v-for="orphan in orphans"
            :key="orphan.kind + ':' + orphan.name"
            class="runtime-orphan"
          >
            <div class="runtime-orphan__copy">
              <strong
                >{{ orphanKindLabels[orphan.kind] }} ·
                {{ orphanName(orphan) }}</strong
              >
              <span
                >{{ formatSize(orphan.bytes) }} · {{ orphanTime(orphan) }}</span
              >
              <span v-if="orphan.kind === 'environment'">
                {{ orphan.identityValid ? "身份完好" : "身份缺失" }} ·
                {{ orphan.prefixValid ? "prefix 完好" : "缺少 prefix" }}
              </span>
            </div>
            <div class="runtime-orphan__actions">
              <BaseButton
                v-if="
                  orphan.kind === 'environment' &&
                  orphan.identityValid &&
                  orphan.prefixValid
                "
                type="button"
                variant="secondary"
                :disabled="runtimeEnvironmentLocked"
                @click="openImport(orphan)"
              >
                导入
              </BaseButton>
              <BaseButton
                type="button"
                variant="ghost"
                :disabled="runtimeEnvironmentLocked"
                @click="confirmOrphan(orphan)"
              >
                删除
              </BaseButton>
            </div>
          </div>
        </div>
        <p v-else-if="audit" class="field__hint">
          未发现未登记的环境目录或日志.
        </p>
      </BaseDisclosure>
    </SettingGroup>
  </div>
</template>

<style scoped>
.runtime-environments {
  display: contents;
}

.runtime-environments__notice {
  margin: 0 0 var(--space-16);
  color: var(--muted);
  font-size: var(--text-sm);
  line-height: 1.7;
}

.runtime-environments__audit {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-12);
  margin: 0 0 var(--space-16);
  color: var(--danger);
  font-size: var(--text-sm);
}

.runtime-environments__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-24);
  margin: var(--space-16) 0;
}

.field__hint {
  overflow-wrap: anywhere;
}

.field__hint .is-invalid {
  color: var(--danger);
}

.runtime-orphans {
  display: flex;
  flex-direction: column;
  gap: var(--space-12);
}

.runtime-orphan {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-16);
  padding: var(--space-12) 0;
  border-bottom: 1px solid var(--line);
}

.runtime-orphan:last-child {
  border-bottom: 0;
}

.runtime-orphan__copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  font-size: var(--text-sm);
  color: var(--muted);
}

.runtime-orphan__copy strong {
  color: var(--bone);
  font-weight: 500;
}

.runtime-orphan__actions {
  display: flex;
  align-items: center;
  gap: var(--space-12);
  flex: none;
}
</style>
