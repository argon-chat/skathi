<script setup lang="ts">
import { onBeforeUnmount, onMounted, reactive, ref } from "vue";

const W = 1060;
const H = 420;
const PERIOD = 8;
const canvas = ref<HTMLCanvasElement | null>(null);
const hover = ref<number | null>(null);
const stats = reactive({ req: 0, p95: 0, err: 0 });

const TAU = Math.PI * 2;
/** Both series are periodic in u with period 1, and u moves by 1 every PERIOD seconds. */
const a = (u: number) =>
  0.56 + 0.16 * Math.sin(TAU * 2 * u) + 0.08 * Math.sin(TAU * 5 * u + 1) + 0.04 * Math.sin(TAU * 11 * u + 2);
const b = (u: number) =>
  0.3 + 0.08 * Math.sin(TAU * 3 * u + 2.2) + 0.05 * Math.sin(TAU * 7 * u + 0.4) + 0.025 * Math.sin(TAU * 13 * u);

const PAD = { l: 56, r: 24, t: 24, b: 40 };
const plotW = W - PAD.l - PAD.r;
const plotH = H - PAD.t - PAD.b;
const N = 120;
const X = (i: number) => PAD.l + (i / (N - 1)) * plotW;
const Y = (v: number) => PAD.t + (1 - v) * plotH;

function draw(now: number) {
  const ctx = canvas.value?.getContext("2d");
  if (!ctx) return;
  const shift = (now / 1000 / PERIOD) % 1;
  const ua = (i: number) => i / (N - 1) + shift;

  ctx.clearRect(0, 0, W, H);
  ctx.font = "12px 'Segoe UI', sans-serif";
  ctx.textBaseline = "middle";
  for (let g = 0; g <= 4; g++) {
    const y = PAD.t + (g / 4) * plotH;
    ctx.strokeStyle = "#302d3b";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(PAD.l, y + 0.5);
    ctx.lineTo(W - PAD.r, y + 0.5);
    ctx.stroke();
    ctx.fillStyle = "#7d7690";
    ctx.textAlign = "right";
    ctx.fillText(`${(1 - g / 4) * 2000}`, PAD.l - 12, y);
  }
  ctx.textAlign = "center";
  for (let s = 0; s <= 6; s++) {
    ctx.fillStyle = "#7d7690";
    ctx.fillText(s === 6 ? "now" : `-${60 - s * 10}s`, PAD.l + (s / 6) * plotW, H - PAD.b + 22);
  }

  const grad = ctx.createLinearGradient(0, PAD.t, 0, H - PAD.b);
  grad.addColorStop(0, "rgba(247, 168, 208, 0.42)");
  grad.addColorStop(1, "rgba(247, 168, 208, 0)");
  ctx.beginPath();
  ctx.moveTo(X(0), Y(a(ua(0))));
  for (let i = 1; i < N; i++) ctx.lineTo(X(i), Y(a(ua(i))));
  ctx.lineTo(X(N - 1), H - PAD.b);
  ctx.lineTo(X(0), H - PAD.b);
  ctx.closePath();
  ctx.fillStyle = grad;
  ctx.fill();

  const line = (f: (u: number) => number, color: string, width: number) => {
    ctx.beginPath();
    ctx.moveTo(X(0), Y(f(ua(0))));
    for (let i = 1; i < N; i++) ctx.lineTo(X(i), Y(f(ua(i))));
    ctx.strokeStyle = color;
    ctx.lineWidth = width;
    ctx.lineJoin = "round";
    ctx.stroke();
  };
  line(a, "#f7a8d0", 2.5);
  line(b, "#9fcdf5", 2);

  for (let i = 0; i < N; i += 4) {
    const v = 0.04 + 0.06 * (0.5 + 0.5 * Math.sin(TAU * 4 * ua(i) + 1.3));
    ctx.fillStyle = "rgba(201, 167, 245, 0.55)";
    ctx.fillRect(X(i) - 3, Y(v), 6, H - PAD.b - Y(v));
  }

  const last = ua(N - 1);
  const ends: [number, string][] = [
    [a(last), "#f7a8d0"],
    [b(last), "#9fcdf5"],
  ];
  for (const [v, c] of ends) {
    ctx.beginPath();
    ctx.arc(X(N - 1), Y(v), 5, 0, TAU);
    ctx.fillStyle = c;
    ctx.fill();
  }

  if (hover.value !== null) {
    const x = Math.min(W - PAD.r, Math.max(PAD.l, hover.value));
    const i = ((x - PAD.l) / plotW) * (N - 1);
    const u = i / (N - 1) + shift;
    ctx.strokeStyle = "#ffffff55";
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(x + 0.5, PAD.t);
    ctx.lineTo(x + 0.5, H - PAD.b);
    ctx.stroke();
    ctx.setLineDash([]);
    const pts: [number, string][] = [
      [a(u), "#f7a8d0"],
      [b(u), "#9fcdf5"],
    ];
    for (const [v, c] of pts) {
      ctx.beginPath();
      ctx.arc(x, Y(v), 6, 0, TAU);
      ctx.fillStyle = "#17161e";
      ctx.fill();
      ctx.lineWidth = 2.5;
      ctx.strokeStyle = c;
      ctx.stroke();
    }
    const bx = x > W - 230 ? x - 186 : x + 16;
    const by = Y(a(u)) - 70 < PAD.t ? PAD.t : Y(a(u)) - 70;
    ctx.fillStyle = "#17161ef0";
    ctx.strokeStyle = "#484151";
    ctx.lineWidth = 1;
    ctx.beginPath();
    if (typeof ctx.roundRect === "function") ctx.roundRect(bx, by, 170, 64, 10);
    else ctx.rect(bx, by, 170, 64);
    ctx.fill();
    ctx.stroke();
    ctx.textAlign = "left";
    ctx.font = "12px 'Segoe UI', sans-serif";
    ctx.fillStyle = "#f7a8d0";
    ctx.fillText(`requests  ${Math.round(a(u) * 2000)}/s`, bx + 14, by + 21);
    ctx.fillStyle = "#9fcdf5";
    ctx.fillText(`latency   ${Math.round(b(u) * 2000) / 10} ms`, bx + 14, by + 43);
  }

  stats.req = Math.round(a(last) * 2000);
  stats.p95 = Math.round(b(last) * 2000) / 10;
  stats.err = Math.round((0.4 + 0.3 * Math.sin(TAU * 2 * last + 0.7)) * 100) / 100;
}

let raf = 0;
const loop = (now: number) => {
  draw(now);
  raf = requestAnimationFrame(loop);
};
onMounted(() => (raf = requestAnimationFrame(loop)));
onBeforeUnmount(() => cancelAnimationFrame(raf));

function onMove(e: PointerEvent) {
  const r = canvas.value!.getBoundingClientRect();
  hover.value = ((e.clientX - r.left) / r.width) * W;
}
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;canvas&gt;</span>
      <p>A 2D chart redrawn every frame from <code>requestAnimationFrame</code>.</p>
    </div>
    <div class="stage">
      <div class="card panel">
        <div class="tiles">
          <div class="tile">
            <span class="k"><i class="sw pink" />Requests</span
            ><b>{{ stats.req.toLocaleString("en-US") }}<small>/s</small></b>
          </div>
          <div class="tile">
            <span class="k"><i class="sw blue" />Latency p95</span><b>{{ stats.p95.toFixed(1) }}<small> ms</small></b>
          </div>
          <div class="tile">
            <span class="k"><i class="sw lilac" />Errors</span><b>{{ stats.err.toFixed(2) }}<small> %</small></b>
          </div>
          <div class="live"><span class="pulse" />live</div>
        </div>
        <canvas
          ref="canvas"
          class="chart"
          data-demo="chart"
          :width="W"
          :height="H"
          @pointermove="onMove"
          @pointerleave="hover = null"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.head code {
  font-family: var(--mono);
  font-size: 13px;
  color: var(--lilac);
}
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.panel {
  padding: 22px 26px 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.tiles {
  display: flex;
  align-items: center;
  gap: 14px;
}
.tile {
  width: 200px;
  padding: 12px 16px;
  border-radius: 14px;
  background: var(--well);
  border: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.k {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--muted);
}
.sw {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
.sw.pink {
  background: var(--pink);
}
.sw.blue {
  background: var(--blue);
}
.sw.lilac {
  background: var(--lilac);
}
.tile b {
  font-family: var(--font-display);
  font-size: 26px;
  font-weight: 600;
  color: var(--text);
}
.tile small {
  font-size: 14px;
  font-weight: 400;
  color: var(--muted);
}
.live {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-radius: 999px;
  background: #2b3a33;
  color: #9fe0bd;
  font-size: 12px;
}
.pulse {
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background: #7fd6a4;
  animation: pulse 2s ease-in-out infinite;
}
@keyframes pulse {
  50% {
    opacity: 0.35;
  }
}
.chart {
  width: 1060px;
  height: 420px;
}
</style>
