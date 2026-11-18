<script setup lang="ts">
import { computed } from "vue";
import { CURSOR_FILL, CURSOR_HOTSPOT, CURSOR_OUTLINE, CURSOR_VIEWBOX } from "./cursorShape";
import { pointer } from "./demo";

const SIZE = 27;
const HX = (CURSOR_HOTSPOT.x / CURSOR_VIEWBOX) * SIZE;
const HY = (CURSOR_HOTSPOT.y / CURSOR_VIEWBOX) * SIZE;

const style = computed(() => ({
  transform: `translate(${pointer.x - HX}px, ${pointer.y - HY}px)`,
  transitionDuration: `${pointer.dur}ms, 200ms`,
}));
</script>

<template>
  <div class="demo-layer">
    <div class="demo-cursor" :class="{ shown: pointer.shown, pressed: pointer.pressed }" :style="style">
      <div v-if="pointer.ripple" :key="pointer.ripple" class="ripple" />
      <svg class="arrow" :width="SIZE" :height="SIZE" :viewBox="`0 0 ${CURSOR_VIEWBOX} ${CURSOR_VIEWBOX}`">
        <path :d="CURSOR_FILL" fill="#f7eafc" />
        <path :d="CURSOR_OUTLINE" fill="#8a44a3" fill-rule="evenodd" />
      </svg>
      <div v-if="pointer.key" class="key">{{ pointer.key }}</div>
    </div>
  </div>
</template>

<style scoped>
.demo-layer {
  position: absolute;
  inset: 0;
  overflow: hidden;
  pointer-events: none;
  z-index: 1000;
}
.demo-cursor {
  position: absolute;
  left: 0;
  top: 0;
  width: 27px;
  height: 27px;
  opacity: 0;
  transition-property: transform, opacity;
  transition-timing-function: cubic-bezier(0.32, 0, 0.18, 1), ease;
}
.demo-cursor.shown {
  opacity: 1;
}
.arrow {
  position: absolute;
  left: 0;
  top: 0;
  width: 27px;
  height: 27px;
  transform-origin: 4.7px 4px;
  transform: scale(1);
  transition: transform 110ms ease;
}
.pressed .arrow {
  transform: scale(0.86);
}
.ripple {
  position: absolute;
  left: -11px;
  top: -12px;
  width: 32px;
  height: 32px;
  border-radius: 16px;
  background: #8a44a3;
  opacity: 0;
  animation: ripple 520ms cubic-bezier(0.2, 0, 0, 1) 1;
}
@keyframes ripple {
  from {
    transform: scale(0.25);
    opacity: 0.5;
  }
  to {
    transform: scale(1.5);
    opacity: 0;
  }
}
.key {
  position: absolute;
  left: 30px;
  top: 26px;
  height: 30px;
  min-width: 44px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  white-space: nowrap;
  border-radius: 6px;
  border: 1px solid #8a44a366;
  border-bottom-width: 3px;
  background: #f7eafc;
  color: #4a1f5a;
  font-family: var(--font);
  font-size: 13px;
  font-weight: 600;
}
</style>
