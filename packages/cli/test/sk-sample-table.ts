// `sk.sample` golden table: every kernel over a procedural source whose texels are known in closed form, against
// the discrete convolution computed here (binomial Gaussians, the continuous box, 16-tap bicubics), not against
// the kernels' own bilinear-merged taps. The same source sampled through the engine's textures is the e2e spec's.

import { type Case, live, tableEffect, vec, withRunTime } from "./shader-table";

type V2 = [number, number];
type V4 = [number, number, number, number];

/** frac((p x + q y) / n) for a power of two n: exact in float32 as in float64, and cheap under fxc. */
const m = (x: number, y: number, p: number, q: number, n: number) => ((p * x + q * y) & (n - 1)) / n;

/** The procedural texel at whole coordinates (texel centres at k + 0.5): premultiplied, alpha 0.35..0.95. */
export function texel(x: number, y: number): V4 {
  const a = 0.35 + 0.6 * m(x, y, 7, 13, 16);
  return [a * m(x, y, 5, 3, 8), a * m(x, y, 3, 9, 16), a * m(x, y, 11, 4, 4), a];
}

const add4 = (s: V4, v: V4, w = 1): V4 => s.map((x, k) => x + v[k]! * w) as V4;
const zero = (): V4 => [0, 0, 0, 0];

/** Bilinear reconstruction of the texels at a point. */
export function bilinear(px: V2): V4 {
  const tx = px[0] - 0.5;
  const ty = px[1] - 0.5;
  const x0 = Math.floor(tx);
  const y0 = Math.floor(ty);
  const fx = tx - x0;
  const fy = ty - y0;
  let s = zero();
  s = add4(s, texel(x0, y0), (1 - fx) * (1 - fy));
  s = add4(s, texel(x0 + 1, y0), fx * (1 - fy));
  s = add4(s, texel(x0, y0 + 1), (1 - fx) * fy);
  return add4(s, texel(x0 + 1, y0 + 1), fx * fy);
}

const binomial = (n: number) => {
  const row = [1];
  for (let i = 0; i < n; i++) row.push((row[i]! * (n - i)) / (i + 1));
  const sum = 2 ** n;
  return row.map((v) => v / sum);
};

/** The discrete binomial Gaussian (row taps-1) around the texel at `px` (a texel centre). */
export function gaussian(taps: number, px: V2): V4 {
  const w = binomial(taps - 1);
  const h = (taps - 1) / 2;
  const [cx, cy] = [Math.floor(px[0]), Math.floor(px[1])];
  let s = zero();
  for (let j = -h; j <= h; j++) for (let i = -h; i <= h; i++) s = add4(s, texel(cx + i, cy + j), w[i + h]! * w[j + h]!);
  return s;
}

export function gaussianLine(taps: number, px: V2, dir: V2): V4 {
  const w = binomial(taps - 1);
  const h = (taps - 1) / 2;
  let s = zero();
  for (let i = -h; i <= h; i++) s = add4(s, bilinear([px[0] + dir[0] * i, px[1] + dir[1] * i]), w[i + h]!);
  return s;
}

/** A continuous box of half-width 2 over the bilinear reconstruction, on a texel centre: weights 1/2, 1, 1, 1, 1/2. */
export function box2(px: V2): V4 {
  const w = [0.5, 1, 1, 1, 0.5].map((v) => v / 4);
  const [cx, cy] = [Math.floor(px[0]), Math.floor(px[1])];
  let s = zero();
  for (let j = -2; j <= 2; j++) for (let i = -2; i <= 2; i++) s = add4(s, texel(cx + i, cy + j), w[i + 2]! * w[j + 2]!);
  return s;
}

const mean = (points: V2[]) => points.reduce((s, p) => add4(s, bilinear(p), 1 / points.length), zero());

export const box = (px: V2, r: number) => {
  const pts: V2[] = [];
  for (let y = 0; y < 4; y++)
    for (let x = 0; x < 4; x++) pts.push([px[0] + (x - 1.5) * r * 0.5, px[1] + (y - 1.5) * r * 0.5]);
  return mean(pts);
};

const GOLDEN = Math.PI * (3 - Math.sqrt(5));
export const disc = (n: number, px: V2, r: number) =>
  mean(
    Array.from({ length: n }, (_, i) => {
      const d = r * Math.sqrt((i + 0.5) / n);
      return [px[0] + d * Math.cos(i * GOLDEN), px[1] + d * Math.sin(i * GOLDEN)] as V2;
    }),
  );
export const directional = (n: number, px: V2, d: V2) =>
  mean(
    Array.from(
      { length: n },
      (_, i) => [px[0] + d[0] * ((i + 0.5) / n - 0.5), px[1] + d[1] * ((i + 0.5) / n - 0.5)] as V2,
    ),
  );
export const radial = (n: number, px: V2, c: V2, amount: number) =>
  mean(
    Array.from(
      { length: n },
      (_, i) => [px[0] + (c[0] - px[0]) * ((amount * i) / n), px[1] + (c[1] - px[1]) * ((amount * i) / n)] as V2,
    ),
  );

const channels = (r: V4, g: V4, b: V4): V4 => [r[0], g[1], b[2], Math.max(r[3], g[3], b[3])];
export const chromatic = (px: V2, s: V2) =>
  channels(bilinear([px[0] + s[0], px[1] + s[1]]), bilinear(px), bilinear([px[0] - s[0], px[1] - s[1]]));
export const dispersion = (px: V2, o: V2, k: number) =>
  channels(
    bilinear([px[0] + o[0] * (1 + k), px[1] + o[1] * (1 + k)]),
    bilinear([px[0] + o[0], px[1] + o[1]]),
    bilinear([px[0] + o[0] * (1 - k), px[1] + o[1] * (1 - k)]),
  );

/** Bicubic resampling over the 4x4 texels around a point, with a cubic's weight function. */
function bicubic(px: V2, w: (t: number) => number): V4 {
  const tx = px[0] - 0.5;
  const ty = px[1] - 0.5;
  const x0 = Math.floor(tx);
  const y0 = Math.floor(ty);
  let s = zero();
  for (let j = -1; j <= 2; j++)
    for (let i = -1; i <= 2; i++) s = add4(s, texel(x0 + i, y0 + j), w(tx - (x0 + i)) * w(ty - (y0 + j)));
  return s;
}
/** Catmull-Rom (Keys, a = -0.5) and the cubic B-spline as kernels of distance. */
const keys = (t: number) => {
  const x = Math.abs(t);
  return x < 1 ? 1.5 * x ** 3 - 2.5 * x ** 2 + 1 : x < 2 ? -0.5 * x ** 3 + 2.5 * x ** 2 - 4 * x + 2 : 0;
};
const bsplineW = (t: number) => {
  const x = Math.abs(t);
  return x < 1 ? (4 - 6 * x * x + 3 * x ** 3) / 6 : x < 2 ? (2 - x) ** 3 / 6 : 0;
};
export const catmullRom = (px: V2) => bicubic(px, keys).map((v) => Math.max(v, 0)) as V4;
export const bspline = (px: V2) => bicubic(px, bsplineW);

/** Unsharp masking against the 3x3 tent [1 2 1]^2 / 16, on a texel centre. */
export function sharpen(px: V2, amount: number): V4 {
  const [cx, cy] = [Math.floor(px[0]), Math.floor(px[1])];
  const c = texel(cx, cy);
  const t = [0.25, 0.5, 0.25];
  let blur = zero();
  for (let j = -1; j <= 1; j++)
    for (let i = -1; i <= 1; i++) blur = add4(blur, texel(cx + i, cy + j), t[i + 1]! * t[j + 1]!);
  return c.map((v, k) => Math.max(v + (v - blur[k]!) * amount, 0)) as V4;
}

export const cellCenter = (px: V2, cell: V2): V2 => [
  (Math.floor(px[0] / cell[0]) + 0.5) * cell[0],
  (Math.floor(px[1] / cell[1]) + 0.5) * cell[1],
];

/** Mirrored repeat by reflection: fold into [0, 2 size), then reflect the upper half. */
export const mirrorRepeat = (p: V2, size: V2): V2 =>
  p.map((v, k) => {
    const period = 2 * size[k]!;
    const m = ((v % period) + period) % period;
    return m > size[k]! ? period - m : m;
  }) as V2;

export function kaleidoscope(p: V2, c: V2, segments: number, rotation: number): V2 {
  const d: V2 = [p[0] - c[0], p[1] - c[1]];
  const wedge = (2 * Math.PI) / segments;
  let a = Math.atan2(d[1], d[0]) - rotation;
  a = ((a % wedge) + wedge) % wedge;
  a = Math.abs(a - wedge / 2);
  const r = Math.hypot(...d);
  return [c[0] + r * Math.cos(a + rotation), c[1] + r * Math.sin(a + rotation)];
}

/** Texel centres, and points between them, away from the procedural source's discontinuity-free but steep spots. */
const CENTRES: V2[] = [
  [20.5, 14.5],
  [37.5, 41.5],
  [-12.5, 6.5],
];
const BETWEEN: V2[] = [
  [20.3, 14.85],
  [33.71, 40.06],
];

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol: number[]) => out.push({ name, slang, expect, tol });
  const v = (x: number[]) => live(vec(x));
  const TOL = [2e-5, 2e-5, 2e-5, 2e-5];
  const P = "SkProc()";

  for (const px of [...CENTRES, ...BETWEEN]) add(`the source at ${px}`, `${P}.at(${v(px)})`, bilinear(px), TOL);
  for (const px of CENTRES) {
    for (const k of px === CENTRES[0] ? [5, 9, 13] : [5, 9])
      add(`gaussian<Gaussian${k}> at ${px}`, `gaussian<Gaussian${k}>(${P}, ${v(px)})`, gaussian(k, px), TOL);
    const axis: V2 = px === CENTRES[1] ? [0, 1] : [1, 0];
    add(
      `gaussianLine<Gaussian9> along ${axis} at ${px}`,
      `gaussianLine<Gaussian9>(${P}, ${v(px)}, ${vec(axis)})`,
      gaussianLine(9, px, axis),
      TOL,
    );
    add(`box(2) at ${px}`, `box(${P}, ${v(px)}, 2.0)`, box2(px), TOL);
    add(`box(5) at ${px}`, `box(${P}, ${v(px)}, 5.0)`, box(px, 5), TOL);
    add(`sharpen(0.8) at ${px}`, `sharpen(${P}, ${v(px)}, 0.8)`, sharpen(px, 0.8), TOL);
  }
  for (const px of [...CENTRES.slice(0, 2), ...BETWEEN]) {
    if (px === BETWEEN[0])
      add(
        `gaussian<Gaussian9> scale 2 at ${px}`,
        `gaussian<Gaussian9>(${P}, ${v(px)}, 2.0)`,
        (() => {
          const o = [-3.1111111111111111, -1.3333333333333333, 0, 1.3333333333333333, 3.1111111111111111];
          const w = [0.03515625, 0.328125, 0.2734375, 0.328125, 0.03515625];
          let s = zero();
          for (let j = 0; j < 5; j++)
            for (let i = 0; i < 5; i++) s = add4(s, bilinear([px[0] + o[i]! * 2, px[1] + o[j]! * 2]), w[i]! * w[j]!);
          return s;
        })(),
        TOL,
      );
    if (px !== CENTRES[1])
      add(`disc<16> at ${px}`, `disc<16>(${P}, ${v(px)}, 5.0)`, disc(16, px, 5), [1e-4, 1e-4, 1e-4, 1e-4]);
    add(
      `directional<8> at ${px}`,
      `directional<8>(${P}, ${v(px)}, float2(7.0, -3.0))`,
      directional(8, px, [7, -3]),
      TOL,
    );
    add(
      `radial<12> at ${px}`,
      `radial<12>(${P}, ${v(px)}, float2(30.0, 20.0), 0.4)`,
      radial(12, px, [30, 20], 0.4),
      TOL,
    );
    add(`chromatic at ${px}`, `chromatic(${P}, ${v(px)}, float2(2.5, -1.0))`, chromatic(px, [2.5, -1]), TOL);
    add(
      `chromaticRadial at ${px}`,
      `chromaticRadial(${P}, ${v(px)}, float2(30.0, 20.0), 0.05)`,
      chromatic(px, [(px[0] - 30) * 0.05, (px[1] - 20) * 0.05]),
      TOL,
    );
    add(`dispersion at ${px}`, `dispersion(${P}, ${v(px)}, float2(4.0, 3.0), 0.2)`, dispersion(px, [4, 3], 0.2), TOL);
    add(
      `displace at ${px}`,
      `displace(${P}, ${v(px)}, float2(1.5, -2.25)) + displace(${P}, ${v(px)}, float3(0.6, 0.0, 0.8), 5.0)`,
      add4(bilinear([px[0] + 1.5, px[1] - 2.25]), bilinear([px[0] + 3, px[1]])),
      TOL,
    );
    add(`catmullRom at ${px}`, `catmullRom(${P}, ${v(px)})`, catmullRom(px), TOL);
    add(`bspline at ${px}`, `bspline(${P}, ${v(px)})`, bspline(px), TOL);
    add(`pixelate at ${px}`, `pixelate(${P}, ${v(px)}, float2(6.0, 4.0))`, bilinear(cellCenter(px, [6, 4])), TOL);
  }
  // A transparent source gives exact zeros through every kernel, never a NaN.
  add(
    "a transparent source stays exactly transparent",
    `sk_transparent(${v([20.5, 14.5])})`,
    [0, 0, 0, 0],
    [0, 0, 0, 0],
  );
  for (const p of [
    [13.25, -4.5],
    [-30.75, 61.2],
    [5, 5],
  ] as V2[]) {
    add(
      `mirrorRepeat and cellCenter of ${p}`,
      `float4(mirrorRepeat(${v(p)}, float2(10.0, 24.0)), cellCenter(${v(p)}, float2(8.0, 6.0)))`,
      [...mirrorRepeat(p, [10, 24]), ...cellCenter(p, [8, 6])],
      [1e-5, 1e-5, 0, 0],
    );
  }
  for (const p of [
    [70, 15],
    [12, 80],
    [41, 49],
  ] as V2[]) {
    add(
      `kaleidoscope of ${p}`,
      `float4(kaleidoscope(${v(p)}, float2(40.0, 40.0), 6, 0.3), kaleidoscope(${v(p)}, float2(40.0, 40.0), 3))`,
      [...kaleidoscope(p, [40, 40], 6, 0.3), ...kaleidoscope(p, [40, 40], 3, 0)],
      [1e-3, 1e-3, 1e-3, 1e-3],
    );
  }
  return withRunTime(out);
}

const TAPS: Array<[RegExp, number]> = [
  [/Gaussian13/, 49],
  [/^gaussian<Gaussian9>/, 25],
  [/box|disc|sharpen/, 16],
  [/radial|catmullRom|Gaussian/, 10],
];

/** Reads a case costs fxc; a constant case folds to almost nothing. */
export const cost = (c: Case) =>
  (TAPS.find(([re]) => re.test(c.name))?.[1] ?? 3) * (c.name.endsWith("at run time") ? 1 : 0.2);

export const DECLARATIONS = `
struct SkProc : ISampler
{
    float m(float2 k, float p, float q, float n) { return frac((p * k.x + q * k.y) / n); }
    float4 texel(float2 k)
    {
        float a = 0.35 + 0.6 * m(k, 7.0, 13.0, 16.0);
        return float4(a * m(k, 5.0, 3.0, 8.0), a * m(k, 3.0, 9.0, 16.0), a * m(k, 11.0, 4.0, 4.0), a);
    }
    float4 at(float2 px)
    {
        float2 t = px - 0.5;
        float2 i = floor(t);
        float2 f = t - i;
        return lerp(lerp(texel(i), texel(i + float2(1.0, 0.0)), f.x), lerp(texel(i + float2(0.0, 1.0)), texel(i + 1.0), f.x), f.y);
    }
    float2 texelOffset() { return float2(0.5, 0.5); }
};
struct SkZero : ISampler
{
    float4 at(float2 px) { return float4(0.0, 0.0, 0.0, 0.0); }
    float2 texelOffset() { return float2(0.5, 0.5); }
};
float4 sk_transparent(float2 px)
{
    SkZero z;
    return gaussian<Gaussian13>(z, px) + box(z, px, 3.0) + disc<8>(z, px, 4.0) + directional<4>(z, px, float2(3.0, 1.0))
        + radial<4>(z, px, float2(0.0, 0.0), 0.5) + chromatic(z, px, float2(1.0, 0.0)) + dispersion(z, px, float2(2.0, 1.0), 0.3)
        + catmullRom(z, px + 0.3) + bspline(z, px + 0.3) + sharpen(z, px, 2.0) + pixelate(z, px, float2(4.0, 4.0));
}
`;

export const IMPORTS = ["sk.sample"];

export const caseEffect = (name: string, chunk: Case[]) => tableEffect(name, chunk, IMPORTS, DECLARATIONS);
