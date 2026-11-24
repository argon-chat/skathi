import type { Details } from "../../details";

export const details: Details = {
  "canvas-2d": {
    examples: [
      {
        title: "Draw a sharp sparkline on a high-DPI screen",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";

const props = defineProps<{ values: number[] }>();
const W = 320;
const H = 80;
const dpr = devicePixelRatio;
const canvas = ref<HTMLCanvasElement | null>(null);

onMounted(() => {
  const ctx = canvas.value!.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.strokeStyle = "#4f8cff";
  ctx.lineWidth = 2;
  ctx.beginPath();
  props.values.forEach((v, i) => ctx.lineTo((i / (props.values.length - 1)) * W, H - v * H));
  ctx.stroke();
});
</script>

<template>
  <canvas ref="canvas" class="sparkline" :width="W * dpr" :height="H * dpr" />
</template>`,
        note: "The bitmap has one pixel per device pixel, and CSS sets the size it is shown at: `.sparkline { width: 320px; height: 80px; }`.",
      },
      {
        title: "Tint an icon with source-atop",
        lang: "ts",
        code: `async function tinted(src: string, color: string): Promise<HTMLCanvasElement> {
  const img = new Image();
  await new Promise((resolve, reject) => {
    img.onload = resolve;
    img.onerror = reject;
    img.src = src;
  });
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const ctx = canvas.getContext("2d")!;
  ctx.drawImage(img, 0, 0);
  ctx.globalCompositeOperation = "source-atop";
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  return canvas;
}`,
      },
    ],
    edgeCases: [
      "Without CSS `width` and `height`, a canvas lays out at its bitmap size in CSS pixels, 300×150 by default. With them, CSS scales the bitmap.",
      "Setting `width` or `height`, even to the same value, clears the bitmap and resets the context state.",
      "`drawImage` of an `<img>` that is still loading draws nothing; of a broken one, it throws `InvalidStateError`. Cross-origin images draw and stay readable: a canvas is never tainted, and `{ alpha: false }` still keeps an alpha channel.",
      "`filter` applies `blur(<px>)` only; other filter functions log an advisory and draw unfiltered. Color emoji in `fillText` draw as outlines.",
      '`toBlob` and `toDataURL` encode `image/webp` losslessly and ignore `quality`; an unknown type falls back to PNG. A canvas with zero width or height gives `"data:,"`.',
    ],
  },
  "offscreen-canvas": {
    examples: [
      {
        title: "Make a JPEG thumbnail off screen",
        lang: "ts",
        code: `async function thumbnail(bitmap: ImageBitmap, size = 256): Promise<Blob> {
  const scale = size / Math.max(bitmap.width, bitmap.height);
  const canvas = new OffscreenCanvas(Math.round(bitmap.width * scale), Math.round(bitmap.height * scale));
  const ctx = canvas.getContext("2d")!;
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  return canvas.convertToBlob({ type: "image/jpeg", quality: 0.85 });
}`,
      },
      {
        title: "Pre-render a sprite once and stamp it many times",
        lang: "ts",
        code: `const sprite = new OffscreenCanvas(32, 32);
const sctx = sprite.getContext("2d")!;
sctx.fillStyle = "#f5b301";
sctx.beginPath();
sctx.arc(16, 16, 14, 0, Math.PI * 2);
sctx.fill();
const dot = sprite.transferToImageBitmap();

for (const p of particles) ctx.drawImage(dot, p.x - 16, p.y - 16);`,
      },
    ],
    edgeCases: [
      "`transferToImageBitmap()` leaves the canvas blank. Without a context it throws `InvalidStateError`.",
      "`convertToBlob` on a canvas with zero width or height rejects with `IndexSizeError`.",
      '`getContext("webgl")`, `"webgl2"` and `"bitmaprenderer"` return `null`; an unknown ID throws a `TypeError`.',
      "A second `getContext` call with a different kind returns `null`.",
    ],
  },
  path2d: {
    examples: [
      {
        title: "Hit-test map markers on a canvas",
        lang: "ts",
        code: `const markers = cities.map((city) => {
  const path = new Path2D();
  path.arc(city.x, city.y, 8, 0, Math.PI * 2);
  return { city, path };
});

canvas.addEventListener("click", (e) => {
  const box = canvas.getBoundingClientRect();
  const x = (e.clientX - box.left) * devicePixelRatio;
  const y = (e.clientY - box.top) * devicePixelRatio;
  const hit = markers.find((m) => ctx.isPointInPath(m.path, x, y));
  if (hit) openCity(hit.city);
});`,
        note: "`isPointInPath` takes bitmap pixels and applies the current transform to the path, so on a canvas scaled by `devicePixelRatio` the pointer offsets are scaled too.",
      },
      {
        title: "Draw an icon from SVG path data",
        lang: "ts",
        code: `const check = new Path2D("M4 12l5 5L20 6");
ctx.lineWidth = 2;
ctx.lineCap = "round";
ctx.stroke(check);`,
      },
    ],
    edgeCases: [
      "SVG path data stops at the first error and keeps everything before it, as SVG does.",
      "A negative radius in `arc`, `ellipse` or `roundRect` throws `IndexSizeError`.",
    ],
  },
  "image-data": {
    examples: [
      {
        title: "Turn a photo grayscale",
        lang: "ts",
        code: `const frame = ctx.getImageData(0, 0, canvas.width, canvas.height);
const px = frame.data;
for (let i = 0; i < px.length; i += 4) {
  const y = 0.2126 * px[i] + 0.7152 * px[i + 1] + 0.0722 * px[i + 2];
  px[i] = px[i + 1] = px[i + 2] = y;
}
ctx.putImageData(frame, 0, 0);`,
      },
      {
        title: "Build a gradient texture from raw pixels",
        lang: "ts",
        code: `const width = 256;
const pixels = new Uint8ClampedArray(width * 4);
for (let x = 0; x < width; x++) pixels.set([x, 64, 255 - x, 255], x * 4);
ctx.putImageData(new ImageData(pixels, width), 0, 0);`,
      },
    ],
    edgeCases: [
      "`new ImageData(0, h)` throws a `TypeError`, where browsers throw `IndexSizeError`.",
      "`new ImageData(pixels, width)` wraps `pixels` without copying and derives `height` from its length. A length that is not a multiple of `4 × width` throws a `TypeError`.",
      "The data must be a `Uint8ClampedArray`; another typed array throws a `TypeError`.",
    ],
  },
  "image-bitmap": {
    examples: [
      {
        title: "Decode a sprite once and draw it every frame",
        lang: "ts",
        code: `const res = await fetch("assets/sprites/ship.png");
const ship = await createImageBitmap(await res.blob());

function frame(t: number) {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(ship, 100 + Math.sin(t / 500) * 50, 80);
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);`,
      },
    ],
    edgeCases: [
      "`close()` frees the pixels: `width` and `height` become 0, and drawing the bitmap throws `InvalidStateError`.",
      "EXIF orientation is applied when the image is decoded.",
    ],
  },
  "create-image-bitmap": {
    examples: [
      {
        title: "Cut a sprite sheet into frames",
        lang: "ts",
        code: `const sheet = await createImageBitmap(await (await fetch("assets/walk.png")).blob());
const frames = await Promise.all(
  Array.from({ length: 8 }, (_, i) => createImageBitmap(sheet, i * 64, 0, 64, 64)),
);
sheet.close();`,
      },
      {
        title: "Decode a picked photo",
        lang: "ts",
        code: `async function loadPhoto(file: File) {
  const bitmap = await createImageBitmap(file);
  photo.value = { bitmap, width: bitmap.width, height: bitmap.height };
}`,
        note: "A picked file is read off the UI thread, then decoded on it.",
      },
    ],
    edgeCases: [
      "A crop rectangle with zero or negative width or height rejects with a `RangeError`. Parts of the rectangle outside the source are transparent.",
      "A revoked `blob:` URL or bytes that do not decode reject with a `TypeError`.",
      'A path string such as `"assets/logo.png"` is read from the app\'s files.',
    ],
  },
  "dom-matrix": {
    examples: [
      {
        title: "Pan and zoom a canvas with one matrix",
        lang: "ts",
        code: `let view = new DOMMatrix();

function zoomAt(x: number, y: number, factor: number) {
  view = new DOMMatrix().translate(x, y).scale(factor).translate(-x, -y).multiply(view);
}

function draw() {
  ctx.setTransform(view);
  drawScene(ctx);
}

function toScene(x: number, y: number): DOMPoint {
  return view.inverse().transformPoint(new DOMPoint(x, y));
}`,
      },
    ],
    edgeCases: [
      "`inverse()` of a matrix that cannot be inverted has every value `NaN` and `is2D` `false`; its `toString()` throws `InvalidStateError`.",
      "`is2D` stays `false` once a 3D operation is applied, even if a later one undoes it.",
      "`ctx.setTransform(matrix)` ignores a matrix with a non-finite value.",
      "`fromFloat32Array` and `fromFloat64Array` take 6 or 16 elements; other lengths throw a `TypeError`.",
    ],
  },
  "dom-point": {
    examples: [
      {
        title: "Rotate a point around a pivot",
        lang: "ts",
        code: `const pivot = new DOMPoint(200, 150);
const turn = new DOMMatrix().translate(pivot.x, pivot.y).rotate(30).translate(-pivot.x, -pivot.y);
const tip = new DOMPoint(260, 150).matrixTransform(turn);
ctx.lineTo(tip.x, tip.y);`,
      },
    ],
  },
  webgpu: {
    examples: [
      {
        title: "Render a shader into a canvas every frame",
        lang: "ts",
        code: `const adapter = await navigator.gpu.requestAdapter();
if (!adapter) throw new Error("No WebGPU adapter on this machine");
const device = await adapter.requestDevice();
const context = canvas.getContext("webgpu")!;
const format = navigator.gpu.getPreferredCanvasFormat();
context.configure({ device, format, alphaMode: "opaque" });
const module = device.createShaderModule({ code: wgsl });
const pipeline = device.createRenderPipeline({
  layout: "auto",
  vertex: { module, entryPoint: "vs" },
  fragment: { module, entryPoint: "fs", targets: [{ format }] },
});

function frame() {
  const encoder = device.createCommandEncoder();
  const view = context.getCurrentTexture().createView();
  const pass = encoder.beginRenderPass({ colorAttachments: [{ view, loadOp: "clear", storeOp: "store" }] });
  pass.setPipeline(pipeline);
  pass.draw(3);
  pass.end();
  device.queue.submit([encoder.finish()]);
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);`,
        note: "`wgsl` holds a vertex entry point `vs` that emits a full-screen triangle and a fragment entry point `fs`.",
      },
      {
        title: "Halve a buffer of samples on the GPU",
        lang: "ts",
        code: `const usage = GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST;
const data = device.createBuffer({ size: samples.byteLength, usage });
const readback = device.createBuffer({ size: samples.byteLength, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
device.queue.writeBuffer(data, 0, samples);
const module = device.createShaderModule({
  code:
    "@group(0) @binding(0) var<storage, read_write> v: array<f32>;" +
    "@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3u) {" +
    " if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 0.5; } }",
});
const pipeline = device.createComputePipeline({ layout: "auto", compute: { module, entryPoint: "main" } });
const entries = [{ binding: 0, resource: { buffer: data } }];
const group = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries });
const encoder = device.createCommandEncoder();
const pass = encoder.beginComputePass();
pass.setPipeline(pipeline);
pass.setBindGroup(0, group);
pass.dispatchWorkgroups(Math.ceil(samples.length / 64));
pass.end();
encoder.copyBufferToBuffer(data, 0, readback, 0, samples.byteLength);
device.queue.submit([encoder.finish()]);
await readback.mapAsync(GPUMapMode.READ);
const halved = new Float32Array(readback.getMappedRange().slice(0));
readback.unmap();`,
        note: "`device` comes from `adapter.requestDevice()`, and `samples` is a `Float32Array`.",
      },
    ],
    edgeCases: [
      '`getPreferredCanvasFormat()` returns `"bgra8unorm"`. `configure` takes `rgba8unorm` and `bgra8unorm` only, and throws `NotSupportedError` for another format, a `colorSpace` other than `"srgb"`, or extended tone mapping.',
      "`getCurrentTexture()` before `configure` throws `InvalidStateError`, and on a canvas with zero width or height it throws `OperationError`.",
      "WGSL is always validated. Errors reach `pushErrorScope` and `popErrorScope` as `GPUValidationError`, or the device's `uncapturederror` event.",
      '`getContext("webgpu")` on a canvas that already has a 2D context returns `null`.',
    ],
  },
  webgl: {
    examples: [
      {
        title: "Instead: set up a WebGPU canvas",
        lang: "ts",
        code: `const context = canvas.getContext("webgpu");
const adapter = await navigator.gpu.requestAdapter();
if (!context || !adapter) throw new Error("WebGPU is unavailable on this machine");
const device = await adapter.requestDevice();
context.configure({ device, format: navigator.gpu.getPreferredCanvasFormat() });`,
      },
    ],
    edgeCases: ['The first `getContext("webgl")` logs an advisory; later calls return `null` silently.'],
  },
  "webgl-2": {
    examples: [
      {
        title: "Instead: set up a WebGPU canvas",
        lang: "ts",
        code: `const context = canvas.getContext("webgpu");
const adapter = await navigator.gpu.requestAdapter();
if (!context || !adapter) throw new Error("WebGPU is unavailable on this machine");
const device = await adapter.requestDevice();
context.configure({ device, format: navigator.gpu.getPreferredCanvasFormat() });`,
      },
    ],
    edgeCases: ['`getContext("webgl2")` returns `null`; the first call logs an advisory.'],
  },
  "web-audio": {
    examples: [
      {
        title: "Play a notification chime",
        lang: "ts",
        code: `const audio = new AudioContext();
const chime = await audio.decodeAudioData(await (await fetch("assets/sounds/chime.wav")).arrayBuffer());

function playChime(volume = 0.6) {
  const source = audio.createBufferSource();
  const gain = audio.createGain();
  gain.gain.value = volume;
  source.buffer = chime;
  source.connect(gain);
  gain.connect(audio.destination);
  source.start();
}`,
      },
      {
        title: "Show a microphone level meter",
        lang: "ts",
        code: `const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
const audio = new AudioContext();
const analyser = audio.createAnalyser();
analyser.fftSize = 256;
audio.createMediaStreamSource(stream).connect(analyser);
const samples = new Float32Array(analyser.fftSize);

function meter() {
  analyser.getFloatTimeDomainData(samples);
  const rms = Math.sqrt(samples.reduce((sum, v) => sum + v * v, 0) / samples.length);
  level.value = Math.min(1, rms * 4);
  requestAnimationFrame(meter);
}
meter();`,
      },
      {
        title: "Fade the music out over two seconds",
        lang: "ts",
        code: `function fadeOut(music: GainNode, seconds = 2) {
  const now = music.context.currentTime;
  music.gain.setValueAtTime(music.gain.value, now);
  music.gain.linearRampToValueAtTime(0, now + seconds);
}`,
      },
      {
        title: "Play a 5.1 track on a surround system",
        lang: "ts",
        code: `const audio = new AudioContext();
const track = await audio.decodeAudioData(await (await fetch("assets/sounds/intro-5.1.flac")).arrayBuffer());
// 6 on a 5.1 device; a stereo device keeps 2 and the track is mixed down.
audio.destination.channelCount = Math.min(track.numberOfChannels, audio.destination.maxChannelCount);
const source = audio.createBufferSource();
source.buffer = track;
source.connect(audio.destination);
source.start();`,
      },
    ],
    edgeCases: [
      "`destination.channelCount` above `maxChannelCount` throws `IndexSizeError`; `channelCount` 0 or above 32 on any node throws `NotSupportedError`.",
      "`start`, `stop` and parameter automation take effect on the first frame at or after the requested time, not on the exact sample.",
      "Connecting nodes of two different `AudioContext`s throws `InvalidAccessError`.",
      "`createMediaElementSource(element)` routes the element's audio to the graph. Connect the node to `context.destination` for playback. Creating a second source for the same element throws `InvalidStateError`.",
      "`decodeAudioData` rejects with `EncodingError` for empty or undecodable data.",
    ],
  },
  "spatial-audio": {
    examples: [
      {
        title: "Place a sound around the listener",
        lang: "ts",
        code: `const audio = new AudioContext();
const panner = audio.createPanner();
panner.panningModel = "spatial"; // HRTF wherever no spatial renderer takes it
panner.positionX.value = 2;
panner.positionZ.value = -1;
source.connect(panner).connect(audio.destination);

const status = audio.spatialAudio;
if (status?.active) console.log(\`\${status.format}: \${status.objectsInUse}/\${status.maxObjects} objects\`);
else if (status?.requested) console.log("spatial audio is off:", status.reason);`,
        note: "Write `'spatial'` only where Skathi runs, or guard it: browsers ignore the value and keep the previous model.",
      },
    ],
    edgeCases: [
      "Turning the spatial sound format off or switching the output device while a context plays reopens its output: the context carries on, its panners render with HRTF or on the new renderer, and `spatialAudio` follows.",
      "`spatialAudio` is read fresh each time; it is a plain object, not live.",
      "Setting `panningModel` back to `'HRTF'` or `'equalpower'` frees the panner's object for the next panner waiting for one.",
    ],
  },
  "get-user-media": {
    examples: [
      {
        title: "Open the microphone the user picked",
        lang: "ts",
        code: `async function openMic(deviceId?: string): Promise<MediaStream> {
  try {
    return await navigator.mediaDevices.getUserMedia({
      audio: deviceId ? { deviceId: { exact: deviceId } } : true,
    });
  } catch (e) {
    if ((e as DOMException).name === "NotAllowedError") showMicHelp();
    throw e;
  }
}`,
        note: "With `exact`, a missing device rejects instead of falling back to the default one.",
      },
    ],
    edgeCases: [
      'Dismissing the prompt rejects with `NotAllowedError` ("Permission dismissed") and asks again next time. Block rejects with "Permission denied" and stops asking until `appPermissions.request("microphone")`. The answer is kept across restarts.',
      "`video` rejects with `NotSupportedError`. A call with neither `audio` nor `video` rejects with a `TypeError`.",
      "`echoCancellation`, `noiseSuppression` and `autoGainControl` run only when the host installed capture processing; asking for them otherwise logs an advisory.",
    ],
  },
  "media-devices": {
    examples: [
      {
        title: "Build microphone and speaker pickers",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const inputs = ref<MediaDeviceInfo[]>([]);
const outputs = ref<MediaDeviceInfo[]>([]);

async function refresh() {
  const devices = await navigator.mediaDevices.enumerateDevices();
  inputs.value = devices.filter((d) => d.kind === "audioinput");
  outputs.value = devices.filter((d) => d.kind === "audiooutput");
}
onMounted(() => {
  refresh();
  navigator.mediaDevices.addEventListener("devicechange", refresh);
});
onUnmounted(() => navigator.mediaDevices.removeEventListener("devicechange", refresh));
</script>

<template>
  <select><option v-for="d in inputs" :key="d.deviceId" :value="d.deviceId">{{ d.label || "Microphone" }}</option></select>
  <select><option v-for="d in outputs" :key="d.deviceId" :value="d.deviceId">{{ d.label }}</option></select>
</template>`,
      },
      {
        title: "Play through the speaker the user chose",
        lang: "ts",
        code: `async function useSpeaker(audio: AudioContext, deviceId: string) {
  await audio.setSinkId(deviceId);
  localStorage.setItem("speaker", deviceId);
}`,
      },
    ],
    edgeCases: [
      'Each kind starts with a `deviceId: "default"` entry labelled `Default - <name>`.',
      "Until the microphone is granted, inputs show as one `audioinput` with an empty `deviceId` and `label`. Outputs are always named.",
      '`videoinput` never appears, and `groupId` is `""`.',
      "Nothing checks for `devicechange` until you add a listener.",
    ],
  },
  "media-stream": {
    examples: [
      {
        title: "Stop the microphone when a call ends",
        lang: "ts",
        code: `function hangUp(stream: MediaStream) {
  for (const track of stream.getTracks()) {
    track.stop();
  }
}`,
      },
      {
        title: "Tell the user when the microphone is unplugged",
        lang: "ts",
        code: `const [track] = stream.getAudioTracks();
console.info("Using", track.label);
track.addEventListener("ended", () => {
  showBanner("Microphone disconnected");
});`,
      },
    ],
    edgeCases: [
      "A track fires `ended` when its device goes away.",
      "`applyConstraints()` records the constraints and changes nothing.",
      "`getSettings()` reports `echoCancellation`, `noiseSuppression` and `autoGainControl` as `false` unless the host installed capture processing.",
      "Setting a `<video>`'s `srcObject` to an audio-only stream throws a `TypeError`; an `<audio>` holds it and plays nothing.",
    ],
  },
  video: {
    examples: [
      {
        title: "Build custom playback controls",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const video = ref<HTMLVideoElement | null>(null);
const playing = ref(false);
const progress = ref(0);

function toggle() {
  const el = video.value!;
  if (el.paused) el.play();
  else el.pause();
}
function onTime() {
  const el = video.value!;
  progress.value = el.duration ? el.currentTime / el.duration : 0;
}
</script>

<template>
  <video ref="video" src="assets/intro.mp4" poster="assets/intro.png" muted
    @playing="playing = true" @pause="playing = false" @timeupdate="onTime" />
  <button @click="toggle">{{ playing ? "Pause" : "Play" }}</button>
  <div class="bar"><div class="fill" :style="{ width: progress * 100 + '%' }" /></div>
</template>`,
      },
      {
        title: "Fall back when this machine cannot decode a format",
        lang: "ts",
        code: `const player = document.querySelector("video")!;
if (player.canPlayType('video/mp4; codecs="hvc1"') === "") {
  player.src = "assets/intro-h264.mp4";
}
player.addEventListener("error", () => console.warn(player.error?.code, player.error?.message));`,
      },
    ],
    edgeCases: [
      "`play()` resolves at once and never rejects. Wait for `playing` to know playback started.",
      "`load()` does nothing and logs an advisory. To reload, clear `src` and set it again.",
      "`canPlayType` asks the system's decoders, so the answer can differ between machines.",
      "`error` is a plain `{ code, message }` object whose message names the decoder this machine is missing.",
    ],
  },
  "sk-surface": {
    examples: [
      {
        title: "Show a native player under Vue controls",
        lang: "vue",
        code: `<script setup lang="ts">
import { resize, toggle } from "native:player";

function resized(e: SkSurfaceEvent) {
  resize(e.width, e.height);
}
</script>

<template>
  <div class="player">
    <sk-surface name="player" class="video" @surfaceresize="resized" />
    <div class="controls"><button @click="toggle()">Play</button></div>
  </div>
</template>`,
        note: 'The embedder renders into `host.surface("player")` and gives the app `resize` and `toggle` in its `"player"` [native module](/embedding/native-modules/); the controls are ordinary elements over it.',
      },
    ],
    edgeCases: [
      "Several elements with one `name` show the same frame. The size the producer sees is the largest of them.",
      "A frame presented while the element is hidden, scrolled out of its clip or `display: none` is not uploaded. The newest one shows when it becomes visible.",
      'After a GPU device loss the frame is gone and `surfacelost` fires with `reason: "device"`; the producer recreates its textures on the new device.',
    ],
  },
  "navigator-media-session": {
    examples: [
      {
        title: "Show the current track in the system media controls",
        lang: "ts",
        code: `const session = navigator.mediaSession;
session.metadata = new MediaMetadata({
  title: track.title,
  artist: track.artist,
  album: track.album,
  artwork: [{ src: track.coverUrl, sizes: "512x512", type: "image/png" }],
});
session.playbackState = "playing";
session.setActionHandler("play", () => player.play());
session.setActionHandler("pause", () => player.pause());
session.setActionHandler("nexttrack", () => player.next());
session.setActionHandler("seekto", (d) => player.seek(d.seekTime!));
session.setPositionState({ duration: track.duration, position: player.currentTime });`,
      },
    ],
    edgeCases: [
      "Only actions with a handler show as buttons. `setActionHandler(action, null)` removes one.",
      "Changes made in one task reach the system as one update.",
      "An unknown action, a position past the duration, or a plain object assigned to `metadata` throws a `TypeError`. An unknown `playbackState` is ignored.",
      "`setPositionState()` with no fields clears the position.",
      "The system shows the largest artwork by `sizes`. On Windows the system loads an http(s) artwork URL itself; on macOS the app fetches the image and sends the bytes.",
    ],
  },
  "media-metadata": {
    examples: [
      {
        title: "Describe a podcast episode",
        lang: "ts",
        code: `function showEpisode(episode: Episode) {
  navigator.mediaSession.metadata = new MediaMetadata({
    title: episode.title,
    artist: episode.show,
    artwork: [
      { src: episode.smallArt, sizes: "96x96" },
      { src: episode.largeArt, sizes: "512x512", type: "image/png" },
    ],
  });
}`,
      },
    ],
    edgeCases: [
      'A field you leave out reads as `""`.',
      "`artwork` returns the same frozen array until you assign a new one.",
      "An artwork entry without `src` throws a `TypeError`.",
    ],
  },
  "media-recorder": {
    edgeCases: ["`MediaRecorder` is `undefined`."],
  },
  webcodecs: {
    edgeCases: ["`VideoEncoder`, `VideoDecoder`, `AudioEncoder` and `AudioDecoder` are `undefined`."],
  },
  webrtc: {
    edgeCases: ["`RTCPeerConnection` is `undefined`."],
  },
};
