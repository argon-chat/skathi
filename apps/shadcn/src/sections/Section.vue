<script setup lang="ts">
// Spelling it `title: string` here would compile to `required: true` and start warning, which is a
// behaviour change wearing a type annotation.
defineProps<{ title?: string; note?: string }>();
</script>

<template>
  <div class="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
    <div class="flex flex-row items-center gap-2">
      <div class="text-sm font-semibold text-card-foreground">{{ title }}</div>
      <div v-if="note" class="text-xs text-muted-foreground">{{ note }}</div>
    </div>
    <div class="flex flex-col gap-3">
      <slot />
    </div>
  </div>
</template>
