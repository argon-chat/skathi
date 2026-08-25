<script setup>
import { ref } from "vue";
import Counter from "./components/Counter.vue";

const title = ref("Hello from a real .vue SFC");
const name = ref("");
const wheelY = ref(0);
</script>

<template>
  <div class="stack" @wheel="wheelY = $event.deltaY">
    <div class="title">
      {{ title }}
      <span class="label">·scoped·</span>
    </div>

    <!-- 100% browser-standard markup: vModelText from the stock compiler -->
    <input class="field" v-model="name" placeholder="type here — native IME editor" />
    <div class="hint">v-model echo: "{{ name }}" ({{ name.length }} chars)</div>

    <Counter />

    <div class="label">App-scoped .label — must be BLUE</div>
    <div class="hint">last wheel ΔY over the card: {{ wheelY.toFixed(1) }}</div>
  </div>
</template>

<style scoped>
.stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 15px;
  color: #ffffff;
}
.field {
  height: 32px;
  width: 260px;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  font-size: 13px;
  color: #ffffff;
}
.label {
  font-size: 12px;
  color: #7fd4ff;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
</style>
