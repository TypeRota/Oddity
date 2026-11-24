<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import {
  BaseModal,
  BaseSelect,
  BaseSwitch,
  DirectoryField,
  SettingRow,
} from "@host/ui-kit";
import { errorText, invoke, notify, reloadHeroBackground } from "@host/api";
import type {
  LauncherBackgroundArchive,
  LauncherBackgroundArchiveEntry,
} from "@host/api";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const archive = ref<LauncherBackgroundArchive | null>(null);
/// 归档偏好由后台保存, 界面只维护一份编辑中的副本.
const archiveOn = ref(false);
const directory = ref("");
const retention = ref("3");
const busy = ref(false);
const retentionOptions = Array.from({ length: 9 }, (_, index) => {
  const value = index + 2;
  return { label: `${value} 套`, value: String(value) };
});
let revision = 0;

// 使用中的一套是"选中的历史素材", 没有选择时就是官方当前素材.
const active = computed(
  () => archive.value?.selected ?? archive.value?.current ?? null,
);
const tiles = computed(() => archive.value?.entries ?? []);
// 索引来自更新版本的启动器时只能看不能改.
const available = computed(() => archive.value?.available !== false);

function adopt(next: LauncherBackgroundArchive) {
  archive.value = next;
  archiveOn.value = next.archive;
  directory.value = next.dir;
  retention.value = String(next.retention);
}

let lastMigration = "idle";

function report(next: LauncherBackgroundArchive) {
  const state = next.migration.state;
  // 迁移结果会随每次读取重试, 只在状态翻转时提示一次.
  if (state === "moved" && lastMigration !== "moved")
    notify(`已迁移 ${next.migration.moved} 套背景素材`);
  else if (state === "failed" && lastMigration !== "failed")
    notify(
      next.migration.reason ?? "背景素材迁移失败, 继续使用原目录",
      "error",
    );
  lastMigration = state;
}

/// 同一时刻只接受一个请求, 迟到的回复直接丢掉.
async function run(
  task: () => Promise<LauncherBackgroundArchive>,
  after?: (next: LauncherBackgroundArchive) => void,
  failed?: () => void,
) {
  const token = ++revision;
  busy.value = true;
  try {
    const next = await task();
    if (token !== revision) return;
    adopt(next);
    report(next);
    after?.(next);
  } catch (cause) {
    if (token !== revision) return;
    notify(errorText(cause), "error", 6500, { key: "launcher-background" });
    failed?.();
  } finally {
    if (token === revision) busy.value = false;
  }
}

function load() {
  void run(() => invoke("launcher_background_archive"));
}

/// 任一偏好变化都整份写回, 后台按新值归档并迁移.
function configure() {
  if (busy.value) return;
  const previous = archive.value;
  void run(
    () =>
      invoke("launcher_background_configure", {
        archive: archiveOn.value,
        dir: directory.value,
        retention: Number(retention.value),
      }),
    (next) => {
      // 开关、目录或选中的素材变了, 启动页都要按新配置重新取一次.
      if (
        next.archive !== previous?.archive ||
        next.dir !== previous?.dir ||
        next.selected !== previous?.selected
      )
        reloadHeroBackground();
    },
    // 后台回滚了这次的改动, 界面跟着回到最后一次生效的状态.
    () => {
      if (archive.value) adopt(archive.value);
    },
  );
}

function choose(entry: LauncherBackgroundArchiveEntry) {
  if (busy.value) return;
  // 点当前素材等于跟随官方更新, 不锁定这一套.
  const id = entry.id === archive.value?.current ? null : entry.id;
  void run(
    () => invoke("launcher_background_select", { id }),
    () => reloadHeroBackground(),
  );
}

watch(
  () => props.open,
  (open) => {
    revision++;
    if (open) load();
    else archive.value = null;
  },
);

onBeforeUnmount(() => {
  revision++;
});
</script>

<template>
  <BaseModal
    :open="open"
    title="背景素材"
    @update:open="emit('update:open', $event)"
  >
    <div class="background-library">
      <SettingRow
        label="归档旧素材"
        description="保存官方启动器更换下来的背景, 可在下方切换."
      >
        <BaseSwitch
          v-model="archiveOn"
          label="归档旧素材"
          :disabled="busy || !available"
          @change="configure"
        />
      </SettingRow>
      <DirectoryField
        v-if="archiveOn"
        id="set-background-dir"
        v-model="directory"
        label="归档目录"
        placeholder="留空使用默认目录"
        :disabled="busy || !available"
        @change="configure"
      />
      <SettingRow
        v-if="archiveOn"
        class="background-library__retention"
        label="保留套数"
        for="set-background-retention-trigger"
      >
        <BaseSelect
          id="set-background-retention"
          v-model="retention"
          label="保留套数"
          :options="retentionOptions"
          :disabled="busy || !available"
          @change="configure"
        />
      </SettingRow>
      <p v-if="!available" class="background-library__hint">
        归档索引来自更新版本的启动器, 请升级后继续使用.
      </p>
      <div v-if="tiles.length" class="background-library__grid">
        <button
          v-for="(tile, index) in tiles"
          :key="tile.id"
          type="button"
          class="background-library__tile"
          :class="{ 'is-active': tile.id === active }"
          :aria-label="`第 ${index + 1} 套背景素材`"
          :aria-current="tile.id === active ? 'true' : undefined"
          :disabled="!available"
          :aria-busy="busy || undefined"
          @click="choose(tile)"
        >
          <img v-if="tile.image" :src="tile.image" alt="" />
          <span v-else class="background-library__empty-tile">
            <svg aria-hidden="true"><use href="#i-monitor" /></svg>
          </span>
          <span v-if="tile.id === active" class="background-library__badge"
            >使用中</span
          >
        </button>
      </div>
      <p v-else-if="archiveOn" class="background-library__hint">
        归档目录里还没有素材, 换背景后会自动保存.
      </p>
      <p v-else class="background-library__hint">
        开启后会保留最近几套背景素材.
      </p>
    </div>
  </BaseModal>
</template>
