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
  padding-top: 4px;
  padding-bottom: 4px;
  border-radius: 6px;
  background: #151c27;
  border-width: 1px;
  border-color: #2f3b4d;
  box-shadow: 0px 6px 18px #00000080;
}
.item {
  height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  padding-left: 10px;
  padding-right: 10px;
  font-size: 11px;
  color: #c9d4e2;
  white-space: nowrap;
}
.item:hover {
  background: #14344f;
  color: #e6edf3;
}
.item.off {
  color: #4e5b6d;
}
.item.off:hover {
  background: #00000000;
  color: #4e5b6d;
}
.item.divided {
  border-top-width: 1px;
  border-color: #232c3a;
}
.label-text {
  flex: 1;
}
.hint {
  font-size: 10px;
  color: #5c6b80;
}
</style>
