<script setup lang="ts">
import { ref } from "vue";

import Theme from "./Theme.vue";
import { Button } from "./components/ui/button";
import TierOne from "./sections/TierOne.vue";
import TierTwo from "./sections/TierTwo.vue";
import TierThree from "./sections/TierThree.vue";

// "system" declares neither class, so the `@media (prefers-color-scheme: dark)` block in Theme.vue
// decides — the OS path.
const MODES = ["system", "light", "dark"] as const;
type Mode = (typeof MODES)[number];

const mode = ref<Mode>("system");
const cycle = () => {
  mode.value = MODES[(MODES.indexOf(mode.value) + 1) % MODES.length];
};
</script>

<template>
  <Theme>
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
