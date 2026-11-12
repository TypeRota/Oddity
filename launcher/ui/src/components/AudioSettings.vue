<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { invoke } from "../core/api";
import { form, queueSettingsSave } from "../core/controller";
import type { AudioStatus } from "../types/launcher";
import BaseSelect from "./BaseSelect.vue";
import BaseButton from "./BaseButton.vue";
import SettingGroup from "./settings/SettingGroup.vue";
import SettingRow from "./settings/SettingRow.vue";
const props = defineProps<{ active: boolean }>();
const status = ref<AudioStatus | null>(null);
const error = ref("");
let timer: ReturnType<typeof setTimeout> | undefined;
let revision = 0;
function refresh() {
  const current = revision;
  void invoke("audio_status", {})
    .then((next) => {
      if (current !== revision) return;
      status.value = next;
      error.value = next.error ?? "";
    })
    .catch((reason: unknown) => {
      if (current === revision) error.value = String(reason);
    })
    .finally(() => {
      if (current === revision && props.active)
        timer = setTimeout(refresh, 1000);
    });
}
watch(
  () => props.active,
  (active) => {
    revision += 1;
    clearTimeout(timer);
    if (active) refresh();
  },
  { immediate: true },
);
onBeforeUnmount(() => {
  revision += 1;
  clearTimeout(timer);
});
const missing = computed(
  () =>
    form.audio.device !== null &&
    status.value !== null &&
    !status.value.devices.some((device) => device.name === form.audio.device),
);
const options = computed(() => [
  { value: "", label: "跟随系统默认" },
  ...(status.value?.devices.map((device) => ({
    value: device.name,
    label: device.label,
  })) ?? []),
  ...(missing.value
    ? [{ value: form.audio.device!, label: "已保存的设备 (未连接)" }]
    : []),
]);
const device = computed({
  get: () => form.audio.device ?? "",
  set: (value: string) => {
    form.audio.device = value || null;
  },
});
const target = computed(() =>
  missing.value || form.audio.device === null
    ? status.value?.defaultDevice
    : form.audio.device,
);
const actual = computed(() =>
  [
    ...new Set(
      status.value?.streams.map(
        (stream) =>
          status.value?.devices.find((device) => device.index === stream.device)
            ?.label ?? "未知设备",
      ) ?? [],
    ),
  ].join(", "),
);
const redirected = computed(() =>
  status.value?.streams.some(
    (stream) =>
      status.value?.devices.find((device) => device.index === stream.device)
        ?.name !== target.value,
  ),
);
</script>
<template>
  <SettingGroup title="游戏音频">
    <SettingRow
      data-setting="audio-output"
      tabindex="-1"
      label="输出设备"
      description="设备断开时使用系统默认设备, 重新连接后自动切回."
      for="set-audio-device-trigger"
    >
      <BaseSelect
        id="set-audio-device"
        v-model="device"
        label="游戏输出设备"
        :options="options"
        :disabled="!status || !!error"
        @change="queueSettingsSave('game', 0)"
      />
    </SettingRow>
    <SettingRow
      data-setting="audio-volume"
      tabindex="-1"
      label="游戏音量"
      description="100% 为原始音量, 0% 为静音."
      for="set-audio-volume"
    >
      <div class="gamepad-strength">
        <input
          id="set-audio-volume"
          v-model.number="form.audio.volume"
          type="range"
          min="0"
          max="150"
          step="1"
          @input="queueSettingsSave('game')"
          @change="queueSettingsSave('game', 0)"
        />
        <output for="set-audio-volume">{{ form.audio.volume }}%</output>
      </div>
    </SettingRow>
    <SettingRow label="恢复原始音量"
      ><BaseButton
        :disabled="form.audio.volume === 100"
        @click="
          form.audio.volume = 100;
          queueSettingsSave('game', 0);
        "
        >恢复为 100%</BaseButton
      ></SettingRow
    >
    <p v-if="form.audio.volume > 100" class="field__hint">
      音量超过 100%, 可能产生失真.
    </p>
    <p v-if="error" class="field__hint" role="status">{{ error }}</p>
    <template v-else>
      <p v-if="missing" class="field__hint" role="status">
        所选设备未连接, 当前使用系统默认设备.
      </p>
      <p class="field__hint" role="status">
        {{
          status
            ? actual
              ? `当前输出: ${actual}.`
              : "等待游戏播放音频, 设置将在音频流建立后应用."
            : "正在读取音频设备."
        }}
      </p>
      <p v-if="redirected" class="field__hint" role="status">
        实际输出与所选设备不同, 请检查系统混音器或音效软件的路由设置.
      </p>
    </template>
  </SettingGroup>
</template>
