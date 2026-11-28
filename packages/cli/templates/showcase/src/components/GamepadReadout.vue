<script setup lang="ts">
import { SkKbd } from "@skathi-native/runtime";
import type { Controller } from "skathi:gamepad";
import { computed } from "vue";

const props = defineProps<{
  axes: readonly number[];
  buttons: readonly number[];
  controller?: Controller | undefined;
}>();
const sticks = ["Left X", "Left Y", "Right X", "Right Y"];
const buttonCount = computed(() => props.buttons.length);
const buttonKeys = computed(() => {
  const mapping = props.controller?.mapping;
  return mapping === undefined ? [] : Array.from({ length: buttonCount.value }, (_, gamepad) => ({ gamepad, mapping }));
});
const displayValue = (value: number) => Math.round(value * 100) / 100;
function axisStyle(value: number) {
  const offset = Math.round(value * 100) / 2;
  return { left: 50 + Math.min(0, offset) + "%", width: Math.abs(offset) + "%" };
}
</script>

<template>
  <div class="readout">
    <div class="axis-list" aria-label="Live axes">
      <div v-for="(value, i) in axes" :key="i" v-memo="[displayValue(value), controller?.mapping]" class="axis">
        <span>{{ controller?.mapping === "standard" && sticks[i] ? sticks[i] : "Axis " + i }}</span>
        <span class="track" aria-hidden="true">
          <span class="zero"></span>
          <span class="level" :style="axisStyle(value)"></span>
        </span>
        <output class="mono">{{ displayValue(value).toFixed(2) }}</output>
      </div>
    </div>
    <div class="button-list" aria-label="Live button values">
      <div
        v-for="(value, i) in buttons"
        :key="i"
        v-memo="[displayValue(value), value > 0.5, controller]"
        class="button-value"
        :class="{ pressed: value > 0.5 }"
      >
        <SkKbd v-if="controller" :controller="controller" :keys="buttonKeys[i]" />
        <span v-else>Button {{ i }}</span>
        <output class="mono">{{ displayValue(value).toFixed(2) }}</output>
      </div>
    </div>
    <p v-if="!axes.length && !buttons.length" class="muted">
      Move a stick or press a button to discover its input index.
    </p>
  </div>
</template>

<style scoped>
.readout,
.axis-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.readout {
  gap: 14px;
}
.axis {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 12px;
}
.axis > span:first-child {
  width: 54px;
}
.axis output {
  width: 42px;
  text-align: right;
}
.track {
  position: relative;
  flex: 1;
  max-width: 280px;
  height: 8px;
  background: var(--surface-strong);
  border-radius: 4px;
}
.level,
.zero {
  position: absolute;
  top: 0;
  height: 100%;
}
.zero {
  left: 50%;
  width: 1px;
  background: var(--text-muted);
}
.level {
  background: var(--accent);
  border-radius: 4px;
}
.button-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.button-value {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 8px;
  border: 1px solid var(--line);
  border-radius: 6px;
  color: var(--text-muted);
  font-size: 12px;
}
.pressed {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--text);
}
</style>
