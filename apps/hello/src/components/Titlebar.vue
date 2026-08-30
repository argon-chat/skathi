<script setup lang="ts">
// The app's own titlebar — docs/WINDOWING.md §12, the worked example.
//
// Two facts make it work, and both are CSS:
//   `window-drag: move`        on the strip   → the OS treats it as HTCAPTION,
//                                               so dragging moves the window,
//                                               double-click maximizes, and
//                                               right-click opens the system
//                                               window menu. All for free.
//   `--window-control: <role>` on each button → the OS answers WM_NCHITTEST
//                                               with HTMIN/HTMAX/HTCLOSE, which
//                                               is what buys the Windows 11
//                                               SNAP LAYOUTS flyout on hover
//                                               over the maximize button.
//
// There is deliberately NO @click on the three caption buttons: gpui performs
// the action itself on the non-client click AND forwards a synthesised one into
// the tree, so a handler would make the window act twice. (Doing both is a real
// mistake, so it fires advisory A-14 rather than silently picking a winner.)
//
// The GEOMETRY is CSS too, now that `env()` is implemented: the strip's height
// and its left inset come from `env(titlebar-area-height)` / `env(titlebar-area-x)`
// in the <style> block below — the same host-side rect
// `navigator.windowControlsOverlay.getTitlebarAreaRect()` reports, so the two
// halves cannot disagree and there is no inline style binding to keep in sync.
// (Before `env()` landed this component bound `:style` from the JS getter and
// ate advisory A-15; §2.4 promised the shape would not have to change, and it
// did not — only the fallbacks in `env(…, 36px)` stopped being used.)
//
// The `.controls` cluster is the APP's. Turning on `titleBarOverlay` (see the
// Window page) makes the HOST draw one instead, in which case the WCO rect
// loses its width and this strip's own buttons must go — which is what the
// `v-if` does. Reading the rect rather than assuming a side is also what would
// keep this correct on macOS, where the traffic lights sit LEFT, and under RTL.
import { computed, onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "vue-native";

// `appWindow.chrome` is `Record<string, unknown>` — the prelude builds it, and
// the runtime has no shape for it. `hostControls` is the "the host draws the
// caption cluster" flag (docs/WINDOWING.md §2.3), which is a boolean.
const hostControls = ref(Boolean(appWindow.chrome.hostControls));
const maximized = ref(appWindow.isMaximized());
// engine.d.ts types `document` as `VnDocument | undefined` because the bun
// test harness has none. Under vn-app the runtime installs it before this
// module evaluates, so it is always there — asserted once, here.
const doc = document!;
const wco = navigator!.windowControlsOverlay;
const title = ref(doc.title);

// Caption glyphs. Plain Unicode rather than the Segoe Fluent Icons private-use
// codepoints the OS caption uses, so this renders with whatever font is
// actually installed.
const MIN = "–";
const CLOSE = "✕";
const maxGlyph = computed(() => (maximized.value ? "❐" : "□"));

const readGeometry = () => {
  hostControls.value = Boolean(appWindow.chrome.hostControls);
};
const readState = () => {
  maximized.value = appWindow.isMaximized();
  title.value = doc.title;
};

onMounted(() => {
  readGeometry();
  // WICG's own note is that `geometrychange` fires "as frequently as a scroll
  // listener" — debounce it in anything heavier than this.
  wco.addEventListener("geometrychange", readGeometry);
  appWindow.addEventListener("maximize", readState);
  appWindow.addEventListener("unmaximize", readState);
  // `document.title` has no change event in any spec (it is a plain property),
  // so the editor on the Window page pokes this custom one.
  globalThis.addEventListener("vn:title", readState);
});
onUnmounted(() => {
  wco.removeEventListener("geometrychange", readGeometry);
  appWindow.removeEventListener("maximize", readState);
  appWindow.removeEventListener("unmaximize", readState);
  globalThis.removeEventListener("vn:title", readState);
});
</script>

<template>
  <header class="titlebar">
    <span class="mark">◆</span>
    <span class="title">{{ title }}</span>
    <div v-if="!hostControls" class="controls">
      <div class="ctl min" title="Minimize">{{ MIN }}</div>
      <div class="ctl max" :title="maximized ? 'Restore' : 'Maximize'">{{ maxGlyph }}</div>
      <div class="ctl close" title="Close">{{ CLOSE }}</div>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  /* The CSS half of Window Controls Overlay (§2.4). Same host-side rect the JS
     getter reads; the fallbacks are what a browser without WCO would use. When
     the host draws its own cluster the rect narrows and this strip's own
     buttons step aside — `titlebar-area-width` is what says so. */
  height: env(titlebar-area-height, 36px);
  width: env(titlebar-area-width, 100%);
  /* Zero on Windows (buttons right); non-zero on macOS, where the traffic
     lights sit left. Assuming a side is the bug this reads its way out of. */
  padding-left: env(titlebar-area-x, 0px);
  /* The whole strip drags the window. Inherited, so every descendant is
     draggable until one says otherwise. */
  window-drag: move;
  /* Text selection fights a drag — every drag region wants this. */
  user-select: none;
  background: #ffffff0a;
  /* `border-bottom` (the shorthand) is not mapped in vn-css v1 and advises;
     the longhands are. */
  border-bottom-width: 1px;
  border-color: #ffffff1f;
}
.mark {
  margin-left: 12px;
  font-size: 12px;
  color: #7aa2ff;
}
.title {
  flex: 1;
  font-size: 12px;
  color: #ffffffb0;
}
.controls {
  display: flex;
  flex-direction: row;
  height: 100%;
  /* The carve-out: this box sits INSIDE the `move` region and opts out, which
     is what `window-drag: none` is for. The padding makes it observable — the
     10px strip left of the first button answers HTCLIENT while the rest of the
     strip answers HTCAPTION. */
  padding-left: 10px;
  window-drag: none;
}
.ctl {
  width: 46px;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  color: #ffffffcc;
  transition: background-color 120ms ease, color 120ms ease;
}
/* Once an element claims HTMAXBUTTON the system stops drawing hover feedback
   for you — losing it is the classic symptom of a half-done custom titlebar.
   Ours is painted by this stylesheet, off the NC mouse-move gpui forwards into
   the content tree. */
.ctl:hover {
  background: #ffffff1a;
}
.min {
  --window-control: minimize;
}
.max {
  --window-control: maximize;
}
.close {
  --window-control: close;
}
.close:hover {
  background: #c42b1c;
  color: #ffffff;
}
</style>
