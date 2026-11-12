<script setup lang="ts">
import { computed, onErrorCaptured, ref } from "vue";
import type { Component } from "vue";
import { state } from "../core/controller";
import type { PageSlot } from "./pages";

const props = defineProps<{
  id: string;
  page: Component;
  area: PageSlot;
}>();

const active = computed(() => state.page === props.id);
const failure = ref("");
// 错误边界按贡献隔离: 单个页面崩溃只替换自己, 同一区域里的其它页面照常渲染.
onErrorCaptured((error) => {
  failure.value = error instanceof Error ? error.message : String(error);
  return false;
});
</script>

<template>
  <!-- 错误态沿用页面容器的语义: 同一 id, .page 定位与 is-active/inert 显隐. -->
  <section
    v-if="failure"
    :id="`page-${id}`"
    class="page"
    :class="{ 'is-active': active, 'page--home': area === 'home' }"
    :inert="!active"
    role="alert"
  >
    <header class="page-head">
      <div class="page-head__copy">
        <h2>页面加载失败</h2>
        <p>{{ failure }}</p>
      </div>
    </header>
  </section>
  <component :is="page" v-else />
</template>
