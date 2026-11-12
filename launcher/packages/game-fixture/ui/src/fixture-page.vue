<script setup lang="ts">
import { computed } from "vue";
import { hostFacts } from "@host/api";
import HelloCard from "./hello-card.vue";

const props = withDefaults(defineProps<{ name?: string }>(), {
  name: "fixture",
});
const title = computed(() => props.name.trim() || "fixture");
// 页面容器契约与宿主页面一致: 同一个 id, .page 定位与 is-active/inert 显隐.
const active = computed(() => hostFacts.page.value === "fixture-fixture");
</script>

<template>
  <section
    class="page fixture-page"
    id="page-fixture-fixture"
    :inert="!active"
    :class="{ 'is-active': active }"
    data-fixture-page
  >
    <HelloCard :name="title" />
    <p class="fixture-page__note">测试包页面, 仅供开发与契约测试引用.</p>
  </section>
</template>

<style scoped>
.fixture-page {
  display: grid;
  gap: var(--space-12);
}
.fixture-page__note {
  margin: 0;
  color: var(--muted);
  font-size: var(--text-sm);
}
</style>
