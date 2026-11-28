<script setup lang="ts">
import { SkKbd } from "@skathi-native/runtime";
import { gamepads, type Controller } from "skathi:gamepad";
import { computed, onScopeDispose, shallowRef } from "vue";

const pad = shallowRef<Controller | undefined>(gamepads.list()[0]);
onScopeDispose(
  gamepads.subscribe((event) => {
    if (event.type === "disconnected") pad.value = gamepads.list()[0];
    else pad.value = event.gamepad;
  }),
);

const sticks = ["Left X", "Left Y", "Right X", "Right Y"];
const axes = computed(() => pad.value?.axisValues.slice(0, 4) ?? []);
const buttons = computed(() => pad.value?.buttonValues.slice(0, 17) ?? []);
const round = (value: number) => Math.round(value * 100) / 100;
function level(value: number) {
  const offset = Math.round(value * 100) / 2;
  return { left: 50 + Math.min(0, offset) + "%", width: Math.abs(offset) + "%" };
}
</script>

<template>
  <section class="pad">
    <header v-if="pad">
      <strong>{{ pad.name }}</strong>
      <span class="muted">{{ pad.mapping || "Unmapped" }}</span>
    </header>
    <p v-else class="muted">Connect a controller and press a button.</p>
    <div class="axes">
      <div v-for="(value, i) in axes" :key="i" class="axis">
        <span>{{ sticks[i] ?? "Axis " + i }}</span>
        <span class="track">
          <span class="zero"></span>
          <span class="level" :style="level(value)"></span>
        </span>
        <output>{{ round(value).toFixed(2) }}</output>
      </div>
    </div>
    <div class="buttons">
      <div v-for="(value, i) in buttons" :key="i" class="button" :class="{ pressed: value > 0.5 }">
        <SkKbd v-if="pad" :controller="pad" :keys="{ gamepad: i, mapping: pad.mapping }" />
        <output>{{ round(value).toFixed(2) }}</output>
      </div>
    </div>
  </section>
</template>

<style scoped>
.pad {
  display: flex;
  flex-direction: column;
  gap: 18px;
  width: 100%;
}
header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
}
.muted {
  color: rgb(255 255 255 / 0.6);
  font-size: 13px;
}
.axes {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.axis {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 13px;
}
.axis > span:first-child {
  width: 60px;
}
.axis output,
.button output {
  font-family: ui-monospace, monospace;
  font-variant-numeric: tabular-nums;
}
.axis output {
  width: 44px;
  text-align: right;
}
.track {
  position: relative;
  flex: 1;
  height: 8px;
  border-radius: 4px;
  background: rgb(255 255 255 / 0.14);
}
.zero,
.level {
  position: absolute;
  top: 0;
  height: 100%;
}
.zero {
  left: 50%;
  width: 1px;
  background: rgb(255 255 255 / 0.5);
}
.level {
  border-radius: 4px;
  background: #b9a2ff;
}
.buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.button {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 8px;
  border: 1px solid rgb(255 255 255 / 0.12);
  border-radius: 6px;
  color: rgb(255 255 255 / 0.65);
  font-size: 12px;
}
.pressed {
  border-color: #b9a2ff;
  background: rgb(185 162 255 / 0.25);
  color: #fff;
}
</style>
