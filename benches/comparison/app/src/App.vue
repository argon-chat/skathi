<script setup lang="ts">
import { onMounted, ref } from "vue";

const count = ref(0);
const rows = Array.from({ length: 1000 }, (_, i) => `Row ${i + 1}`);
const list = ref<HTMLElement | null>(null);

function signal(state: string) {
  document.title = `Compare ${state}`;
}

function scroll() {
  const el = list.value;
  if (!el) return;
  let pos = 0;
  const id = setInterval(() => {
    pos += 100;
    el.scrollTop = pos;
    if (pos >= el.scrollHeight - el.clientHeight) {
      clearInterval(id);
      setTimeout(() => signal("scrolled"), 500);
    }
  }, 16);
}

onMounted(() => {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      signal("ready");
      setTimeout(scroll, 20000);
    }),
  );
});
</script>

<template>
  <main class="page">
    <h1>Compare</h1>
    <button class="button" @click="count++">Count: {{ count }}</button>
    <div ref="list" class="list">
      <div v-for="row in rows" :key="row" class="row">{{ row }}</div>
    </div>
  </main>
</template>

<style>
.page {
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  box-sizing: border-box;
  font-family: system-ui, sans-serif;
  color: #18181b;
  background: #ffffff;
}
h1 {
  margin: 0;
  font-size: 24px;
}
.button {
  align-self: flex-start;
  padding: 8px 16px;
  border-radius: 6px;
  border: 1px solid #a1a1aa;
  background: #f4f4f5;
  font-size: 14px;
}
.list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  border: 1px solid #e4e4e7;
}
.row {
  height: 28px;
  line-height: 28px;
  padding: 0 12px;
  border-bottom: 1px solid #f4f4f5;
  font-size: 14px;
}
</style>
