<script setup lang="ts">
defineProps<{ title: string; lead: string }>();
</script>

<template>
  <section class="page">
    <header class="head">
      <h1 class="title">{{ title }}</h1>
      <p class="lead">{{ lead }}</p>
    </header>
    <slot />
  </section>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 28px 32px 40px;
}
.head {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 4px;
}
.title {
  margin: 0;
  font-size: 24px;
  font-weight: 700;
  color: var(--text);
}
.lead {
  margin: 0;
  max-width: 640px;
  font-size: 14px;
  line-height: 21px;
  color: var(--text-muted);
}
</style>
