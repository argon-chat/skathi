<script setup lang="ts">
// The multi-window SPIKE — docs/MULTIWINDOW.md.
//
// There is ONE Vue app here. `panel` is an ordinary `<section>` in an ordinary
// template; nothing about it is special until `__vn_window_open(panel.id, …)`
// is called, at which point the host stops painting it in THIS window and opens
// a second OS window that paints that same node instead.
//
// The proof this page exists to show is the pair of counters: `count` is read
// both inside the panel (which ends up in window B) and outside it (which stays
// in window A). Clicking `+1` in window B runs the same handler on the same
// `ref` and both numbers move in the same frame group.
//
// `__vn_window_open` is a raw host fn on purpose. The app-facing spelling —
// `<Teleport to="vn:window">` or `<VnWindow>` — is the TypeScript runtime's
// job; its contract is docs/MULTIWINDOW.md §3. What a template ref hands you is
// the mirror node, and `node.id` IS the retained-tree id the host addresses, so
// no runtime change was needed to spike this.
import { ref } from "vue";

const count = ref(0);
// `ref="panel"` holds the MIRROR NODE for that `<section>`, and `.id` on it is
// the retained-tree id the host addresses — MULTIWINDOW.md §5.2 invariant 1.
// `VnElement` is apps/env.d.ts's name for what a template ref to a native
// element is holding; `runtime.ts`'s own `MirrorElement` is not exported.
const panel = ref<VnElement | null>(null);
const out = ref(false);

function popOut() {
  if (out.value || !panel.value) return;
  // Placed to the RIGHT of window A rather than centred, so a screenshot can
  // show both at once. `screenX`/`outerWidth` are the tier-1 geometry reads
  // that already work (docs/WINDOWING.md §1.2).
  const options = JSON.stringify({
    title: "vue-native — pop-out (window B)",
    x: Math.round(screenX + outerWidth + 12),
    y: Math.round(screenY),
    width: 380,
    height: 300,
  });
  __vn_window_open(panel.value.id, options, () => {
    // Fired from the host when the window closes, however it closed.
    out.value = false;
  });
  out.value = true;
}

function bringBack() {
  if (!out.value || !panel.value) return;
  __vn_window_close(panel.value.id);
}
</script>

<template>
  <main class="page">
    <h1 class="h">One app, two windows</h1>

    <p class="lede">
      The panel below lives in this component's template. Pop it out and the
      host paints it in a second OS window — same node id, same component
      instance, same reactive graph.
    </p>

    <!-- Stays in window A. Its whole job is to disagree with nothing. -->
    <div class="mirror">
      window A sees count = <span class="num">{{ count }}</span>
    </div>

    <!-- Distinct classes rather than positions: `v-show` keeps a hidden button
         in the tree (`display: none`, as in a browser), so nth-of-type would
         count the invisible ones too. -->
    <div class="row">
      <button class="btn pop" @click="popOut" v-show="!out">pop the panel out →</button>
      <button class="btn back" @click="bringBack" v-show="out">← bring it back</button>
      <button class="btn inc-a" @click="count++">+1 (from window A)</button>
    </div>

    <p class="hint" v-show="out">
      The panel is being painted by window B. It is still in this tree.
    </p>

    <!-- The subtree that moves. -->
    <section class="panel" ref="panel">
      <div class="ptitle">the panel</div>
      <div class="count">{{ count }}</div>
      <button class="btn big inc-b" @click="count++">+1</button>
      <div class="foot">clicks here update window A too</div>
    </section>
  </main>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px;
  color: #ffffffe6;
  font-size: 13px;
}
.h {
  font-size: 19px;
  color: #ffffff;
}
.lede {
  color: #ffffffa0;
  line-height: 19px;
}
.mirror {
  padding-top: 10px;
  padding-bottom: 10px;
  padding-left: 14px;
  padding-right: 14px;
  border-radius: 10px;
  background: #ffffff0d;
  color: #ffffffc0;
}
.num {
  color: #7aa2ff;
  font-size: 15px;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.btn {
  padding-top: 6px;
  padding-bottom: 6px;
  padding-left: 12px;
  padding-right: 12px;
  border-radius: 8px;
  background: #ffffff14;
  color: #ffffffe6;
  font-size: 12px;
}
.btn:hover {
  background: #ffffff26;
}
.btn.big {
  font-size: 15px;
  padding-top: 8px;
  padding-bottom: 8px;
  padding-left: 20px;
  padding-right: 20px;
}
.hint {
  color: #7aa2ff;
  font-size: 12px;
}
.panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 18px;
  border-radius: 12px;
  background: #7aa2ff1f;
  color: #ffffffe6;
}
.ptitle {
  color: #ffffff90;
  font-size: 12px;
}
.count {
  font-size: 34px;
  color: #ffffff;
}
.foot {
  color: #ffffff70;
  font-size: 11px;
}
</style>
