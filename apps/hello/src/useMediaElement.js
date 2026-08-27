// `useMediaElement(ref)` — HTMLMediaElement, in userland, temporarily.
//
// ## Why this file exists, and when it gets deleted
//
// `<video>`'s state does not live in the mirror tree: `currentTime` moves
// inside the OS media pipeline sixty times a second and no prop write puts it
// there. The host therefore publishes it as four global functions —
// `__vn_media_state`, `__vn_media_call`, `__vn_media_can_play`,
// `__vn_media_events` — exactly the way `__vn_bounds` and
// `__vn_computed_style` publish layout and the cascade.
//
// Turning those into `el.currentTime` and `el.play()` is the JS runtime's job
// (`packages/vue-native/src/runtime.js`), and it is a four-place additive
// change spelled out in `crates/vn-video/CONTRACT.md` §6. That file belongs to
// another track this round and was deliberately not touched, so the accessors
// and the media event names do not exist yet — which means `@timeupdate` in a
// template warns and never fires.
//
// So this composable does the same job in the app, over the same host
// functions. **When the runtime change lands, delete this file and replace
// `media.currentTime` with `videoEl.currentTime` and `on('ended', …)` with
// `@ended`.** Nothing in Rust moves.
//
// ## Why it polls the EVENT ring rather than the state
//
// Reading `currentTime` every frame and writing it into a `ref` would rerender
// the page 165 times a second for a readout that a browser updates four times a
// second. So the loop reads `__vn_media_events(id)` — one host call returning
// an empty string on almost every frame — and only refreshes the reactive
// state when the host actually raised something. `timeupdate` arrives on the
// spec's 250 ms cadence, so the UI ticks at 4 Hz, like a browser's.

import { onBeforeUnmount, onMounted, reactive, watch } from "vue";

/** `__vn_media_call` verbs — mirror of `vn_tree::video::media_call`. */
const CALL = {
  play: 0,
  pause: 1,
  currentTime: 2,
  volume: 3,
  muted: 4,
  playbackRate: 5,
  loop: 6,
};

const NUMBERS = new Set([
  "currentTime",
  "duration",
  "volume",
  "playbackRate",
  "readyState",
  "networkState",
  "videoWidth",
  "videoHeight",
  "error",
]);
const BOOLEANS = new Set(["paused", "ended", "seeking", "muted", "loop"]);

function parseRanges(raw) {
  if (!raw) return [];
  return raw.split(",").map((pair) => {
    const [start, end] = pair.split(":").map(Number);
    return { start, end };
  });
}

/**
 * @param {import("vue").Ref} elRef  template ref on the <video> element
 * @returns reactive media state + the transport methods
 */
export function useMediaElement(elRef) {
  const state = reactive({
    // Everything HTMLMediaElement reports, with the spec's initial values so a
    // template can bind before the first frame without guarding.
    currentTime: 0,
    duration: NaN,
    paused: true,
    ended: false,
    seeking: false,
    readyState: 0,
    networkState: 0,
    volume: 1,
    muted: false,
    playbackRate: 1,
    loop: false,
    videoWidth: 0,
    videoHeight: 0,
    buffered: [],
    seekable: [],
    error: null,
    errorMessage: "",
    /** Every event the host has raised, newest last — the demo's proof. */
    log: [],
  });

  const listeners = new Map();
  let raf = 0;

  const nodeId = () => elRef.value?.id ?? 0;

  function refresh() {
    const id = nodeId();
    if (!id) return;
    const raw = __vn_media_state(id);
    if (!raw) return;
    state.error = null;
    state.errorMessage = "";
    for (const line of raw.split("\n")) {
      if (!line) continue;
      const eq = line.indexOf("=");
      if (eq < 0) continue;
      const key = line.slice(0, eq);
      const value = line.slice(eq + 1);
      if (key === "buffered" || key === "seekable") state[key] = parseRanges(value);
      else if (key === "errorMessage") state.errorMessage = value;
      else if (NUMBERS.has(key)) state[key] = Number(value);
      else if (BOOLEANS.has(key)) state[key] = value === "true";
    }
  }

  let sinceRefresh = 0;
  function pump() {
    const id = nodeId();
    if (id) {
      // One host call per frame, empty on almost all of them.
      const raw = __vn_media_events(id);
      if (raw) {
        refresh();
        sinceRefresh = 0;
        for (const name of raw.split(",")) {
          state.log.push(name);
          if (state.log.length > 40) state.log.shift();
          const fns = listeners.get(name);
          if (fns) for (const fn of fns) fn(state);
        }
      } else if (++sinceRefresh >= 15) {
        // …plus a slow poll. Events alone are not enough: a `<video>` whose
        // load fails raises `error` ONCE, and if that one event is drained by
        // anything else (devtools, a second reader) the state would never be
        // read again and the element would sit at "loading…" forever. A
        // browser's `el.error` is a live property, so this one is too.
        // `reactive` skips writes that do not change a value, so a poll on an
        // idle element costs one host call and no rerender.
        sinceRefresh = 0;
        refresh();
      }
    }
    raf = requestAnimationFrame(pump);
  }

  const call = (verb, arg = 0) => {
    const id = nodeId();
    return id ? __vn_media_call(id, verb, arg) : false;
  };

  onMounted(() => {
    refresh();
    raf = requestAnimationFrame(pump);
  });
  onBeforeUnmount(() => cancelAnimationFrame(raf));
  // A `v-if`'d or swapped element arrives after mount.
  watch(elRef, () => refresh());

  return {
    state,
    /** `el.addEventListener(name, fn)`, until the runtime has the codes. */
    on(name, fn) {
      const fns = listeners.get(name) ?? [];
      fns.push(fn);
      listeners.set(name, fns);
    },
    play: () => call(CALL.play),
    pause: () => call(CALL.pause),
    seek: (seconds) => {
      call(CALL.currentTime, seconds);
      // The seek lands on the host immediately; mirror it so a dragged scrub
      // bar does not snap back for the ~250 ms until `seeked` arrives.
      state.currentTime = seconds;
    },
    setMuted: (muted) => call(CALL.muted, muted ? 1 : 0),
    setVolume: (volume) => call(CALL.volume, volume),
    setRate: (rate) => call(CALL.playbackRate, rate),
    setLoop: (loop) => call(CALL.loop, loop ? 1 : 0),
    /** `canPlayType(type)` → "" | "maybe" | "probably", probed on THIS machine. */
    canPlayType: (type) => {
      const id = nodeId();
      return id ? __vn_media_can_play(id, type) : "";
    },
    refresh,
  };
}

/** `mm:ss.d`, the shape a transport readout wants. */
export function formatTime(seconds) {
  if (!Number.isFinite(seconds)) return "--:--";
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${String(m).padStart(2, "0")}:${s.toFixed(1).padStart(4, "0")}`;
}
