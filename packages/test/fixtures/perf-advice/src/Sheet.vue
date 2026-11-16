<script setup lang="ts">
import { ref } from "vue";

const cols = Array.from({ length: 12 }, (_, i) => i);
const rows = Array.from({ length: 200 }, (_, i) => i);
const scrollY = ref(0);

function onScroll(e: Event): void {
  scrollY.value = (e.target as HTMLElement).scrollTop;
}
</script>

<template>
  <div class="sheet">
    <div class="heads">
      <div :style="{ marginTop: -scrollY + 'px' }">
        <div v-for="r in rows" :key="r" class="head">{{ r + 1 }}</div>
      </div>
    </div>
    <div class="scroller" @scroll="onScroll">
      <div v-for="r in rows" :key="r" class="row">
        <div v-for="c in cols" :key="c" class="cell">{{ r * cols.length + c }}</div>
      </div>
    </div>
  </div>
</template>

<style>
.sheet {
  display: flex;
  height: 100vh;
  font-size: 12px;
}
.heads {
  width: 40px;
  flex-shrink: 0;
  overflow: hidden;
}
.head,
.row {
  height: 24px;
  flex-shrink: 0;
}
.scroller {
  flex: 1;
  min-width: 0;
  overflow: auto;
}
.row {
  display: flex;
}
.cell {
  width: 64px;
  flex-shrink: 0;
}
</style>
