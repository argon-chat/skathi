<script setup>
import { ref, computed } from "vue";

const name = ref("");
const focused = ref("nothing");

const submitted = ref([]);
const submit = () => {
  const v = name.value.trim();
  if (!v) return;
  submitted.value = [...submitted.value.slice(-2), v];
  name.value = "";
};
const ellipsis = (s) => (s.length > 14 ? s.slice(0, 14) + "…" : s);
const submittedText = computed(() => submitted.value.map(ellipsis).join(" · ") || "—");

// preventDefault() on keydown cancels the widget's editing, as in a browser.
const blocked = ref(0);
const lastKey = ref("—");
const guardDigits = (e) => {
  lastKey.value = JSON.stringify(e.key);
  if (e.key.length === 1 && e.key >= "0" && e.key <= "9") {
    e.preventDefault();
    blocked.value++;
  }
};
</script>

<template>
  <div class="card">
    <div class="title">Forms — native IME editor driven by stock v-model</div>
    <input
      class="field"
      v-model="name"
      tabindex="1"
      placeholder="type here, press Enter"
      @keydown="guardDigits"
      @keydown.enter="submit"
      @focus="focused = 'input'"
      @blur="focused = 'nothing'"
    />
    <div class="hint">v-model echo: "{{ name }}" ({{ name.length }} chars)</div>
    <div class="hint">Enter-submitted: {{ submittedText }}</div>
    <div class="hint">last e.key: {{ lastKey }} — digits blocked by preventDefault: {{ blocked }}</div>
    <div class="hint">focus → {{ focused }}</div>
  </div>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 14px;
  color: #ffffff;
}
.field {
  height: 32px;
  width: 280px;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  font-size: 13px;
  color: #ffffff;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
</style>
