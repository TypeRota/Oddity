<script setup lang="ts">
import { motion, useAnimate } from "motion-v";
import { computed, nextTick, reactive, ref, watch, type Component } from "vue";
import {
  state,
  showPage,
  ui,
  form,
  applyTheme,
  applyReduceMotion,
  applyHeroMedia,
  queueSettingsSave,
  redetect,
  runtimeEnvironmentBusy,
  runtimePathsLocked,
} from "../core/controller";
import {
  settingsEntries,
  settingsNavigation as navigation,
  type SettingsCategory,
  type SettingsEntry,
} from "../core/settingsCatalog";
import {
  selectSettingsSections,
  settingsSearchEntries,
  settingsSections,
  type SettingsSearchEntry,
} from "../host/settings";
import { factsFrom } from "../host/facts";
import { useControlMotion } from "../composables/useControlMotion";
import { useScrubNavigation } from "../composables/useScrubNavigation";
import RuntimeEnvironments from "../components/RuntimeEnvironments.vue";
import BaseSelect from "../components/BaseSelect.vue";
import BaseSwitch from "../components/BaseSwitch.vue";
import BaseButton from "../components/BaseButton.vue";
import BaseInput from "../components/BaseInput.vue";
import SettingGroup from "../components/settings/SettingGroup.vue";
import SettingRow from "../components/settings/SettingRow.vue";
import LogExport from "../components/LogExport.vue";
import EnvironmentSnapshots from "../components/EnvironmentSnapshots.vue";
import AudioSettings from "../components/AudioSettings.vue";
import GamepadSettings from "../components/GamepadSettings.vue";
import BaseDisclosure from "../components/BaseDisclosure.vue";
import type { IntegrityState } from "../types/launcher";

const root = ref<HTMLElement | null>(null);
const { control, surface, reduced } = useControlMotion();
const [, animate] = useAnimate();
// 分类面板按首次访问挂载, 避免启动时构建全部设置项。
const visited = reactive(new Set<SettingsCategory>());
const snapshotRevision = ref(0);
watch(
  () => [state.page, navigation.category] as const,
  ([page, category]) => {
    if (page === "settings") visited.add(category);
    if (page === "settings" && category === "maintenance") {
      snapshotRevision.value += 1;
    }
  },
  { immediate: true },
);
// 事实随 bootstrap 更新; 分区贡献按当前游戏过滤.
const facts = computed(() => factsFrom(state.bootstrap));
// 分区列表 = 宿主通用分区 + 当前可见的贡献分区, 导航与搜索共用这一份.
const sections = computed(() => settingsSections(facts.value));
const contributedSections = computed(() => selectSettingsSections(facts.value));
const sectionComponents = computed(
  () =>
    Object.fromEntries(
      contributedSections.value.map((section) => [
        section.id,
        section.component,
      ]),
    ) as Record<string, Component>,
);
const searchEntries = computed(() =>
  settingsSearchEntries(facts.value, contributedSections.value),
);
watch(sections, (list) => {
  // 当前分区随游戏切换消失时回到第一个可见分区, 避免停在空面板.
  if (!list.some((section) => section.id === navigation.category)) {
    navigation.category = list[0]?.id ?? "appearance";
  }
});
const themeOptions = [
  { label: "亮色", value: "light" },
  { label: "暗色", value: "dark" },
];
const mediaOptions = [
  { label: "动态视频", value: "video" },
  { label: "静态照片", value: "photo" },
];
function updateSnapshotSuccessMinutes(value: number) {
  if (form.snapshotSuccessMinutes !== value) {
    form.snapshotSuccessMinutes = value;
    queueSettingsSave("host", 0);
  }
}
const searching = computed(() => !!navigation.query.trim());
function sectionLabel(category: string): string {
  return sections.value.find((section) => section.id === category)?.label ?? "";
}
const results = computed(() => {
  const words = navigation.query.trim().toLocaleLowerCase().split(/\s+/);
  return searchEntries.value.filter((entry) =>
    words.every((word) =>
      `${entry.label} ${entry.description} ${entry.keywords} ${sectionLabel(entry.category)}`
        .toLocaleLowerCase()
        .includes(word),
    ),
  );
});
const entry = (id: SettingsEntry["id"]) =>
  settingsEntries.find((entry) => entry.id === id)!;
function chooseCategory(category: SettingsCategory) {
  navigation.category = category;
  navigation.query = "";
}
const categoriesNav = ref<HTMLElement | null>(null);
const categoryScrubbing = ref(false);
const {
  down: categoryDown,
  lost: categoryLost,
  suppressed: categorySuppressed,
} = useScrubNavigation({
  root: categoriesNav,
  item: "[data-category]",
  value: (element) => element.dataset.category,
  active: () => navigation.category,
  select: (value) => chooseCategory(value as SettingsCategory),
  axis: "auto",
  enabled: () => !searching.value,
  focusOnFinish: true,
  scrollIntoView: true,
  scrubbing: categoryScrubbing,
});
function clickCategory(event: MouseEvent, category: SettingsCategory) {
  if (categorySuppressed(event)) return;
  chooseCategory(category);
}
async function locate(item: SettingsSearchEntry) {
  visited.add(item.category);
  chooseCategory(item.category);
  await nextTick();
  const target = root.value!.querySelector<HTMLElement>(
    `[data-setting="${item.id}"]`,
  )!;
  target.scrollIntoView({ block: "center", behavior: "instant" });
  const control = target.querySelector<HTMLElement>(
    "button:not(:disabled), input:not(:disabled):not([hidden])",
  );
  (control ?? target).focus({ preventScroll: true });
  void animate(
    target,
    { backgroundColor: ["var(--signal-dim)", "rgba(0, 0, 0, 0)"] },
    { duration: reduced.value ? 0 : 1.1 },
  );
}
function categoryKeys(event: KeyboardEvent) {
  const list = sections.value;
  const index = list.findIndex(
    (category) => category.id === navigation.category,
  );
  const next =
    event.key === "ArrowDown" || event.key === "ArrowRight"
      ? (index + 1) % list.length
      : event.key === "ArrowUp" || event.key === "ArrowLeft"
        ? (index - 1 + list.length) % list.length
        : event.key === "Home"
          ? 0
          : event.key === "End"
            ? list.length - 1
            : -1;
  if (next < 0) return;
  event.preventDefault();
  chooseCategory(list[next].id);
  root.value
    ?.querySelector<HTMLElement>(`[data-category="${navigation.category}"]`)
    ?.focus();
}
function searchKeys(event: KeyboardEvent) {
  if (event.key === "Escape" && navigation.query) {
    event.preventDefault();
    event.stopPropagation();
    navigation.query = "";
    root.value?.querySelector<HTMLInputElement>("#settings-search")?.focus();
  } else if (
    event.key === "ArrowDown" &&
    searching.value &&
    event.target === root.value?.querySelector("#settings-search")
  ) {
    event.preventDefault();
    root.value?.querySelector<HTMLElement>(".settings-result")?.focus();
  }
}
function resultKeys(event: KeyboardEvent) {
  const buttons = [
    ...root.value!.querySelectorAll<HTMLButtonElement>(".settings-result"),
  ];
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    const next = index + (event.key === "ArrowDown" ? 1 : -1);
    if (next < 0)
      root.value?.querySelector<HTMLInputElement>("#settings-search")?.focus();
    else buttons[Math.min(next, buttons.length - 1)]?.focus();
  }
}
const isActive = computed(() => state.page === "settings");
const integrity = computed(() => state.bootstrap?.integrity ?? null);
const integrityChanged = computed(() => integrity.value?.changed ?? []);
const integrityTotal = computed(() => integrity.value?.changedTotal ?? 0);
const integrityTruncated = computed(() => {
  const status = integrity.value;
  return Boolean(status && status.changedTotal > status.changed.length);
});
const integrityLabels: Record<IntegrityState, string> = {
  checking: "正在检查",
  verified: "在线已验证",
  baseline: "离线基线通过",
  unverified: "未验证",
  changed: "已修改",
  unsupported: "不支持校验",
};
const integrityLabel = computed(() =>
  integrity.value ? integrityLabels[integrity.value.state] : "未检查",
);
const integrityBad = computed(() => integrity.value?.state === "changed");
const integrityDot = computed(() => {
  const value = integrity.value?.state;
  if (value === "verified" || value === "baseline") return "is-ok";
  if (value === "changed") return "is-bad";
  return "";
});
function formatIntegrityTime(value: number) {
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}
const integritySummary = computed(() => {
  const status = integrity.value;
  if (!status) return "等待检查";
  const parts: string[] = [];
  switch (status.state) {
    case "checking":
      parts.push(status.checkedAt ? "正在重新检查" : "正在检查");
      break;
    case "verified":
      parts.push("官方清单比对通过");
      break;
    case "baseline":
      parts.push("联网失败, 已按内置清单比对");
      break;
    case "unverified":
      parts.push("本次未能完成校验");
      break;
    case "changed":
      parts.push(
        status.source === "bundled"
          ? `发现 ${status.changedTotal} 个文件与内置清单不一致`
          : `发现 ${status.changedTotal} 个文件与官方发布不一致`,
      );
      break;
    case "unsupported":
      parts.push("当前安装方式不支持校验");
      break;
  }
  if (status.source === "online") parts.push("来源: 在线官方清单");
  if (status.source === "bundled") parts.push("来源: 内置清单");
  if (status.checkedAt)
    parts.push(`上次检查: ${formatIntegrityTime(status.checkedAt)}`);
  return parts.join(" · ");
});
</script>

<template>
  <motion.section
    id="page-settings"
    class="page settings-page"
    :class="{ 'is-active': isActive }"
    :inert="!isActive"
    :aria-hidden="!isActive || undefined"
    :initial="false"
    :animate="{
      opacity: isActive ? 1 : 0,
      x: isActive ? 0 : 22,
      visibility: isActive ? 'visible' : 'hidden',
    }"
    :transition="surface"
    aria-labelledby="settings-title"
  >
    <div ref="root" class="settings-workspace" @keydown="searchKeys">
      <header class="page-head settings-head">
        <div class="page-head__copy">
          <h2 id="settings-title" tabindex="-1">设置</h2>
        </div>
        <div class="settings-head__tools">
          <span class="settings-save" role="status" aria-live="polite">{{
            ui.saveState === "saving"
              ? "正在保存"
              : ui.saveState === "saved"
                ? "已保存"
                : ui.saveState === "error"
                  ? "保存失败"
                  : ""
          }}</span>
          <BaseButton
            id="settings-redetect"
            data-pad-primary
            data-pad-action="重新检测"
            variant="ghost"
            icon="i-arrows-clockwise"
            :busy="ui.redetecting"
            @click="redetect"
            >重新检测</BaseButton
          >
        </div>
      </header>
      <div class="settings-search">
        <svg
          class="settings-search__icon"
          viewBox="0 0 24 24"
          fill="none"
          aria-hidden="true"
        >
          <circle
            cx="10.5"
            cy="10.5"
            r="6.5"
            stroke="currentColor"
            stroke-width="1.6"
          />
          <path
            d="m15.5 15.5 4.5 4.5"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
          />
        </svg>
        <BaseInput
          id="settings-search"
          v-model="navigation.query"
          type="search"
          aria-label="搜索设置"
          placeholder="搜索设置"
          autocomplete="off"
          :aria-controls="searching ? 'settings-results' : undefined"
        />
        <BaseButton
          v-if="navigation.query"
          class="settings-search__clear"
          variant="ghost"
          size="sm"
          icon="i-x"
          icon-only
          aria-label="清除搜索"
          @click="navigation.query = ''"
        />
      </div>
      <div class="settings-frame">
        <nav
          ref="categoriesNav"
          class="settings-categories"
          :class="{ 'is-scrubbable': !searching }"
          aria-label="设置分类"
          :data-scrubbing="categoryScrubbing"
          @keydown="categoryKeys"
          @pointerdown="categoryDown"
          @lostpointercapture="categoryLost"
        >
          <motion.button
            v-for="category in sections"
            :key="category.id"
            type="button"
            class="settings-category category-tab"
            :data-category="category.id"
            :aria-current="
              !searching && navigation.category === category.id
                ? 'true'
                : undefined
            "
            :tabindex="navigation.category === category.id ? 0 : -1"
            :while-press="reduced ? undefined : { scale: 0.97 }"
            :transition="control"
            :aria-controls="`settings-panel-${category.id}`"
            @click="clickCategory($event, category.id)"
          >
            <motion.span
              v-if="navigation.category === category.id"
              class="settings-category__focus category-tab__selection"
              layout-id="settings-category-focus"
              :layout-dependency="navigation.category"
              :animate="{ opacity: searching ? 0.35 : 1 }"
              :transition="control"
            />
            <svg class="icon" aria-hidden="true">
              <use :href="`#${category.icon}`" /></svg
            ><span>{{ category.label }}</span>
          </motion.button>
        </nav>
        <form
          id="settings-form"
          class="settings-stage"
          :data-save-state="ui.saveState"
          @submit.prevent
        >
          <fieldset class="settings-fields" :disabled="runtimeEnvironmentBusy">
            <motion.div
              v-for="category in sections"
              :key="category.id"
              :id="`settings-panel-${category.id}`"
              class="settings-panel"
              :class="{
                'is-current': !searching && navigation.category === category.id,
              }"
              role="region"
              :aria-label="category.label"
              :aria-hidden="
                searching || navigation.category !== category.id || undefined
              "
              :inert="searching || navigation.category !== category.id"
              :style="{
                pointerEvents:
                  !searching && navigation.category === category.id
                    ? 'auto'
                    : 'none',
                zIndex:
                  !searching && navigation.category === category.id ? 1 : 0,
              }"
              :initial="false"
              :animate="{
                opacity:
                  !searching && navigation.category === category.id ? 1 : 0,
                x: !searching && navigation.category === category.id ? 0 : 12,
              }"
              :transition="surface"
            >
              <AudioSettings
                v-if="visited.has(category.id) && category.id === 'audio'"
                :active="
                  navigation.category === 'audio' && state.page === 'settings'
                "
              />
              <GamepadSettings
                v-else-if="
                  visited.has(category.id) && category.id === 'gamepad'
                "
              />
              <template
                v-else-if="
                  visited.has(category.id) && category.id === 'appearance'
                "
              >
                <SettingGroup title="外观">
                  <SettingRow
                    data-setting="theme"
                    tabindex="-1"
                    :label="entry('theme').label"
                    :description="entry('theme').description"
                    for="set-theme-trigger"
                    v-slot="{ describedby }"
                  >
                    <BaseSelect
                      id="set-theme"
                      label="主题"
                      v-model="form.theme"
                      :options="themeOptions"
                      :aria-describedby="describedby"
                      @change="
                        applyTheme(form.theme);
                        queueSettingsSave('host', 0);
                      "
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="media"
                    tabindex="-1"
                    :label="entry('media').label"
                    :description="entry('media').description"
                    for="set-hero-media-trigger"
                    v-slot="{ describedby }"
                  >
                    <BaseSelect
                      id="set-hero-media"
                      label="主页主视觉"
                      v-model="form.heroMedia"
                      :options="mediaOptions"
                      :aria-describedby="describedby"
                      @change="
                        applyHeroMedia(form.heroMedia);
                        queueSettingsSave('host', 0);
                      "
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="adaptive-theme"
                    tabindex="-1"
                    :label="entry('adaptive-theme').label"
                    :description="entry('adaptive-theme').description"
                    for="set-adaptive-theme"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-adaptive-theme"
                      v-model="form.adaptiveTheme"
                      :label="entry('adaptive-theme').label"
                      :describedby="describedby"
                      @change="queueSettingsSave('host', 0)"
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="relative-heatmap"
                    tabindex="-1"
                    :label="entry('relative-heatmap').label"
                    :description="entry('relative-heatmap').description"
                    for="set-relative-heatmap"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-relative-heatmap"
                      v-model="form.relativeHeatmap"
                      :label="entry('relative-heatmap').label"
                      :describedby="describedby"
                      @change="queueSettingsSave('host', 0)"
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="bulletins"
                    tabindex="-1"
                    :label="entry('bulletins').label"
                    :description="entry('bulletins').description"
                    for="set-stack-bulletins"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-stack-bulletins"
                      v-model="form.stackBulletins"
                      :label="entry('bulletins').label"
                      :describedby="describedby"
                      @change="queueSettingsSave('host', 0)"
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="motion"
                    tabindex="-1"
                    :label="entry('motion').label"
                    :description="entry('motion').description"
                    for="set-motion"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-motion"
                      v-model="form.reduceMotion"
                      label="减少动效"
                      :describedby="describedby"
                      @change="
                        applyReduceMotion(form.reduceMotion);
                        applyHeroMedia(form.heroMedia);
                        queueSettingsSave('host', 0);
                      "
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="focus-pause"
                    tabindex="-1"
                    :label="entry('focus-pause').label"
                    :description="entry('focus-pause').description"
                    for="set-pause-hero-on-blur"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-pause-hero-on-blur"
                      v-model="form.pauseHeroOnBlur"
                      :label="entry('focus-pause').label"
                      :describedby="describedby"
                      @change="
                        applyHeroMedia(form.heroMedia);
                        queueSettingsSave('host', 0);
                      "
                    />
                  </SettingRow>
                </SettingGroup>
                <SettingGroup title="窗口行为">
                  <SettingRow
                    data-setting="tray"
                    tabindex="-1"
                    :label="entry('tray').label"
                    :description="entry('tray').description"
                    for="set-minimize-to-tray"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-minimize-to-tray"
                      v-model="form.minimizeToTrayOnClose"
                      :label="entry('tray').label"
                      :describedby="describedby"
                      @change="queueSettingsSave('host', 0)"
                    />
                  </SettingRow>
                  <SettingRow
                    data-setting="return"
                    tabindex="-1"
                    :label="entry('return').label"
                    :description="entry('return').description"
                    for="set-show-after-game-exit"
                    v-slot="{ describedby }"
                  >
                    <BaseSwitch
                      id="set-show-after-game-exit"
                      v-model="form.showLauncherAfterGameExit"
                      :label="entry('return').label"
                      :describedby="describedby"
                      @change="queueSettingsSave('host', 0)"
                    />
                  </SettingRow>
                </SettingGroup>
              </template>
              <div
                v-else-if="
                  visited.has(category.id) && category.id === 'runtime'
                "
                class="runtime-settings"
                data-setting="runtime"
                tabindex="-1"
              >
                <SettingGroup title="启动环境向导">
                  <BaseButton
                    icon="i-terminal-window"
                    :disabled="runtimePathsLocked"
                    @click="showPage('runtime-setup')"
                    >打开向导</BaseButton
                  >
                </SettingGroup>
                <RuntimeEnvironments />
              </div>
              <component
                v-else-if="
                  visited.has(category.id) && sectionComponents[category.id]
                "
                :is="sectionComponents[category.id]"
              />
              <template v-else-if="visited.has(category.id)">
                <div data-setting="snapshots" tabindex="-1">
                  <EnvironmentSnapshots
                    :revision="snapshotRevision"
                    :success-minutes="form.snapshotSuccessMinutes"
                    @update:success-minutes="updateSnapshotSuccessMinutes"
                  />
                </div>
                <div data-setting="logs" tabindex="-1"><LogExport /></div>
                <div data-setting="diagnostics" tabindex="-1">
                  <SettingGroup title="运行诊断"
                    ><div id="settings-checks" class="checks">
                      <div
                        data-setting="integrity"
                        tabindex="-1"
                        class="check-row"
                      >
                        <i
                          class="check-dot"
                          :class="integrityDot"
                          aria-hidden="true"
                        />
                        <div class="check-copy">
                          <strong>Oddity 安全性验证</strong
                          ><span>{{ integritySummary }}</span>
                        </div>
                        <span
                          class="settings-check-status"
                          :class="{ 'is-bad': integrityBad }"
                          >{{ integrityLabel }}</span
                        >
                      </div>
                      <BaseDisclosure
                        v-if="integrityChanged.length"
                        title="查看变更文件"
                      >
                        <ul class="integrity-files">
                          <li v-for="file in integrityChanged" :key="file">
                            {{ file }}
                          </li>
                        </ul>
                        <p
                          v-if="integrityTruncated"
                          class="integrity-files__hint"
                        >
                          共 {{ integrityTotal }} 项, 仅显示前
                          {{ integrityChanged.length }} 项.
                        </p>
                      </BaseDisclosure>
                      <div
                        v-for="(item, index) in state.bootstrap?.checks ?? []"
                        :key="index"
                        class="check-row"
                      >
                        <i
                          class="check-dot"
                          :class="item.ok ? 'is-ok' : 'is-bad'"
                          aria-hidden="true"
                        />
                        <div class="check-copy">
                          <strong>{{ item.label }}</strong
                          ><span>{{ item.detail }}</span>
                        </div>
                        <span
                          class="settings-check-status"
                          :class="{ 'is-bad': !item.ok }"
                          >{{ item.ok ? "正常" : "待处理" }}</span
                        >
                      </div>
                    </div></SettingGroup
                  >
                </div>
              </template>
            </motion.div>
          </fieldset>
          <div
            v-if="searching"
            id="settings-results"
            class="settings-search-results"
            @keydown="resultKeys"
          >
            <p class="settings-results-count" role="status">
              {{
                results.length
                  ? `${results.length} 项匹配设置`
                  : `未找到“${navigation.query.trim()}”`
              }}
            </p>
            <button
              v-for="item in results"
              :key="item.id"
              class="settings-result"
              type="button"
              @click="locate(item)"
            >
              <span
                ><strong>{{ item.label }}</strong
                ><small
                  >{{ sectionLabel(item.category) }} ·
                  {{ item.description }}</small
                ></span
              ><svg class="icon" aria-hidden="true">
                <use href="#i-arrow-up-right" />
              </svg>
            </button>
          </div>
        </form>
      </div>
    </div>
  </motion.section>
</template>
