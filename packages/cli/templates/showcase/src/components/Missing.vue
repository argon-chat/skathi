<script setup lang="ts">
defineProps<{ part: string }>();
</script>

<template>
  <div class="missing">
    <span class="pill warn">{{ part }} left out</span>
    <span class="muted">
      This build has no <span class="mono">{{ part }}</span> engine part: sk.config.json lists it under
      <span class="mono">"windows": { "without": [...] }</span>, which keeps the exe smaller. Take it out of that list
      to bring the API back.
    </span>
  </div>
</template>

<style scoped>
.missing {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px;
  border-radius: 10px;
  border: 1px dashed #fbbf2440;
  background: #fbbf240a;
}
</style>
