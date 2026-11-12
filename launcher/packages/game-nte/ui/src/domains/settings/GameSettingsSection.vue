<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  form,
  hostFacts,
  migrationBusy,
  queueSettingsSave,
  runtimeEnvironmentBusy,
  runtimePathsLocked,
  settingsPathsLocked,
  type MeteredPolicy,
} from "@host/api";
import {
  BaseButton,
  BaseInput,
  BaseSelect,
  BaseSwitch,
  DirectoryField,
  GameAccountDialog,
  InstallMigration,
  SettingGroup,
  SettingRow,
} from "@host/ui-kit";

const gameAccountsOpen = ref(false);
const distributionOptions = computed(() =>
  hostFacts.channels.value.map((channel) => ({
    label: channel.label,
    value: channel.id,
  })),
);
const cnRootKey = computed(
  () =>
    hostFacts.channels.value.find((channel) => channel.default)
      ?.installRootKey ?? "cn",
);
const globalRootKey = computed(
  () =>
    hostFacts.channels.value.find((channel) => !channel.default)
      ?.installRootKey ?? "global",
);
const cnInstallRoot = computed({
  get: () => form.installRoots[cnRootKey.value] ?? "",
  set: (value: string) => {
    form.installRoots[cnRootKey.value] = value;
  },
});
const globalInstallRoot = computed({
  get: () => form.installRoots[globalRootKey.value] ?? "",
  set: (value: string) => {
    form.installRoots[globalRootKey.value] = value;
  },
});
const distributionModel = computed({
  get: () => form.distribution ?? "",
  set: (value: string) => {
    form.distribution = value;
  },
});
const directSupported = computed(
  () => form.distribution === "cn" && hostFacts.game.value === "nte",
);
const launchOptions = computed(() => [
  {
    label: "后台启动 (推荐)",
    value: "background",
    hint: "等待官方资源检查, 自动登录并启动游戏, 隐藏官方主窗口.",
  },
  {
    label: "前台启动",
    value: "official",
    hint: "显示官方窗口, 请从该窗口点击启动游戏.",
  },
  {
    label: "直接启动",
    value: "direct",
    hint: "使用所选游戏账号登录并直接启动游戏. 目前仅支持国服.",
    disabled: !directSupported.value,
  },
]);
const launchModeHint = computed(
  () =>
    launchOptions.value.find((option) => option.value === form.launchMode)
      ?.hint,
);
const meteredOptions = [
  {
    label: "询问用户",
    value: "ask",
    hint: "检测到计费网络时先暂停, 由通知里的按钮确认后继续.",
  },
  {
    label: "暂停自动下载",
    value: "pause",
    hint: "计费网络下不自动下载, 切换到普通网络后继续.",
  },
  {
    label: "照常下载",
    value: "allow",
    hint: "无论是否计费都自动下载.",
  },
];
const meteredHint = computed(
  () =>
    meteredOptions.find((option) => option.value === form.updates.metered)
      ?.hint,
);
const LIMIT_MAX_MIB = 1024;
const MIB = 1024 * 1024;
function formatLimit(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0";
  const mib = bytes / MIB;
  // 小数值保留更多小数位, 避免 1 KiB/s 这类设置被格式化成 0.
  const text = mib >= 1 ? mib.toFixed(3) : mib.toFixed(6);
  return String(Number(text));
}
const limitDraft = ref(formatLimit(form.updates.limitBytesPerSecond));
const limitInvalid = ref(false);
let limitEditing = false;
watch(
  () => form.updates.limitBytesPerSecond,
  (bytes) => {
    if (!limitEditing) {
      limitDraft.value = formatLimit(bytes);
      limitInvalid.value = false;
    }
  },
);
function applyLimitDraft() {
  // type=number 时 Vue 的 v-model 会把值转成 number.
  const text = String(limitDraft.value).trim();
  if (!text) {
    form.updates.limitBytesPerSecond = 0;
    limitInvalid.value = false;
    queueSettingsSave("game");
    return;
  }
  const value = Number(text);
  if (!Number.isFinite(value) || value < 0 || value > LIMIT_MAX_MIB) {
    limitInvalid.value = true;
    return;
  }
  limitInvalid.value = false;
  const bytes = Math.round(value * MIB);
  if (bytes !== form.updates.limitBytesPerSecond) {
    form.updates.limitBytesPerSecond = bytes;
    queueSettingsSave("game");
  }
}
function onLimitInput() {
  limitEditing = true;
  applyLimitDraft();
}
function onLimitChange() {
  limitEditing = false;
  applyLimitDraft();
  // 非法输入在失焦时回到最后一次有效值.
  limitDraft.value = formatLimit(form.updates.limitBytesPerSecond);
  limitInvalid.value = false;
  queueSettingsSave("game", 0);
}
const meteredPolicy = computed({
  get: () => form.updates.metered as string,
  set: (value: string) => {
    form.updates.metered = value as MeteredPolicy;
  },
});
</script>

<template>
  <SettingGroup title="游戏安装">
    <p v-if="settingsPathsLocked" class="settings-lock" role="status">
      {{
        migrationBusy
          ? "安装目录迁移进行中, 暂时无法修改运行路径."
          : hostFacts.gamePhase.value !== "idle" || runtimeEnvironmentBusy
            ? "游戏或运行环境任务进行中, 暂时无法修改运行路径."
            : "游戏资源任务进行中, 暂时无法修改游戏地区和安装目录."
      }}
    </p>
    <SettingRow
      data-setting="distribution"
      tabindex="-1"
      label="游戏地区"
      for="set-distribution-trigger"
      ><BaseSelect
        id="set-distribution"
        label="游戏地区"
        v-model="distributionModel"
        :options="distributionOptions"
        :disabled="settingsPathsLocked"
        @change="queueSettingsSave('game', 0)"
    /></SettingRow>
    <div class="settings-path" data-setting="install" tabindex="-1">
      <DirectoryField
        id="set-install"
        v-model="cnInstallRoot"
        label="国服安装目录"
        placeholder="/path/to/Neverness To Everness"
        :disabled="settingsPathsLocked"
        @input="queueSettingsSave('game')"
        @change="queueSettingsSave('game', 0)"
      />
    </div>
    <div class="settings-path" data-setting="global-install" tabindex="-1">
      <DirectoryField
        id="set-global-install"
        v-model="globalInstallRoot"
        label="国际服安装目录"
        placeholder="~/Games/Neverness To Everness Global"
        :disabled="settingsPathsLocked"
        @input="queueSettingsSave('game')"
        @change="queueSettingsSave('game', 0)"
      />
    </div>
    <SettingRow
      data-setting="migration"
      tabindex="-1"
      label="迁移安装目录"
      description="移动到其他位置, 保留运行环境和预下载资源."
    >
      <div class="settings-migration">
        <InstallMigration distribution="cn" :disabled="settingsPathsLocked" />
        <InstallMigration
          distribution="global"
          :disabled="settingsPathsLocked"
        />
      </div>
    </SettingRow>
  </SettingGroup>
  <SettingGroup title="启动"
    ><SettingRow
      data-setting="launch"
      tabindex="-1"
      label="启动方式"
      :description="launchModeHint"
      for="set-mode-trigger"
      ><BaseSelect
        id="set-mode"
        label="启动方式"
        v-model="form.launchMode"
        :options="launchOptions"
        :disabled="runtimePathsLocked"
        @change="queueSettingsSave('game', 0)"
    /></SettingRow>
    <SettingRow
      data-setting="game-account"
      tabindex="-1"
      label="游戏账号"
      :description="
        directSupported
          ? '用于下次直接启动, 与塔吉多账号分别保存.'
          : '游戏账号登录目前仅支持《异环》国服.'
      "
    >
      <BaseButton :disabled="!directSupported" @click="gameAccountsOpen = true"
        >管理游戏账号</BaseButton
      >
    </SettingRow>
  </SettingGroup>
  <SettingGroup title="资源更新">
    <div class="settings-path" data-setting="preload-dir" tabindex="-1">
      <DirectoryField
        id="set-preload-root"
        v-model="form.preloadRoot"
        label="预下载资源目录"
        placeholder="~/Games/OddityPreload"
        :disabled="settingsPathsLocked"
        @input="queueSettingsSave('game')"
        @change="queueSettingsSave('game', 0)"
      />
      <p class="field__hint">留空时使用安装目录内的默认缓存路径.</p>
    </div>
    <SettingRow
      data-setting="updates"
      tabindex="-1"
      label="自动下载"
      for="set-auto-download"
      description="发现游戏资源更新后自动下载. 仅在启动器运行且游戏未运行时执行."
      v-slot="{ describedby }"
    >
      <BaseSwitch
        id="set-auto-download"
        v-model="form.updates.autoDownload"
        label="自动下载"
        :describedby="describedby"
        @change="queueSettingsSave('game', 0)"
      />
    </SettingRow>
    <SettingRow
      v-if="form.updates.autoDownload"
      label="计费网络"
      :description="meteredHint"
      for="set-metered-trigger"
    >
      <BaseSelect
        id="set-metered"
        label="计费网络"
        v-model="meteredPolicy"
        :options="meteredOptions"
        @change="queueSettingsSave('game', 0)"
      />
    </SettingRow>
    <SettingRow
      v-if="form.updates.autoDownload"
      label="自动下载时段"
      for="set-updates-window"
      description="仅在指定时段内开始或继续自动下载, 支持跨零点."
      v-slot="{ describedby }"
    >
      <BaseSwitch
        id="set-updates-window"
        v-model="form.updates.windowEnabled"
        label="自动下载时段"
        :describedby="describedby"
        @change="queueSettingsSave('game', 0)"
      />
    </SettingRow>
    <div
      v-if="form.updates.autoDownload && form.updates.windowEnabled"
      class="field-grid updates-schedule"
    >
      <label class="field"
        ><span>开始时间</span
        ><BaseInput
          id="set-updates-window-start"
          v-model="form.updates.windowStart"
          type="time"
          @change="queueSettingsSave('game', 0)"
      /></label>
      <label class="field"
        ><span>结束时间</span
        ><BaseInput
          id="set-updates-window-end"
          v-model="form.updates.windowEnd"
          type="time"
          @change="queueSettingsSave('game', 0)"
      /></label>
    </div>
    <SettingRow
      label="下载限速 (MiB/s)"
      for="set-download-limit"
      description="支持小数, 0 表示不限速. 手动下载同样生效."
      v-slot="{ describedby }"
    >
      <BaseInput
        id="set-download-limit"
        v-model="limitDraft"
        type="number"
        min="0"
        :max="LIMIT_MAX_MIB"
        step="any"
        inputmode="decimal"
        :invalid="limitInvalid"
        :aria-describedby="describedby"
        @input="onLimitInput"
        @change="onLimitChange"
      />
    </SettingRow>
  </SettingGroup>
  <GameAccountDialog
    v-model:open="gameAccountsOpen"
    :selected="form.gameAccountId"
    :theme="form.theme"
    :reduce-motion="form.reduceMotion"
    @select="
      form.gameAccountId = $event;
      queueSettingsSave('game', 0);
    "
  />
</template>
