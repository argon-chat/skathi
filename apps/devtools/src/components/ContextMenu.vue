<script setup lang="ts">
import { computed } from "vue";
import { MENU_ITEM_H, viewport } from "../layout";
import { closeMenu, ui } from "../store";
import type { MenuItem } from "../protocol";

const WIDTH = 236;

const at = computed(() => {
  const m = ui.menu;
  if (!m) return { left: 0, top: 0 };
  const height = m.items.length * MENU_ITEM_H + 8;
  return {
    left: Math.max(4, Math.min(m.x, viewport.w - WIDTH - 4)),
    top: Math.max(4, m.y + height > viewport.h - 4 ? m.y - height : m.y),
  };
});

function run(item: MenuItem): void {
  if (item.disabled) return;
  closeMenu();
  item.run();
}
</script>

<template>
  <template v-if="ui.menu">
    <div class="menu-shield" @mousedown="closeMenu()"></div>
    <div class="menu" :style="{ left: at.left + 'px', top: at.top + 'px', width: WIDTH + 'px' }" @mousedown.stop>
      <div
        v-for="(item, i) in ui.menu.items"
        :key="i"
        class="item"
        :class="{ off: item.disabled, divided: item.divider }"
        @click="run(item)"
      >
        <span class="label-text">{{ item.label }}</span>
        <span v-if="item.hint" class="hint mono">{{ item.hint }}</span>
      </div>
    </div>
  </template>
</template>

<style scoped>
.menu-shield {
  position: absolute;
  left: 0px;
  top: 0px;
  width: 100%;
  height: 100%;
  background: #00000001;
}
.menu {
  position: absolute;
  display: flex;
  flex-direction: column;
  padding-top: var(--sp-2);
  padding-bottom: var(--sp-2);
  border-radius: var(--r-l);
  background: var(--surface-raised);
  border-width: 1px;
  border-color: var(--line-strong);
  box-shadow: 0px 6px 18px #00000080;
}
.item {
  height: var(--h-row-loose);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  font-size: var(--fs-s);
  color: var(--text);
  white-space: nowrap;
}
.item:hover {
  background: var(--surface-active);
  color: var(--text-strong);
}
.item.off {
  color: var(--text-ghost);
}
.item.off:hover {
  background: transparent;
  color: var(--text-ghost);
}
.item.divided {
  border-top-width: 1px;
  border-color: var(--line);
}
.label-text {
  flex: 1;
}
.hint {
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
</style>
