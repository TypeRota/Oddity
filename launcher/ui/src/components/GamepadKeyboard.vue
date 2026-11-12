<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { motion } from "motion-v";
import { gamepad } from "../core/gamepad";
import { gamepadActions } from "../core/gamepadActions";
import {
  loadPinyinEngine,
  type PinyinCandidate,
  type PinyinEngine,
} from "../core/pinyin";
import { useControlMotion } from "../composables/useControlMotion";
import BaseButton from "./BaseButton.vue";

const PAGE_SIZE = 5;
const draft = ref("");
const shift = ref(false);
const symbols = ref(false);
const pinyin = ref(false);
const composition = ref("");
const candidates = ref<PinyinCandidate[]>([]);
const candidatePage = ref(0);
const loading = ref(false);
const error = ref("");
let engine: PinyinEngine | null = null;
let engineFailed = false;
let searchVersion = 0;
const { surface, control } = useControlMotion();
watch([draft, composition], () => {
  error.value = "";
});
watch(
  () => gamepad.editing,
  (field) => {
    if (field) {
      draft.value = field.value;
      shift.value = false;
      symbols.value = false;
      pinyin.value = false;
      composition.value = "";
      candidates.value = [];
      candidatePage.value = 0;
      error.value = "";
      engineFailed = false;
      gamepadActions.keyboard = {
        cycle: cycleLayout,
        commit,
        backspace,
        clear,
        page: pageCandidates,
        pinyin: false,
      };
    } else if (gamepadActions.keyboard) {
      gamepadActions.keyboard = null;
    }
  },
);
watch(pinyin, (value) => {
  if (gamepadActions.keyboard) gamepadActions.keyboard.pinyin = value;
});
onBeforeUnmount(() => {
  if (gamepadActions.keyboard) gamepadActions.keyboard = null;
});
const numeric = computed(
  () =>
    gamepad.editing instanceof HTMLInputElement &&
    ["number", "tel", "time"].includes(gamepad.editing.type),
);
const time = computed(
  () =>
    gamepad.editing instanceof HTMLInputElement &&
    gamepad.editing.type === "time",
);
const password = computed(
  () =>
    gamepad.editing instanceof HTMLInputElement &&
    gamepad.editing.type === "password",
);
const pinyinAllowed = computed(() => !numeric.value && !password.value);
const composing = computed(() => pinyin.value && composition.value.length > 0);
const pageCount = computed(() =>
  Math.max(1, Math.ceil(candidates.value.length / PAGE_SIZE)),
);
const visibleCandidates = computed(() =>
  candidates.value.slice(
    candidatePage.value * PAGE_SIZE,
    (candidatePage.value + 1) * PAGE_SIZE,
  ),
);
const rows = computed(() =>
  numeric.value
    ? ["123", "456", "789", time.value ? "0:" : "0.-"]
    : pinyin.value
      ? ["qwertyuiop", "asdfghjkl", "zxcvbnm"]
      : symbols.value
        ? ["!@#$%^&*()", "_-+=[]{}", ":;\"'/?\\", ".,<>|~`"]
        : ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"],
);
const display = computed(() => {
  if (password.value) return "•".repeat(draft.value.length);
  return draft.value;
});
function ensureEngine(): Promise<PinyinEngine | null> {
  if (engine) return Promise.resolve(engine);
  if (engineFailed) return Promise.resolve(null);
  loading.value = true;
  return loadPinyinEngine()
    .then((loaded) => {
      engine = loaded;
      return loaded;
    })
    .catch((cause) => {
      engineFailed = true;
      error.value = `拼音词库加载失败: ${String(cause)}`;
      return null;
    })
    .finally(() => {
      loading.value = false;
    });
}
async function refreshCandidates() {
  const input = composition.value;
  const version = ++searchVersion;
  if (!input) {
    candidates.value = [];
    candidatePage.value = 0;
    return;
  }
  const current = await ensureEngine();
  if (version !== searchVersion || composition.value !== input) return;
  candidates.value = current ? current.search(input) : [];
  candidatePage.value = 0;
}
function enterPinyin() {
  shift.value = false;
  symbols.value = false;
  composition.value = "";
  candidates.value = [];
  candidatePage.value = 0;
  engineFailed = false;
  error.value = "";
  void ensureEngine();
}
function togglePinyin() {
  if (!pinyinAllowed.value) return;
  pinyin.value = !pinyin.value;
  if (pinyin.value) {
    enterPinyin();
    return;
  }
  composition.value = "";
  candidates.value = [];
  candidatePage.value = 0;
  shift.value = false;
  symbols.value = false;
}
function cycleLayout() {
  if (numeric.value) return;
  if (!pinyinAllowed.value) {
    if (symbols.value) {
      symbols.value = false;
      shift.value = false;
    } else if (shift.value) {
      shift.value = false;
      symbols.value = true;
    } else {
      shift.value = true;
    }
    return;
  }
  if (pinyin.value) {
    togglePinyin();
    return;
  }
  if (symbols.value) {
    symbols.value = false;
    pinyin.value = true;
    enterPinyin();
    return;
  }
  if (shift.value) {
    shift.value = false;
    symbols.value = true;
    return;
  }
  shift.value = true;
}
function typeKey(key: string) {
  if (pinyin.value) {
    if (!/^[a-z]$/.test(key)) return;
    composition.value = (composition.value + key).slice(0, 32);
    void refreshCandidates();
    return;
  }
  draft.value += shift.value ? key.toUpperCase() : key;
}
function space() {
  if (pinyin.value && composition.value) {
    const first = candidates.value[0];
    if (first) choose(first);
    else commitRaw();
    return;
  }
  draft.value += " ";
}
function choose(candidate: PinyinCandidate) {
  const consumed = Math.min(candidate.matchedLength, composition.value.length);
  draft.value += candidate.word;
  composition.value = composition.value.slice(consumed);
  if (composition.value) {
    void refreshCandidates();
    return;
  }
  candidates.value = [];
  candidatePage.value = 0;
}
function commitRaw() {
  if (!composition.value) return;
  draft.value += composition.value;
  composition.value = "";
  candidates.value = [];
  candidatePage.value = 0;
}
function pageCandidates(delta: number): boolean {
  if (!composing.value || pageCount.value <= 1) return false;
  const next = candidatePage.value + delta;
  if (next < 0 || next >= pageCount.value) return false;
  candidatePage.value = next;
  return true;
}
function backspace() {
  if (pinyin.value && composition.value) {
    composition.value = composition.value.slice(0, -1);
    void refreshCandidates();
    return;
  }
  draft.value = draft.value.slice(0, -1);
}
function clear() {
  draft.value = "";
  composition.value = "";
  candidates.value = [];
  candidatePage.value = 0;
  error.value = "";
}
function commit() {
  const field = gamepad.editing;
  if (!field) return;
  if (composition.value) commitRaw();
  if (time.value && !/^([01]\d|2[0-3]):[0-5]\d$/.test(draft.value)) {
    error.value = "请输入 24 小时时间 (HH:mm).";
    return;
  }
  field.value = draft.value;
  field.dispatchEvent(new Event("input", { bubbles: true }));
  field.dispatchEvent(new Event("change", { bubbles: true }));
  gamepad.editing = null;
  field.focus({ preventScroll: true });
}
</script>
<template>
  <div
    v-if="gamepad.editing"
    class="pad-keyboard-backdrop"
    data-pad-modal
    @keydown.esc.prevent.stop="gamepad.editing = null"
  >
    <motion.section
      class="pad-keyboard"
      role="dialog"
      aria-modal="true"
      :aria-label="time ? '设置时间' : '屏幕键盘'"
      :initial="{ opacity: 0, y: 24 }"
      :animate="{ opacity: 1, y: 0 }"
      :transition="surface"
    >
      <header>
        <h2>{{ time ? "设置时间" : "屏幕键盘" }}</h2>
        <BaseButton
          icon="i-x"
          icon-only
          size="sm"
          data-pad-close
          aria-label="关闭屏幕键盘"
          @click="gamepad.editing = null"
        />
      </header>
      <output class="pad-keyboard__display">
        <template v-if="display">{{ display }}</template>
        <span v-if="composing" class="pad-keyboard__composition">{{
          composition
        }}</span>
        <template v-if="!display && !composing">{{
          time ? "HH:mm" : password ? "输入密码" : "输入内容"
        }}</template>
      </output>
      <div
        v-if="composing"
        class="pad-keyboard__candidates"
        role="group"
        aria-label="拼音候选"
      >
        <BaseButton
          v-for="(candidate, index) in visibleCandidates"
          :key="`${candidate.word}-${index}`"
          class="pad-keyboard__candidate"
          variant="ghost"
          size="sm"
          @click="choose(candidate)"
          >{{ index + 1 }}. {{ candidate.word }}</BaseButton
        >
        <BaseButton
          v-if="!loading && !visibleCandidates.length"
          variant="ghost"
          size="sm"
          @click="commitRaw"
          >原样上屏</BaseButton
        >
        <span v-if="loading" class="pad-keyboard__candidate-hint"
          >词库加载中…</span
        >
        <span
          v-if="!loading && candidates.length > PAGE_SIZE"
          class="pad-keyboard__candidate-pager"
        >
          <BaseButton
            variant="ghost"
            size="sm"
            :disabled="candidatePage === 0"
            @click="pageCandidates(-1)"
            >上一页</BaseButton
          >
          <span>{{ candidatePage + 1 }}/{{ pageCount }}</span>
          <BaseButton
            variant="ghost"
            size="sm"
            :disabled="candidatePage >= pageCount - 1"
            @click="pageCandidates(1)"
            >下一页</BaseButton
          >
        </span>
      </div>
      <p v-if="error" class="field__hint" role="alert">{{ error }}</p>
      <div class="pad-keyboard__keys">
        <div
          v-for="(row, index) in rows"
          :key="index"
          class="pad-keyboard__row"
        >
          <motion.button
            v-for="key in row"
            :key="key"
            type="button"
            :while-press="{ scale: 0.9 }"
            :transition="control"
            @click="typeKey(key)"
            >{{ shift && !pinyin ? key.toUpperCase() : key }}</motion.button
          >
        </div>
        <div class="pad-keyboard__row pad-keyboard__actions">
          <BaseButton v-if="!numeric && !pinyin" @click="shift = !shift">{{
            shift ? "小写" : "大写"
          }}</BaseButton
          ><BaseButton v-if="!numeric && !pinyin" @click="symbols = !symbols">{{
            symbols ? "字母" : "符号"
          }}</BaseButton
          ><BaseButton v-if="pinyinAllowed" @click="togglePinyin">{{
            pinyin ? "字母" : "拼音"
          }}</BaseButton
          ><BaseButton v-if="!numeric" @click="space">空格</BaseButton
          ><BaseButton v-if="time" @click="clear">清空</BaseButton
          ><BaseButton @click="backspace">删除</BaseButton
          ><BaseButton variant="primary" @click="commit">完成</BaseButton>
        </div>
      </div>
    </motion.section>
  </div>
</template>
