<script setup lang="ts">
import { useTemplateRef } from "vue";

const columns = Array.from({ length: 30 }, (_, i) => `Column ${i + 1}`);
const rows = Array.from({ length: 40 }, (_, r) => columns.map((_, c) => `${r + 1}:${c + 1}`));

// #region sync
const header = useTemplateRef<HTMLElement>("header");

function onScroll(e: Event) {
  const left = (e.currentTarget as HTMLElement).scrollLeft;
  if (header.value) header.value.style.transform = `translateX(${-left}px)`;
}
// #endregion sync
</script>

<template>
  <div class="table">
    <div class="header-clip">
      <div ref="header" class="header">
        <div v-for="name in columns" :key="name" class="cell">{{ name }}</div>
      </div>
    </div>
    <div class="body" @scroll="onScroll">
      <div v-for="(row, r) in rows" :key="r" class="row">
        <div v-for="(text, c) in row" :key="c" class="cell">{{ text }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.header-clip {
  overflow: hidden;
}
.header,
.row {
  display: flex;
}
.body {
  height: 360px;
  overflow: auto;
}
.cell {
  flex: none;
  width: 100px;
  height: 24px;
  padding: 0 6px;
  line-height: 24px;
}
</style>
