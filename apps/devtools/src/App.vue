<script setup>
// The inspector shell: titlebar + three panes, and the poll loops that keep
// them live.
//
// Everything below the titlebar is a view over `store.js`. This component owns
// only the CLOCKS, because the polling budget is a property of the whole app
// rather than of any one pane: every command costs the TARGET a frame, so the
// loops are slow, serialized (`wire.js`), and skipped while their previous run
// is still in flight.
//
//   picker   150 ms   only while picking
//   logs     700 ms   `logs since=<cursor>` — the cheapest command there is
//   tree    2500 ms   optional (`auto` in the Elements header)
//   styles   —        on selection, and on demand
//   preview  —        on demand only (see PreviewPane)
import { onMounted, onUnmounted } from "vue";
import Titlebar from "./components/Titlebar.vue";
import ElementsPane from "./components/ElementsPane.vue";
import StylesPane from "./components/StylesPane.vue";
import ConsolePane from "./components/ConsolePane.vue";
import PreviewPane from "./components/PreviewPane.vue";
import { state, attach, bestTarget, highlight, pickStep, pollLogs, refreshTargets, refreshTree } from "./store.js";

const SELECT_REFRESH_MS = 4000;

let timers = [];
const every = (ms, fn) => timers.push(setInterval(fn, ms));

onMounted(async () => {
  const found = refreshTargets();
  const target = bestTarget(found);
  if (target) await attach(target);

  every(150, () => pickStep());
  every(700, () => pollLogs());
  every(2500, () => {
    if (state.autoTree && state.status === "attached") refreshTree();
  });
  // The selection outline is deliberately mortal: the target forgets a
  // highlight it has not been told about for a few seconds, so an inspector
  // that crashes or is killed cannot leave a rectangle painted on somebody's
  // app. Re-arming it here is what makes it look permanent while we are alive.
  every(SELECT_REFRESH_MS, () => {
    if (state.selected !== null && state.status === "attached" && !state.picking) {
      highlight(state.selected, SELECT_REFRESH_MS + 2000);
    }
  });
  // A target list that never refreshes goes stale the moment somebody restarts
  // the app they are debugging — which is exactly when they look at it.
  every(3000, () => refreshTargets());
});

onUnmounted(() => {
  for (const t of timers) clearInterval(t);
  timers = [];
});
</script>

<template>
  <div class="app">
    <Titlebar />

    <div v-if="state.error" class="banner">{{ state.error }}</div>

    <div class="split">
      <ElementsPane class="elements" />
      <StylesPane class="styles" />
      <PreviewPane class="preview" />
    </div>

    <ConsolePane class="console" />
  </div>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  background: #0a0f1acc;
  color: #e6edf3;
  font-family: "Segoe UI", sans-serif;
}
.banner {
  padding: 5px;
  padding-left: 12px;
  font-size: 11px;
  color: #ff7b72;
  background: #ff7b7217;
  border-bottom-width: 1px;
  border-color: #ff7b7233;
}
/* The shell owns every pane's size; the panes themselves only say `min-height:
   0` so their scroll boxes can shrink below their content. A pane that also
   declared a height of its own would collide with these rules at the same
   specificity and the winner would be whichever module the builder emitted
   last. */
.split {
  display: flex;
  flex-direction: row;
  flex: 1;
  min-height: 0;
}
.elements {
  width: 38%;
  min-width: 0;
}
.styles {
  flex: 1;
  min-width: 0;
}
.preview {
  width: 250px;
}
.console {
  height: 260px;
  min-height: 0;
}
</style>
