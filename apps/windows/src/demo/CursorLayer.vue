<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { mainLayer, pointer, registerPopLayer, type Point } from "./cursor";
import { CURSOR_FILL, CURSOR_HOTSPOT, CURSOR_OUTLINE, CURSOR_VIEWBOX } from "./cursorShape";

const props = defineProps<{
  /** For a pop-out: where the main window's client origin is in this window. Absent in the main window. */
  offset?: () => Point | null;
}>();

const SIZE = 27;
const HX = (CURSOR_HOTSPOT.x / CURSOR_VIEWBOX) * SIZE;
const HY = (CURSOR_HOTSPOT.y / CURSOR_VIEWBOX) * SIZE;

const layer = ref<Element | null>(null);
let unregister = () => {};

onMounted(() => {
  const el = layer.value;
  if (!el) return;
  if (!props.offset) {
    mainLayer.value = el;
    return;
  }
  const offset = props.offset;
  unregister = registerPopLayer({ layer: el, root: el.closest(".sk-window-root"), offset });
});
onBeforeUnmount(() => {
  unregister();
  if (!props.offset && mainLayer.value === layer.value) mainLayer.value = null;
});

const pos = computed<Point | null>(() => {
  const x = pointer.x;
  const y = pointer.y;
  if (!props.offset) return { x, y };
  const off = props.offset();
  return off ? { x: x + off.x, y: y + off.y } : null;
});

const style = computed(() =>
  pos.value
    ? {
        transform: `translate(${pos.value.x - HX}px, ${pos.value.y - HY}px)`,
        transitionDuration: `${pointer.dur}ms, 200ms`,
      }
    : {},
);
</script>

<template>
  <div ref="layer" class="demo-layer">
    <div v-if="pos" class="demo-cursor" :class="{ shown: pointer.shown, pressed: pointer.pressed }" :style="style">
      <div v-if="pointer.ripple" :key="pointer.ripple" class="ripple" />
      <svg class="arrow" :width="SIZE" :height="SIZE" :viewBox="`0 0 ${CURSOR_VIEWBOX} ${CURSOR_VIEWBOX}`">
        <path :d="CURSOR_FILL" fill="#f7eafc" />
        <path :d="CURSOR_OUTLINE" fill="#8a44a3" fill-rule="evenodd" />
      </svg>
      <div v-if="pointer.key && !offset" class="key">{{ pointer.key }}</div>
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
