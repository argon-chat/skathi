<script setup lang="ts">
import { TINY_CLIP_DATA_URI } from "../tinyClip";

import { computed, onMounted, reactive, ref } from "vue";

const videoEl = ref<VnMediaElement | null>(null);
const brokenEl = ref<VnMediaElement | null>(null);

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

/** This handler is only ever bound to the one element above, so that is what the cast says. */
function onMedia(e: VnEvent) {
  log.value.push(e.type);
  if (log.value.length > 40) log.value.shift();
  readBack((e.target as VnMediaElement | null) ?? videoEl.value);
}

function onBrokenError(e: VnEvent) {
  // `e.target.error` is a real MediaError: `{ code, message }`, where the message is the advisory
  // naming what THIS machine cannot decode.
  brokenError.value = (e.target as VnMediaElement | null)?.error ?? null;
}

const muted = ref(true); // browsers require it for autoplay; be a good neighbour
const looping = ref(false);
const rate = ref(1);

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
  // The seek lands on the host immediately; mirror it so a dragged scrub bar does not snap back
  // for the ~250 ms until `seeked` arrives.
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

function formatTime(seconds: number) {
  if (!Number.isFinite(seconds)) return "--:--";
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${String(m).padStart(2, "0")}:${s.toFixed(1).padStart(4, "0")}`;
}

const scrubbing = ref(false);

function seekFromEvent(e: VnMouseEvent) {
  // `currentTarget` is rewritten at each step of the propagation walk, so it is the `.track` div
  // this handler is bound to — not whatever was clicked.
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

const CODECS: [label: string, mime: string][] = [
  ["H.264 + AAC (MP4)", 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'],
  ["HEVC / H.265 (MP4)", 'video/mp4; codecs="hev1.1.6.L93.B0"'],
  ["AV1 (MP4)", 'video/mp4; codecs="av01.0.05M.08"'],
  ["VP9 (WebM)", 'video/webm; codecs="vp9"'],
  ["VP8 + Vorbis (WebM)", 'video/webm; codecs="vp8, vorbis"'],
  ["Ogg Theora", 'video/ogg; codecs="theora"'],
  ["FFV1 in MKV (a codec the probe does not know)", 'video/x-matroska; codecs="ffv1"'],
];
const probe = ref<{ label: string; answer: string }[]>([]);
onMounted(() => {
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
  setTimeout(() => {
    const el = videoEl.value;
    probe.value = CODECS.map(([label, type]) => ({
      label,
      answer: (el && el.canPlayType(type)) || "no",
    }));
    readBack(el);
  }, 250);
});

const recent = computed(() => log.value.slice(-8).join(" · "));

// The origin is overridable so this page can be pointed at a real server; the default is a local
// one, so the demo has no third-party dependency and no internet requirement.
const httpOrigin = globalThis.__VN_HTTP_ORIGIN__ || "http://127.0.0.1:8731";
const tinyClip = TINY_CLIP_DATA_URI;
const httpClip = `${httpOrigin}/clip-1080p.mp4`;
const httpImage = `${httpOrigin}/clip-1080p.poster.png`;
</script>

<template>
  <div class="page">
    <div class="card">
      <div class="title">&lt;video&gt; — Media Foundation, decoded on gpui's own device</div>

      <div class="stage">
        <div class="frame">
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
            {{ media.videoWidth }}×{{ media.videoHeight }} · H.264 High + AAC · readyState {{ media.readyState }}
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

          <div
            class="track"
            @mousedown="
              scrubbing = true;
              seekFromEvent($event);
            "
            @mousemove="scrubbing && seekFromEvent($event)"
            @mouseup="scrubbing = false"
            @mouseleave="scrubbing = false"
          >
            <div class="fill" :style="{ width: progress + '%' }" />
          </div>

          <div class="readout">
            {{ formatTime(media.currentTime) }} / {{ formatTime(media.duration) }} ·
            {{ stopped ? "paused" : "playing" }}
            <span v-if="media.ended"> · ended</span>
            <span v-if="media.seeking"> · seeking</span>
          </div>
          <div class="hint">events: {{ recent || "…" }}</div>
        </div>
      </div>
    </div>

    <div class="card">
      <div class="title">The error path — a source this machine cannot decode</div>
      <div class="stage">
        <div class="frame small">
          <video ref="brokenEl" class="video" src="assets/unsupported-codec.mkv" @error="onBrokenError" />
        </div>
        <div class="side">
          <div class="meta bad" v-if="brokenError">MediaError code {{ brokenError.code }}</div>
          <div class="meta" v-else>loading…</div>
          <div class="advisory">
            {{ brokenError ? brokenError.message : "no error yet" }}
          </div>
          <div class="hint">
            FFV1 (a lossless archival codec) in a Matroska container: MF reads the container fine and has no decoder for
            the codec, which is the interesting half of the failure. A browser answers MEDIA_ERR_SRC_NOT_SUPPORTED and
            stops; here the same error also names which decoders THIS machine has and what to re-encode to. The
            identical text goes to the compat advisory channel, once, whether or not anyone bound @error.
          </div>
        </div>
      </div>
    </div>

    <div class="card">
      <div class="title">src grammars — data:, http(s) and relative</div>
      <div class="src-row">
        <div class="src-cell">
          <video class="mini" :src="tinyClip" autoplay loop muted />
          <div class="src-label">
            data: URI — 6 KB of MP4 inlined in the bundle. Staged to a content-hashed file in the OS temp directory,
            because SetSourceFromByteStream OPENS the URL it is given rather than reading the stream.
          </div>
        </div>
        <div class="src-cell">
          <video class="mini" :src="httpClip" autoplay loop muted />
          <div class="src-label">
            http(s) — Media Foundation's own network source streams and range-seeks it, so a long clip starts playing
            while it is still arriving.
          </div>
        </div>
        <div class="src-cell">
          <img class="mini" :src="httpImage" alt="http image" />
          <div class="src-label">
            &lt;img&gt; over http — gpui's asset loader fetches it through the ReqwestClient the host installs,
            asynchronously: alt text for a frame, then the picture.
          </div>
        </div>
        <div class="src-cell">
          <img class="mini relative-img" src="assets/clip-1080p.poster.png" alt="relative image" />
          <div class="src-label">
            &lt;img&gt; by relative URL — the app root in dev, the exe's own payload once packaged. The same key either
            way, so the two cannot disagree.
          </div>
        </div>
      </div>
      <div class="hint">
        {{ httpOrigin }} is a local origin. An unreachable one renders the alt text (img) or raises MediaError code 2/4
        with an advisory (video) — it never hangs and never lies.
      </div>
    </div>

    <div class="card">
      <div class="title">canPlayType() — probed on this machine, right now</div>
      <div class="probe">
        <div v-for="row in probe" :key="row.label" class="probe-row">
          <div class="probe-name">{{ row.label }}</div>
          <div class="probe-answer" :class="row.answer">{{ row.answer }}</div>
        </div>
      </div>
      <div class="hint">
        Only the first row is guaranteed: H.264 + AAC in MP4 is what Windows ships inbox. Every other line depends on
        Store codec extensions that may not be installed — which is why this is a runtime probe and never a compiled-in
        table.
      </div>
    </div>
  </div>
</template>

<style scoped>
.src-row {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
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
