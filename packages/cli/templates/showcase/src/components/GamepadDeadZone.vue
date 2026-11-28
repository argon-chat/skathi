<script setup lang="ts">
import { gamepads, type Controller, type DeadZoneTarget } from "skathi:gamepad";
import { computed, ref, watch } from "vue";
import { describe } from "../engine";

const props = defineProps<{ controller: Controller; effective: number }>();
const emit = defineEmits<{ change: [] }>();
const scope = ref<DeadZoneTarget["scope"]>("device");
const target = computed<DeadZoneTarget>(() => {
  const pad = props.controller;
  if (scope.value === "global") return { scope: "global" };
  if (scope.value === "model" && pad.vendorId)
    return { scope: "model", vendorId: pad.vendorId, productId: pad.productId };
  return pad.systemId ? { scope: "device", systemId: pad.systemId } : { scope: "device", id: pad.id };
});
const override = ref<number | null>(null);
const error = ref("");
const percent = (radius: number) => Math.round(radius * 10_000) / 100;
const status = computed(() => {
  const setting = override.value === null ? "Inherited" : `Override ${percent(override.value)}%`;
  return `${setting} · Effective ${percent(props.effective)}%`;
});

watch(
  target,
  (selected) => {
    error.value = "";
    try {
      override.value = gamepads.getDeadZone(selected);
    } catch (cause) {
      override.value = null;
      error.value = describe(cause);
    }
  },
  { immediate: true },
);

function set(radius: number | null) {
  try {
    gamepads.setDeadZone(target.value, radius);
    override.value = radius;
    error.value = "";
    emit("change");
  } catch (cause) {
    error.value = describe(cause);
  }
}
</script>

<template>
  <div class="dead-zone" role="group" aria-label="Stick dead zone">
    <strong>Stick dead zone</strong>
    <div class="row">
      <select
        :value="target.scope"
        class="input"
        aria-label="Dead zone scope"
        @change="scope = ($event.target as HTMLSelectElement).value as DeadZoneTarget['scope']"
      >
        <option value="device">{{ controller.systemId ? "This controller" : "This connection" }}</option>
        <option value="model" :disabled="!controller.vendorId">This model</option>
        <option value="global">All controllers</option>
      </select>
      <label class="row"
        >Radius
        <input
          type="range"
          min="0"
          max="99"
          step="1"
          :value="percent(override ?? effective)"
          aria-label="Dead zone radius (%)"
          @change="set(Number(($event.target as HTMLInputElement).value) / 100)"
        />
      </label>
      <button class="btn" :disabled="override === null" @click="set(null)">Reset</button>
    </div>
    <output class="muted" role="status">{{ error || status }}</output>
  </div>
</template>

<style scoped>
.dead-zone {
  border-top: 1px solid var(--line);
  padding-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 12px;
}
.dead-zone > .row {
  flex-wrap: wrap;
}
</style>
