<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch } from "vue";
import {
  AnimatePresence,
  BaseButton,
  BaseModal,
  createTimeline,
  motion,
  useControlMotion,
} from "@host/ui-kit";
import { update } from "@host/api";
import type { OptionalResource } from "@host/api";
const {
  state,
  formatSize,
  loadResourceCatalog,
  resourceCatalogMatches,
  downloadBusy,
  downloadOptionalResource,
  resourceDownloadsStarting,
  removeOptionalResource,
  gameSettings,
} = update;

const { reduced, surface } = useControlMotion();
const bodyId = useId();
const expanded = ref(false);
const summary = ref<HTMLElement | null>(null);
const displayedBytes = ref(0);
let timeline: ReturnType<typeof createTimeline> | undefined;

const active = computed(() => state.page === "nte-update");
const interactionDisabled = computed(
  () => downloadBusy() || state.resourcesSaving || state.game !== "idle",
);
const catalog = computed(() =>
  resourceCatalogMatches.value ? state.resourceCatalog : null,
);
const resources = computed(() => catalog.value?.resources ?? []);
const installed = computed(() =>
  resources.value.filter((resource) => resource.state === "installed"),
);
const installedBytes = computed(() =>
  installed.value.reduce((total, resource) => total + resource.size, 0),
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
  const count = `${installed.value.length}/${resources.value.length} 项`;
  const bytes = Math.round(displayedBytes.value);
  return bytes > 0 ? `${count} · ${formatSize(bytes)}` : count;
});
const stateLabel = (resource: OptionalResource) =>
  resource.state === "installed"
    ? resource.needsUpdate
      ? "待更新"
      : "已安装"
    : resource.state === "partial"
      ? "部分安装"
      : "";
const removal = ref<{ tag: string; name: string; preload: boolean } | null>(
  null,
);
const resourceTasks = computed(() =>
  state.download?.mode === "resources"
    ? state.download.resourceTasks || {}
    : {},
);
const downloadDisabled = computed(
  () =>
    state.resourcesSaving ||
    state.resourcesLoading ||
    state.game !== "idle" ||
    (downloadBusy() && state.download?.mode !== "resources"),
);
const rowBusy = (tag: string) =>
  resourceDownloadsStarting.has(tag) ||
  ["checking", "downloading"].includes(resourceTasks.value[tag]?.phase || "");
const downloadLabel = (resource: OptionalResource) => {
  const task = resourceTasks.value[resource.tag];
  if (resourceDownloadsStarting.has(resource.tag) || task?.phase === "checking")
    return "检查中";
  if (task?.phase === "downloading")
    return task.totalBytes
      ? `${Math.min(99, Math.floor((task.downloadedBytes / task.totalBytes) * 100))}%`
      : "下载中";
  if (task?.phase === "failed") return "重试";
  return resource.state === "installed"
    ? "更新"
    : resource.state === "partial"
      ? "继续下载"
      : "下载";
};
function closeRemoval(open: boolean) {
  if (!open && !state.resourcesSaving) removal.value = null;
}
async function confirmRemoval() {
  const target = removal.value;
  if (!target) return;
  if (await removeOptionalResource(target.tag, target.preload))
    removal.value = null;
}

watch(
  installedBytes,
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
  () => {
    const game = gameSettings(state.bootstrap?.settings);
    return [
      active.value,
      Boolean(state.bootstrap?.launcherRoot),
      game.distribution,
      state.bootstrap?.installRoot,
      JSON.stringify(game.installRoots ?? {}),
      game.preloadRoot,
    ];
  },
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
          <span id="resource-options-title">语音资源管理</span>
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
              语音资源读取失败.
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
                  <span class="resource-row__tag">{{ resource.name }}</span>
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
                <BaseButton
                  v-if="resource.state !== 'absent'"
                  :id="'resource-remove-' + resource.tag"
                  size="sm"
                  variant="ghost"
                  :disabled="interactionDisabled || state.resourcesLoading"
                  @click="
                    removal = {
                      tag: resource.tag,
                      name: resource.name,
                      preload: false,
                    }
                  "
                  >卸载</BaseButton
                >
                <BaseButton
                  v-if="(catalog?.preloadResourceBytes[resource.tag] || 0) > 0"
                  :id="'resource-preload-remove-' + resource.tag"
                  size="sm"
                  variant="ghost"
                  :disabled="interactionDisabled || state.resourcesLoading"
                  @click="
                    removal = {
                      tag: resource.tag,
                      name: resource.name,
                      preload: true,
                    }
                  "
                  >删除预下载</BaseButton
                >
                <BaseButton
                  :id="'resource-download-' + resource.tag"
                  size="sm"
                  variant="ghost"
                  :busy="rowBusy(resource.tag)"
                  :disabled="
                    downloadDisabled ||
                    (resource.state === 'installed' &&
                      !resource.needsUpdate &&
                      resourceTasks[resource.tag]?.phase !== 'failed')
                  "
                  :title="
                    resourceTasks[resource.tag]?.error ||
                    (resource.state === 'installed' && !resource.needsUpdate
                      ? '已是最新'
                      : undefined)
                  "
                  @click="downloadOptionalResource(resource.tag)"
                  >{{ downloadLabel(resource) }}</BaseButton
                >
              </motion.li>
            </ul>
          </div>
        </motion.div>
      </AnimatePresence>
    </motion.section>
  </AnimatePresence>
  <BaseModal
    :open="Boolean(removal)"
    :title="removal?.preload ? '删除语音预下载' : '卸载语音资源'"
    @update:open="closeRemoval"
  >
    <p v-if="removal?.preload">
      删除 {{ removal.name }} 的预下载缓存? 已安装的语音仍会随版本更新预下载.
    </p>
    <p v-else>
      卸载 {{ removal?.name }}? 如游戏正在使用该语种,
      请在游戏内切换至其他已安装语种.
    </p>
    <div class="update-actions">
      <BaseButton
        id="voice-resource-remove-confirm"
        variant="danger"
        :busy="state.resourcesSaving"
        @click="confirmRemoval"
        >{{ removal?.preload ? "删除" : "卸载" }}</BaseButton
      >
      <BaseButton
        variant="ghost"
        :disabled="state.resourcesSaving"
        @click="removal = null"
        >取消</BaseButton
      >
    </div>
  </BaseModal>
</template>
