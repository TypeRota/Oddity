<script setup lang="ts">
import { computed } from "vue";
import { motion } from "motion-v";
import BaseButton from "./BaseButton.vue";
import BaseModal from "./BaseModal.vue";
import {
  hang,
  hangView,
  dismissHangDialog,
  hangEndGame,
  hangExportLogs,
} from "../core/controller";
import { useControlMotion } from "../composables/useControlMotion";

const { reduced } = useControlMotion();
const view = computed(() => hangView.value);
const rows = computed(() => view.value?.rows ?? []);
// 结束请求已发出时主按钮只反映进行中, 避免重复结束进程.
const ending = computed(() => view.value?.ending ?? false);
function update(open: boolean) {
  if (!open) dismissHangDialog();
}
function runPrimary() {
  if (view.value?.running) hangEndGame();
  else hangExportLogs();
}
</script>

<template>
  <BaseModal
    :open="hang.dialog"
    :title="view?.title ?? '游戏异常'"
    @update:open="update"
  >
    <div v-if="view" class="hang-dialog">
      <dl v-if="rows.length" class="hang-dialog__details">
        <motion.div
          v-for="(row, index) in rows"
          :key="row.label"
          class="hang-dialog__row"
          :initial="{ opacity: 0, y: reduced ? 0 : 6 }"
          :animate="{ opacity: 1, y: 0 }"
          :transition="{ delay: reduced ? 0 : index * 0.05, duration: 0.2 }"
        >
          <dt>{{ row.label }}</dt>
          <dd>{{ row.value }}</dd>
        </motion.div>
      </dl>
      <div class="hang-dialog__actions">
        <BaseButton
          variant="primary"
          :icon="view.primaryIcon"
          :busy="ending"
          @click="runPrimary"
          >{{ view.primary }}</BaseButton
        >
        <BaseButton
          variant="ghost"
          :disabled="ending"
          @click="dismissHangDialog"
          >{{ view.secondary }}</BaseButton
        >
      </div>
    </div>
  </BaseModal>
</template>

<style scoped>
.hang-dialog {
  display: grid;
  gap: 14px;
}
.hang-dialog__details {
  display: grid;
  gap: 8px;
  margin: 0;
}
.hang-dialog__row {
  display: flex;
  justify-content: space-between;
  gap: 16px;
}
.hang-dialog__row dt {
  flex: none;
  color: var(--muted);
}
.hang-dialog__row dd {
  margin: 0;
  overflow-wrap: anywhere;
}
.hang-dialog__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
</style>
