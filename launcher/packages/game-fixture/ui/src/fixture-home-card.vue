<script setup lang="ts">
import { computed } from "vue";
import { greeting } from "./greeting";

const title = computed(() => greeting("测试游戏"));
</script>

<template>
  <article class="fixture-home-card">
    <p class="fixture-home-card__title">{{ title }}</p>
    <p class="fixture-home-card__note">
      fixture 包的首页卡片, 只在测试游戏激活时显示.
    </p>
  </article>
</template>

<style scoped>
.fixture-home-card {
  display: grid;
  gap: var(--space-8);
  padding: var(--space-20);
  border: 1px solid var(--glass-line);
  border-radius: var(--r-lg);
  background: var(--glass-card);
}
.fixture-home-card__title {
  margin: 0;
  color: var(--bone);
  font-size: var(--text-lg);
  font-weight: var(--weight-medium);
}
.fixture-home-card__note {
  margin: 0;
  color: var(--muted);
  font-size: var(--text-sm);
}
</style>
