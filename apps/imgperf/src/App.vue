<script setup lang="ts">
// Every scenario is a tab; switching tabs calls __sk_perf_mark(), which flushes the host
// profiler's window and relabels the following seconds, so the log reads as one block per
// scenario.
import { ref, computed, onMounted, onUnmounted } from "vue";

// `__sk_perf_mark` exists only under SK_PERF=1, so the guard is real.
const mark = (name: string) => globalThis.__sk_perf_mark?.(name);
const log = (msg: string) => (globalThis as { __sk_log?: (m: string) => void }).__sk_log?.(msg);

const pad = (n: number) => String(n).padStart(2, "0");
const thumb = (i: number) => `assets/thumbs/t${pad(i % 50)}.png`;
const photo = (i: number) => `assets/photos/p${pad(i % 50)}.png`;

const grid400 = Array.from({ length: 400 }, (_, i) => thumb(i));
const grid50 = Array.from({ length: 50 }, (_, i) => photo(i));
const scrollRows = Array.from({ length: 120 }, (_, i) => ({
  id: i,
  a: thumb(i * 7),
  b: photo(i * 3),
  c: thumb(i * 13 + 5),
}));

const divs400 = Array.from({ length: 400 }, (_, i) => `hsl(${(i * 37) % 360} 60% 45%)`);

type Tab = "idle" | "grid400" | "divs400" | "grid50" | "scroll" | "pan" | "churn" | "fit" | "compat";
const TABS: Tab[] = ["idle", "grid400", "divs400", "grid50", "scroll", "pan", "churn", "fit", "compat"];

const tab = ref<Tab>("idle");
const setTab = (t: Tab) => {
  if (tab.value === "pan" && t !== "pan") {
    cancelAnimationFrame!(panRaf);
    panRaf = 0;
  }
  if (tab.value === "churn" && t !== "churn") stopChurn();
  tab.value = t;
  mark(t);
  if (t === "pan" && !panRaf) panRaf = requestAnimationFrame!(panTick);
};

const churnPool = [...Array(50).keys()].map(thumb).concat([...Array(50).keys()].map(photo));
const churnList = ref(churnPool.slice(0, 60));
let churnTimer: number | undefined;
let churnRounds = 0;
const churning = ref(false);
const startChurn = () => {
  if (churnTimer) return;
  churning.value = true;
  churnRounds = 0;
  mark("churn-running");
  churnTimer = setInterval(() => {
    churnRounds++;
    const off = churnRounds * 17;
    churnList.value = Array.from({ length: 60 }, (_, i) => churnPool[(i * 3 + off) % churnPool.length]!);
    if (churnRounds >= 20) stopChurn();
  }, 500);
};
const stopChurn = () => {
  if (!churnTimer) return;
  clearInterval(churnTimer);
  churnTimer = undefined;
  churning.value = false;
  log(`churn done: ${churnRounds} swaps`);
  mark("churn-done");
};
onUnmounted(() => {
  stopChurn();
  stopDirector();
  if (panRaf) cancelAnimationFrame!(panRaf);
});

// It is what a scroll costs MINUS gpui's scroll-offset machinery — new images enter every frame,
// so it isolates decode + atlas upload under continuous motion.
const panPool = Array.from({ length: 400 }, (_, i) => (i % 4 === 0 ? photo(i) : thumb(i)));
const panStart = ref(0);
const panList = computed(() => Array.from({ length: 60 }, (_, i) => panPool[(panStart.value + i) % panPool.length]));
let panRaf = 0;
const panTick = () => {
  panStart.value = (panStart.value + 1) % panPool.length;
  panRaf = requestAnimationFrame!(panTick);
};

const fits = ["fill", "contain", "cover", "scale-down", "none"];

const SEQUENCE: [Tab, number][] = [
  ["idle", 6000],
  ["divs400", 8000],
  ["grid400", 8000],
  ["grid50", 8000],
  ["pan", 10000],
  ["churn", 13000],
  ["fit", 14000],
  ["compat", 14000],
  ["idle", 4000],
];
let directorTimer: number | undefined;
const auto = ref(true);
const runSequence = (i: number) => {
  if (!auto.value) return;
  if (i >= SEQUENCE.length) {
    log("director: sequence done, looping");
    i = 0;
  }
  const [name, ms] = SEQUENCE[i]!;
  setTab(name);
  if (name === "churn") startChurn();
  directorTimer = setTimeout(() => runSequence(i + 1), ms);
};
const stopDirector = () => {
  auto.value = false;
  if (directorTimer) clearTimeout(directorTimer);
  directorTimer = undefined;
};
onMounted(() => {
  // `SK_SCENARIO` is an arbitrary string off the environment, so it is CHECKED against the tab
  // list rather than trusted.
  const pinned = globalThis.__SK_SCENARIO__;
  if (pinned) {
    auto.value = false;
    if (!TABS.includes(pinned as Tab)) {
      log(`pinned scenario "${pinned}" is not a scenario — staying idle. known: ${TABS.join(", ")}`);
      return;
    }
    const scenario = pinned as Tab;
    log(`pinned scenario: ${scenario}`);
    setTimeout(() => {
      setTab(scenario);
      if (scenario === "churn") startChurn();
    }, 1500);
    return;
  }
  log("director: starting scripted sequence");
  setTimeout(() => runSequence(0), 1500);
});

const DATA_PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAAXNSR0IArs4c6QAAAARnQU1BAACx" +
  "jwv8YQUAAAAJcEhZcwAADsMAAA7DAcdvqGQAAAH1SURBVDhPFczBFEUhAADRhxBCCCGEEEIIIYQQQgghtJxlCCFkMP/8C3C/78Pw" +
  "YfwwfZg/LB/WD9uH/cPx4fxwfbg/PB/eD9+H3xcwBIwBU8AcsASsAVvAHnAEnAFXwB3wBLwBX/gHEUPEGDFFzBFLxBqxRewRR8QZ" +
  "cUXcEU/EG/HFf5AwJIwJU8KcsCSsCVvCnnAknAlXwp3wJLwJX/oHGUPGmDFlzBlLxpqxZewZR8aZcWXcGU/Gm/Hlf1AwFIwFU8Fc" +
  "sBSsBVvBXnAUnAVXwV3wFLwFX/kHFUPFWDFVzBVLxVqxVewVR8VZcVXcFU/FW/HVf9AwNIwNU8PcsDSsDVvD3nA0nA1Xw93wNLwN" +
  "X/sHHUPH2DF1zB1Lx9qxdewdR8fZcXXcHU/H2/H1fzAwDIwD08A8sAysA9vAPnAMnAPXwD3wDLwD3/gHE8PEODFNzBPLxDqxTewT" +
  "x8Q5cU3cE8/EO/HNf7AwLIwL08K8sCysC9vCvnAsnAvXwr3wLLwL3/oHG8PGuDFtzBvLxrqxbewbx8a5cW3cG8/Gu/Htf3AwHIwH" +
  "08F8sBysB9vBfnAcnAfXwX3wHLwH3/kHF8PFeDFdzBfLxXqxXewXx8V5cV3cF8/Fe/Hdf/AwPIwP08P8sDysD9vD/nA8nA/Xw/3w" +
  "PLwP38Mfctm3H7PXMlMAAAAASUVORK5CYII=";
const DATA_SVG =
  "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20viewBox%3D%220%200%2040%2040%22%3E" +
  "%3Ccircle%20cx%3D%2220%22%20cy%3D%2220%22%20r%3D%2216%22%20fill%3D%22%2364d2a0%22%2F%3E%3C%2Fsvg%3E";

const counts = computed(() => ({
  idle: 0,
  grid400: grid400.length,
  divs400: 0,
  grid50: grid50.length,
  scroll: scrollRows.length * 3,
  pan: 60,
  churn: churnList.value.length,
  fit: fits.length,
  compat: 7,
}));
</script>

<template>
  <div class="app">
    <div class="tabs">
      <button
        v-for="t in TABS"
        :key="t"
        class="tab"
        :class="{ on: tab === t }"
        @click="
          stopDirector();
          setTab(t);
        "
      >
        {{ t }}
      </button>
      <span class="meta">{{ tab }} · {{ counts[tab] }} img nodes{{ auto ? " · auto" : "" }}</span>
    </div>

    <div v-if="tab === 'idle'" class="pane">
      <p class="lead">imgperf — pick a scenario. SK_PERF=1 prints frame stats once a second.</p>
      <img src="assets/logo.svg" alt="logo" class="one" />
      <p class="lead">the SVG above goes through gpui's resvg path, from the same img() element.</p>
    </div>

    <div v-else-if="tab === 'grid400'" class="pane scroller">
      <div class="wrap">
        <img v-for="(s, i) in grid400" :key="i" :src="s" alt="" class="thumb" />
      </div>
    </div>

    <div v-else-if="tab === 'divs400'" class="pane scroller">
      <div class="wrap">
        <div v-for="(c, i) in divs400" :key="i" class="thumb" :style="{ backgroundColor: c }"></div>
      </div>
    </div>

    <div v-else-if="tab === 'grid50'" class="pane scroller">
      <div class="wrap">
        <img v-for="(s, i) in grid50" :key="i" :src="s" alt="" class="photo" />
      </div>
    </div>

    <div v-else-if="tab === 'scroll'" class="pane scroller">
      <div v-for="row in scrollRows" :key="row.id" class="row">
        <img :src="row.a" alt="" class="thumb" />
        <img :src="row.b" alt="" class="mid" />
        <img :src="row.c" alt="" class="thumb" />
        <span class="rowlabel">row {{ row.id }}</span>
      </div>
    </div>

    <div v-else-if="tab === 'pan'" class="pane scroller">
      <div class="wrap">
        <img v-for="(s, i) in panList" :key="i" :src="s" alt="" class="thumb" />
      </div>
    </div>

    <div v-else-if="tab === 'churn'" class="pane scroller">
      <div class="controls">
        <button class="btn" @click="startChurn">shuffle every 500ms for 10s</button>
        <button class="btn" @click="stopChurn">stop</button>
        <span class="meta">{{ churning ? "running…" : "idle" }}</span>
      </div>
      <div class="wrap">
        <img v-for="s in churnList" :key="s" :src="s" alt="" class="thumb" />
      </div>
    </div>

    <div v-else-if="tab === 'fit'" class="pane scroller">
      <div class="fits">
        <div v-for="f in fits" :key="f" class="fitcell">
          <div class="fitlabel">object-fit: {{ f }}</div>
          <img src="assets/large/l0.png" alt="large" class="fitbox" :class="'fit-' + f" />
        </div>
      </div>
      <div class="fitcell">
        <div class="fitlabel">2048px JPEG, intrinsic size capped by max-width</div>
        <img src="assets/large/l0.jpg" alt="jpeg" class="capped" />
      </div>
    </div>

    <div v-else class="pane scroller">
      <div class="compat">
        <div class="cell"><b>data: png</b><img :src="DATA_PNG" alt="data png" class="one" /></div>
        <div class="cell"><b>data: svg (percent-encoded)</b><img :src="DATA_SVG" alt="data svg" class="one" /></div>
        <div class="cell"><b>relative .svg</b><img src="assets/logo.svg" alt="svg" class="one" /></div>
        <div class="cell">
          <b>width/height attributes</b><img src="assets/photos/p07.png" width="96" height="48" alt="attrs" />
        </div>
        <div class="cell"><b>no CSS size — intrinsic 64px</b><img src="assets/thumbs/t03.png" alt="intrinsic" /></div>
        <div class="cell">
          <b>missing file → alt text</b><img src="assets/nope.png" alt="[broken: nope.png]" class="one" />
        </div>
        <div class="cell">
          <b>http(s) → advisory + alt</b><img src="https://example.com/x.png" alt="[http not wired]" class="one" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
  height: 100%;
  color: #e8ecf5;
  font-size: 12px;
}
.tabs {
  display: flex;
  flex-direction: row;
  gap: 6px;
  align-items: center;
}
.tab {
  padding: 4px 10px;
  border-radius: 8px;
  background-color: #ffffff14;
  cursor: pointer;
}
.tab.on {
  background-color: #7fd4ff44;
}
.tab:hover {
  background-color: #ffffff28;
}
.meta {
  color: #ffffff88;
  padding-left: 8px;
}
.lead {
  color: #ffffffaa;
}
.pane {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex-grow: 1;
}
.scroller {
  overflow-y: scroll;
}
.wrap {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 4px;
}
.thumb {
  width: 64px;
  height: 64px;
  border-radius: 6px;
}
.mid {
  width: 120px;
  height: 80px;
  object-fit: cover;
  border-radius: 6px;
}
.photo {
  width: 128px;
  height: 128px;
  object-fit: cover;
  border-radius: 8px;
}
.one {
  width: 96px;
  height: 96px;
  object-fit: contain;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  align-items: center;
  padding: 4px;
}
.rowlabel {
  color: #ffffff77;
}
.controls {
  display: flex;
  flex-direction: row;
  gap: 8px;
  align-items: center;
}
.btn {
  padding: 4px 10px;
  border-radius: 8px;
  background-color: #7fd4ff33;
  cursor: pointer;
}
.btn:hover {
  background-color: #7fd4ff55;
}
.fits {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 12px;
}
.fitcell {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.fitlabel {
  color: #ffffff99;
}
.fitbox {
  width: 220px;
  height: 130px;
  background-color: #ffffff10;
  border-radius: 6px;
}
.fit-fill {
  object-fit: fill;
}
.fit-contain {
  object-fit: contain;
}
.fit-cover {
  object-fit: cover;
}
.fit-scale-down {
  object-fit: scale-down;
}
.fit-none {
  object-fit: none;
}
.capped {
  max-width: 320px;
  border-radius: 6px;
}
.compat {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 16px;
}
.cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 220px;
  padding: 8px;
  background-color: #ffffff0d;
  border-radius: 10px;
}
</style>
