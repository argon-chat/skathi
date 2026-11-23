<script setup lang="ts">
const rows = Array.from({ length: 40 }, (_, i) => ({
  id: i,
  name: ["Jonah", "Sana", "Mira"][i % 3]!,
}));
</script>

<template>
  <div class="scroll-page">
    <div id="scroller" class="scroller">
      <div v-for="r in rows" :key="r.id" :id="`row-${r.id}`" class="srow">
        <span class="face">{{ r.name[0] }}</span>
        <div class="body">
          <div class="meta">
            <b>{{ r.name }}</b
            ><span class="time">10:{{ 10 + r.id }}</span>
          </div>
          <p>Row {{ r.id }} stays one rigid block while it scrolls.</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
.scroll-page {
  position: absolute;
  inset: 0;
  padding: 23px 31px;
}
.scroller {
  width: 600px;
  height: 500px;
  overflow-y: auto;
  border: 1px solid var(--border);
}
.srow {
  padding: 9px 13px;
  display: flex;
  gap: 12px;
  align-items: center;
}
.srow .face {
  width: 31px;
  height: 31px;
  border-radius: 15.5px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #9fcdf5;
  color: var(--ink);
  font-size: 12px;
  font-weight: 700;
}
.srow p {
  margin: 0;
  color: var(--body);
  font-size: 13px;
}
</style>
