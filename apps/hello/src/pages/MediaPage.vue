<script setup lang="ts">
import { TINY_CLIP_DATA_URI } from "../tinyClip";

// <video> — the OS media pipeline inside a CSS box.
//
// Two elements: a working 1080p H.264 clip with a full transport built out of
// ordinary divs, and a deliberately broken one that proves the error path says
// something useful instead of showing a black rectangle.
//
// Things to notice while looking at this file:
//
// * `<video>` is a NORMAL element. The `border-radius` and `overflow: hidden`
//   on `.frame` below really do clip it, and the `:hover` transition really
//   does apply — that is the payoff for decoding into our own scene graph
//   (Media Foundation frame-server mode) instead of onto a sibling
//   DirectComposition visual, where none of it would work.
// * The transport is `videoEl.play()` / `videoEl.currentTime = 8` /
//   `@timeupdate`, i.e. plain HTMLMediaElement — nothing app-specific and
//   nothing host-specific. That is new: until m12's integration the JS runtime
//   did not know the media event codes, so this page carried a
//   `useMediaElement` composable that talked to `__vn_media_state` /
//   `__vn_media_call` itself. `crates/vn-video/CONTRACT.md` §6 said that file
//   dies the day runtime.js learns the codes. It has, and it did.
// * The properties are LIVE reads off the host (`el.currentTime` is one host
//   call, like `getBoundingClientRect()`), so nothing about them is reactive.
//   The events are what drive the UI — exactly as in a browser, where you also
//   copy `e.target.currentTime` into state on `@timeupdate` rather than
//   watching the element.

import { computed, onMounted, reactive, ref } from "vue";

// Template refs on native elements. `VnMediaElement` (apps/env.d.ts) is the
// HTMLMediaElement surface runtime.ts really installs — its own `MirrorElement`
// interface has all of it and is not exported, so an app has no name for what a
// `ref="videoEl"` is holding.
const videoEl = ref<VnMediaElement | null>(null);
const brokenEl = ref<VnMediaElement | null>(null);

// What the last event left on the element. `timeupdate` arrives on the spec's
// 4 Hz cadence, so this rerenders four times a second, not 165.
const media = reactive({
  currentTime: 0,
  duration: NaN,
  paused: true,
  ended: false,
  seeking: false,
  readyState: 0,
  videoWidth: 0,
  videoHeight: 0,
});
const log = ref<string[]>([]);
const brokenError = ref<VnMediaError | null>(null);

function readBack(el: VnMediaElement | null | undefined) {
  if (!el) return;
  media.currentTime = el.currentTime;
  media.duration = el.duration;
  media.paused = el.paused;
  media.ended = el.ended;
  media.seeking = el.seeking;
  media.readyState = el.readyState;
  media.videoWidth = el.videoWidth;
  media.videoHeight = el.videoHeight;
}

/** One handler for every media event: log it, then re-read the element.
 *
 *  `VnEvent.target` is `unknown` — the protocol delivers a node id and the
 *  runtime hands back the mirror node it names, which the type system has no
 *  way to connect to `<video>`. This handler is only ever bound to the one
 *  element above, so that is what the cast says. */
function onMedia(e: VnEvent) {
  log.value.push(e.type);
  if (log.value.length > 40) log.value.shift();
  readBack((e.target as VnMediaElement | null) ?? videoEl.value);
}

function onBrokenError(e: VnEvent) {
  // `e.target.error` is a real MediaError: `{ code, message }`, where the
  // message is the advisory naming what THIS machine cannot decode.
  brokenError.value = (e.target as VnMediaElement | null)?.error ?? null;
}

const muted = ref(true); // browsers require it for autoplay; be a good neighbour
const looping = ref(false);
const rate = ref(1);

// `paused` and `ended` are separate in HTML: reaching the end sets `ended` and
// leaves `paused` alone (the spec only changes `paused` in play()/pause()), so
// a transport button has to read BOTH or it says "pause" over a stopped video.
const stopped = computed(() => media.paused || media.ended);

function toggle() {
  const el = videoEl.value;
  if (!el) return;
  if (stopped.value) el.play();
  else el.pause();
  readBack(el);
}

function seek(seconds: number) {
  const el = videoEl.value;
  if (!el) return;
  el.currentTime = Math.max(0, seconds);
  // The seek lands on the host immediately; mirror it so a dragged scrub bar
  // does not snap back for the ~250 ms until `seeked` arrives.
  media.currentTime = Math.max(0, seconds);
}

function setRate(r: number) {
  rate.value = r;
  if (videoEl.value) videoEl.value.playbackRate = r;
}

function toggleMute() {
  muted.value = !muted.value;
  if (videoEl.value) videoEl.value.muted = muted.value;
}

function toggleLoop() {
  looping.value = !looping.value;
  if (videoEl.value) videoEl.value.loop = looping.value;
}

/** `mm:ss.d`, the shape a transport readout wants. */
function formatTime(seconds: number) {
  if (!Number.isFinite(seconds)) return "--:--";
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${String(m).padStart(2, "0")}:${s.toFixed(1).padStart(4, "0")}`;
}

// --- the scrub bar ----------------------------------------------------------
// No <input type="range"> in this renderer, so the track is a div and the seek
// is arithmetic on getBoundingClientRect — which works because the DOM surface
// answers it off the real painted rectangle.
const scrubbing = ref(false);

function seekFromEvent(e: VnMouseEvent) {
  // `currentTarget` is rewritten at each step of the propagation walk, so it
  // is the `.track` div this handler is bound to — not whatever was clicked.
  const track = e.currentTarget as VnElement | null;
  if (!track || !Number.isFinite(media.duration)) return;
  const rect = track.getBoundingClientRect();
  if (!rect.width) return;
  const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
  seek(fraction * media.duration);
}

const progress = computed(() => {
  const { currentTime, duration } = media;
  if (!Number.isFinite(duration) || duration <= 0) return 0;
  return Math.min(100, Math.max(0, (currentTime / duration) * 100));
});

// --- the codec probe --------------------------------------------------------
// A LIVE probe of this machine, not a table: H.264 + AAC in MP4 is the only
// combination guaranteed on a clean Windows install; HEVC, AV1, VP9 and the
// WebM family all arrive as separately installable Store packages that an
// enterprise image may have stripped. macOS will answer differently again.
const CODECS: [label: string, mime: string][] = [
  ['H.264 + AAC (MP4)', 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'],
  ['HEVC / H.265 (MP4)', 'video/mp4; codecs="hev1.1.6.L93.B0"'],
  ['AV1 (MP4)', 'video/mp4; codecs="av01.0.05M.08"'],
  ['VP9 (WebM)', 'video/webm; codecs="vp9"'],
  ['VP8 + Vorbis (WebM)', 'video/webm; codecs="vp8, vorbis"'],
  ['Ogg Theora', 'video/ogg; codecs="theora"'],
  ['FFV1 in MKV (a codec the probe does not know)', 'video/x-matroska; codecs="ffv1"'],
];
/** `canPlayType` answers `"" | "maybe" | "probably"`; `""` is rendered as
 *  "no", which is also the class name the stylesheet colours it by. */
const probe = ref<{ label: string; answer: string }[]>([]);
onMounted(() => {
  // `VN_SCENARIO=media` — start looping playback with no input at all, so a
  // soak or a frame-time measurement is one process and one screen.
  if (globalThis.__VN_SCENARIO__ === "media") {
    setTimeout(() => {
      const el = videoEl.value;
      if (!el) return;
      el.loop = true;
      el.play();
      looping.value = true;
      readBack(el);
    }, 500);
  }
  // After the element exists — `canPlayType` is asked OF the media engine.
  setTimeout(() => {
    const el = videoEl.value;
    probe.value = CODECS.map(([label, type]) => ({
      label,
      // `""` (the spec's "no") and a missing element both render as "no".
      answer: (el && el.canPlayType(type)) || "no",
    }));
    readBack(el);
  }, 250);
});

// The event log, newest last — the proof that `ended` and `error` really fire.
const recent = computed(() => log.value.slice(-8).join(" · "));

// --- the other two src grammars (m13 item 3) --------------------------------
// `<video src>` and `<img src>` take the same three shapes a browser does, and
// each reaches the platform as the thing it actually wants: a PATH (the OS owns
// the read-ahead), a URL VERBATIM (its network source streams and range-seeks),
// or BYTES (a data: URI has neither a path nor a server).
//
// The origin is overridable so this page can be pointed at a real server; the
// default is a local one, so the demo has no third-party dependency and no
// internet requirement.
const httpOrigin = globalThis.__VN_HTTP_ORIGIN__ || "http://127.0.0.1:8731";
const tinyClip = TINY_CLIP_DATA_URI;
const httpClip = `${httpOrigin}/clip-1080p.mp4`;
const httpImage = `${httpOrigin}/clip-1080p.poster.png`;
</script>

<template>
  <div class="page">
    <!-- ===== the working clip ============================================ -->
    <div class="card">
      <div class="title">&lt;video&gt; — Media Foundation, decoded on gpui's own device</div>

      <div class="stage">
        <!-- The frame is a plain CSS box. The radius and the clip below are
             the point: a video that escaped the scene graph could not have
             either. -->
        <div class="frame">
          <!-- The event set is HTML's, bound with plain `@` handlers. The
               listener bit is what makes the host build and send the payload
               at all, so an unbound `@timeupdate` costs nothing. -->
          <video
            ref="videoEl"
            class="video"
            src="assets/clip-1080p.mp4"
            poster="assets/clip-1080p.poster.png"
            muted
            @loadedmetadata="onMedia"
            @loadeddata="onMedia"
            @canplay="onMedia"
            @play="onMedia"
            @playing="onMedia"
            @pause="onMedia"
            @timeupdate="onMedia"
            @durationchange="onMedia"
            @ratechange="onMedia"
            @volumechange="onMedia"
            @seeking="onMedia"
            @seeked="onMedia"
            @waiting="onMedia"
            @ended="onMedia"
            @error="onMedia"
          />
        </div>

        <div class="side">
          <div class="meta">
            {{ media.videoWidth }}×{{ media.videoHeight }} · H.264 High + AAC ·
            readyState {{ media.readyState }}
          </div>

          <div class="row">
            <div class="btn" @click="toggle">{{ stopped ? "play" : "pause" }}</div>
            <div class="btn ghost" @click="seek(media.currentTime - 5)">−5s</div>
            <div class="btn ghost" @click="seek(media.currentTime + 5)">+5s</div>
            <div class="btn ghost" :class="{ on: muted }" @click="toggleMute">
              {{ muted ? "muted" : "sound" }}
            </div>
            <div class="btn ghost" :class="{ on: looping }" @click="toggleLoop">loop</div>
          </div>

          <div class="row">
            <div class="btn ghost" :class="{ on: rate === 0.5 }" @click="setRate(0.5)">0.5×</div>
            <div class="btn ghost" :class="{ on: rate === 1 }" @click="setRate(1)">1×</div>
            <div class="btn ghost" :class="{ on: rate === 2 }" @click="setRate(2)">2×</div>
            <div class="btn ghost" @click="seek(2)">seek 2s</div>
            <div class="btn ghost" @click="seek(8)">seek 8s</div>
          </div>

          <!-- The scrub bar. `currentTime` updates at the spec's 4 Hz, so this
               moves exactly as often as a browser's does. -->
          <div
            class="track"
            @mousedown="scrubbing = true; seekFromEvent($event)"
            @mousemove="scrubbing && seekFromEvent($event)"
            @mouseup="scrubbing = false"
            @mouseleave="scrubbing = false"
          >
            <div class="fill" :style="{ width: progress + '%' }" />
          </div>

          <div class="readout">
            {{ formatTime(media.currentTime) }} / {{ formatTime(media.duration) }}
            · {{ stopped ? "paused" : "playing" }}
            <span v-if="media.ended"> · ended</span>
            <span v-if="media.seeking"> · seeking</span>
          </div>
          <div class="hint">events: {{ recent || "…" }}</div>
        </div>
      </div>
    </div>

    <!-- ===== the broken clip ============================================= -->
    <div class="card">
      <div class="title">The error path — a source this machine cannot decode</div>
      <div class="stage">
        <div class="frame small">
          <!-- One `@error` handler, and `e.target.error` is a real MediaError. -->
          <video
            ref="brokenEl"
            class="video"
            src="assets/unsupported-codec.mkv"
            @error="onBrokenError"
          />
        </div>
        <div class="side">
          <div class="meta bad" v-if="brokenError">MediaError code {{ brokenError.code }}</div>
          <div class="meta" v-else>loading…</div>
          <div class="advisory">
            {{ brokenError ? brokenError.message : "no error yet" }}
          </div>
          <div class="hint">
            FFV1 (a lossless archival codec) in a Matroska container: MF reads the container fine
            and has no decoder for the codec, which is the interesting half of the failure. A
            browser answers MEDIA_ERR_SRC_NOT_SUPPORTED and stops; here the same error also names
            which decoders THIS machine has and what to re-encode to. The identical text goes to
            the compat advisory channel, once, whether or not anyone bound @error.
          </div>
        </div>
      </div>
    </div>

    <!-- ===== the other two src grammars (m13 item 3) ===================== -->
    <div class="card">
      <div class="title">src grammars — data: and http(s)</div>
      <div class="src-row">
        <div class="src-cell">
          <video class="mini" :src="tinyClip" autoplay loop muted />
          <div class="src-label">
            data: URI — 6 KB of MP4 inlined in the bundle. Staged to a content-hashed file in
            the OS temp directory, because SetSourceFromByteStream OPENS the URL it is given
            rather than reading the stream (m13 report, item 3).
          </div>
        </div>
        <div class="src-cell">
          <video class="mini" :src="httpClip" autoplay loop muted />
          <div class="src-label">
            http(s) — Media Foundation's own network source streams and range-seeks it, so a
            long clip starts playing while it is still arriving.
          </div>
        </div>
        <div class="src-cell">
          <img class="mini" :src="httpImage" alt="http image" />
          <div class="src-label">
            &lt;img&gt; over http — gpui's asset loader fetches it through the ReqwestClient the
            host installs, asynchronously: alt text for a frame, then the picture.
          </div>
        </div>
      </div>
      <div class="hint">
        {{ httpOrigin }} is a local origin. An unreachable one renders the alt text (img) or
        raises MediaError code 2/4 with an advisory (video) — it never hangs and never lies.
      </div>
    </div>

    <!-- ===== the live codec probe ======================================== -->
    <div class="card">
      <div class="title">canPlayType() — probed on this machine, right now</div>
      <div class="probe">
        <div v-for="row in probe" :key="row.label" class="probe-row">
          <div class="probe-name">{{ row.label }}</div>
          <div class="probe-answer" :class="row.answer">{{ row.answer }}</div>
        </div>
      </div>
      <div class="hint">
        Only the first row is guaranteed: H.264 + AAC in MP4 is what Windows ships inbox. Every
        other line depends on Store codec extensions that may not be installed — which is why this
        is a runtime probe and never a compiled-in table.
      </div>
    </div>
  </div>
</template>

<style scoped>
.src-row {
  display: flex;
  flex-direction: row;
  gap: 14px;
}
.src-cell {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 240px;
}
.mini {
  width: 240px;
  height: 135px;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid #ffffff2e;
  background: #0006;
}
.src-label {
  font-size: 10px;
  color: #b9c4d6;
}

.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 14px;
  color: #ffffff;
}
.stage {
  display: flex;
  flex-direction: row;
  gap: 14px;
  align-items: flex-start;
}
/* The proof that the video is inside the scene graph: a rounded, clipped,
   bordered box that a DirectComposition sibling visual could not honour. */
.frame {
  width: 420px;
  height: 236px;
  border-radius: 14px;
  overflow: hidden;
  border: 1px solid #7fd4ff44;
  background: #0b1016;
  transition: border-color 200ms ease;
}
.frame:hover {
  border-color: #7fd4ffcc;
}
.frame.small {
  width: 220px;
  height: 124px;
}
.video {
  width: 100%;
  height: 100%;
}
.side {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  /* A flex item's default `min-width: auto` is its content's intrinsic width,
     so a long advisory pushes the row wider than the card instead of wrapping.
     Same fix as in a browser. */
  min-width: 0;
}
.meta {
  font-size: 12px;
  color: #7fd4ff;
}
.meta.bad {
  color: #ff9f9f;
}
.row {
  display: flex;
  flex-direction: row;
  gap: 6px;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.btn:hover {
  background: #4f8cffcc;
}
.btn.ghost {
  background: #ffffff0d;
  border: 1px solid #ffffff2e;
  color: #ffffffcc;
}
.btn.ghost:hover {
  background: #ffffff22;
}
.btn.ghost.on {
  background: #7fd4ff40;
  border: 1px solid #7fd4ff88;
  color: #ffffff;
}
.track {
  height: 10px;
  border-radius: 5px;
  background: #ffffff1a;
  border: 1px solid #ffffff26;
  cursor: pointer;
  overflow: hidden;
}
.fill {
  height: 100%;
  background: #7fd4ffcc;
}
.readout {
  font-size: 12px;
  color: #a5f3a5;
}
.advisory {
  font-size: 11px;
  color: #ffd9a0;
  padding: 8px;
  border-radius: 8px;
  background: #ffb35c14;
  border: 1px solid #ffb35c33;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
.probe {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.probe-row {
  display: flex;
  flex-direction: row;
  gap: 10px;
}
.probe-name {
  width: 320px;
  font-size: 12px;
  color: #ffffffcc;
}
.probe-answer {
  font-size: 12px;
  color: #ffffff88;
}
.probe-answer.probably {
  color: #a5f3a5;
}
.probe-answer.maybe {
  color: #ffd9a0;
}
.probe-answer.no {
  color: #ff9f9f;
}
</style>
