<script setup lang="ts">
import { computed } from "vue";
import { contributionActive, type CapabilityRequirement } from "./capability";
import { factsFrom } from "./facts";
import { state } from "../core/controller";

/**
 * 能力要求不满足时, 插槽内容不进入 DOM.
 *
 * 判定与贡献挂载共用 contributionActive; 声明方只给出要求, 不自己求值,
 * 也不保留置灰占位.
 */
const props = defineProps<{
  requires: readonly CapabilityRequirement[];
}>();

const active = computed(() =>
  contributionActive(props.requires, factsFrom(state.bootstrap)),
);
</script>

<template>
  <slot v-if="active" />
</template>
