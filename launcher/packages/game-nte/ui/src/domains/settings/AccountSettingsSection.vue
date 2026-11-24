<script setup lang="ts">
import { computed } from "vue";
import { hostFacts } from "@host/api";
import TaygedoSettings from "./TaygedoSettings.vue";

const active = computed(() => hostFacts.page.value === "settings");
</script>

<template>
  <div data-setting="account" tabindex="-1">
    <TaygedoSettings v-if="active" />
  </div>
</template>
