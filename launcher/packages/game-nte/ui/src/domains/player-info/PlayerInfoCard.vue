<script setup lang="ts">
import {
  computed,
  onBeforeUnmount,
  onMounted,
  ref,
  useId,
  useTemplateRef,
  watch,
} from "vue";
import {
  AnimatePresence,
  GamepadGlyph,
  MotionConfig,
  motion,
  useReducedMotion,
} from "@host/ui-kit";
import { hostFacts, playerActions, playerCard, playerFacts } from "@host/api";
import { usePlayerMotion } from "./usePlayerMotion";

const panelId = useId();
const triggerId = useId();
const open = playerCard.expanded;
const gamepadActive = playerCard.gamepad;
const reduceMotion = hostFacts.reduceMotion;
const selectedId = ref("");
const direction = ref(1);
const reducedPreference = useReducedMotion();
const reduced = computed(() => reduceMotion.value || reducedPreference.value);
const spring = computed(() =>
  reduced.value
    ? { duration: 0 }
    : {
        type: "spring" as const,
        stiffness: 380,
        damping: 28,
        mass: 0.9,
        opacity: { duration: 0.12 },
      },
);
const surface = useTemplateRef<HTMLElement>("surface");
const details = useTemplateRef<HTMLElement>("details");
const { detailsReady, summaryReady, moving } = usePlayerMotion(
  open,
  reduced,
  surface,
  details,
);
const live = computed(() => playerFacts.live.value);
const accounts = computed(() => playerFacts.accounts.value);
const entries = computed(() => {
  const snapshots = [live.value.observation, live.value.record].filter(
    (record) => record !== null,
  );
  const items = accounts.value.map((account) => {
    const snapshot = snapshots.find(
      (record) => record.roleId && account.gameRecord?.roleId === record.roleId,
    );
    return {
      id: account.id,
      label: account.maskedPhone,
      record: snapshot
        ? {
            ...snapshot,
            roleName: snapshot.roleName || account.gameRecord?.roleName || "",
          }
        : account.gameRecord,
      failure: snapshot ? null : account.gameRecordError,
      source: snapshot ? "game" : "community",
    };
  });
  for (const snapshot of [...snapshots].reverse()) {
    if (
      items.some(
        (item) =>
          item.source === "game" && item.record?.roleId === snapshot.roleId,
      )
    )
      continue;
    items.unshift({
      id: `game:${snapshot.roleId || "unidentified"}`,
      label: snapshot.roleId ? "游戏角色" : "当前游戏",
      record: snapshot,
      failure: null,
      source: "game",
    });
  }
  return items.sort(
    (a, b) => Number(b.source === "game") - Number(a.source === "game"),
  );
});
const entry = computed(
  () =>
    entries.value.find((item) => item.id === selectedId.value) ??
    entries.value[0],
);
const index = computed(() =>
  Math.max(
    0,
    entries.value.findIndex((item) => item.id === entry.value?.id),
  ),
);
const record = computed(() => entry.value?.record);
const needsLogin = computed(() =>
  ["auth", "verification"].includes(entry.value?.failure?.kind ?? ""),
);
const communityBusy = computed(
  () =>
    playerFacts.recordsLoading.value ||
    (playerFacts.attendanceBusy.value && !playerFacts.attendanceRunning.value),
);
const recordsLoading = playerFacts.recordsLoading;
const recordsWaiting = playerFacts.recordsWaiting;
const attendanceLoaded = playerFacts.attendanceLoaded;
const attendanceLoading = playerFacts.attendanceLoading;
const busy = communityBusy;
const displayName = computed(
  () => record.value?.roleName || entry.value?.label || "玩家信息",
);
function select(offset: number) {
  if (!entries.value.length) return;
  direction.value = offset;
  selectedId.value =
    entries.value[
      (index.value + offset + entries.value.length) % entries.value.length
    ].id;
}
function close() {
  open.value = false;
  document.getElementById(triggerId)?.focus();
}
function settings() {
  close();
  playerActions.openAccountSettings();
}
function outside(event: PointerEvent) {
  const root = document.getElementById(triggerId)?.parentElement;
  if (
    open.value &&
    event.target instanceof Node &&
    !root?.contains(event.target)
  )
    open.value = false;
}
function refresh(force = false) {
  if (hostFacts.page.value === "home" && !document.hidden)
    void playerActions.refreshRecords(force);
}
function refreshLive() {
  if (hostFacts.page.value === "home" && !document.hidden)
    void playerActions.refreshLive();
}
watch(
  () => [
    accounts.value.map((a) => a.id).join(","),
    hostFacts.page.value,
    communityBusy.value,
  ],
  () => refresh(),
);
watch(
  () => [
    hostFacts.distribution.value,
    hostFacts.page.value,
    hostFacts.ready.value,
    open.value,
  ],
  refreshLive,
);
let communityTimer: ReturnType<typeof setInterval>;
let liveTimer: ReturnType<typeof setInterval>;
const onFocus = () => {
  refresh();
  refreshLive();
};
onMounted(() => {
  open.value = false;
  void playerActions.loadAccounts();
  refresh();
  refreshLive();
  communityTimer = setInterval(refresh, 30_000);
  liveTimer = setInterval(() => {
    if (live.value.enabled || live.value.record || live.value.observation)
      refreshLive();
  }, 1000);
  window.addEventListener("focus", onFocus);
  document.addEventListener("visibilitychange", onFocus);
  document.addEventListener("pointerdown", outside);
});
onBeforeUnmount(() => {
  open.value = false;
  clearInterval(communityTimer);
  clearInterval(liveTimer);
  window.removeEventListener("focus", onFocus);
  document.removeEventListener("visibilitychange", onFocus);
  document.removeEventListener("pointerdown", outside);
});
function value(number: number | null | undefined) {
  return number ?? "--";
}
function meter(
  current: number | null | undefined,
  max: number | null | undefined,
) {
  return `${value(current)} / ${value(max)}`;
}
function grouped(number: number | null | undefined) {
  return number == null
    ? "--"
    : String(number).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}
function remaining(milliseconds: number | null | undefined) {
  if (milliseconds == null) return "";
  const days = Math.floor(milliseconds / 86_400_000);
  if (days > 0) return `${days} 天后刷新`;
  const hours = Math.floor(milliseconds / 3_600_000);
  if (hours > 0) return `${hours} 小时后刷新`;
  return `${Math.max(1, Math.ceil(milliseconds / 60_000))} 分钟后刷新`;
}
function percent(data: NonNullable<typeof record.value>) {
  return !data.staminaMax || data.stamina == null
    ? 0
    : Math.max(0, Math.min(100, (data.stamina / data.staminaMax) * 100));
}
</script>

<template>
  <MotionConfig :reduced-motion="reduceMotion ? 'always' : 'user'"
    ><div
      class="game-records"
      :data-moving="moving"
      @keydown.esc.stop.prevent="close"
    >
      <div ref="surface" class="game-records__surface">
        <motion.button
          :id="triggerId"
          type="button"
          class="game-records__capsule"
          :aria-expanded="open"
          :aria-controls="panelId"
          :aria-label="open ? '收起游戏状态' : '打开游戏状态'"
          :while-hover="
            reduced ? {} : { backgroundColor: 'rgba(128, 128, 128, 0.08)' }
          "
          :while-press="reduced ? {} : { scale: 0.95 }"
          :transition="spring"
          @click="open = !open"
        >
          <span class="game-records__badge"
            ><svg class="icon" aria-hidden="true">
              <use href="#i-users" /></svg
          ></span>
          <motion.span
            class="game-records__summary"
            :animate="{
              opacity: summaryReady ? 1 : 0,
              x: summaryReady ? 0 : -6,
            }"
            :transition="{ duration: reduced ? 0 : 0.12 }"
          >
            <span class="game-records__name">{{ displayName }}</span>
            <span class="game-records__stamina">{{
              record
                ? `体力 ${meter(record.stamina, record.staminaMax)}`
                : entry
                  ? "查看游戏状态"
                  : "登录塔吉多"
            }}</span>
          </motion.span>
          <motion.svg
            class="icon game-records__caret"
            aria-hidden="true"
            :animate="{ rotate: open ? 180 : 0, opacity: summaryReady ? 1 : 0 }"
            :transition="spring"
          >
            <use href="#i-caret-down" />
          </motion.svg>
          <GamepadGlyph v-if="gamepadActive" button="north" />
        </motion.button>

        <AnimatePresence
          ><motion.aside
            v-if="open"
            :id="panelId"
            key="records"
            class="game-records__panel"
            :aria-labelledby="triggerId"
            :aria-busy="recordsLoading"
            :aria-hidden="!open || undefined"
            :inert="!open"
            :initial="{ opacity: 0, y: 8 }"
            :animate="{
              opacity: detailsReady ? 1 : 0,
              y: detailsReady ? 0 : 8,
            }"
            :exit="{ opacity: 0 }"
            :transition="{ duration: reduced ? 0 : 0.14 }"
            ><div ref="details" class="game-records__details">
              <header v-if="entry" class="game-records__head">
                <p class="game-records__identity">
                  <span>{{ displayName }}</span>
                  <span v-if="record?.roleId">UID {{ record.roleId }}</span>
                  <span v-else-if="entry?.source === 'game'">身份未识别</span>
                </p>
                <div class="game-records__actions">
                  <motion.button
                    v-if="entries.length"
                    class="game-records__icon-button"
                    :while-press="reduced ? {} : { scale: 0.88 }"
                    :transition="spring"
                    type="button"
                    aria-label="刷新游戏状态"
                    :disabled="busy"
                    @click="refresh(true)"
                  >
                    <svg class="icon" aria-hidden="true">
                      <use href="#i-arrows-clockwise" />
                    </svg>
                  </motion.button>
                </div>
              </header>
              <motion.article
                v-if="entry"
                :key="entry.id"
                class="game-records__account"
                aria-live="polite"
                :initial="{
                  opacity: reduced ? 1 : 0,
                  x: reduced ? 0 : direction * 16,
                }"
                :animate="{ opacity: detailsReady ? 1 : 0, x: 0 }"
                :transition="spring"
              >
                <template v-if="record">
                  <motion.dl
                    class="game-records__primary"
                    :initial="{ opacity: 0, y: 6 }"
                    :animate="{
                      opacity: detailsReady ? 1 : 0,
                      y: detailsReady ? 0 : 6,
                    }"
                    :transition="{ ...spring, delay: reduced ? 0 : 0.045 }"
                  >
                    <div>
                      <dt>体力 <small>本性像素</small></dt>
                      <dd>
                        <strong>{{ value(record.stamina) }}</strong
                        ><span> / {{ value(record.staminaMax) }}</span>
                      </dd>
                    </div>
                    <div>
                      <dt>周本剩余</dt>
                      <dd>
                        <strong>{{ value(record.weeklyRemaining) }}</strong
                        ><span> 次</span>
                      </dd>
                    </div>
                  </motion.dl>
                  <div class="game-records__track" aria-hidden="true">
                    <i :style="{ width: percent(record) + '%' }"></i>
                  </div>
                  <motion.dl
                    class="game-records__secondary"
                    :initial="{ opacity: 0, y: 6 }"
                    :animate="{
                      opacity: detailsReady ? 1 : 0,
                      y: detailsReady ? 0 : 6,
                    }"
                    :transition="{ ...spring, delay: reduced ? 0 : 0.09 }"
                  >
                    <div>
                      <dt>都市活力</dt>
                      <dd>
                        {{ meter(record.cityStamina, record.cityStaminaMax) }}
                      </dd>
                    </div>
                    <div>
                      <dt>今日活跃度</dt>
                      <dd>{{ value(record.activity) }}</dd>
                    </div>
                    <div>
                      <dt>备用本性像素</dt>
                      <dd>{{ value(record.staminaBackup) }} / 2400</dd>
                    </div>
                    <div>
                      <dt>备用都市活力</dt>
                      <dd>{{ value(record.cityStaminaBackup) }} / 1000</dd>
                    </div>
                    <div>
                      <dt>大亨激励金</dt>
                      <dd>
                        {{ grouped(record.incentiveObtained) }} /
                        {{ grouped(record.incentiveMaximum) }}
                        <small
                          v-if="record.incentiveRemaining != null"
                          class="game-records__note"
                          >{{ remaining(record.incentiveRemaining) }}</small
                        >
                      </dd>
                    </div>
                  </motion.dl>
                </template>
                <p v-else-if="recordsLoading" class="game-records__hint">
                  正在读取角色信息...
                </p>
                <p v-else class="game-records__hint">等待获取角色信息.</p>
                <p
                  v-if="recordsWaiting"
                  class="game-records__hint"
                  role="status"
                >
                  正在等待账号任务完成, 稍后自动刷新.
                </p>
                <div v-if="needsLogin" class="game-records__status">
                  <motion.button
                    v-if="needsLogin"
                    type="button"
                    class="game-records__link"
                    @click="settings"
                  >
                    前往设置重新登录 <span aria-hidden="true">↗</span>
                  </motion.button>
                </div>
              </motion.article>
              <div v-else class="game-records__empty">
                <span class="game-records__empty-mark" aria-hidden="true"
                  ><svg class="icon"><use href="#i-users" /></svg
                ></span>
                <p>
                  {{
                    !attendanceLoaded && attendanceLoading
                      ? "正在读取账号..."
                      : "登录塔吉多, 查看体力与周本信息."
                  }}
                </p>
                <motion.button
                  type="button"
                  class="game-records__link"
                  @click="settings"
                >
                  前往设置登录 <span aria-hidden="true">↗</span>
                </motion.button>
              </div>

              <footer v-if="entries.length" class="game-records__foot">
                <nav
                  v-if="entries.length > 1"
                  class="game-records__pager"
                  aria-label="切换账号"
                  @keydown.left.prevent="select(-1)"
                  @keydown.right.prevent="select(1)"
                >
                  <motion.button
                    class="game-records__icon-button game-records__previous"
                    type="button"
                    aria-label="上一个账号"
                    @click="select(-1)"
                  >
                    <svg class="icon" aria-hidden="true">
                      <use href="#i-caret-down" />
                    </svg>
                  </motion.button>
                  <span aria-live="polite" aria-atomic="true"
                    >{{ String(index + 1).padStart(2, "0") }}
                    <i>/ {{ String(entries.length).padStart(2, "0") }}</i></span
                  >
                  <motion.button
                    class="game-records__icon-button game-records__next"
                    type="button"
                    aria-label="下一个账号"
                    @click="select(1)"
                  >
                    <svg class="icon" aria-hidden="true">
                      <use href="#i-caret-down" />
                    </svg>
                  </motion.button>
                </nav>
              </footer></div></motion.aside
        ></AnimatePresence>
      </div></div
  ></MotionConfig>
</template>

<style scoped>
.game-records {
  position: absolute;
  top: var(--scene-inset);
  right: var(--scene-inset);
  z-index: 5;
  width: 280px;
  color: var(--bone);
  font-family: var(--font);
}
.game-records__surface {
  position: relative;
  width: 204px;
  height: 44px;
  margin-left: auto;
  overflow: clip;
  border: 1px solid var(--glass-line);
  border-radius: 14px;
  background: var(--glass-bg-strong);
  color: var(--bone);
  box-shadow:
    0 6px 24px rgb(0 0 0 / 12%),
    inset 0 1px 0 var(--glass-highlight);
  -webkit-backdrop-filter: blur(28px) saturate(var(--glass-saturate));
  backdrop-filter: blur(28px) saturate(var(--glass-saturate));
  transform-origin: center;
}
.game-records__capsule {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 42px;
  padding: 0 12px 0 7px;
  color: inherit;
  text-align: left;
  cursor: pointer;
}
.game-records__badge {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  flex: none;
  color: inherit;
}
.game-records__badge .icon {
  width: 18px;
  height: 18px;
}
.game-records__summary {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 1px;
}
.game-records__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  line-height: 15px;
  font-weight: 600;
}
.game-records__stamina {
  color: var(--muted);
  font-size: 10px;
  line-height: 14px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.game-records__caret {
  width: 10px;
  height: 10px;
  flex: none;
}
.game-records__panel {
  position: absolute;
  top: 42px;
  right: 0;
  width: 278px;
  overflow: clip;
}
.game-records__details {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px 16px 16px;
}
.game-records__head,
.game-records__actions,
.game-records__foot,
.game-records__pager {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.game-records__head {
  border-top: 1px solid var(--line);
  padding-top: 12px;
}
.game-records__icon-button {
  display: grid;
  place-items: center;
  flex: none;
  width: 26px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--line);
  border-radius: 8px;
  color: inherit;
  background: transparent;
  cursor: pointer;
}
.game-records__icon-button .icon {
  width: 13px;
  height: 13px;
}
.game-records__icon-button:hover {
  background: var(--hover);
}
.game-records__icon-button:disabled {
  opacity: 0.45;
}
.game-records__account {
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: clip;
}
.game-records__identity {
  display: flex;
  gap: 10px;
  margin: 0;
  color: var(--faint);
  font-size: 10px;
}
.game-records__identity span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.game-records__primary {
  display: grid;
  grid-template-columns: 1.45fr 1fr;
  gap: 12px;
  margin: 18px 0 0;
}
dt {
  color: var(--muted);
  font-size: 10px;
}
dt small {
  margin-left: 3px;
  color: var(--faint);
  font-size: 9px;
}
dd {
  margin: 5px 0 0;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.game-records__primary dd strong {
  font-size: 30px;
  line-height: 1;
  font-weight: 500;
  letter-spacing: -0.05em;
}
.game-records__primary dd span {
  color: var(--faint);
  font-size: 11px;
}
.game-records__primary > div:last-child {
  padding-left: 16px;
  border-left: 1px solid var(--line);
}
.game-records__track {
  height: 2px;
  margin: 12px 0 16px;
  background: var(--line);
  overflow: clip;
}
.game-records__track i {
  display: block;
  height: 100%;
  background: var(--signal-strong);
}
.game-records__secondary {
  display: grid;
  grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr);
  gap: 12px;
  margin: 0;
}
.game-records__secondary > div {
  min-width: 0;
}
.game-records__secondary > div:nth-child(even) {
  padding-left: 16px;
}
.game-records__secondary dd {
  font-size: 12px;
  white-space: normal;
  overflow-wrap: anywhere;
}
.game-records__note {
  display: block;
  margin-top: 2px;
  color: var(--faint);
  font-size: 9px;
  line-height: 12px;
}
.game-records__status {
  padding-top: 10px;
}
.game-records__error {
  margin: 0;
  color: var(--danger);
  font-size: 11px;
  line-height: 16px;
  overflow-wrap: anywhere;
}
.game-records__foot {
  justify-content: flex-end;
  border-top: 1px solid var(--line);
  padding-top: 10px;
  min-height: 0;
  font-size: 10px;
  color: var(--faint);
}
.game-records__foot:empty {
  display: none;
}
.game-records__pager {
  gap: 10px;
  font-variant-numeric: tabular-nums;
}
.game-records__pager i {
  font-style: normal;
  color: var(--faint);
}
.game-records__previous svg {
  transform: rotate(90deg);
}
.game-records__next svg {
  transform: rotate(-90deg);
}
.game-records__hint {
  margin: 14px 0 0;
  font-size: 11px;
  color: var(--muted);
}
.game-records__empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 10px 0 4px;
  text-align: center;
}
.game-records__empty p {
  margin: 0;
  color: var(--muted);
  font-size: 11px;
}
.game-records__empty-mark {
  display: none;
}
.game-records__link {
  padding: 0;
  border: 0;
  color: var(--accent-text);
  background: transparent;
  font: 500 11px var(--font);
  cursor: pointer;
}
</style>
