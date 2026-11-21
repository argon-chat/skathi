<script setup lang="ts">
import { ref } from "vue";
import { SkWindow, type SkWindowBounds } from "@skathi-native/runtime";
import { appWindow } from "skathi:app";

const colors = ["#e5484d", "#f76b15", "#ffc53d", "#30a46c", "#0090ff", "#8e4ec6"];
const ink = ref("#0090ff");
const open = ref(false);
const bounds = ref<SkWindowBounds>({ width: 200, height: 260 });

function showPalette() {
  if (navigator.platform === "Win32") {
    const main = appWindow.getBounds();
    bounds.value = { x: main.x + main.width + 8, y: main.y, width: 200, height: 260 };
  }
  open.value = true;
}
</script>

<template>
  <main class="editor">
    <div class="canvas" :style="{ background: ink }" />
    <button v-if="!open" @click="showPalette">Show palette</button>
    <SkWindow v-if="open" title="Palette" :bounds="bounds" :taskbar="false" :min-width="160" @close="open = false">
      <div class="swatches">
        <button
          v-for="c in colors"
          :key="c"
          class="swatch"
          :class="{ on: c === ink }"
          :style="{ background: c }"
          @click="ink = c"
        />
      </div>
    </SkWindow>
  </main>
</template>

<style>
.editor {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 24px;
}
.canvas {
  height: 240px;
  border-radius: 12px;
}
.swatches {
  display: grid;
  grid-template-columns: repeat(3, 48px);
  gap: 10px;
  padding: 16px;
}
.swatch {
  width: 48px;
  height: 48px;
  border-radius: 24px;
}
.swatch.on {
  border: 3px solid #ffffff;
}
</style>
