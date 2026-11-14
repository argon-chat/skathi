<script setup lang="ts">
import { onMounted, onUnmounted, ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe, everyFrame } from "../engine";

const W = 640;
const H = 220;
const dpr = devicePixelRatio;

const pad = shallowRef<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;
let last: { x: number; y: number } | null = null;
let hue = 220;

function clear() {
  if (!ctx) return;
  const bg = ctx.createLinearGradient(0, 0, W, H);
  bg.addColorStop(0, "#14141c");
  bg.addColorStop(1, "#1c1630");
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);
  ctx.fillStyle = "#ffffff40";
  ctx.font = "13px Segoe UI";
  ctx.fillText("Drag to draw", 16, 26);
}

const at = (e: MouseEvent) => {
  const rect = pad.value!.getBoundingClientRect();
  return { x: e.clientX - rect.left, y: e.clientY - rect.top };
};
const down = (e: MouseEvent) => (last = at(e));
const up = () => (last = null);
function move(e: MouseEvent) {
  if (!ctx || !last) return;
  const next = at(e);
  hue = (hue + 2) % 360;
  ctx.strokeStyle = `hsl(${hue}, 90%, 68%)`;
  ctx.lineWidth = 6;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(last.x, last.y);
  ctx.lineTo(next.x, next.y);
  ctx.stroke();
  last = next;
}

onMounted(() => {
  ctx = pad.value?.getContext("2d") ?? null;
  ctx?.scale(dpr, dpr);
  clear();
});

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
</script>

<template>
  <Page title="Graphics" lead="Canvas 2D and WebGPU draw into the same GPU-composited tree as the rest of the page.">
    <Card title="Canvas 2D" api="getContext('2d')">
      <canvas
        ref="pad"
        class="canvas"
        :width="W * dpr"
        :height="H * dpr"
        @mousedown="down"
        @mousemove="move"
        @mouseup="up"
        @mouseleave="up"
      />
      <div class="row">
        <button class="btn" @click="clear">Clear</button>
      </div>
    </Card>

    <Card title="WebGPU" :api="adapterName || 'navigator.gpu'">
      <canvas ref="surface" class="canvas" :width="W * dpr" :height="H * dpr" />
      <p class="faint">A WGSL fragment shader, redrawn every frame with the time in a uniform buffer.</p>
    </Card>
  </Page>
</template>

<style scoped>
.canvas {
  width: 640px;
  height: 220px;
  border-radius: 10px;
}
</style>
