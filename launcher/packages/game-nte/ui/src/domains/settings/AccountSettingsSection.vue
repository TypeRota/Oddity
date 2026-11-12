<script setup lang="ts">
import { computed } from "vue";
import { form, hostFacts, settingsNavigation } from "@host/api";
import { BaseButton, SettingGroup } from "@host/ui-kit";
import TaygedoSettings from "./TaygedoSettings.vue";

const active = computed(() => hostFacts.page.value === "settings");
function chooseGameCategory() {
  settingsNavigation.category = "nte-game";
  settingsNavigation.query = "";
}
</script>

<template>
  <div data-setting="account" tabindex="-1">
    <TaygedoSettings v-if="active && form.distribution === 'cn'" />
    <SettingGroup v-else-if="form.distribution === 'global'" title="账号与签到"
      ><p class="field__hint">
        塔吉多签到和实时数据仅适用于国服. 国际服请使用官方账号服务.
      </p>
      <BaseButton
        variant="ghost"
        icon="i-game-controller"
        @click="chooseGameCategory"
        >选择游戏地区</BaseButton
      ></SettingGroup
    >
  </div>
</template>
