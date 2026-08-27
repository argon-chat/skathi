<script setup>
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
// * `:autoplay` is EDGE-TRIGGERED, exactly as it is on `<lottie>`, so binding
//   it to a ref makes it the play/pause switch. A browser reads `autoplay`
//   once and expects `play()`/`pause()`; we have both — `media.play()` below
//   goes through the host's media surface — and the binding is the documented
//   superset (crates/vn-tree/src/video.rs, decision 1).
// * `useMediaElement` is TEMPORARY. It reads the host's media surface directly
//   because the JS runtime does not yet spell it `el.currentTime` /
//   `@timeupdate`; see the file's header and CONTRACT.md §6.

import { computed, onMounted, ref } from "vue";
import { formatTime, useMediaElement } from "../useMediaElement.js";

const videoEl = ref(null);
const brokenEl = ref(null);

const media = useMediaElement(videoEl);
const broken = useMediaElement(brokenEl);

// The transport switch. `paused` comes back from the host, so the button
// label follows what is ACTUALLY happening — including the pause the element
// performs itself when a non-looping clip ends.
const playing = ref(false);
const muted = ref(true); // browsers require it for autoplay; be a good neighbour
const looping = ref(false);
const rate = ref(1);

// `paused` and `ended` are separate in HTML: reaching the end sets `ended` and
// leaves `paused` alone (the spec only changes `paused` in play()/pause()), so
// a transport button has to read BOTH or it says "pause" over a stopped video.
const stopped = computed(() => media.state.paused || media.state.ended);

function toggle() {
  playing.value = stopped.value;
  if (playing.value) media.play();
  else media.pause();
}

function setRate(r) {
  rate.value = r;
  media.setRate(r);
}

function toggleMute() {
  muted.value = !muted.value;
  media.setMuted(muted.value);
}

function toggleLoop() {
  looping.value = !looping.value;
  media.setLoop(looping.value);
}

// --- the scrub bar ----------------------------------------------------------
// No <input type="range"> in this renderer, so the track is a div and the seek
// is arithmetic on getBoundingClientRect — which works because the DOM surface
// answers it off the real painted rectangle.
const scrubbing = ref(false);

function seekFromEvent(e) {
  const track = e.currentTarget;
  if (!track || !Number.isFinite(media.state.duration)) return;
  const rect = track.getBoundingClientRect();
  if (!rect.width) return;
  const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
  media.seek(fraction * media.state.duration);
}

const progress = computed(() => {
  const { currentTime, duration } = media.state;
  if (!Number.isFinite(duration) || duration <= 0) return 0;
  return Math.min(100, Math.max(0, (currentTime / duration) * 100));
});

// --- the codec probe --------------------------------------------------------
// A LIVE probe of this machine, not a table: H.264 + AAC in MP4 is the only
// combination guaranteed on a clean Windows install; HEVC, AV1, VP9 and the
// WebM family all arrive as separately installable Store packages that an
// enterprise image may have stripped. macOS will answer differently again.
const CODECS = [
  ['H.264 + AAC (MP4)', 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'],
  ['HEVC / H.265 (MP4)', 'video/mp4; codecs="hev1.1.6.L93.B0"'],
  ['AV1 (MP4)', 'video/mp4; codecs="av01.0.05M.08"'],
  ['VP9 (WebM)', 'video/webm; codecs="vp9"'],
  ['VP8 + Vorbis (WebM)', 'video/webm; codecs="vp8, vorbis"'],
  ['Ogg Theora', 'video/ogg; codecs="theora"'],
  ['FFV1 in MKV (a codec the probe does not know)', 'video/x-matroska; codecs="ffv1"'],
];
const probe = ref([]);
onMounted(() => {
  // `VN_SCENARIO=media` — start looping playback with no input at all, so a
  // soak or a frame-time measurement is one process and one screen.
  if (globalThis.__VN_SCENARIO__ === "media") {
    setTimeout(() => {
      media.setLoop(true);
      media.play();
      playing.value = true;
      looping.value = true;
    }, 500);
  }
  // After the element exists — `canPlayType` is asked OF the media engine.
  setTimeout(() => {
    probe.value = CODECS.map(([label, type]) => ({
      label,
      answer: media.canPlayType(type) || "no",
    }));
  }, 250);
});

// The event log, newest last — the proof that `ended` and `error` really fire.
const recent = computed(() => media.state.log.slice(-8).join(" · "));
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
          <video
            ref="videoEl"
            class="video"
            src="assets/clip-1080p.mp4"
            poster="assets/clip-1080p.poster.png"
            :autoplay="playing"
            muted
          />
        </div>

        <div class="side">
          <div class="meta">
            {{ media.state.videoWidth }}×{{ media.state.videoHeight }} · H.264 High + AAC ·
            readyState {{ media.state.readyState }}
          </div>

          <div class="row">
            <div class="btn" @click="toggle">{{ stopped ? "play" : "pause" }}</div>
            <div class="btn ghost" @click="media.seek(Math.max(0, media.state.currentTime - 5))">
              −5s
            </div>
            <div class="btn ghost" @click="media.seek(media.state.currentTime + 5)">+5s</div>
            <div class="btn ghost" :class="{ on: muted }" @click="toggleMute">
              {{ muted ? "muted" : "sound" }}
            </div>
            <div class="btn ghost" :class="{ on: looping }" @click="toggleLoop">loop</div>
          </div>

          <div class="row">
            <div class="btn ghost" :class="{ on: rate === 0.5 }" @click="setRate(0.5)">0.5×</div>
            <div class="btn ghost" :class="{ on: rate === 1 }" @click="setRate(1)">1×</div>
            <div class="btn ghost" :class="{ on: rate === 2 }" @click="setRate(2)">2×</div>
            <div class="btn ghost" @click="media.seek(2)">seek 2s</div>
            <div class="btn ghost" @click="media.seek(8)">seek 8s</div>
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
            {{ formatTime(media.state.currentTime) }} / {{ formatTime(media.state.duration) }}
            · {{ stopped ? "paused" : "playing" }}
            <span v-if="media.state.ended"> · ended</span>
            <span v-if="media.state.seeking"> · seeking</span>
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
          <video ref="brokenEl" class="video" src="assets/unsupported-codec.mkv" />
        </div>
        <div class="side">
          <div class="meta bad" v-if="broken.state.error">
            MediaError code {{ broken.state.error }}
          </div>
          <div class="meta" v-else>loading…</div>
          <div class="advisory">
            {{ broken.state.errorMessage || "no error yet" }}
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
