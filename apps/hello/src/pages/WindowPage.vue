<script setup>
// The windowing surface, end to end — docs/WINDOWING.md.
//
// Everything on this page is either a BROWSER-STANDARD API (`document.title`,
// `innerWidth`, `screen`, `devicePixelRatio`, `matchMedia`, the Fullscreen API)
// or the namespaced tier-3 extension (`appWindow`), and the page labels which
// is which — the three-tier split is the whole point of the design.
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "vue-native";

defineOptions({ name: "WindowPage" });

// ---- live state -----------------------------------------------------------
// Every number below is read straight from the window, not from a mirror we
// keep in sync ourselves: the host publishes one snapshot per frame and every
// getter reads it.
const geom = ref({});
const maximized = ref(false);
const focused = ref(false);
const bar = ref({ x: 0, y: 0, width: 0, height: 0 });
const chrome = ref(appWindow.chrome);

const readAll = () => {
  geom.value = {
    inner: `${Math.round(innerWidth)} x ${Math.round(innerHeight)}`,
    outer: `${Math.round(outerWidth)} x ${Math.round(outerHeight)}`,
    screenXY: `${Math.round(screenX)}, ${Math.round(screenY)}`,
    dpr: devicePixelRatio,
    screen: `${screen.width} x ${screen.height}`,
    avail: `${screen.availWidth} x ${screen.availHeight} @ ${screen.availLeft},${screen.availTop}`,
    extended: screen.isExtended,
  };
  maximized.value = appWindow.isMaximized();
  focused.value = appWindow.isFocused();
  bar.value = navigator.windowControlsOverlay.getTitlebarAreaRect();
  chrome.value = appWindow.chrome;
  readMq();
};

// `resize` is the STANDARD event on `window` — `appWindow` deliberately does
// not carry one, because duplicating it would teach a non-portable idiom for a
// portable problem.
const onResize = () => readAll();
const onState = () => readAll();
const onFocus = () => readAll();

onMounted(() => {
  readAll();
  addEventListener("resize", onResize);
  addEventListener("focus", onFocus);
  addEventListener("blur", onFocus);
  appWindow.addEventListener("maximize", onState);
  appWindow.addEventListener("unmaximize", onState);
  appWindow.addEventListener("move", onState);
  navigator.windowControlsOverlay.addEventListener("geometrychange", onState);
});
onUnmounted(() => {
  removeEventListener("resize", onResize);
  removeEventListener("focus", onFocus);
  removeEventListener("blur", onFocus);
  appWindow.removeEventListener("maximize", onState);
  appWindow.removeEventListener("unmaximize", onState);
  appWindow.removeEventListener("move", onState);
  navigator.windowControlsOverlay.removeEventListener("geometrychange", onState);
});

// ---- title ----------------------------------------------------------------
// `document.title` is the OS window title (taskbar, Alt-Tab, window
// enumeration). The PAINTED titlebar text comes from the app's own markup —
// they are independent in a browser too, and Titlebar.vue happens to mirror it.
const titleDraft = ref(document.title);
const applyTitle = () => {
  document.title = titleDraft.value;
  // `document.title` has no change event in any spec, so the titlebar is poked
  // explicitly rather than polled.
  dispatchEvent(new Event("vn:title"));
  // Read back: the getter applies HTML's whitespace normalisation, exactly as
  // a browser does, so "  a\n b  " comes back as "a b".
  titleDraft.value = document.title;
};

// ---- backdrop -------------------------------------------------------------
// One unified value list (Tauri's approach) with Electron's lowercase
// spellings. `blur` resolves to `acrylic` here and says so; `mica` on Windows
// 10 would fall back to `acrylic` and say so.
const MATERIALS = ["mica", "mica-alt", "acrylic", "transparent", "none"];
const backdrop = ref(appWindow.backdrop);
const setBackdrop = (m) => {
  appWindow.backdrop = m;
  backdrop.value = appWindow.backdrop;
};

// ---- media queries --------------------------------------------------------
// `matchMedia` runs the SAME vn-css evaluator the cascade does, against the
// same env — before this, `@media (max-width: 600px)` worked while
// `matchMedia("(max-width: 600px)")` answered false forever.
const MQ = {
  wide: matchMedia("(min-width: 800px)"),
  dpr: matchMedia(`(min-resolution: ${devicePixelRatio}dppx)`),
  wco: matchMedia("(display-mode: window-controls-overlay)"),
  fullscreen: matchMedia("(display-mode: fullscreen)"),
};
// `MediaQueryList.matches` is a plain getter, not a Vue ref, so the values are
// copied into reactive state — both by the standard `change` event and by
// `readAll`, which every window event already calls.
const mq = ref({});
const readMq = () => (mq.value = Object.fromEntries(
  Object.entries(MQ).map(([k, v]) => [k, v.matches]),
));
onMounted(() => Object.values(MQ).forEach((m) => m.addEventListener("change", readMq)));
onUnmounted(() => Object.values(MQ).forEach((m) => m.removeEventListener("change", readMq)));

// ---- verbs ----------------------------------------------------------------
// All synchronous, all returning undefined: JS, gpui and the Win32 UI thread
// are the same thread, so a Promise would lie about ordering. `await` still
// works, so ported Tauri code runs unmodified.
const nudge = (dx, dy) => appWindow.setBounds({ x: appWindow.getBounds().x + dx, y: appWindow.getBounds().y + dy });
const grow = (d) => resizeBy(d, d);
// The STANDARD Fullscreen API, not an `appWindow` verb: fullscreen state is
// observed through `document.fullscreenchange`, which is why §3.4 deliberately
// carries no `appWindow` event for it. Esc always exits.
const toggleFullscreen = () =>
  document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen();
</script>

<template>
  <div class="page">
    <div class="hd">
      <span class="h1">Windowing</span>
      <span class="sub">docs/WINDOWING.md — three tiers, one window</span>
    </div>

    <!-- chrome mode ------------------------------------------------------ -->
    <section class="card">
      <div class="cap">chrome mode</div>
      <div class="grid">
        <div class="kv"><span class="k">titleBarStyle</span><span class="v">{{ chrome.titleBarStyle }}</span></div>
        <div class="kv"><span class="k">display-mode</span><span class="v">{{ chrome.displayMode }}</span></div>
        <div class="kv"><span class="k">host fallback titlebar</span><span class="v">{{ chrome.fallbackTitleBar ? "on" : "retired" }}</span></div>
        <div class="kv"><span class="k">WCO visible</span><span class="v">{{ chrome.overlayVisible }}</span></div>
        <div class="kv"><span class="k">titlebar area rect</span><span class="v">{{ bar.x }}, {{ bar.y }} — {{ bar.width }} x {{ bar.height }}</span></div>
      </div>
      <div class="note">
        The strip at the top of this window is the app's own &lt;header&gt;,
        marked `window-drag: move`; its three buttons carry `--window-control`,
        which is what makes Windows 11 offer snap layouts when you hover the
        maximize one. The host's built-in titlebar retired on the first frame
        this app painted a drag region.
      </div>
    </section>

    <!-- title ------------------------------------------------------------ -->
    <section class="card">
      <div class="cap">document.title <span class="tier t1">tier 1</span></div>
      <div class="row">
        <input class="in" v-model="titleDraft" @keyup.enter="applyTitle" />
        <div class="btn" @click="applyTitle">Set title</div>
      </div>
      <div class="note">
        Drives the OS window title: taskbar, Alt-Tab, window enumeration. The
        getter normalises whitespace exactly as HTML says, so leading, trailing
        and repeated spaces come back collapsed.
      </div>
    </section>

    <!-- backdrop --------------------------------------------------------- -->
    <section class="card">
      <div class="cap">backdrop material <span class="tier t3">tier 3</span></div>
      <div class="row">
        <div
          v-for="m in MATERIALS"
          :key="m"
          class="chip"
          :class="{ on: backdrop === m }"
          @click="setBackdrop(m)"
        >
          {{ m }}
        </div>
      </div>
      <div class="swatch">
        <div class="pane">
          <span>the desktop shows through here — switch the material and watch</span>
        </div>
      </div>
    </section>

    <!-- state ------------------------------------------------------------ -->
    <section class="card">
      <div class="cap">window state <span class="tier t3">tier 3</span></div>
      <div class="row">
        <div class="btn" @click="appWindow.minimize()">minimize</div>
        <div class="btn" @click="appWindow.toggleMaximize()">toggleMaximize</div>
        <div class="btn" @click="appWindow.unmaximize()">unmaximize</div>
        <div class="btn" @click="appWindow.center()">center</div>
        <div class="btn" @click="toggleFullscreen()">fullscreen</div>
      </div>
      <div class="row">
        <div class="btn" @click="nudge(-24, 0)">moveBy −24</div>
        <div class="btn" @click="nudge(24, 0)">moveBy +24</div>
        <div class="btn" @click="grow(-40)">resizeBy −40</div>
        <div class="btn" @click="grow(40)">resizeBy +40</div>
      </div>
      <div class="row">
        <span class="pill" :class="{ live: maximized }">isMaximized: {{ maximized }}</span>
        <span class="pill" :class="{ live: focused }">isFocused: {{ focused }}</span>
      </div>
    </section>

    <!-- geometry --------------------------------------------------------- -->
    <section class="card">
      <div class="cap">geometry &amp; screen <span class="tier t1">tier 1</span></div>
      <div class="grid">
        <div class="kv"><span class="k">innerWidth x innerHeight</span><span class="v">{{ geom.inner }}</span></div>
        <div class="kv"><span class="k">outerWidth x outerHeight</span><span class="v">{{ geom.outer }}</span></div>
        <div class="kv"><span class="k">screenX, screenY</span><span class="v">{{ geom.screenXY }}</span></div>
        <div class="kv"><span class="k">devicePixelRatio</span><span class="v">{{ geom.dpr }}</span></div>
        <div class="kv"><span class="k">screen</span><span class="v">{{ geom.screen }}</span></div>
        <div class="kv"><span class="k">screen.avail*</span><span class="v">{{ geom.avail }}</span></div>
        <div class="kv"><span class="k">screen.isExtended</span><span class="v">{{ geom.extended }}</span></div>
      </div>
      <div class="note">
        outer − inner is the invisible resize border, not the titlebar height —
        read the titlebar height from the WCO rect above.
      </div>
    </section>

    <!-- matchMedia ------------------------------------------------------- -->
    <section class="card">
      <div class="cap">matchMedia <span class="tier t1">tier 1</span></div>
      <div class="grid">
        <div class="kv"><span class="k">(min-width: 800px)</span><span class="v">{{ mq.wide }}</span></div>
        <div class="kv"><span class="k">(min-resolution: {{ geom.dpr }}dppx)</span><span class="v">{{ mq.dpr }}</span></div>
        <div class="kv"><span class="k">(display-mode: window-controls-overlay)</span><span class="v">{{ mq.wco }}</span></div>
        <div class="kv"><span class="k">(display-mode: fullscreen)</span><span class="v">{{ mq.fullscreen }}</span></div>
      </div>
      <div class="responsive">
        this box is styled by a real @media (min-width: 800px) rule — it and the
        JS answer above come from the same evaluator, so they cannot disagree
      </div>
    </section>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.hd {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
}
.h1 {
  font-size: 16px;
  color: #ffffff;
}
.sub {
  font-size: 11px;
  color: #ffffff70;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-radius: 10px;
  background: #ffffff0d;
}
.cap {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: #ffffff80;
}
.tier {
  padding: 2px;
  border-radius: 4px;
  font-size: 10px;
}
.t1 {
  background: #4f8cff40;
  color: #b9d0ff;
}
.t3 {
  background: #ffb04f40;
  color: #ffd9a8;
}
.row {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}
.grid {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.kv {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
}
.k {
  width: 260px;
  font-size: 11px;
  color: #ffffff70;
}
.v {
  flex: 1;
  font-size: 12px;
  color: #e8eefc;
}
.note {
  font-size: 11px;
  color: #ffffff70;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  font-size: 12px;
  color: #e8eefc;
  background: #ffffff14;
  cursor: pointer;
  transition: background-color 140ms ease;
}
.btn:hover {
  background: #ffffff2b;
}
.chip {
  padding: 5px;
  border-radius: 6px;
  font-size: 12px;
  color: #ffffffb0;
  background: #ffffff10;
  cursor: pointer;
  transition: background-color 140ms ease, color 140ms ease;
}
.chip:hover {
  background: #ffffff24;
}
.chip.on {
  background: #4f8cff66;
  color: #ffffff;
}
.pill {
  padding: 4px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffff90;
  background: #ffffff10;
}
.pill.live {
  background: #4ade8040;
  color: #d7ffe4;
}
.in {
  width: 320px;
  padding: 6px;
  border-radius: 6px;
  font-size: 12px;
  color: #ffffff;
  background: #00000040;
}
.swatch {
  padding: 10px;
  border-radius: 8px;
  background: #00000030;
}
.pane {
  padding: 14px;
  border-radius: 8px;
  font-size: 11px;
  color: #ffffff90;
  background: #ffffff0a;
}
.responsive {
  padding: 8px;
  border-radius: 8px;
  font-size: 11px;
  color: #ffb4b4;
  background: #ff4f4f26;
}
@media (min-width: 800px) {
  .responsive {
    color: #b4ffc8;
    background: #4ade8026;
  }
}
</style>
