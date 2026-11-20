<script setup lang="ts">
import { onMounted, onUnmounted, ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import PressurePad from "../components/PressurePad.vue";
import { describe, everyFrame } from "../engine";
import { icons, type IconName } from "../icons";

const maskIcons: IconName[] = ["home", "shaders", "audio", "permissions"];
const tints = ["#8b93ff", "#f0b45a", "#5ad1a8", "#f07a9a"];
const iconMask = (name: IconName) =>
  `url("data:image/svg+xml,${encodeURIComponent(
    `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='${icons[name]}'/></svg>`,
  )}")`;

const W = 640;
const H = 220;
const dpr = devicePixelRatio;

const SHADER = /* wgsl */ `
struct Uniforms { time: f32, aspect: f32 }
@group(0) @binding(0) var<uniform> u: Uniforms;

struct Varying { @builtin(position) position: vec4f, @location(0) uv: vec2f }

@vertex fn vs(@builtin(vertex_index) i: u32) -> Varying {
  var corners = array<vec2f, 3>(vec2f(-1.0, -3.0), vec2f(-1.0, 1.0), vec2f(3.0, 1.0));
  var out: Varying;
  out.position = vec4f(corners[i], 0.0, 1.0);
  out.uv = corners[i] * 0.5 + 0.5;
  return out;
}

@fragment fn fs(v: Varying) -> @location(0) vec4f {
  let p = (v.uv - 0.5) * vec2f(u.aspect, 1.0) * 6.0;
  let t = u.time;
  let w = sin(p.x + t) + sin(p.y * 1.3 - t * 0.8) + sin(length(p) * 1.5 - t * 1.2);
  let c = 0.5 + 0.5 * cos(vec3f(0.0, 2.1, 4.2) + w + t * 0.3);
  return vec4f(c * vec3f(0.55, 0.6, 1.0), 1.0);
}`;

const surface = shallowRef<HTMLCanvasElement | null>(null);
const adapterName = ref("");
let stopGpu: (() => void) | null = null;

async function startGpu(canvas: HTMLCanvasElement) {
  const adapter = await navigator.gpu.requestAdapter();
  const context = canvas.getContext("webgpu");
  if (!adapter || !context) throw new Error("No GPU adapter for WebGPU on this machine");
  adapterName.value = adapter.info.description;
  const device = await adapter.requestDevice();
  const format = navigator.gpu.getPreferredCanvasFormat();
  context.configure({ device, format, alphaMode: "opaque" });
  const module = device.createShaderModule({ code: SHADER });
  const pipeline = device.createRenderPipeline({
    layout: "auto",
    vertex: { module, entryPoint: "vs" },
    fragment: { module, entryPoint: "fs", targets: [{ format }] },
  });
  const uniforms = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  const bindings = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [{ binding: 0, resource: { buffer: uniforms } }],
  });
  const stop = everyFrame((time) => {
    device.queue.writeBuffer(uniforms, 0, new Float32Array([time / 1000, W / H, 0, 0]));
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [{ view: context.getCurrentTexture().createView(), loadOp: "clear", storeOp: "store" }],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, bindings);
    pass.draw(3);
    pass.end();
    device.queue.submit([encoder.finish()]);
  });
  stopGpu = () => {
    stop();
    device.destroy();
  };
}

onMounted(() => {
  if (surface.value) startGpu(surface.value).catch((e: unknown) => (adapterName.value = describe(e)));
});
onUnmounted(() => stopGpu?.());

const stage = shallowRef<HTMLElement | null>(null);
const locked = ref(false);
const lockError = ref("");
const spin = ref({ x: -20, y: 30 });
const travelled = ref(0);

function lock() {
  lockError.value = "";
  stage.value?.requestPointerLock().catch((e: unknown) => (lockError.value = describe(e)));
}
function look(e: MouseEvent) {
  if (!locked.value) return;
  spin.value = {
    x: Math.max(-80, Math.min(80, spin.value.x - e.movementY * 0.4)),
    y: (spin.value.y + e.movementX * 0.4) % 360,
  };
  travelled.value += Math.hypot(e.movementX, e.movementY);
}
const onLockChange = () => (locked.value = document.pointerLockElement === stage.value);
onMounted(() => document.addEventListener("pointerlockchange", onLockChange));
onUnmounted(() => {
  document.removeEventListener("pointerlockchange", onLockChange);
  if (locked.value) document.exitPointerLock();
});
</script>

<template>
  <Page title="Graphics" lead="Canvas 2D and WebGPU draw into the same GPU-composited tree as the rest of the page.">
    <PressurePad />

    <Card title="WebGPU" :api="adapterName || 'navigator.gpu'">
      <canvas ref="surface" class="canvas" :width="W * dpr" :height="H * dpr" />
      <p class="faint">A WGSL fragment shader, redrawn every frame with the time in a uniform buffer.</p>
    </Card>

    <Card title="Masks" api="mask-image">
      <div class="row">
        <span
          v-for="(name, i) in maskIcons"
          :key="name"
          class="mask-icon"
          :style="{ maskImage: iconMask(name), color: tints[i] }"
        />
        <div class="fade">
          <p v-for="n in 4" :key="n">Line {{ n }} of text under a gradient mask that fades it out.</p>
        </div>
      </div>
      <p class="faint">SVG icons masking a currentColor background, and a gradient that fades the text out.</p>
    </Card>

    <Card title="Pointer lock" api="requestPointerLock()">
      <div ref="stage" class="stage" :class="{ locked }" @click="lock" @mousemove="look">
        <div class="cube" :style="{ transform: `rotate(${spin.y}deg) skewX(${spin.x / 4}deg)` }">
          <div class="face" />
        </div>
        <span class="hint">{{ locked ? "Move the mouse to turn the shape" : "Click to lock the pointer" }}</span>
      </div>
      <div class="row">
        <span class="badge" :class="{ on: locked }">{{ locked ? "Locked" : "Not locked" }}</span>
        <span class="faint">Esc releases the pointer</span>
        <span class="faint mono">{{ Math.round(travelled) }} px of movement</span>
      </div>
      <p v-if="lockError" class="faint">{{ lockError }}</p>
    </Card>
  </Page>
</template>

<style scoped>
.canvas {
  width: 100%;
  max-width: 640px;
  aspect-ratio: 32 / 11;
  border-radius: 10px;
}
.mask-icon {
  width: 32px;
  height: 32px;
  background: currentColor;
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
}
.fade {
  margin-left: 12px;
  font-size: 13px;
  line-height: 18px;
  mask-image: linear-gradient(to bottom, #000 40%, transparent);
}
.stage {
  position: relative;
  width: 640px;
  height: 180px;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: radial-gradient(circle at 50% 40%, #1f1b3a, #121218);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}
.stage.locked {
  border-color: var(--accent);
}
.cube {
  width: 72px;
  height: 72px;
}
.face {
  width: 100%;
  height: 100%;
  border-radius: 14px;
  background: linear-gradient(135deg, var(--accent), var(--accent-strong));
  box-shadow: 0 10px 30px #6d7cff55;
}
.hint {
  position: absolute;
  left: 14px;
  bottom: 10px;
  font-size: 12px;
  color: var(--text-faint);
}
.badge {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 12px;
  border: 1px solid var(--line-strong);
  color: var(--text-faint);
}
.badge.on {
  color: var(--text);
  border-color: var(--accent);
  background: var(--accent-soft);
}
</style>
