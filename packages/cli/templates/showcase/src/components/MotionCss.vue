<script setup lang="ts">
import { ref } from "vue";
import Card from "./Card.vue";

const open = ref(false);
const active = ref(false);
const tiles = ref(["A", "B", "C", "D"]);
</script>

<template>
  <Card title="Panel transitions" api="@starting-style · allow-discrete">
    <div class="row">
      <button class="btn" aria-controls="css-motion-panel" :aria-expanded="open" @click="open = !open">
        {{ open ? "Close panel" : "Open panel" }}
      </button>
    </div>
    <div class="css-motion-stage">
      <div id="css-motion-panel" class="css-motion-panel" role="region" aria-label="Draft status" :hidden="!open">
        Draft saved
      </div>
    </div>
  </Card>

  <Card title="Staggered transitions" api="sibling-index() · sibling-count()">
    <div class="row">
      <button class="btn" :aria-pressed="active" @click="active = !active">Move tiles</button>
      <button class="btn" @click="tiles = [...tiles].reverse()">Reverse order</button>
    </div>
    <div class="row gap css-motion-tiles" :class="{ 'css-motion-tiles-active': active }">
      <div v-for="tile in tiles" :key="tile" class="css-motion-tile">{{ tile }}</div>
    </div>
  </Card>
</template>

<style>
.css-motion-stage {
  height: 62px;
}
.css-motion-panel {
  display: block;
  width: 220px;
  padding: 12px 14px;
  background: var(--surface-strong);
  border: 1px solid var(--line);
  border-radius: 8px;
  opacity: 1;
  translate: 0;
  transition:
    opacity 260ms ease,
    translate 260ms ease,
    display 260ms allow-discrete;
}
@starting-style {
  .css-motion-panel {
    opacity: 0;
    translate: 0 8px;
  }
}
.css-motion-panel[hidden] {
  display: none;
  opacity: 0;
  translate: 0 8px;
}
.css-motion-tiles {
  height: 62px;
  align-items: center;
}
.css-motion-tile {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  background: var(--surface-strong);
  border: 1px solid var(--line);
  border-radius: 7px;
  transition:
    translate 220ms ease,
    rotate 220ms ease;
  transition-delay: calc(sibling-index() * 55ms);
}
.css-motion-tiles-active .css-motion-tile {
  translate: 0 -8px;
  rotate: calc(180deg / sibling-count() * sibling-index());
}
</style>
