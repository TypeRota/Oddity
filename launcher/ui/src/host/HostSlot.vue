<script setup lang="ts">
import HostContribution from "./HostContribution.vue";
import type { SlotContribution } from "./contributions";
import type { PageSlot } from "./pages";

withDefaults(
  defineProps<{
    contributions: readonly SlotContribution[];
    loading?: boolean;
    area?: PageSlot;
  }>(),
  { loading: false, area: "drawer" },
);
</script>

<template>
  <div
    v-if="loading"
    class="host-slot host-slot--loading"
    role="status"
    aria-label="正在加载页面"
  >
    <span class="host-slot__skeleton" aria-hidden="true" />
    <span class="host-slot__skeleton" aria-hidden="true" />
  </div>
  <template v-else>
    <HostContribution
      v-for="contribution in contributions"
      :id="contribution.id"
      :area="area"
      :key="contribution.id"
      :page="contribution.component"
    />
  </template>
</template>

<style scoped>
.host-slot {
  position: absolute;
  inset: 0;
  display: grid;
  align-content: start;
  gap: 12px;
  padding: 32px;
  pointer-events: none;
}
.host-slot__skeleton {
  height: 96px;
  border: 1px solid var(--glass-line);
  border-radius: var(--r-lg);
  background: var(--glass-card);
}
</style>
