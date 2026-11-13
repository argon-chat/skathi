<script setup lang="ts">
import { computed, ref } from "vue";
import { VnWindow, type VnWindowMetrics, type VnWindowTarget } from "vue-native";

const count = ref(0);

// `v-if` IS the lifecycle: mounting `<VnWindow>` opens the window, unmounting closes it, and the
// window's own X button fires `@close` so the flag goes back to false.
const palette = ref(false);
const player = ref(false);
const notes = ref(false);

const brush = ref("charcoal");
const swatches = ["charcoal", "rust", "moss", "slate"];

const paletteWin = ref<{ el: VnWindowTarget | null } | null>(null);
const paletteRoot = computed(() => paletteWin.value?.el ?? null);

const paletteSize = ref<VnWindowMetrics>({ width: 0, height: 0, scale: 1 });
const paletteFocused = ref(false);
const paletteMetrics = computed(() =>
  paletteSize.value.width
    ? `${Math.round(paletteSize.value.width)}x${Math.round(paletteSize.value.height)}`
    : "none",
);

const note = ref("");

const paletteTitle = computed(() => `Palette - ${brush.value}`);
</script>

<template>
  <main class="page">
    <h1 class="h">One app, three windows</h1>

    <p class="lede">
      Everything below lives in this one component. A VnWindow moves a subtree's
      PAINT into a second OS window and leaves it in this tree.
    </p>

    <div class="card mirror">
      window A sees count = <span class="num">{{ count }}</span>
    </div>
    <div class="card meta">
      brush = <span class="num brush-mirror">{{ brush }}</span>
      · palette viewport = <span class="num pal-size">{{ paletteMetrics }}</span>
      · focused = <span class="num pal-focus">{{ paletteFocused }}</span>
    </div>

    <!-- Distinct classes rather than positions: `v-show` keeps a hidden button in the tree
         (`display: none`, as in a browser), so nth-of-type would count the invisible ones too. -->
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

    <div class="pulse pulse-main" />

    <p class="hint" v-show="palette">
      The panel is being painted by the palette window. It is still in this tree.
    </p>

    <div class="card note-mirror" v-show="notes">
      note = <span class="num note-text">{{ note || "(empty)" }}</span>
    </div>

    <!-- ORDER MATTERS, and it is the one subtlety of the whole idiom: this Teleport is written
         BEFORE the `<VnWindow>` it targets, and `disabled` is driven by `palette` as well as by the
         target. -->
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

        <input class="brush-name" v-model="brush" />

        <div class="foot">clicks and keystrokes here update window A too</div>
      </section>
    </Teleport>

    <!-- `alwaysOnTop` because that is what a palette is. -->
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
      <!-- A slot child rather than the Teleport'd panel: this node is CREATED with the window, so
           the run starts in the window that paints it. -->
      <div class="pulse pulse-palette" />
    </VnWindow>


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

        <div class="wtool">fixed</div>
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

@media (max-width: 420px) {
  .panel {
    background: #ff9d5c26;
  }
  .ptitle {
    color: #ff9d5c;
  }
}

/* Document order is what CSS decides duplicate names by, so the override has to come LAST — and
   which of the two is declared at all is a question about the WINDOW. */
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

.wtool {
  position: fixed;
  right: 8px;
  bottom: 8px;
  width: 52px;
  height: 18px;
  border-radius: 4px;
  background: #7aa2ff55;
  color: #ffffff;
  font-size: 10px;
}
</style>
