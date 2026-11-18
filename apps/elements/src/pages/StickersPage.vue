<script setup lang="ts">
type Tile = { src: string; tint?: string; hero?: boolean };
const s = (name: string) => `assets/stickers/${name}`;
const tiles: Tile[] = [
  { src: s("heart.json"), hero: true },
  { src: s("bounce.json") },
  { src: s("dots.json") },
  { src: s("flower.json") },
  { src: s("sparkle.json") },
  { src: s("face.json") },
  { src: s("loader.json") },
  { src: s("bars.json") },
  { src: s("star.tgs") },
  { src: s("heart.json"), tint: "#c9a7f5" },
  { src: s("flower.json") },
  { src: s("bounce.json"), tint: "#f7a8d0" },
  { src: s("sparkle.json"), tint: "#9fcdf5" },
  { src: s("face.json") },
  { src: s("bars.json") },
];
const sources = new Set(tiles.map((t) => t.src)).size;
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;sk-sticker&gt;</span>
      <p>{{ tiles.length }} stickers from {{ sources }} sources. Copies of a source share one decode and one clock.</p>
    </div>
    <div class="stage">
      <div class="grid">
        <div v-for="(t, i) in tiles" :key="i" class="cell" :class="{ hero: t.hero }">
          <sk-sticker
            class="sticker"
            :src="t.src"
            :tint="!!t.tint"
            :style="t.tint ? { color: t.tint } : undefined"
            play="on"
          />
          <span v-if="t.tint" class="badge">tint</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.grid {
  display: grid;
  grid-template-columns: repeat(6, 150px);
  grid-auto-rows: 150px;
  gap: 16px;
}
.cell {
  position: relative;
  border-radius: 22px;
  background: var(--surface);
  border: 1px solid var(--border);
  display: flex;
  align-items: center;
  justify-content: center;
}
.cell.hero {
  grid-column: span 2;
  grid-row: span 2;
  background: radial-gradient(circle at 50% 45%, #4a2f4a99 0%, var(--surface) 70%);
}
.sticker {
  width: 108px;
  height: 108px;
}
.hero .sticker {
  width: 250px;
  height: 250px;
}
.badge {
  position: absolute;
  right: 10px;
  top: 10px;
  padding: 1px 7px;
  border-radius: 6px;
  background: #2a2733;
  color: var(--muted);
  font-family: var(--mono);
  font-size: 11px;
}
</style>
