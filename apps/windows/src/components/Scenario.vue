<script setup lang="ts">
import { computed } from "vue";
import { scenarioById } from "../scenarios";

const props = defineProps<{ id: string }>();
const s = computed(() => scenarioById.get(props.id)!);
</script>

<template>
  <section class="page">
    <header class="head">
      <div class="eyebrow">{{ s.guide }}</div>
      <h1 class="title">{{ s.title }}</h1>
      <p class="lead">{{ s.lead }}</p>
      <div class="apis">
        <span v-for="api in s.apis" :key="api" class="api">{{ api }}</span>
      </div>
      <p v-if="s.platform" class="platform">
        <svg class="info" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 11v6 M12 7.5h.01" stroke-linecap="round" />
        </svg>
        <span>{{ s.platform }}</span>
      </p>
    </header>
    <slot />
  </section>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 32px 40px 40px;
  max-width: 920px;
}
.head {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 8px;
}
.eyebrow {
  font-size: 12px;
  font-weight: 600;
  color: var(--accent-text);
}
.title {
  margin: 0;
  font-family: var(--font-display);
  font-size: 28px;
  font-weight: 600;
  color: var(--text);
}
.lead {
  margin: 0;
  max-width: 680px;
  font-size: 14px;
  line-height: 21px;
  color: var(--text-2);
}
.apis {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 4px;
}
.api {
  height: 22px;
  padding: 0 8px;
  display: flex;
  align-items: center;
  border-radius: 4px;
  background: var(--well);
  color: var(--text-2);
  font-family: var(--mono);
  font-size: 11px;
}
.platform {
  margin: 4px 0 0;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-3);
}
.info {
  flex-shrink: 0;
}
</style>
