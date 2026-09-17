<script setup lang="ts">
// MULTI-WINDOW, phase 1 — docs/MULTIWINDOW.md.
//
// There is ONE Vue app on this page. Every `<VnWindow>` below paints part of
// THIS component's template in a second OS window: same node ids, same
// component instances, same reactive graph, same `count` ref. Nothing is
// remounted when a window opens, nothing is serialized across it, and there is
// no store bridge — a click in a pop-out runs the same handler on the same
// `ref`, and both windows repaint from that one wave.
//
// The three pop-outs are the three cases phase 1 exists to make real:
//
//   1. a TOOL PALETTE — always on top, editing this document's state. Its
//      `+1` moves the counter painted in the main window; its `<input>` holds
//      real keyboard focus in its own window (which needed the focus trio to
//      become per-window, §5.1). It is built with `<Teleport :to="win.el">`,
//      so the panel MOVES between the windows and comes back when the palette
//      closes — the same nodes, the same state, the same instance.
//   2. a VIDEO PLAYER — the `<video>` / `<lottie>` case the spike REFUSED,
//      because a decoded frame's atlas tile has to be returned to the window
//      that created it and `Window::drop_image` is per-window (§7.4, A-M06).
//   3. a NOTES window with its OWN TITLEBAR — `chrome="hidden"`, so the app
//      paints the caption and a `window-drag: move` region moves the window.
//      That needed `control_hitboxes` / `any_drag_region` / `os_chrome` to
//      become per-window too (§7.6).
//
// `<VnWindow>` is tier 3 and namespaced: an app opts in by importing it.
// `window.open` is untouched, still reserved and still advises (A-22) — it
// means "a second document", which is a different feature (D-M04).
import { computed, ref } from "vue";
import { VnWindow, type VnWindowMetrics, type VnWindowTarget } from "vue-native";

const count = ref(0);

// Which pop-outs are open. `v-if` IS the lifecycle: mounting `<VnWindow>` opens
// the window, unmounting closes it, and the window's own X button fires
// `@close` so the flag goes back to false. One direction, one source of truth.
const palette = ref(false);
const player = ref(false);
const notes = ref(false);

// State the palette EDITS and the main window READS. This is the whole reason a
// tool palette wants to be one app rather than two: `brush` is an ordinary
// `ref` here, not a message on a channel.
const brush = ref("charcoal");
const swatches = ["charcoal", "rust", "moss", "slate"];

// D-M03: `<VnWindow>` exposes its container node, so an ordinary `<Teleport>`
// composes onto it. **That is the browser spelling** — in a browser,
// `window.open()` supplies an element and `<Teleport :to="w.document.body">`
// does the moving; nothing about the Teleport is special. Here the target is a
// mirror node instead of a DOM node, and the Teleport is untouched.
//
// It also buys the semantics a slot cannot: the panel is not created when the
// window opens and destroyed when it closes — it MOVES, keeping its node ids,
// its component instance and its state, and comes back to window A when the
// palette goes away.
const paletteWin = ref<{ el: VnWindowTarget | null } | null>(null);
const paletteRoot = computed(() => paletteWin.value?.el ?? null);

// What the palette window reports about ITSELF. There is one JS realm, so
// `window.innerWidth` keeps meaning the MAIN window's viewport (A-M04); a
// pop-out's own metrics arrive through `@resize` instead, which is also what
// `<VnWindow>` exposes as `width`/`height`. CSS needs none of this: `@media`
// inside a pop-out already answers against that window (§6).
const paletteSize = ref<VnWindowMetrics>({ width: 0, height: 0, scale: 1 });
const paletteFocused = ref(false);
const paletteMetrics = computed(() =>
  paletteSize.value.width
    ? `${Math.round(paletteSize.value.width)}x${Math.round(paletteSize.value.height)}`
    : "none",
);

// The notes window's own caption text is CSS, not an OS caption.
const note = ref("");

/** A live prop: renaming the window renames the OS caption. */
const paletteTitle = computed(() => `Palette - ${brush.value}`);
</script>

<template>
  <main class="page">
    <h1 class="h">One app, three windows</h1>

    <p class="lede">
      Everything below lives in this one component. A VnWindow moves a subtree's
      PAINT into a second OS window and leaves it in this tree.
    </p>

    <!-- Stays in window A. Its whole job is to disagree with nothing. -->
    <div class="card mirror">
      window A sees count = <span class="num">{{ count }}</span>
    </div>
    <div class="card meta">
      brush = <span class="num brush-mirror">{{ brush }}</span>
      · palette viewport = <span class="num pal-size">{{ paletteMetrics }}</span>
      · focused = <span class="num pal-focus">{{ paletteFocused }}</span>
    </div>

    <!-- Distinct classes rather than positions: `v-show` keeps a hidden button
         in the tree (`display: none`, as in a browser), so nth-of-type would
         count the invisible ones too. -->
    <div class="row">
      <button class="btn pop" @click="palette = true" v-show="!palette">pop the panel out</button>
      <button class="btn back" @click="palette = false" v-show="palette">bring it back</button>
      <button class="btn inc-a" @click="count++">+1 (from window A)</button>
    </div>

    <div class="row">
      <button class="btn pop-player" @click="player = !player">
        {{ player ? "close the player" : "pop out the player" }}
      </button>
      <button class="btn pop-notes" @click="notes = !notes">
        {{ notes ? "close the notes" : "pop out the notes" }}
      </button>
    </div>

    <!-- **The per-window `@keyframes` proof** (§6). One `animation-name`,
         declared twice in one sheet — once at the top level and once inside the
         same `@media` block the panel's background uses. This one is painted by
         the 620px main window, the one below by the 380px palette, and they
         settle on different opacities. -->
    <div class="pulse pulse-main" />

    <p class="hint" v-show="palette">
      The panel is being painted by the palette window. It is still in this tree.
    </p>

    <div class="card note-mirror" v-show="notes">
      note = <span class="num note-text">{{ note || "(empty)" }}</span>
    </div>

    <!-- `disabled` while there is no window: the panel renders HERE, in window
         A, exactly where it is written. The moment the target exists Vue moves
         the same nodes into it, and moves them back when it goes away.
         ORDER MATTERS, and it is the one subtlety of the whole idiom: this
         Teleport is written BEFORE the `<VnWindow>` it targets, and `disabled`
         is driven by `palette` as well as by the target. Vue patches a
         component's children in order, so on close the Teleport moves the panel
         home FIRST and the container is unmounted empty. The other order
         removes the container while the panel is still inside it — and in
         vue-native, as in the DOM, removing a node destroys its subtree. -->
    <Teleport :to="paletteRoot" :disabled="!palette || !paletteRoot">
      <section class="panel">
        <div class="ptitle">the panel</div>
        <div class="count">{{ count }}</div>
        <button class="btn big inc-b" @click="count++">+1</button>

        <div class="swatches">
          <button
            v-for="s in swatches"
            :key="s"
            class="swatch"
            :class="{ on: brush === s }"
            @click="brush = s"
          >
            {{ s }}
          </button>
        </div>

        <!-- A real `<input>` in a second window: it takes keyboard focus in
             THAT window and the keystrokes reach it, which is what the
             per-window focus trio bought (§7.3). -->
        <input class="brush-name" v-model="brush" />

        <div class="foot">clicks and keystrokes here update window A too</div>
      </section>
    </Teleport>

    <!-- ================================================================== -->
    <!-- 1. the tool palette                                                -->
    <!-- ================================================================== -->
    <!-- `alwaysOnTop` because that is what a palette is. It does NOT take the
         keyboard when it opens (`activate` defaults to false) — the user was
         typing in the document and must not lose the caret. -->
    <VnWindow
      v-if="palette"
      ref="paletteWin"
      :title="paletteTitle"
      :bounds="{ x: 60, y: 470, width: 380, height: 320 }"
      always-on-top
      :min-width="240"
      :min-height="200"
      @close="palette = false"
      @resize="(m: VnWindowMetrics) => (paletteSize = m)"
      @focus="paletteFocused = true"
      @blur="paletteFocused = false"
    >
      <!-- A slot child rather than the Teleport'd panel: this node is CREATED
           with the window, so the run starts in the window that paints it. A
           run that is later teleported into another window re-resolves its
           block in place, which is what a browser does when a media query
           flips under a running animation. -->
      <div class="pulse pulse-palette" />
    </VnWindow>


    <!-- ================================================================== -->
    <!-- 2. the video pop-out                                               -->
    <!-- ================================================================== -->
    <!-- The case the spike refused. A decoded frame becomes an atlas TILE, and
         `Window::drop_image` is per-window: the tile has to go back to the
         window that created it, or it is freed from every atlas except the one
         holding it. Phase 1 records the creating window on the element, so this
         plays here. Slot children rather than a Teleport, which is the other
         legitimate shape: this subtree is CREATED with the window. -->
    <VnWindow
      v-if="player"
      title="Player"
      :bounds="{ x: 470, y: 470, width: 360, height: 280 }"
      @close="player = false"
    >
      <section class="player">
        <video class="clip" src="assets/clip-tiny.mp4" autoplay loop muted />
        <div class="foot">a video decoding into THIS window's atlas</div>
      </section>
    </VnWindow>

    <!-- ================================================================== -->
    <!-- 3. the notes window, with a titlebar the app draws                 -->
    <!-- ================================================================== -->
    <!-- `chrome="hidden"` means the OS draws no caption, so `.bar` below IS the
         titlebar: `window-drag: move` moves the window and `--window-control`
         gives the close button the OS's own behaviour, including the Windows 11
         snap-layouts flyout on maximise. Exactly the contract
         `titleBarStyle: "hidden"` has for the main window — and it needed the
         hitbox list to stop being the primary window's (§7.6). -->
    <VnWindow
      v-if="notes"
      title="Notes"
      chrome="hidden"
      :bounds="{ x: 860, y: 470, width: 320, height: 260 }"
      :title-bar-height="34"
      @close="notes = false"
    >
      <section class="notes">
        <div class="bar">
          <span class="bar-title">Notes</span>
          <span class="bar-close" @click="notes = false">close</span>
        </div>
        <textarea class="note-input" v-model="note" />
        <div class="foot">this window's caption is CSS</div>
      </section>
    </VnWindow>
  </main>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 10px;
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
.card {
  padding-top: 10px;
  padding-bottom: 10px;
  padding-left: 14px;
  padding-right: 14px;
  border-radius: 10px;
  background: #ffffff0d;
  color: #ffffffc0;
}
.meta {
  color: #ffffff90;
  font-size: 12px;
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

/* ---- the palette ------------------------------------------------------- */
.panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 16px;
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
.swatches {
  display: flex;
  flex-direction: row;
  gap: 6px;
}
.swatch {
  padding-top: 4px;
  padding-bottom: 4px;
  padding-left: 8px;
  padding-right: 8px;
  border-radius: 6px;
  background: #ffffff14;
  color: #ffffffc0;
  font-size: 11px;
}
.swatch.on {
  background: #7aa2ff55;
  color: #ffffff;
}
.brush-name {
  width: 200px;
  padding-top: 5px;
  padding-bottom: 5px;
  padding-left: 8px;
  padding-right: 8px;
  border-radius: 6px;
  background: #00000040;
  color: #ffffffe6;
  font-size: 12px;
}

/* **The per-window `@media` proof** (docs/MULTIWINDOW.md §6). The palette
   window is 380 px wide and the main window is 620, so this rule is ON in the
   palette and OFF in the main window — for the same element, at the same
   moment, in one stylesheet. The spike could not do that: the cascade read one
   global `Env`, so a 380 px pop-out answered against the main window's 620. */
@media (max-width: 420px) {
  .panel {
    background: #ff9d5c26;
  }
  .ptitle {
    color: #ff9d5c;
  }
}

/* **The per-window `@keyframes` proof** (§6). ONE `animation-name`, declared
   twice: the default below, then an override inside the same
   `@media (max-width: 420px)` the panel's background uses. Document order is
   what CSS decides duplicate names by, so the override has to come LAST — and
   which of the two is declared at all is a question about the WINDOW.

   `forwards` and one iteration, so each element parks on its own answer and the
   assertion is a settled computed style rather than a race against a clock. */
.pulse {
  width: 40px;
  height: 6px;
  border-radius: 3px;
  background: #7aa2ff;
  animation: pulse 80ms linear 1 forwards;
}
@keyframes pulse {
  to {
    opacity: 0.2;
  }
}
@media (max-width: 420px) {
  @keyframes pulse {
    to {
      opacity: 0.8;
    }
  }
}

/* ---- the player -------------------------------------------------------- */
.player {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 12px;
}
.clip {
  width: 320px;
  height: 180px;
  border-radius: 8px;
  background: #00000060;
}

/* ---- the notes window's own chrome ------------------------------------- */
.notes {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
}
.bar {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  height: 34px;
  padding-left: 12px;
  padding-right: 4px;
  background: #ffffff14;
  /* This is what makes the window movable: the OS answers `HTCAPTION` for the
     strip, so a drag on it moves the window and a double-click maximises it. */
  window-drag: move;
}
.bar-title {
  color: #ffffffc0;
  font-size: 12px;
}
.bar-close {
  padding-top: 5px;
  padding-bottom: 5px;
  padding-left: 10px;
  padding-right: 10px;
  border-radius: 4px;
  color: #ffffffc0;
  font-size: 11px;
  /* `HTCLOSE`: the OS paints its own hover highlight and performs the close. */
  --window-control: close;
}
.note-input {
  width: 100%;
  height: 110px;
  padding: 10px;
  background: #00000040;
  color: #ffffffe6;
  font-size: 12px;
}
.note-mirror {
  color: #ffffffc0;
}
</style>
