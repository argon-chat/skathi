<script setup lang="ts">
// The target's window, on demand.
//
// NOT a live preview: `screenshot` re-renders the target's last scene into its
// render target and reads it back through a staging texture, then encodes a PNG
// — by far the most expensive command on the wire. A 2 fps "live" pane would
// cost the app under inspection more than everything else here put together, so
// this is a button and a timestamp.
//
// The image is a `file://` URL into the target's own dist slot. Each capture
// lands on a fresh path (the host rotates `1.png`, `2.png`, …), which is also
// what stops gpui's image cache — which never evicts — from handing back the
// previous frame under the same key.
import { computed } from "vue";
import { state, capture } from "../store";

const taken = computed(() => {
  if (!state.shot) return "";
  const secs = Math.round((Date.now() - state.shot.at) / 1000);
  return secs < 2 ? "just now" : `${secs}s ago`;
});
</script>

<template>
  <section class="pane">
    <header class="head">
      <span class="title">Preview</span>
      <span v-if="state.shot" class="meta">{{ state.shot.width }}×{{ state.shot.height }} · {{ taken }}</span>
      <div class="spacer" />
      <div class="btn" @click="capture">{{ state.shotBusy ? "capturing…" : "capture" }}</div>
      <div class="btn" @click="state.shot = null" v-if="state.shot">clear</div>
    </header>
    <div class="body">
      <div v-if="state.shotError" class="error">{{ state.shotError }}</div>
      <img v-if="state.shot" :src="state.shot.url" class="shot" />
      <div v-else class="empty">
        capture the target window — the client area exactly, unfocused and occluded, never raised
      </div>
      <div v-if="state.shot && state.shot.hasAlpha" class="note">
        the capture has transparent pixels: this window uses a mica/acrylic backdrop, which DWM
        composites UNDER the app — the readback cannot see it
      </div>
    </div>
  </section>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  /* NO `height: 100%` here: the SHELL sizes the panes (App.vue), and a
     `height` on a component root would collide with the parent's rule at the
     same specificity — whichever module the builder emitted last would win,
     which is not a layout, it is a coin flip. */
  min-height: 0;
  border-left-width: 1px;
  border-color: #ffffff14;
}
.head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding-left: 10px;
  padding-right: 8px;
  height: 30px;
  background: #ffffff08;
  border-bottom-width: 1px;
  border-color: #ffffff14;
}
.title {
  font-size: 11px;
  color: #cfe1ff;
}
.meta {
  font-size: 10px;
  color: #ffffff59;
}
.spacer {
  flex: 1;
}
.btn {
  font-size: 10px;
  color: #ffffff99;
  padding-left: 7px;
  padding-right: 7px;
  padding-top: 2px;
  padding-bottom: 2px;
  border-radius: 4px;
  border-width: 1px;
  border-color: #ffffff26;
  cursor: pointer;
}
.btn:hover {
  background: #ffffff1a;
  color: #ffffff;
}
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px;
}
.shot {
  width: 100%;
  object-fit: contain;
  border-radius: 4px;
  border-width: 1px;
  border-color: #ffffff26;
}
.empty {
  font-size: 11px;
  color: #ffffff70;
}
.error {
  font-size: 10px;
  color: #ff7b72;
  padding: 6px;
  background: #ff7b721a;
  border-radius: 4px;
}
.note {
  margin-top: 6px;
  font-size: 10px;
  color: #ffd479;
}
</style>
