<script setup lang="ts">
// Web Audio — a real Firewheel graph behind the browser's AudioContext.
//
// Nothing on this page is host-specific except one clearly-marked node: it is
// `new AudioContext()`, `createOscillator()`, `createGain()`,
// `createAnalyser()`, `decodeAudioData()` and `AudioBufferSourceNode`, wired
// the way the same code would be wired in Chrome. What is host-specific is
// what is UNDERNEATH: `crates/vn-runtime` turns on redjs's `audio` feature,
// which maps the node graph onto Firewheel and opens a cpal output stream on
// the OS default device. There is no shim and no polyfill — with the feature
// off, `AudioContext` is not defined at all, so feature detection tells the
// truth either way.
//
// THE GRAPH, and why it has two analysers:
//
//     oscillator ──▶ pre ──▶ gain ──▶ post ──▶ destination
//                    │                 │
//                 the bars          the proof
//
// `pre` taps the signal BEFORE the gain, `post` after it. With the gain at 0
// the bars keep moving (there IS a tone) while the post-gain RMS reads 0.000
// (nothing leaves the graph). That pairing is the measurement: a stub analyser
// returning zeros would fail the first half, and a `gain.value` that did
// nothing would fail the second. It is also why this page opens SILENT — the
// gain starts at 0 on purpose, and the readouts below tell you the oscillator
// is running anyway.
//
// The bars are plain divs whose heights come off `getByteFrequencyData` inside
// a `requestAnimationFrame` loop. No canvas: 48 divs restyling every frame is
// a fine workout for the cascade, and it means the visualization is inspectable
// with the devtools selector query like any other element.

import { computed, onActivated, onDeactivated, onMounted, onUnmounted, reactive, ref } from "vue";

// --- context ----------------------------------------------------------------

// `AudioContext` is ABSENT, not stubbed, when the host was built without the
// `audio` feature — so this probe is the honest question and env.d.ts declares
// the global `| undefined` to keep it meaningful.
const supported = typeof AudioContext !== "undefined";
const ctx = ref<VnAudioContext | null>(null);
const state = reactive({
  ctxState: "—",
  sampleRate: 0,
  currentTime: 0,
  running: false, // the clock advances ⇒ there is a real output device
});

// Nodes, kept out of `reactive()` on purpose: they are host objects with live
// getters, not data, and wrapping them in a Proxy buys nothing but confusion.
let osc: VnOscillatorNode | null = null;
let preAnalyser: VnAnalyserNode | null = null;
let postAnalyser: VnAnalyserNode | null = null;
// `VnGainNodeWithParam`, not `VnGainNode`: engine.d.ts types the latter's
// `gain` as `{ value: number }` — the half the LiveKit shim writes — and
// `rampGain()` below drives the AudioParam TIMELINE, which redjs implements in
// full. See the note on the type in apps/env.d.ts.
let gainNode: VnGainNodeWithParam | null = null;
let streamNode: VnRedStreamSourceNode | null = null;

const BARS = 48;
const bars = ref<number[]>(new Array(BARS).fill(0));
const meters = reactive({ pre: 0, post: 0 });
const log = ref<string[]>([]);

function note(line: string) {
  log.value.push(line);
  if (log.value.length > 60) log.value.shift();
}

function ensureContext(): VnAudioContext {
  if (ctx.value) return ctx.value;
  // Guarded by `supported` at every entry point (`onMounted` returns early,
  // and the whole control surface is behind `v-else` of `v-if="!supported"`),
  // so reaching here with no constructor is not a state this page can be in.
  const a = new AudioContext!();
  ctx.value = a;

  preAnalyser = a.createAnalyser();
  postAnalyser = a.createAnalyser();
  // 2048 → 1024 frequency bins; we fold them into 48 bars below.
  preAnalyser.fftSize = 2048;
  postAnalyser.fftSize = 2048;
  preAnalyser.smoothingTimeConstant = 0.7;

  gainNode = a.createGain() as VnGainNodeWithParam;
  gainNode.gain.value = gain.value; // 0 at boot — see the header

  preAnalyser.connect(gainNode);
  gainNode.connect(postAnalyser);
  postAnalyser.connect(a.destination);

  state.sampleRate = a.sampleRate;
  state.ctxState = a.state;
  note(`AudioContext @ ${a.sampleRate} Hz, state=${a.state}`);
  return a;
}

// --- oscillator -------------------------------------------------------------

const WAVES = ["sine", "square", "sawtooth", "triangle"];
const wave = ref<string>("square");
const freq = ref(440);
const detune = ref(0);
const playing = ref(false);

function startTone() {
  const a = ensureContext();
  if (osc) stopTone();
  osc = a.createOscillator();
  osc.type = wave.value;
  osc.frequency.value = freq.value;
  osc.detune.value = detune.value;
  osc.onended = () => note("oscillator: onended");
  // `preAnalyser` is built by `ensureContext()` on the line above.
  osc.connect(preAnalyser!);
  osc.start();
  playing.value = true;
  note(`oscillator start — ${wave.value} @ ${freq.value} Hz`);
}

function stopTone() {
  if (!osc) return;
  osc.stop();
  osc.disconnect();
  osc = null;
  playing.value = false;
  note("oscillator stop");
}

function setWave(w: string) {
  wave.value = w;
  // A live `type` write reaches the audio thread on the next block — no
  // rewiring, no click, exactly as in a browser.
  if (osc) osc.type = w;
}

function setFreq(hz: number) {
  freq.value = Math.round(hz);
  if (osc) osc.frequency.value = freq.value;
}

function setDetune(cents: number) {
  detune.value = Math.round(cents);
  if (osc) osc.detune.value = detune.value;
}

// --- sliders ----------------------------------------------------------------
//
// There is no `<input type="range">` in this renderer, so a track is a div and
// the value is arithmetic on `getBoundingClientRect()` — which works because
// the DOM surface answers it off the real painted rectangle. Same shape as the
// Media page's scrub bar.

/** 0..1 across the clicked track. */
function fraction(e: VnMouseEvent) {
  // Rewritten at each step of the propagation walk, so it is the `.track` div
  // this handler is bound to rather than whatever was clicked inside it.
  const track = e.currentTarget as VnElement | null;
  if (!track) return 0;
  const rect = track.getBoundingClientRect();
  if (!rect.width) return 0;
  return Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
}

// --- gain -------------------------------------------------------------------

// STARTS AT ZERO. The page is a measurement, not a noise; raise it yourself if
// you actually want to hear the tone.
const gain = ref(0);

function setGain(v: number) {
  gain.value = Math.round(Math.min(1, Math.max(0, v)) * 100) / 100;
  if (gainNode) gainNode.gain.value = gain.value;
}

/** A scheduled fade, so the AudioParam timeline is exercised, not just `.value`. */
function rampGain(target: number, seconds: number) {
  const a = ensureContext();
  if (!gainNode) return;
  const t0 = a.currentTime;
  gainNode.gain.cancelScheduledValues(t0);
  gainNode.gain.setValueAtTime(Math.max(gain.value, 0.0001), t0);
  // Exponential, because that is what a fade that sounds even to the ear is —
  // and because this runtime implements it as a real geometric ramp now
  // rather than as a linear stand-in.
  gainNode.gain.exponentialRampToValueAtTime(Math.max(target, 0.0001), t0 + seconds);
  gain.value = target;
  note(`gain: exponential ramp to ${target} over ${seconds}s`);
}

// --- generated AudioBuffer (a chirp made in JS) ------------------------------

const chirp = reactive<{ buffer: VnAudioBuffer | null; playing: boolean; peak: number }>({
  buffer: null,
  playing: false,
  peak: 0,
});

function buildChirp() {
  const a = ensureContext();
  const seconds = 1.2;
  const rate = a.sampleRate || 48000;
  const frames = Math.floor(seconds * rate);
  // createBuffer + copyToChannel: the browser's way to hand raw PCM to the
  // graph without a file. Stereo, so the two channels can differ audibly.
  const buf = a.createBuffer(2, frames, rate);
  const left = new Float32Array(frames);
  const right = new Float32Array(frames);
  const f0 = 180;
  const f1 = 2400;
  const k = Math.log(f1 / f0) / seconds;
  let peak = 0;
  for (let i = 0; i < frames; i++) {
    const t = i / rate;
    // Exponential sweep: phase is the integral of the instantaneous frequency.
    const phase = (2 * Math.PI * f0 * (Math.exp(k * t) - 1)) / k;
    // Hann window so the ends do not click.
    const win = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / frames);
    const s = Math.sin(phase) * win;
    left[i] = s;
    right[i] = s * 0.4; // an audible L/R difference to prove the channels are separate
    if (Math.abs(s) > peak) peak = Math.abs(s);
  }
  buf.copyToChannel(left, 0);
  buf.copyToChannel(right, 1);
  chirp.buffer = buf;
  chirp.peak = Math.round(peak * 1000) / 1000;
  note(`chirp built: ${buf.numberOfChannels}ch × ${buf.length} frames @ ${buf.sampleRate} Hz`);
}

function playChirp() {
  const a = ensureContext();
  if (!chirp.buffer) buildChirp();
  const src = a.createBufferSource();
  src.buffer = chirp.buffer;
  src.connect(preAnalyser!);
  src.onended = () => {
    chirp.playing = false;
    note("chirp: onended");
  };
  src.start();
  chirp.playing = true;
  note("chirp: start");
}

// --- decodeAudioData --------------------------------------------------------

const decoded = reactive({ status: "not loaded", channels: 0, frames: 0, rate: 0, seconds: 0, peak: 0 });
let decodedBuffer: VnAudioBuffer | null = null;

async function loadFixture() {
  const a = ensureContext();
  decoded.status = "fetching…";
  try {
    // engine.d.ts declares `fetch` optional — a bare engine may not have it.
    // The app host does.
    // Ordinary `fetch` of an app asset, ordinary `arrayBuffer()`, ordinary
    // `decodeAudioData`. The decoder underneath is symphonia; WAV, FLAC, MP3,
    // OGG/Vorbis, AAC-LC/M4A, ALAC, AIFF, CAF and MKV all land here.
    const res = await fetch!("assets/chirp-mono-22k.wav");
    if (!res.ok) throw new Error(`fetch: HTTP ${res.status}`);
    const bytes = await res.arrayBuffer();
    decoded.status = `decoding ${bytes.byteLength} bytes…`;
    const buf = await a.decodeAudioData(bytes);
    decodedBuffer = buf;
    const ch0 = buf.getChannelData(0);
    let peak = 0;
    for (let i = 0; i < ch0.length; i++) peak = Math.max(peak, Math.abs(ch0[i]!));
    decoded.channels = buf.numberOfChannels;
    decoded.frames = buf.length;
    // NOTE the resample: the fixture is 22 050 Hz on disk and comes back at the
    // OUTPUT stream's rate, because that is what the graph needs. `duration`
    // is preserved; `length` is not the file's frame count.
    decoded.rate = buf.sampleRate;
    decoded.seconds = Math.round(buf.duration * 1000) / 1000;
    decoded.peak = Math.round(peak * 1000) / 1000;
    decoded.status = "decoded";
    note(`decodeAudioData: ${buf.numberOfChannels}ch, ${buf.length} frames @ ${buf.sampleRate} Hz`);
  } catch (e) {
    // The advisory is the error message: a rejection here names the formats
    // that WOULD have worked and the ones that never will.
    decoded.status = "failed";
    note(`decodeAudioData failed: ${e instanceof Error ? e.message : String(e)}`);
  }
}

function playDecoded() {
  const a = ensureContext();
  if (!decodedBuffer) return;
  const src = a.createBufferSource();
  src.buffer = decodedBuffer;
  src.connect(preAnalyser!);
  src.start();
  note("decoded fixture: start");
}

// --- streaming PCM (RedStreamSourceNode) ------------------------------------
//
// NOT a browser API, and named so you can tell at a glance. This is the seam a
// native producer — a LiveKit/WebRTC remote track, a decoder, a capture bridge
// — pushes interleaved f32 frames into so they play out of the graph. The Rust
// side is `redjs::webapi::pcm::sink(streamId)` → `PcmSink::push_interleaved`,
// which is Send + Sync and never touches the JS context.
//
// Here we drive it from JS, which is the same pipe with a shorter cable: it is
// what makes the seam demoable and testable without writing native code.

const stream = reactive({ open: false, id: -1, pushed: 0, queued: 0, ready: false, under: false, over: false });
let streamPhase = 0;
let streamBlock: Float32Array | null = null;

function openStream() {
  const a = ensureContext();
  if (streamNode) closeStream();
  streamNode = a.createRedStreamSource({
    sampleRate: 48000,
    channelCount: 1,
    latencySeconds: STREAM_TARGET_SECONDS,
  });
  streamNode.connect(preAnalyser!);
  streamBlock = new Float32Array(STREAM_BLOCK);
  streamPhase = 0;
  stream.open = true;
  stream.id = streamNode.streamId;
  stream.pushed = 0;
  // The glitch flags latch until the stream is reopened, so reopening has to
  // clear them — otherwise last session's overflow reads as this one's.
  stream.under = false;
  stream.over = false;
  streamClock = performance.now();
  streamDebt = 0;
  note(`RedStreamSourceNode open — streamId=${streamNode.streamId} (hand this to native code)`);
}

function closeStream() {
  if (!streamNode) return;
  // Probed rather than assumed: making a late native push report NotReady is
  // the seam's lifetime rule, and not every engine revision has the method.
  streamNode.close?.();
  streamNode.disconnect();
  streamNode = null;
  stream.open = false;
  stream.ready = false;
  note("RedStreamSourceNode closed — further pushes are refused, not swallowed");
}

/** The buffer depth we open the channel with, in seconds. */
const STREAM_TARGET_SECONDS = 0.08;
/** Frames per push — 10 ms at 48 kHz, about what a voice codec hands you. */
const STREAM_BLOCK = 480;

let streamClock = 0; // performance.now() of the last feed
let streamDebt = 0; // frames owed to the channel

/**
 * Called once per rAF while the stream is open: produce as many frames as REAL
 * TIME has passed, and push them.
 *
 * Pacing by the wall clock rather than by the buffer depth is the honest model,
 * and it is the one a LiveKit track actually has: the producer owns a clock
 * (the sender's), the device owns another, and the channel's job is to absorb
 * the difference. Two things you can only see by doing it this way:
 *
 * * `queuedSeconds` reads 80 ms the instant the node is created and nothing has
 *   been pushed — `fixed_resample` PRE-FILLS the channel with `latencySeconds`
 *   of silence so the device has something to read before the first frame
 *   arrives. A producer that waits for the queue to drop below the target
 *   therefore waits forever (the underflow corrector keeps topping it back up),
 *   which is exactly the bug this comment exists to stop someone rediscovering.
 * * rAF runs at the display rate — 165 Hz here — so a fixed block per frame is
 *   1.65× realtime and overflows continuously. The `overflowed` flag below is
 *   what tells you, and it is the reason it is worth surfacing.
 *
 * The catch-up is capped at 4 blocks: after a long stall (a tab switch, a
 * breakpoint) a producer should drop the missed audio, not dump 2 seconds of
 * stale voice into the channel at once.
 */
function feedStream() {
  if (!streamNode || !streamBlock) return;
  const now = performance.now();
  const dt = Math.min(0.25, Math.max(0, (now - streamClock) / 1000));
  streamClock = now;
  streamDebt += dt * 48000;

  let blocks = Math.min(4, Math.floor(streamDebt / STREAM_BLOCK));
  streamDebt -= blocks * STREAM_BLOCK;
  while (blocks-- > 0) {
    for (let i = 0; i < streamBlock!.length; i++) {
      streamBlock![i] = Math.sin(streamPhase) * 0.8;
      streamPhase += (2 * Math.PI * 660) / 48000;
      if (streamPhase > 2 * Math.PI) streamPhase -= 2 * Math.PI;
    }
    stream.pushed += streamNode.push(streamBlock!);
  }

  stream.queued = Math.round((streamNode.queuedSeconds ?? 0) * 1000);
  stream.ready = streamNode.ready;
  // Read-and-clear on the host side, so these latch here until the stream is
  // reopened — which is what makes a rare glitch visible after the fact.
  if (streamNode.underflowed) stream.under = true;
  if (streamNode.overflowed) stream.over = true;
}

// --- suspend / resume -------------------------------------------------------

async function toggleSuspend() {
  const a = ensureContext();
  if (a.state === "suspended") {
    await a.resume();
    note(`resume() → state=${a.state}`);
  } else {
    await a.suspend();
    note(`suspend() → state=${a.state}`);
  }
  state.ctxState = a.state;
}

// --- the rAF loop -----------------------------------------------------------

let raf = 0;
let alive = false;
// The byte view the analyser fills. Allocated once — a new Uint8Array per frame
// at 165 fps is 27 MB/s of garbage.
let freqBytes: Uint8Array | null = null;
let preTime: Float32Array | null = null;
let postTime: Float32Array | null = null;

function rms(view: Float32Array) {
  let acc = 0;
  for (let i = 0; i < view.length; i++) acc += view[i]! * view[i]!;
  return Math.sqrt(acc / view.length);
}

function frame() {
  if (!alive) return;
  const a = ctx.value;
  if (a && preAnalyser) {
    if (!freqBytes || freqBytes.length !== preAnalyser.frequencyBinCount) {
      freqBytes = new Uint8Array(preAnalyser.frequencyBinCount);
      preTime = new Float32Array(1024);
      postTime = new Float32Array(1024);
    }
    if (stream.open) feedStream();

    preAnalyser.getByteFrequencyData(freqBytes!);
    // Fold ~1024 linear bins into 48 bars. Logarithmic-ish widths, so the bass
    // does not get one bar and the top octave forty.
    const out: number[] = new Array(BARS);
    const n = freqBytes!.length;
    for (let b = 0; b < BARS; b++) {
      const lo = Math.floor(Math.pow(b / BARS, 2) * n);
      const hi = Math.max(lo + 1, Math.floor(Math.pow((b + 1) / BARS, 2) * n));
      let peak = 0;
      for (let i = lo; i < hi && i < n; i++) if (freqBytes![i]! > peak) peak = freqBytes![i]!;
      out[b] = peak;
    }
    bars.value = out;

    preAnalyser.getFloatTimeDomainData(preTime!);
    postAnalyser!.getFloatTimeDomainData(postTime!);
    meters.pre = Math.round(rms(preTime!) * 1000) / 1000;
    meters.post = Math.round(rms(postTime!) * 1000) / 1000;

    const now = Math.round(a.currentTime * 100) / 100;
    state.running = now > 0;
    state.currentTime = now;
    state.ctxState = a.state;
  }
  // engine.d.ts declares rAF optional — the unit-test harness has no frame
  // pump. Under vn-app it is the host's, one callback per PAINTED frame.
  raf = requestAnimationFrame!(frame);
}

function startLoop() {
  if (alive) return;
  alive = true;
  raf = requestAnimationFrame!(frame);
}

function stopLoop() {
  alive = false;
  if (raf) cancelAnimationFrame!(raf);
  raf = 0;
}

onMounted(() => {
  if (!supported) return;
  ensureContext();
  startLoop();
  // `VN_SCENARIO=audio` — a soak with no input: tone running, output muted.
  if (globalThis.__VN_SCENARIO__ === "audio") setTimeout(startTone, 400);
});
// KeepAlive: the page's subtree is moved out of the tree, not unmounted, so the
// rAF loop has to stop by hand or it keeps restyling an invisible page.
onActivated(startLoop);
onDeactivated(stopLoop);
onUnmounted(() => {
  stopLoop();
  stopTone();
  closeStream();
});

// --- readouts ---------------------------------------------------------------

const silent = computed(() => meters.post < 0.001);
const recent = computed(() => log.value.slice(-9).reverse());

// A capability matrix, read live off the objects rather than written down: a
// row that says "no" because the method really is absent cannot go stale.
const surface = computed<[string, string, string][]>(() => {
  if (!supported) return [];
  const a = ctx.value;
  // The probe deliberately asks for keys the interface does not declare —
  // several of these are ABSENT and the row saying so is the point. A
  // `Record<string, unknown>` view is what "ask the object, do not trust the
  // table" means in the type system.
  const bag = a as unknown as Record<string, unknown> | null;
  const has = (k: string) => (bag && typeof bag[k] === "function" ? "yes" : "—");
  return [
    ["createOscillator", has("createOscillator"), "sine/square/sawtooth/triangle, PolyBLEP band-limited"],
    ["createGain", has("createGain"), "AudioParam .value + setValueAtTime/linear/exponential/setTarget"],
    ["createAnalyser", has("createAnalyser"), "real FFT + time domain; fftSize caps at 4096 (spec: 32768)"],
    ["createBufferSource", has("createBufferSource"), "createBuffer / copyToChannel / getChannelData"],
    ["createBiquadFilter", has("createBiquadFilter"), "state-variable filter"],
    ["createPanner", has("createPanner"), "distance models + cones + REAL HRTF"],
    ["createStereoPanner", has("createStereoPanner"), "equal-power pan"],
    ["decodeAudioData", has("decodeAudioData"), "WAV, FLAC, MP3, OGG/Vorbis, AAC-LC/M4A, ALAC, AIFF, CAF, MKV"],
    ["setSinkId", has("setSinkId"), "output device selection with no <audio> element"],
    ["createRedStreamSource", has("createRedStreamSource"), "NOT a browser API — the streaming-PCM seam"],
    ["createMediaElementSource", has("createMediaElementSource"), "present but SILENT — see the advisory below"],
    ["createDelay", has("createDelay"), "absent, not stubbed"],
    ["createConvolver", has("createConvolver"), "absent, not stubbed"],
    ["createDynamicsCompressor", has("createDynamicsCompressor"), "absent, not stubbed"],
    ["createChannelSplitter", has("createChannelSplitter"), "absent — connect() has no port indices yet"],
    ["createPeriodicWave", has("createPeriodicWave"), "absent — oscillator.type = 'custom' throws by name"],
    ["audioWorklet", bag && bag.audioWorklet ? "yes" : "—", "absent — no worklet thread, no WASM in the runtime"],
    ["OfflineAudioContext", typeof OfflineAudioContext !== "undefined" ? "yes" : "—", "absent — no offline rendering"],
  ];
});
</script>

<template>
  <div class="page">
    <!-- ===== not supported ============================================== -->
    <div v-if="!supported" class="card">
      <div class="title">Web Audio</div>
      <div class="advisory">
        <div class="ahead">AudioContext is not defined in this build.</div>
        <div>
          The host was compiled with --no-default-features (or vn-runtime/audio
          off), so the Web Audio implementation is not linked in at all. It is
          deliberately absent rather than stubbed: a feature probe for
          AudioContext is the standard way to ask, and a silent fake would make
          it lie.
        </div>
      </div>
    </div>

    <template v-else>
      <!-- ===== the graph ================================================= -->
      <div class="card">
        <div class="title">
          oscillator → analyser → gain → analyser → destination
        </div>
        <div class="sub">
          Firewheel under a browser-shaped AudioContext. The gain starts at 0 —
          the bars below are tapped BEFORE it, so they move while nothing leaves
          the graph.
        </div>

        <!-- the spectrum, 48 plain divs -->
        <div class="bars">
          <div
            v-for="(v, i) in bars"
            :key="i"
            class="bar"
            :class="{ hot: v > 200 }"
            :style="{ height: Math.max(2, (v / 255) * 96) + 'px' }"
          />
        </div>

        <div class="row">
          <div class="group">
            <span class="lbl">wave</span>
            <div
              v-for="w in WAVES"
              :key="w"
              class="chip"
              :class="{ on: wave === w }"
              @click="setWave(w)"
            >{{ w }}</div>
          </div>
          <div class="group">
            <div class="btn" :class="{ primary: !playing }" @click="playing ? stopTone() : startTone()">
              {{ playing ? "stop" : "start" }}
            </div>
          </div>
        </div>

        <div class="slider-row">
          <span class="lbl">frequency</span>
          <div class="track" @click="(e: VnMouseEvent) => setFreq(40 + fraction(e) * 4000)">
            <div class="fill" :style="{ width: ((freq - 40) / 4000) * 100 + '%' }" />
          </div>
          <span class="val">{{ freq }} Hz</span>
        </div>

        <div class="slider-row">
          <span class="lbl">detune</span>
          <div class="track" @click="(e: VnMouseEvent) => setDetune(-1200 + fraction(e) * 2400)">
            <div class="fill" :style="{ width: ((detune + 1200) / 2400) * 100 + '%' }" />
          </div>
          <span class="val">{{ detune }} cents</span>
        </div>

        <div class="slider-row">
          <span class="lbl">gain</span>
          <div class="track" @click="(e: VnMouseEvent) => setGain(fraction(e))">
            <div class="fill gainfill" :style="{ width: gain * 100 + '%' }" />
          </div>
          <span class="val">{{ gain.toFixed(2) }}</span>
          <div class="btn small" @click="rampGain(0.15, 1.5)">ramp ↑</div>
          <div class="btn small" @click="rampGain(0.0001, 1.5)">ramp ↓</div>
        </div>

        <!-- The measurement. This is the part that makes "it works" checkable
             without listening to anything. -->
        <div class="meters">
          <div class="meter">
            <span class="mlbl">pre-gain RMS</span>
            <span class="mval" :class="{ live: meters.pre > 0.001 }">{{ meters.pre.toFixed(3) }}</span>
          </div>
          <div class="meter">
            <span class="mlbl">post-gain RMS</span>
            <span class="mval" :class="{ live: meters.post > 0.001 }">{{ meters.post.toFixed(3) }}</span>
          </div>
          <div class="verdict" :class="{ quiet: silent }">
            {{ silent ? "output silent" : "output audible" }}
          </div>
        </div>
      </div>

      <!-- ===== context state ============================================= -->
      <div class="card">
        <div class="title">AudioContext — state, clock, device</div>
        <div class="kv">
          <div><span class="k">state</span><span class="v">{{ state.ctxState }}</span></div>
          <div><span class="k">sampleRate</span><span class="v">{{ state.sampleRate }} Hz</span></div>
          <div><span class="k">currentTime</span><span class="v">{{ state.currentTime.toFixed(2) }} s</span></div>
          <div>
            <span class="k">output stream</span>
            <span class="v">{{ state.running ? "running (clock advancing)" : "none (clock frozen)" }}</span>
          </div>
        </div>
        <div class="row">
          <div class="btn" @click="toggleSuspend">
            {{ state.ctxState === "suspended" ? "resume()" : "suspend()" }}
          </div>
        </div>
        <div v-if="!state.running" class="advisory">
          <div class="ahead">No output device.</div>
          <div>
            The graph API still works — nodes connect, params take,
            decodeAudioData decodes — but currentTime never advances, so
            anything scheduled at an absolute future time never fires. This is
            the headless case, and it degrades rather than crashing.
          </div>
        </div>
        <div class="advisory soft">
          <div class="ahead">Deviation: suspend() mutes, it does not pause.</div>
          <div>
            The master bus goes to zero but the stream keeps running, so
            currentTime keeps advancing while suspended. The pre-gain analyser
            above keeps seeing the oscillator for the same reason — it sits
            upstream of the master.
          </div>
        </div>
      </div>

      <!-- ===== AudioBuffer built in JS =================================== -->
      <div class="card">
        <div class="title">AudioBuffer — a chirp synthesized in JS</div>
        <div class="sub">
          createBuffer(2, n, rate) + copyToChannel(), then an
          AudioBufferSourceNode. A 180 Hz → 2.4 kHz exponential sweep,
          Hann-windowed, right channel at 0.4× the left.
        </div>
        <div class="row">
          <div class="btn" @click="buildChirp">build buffer</div>
          <div class="btn primary" :class="{ off: !chirp.buffer }" @click="playChirp">play</div>
          <span v-if="chirp.buffer" class="val">
            {{ chirp.buffer.numberOfChannels }}ch ·
            {{ chirp.buffer.length }} frames ·
            {{ chirp.buffer.sampleRate }} Hz ·
            {{ chirp.buffer.duration.toFixed(2) }}s · peak {{ chirp.peak }}
          </span>
        </div>
        <div class="sub">
          Watch the bars sweep left→right while post-gain RMS stays 0.000 at
          gain 0 — the sweep is real signal that nobody can hear.
        </div>
      </div>

      <!-- ===== decodeAudioData =========================================== -->
      <div class="card">
        <div class="title">decodeAudioData — a WAV off disk</div>
        <div class="sub">
          fetch("assets/chirp-mono-22k.wav") → arrayBuffer() →
          decodeAudioData(). The decoder is symphonia; the fixture is 16-bit
          mono PCM at 22 050 Hz.
        </div>
        <div class="row">
          <div class="btn" @click="loadFixture">fetch + decode</div>
          <div class="btn primary" :class="{ off: decoded.status !== 'decoded' }" @click="playDecoded">play</div>
          <span class="val">{{ decoded.status }}</span>
        </div>
        <div v-if="decoded.status === 'decoded'" class="kv">
          <div><span class="k">channels</span><span class="v">{{ decoded.channels }}</span></div>
          <div><span class="k">frames</span><span class="v">{{ decoded.frames }}</span></div>
          <div><span class="k">sampleRate</span><span class="v">{{ decoded.rate }} Hz</span></div>
          <div><span class="k">duration</span><span class="v">{{ decoded.seconds }} s</span></div>
          <div><span class="k">peak</span><span class="v">{{ decoded.peak }}</span></div>
        </div>
        <div v-if="decoded.status === 'decoded' && decoded.rate !== 22050" class="advisory soft">
          <div class="ahead">Resampled on decode.</div>
          <div>
            The file is 22 050 Hz; the buffer comes back at {{ decoded.rate }} Hz
            because that is the output stream's rate. duration is preserved,
            length is not the file's frame count. Browsers do the same thing.
          </div>
        </div>
      </div>

      <!-- ===== streaming PCM ============================================= -->
      <div class="card">
        <div class="titlerow">
          <div class="title">RedStreamSourceNode — streaming PCM in</div>
          <div class="tag">not a browser API</div>
        </div>
        <div class="sub">
          The seam a native producer pushes decoded frames into: a LiveKit
          remote track, a decoder, a capture bridge. Named with a Red prefix on
          purpose — the browser's createMediaStreamSource presumes a MediaStream
          the engine PULLS from, and this runtime has neither.
        </div>
        <div class="sub">
          Rust side: redjs::webapi::pcm::sink(streamId).push_interleaved(&amp;[f32])
          — Send + Sync, non-blocking, and it never touches the JS context or
          the graph. Below, the same pipe is driven from JS with a shorter cable,
          paced by the wall clock the way a real track is.
        </div>
        <div class="row">
          <div class="btn" :class="{ primary: !stream.open }" @click="stream.open ? closeStream() : openStream()">
            {{ stream.open ? "close" : "open + feed 660 Hz" }}
          </div>
          <span v-if="stream.open" class="val">
            streamId {{ stream.id }} · pushed {{ stream.pushed }} frames ·
            queued {{ stream.queued }} ms · {{ stream.ready ? "consuming" : "not consuming" }}
          </span>
        </div>
        <div v-if="stream.under || stream.over" class="advisory soft">
          <div class="ahead">Backpressure observed since this stream opened</div>
          <div v-if="stream.under">underflow — the producer fell behind the device, and silence was inserted to re-establish the latency target</div>
          <div v-if="stream.over">overflow — the producer ran ahead, and frames were dropped rather than blocking the pushing thread</div>
        </div>
      </div>

      <!-- ===== surface ==================================================== -->
      <div class="card">
        <div class="title">What is really here</div>
        <div class="sub">Read live off the object, not written down — a row cannot go stale.</div>
        <div class="matrix">
          <div v-for="row in surface" :key="row[0]" class="mrow" :class="{ missing: row[1] === '—' }">
            <span class="mname">{{ row[0] }}</span>
            <span class="mhas">{{ row[1] }}</span>
            <span class="mnote">{{ row[2] }}</span>
          </div>
        </div>
        <div class="advisory">
          <div class="ahead">createMediaElementSource returns a SILENT pass-through.</div>
          <div>
            It is neither faked nor thrown. A &lt;video&gt; or &lt;audio&gt;
            element decodes AND plays out inside the platform media engine
            (IMFMediaEngine on Windows), which owns its own output stream and
            never hands the graph a sample; routing it here would mean decoding
            the file a second time, out of sync with the picture.
          </div>
          <div class="gap">
            The node still connects, because try/catch around this call is the
            standard capability probe and throwing would send callers down the
            wrong branch — but everything downstream sees zeros, and the host
            prints this same advisory on stderr the first time you call it. For
            audio you want inside the graph: decodeAudioData, or
            createRedStreamSource.
          </div>
        </div>
      </div>

      <!-- ===== log ======================================================== -->
      <div class="card">
        <div class="title">Log</div>
        <div class="log">
          <div v-for="(line, i) in recent" :key="i" class="logline">{{ line }}</div>
        </div>
      </div>
    </template>
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
.sub {
  font-size: 11px;
  color: #ffffff88;
  line-height: 16px;
}
.tag {
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 5px;
  color: #ffd479;
  background: #ffd4791a;
  border: 1px solid #ffd47940;
}

/* --- the spectrum ------------------------------------------------------- */
.bars {
  display: flex;
  flex-direction: row;
  align-items: flex-end;
  gap: 2px;
  height: 100px;
  padding: 2px 6px;
  border-radius: 10px;
  background: #0b1016;
  border: 1px solid #7fd4ff33;
}
.bar {
  width: 10px;
  border-radius: 3px;
  background: #4f8cffcc;
  transition: background-color 120ms ease;
}
.bar.hot {
  background: #ff9f7fdd;
}

/* --- controls ----------------------------------------------------------- */
.row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  align-items: center;
}
.group {
  display: flex;
  flex-direction: row;
  gap: 6px;
  align-items: center;
}
.lbl {
  font-size: 11px;
  color: #ffffff88;
  width: 70px;
}
.val {
  font-size: 11px;
  color: #ffffffcc;
}
.chip {
  padding: 5px 9px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffffaa;
  background: #ffffff0d;
  cursor: pointer;
  transition: background-color 150ms ease, color 150ms ease;
}
.chip:hover {
  background: #ffffff22;
  color: #ffffff;
}
.chip.on {
  background: #4f8cff59;
  color: #ffffff;
}
.btn {
  padding: 6px 12px;
  border-radius: 6px;
  font-size: 12px;
  color: #ffffffcc;
  background: #ffffff14;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.btn:hover {
  background: #ffffff2a;
}
.btn.primary {
  background: #4f8cff59;
  color: #ffffff;
}
.btn.small {
  padding: 4px 8px;
  font-size: 11px;
}
.btn.off {
  color: #ffffff55;
  background: #ffffff08;
}

.slider-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  align-items: center;
}
.track {
  flex: 1;
  height: 8px;
  border-radius: 4px;
  background: #ffffff1a;
  cursor: pointer;
  overflow: hidden;
}
.fill {
  height: 8px;
  border-radius: 4px;
  background: #4f8cffcc;
}
.fill.gainfill {
  background: #ff9f7fcc;
}

/* --- meters ------------------------------------------------------------- */
.meters {
  display: flex;
  flex-direction: row;
  gap: 10px;
  align-items: center;
}
.meter {
  display: flex;
  flex-direction: row;
  gap: 6px;
  align-items: center;
  padding: 6px 10px;
  border-radius: 8px;
  background: #0b1016;
  border: 1px solid #ffffff1a;
}
.mlbl {
  font-size: 11px;
  color: #ffffff77;
}
.mval {
  font-size: 13px;
  color: #ffffff55;
}
.mval.live {
  color: #7fffb0;
}
.verdict {
  padding: 6px 10px;
  border-radius: 8px;
  font-size: 11px;
  color: #ff9f7f;
  background: #ff9f7f1a;
  border: 1px solid #ff9f7f40;
}
.verdict.quiet {
  color: #7fffb0;
  background: #7fffb01a;
  border-color: #7fffb040;
}

/* --- key/value + matrix -------------------------------------------------- */
.kv {
  display: flex;
  flex-direction: row;
  gap: 14px;
  flex-wrap: wrap;
}
.kv > div {
  display: flex;
  flex-direction: row;
  gap: 6px;
  align-items: baseline;
}
.k {
  font-size: 10px;
  color: #ffffff66;
}
.v {
  font-size: 12px;
  color: #ffffffdd;
}
.matrix {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.mrow {
  display: flex;
  flex-direction: row;
  gap: 10px;
  align-items: baseline;
  padding: 3px 6px;
  border-radius: 5px;
  background: #7fffb00d;
}
.mrow.missing {
  background: #ffffff08;
}
.mname {
  width: 200px;
  font-size: 11px;
  color: #ffffffdd;
}
.mhas {
  width: 34px;
  font-size: 11px;
  color: #7fffb0;
}
.mrow.missing .mhas {
  color: #ffffff44;
}
.mnote {
  flex: 1;
  font-size: 10px;
  color: #ffffff77;
}

/* --- advisories + log ---------------------------------------------------- */
.advisory {
  padding: 10px;
  border-radius: 8px;
  font-size: 11px;
  line-height: 17px;
  color: #ffd479;
  background: #ffd4791a;
  border: 1px solid #ffd47940;
}
.advisory.soft {
  color: #ffffff99;
  background: #ffffff0d;
  border-color: #ffffff1a;
}
/* Inline elements do not reflow in this renderer — there is no inline layout
   pass, so a <code> or a <b> inside a sentence becomes its own line. Prose is
   therefore written as plain text and structure comes from stacked blocks. */
.advisory .ahead {
  color: #ffffff;
  margin-bottom: 4px;
}
.advisory .gap {
  margin-top: 6px;
}
.titlerow {
  display: flex;
  flex-direction: row;
  gap: 8px;
  align-items: center;
}
.log {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 8px;
  border-radius: 8px;
  background: #0b1016;
}
.logline {
  font-size: 10px;
  color: #ffffff88;
}
</style>
