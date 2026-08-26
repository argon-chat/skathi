<script setup>
// shadcn-vue compatibility trial — the demo page.
//
// Only the harness (this file, Theme.vue, sections/Section.vue, the tier
// wrappers) is vue-native-specific. Everything it renders is upstream
// shadcn-vue.
import { ref } from "vue";

import Theme from "./Theme.vue";
import { Button } from "./components/ui/button";
import TierOne from "./sections/TierOne.vue";
import TierTwo from "./sections/TierTwo.vue";
import TierThree from "./sections/TierThree.vue";

// The whole theme is CSS custom properties (Theme.vue). Switching it is ONE
// class on ONE element: `.dark` / `.light` redefine every token, the tokens
// inherit down the tree, and every `bg-primary` / `text-muted-foreground` in
// every shadcn component re-resolves. Nothing below this line knows a theme
// exists — which is the point of the exercise.
//
// "system" declares neither class, so the `@media (prefers-color-scheme: dark)`
// block in Theme.vue decides — the OS path.
const MODES = ["system", "light", "dark"];
const mode = ref("system");
const cycle = () => {
  mode.value = MODES[(MODES.indexOf(mode.value) + 1) % MODES.length];
};
</script>

<template>
  <Theme>
    <!--
      `text-foreground` on the scroll container is the equivalent of shadcn's own
      `body { @apply bg-background text-foreground }`: several button/badge
      variants (outline, ghost, link) deliberately set NO colour and inherit it.
      Without it they render white-on-white, exactly as they would in a browser
      with no body rule.
    -->
    <div
      :class="[
        mode === 'system' ? '' : mode,
        'flex h-full flex-col gap-4 overflow-y-auto rounded-xl bg-background p-6 text-foreground',
      ]"
    >
      <div class="flex flex-row items-center gap-4">
        <div class="flex flex-1 flex-col gap-1">
          <div class="text-2xl font-semibold text-foreground">shadcn-vue on vue-native</div>
          <div class="text-sm text-muted-foreground">
            reka-ui primitives + Tailwind 3.4, rendered by gpui — no browser, no DOM.
          </div>
        </div>
        <Button id="theme-toggle" variant="outline" @click="cycle">theme: {{ mode }}</Button>
      </div>

      <div class="grid grid-cols-3 gap-4">
        <TierOne />
        <TierTwo />
        <TierThree />
      </div>
    </div>
  </Theme>
</template>
