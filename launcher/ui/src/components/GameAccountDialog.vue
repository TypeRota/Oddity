<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from "vue";
import { invoke } from "../core/api";
import type { GameAccount } from "../types/launcher";
import BaseModal from "./BaseModal.vue";
import BaseTabs from "./BaseTabs.vue";
import BaseInput from "./BaseInput.vue";
import BaseButton from "./BaseButton.vue";

const props = defineProps<{
  open: boolean;
  selected: string;
  theme: string;
  reduceMotion: boolean;
}>();
const emit = defineEmits<{
  "update:open": [value: boolean];
  select: [id: string];
}>();
const accounts = ref<GameAccount[]>([]);
const adding = ref(false);
const confirming = ref("");
const method = ref<"sms" | "password">("sms");
const phone = ref("");
const secret = ref("");
const deviceId = ref("");
const sentPhone = ref("");
const busy = ref(false);
const error = ref("");
const cooldown = ref(0);
let revision = 0;
let timer: ReturnType<typeof setInterval> | undefined;
const methods = [
  { value: "sms" as const, label: "短信验证码" },
  { value: "password" as const, label: "密码" },
];
async function refresh() {
  const current = revision;
  error.value = "";
  busy.value = true;
  try {
    const result = await invoke("game_accounts");
    if (current === revision) {
      accounts.value = result;
      adding.value = result.length === 0;
    }
  } catch (cause) {
    if (current === revision)
      error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    busy.value = false;
  }
}
watch(
  () => props.open,
  (open) => {
    revision++;
    secret.value = "";
    deviceId.value = "";
    sentPhone.value = "";
    error.value = "";
    confirming.value = "";
    if (open) void refresh();
  },
);
watch(method, () => {
  secret.value = "";
});
onBeforeUnmount(() => {
  revision++;
  clearInterval(timer);
});
async function send() {
  if (busy.value || cooldown.value) return;
  if (!/^1[0-9]{10}$/.test(phone.value.trim())) {
    error.value = "请输入11位中国内地手机号码.";
    return;
  }
  const current = revision;
  const requestedPhone = phone.value.trim();
  busy.value = true;
  error.value = "";
  try {
    const id = await invoke("game_send_code", { phone: requestedPhone });
    cooldown.value = 60;
    clearInterval(timer);
    timer = setInterval(() => {
      if (--cooldown.value <= 0) clearInterval(timer);
    }, 1000);
    if (current === revision) {
      deviceId.value = id;
      sentPhone.value = requestedPhone;
    }
  } catch (cause) {
    if (current === revision)
      error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    busy.value = false;
  }
}
async function login() {
  if (busy.value) return;
  if (
    method.value === "sms" &&
    (phone.value.trim() !== sentPhone.value || !deviceId.value)
  ) {
    error.value = "请先为当前手机号获取验证码.";
    return;
  }
  const current = revision;
  busy.value = true;
  error.value = "";
  try {
    const account = await invoke("game_login", {
      phone: phone.value.trim(),
      secret: secret.value,
      password: method.value === "password",
      deviceId: deviceId.value,
    });
    if (current === revision) {
      emit("select", account.accountId);
      emit("update:open", false);
    }
  } catch (cause) {
    if (current === revision) {
      const message = cause instanceof Error ? cause.message : String(cause);
      if (message.includes("短信验证码登录")) method.value = "sms";
      error.value = message;
    }
  } finally {
    secret.value = "";
    busy.value = false;
  }
}
async function remove(id: string) {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    await invoke("game_account_remove", { accountId: id });
    confirming.value = "";
    if (props.selected === id) emit("select", "");
    accounts.value = accounts.value.filter(
      (account) => account.accountId !== id,
    );
    adding.value = accounts.value.length === 0;
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <BaseModal
    :open="open"
    title="游戏账号"
    :theme="theme"
    :reduce-motion="reduceMotion"
    :view-key="adding ? 'add' : 'list'"
    @update:open="emit('update:open', $event)"
  >
    <div class="game-accounts">
      <template v-if="!adding">
        <article
          v-for="account in accounts"
          :key="account.accountId"
          class="game-accounts__row"
        >
          <strong>{{ account.maskedPhone }}</strong>
          <BaseButton
            :disabled="busy || selected === account.accountId"
            @click="emit('select', account.accountId)"
          >
            {{ selected === account.accountId ? "已选择" : "选择账号" }}
          </BaseButton>
          <template v-if="confirming === account.accountId">
            <span class="game-accounts__error">确认移除此账号?</span>
            <BaseButton
              variant="danger"
              :disabled="busy"
              @click="remove(account.accountId)"
              >确认移除</BaseButton
            >
            <BaseButton
              variant="ghost"
              :disabled="busy"
              @click="confirming = ''"
              >取消</BaseButton
            >
          </template>
          <BaseButton
            v-else
            variant="danger"
            :disabled="busy"
            @click="confirming = account.accountId"
            >移除</BaseButton
          >
        </article>
        <BaseButton
          :disabled="busy"
          @click="
            adding = true;
            confirming = '';
          "
          >添加游戏账号</BaseButton
        >
        <p class="game-accounts__hint">选择的账号用于下次直接启动.</p>
      </template>
      <form v-else class="game-accounts__form" @submit.prevent="login">
        <BaseTabs
          v-model="method"
          :options="methods"
          label="游戏登录方式"
          :disabled="busy"
          @change="error = ''"
        />
        <label class="field"
          ><span>手机号码</span>
          <BaseInput
            v-model="phone"
            type="tel"
            inputmode="numeric"
            autocomplete="username"
            maxlength="11"
            required
            :disabled="busy"
          />
        </label>
        <label class="field"
          ><span>{{ method === "sms" ? "短信验证码" : "密码" }}</span>
          <BaseInput
            :key="method"
            v-model="secret"
            :type="method === 'password' ? 'password' : 'text'"
            :autocomplete="
              method === 'password' ? 'current-password' : 'one-time-code'
            "
            required
            :disabled="busy"
          />
        </label>
        <BaseButton
          v-if="method === 'sms'"
          :disabled="busy || cooldown > 0"
          @click="send"
        >
          {{ cooldown > 0 ? `${cooldown}秒后重发` : "获取验证码" }}
        </BaseButton>
        <p class="game-accounts__hint">
          登录状态保存在桌面密钥环, 不保存密码和验证码.
        </p>
        <div class="game-accounts__actions">
          <BaseButton
            v-if="accounts.length"
            :disabled="busy"
            @click="
              adding = false;
              secret = '';
            "
            >返回账号列表</BaseButton
          >
          <BaseButton variant="primary" type="submit" :busy="busy"
            >登录并记住账号</BaseButton
          >
        </div>
      </form>
      <p v-if="error" role="alert" class="game-accounts__error">{{ error }}</p>
    </div>
  </BaseModal>
</template>

<style scoped>
.game-accounts,
.game-accounts__form {
  display: grid;
  gap: 18px;
}
.game-accounts__row,
.game-accounts__actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.game-accounts__row strong {
  margin-right: auto;
}
.game-accounts__hint {
  color: var(--muted);
  font-size: 13px;
  margin: 0;
}
.game-accounts__error {
  color: var(--danger);
  margin: 0;
}
</style>
