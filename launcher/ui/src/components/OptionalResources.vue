<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch } from "vue";
import { AnimatePresence, motion } from "motion-v";
import { createTimeline } from "animejs";
import BaseButton from "./BaseButton.vue";
import BaseSwitch from "./BaseSwitch.vue";
import { useControlMotion } from "../composables/useControlMotion";
import {
  state,
  formatSize,
  loadResourceCatalog,
  selectOptionalResource,
  toggleAllOptionalResources,
  resourceCatalogMatches,
  downloadBusy,
} from "../core/controller";
import type { OptionalResource } from "../types/launcher";

const { reduced, surface } = useControlMotion();
const bodyId = useId();
const expanded = ref(false);
const summary = ref<HTMLElement | null>(null);
const displayedBytes = ref(0);
let timeline: ReturnType<typeof createTimeline> | undefined;

const active = computed(() => state.page === "update");
const interactionDisabled = computed(
  () => downloadBusy() || state.game !== "idle",
);
const catalog = computed(() =>
  resourceCatalogMatches.value ? state.resourceCatalog : null,
);
const resources = computed(() => catalog.value?.resources ?? []);
const selectable = computed(() =>
  resources.value.filter((resource) => resource.state === "absent"),
);
const selected = computed(() =>
  resources.value.filter((resource) => resource.selected),
);
const selectedBytes = computed(() =>
  selected.value.reduce((total, resource) => total + resource.size, 0),
);
const allSelected = computed(
  () =>
    selectable.value.length > 0 &&
    selectable.value.every((resource) => resource.selected),
);
// 地区或目录改动后先保留占位, 等保存完成再读取新目录.
const pendingReload = computed(
  () => Boolean(state.resourceCatalog) && !resourceCatalogMatches.value,
);
const visible = computed(
  () =>
    active.value &&
    Boolean(state.bootstrap?.launcherRoot) &&
    (state.resourcesLoading ||
      Boolean(state.resourcesError) ||
      pendingReload.value ||
      resources.value.length > 0),
);
const summaryText = computed(() => {
  if (!catalog.value) return state.resourcesError ? "读取失败" : "读取中";
  const count = `${selected.value.length}/${resources.value.length} 项`;
  const bytes = Math.round(displayedBytes.value);
  return bytes > 0 ? `${count} · ${formatSize(bytes)}` : count;
});
const stateLabel = (resource: OptionalResource) =>
  resource.state === "installed"
    ? "已安装"
    : resource.state === "partial"
      ? "部分安装"
      : "";
const toggleAllLabel = computed(() =>
  allSelected.value ? "取消全选" : "全选",
);

watch(
  selectedBytes,
  (value, previous) => {
    if (reduced.value) {
      timeline?.cancel();
      displayedBytes.value = value;
      return;
    }
    timeline?.cancel();
    const counter = { value: previous ?? 0 };
    displayedBytes.value = counter.value;
    timeline = createTimeline({ defaults: { ease: "outQuart" } });
    timeline.add(
      counter,
      {
        value,
        duration: 460,
        onUpdate: () => {
          displayedBytes.value = counter.value;
        },
      },
      0,
    );
    if (summary.value) {
      timeline.add(
        summary.value,
        {
          scale: [1, 1.045, 1],
          duration: 460,
        },
        0,
      );
    }
  },
  { immediate: true },
);

watch(
  () => [
    active.value,
    Boolean(state.bootstrap?.launcherRoot),
    state.bootstrap?.settings.distribution,
    state.bootstrap?.installRoot,
    state.bootstrap?.settings.installRoot,
    state.bootstrap?.settings.globalInstallRoot,
    state.bootstrap?.settings.preloadRoot,
  ],
  () => {
    if (!active.value || !state.bootstrap?.launcherRoot) return;
    if (
      state.resourcesLoading ||
      downloadBusy() ||
      resourceCatalogMatches.value
    )
      return;
    void loadResourceCatalog();
  },
  { immediate: true },
);

watch(
  () => state.download?.phase,
  (phase, previous) => {
    // 正式安装会改变可选资源的文件状态; 预下载只写缓存, 目录不变.
    if (previous === "game" && phase === "ready")
      void loadResourceCatalog(true);
  },
);

onBeforeUnmount(() => timeline?.cancel());
</script>

<template>
  <AnimatePresence :initial="false">
    <motion.section
      v-if="visible"
      class="resource-options"
      aria-labelledby="resource-options-title"
      :initial="reduced ? false : { opacity: 0, y: 8 }"
      :animate="{ opacity: 1, y: 0 }"
      :exit="{ opacity: 0 }"
      :transition="reduced ? { duration: 0 } : surface"
    >
      <div class="resource-options__head">
        <button
          type="button"
          class="resource-options__toggle"
          :aria-expanded="expanded"
          :aria-controls="bodyId"
          @click="expanded = !expanded"
        >
          <span id="resource-options-title">可选资源</span>
          <span ref="summary" class="resource-options__summary">{{
            summaryText
          }}</span>
          <motion.svg
            class="icon"
            viewBox="0 0 256 256"
            aria-hidden="true"
            :initial="false"
            :animate="{ rotate: expanded ? 180 : 0 }"
            :transition="reduced ? { duration: 0 } : surface"
            ><use href="#i-caret-down"
          /></motion.svg>
        </button>
        <BaseButton
          v-if="selectable.length"
          id="optional-resources-all"
          size="sm"
          variant="ghost"
          :disabled="interactionDisabled || state.resourcesLoading"
          @click="toggleAllOptionalResources"
          >{{ toggleAllLabel }}</BaseButton
        >
      </div>
      <AnimatePresence :initial="false">
        <motion.div
          v-if="expanded"
          :id="bodyId"
          class="resource-options__clip"
          :initial="reduced ? false : { height: 0, opacity: 0 }"
          :animate="{ height: 'auto', opacity: 1 }"
          :exit="{ height: 0, opacity: 0 }"
          :transition="reduced ? { duration: 0 } : surface"
        >
          <div class="resource-options__body">
            <p
              v-if="state.resourcesError && !catalog"
              class="resource-options__hint"
            >
              可选资源读取失败.
              <BaseButton
                id="optional-resources-retry"
                size="sm"
                variant="ghost"
                @click="loadResourceCatalog(true)"
                >重试</BaseButton
              >
            </p>
            <div
              v-else-if="(state.resourcesLoading || pendingReload) && !catalog"
              class="resource-options__skeleton"
              aria-hidden="true"
            >
              <motion.span
                v-for="index in 3"
                :key="index"
                class="resource-options__skeleton-row"
                :initial="false"
                :animate="
                  reduced ? { opacity: 0.7 } : { opacity: [0.45, 0.85, 0.45] }
                "
                :transition="
                  reduced
                    ? { duration: 0 }
                    : {
                        duration: 1.3,
                        repeat: Infinity,
                        ease: 'easeInOut',
                        delay: index * 0.12,
                      }
                "
              />
            </div>
            <ul v-else class="resource-list">
              <motion.li
                v-for="(resource, index) in resources"
                :key="resource.tag"
                class="resource-row"
                :class="{ 'is-locked': resource.state !== 'absent' }"
                :initial="reduced ? false : { opacity: 0, y: 6 }"
                :animate="{ opacity: 1, y: 0 }"
                :transition="
                  reduced
                    ? { duration: 0 }
                    : { ...surface, delay: index * 0.03 }
                "
              >
                <span class="resource-row__copy">
                  <span class="resource-row__tag">{{ resource.tag }}</span>
                  <span class="resource-row__meta"
                    >{{ resource.version }} ·
                    {{ formatSize(resource.size) }}</span
                  >
                </span>
                <span
                  v-if="resource.state !== 'absent'"
                  class="resource-row__state"
                  :class="'is-' + resource.state"
                  >{{ stateLabel(resource) }}</span
                >
                <span
                  class="resource-row__switch"
                  :title="
                    resource.state === 'absent'
                      ? undefined
                      : '已安装的资源随更新维护'
                  "
                >
                  <BaseSwitch
                    :id="'optional-resource-' + resource.tag"
                    :label="'选择 ' + resource.tag"
                    :model-value="resource.selected"
                    :disabled="
                      interactionDisabled ||
                      state.resourcesLoading ||
                      resource.state !== 'absent'
                    "
                    @update:model-value="
                      selectOptionalResource(resource.tag, $event)
                    "
                  />
                </span>
              </motion.li>
            </ul>
          </div>
        </motion.div>
      </AnimatePresence>
    </motion.section>
  </AnimatePresence>
</template>
