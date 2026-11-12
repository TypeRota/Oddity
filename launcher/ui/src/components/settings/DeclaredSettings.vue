<script setup lang="ts">
import { computed } from "vue";
import {
  declaredSettings,
  queueSettingsSave,
  settingsSchema,
} from "../../core/controller";
import type {
  SettingsFieldValue,
  SettingsFieldView,
} from "../../types/launcher";
import BaseInput from "../BaseInput.vue";
import BaseSelect from "../BaseSelect.vue";
import BaseSwitch from "../BaseSwitch.vue";
import SettingGroup from "./SettingGroup.vue";
import SettingRow from "./SettingRow.vue";

const props = defineProps<{ owner: string; title: string }>();
const schema = computed(() => settingsSchema(props.owner));
const fields = computed(() =>
  [...(schema.value?.fields ?? [])].sort(
    (left, right) =>
      left.order - right.order || left.key.localeCompare(right.key),
  ),
);
const values = computed(() => declaredSettings[props.owner] ?? {});
const idBase = computed(
  () => `settings-${props.owner.replace(/[^a-z0-9]+/gi, "-")}`,
);

function set(key: string, value: SettingsFieldValue) {
  const target = (declaredSettings[props.owner] ??= {});
  target[key] = value;
  queueSettingsSave(props.owner, 0);
}

function setInteger(key: string, value: string) {
  const number = Number(value);
  if (!Number.isFinite(number)) return;
  set(key, Math.trunc(number));
}

function options(field: SettingsFieldView) {
  return (field.options ?? []).map((value) => ({ label: value, value }));
}
</script>

<template>
  <SettingGroup :title="title">
    <SettingRow
      v-for="field in fields"
      :key="field.key"
      :data-setting="field.key"
      tabindex="-1"
      :label="field.label"
      :description="field.description"
      :for="`${idBase}-${field.key}`"
    >
      <BaseSwitch
        v-if="field.type === 'boolean'"
        :id="`${idBase}-${field.key}`"
        :model-value="Boolean(values[field.key])"
        :label="field.label"
        @update:model-value="set(field.key, $event)"
      />
      <BaseInput
        v-else-if="field.type === 'integer'"
        :id="`${idBase}-${field.key}`"
        type="number"
        :min="field.min"
        :max="field.max"
        :model-value="String(values[field.key] ?? '')"
        @update:model-value="setInteger(field.key, $event)"
      />
      <BaseSelect
        v-else-if="field.type === 'select'"
        :id="`${idBase}-${field.key}`"
        :label="field.label"
        :model-value="String(values[field.key] ?? '')"
        :options="options(field)"
        @update:model-value="set(field.key, $event)"
      />
      <BaseInput
        v-else
        :id="`${idBase}-${field.key}`"
        :maxlength="field.maxChars"
        :model-value="String(values[field.key] ?? '')"
        @update:model-value="set(field.key, $event)"
      />
    </SettingRow>
  </SettingGroup>
</template>
