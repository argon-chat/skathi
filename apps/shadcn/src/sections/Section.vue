<script setup lang="ts">
// Harness chrome, not a shadcn component: a labelled panel + a per-row caption
// so a screenshot says which component it is showing.
//
// BOTH props stay OPTIONAL. The runtime declaration this replaces was
// `{ title: String, note: String }` — neither `required`, so a missing `title`
// was legal and rendered an empty heading. Spelling it `title: string` here
// would compile to `required: true` and start warning, which is a behaviour
// change wearing a type annotation. Every call site passes one anyway.
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
