<script setup lang="ts">
defineProps<{ title: string; api?: string }>();
</script>

<template>
  <div class="card">
    <div class="top">
      <span class="title">{{ title }}</span>
      <span v-if="api" class="api">{{ api }}</span>
    </div>
    <slot />
  </div>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 18px;
  border-radius: var(--radius);
  border: 1px solid var(--line);
  background: var(--surface);
}
.top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.api {
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text-faint);
}
</style>
