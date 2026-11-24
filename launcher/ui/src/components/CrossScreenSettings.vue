<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from "vue";
import { invoke } from "../core/api";
import { state } from "../core/controller";
import type { CrossScreenStatus } from "../types/launcher";
import BaseSwitch from "./BaseSwitch.vue";
import BaseButton from "./BaseButton.vue";
import SettingGroup from "./settings/SettingGroup.vue";
import SettingRow from "./settings/SettingRow.vue";

const props = defineProps<{ active: boolean }>();
const status = ref<CrossScreenStatus | null>(null);
const busy = ref(false);
const enabled = ref(false);
const error = ref("");
let revision = 0;
function refresh() {
  const current = ++revision;
  status.value = null;
  error.value = "";
  void invoke("cross_screen_status").then(
    (value) => {
      if (current === revision) {
        status.value = value;
        enabled.value = value.enabled;
      }
    },
    (reason: Error) => {
      if (current === revision) error.value = reason.message;
    },
  );
}
watch(
  () => [props.active, state.bootstrap?.settings.activeGame] as const,
  ([active]) => {
    revision += 1;
    if (active) refresh();
  },
  { immediate: true },
);
onBeforeUnmount(() => {
  revision += 1;
});
function configure(value: boolean) {
  if (busy.value) return;
  const current = ++revision;
  busy.value = true;
  enabled.value = value;
  error.value = "";
  void invoke("cross_screen_configure", { enabled: value })
    .then(
      (value) => {
        if (current === revision) {
          status.value = value;
          enabled.value = value.enabled;
        }
      },
      (reason: Error) => {
        if (current === revision) error.value = reason.message;
      },
    )
    .finally(() => {
      busy.value = false;
      if (current === revision) enabled.value = status.value?.enabled ?? false;
    });
}
function action(command: "cross_screen_install" | "cross_screen_restore") {
  if (busy.value) return;
  const current = ++revision;
  busy.value = true;
  error.value = "";
  void invoke(command)
    .then(
      () => {
        if (current === revision) refresh();
      },
      (reason: Error) => {
        if (current === revision) error.value = reason.message;
      },
    )
    .finally(() => {
      busy.value = false;
    });
}
</script>
<template>
  <SettingGroup title="跨屏游玩">
    <SettingRow
      data-setting="cross-screen"
      tabindex="-1"
      label="跨屏游玩"
      description="在 Moonlight 中选择游戏. 分辨率和帧率随客户端自动适配, 结束后恢复显示设置."
      for="set-cross-screen"
    >
      <BaseSwitch
        id="set-cross-screen"
        :model-value="enabled"
        :disabled="
          busy ||
          state.game !== 'idle' ||
          !status ||
          (!status.enabled && !status.supported)
        "
        @update:model-value="configure"
      />
    </SettingRow>
    <p class="field__hint" role="status">
      {{ error || status?.message || "正在检查串流环境..." }}
    </p>
    <BaseButton
      v-if="status?.missing.length"
      :busy="busy"
      @click="action('cross_screen_install')"
      >安装组件</BaseButton
    >
    <BaseButton v-if="error" :disabled="busy" @click="refresh"
      >重新检查</BaseButton
    >
    <BaseButton
      v-if="status?.needsRestore"
      :disabled="busy || state.game !== 'idle'"
      @click="action('cross_screen_restore')"
      >恢复显示设置</BaseButton
    >
  </SettingGroup>
</template>
