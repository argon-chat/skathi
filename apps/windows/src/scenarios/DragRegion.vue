<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "@skathi-native/runtime";
import Scenario from "../components/Scenario.vue";

const pos = ref("");
const read = () => (pos.value = `${Math.round(screenX)}, ${Math.round(screenY)}`);
onMounted(() => {
  read();
  appWindow.addEventListener("move", read);
});
onUnmounted(() => appWindow.removeEventListener("move", read));
</script>

<template>
  <Scenario id="title-bar/drag-region">
    <div class="card">
      <div class="card-title">The title bar above, piece by piece</div>
      <div class="diagram">
        <div class="zone drag">app-region: drag, the mark and title included</div>
        <div class="zone caption">--window-control</div>
      </div>
      <p class="muted">
        The whole bar is a drag region. Everything empty in it moves the window; the caption buttons are carved out by
        --window-control and answer the OS as real minimize, maximize and close buttons. Any button you add to the bar
        needs app-region: no-drag.
      </p>
    </div>

    <div class="card reads">
      <div class="read">
        <span class="k">screenX, screenY</span><span class="v">{{ pos }}</span>
      </div>
      <div class="read">
        <span class="k">isMaximized()</span><span class="v">{{ appWindow.isMaximized() }}</span>
      </div>
      <p class="muted">The position updates from the move event, once per frame while you drag.</p>
    </div>
  </Scenario>
</template>

<style scoped>
.diagram {
  height: 44px;
  display: flex;
  gap: 4px;
}
.zone {
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  font-family: var(--mono);
  font-size: 12px;
}
.drag {
  flex: 1;
  border: 1px dashed var(--accent);
  background: var(--accent-soft);
  color: var(--accent-text);
}
.caption {
  width: 160px;
  background: var(--well);
  color: var(--text-2);
}
.reads {
  gap: 10px;
}
.read {
  display: flex;
  align-items: center;
  gap: 16px;
}
.k {
  width: 200px;
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text-3);
}
.v {
  font-size: 14px;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
</style>
