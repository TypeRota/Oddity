<script setup lang="ts">
import { computed } from "vue";
import { greeting } from "./greeting";

const props = defineProps<{ name: string }>();
const text = computed(() => greeting(props.name));
</script>

<template>
  <article class="hello-card">
    <p class="hello-card__title">{{ text }}</p>
  </article>
</template>

<style scoped>
/* 只复用宿主设计变量, fixture 包不新增视觉规范. */
.hello-card {
  padding: var(--space-20);
  border: 1px solid var(--glass-line);
  border-radius: var(--r-lg);
  background: var(--glass-card);
  color: var(--bone);
}
.hello-card__title {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: var(--weight-medium);
}
</style>
