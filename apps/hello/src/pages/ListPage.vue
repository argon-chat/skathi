<script setup>
import { ref } from "vue";

const rows = Array.from({ length: 25 }, (_, i) => `row ${i + 1}`);
const scrollTop = ref(0);
const onScroll = (e) => (scrollTop.value = e.target.scrollTop);
const wheelY = ref(0);
</script>

<template>
  <div class="card" @wheel="wheelY = $event.deltaY">
    <div class="title">List — overflow: scroll + @scroll</div>
    <div class="scrollbox" tabindex="2" @scroll="onScroll">
      <div v-for="row in rows" :key="row" class="row">{{ row }}</div>
    </div>
    <div class="hint">scrollTop: {{ scrollTop.toFixed(0) }}px (wheel over the box)</div>
    <div class="hint">last wheel ΔY over the card: {{ wheelY.toFixed(1) }}</div>
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
.scrollbox {
  overflow: scroll;
  height: 200px;
  width: 280px;
  display: flex;
  flex-direction: column;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
}
.row {
  padding: 4px;
  font-size: 12px;
  color: #ffffffcc;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
</style>
