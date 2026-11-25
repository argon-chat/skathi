// `sk.noise` golden table: a float64 reference written from the published algorithms (PCG, Perlin's improved
// noise, simplex noise, Worley, Jimenez's IGN, Roberts' R2, Bayer's recursion), the cases the shaders must
// reproduce, and their effect. Hashes are compared bit for bit; derivatives against finite differences of the
// reference value, not against a second copy of the analytic formula.

import { type Case, live, num, tableEffect, vec, withRunTime } from "./shader-table";

const u32 = (x: number) => x >>> 0;
const mul = (a: number, b: number) => Math.imul(a, b) >>> 0;

export function pcg(v: number): number {
  const state = u32(mul(v, 747796405) + 2891336453);
  const word = mul(((state >>> ((state >>> 28) + 4)) ^ state) >>> 0, 277803737);
  return u32((word >>> 22) ^ word);
}
export const pcg2 = (x: number, y: number) => pcg(u32(x + pcg(y)));
export const pcg3 = (x: number, y: number, z: number) => pcg(u32(x + pcg(u32(y + pcg(z)))));

export function pcg3d(a: number, b: number, c: number): [number, number, number] {
  let x = u32(mul(a, 1664525) + 1013904223);
  let y = u32(mul(b, 1664525) + 1013904223);
  let z = u32(mul(c, 1664525) + 1013904223);
  x = u32(x + mul(y, z));
  y = u32(y + mul(z, x));
  z = u32(z + mul(x, y));
  x = u32(x ^ (x >>> 16));
  y = u32(y ^ (y >>> 16));
  z = u32(z ^ (z >>> 16));
  x = u32(x + mul(y, z));
  y = u32(y + mul(z, x));
  z = u32(z + mul(x, y));
  return [x, y, z];
}

export const unit = (h: number) => (h >>> 8) / 16777216;
const cell = (p: number) => u32(Math.floor(p) | 0);

type V2 = [number, number];
type V3 = [number, number, number];

const SQ = Math.SQRT1_2;
/** The eight directions 45 degrees apart, as Perlin's table lists them. */
const DIRS2: V2[] = [
  [1, 0],
  [SQ, SQ],
  [0, 1],
  [-SQ, SQ],
  [-1, 0],
  [-SQ, -SQ],
  [0, -1],
  [SQ, -SQ],
];
const grad2 = (h: number): V2 => DIRS2[h >>> 29]!;
/** The twelve cube edges: which axis is zero, then the two signs. */
function grad3(h: number): V3 {
  const k = (h >>> 4) % 12;
  const a = k & 1 ? -SQ : SQ;
  const b = k & 2 ? -SQ : SQ;
  const axis = k >> 2;
  return axis === 0 ? [a, b, 0] : axis === 1 ? [a, 0, b] : [0, a, b];
}
const fade = (t: number) => t * t * t * (t * (t * 6 - 15) + 10);
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

/** Perlin noise as the lerp of corner ramps, the way Perlin's reference writes it. */
export function perlin2(p: V2, period?: V2): number {
  const [ix, iy] = [Math.floor(p[0]), Math.floor(p[1])];
  const [fx, fy] = [p[0] - ix, p[1] - iy];
  const wrap = (c: number, k: 0 | 1) => {
    if (!period) return u32(c);
    const n = Math.max(Math.round(period[k]), 1);
    return u32(c - n * Math.floor(c / n));
  };
  const ramp = (dx: number, dy: number) => {
    const g = grad2(pcg2(wrap(ix + dx, 0), wrap(iy + dy, 1)));
    return g[0] * (fx - dx) + g[1] * (fy - dy);
  };
  const u = fade(fx);
  const v = fade(fy);
  return lerp(lerp(ramp(0, 0), ramp(1, 0), u), lerp(ramp(0, 1), ramp(1, 1), u), v) * Math.SQRT2;
}

export function value2(p: V2, period?: V2): number {
  const [ix, iy] = [Math.floor(p[0]), Math.floor(p[1])];
  const wrap = (c: number, k: 0 | 1) => {
    if (!period) return u32(c);
    const n = Math.max(Math.round(period[k]), 1);
    return u32(c - n * Math.floor(c / n));
  };
  const at = (dx: number, dy: number) => unit(pcg2(wrap(ix + dx, 0), wrap(iy + dy, 1))) * 2 - 1;
  const u = fade(p[0] - ix);
  const v = fade(p[1] - iy);
  return lerp(lerp(at(0, 0), at(1, 0), u), lerp(at(0, 1), at(1, 1), u), v);
}

function lattice3(p: V3, corner: (c: V3, f: V3) => number): number {
  const i = p.map(Math.floor) as V3;
  const f = p.map((v, k) => v - i[k]!) as V3;
  const c = (dx: number, dy: number, dz: number) =>
    corner([i[0] + dx, i[1] + dy, i[2] + dz], [f[0] - dx, f[1] - dy, f[2] - dz]);
  const [u, v, w] = f.map(fade) as V3;
  const z0 = lerp(lerp(c(0, 0, 0), c(1, 0, 0), u), lerp(c(0, 1, 0), c(1, 1, 0), u), v);
  const z1 = lerp(lerp(c(0, 0, 1), c(1, 0, 1), u), lerp(c(0, 1, 1), c(1, 1, 1), u), v);
  return lerp(z0, z1, w);
}

export const perlin3 = (p: V3) =>
  lattice3(p, (c, f) => {
    const g = grad3(pcg3(u32(c[0]), u32(c[1]), u32(c[2])));
    return g[0] * f[0] + g[1] * f[1] + g[2] * f[2];
  }) *
  (2 / Math.sqrt(3));

export const value3 = (p: V3) => lattice3(p, (c) => unit(pcg3(u32(c[0]), u32(c[1]), u32(c[2]))) * 2 - 1);

const F2 = (Math.sqrt(3) - 1) / 2;
const G2 = (3 - Math.sqrt(3)) / 6;

/** 2D simplex noise; with `angle`, flow noise: gradients turned by it, the sign from hash bit 8. */
export function simplex2(p: V2, angle = 0): number {
  const s = (p[0] + p[1]) * F2;
  const i: V2 = [Math.floor(p[0] + s), Math.floor(p[1] + s)];
  const t = (i[0] + i[1]) * G2;
  const x0: V2 = [p[0] - i[0] + t, p[1] - i[1] + t];
  const o: V2 = x0[0] > x0[1] ? [1, 0] : [0, 1];
  let sum = 0;
  for (const [dx, dy] of [[0, 0], o, [1, 1]] as V2[]) {
    const x: V2 = [x0[0] - dx + (dx + dy) * G2, x0[1] - dy + (dx + dy) * G2];
    const h = pcg2(u32(i[0] + dx), u32(i[1] + dy));
    const [gx, gy] = grad2(h);
    const a = h & 0x100 ? -angle : angle;
    const g: V2 = [gx * Math.cos(a) - gy * Math.sin(a), gx * Math.sin(a) + gy * Math.cos(a)];
    const k = Math.max(0.5 - x[0] * x[0] - x[1] * x[1], 0);
    sum += k ** 4 * (g[0] * x[0] + g[1] * x[1]);
  }
  return sum * 99;
}

export function simplex3(p: V3): number {
  const s = (p[0] + p[1] + p[2]) / 3;
  const i = p.map((v) => Math.floor(v + s)) as V3;
  const t = (i[0] + i[1] + i[2]) / 6;
  const x0 = p.map((v, k) => v - i[k]! + t) as V3;
  // The simplex holding x0: walk the axes from the largest coordinate to the smallest.
  const order = [0, 1, 2].sort((a, b) => x0[b]! - x0[a]! || a - b);
  const corners: V3[] = [[0, 0, 0]];
  const c: V3 = [0, 0, 0];
  for (const axis of order) {
    c[axis] = 1;
    corners.push([...c]);
  }
  let sum = 0;
  corners.forEach((o, n) => {
    const x = x0.map((v, k) => v - o[k]! + n / 6) as V3;
    const g = grad3(pcg3(u32(i[0] + o[0]), u32(i[1] + o[1]), u32(i[2] + o[2])));
    const k = Math.max(0.5 - x[0] ** 2 - x[1] ** 2 - x[2] ** 2, 0);
    sum += k ** 4 * (g[0] * x[0] + g[1] * x[1] + g[2] * x[2]);
  });
  return sum * 85;
}

export interface CellRef {
  f1: number;
  f2: number;
  cell: V2;
  id: number;
}

/** Worley noise by brute force over the 5x5 cells around p, sorted; the shader searches 3x3. */
export function worley2(p: V2, jitter = 1, period?: V2): CellRef {
  const wrap = (c: number, k: 0 | 1) => {
    if (!period) return u32(c);
    const n = Math.max(Math.round(period[k]), 1);
    return u32(c - n * Math.floor(c / n));
  };
  const i: V2 = [Math.floor(p[0]), Math.floor(p[1])];
  const pts: Array<{ d: number; cell: V2 }> = [];
  for (let y = -2; y <= 2; y++)
    for (let x = -2; x <= 2; x++) {
      const cell: V2 = [i[0] + x, i[1] + y];
      const [hx, hy] = pcg3d(wrap(cell[0], 0), wrap(cell[1], 1), 0);
      const fp: V2 = [cell[0] + 0.5 + (unit(hx) - 0.5) * jitter, cell[1] + 0.5 + (unit(hy) - 0.5) * jitter];
      pts.push({ d: Math.hypot(fp[0] - p[0], fp[1] - p[1]), cell });
    }
  pts.sort((a, b) => a.d - b.d);
  const near = pts[0]!;
  return { f1: near.d, f2: pts[1]!.d, cell: near.cell, id: pcg2(wrap(near.cell[0], 0), wrap(near.cell[1], 1)) };
}

/** Distance to the nearest Voronoi edge: the nearest bisector of the nearest point and any other within 5x5. */
export function worleyEdge(p: V2, jitter = 1): number {
  const i: V2 = [Math.floor(p[0]), Math.floor(p[1])];
  const pts: V2[] = [];
  for (let y = -3; y <= 3; y++)
    for (let x = -3; x <= 3; x++) {
      const [hx, hy] = pcg3d(u32(i[0] + x), u32(i[1] + y), 0);
      pts.push([i[0] + x + 0.5 + (unit(hx) - 0.5) * jitter, i[1] + y + 0.5 + (unit(hy) - 0.5) * jitter]);
    }
  const dist = (q: V2) => Math.hypot(q[0] - p[0], q[1] - p[1]);
  const a = pts.reduce((m, q) => (dist(q) < dist(m) ? q : m));
  let best = Infinity;
  for (const b of pts) {
    if (b === a) continue;
    const n: V2 = [b[0] - a[0], b[1] - a[1]];
    const len = Math.hypot(n[0], n[1]);
    const mid: V2 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
    best = Math.min(best, ((mid[0] - p[0]) * n[0] + (mid[1] - p[1]) * n[1]) / len);
  }
  return best;
}

export function worley3(p: V3, jitter = 1): { f1: number; f2: number; id: number } {
  const i = p.map(Math.floor) as V3;
  const ds: Array<{ d: number; c: V3 }> = [];
  for (let z = -2; z <= 2; z++)
    for (let y = -2; y <= 2; y++)
      for (let x = -2; x <= 2; x++) {
        const c: V3 = [i[0] + x, i[1] + y, i[2] + z];
        const h = pcg3d(u32(c[0]), u32(c[1]), u32(c[2]));
        const fp = c.map((v, k) => v + 0.5 + (unit(h[k]!) - 0.5) * jitter);
        ds.push({ d: Math.hypot(fp[0]! - p[0], fp[1]! - p[1], fp[2]! - p[2]), c });
      }
  ds.sort((a, b) => a.d - b.d);
  const c = ds[0]!.c;
  return { f1: ds[0]!.d, f2: ds[1]!.d, id: pcg3(u32(c[0]), u32(c[1]), u32(c[2])) };
}

type Noise2 = (p: V2) => number;

const next = (q: V2, lac: number): V2 => [
  (0.8 * q[0] - 0.6 * q[1]) * lac + 19.1,
  (0.6 * q[0] + 0.8 * q[1]) * lac - 7.3,
];

function fractal(n: Noise2, p: V2, octaves: number, lac: number, gain: number, shape: (v: number) => number) {
  let sum = 0;
  let norm = 0;
  let amp = 1;
  let q = p;
  for (let i = 0; i < 8; i++) {
    const w = Math.min(Math.max(octaves - i, 0), 1) * amp;
    sum += w * shape(n(q));
    norm += w;
    amp *= gain;
    q = next(q, lac);
  }
  return sum / Math.max(norm, 1e-6);
}

export const fbm = (n: Noise2, p: V2, octaves = 5, lac = 2, gain = 0.5) => fractal(n, p, octaves, lac, gain, (v) => v);
export const turbulence = (n: Noise2, p: V2, octaves = 5, lac = 2, gain = 0.5) =>
  fractal(n, p, octaves, lac, gain, Math.abs);
export const ridged = (n: Noise2, p: V2, octaves = 5, lac = 2, gain = 0.5) =>
  fractal(n, p, octaves, lac, gain, (v) => (1 - Math.abs(v)) ** 2);

export function fbm3(n: (p: V3) => number, p: V3, octaves = 5, lac = 2, gain = 0.5): number {
  let sum = 0;
  let norm = 0;
  let amp = 1;
  let q = p;
  for (let i = 0; i < 8; i++) {
    const w = Math.min(Math.max(octaves - i, 0), 1) * amp;
    sum += w * n(q);
    norm += w;
    amp *= gain;
    q = [(0.8 * q[0] - 0.6 * q[1]) * lac + 19.1, (0.6 * q[0] + 0.8 * q[1]) * lac - 7.3, q[2] * lac + 3.7];
  }
  return sum / norm;
}

/** Central differences of a 2D function: (d/dx, d/dy). */
export function grad(f: Noise2, p: V2, h = 1e-5): V2 {
  return [(f([p[0] + h, p[1]]) - f([p[0] - h, p[1]])) / (2 * h), (f([p[0], p[1] + h]) - f([p[0], p[1] - h])) / (2 * h)];
}

export function grad3d(f: (p: V3) => number, p: V3, h = 1e-5): V3 {
  return [0, 1, 2].map((k) => {
    const a = p.map((v, i) => (i === k ? v + h : v)) as V3;
    const b = p.map((v, i) => (i === k ? v - h : v)) as V3;
    return (f(a) - f(b)) / (2 * h);
  }) as V3;
}

const f32 = Math.fround;
const frac = (x: number) => x - Math.floor(x);

/** Interleaved gradient noise, rounded to float32 at each step as the GPU computes it. */
export function ign(px: V2, frame?: number): number {
  let p: V2 = [Math.floor(px[0]), Math.floor(px[1])];
  if (frame !== undefined) {
    const f = frame - 64 * Math.floor(frame / 64);
    p = [Math.floor(f32(p[0] + f32(5.588238 * f))), Math.floor(f32(p[1] + f32(5.588238 * f)))];
  }
  const d = f32(f32(p[0] * f32(0.06711056)) + f32(p[1] * f32(0.00583715)));
  return f32(frac(f32(f32(52.9829189) * f32(frac(d)))));
}

export function r2(px: V2): number {
  const g = 1.32471795724474602596;
  return frac(Math.floor(px[0]) / g + Math.floor(px[1]) / (g * g));
}

/** The Bayer matrix by its recursive definition: M(2n) = [[4M, 4M + 2], [4M + 3, 4M + 1]]. */
export function bayerMatrix(size: number): number[][] {
  let m = [[0]];
  while (m.length < size) {
    const n = m.length;
    const next = Array.from({ length: 2 * n }, () => new Array<number>(2 * n).fill(0));
    for (let y = 0; y < n; y++)
      for (let x = 0; x < n; x++) {
        const v = 4 * m[y]![x]!;
        next[y]![x] = v;
        next[y]![x + n] = v + 2;
        next[y + n]![x] = v + 3;
        next[y + n]![x + n] = v + 1;
      }
    m = next;
  }
  return m;
}

export const triangular = (u: number) => {
  const v = u * 2 - 1;
  return Math.sign(v) * (1 - Math.sqrt(1 - Math.abs(v)));
};
const at2 = (p: V2) => live(vec(p));
const at3 = (p: V3) => live(vec(p));
const words = (h: number) => [h >>> 16, h & 0xffff];
const slangWords = (e: string) => `float(${e} >> 16u), float(${e} & 0xffffu)`;

/** Sample points away from lattice and simplex boundaries, positive and negative. */
const POINTS2: V2[] = [
  [0.37, 0.61],
  [3.21, -1.73],
  [-7.42, 12.18],
  [104.63, 57.29],
  [-0.81, -0.33],
  [2.5, 2.25],
];
const POINTS3: V3[] = [
  [0.37, 0.61, 0.13],
  [3.21, -1.73, 2.57],
  [-7.42, 12.18, -3.39],
  [41.63, 7.29, 0.71],
];

const NOISES2: Array<[string, Noise2]> = [
  ["Perlin", (p) => perlin2(p)],
  ["Simplex", (p) => simplex2(p)],
  ["ValueNoise", (p) => value2(p)],
];
const NOISES3: Array<[string, (p: V3) => number]> = [
  ["Perlin", perlin3],
  ["Simplex", simplex3],
  ["ValueNoise", value3],
];

const GRID = { n: 20, step: [0.173, 0.211], start: [0.05, 0.07] };

/** A derivative's tolerance: relative to its size, at least `t`. */
const dtol = (d: number, t = 2e-3) => t * Math.max(1, Math.abs(d));

/** Every case the shaders must reproduce. */
export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol: number[]) => out.push({ name, slang, expect, tol });

  // Integer hashes, bit for bit: each word as two exact 16-bit halves; literal, then only known at run time.
  const hashCase = (name: string, exprs: string[], want: number[]) => {
    const body = (k: string) =>
      `float4(${exprs.map((e) => slangWords(e.replace(/K/g, k))).join(", ")}${exprs.length === 1 ? ", 0.0, 0.0" : ""})`;
    const expect = [...want.flatMap(words), ...(want.length === 1 ? [0, 0] : [])];
    add(name, body("u"), expect, [0, 0, 0, 0]);
    add(`${name}, at run time`, body("u * uint(one)"), expect, [0, 0, 0, 0]);
  };
  for (const v of [0, 1, 7, 0x7fffffff, 0xffffffff, 123456789, 0xdeadbeef])
    hashCase(`pcg(${v})`, [`pcg(${v}K)`], [pcg(v)]);
  for (const [x, y, z] of [
    [0, 0, 0],
    [1, 2, 3],
    [0xffffffff, 17, 99],
    [4000000000, 3000000000, 2000000000],
  ] as V3[]) {
    const h = pcg3d(x, y, z);
    const u3 = `uint3(${x}u, ${y}u, ${z}u) * uint3(1K, 1K, 1K)`;
    hashCase(`pcg3d(${x}, ${y}, ${z}).xy`, [`pcg3d(${u3}).x`, `pcg3d(${u3}).y`], [h[0], h[1]]);
    hashCase(
      `pcg3d(${x}, ${y}, ${z}).z, pcg(uint2)`,
      [`pcg3d(${u3}).z`, `pcg(uint2(${x}u, ${y}u) * uint2(1K, 1K))`],
      [h[2], pcg2(x, y)],
    );
    hashCase(`pcg(uint3(${x}, ${y}, ${z}))`, [`pcg(${u3})`], [pcg3(x, y, z)]);
  }
  for (const p of POINTS2) {
    const [cx, cy] = [cell(p[0]), cell(p[1])];
    const h3 = pcg3d(cx, cy, 0);
    const h1 = pcg3d(cx, 0, 0);
    add(
      `hash, hash2 at ${p}`,
      `float4(hash(${at2(p)}), hash(${live(num(p[0]))}), hash2(${at2(p)}))`,
      [unit(pcg2(cx, cy)), unit(pcg(cx)), unit(h3[0]), unit(h3[1])],
      [0, 0, 0, 0],
    );
    add(
      `hash3 at ${p}`,
      `float4(hash3(${at2(p)}), hash2(${live(num(p[0]))}).y)`,
      [unit(h3[0]), unit(h3[1]), unit(h3[2]), unit(h1[1])],
      [0, 0, 0, 0],
    );
  }
  for (const p of POINTS3) {
    const c = p.map(cell) as V3;
    add(
      `hash3 at ${p}`,
      `float4(hash3(${at3(p)}), hash(${at3(p)}))`,
      [...pcg3d(...c).map(unit), unit(pcg3(...c))],
      [0, 0, 0, 0],
    );
  }

  // Gradient noise is 0 on the integer lattice.
  for (const p of [
    [0, 0],
    [3, -5],
    [-17, 42],
  ] as V2[]) {
    add(
      `Perlin at lattice point ${p}`,
      `float4(Perlin.at(${at2(p)}), Perlin.at(float3(${at2(p)}, 2.0)), Perlin.tiled(${at2(p)}, float2(4.0, 4.0)), 0.0)`,
      [0, 0, 0, 0],
      [0, 0, 0, 0],
    );
  }

  for (const p of POINTS2) {
    for (const [name, f] of NOISES2) {
      const g = grad(f, p);
      add(
        `${name}.derivatives(${p})`,
        `float4(${name}.derivatives(${at2(p)}), ${name}.at(${at2(p)}))`,
        [f(p), g[0], g[1], f(p)],
        [2e-5, dtol(g[0]), dtol(g[1]), 2e-5],
      );
    }
    for (const [name, f] of [
      ["Perlin", (q: V2) => perlin2(q, [5, 3])],
      ["ValueNoise", (q: V2) => value2(q, [5, 3])],
    ] as const) {
      const g = grad(f, p);
      add(
        `${name}.tiledDerivatives(${p}), one period on`,
        `float4(${name}.tiledDerivatives(${at2(p)}, float2(5.0, 3.0)), ${name}.tiled(${at2(p)} + float2(10.0, -6.0), float2(5.0, 3.0)))`,
        [f(p), g[0], g[1], f(p)],
        [2e-5, dtol(g[0]), dtol(g[1]), 1e-4],
      );
    }
    for (const angle of [0.7, 2.9]) {
      const f = (q: V2) => simplex2(q, angle);
      const g = grad(f, p);
      add(
        `Simplex.flowDerivatives(${p}, ${angle}), one turn on`,
        `float4(Simplex.flowDerivatives(${at2(p)}, ${live(num(angle))}), Simplex.flow(${at2(p)}, ${num(angle + 2 * Math.PI)}))`,
        [f(p), g[0], g[1], f(p)],
        [2e-5, dtol(g[0]), dtol(g[1]), 1e-4],
      );
    }
  }
  for (const p of POINTS3) {
    for (const [name, f] of NOISES3) {
      const g = grad3d(f, p);
      add(
        `${name}.derivatives(${p})`,
        `${name}.derivatives(${at3(p)})`,
        [f(p), ...g],
        [2e-5, ...g.map((d) => dtol(d))],
      );
    }
  }

  // Ranges: the largest |n| of a 20x20 grid, reduced in the shader, against the reference's (all within 1).
  for (const [name, f] of NOISES2) {
    let hi = 0;
    for (let y = 0; y < GRID.n; y++)
      for (let x = 0; x < GRID.n; x++)
        hi = Math.max(hi, Math.abs(f([x * GRID.step[0]! + GRID.start[0]!, y * GRID.step[1]! + GRID.start[1]!])));
    add(
      `${name}: largest |n| over a grid`,
      `float4(sk_largest_${name}(one), 0.0, 0.0, 0.0)`,
      [hi, 0, 0, 0],
      [1e-4, 0, 0, 0],
    );
  }

  for (const p of POINTS2.slice(0, 4)) {
    for (const jitter of [1, 0.6]) {
      const w = worley2(p, jitter);
      add(
        `worley(${p}, ${jitter})`,
        `sk_cell_case(worley(${at2(p)}, ${num(jitter)}))`,
        [w.f1, w.f2, ...words(w.id)],
        [2e-5, 2e-5, 0, 0],
      );
    }
    const t = worley2(p, 1, [4, 6]);
    add(
      `worley(${p}) tiled 4x6, two periods on`,
      `sk_cell_case(worley(${at2(p)} + float2(8.0, -12.0), 1.0, float2(4.0, 6.0)))`,
      [t.f1, t.f2, ...words(t.id)],
      [1e-4, 1e-4, 0, 0],
    );
    add(`worleyEdge(${p})`, `float4(worleyEdge(${at2(p)}), 0.0, 0.0, 0.0)`, [worleyEdge(p), 0, 0, 0], [5e-5, 0, 0, 0]);
  }
  for (const p of POINTS3.slice(0, 3)) {
    const w = worley3(p);
    add(`worley(${p}) in 3D`, `sk_cell3_case(worley(${at3(p)}))`, [w.f1, w.f2, ...words(w.id)], [2e-5, 2e-5, 0, 0]);
  }

  for (const p of POINTS2.slice(0, 4)) {
    for (const [name, f] of NOISES2) {
      add(
        `fbm, turbulence, ridged of ${name} at ${p}`,
        `float4(fbm<${name}>(${at2(p)}), turbulence<${name}>(${at2(p)}, 3.5), ridged<${name}>(${at2(p)}, 4.0, 2.2, 0.45), fbm<${name}>(${at2(p)}, 1.0))`,
        [fbm(f, p), turbulence(f, p, 3.5), ridged(f, p, 4, 2.2, 0.45), f(p)],
        // Octaves reach |p| times lacunarity^3, where a float32 coordinate keeps fewer fraction bits.
        [5e-5, 5e-5, 5e-5, 2e-5].map((t) => t * Math.max(1, Math.hypot(...p) / 10)),
      );
    }
    const fd = (q: V2) => fbm((r) => simplex2(r), q, 3);
    const g = grad(fd, p);
    add(
      `fbmDerivatives<Simplex>(${p}, 3)`,
      `float4(fbmDerivatives<Simplex>(${at2(p)}, 3.0), 0.0)`,
      [fd(p), g[0], g[1], 0],
      [5e-5, dtol(g[0], 4e-3), dtol(g[1], 4e-3), 0],
    );
    const s = grad((q) => simplex2(q), p);
    add(
      `curl<Simplex>(${p}), and of its fbm`,
      `float4(curl<Simplex>(${at2(p)}), curl<Simplex>(${at2(p)}, 3.0))`,
      [s[1], -s[0], g[1], -g[0]],
      [dtol(s[1]), dtol(s[0]), dtol(g[1], 4e-3), dtol(g[0], 4e-3)],
    );
    const warped: V2 = [
      p[0] + 0.8 * fbm((r) => simplex2(r), p, 4),
      p[1] + 0.8 * fbm((r) => simplex2(r), [p[0] + 5.2, p[1] + 1.3], 4),
    ];
    add(
      `warp<Simplex>(${p}, 0.8)`,
      `float4(warp<Simplex>(${at2(p)}, 0.8), 0.0, 0.0)`,
      [...warped, 0, 0],
      [1e-4, 1e-4, 0, 0],
    );
    add(
      `curl<Perlin> is divergence-free at ${p}`,
      `float4(sk_divergence(${at2(p)}), 0.0, 0.0, 0.0)`,
      [0, 0, 0, 0],
      [0.02, 0, 0, 0],
    );
  }
  for (const p of POINTS3.slice(0, 2)) {
    for (const [name, f] of NOISES3)
      add(
        `fbm<${name}>(${p}) in 3D`,
        `float4(fbm<${name}>(${at3(p)}, 4.5), 0.0, 0.0, 0.0)`,
        [fbm3(f, p, 4.5), 0, 0, 0],
        [5e-5, 0, 0, 0],
      );
  }

  for (const px of [
    [10.5, 20.5],
    [123.5, 7.5],
    [301.5, 255.5],
    [64.5, 199.5],
  ] as V2[]) {
    add(
      `ign, r2 at ${px}`,
      `float4(ign(${at2(px)}), ign(${at2(px)}, 13.0), r2(${at2(px)}), 0.0)`,
      [ign(px), ign(px, 13), r2(px), 0],
      [1e-3, 1e-3, 2e-4, 0],
    );
  }
  for (const size of [2, 4, 8]) {
    const m = bayerMatrix(size);
    const xs = [0, 1, 2, 3].map((x) => (x * (size > 4 ? 2 : 1)) % size);
    for (let y = 0; y < size; y += size > 4 ? 3 : 1) {
      // Pixels several matrices away, on both sides of 0, land on the same entries.
      const at = (x: number) => `bayer(${live(vec([x + 16 * size + 0.5, y - 8 * size + 0.5]))}, ${size})`;
      add(
        `bayer ${size}x${size}, row ${y}`,
        `float4(${xs.map(at).join(", ")})`,
        xs.map((x) => (m[y]![x]! + 0.5) / (size * size)),
        [0, 0, 0, 0],
      );
    }
  }
  for (const u of [0, 0.1, 0.5, 0.75, 0.999])
    add(
      `triangular(${u})`,
      `float4(triangular(${live(num(u))}), 0.0, 0.0, 0.0)`,
      [triangular(u), 0, 0, 0],
      [1e-6, 0, 0, 0],
    );
  for (const px of [
    [3.5, 9.5],
    [250.5, 40.5],
  ] as V2[]) {
    const h = pcg3d(cell(px[0]), cell(px[1]), 0).map(unit);
    const hf = pcg3d(cell(px[0]), cell(px[1]), 5).map(unit);
    add(
      `dither at ${px}`,
      `float4(dither(float3(0.25, 0.5, 0.75), ${at2(px)}), dither(float3(0.5, 0.5, 0.5), ${at2(px)}, 5.0, 63.0).y)`,
      [
        0.25 + triangular(h[0]!) / 255,
        0.5 + triangular(h[1]!) / 255,
        0.75 + triangular(h[2]!) / 255,
        0.5 + triangular(hf[1]!) / 63,
      ],
      [1e-6, 1e-6, 1e-6, 1e-6],
    );
  }
  return withRunTime(out);
}

/** Shader helpers the cases call. */
export const DECLARATIONS = `
float4 sk_cell_case(Cell c) { return float4(c.f1, c.f2, float(c.id >> 16u), float(c.id & 0xffffu)); }
float4 sk_cell3_case(Cell3 c) { return float4(c.f1, c.f2, float(c.id >> 16u), float(c.id & 0xffffu)); }
${NOISES2.map(
  ([n]) => `float sk_largest_${n}(float one)
{
    float hi = 0.0;
    for (int y = 0; y < ${GRID.n}; y++)
        for (int x = 0; x < ${GRID.n}; x++)
            hi = max(hi, abs(${n}.at(float2(float(x) * ${GRID.step[0]} + ${GRID.start[0]}, float(y) * ${GRID.step[1]} + ${GRID.start[1]}) * one)));
    return hi;
}`,
).join("\n")}
float sk_divergence(float2 p)
{
    float h = 1e-3;
    float2 ax = curl<Perlin>(p + float2(h, 0.0)) - curl<Perlin>(p - float2(h, 0.0));
    float2 ay = curl<Perlin>(p + float2(0.0, h)) - curl<Perlin>(p - float2(0.0, h));
    return (ax.x + ay.y) / (2.0 * h);
}
`;

export const IMPORTS = ["sk.noise"];

/** One effect per chunk of cases. */
export const caseEffect = (name: string, chunk: Case[]) => tableEffect(name, chunk, IMPORTS, DECLARATIONS);

/**
 * A table split into `count` parts, each ordered so that `tiles` consecutive slices of equal length cost
 * about the same to compile: fxc's time grows faster than a shader's size, so a tile holding every
 * heavy case (the loops, the 49-read kernels) would take most of a minute on its own.
 */
export function parts(all: Case[], cost: (c: Case) => number, count = 2, tiles = 8): Case[][] {
  const n = count * tiles;
  const per = Math.ceil(all.length / n);
  const bins = Array.from({ length: n }, () => ({ cost: 0, cases: [] as Case[] }));
  for (const c of [...all].sort((a, b) => cost(b) - cost(a))) {
    const bin = bins.filter((b) => b.cases.length < per).reduce((m, b) => (b.cost < m.cost ? b : m));
    bin.cases.push(c);
    bin.cost += cost(c);
  }
  return Array.from({ length: count }, (_, i) => bins.slice(i * tiles, (i + 1) * tiles).flatMap((b) => b.cases));
}

/** What a case costs fxc: the loops most, cellular searches next; a constant case partly folds. */
export const cost = (c: Case) =>
  (/fbm|turbulence|curl|warp|largest/.test(c.name) ? 8 : /worley/.test(c.name) ? 3 : 1) *
  (c.name.endsWith("at run time") ? 1 : 0.5);
