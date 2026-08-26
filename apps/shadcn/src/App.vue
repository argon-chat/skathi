<script setup>
// shadcn-vue compatibility trial — the demo page.
//
// Only the harness (this file, sections/Section.vue, the tier wrappers) is
// vue-native-specific. Everything it renders is upstream shadcn-vue.
import TierOne from "./sections/TierOne.vue";
import TierTwo from "./sections/TierTwo.vue";
import TierThree from "./sections/TierThree.vue";
</script>

<template>
  <!--
    `text-foreground` on the scroll container is the equivalent of shadcn's own
    `body { @apply bg-background text-foreground }`: several button/badge
    variants (outline, ghost, link) deliberately set NO colour and inherit it.
    Without it they render white-on-white, exactly as they would in a browser
    with no body rule.
  -->
  <div class="flex h-full flex-col gap-4 overflow-y-auto rounded-xl bg-background p-6 text-foreground">
    <div class="flex flex-col gap-1">
      <div class="text-2xl font-semibold text-foreground">shadcn-vue on vue-native</div>
      <div class="text-sm text-muted-foreground">
        reka-ui primitives + Tailwind 3.4, rendered by gpui — no browser, no DOM.
      </div>
    </div>

    <div class="grid grid-cols-3 gap-4">
      <TierOne />
      <TierTwo />
      <TierThree />
    </div>
  </div>
</template>
