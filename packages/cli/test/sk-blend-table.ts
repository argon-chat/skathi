// `sk.blend` golden table: W3C Compositing and Blending Level 1 written out in float64, case split
// for case split as the spec prints it, and the cases the shader must reproduce.

import { type Case, live, num, vec, withRunTime } from "./shader-table";
import { SPACES, type V3 } from "./sk-color-table";

export const IMPORTS = ["sk.color", "sk.blend"];

type V4 = [number, number, number, number];

const lum = ([r, g, b]: V3) => 0.3 * r + 0.59 * g + 0.11 * b;

function clipColor(c: V3): V3 {
  const l = lum(c);
  const n = Math.min(...c);
  const x = Math.max(...c);
  let out = c;
  if (n < 0) out = out.map((v) => l + ((v - l) * l) / (l - n)) as V3;
  if (x > 1) out = out.map((v) => l + ((v - l) * (1 - l)) / (x - l)) as V3;
  return out;
}

const setLum = (c: V3, l: number): V3 => {
  const d = l - lum(c);
  return clipColor(c.map((v) => v + d) as V3);
};

const sat = (c: V3) => Math.max(...c) - Math.min(...c);

function setSat(c: V3, s: number): V3 {
  const order = [0, 1, 2].sort((a, b) => c[a]! - c[b]!);
  const [lo, mid, hi] = order as [number, number, number];
  const out: V3 = [0, 0, 0];
  if (c[hi]! > c[lo]!) {
    out[mid] = ((c[mid]! - c[lo]!) * s) / (c[hi]! - c[lo]!);
    out[hi] = s;
  }
  return out;
}

type Separable = (cb: number, cs: number) => number;

const multiply: Separable = (cb, cs) => cb * cs;
const screen: Separable = (cb, cs) => cb + cs - cb * cs;
const hardLight: Separable = (cb, cs) => (cs <= 0.5 ? multiply(cb, 2 * cs) : screen(cb, 2 * cs - 1));

const SEPARABLE: Record<string, Separable> = {
  Normal: (_cb, cs) => cs,
  Multiply: multiply,
  Screen: screen,
  Overlay: (cb, cs) => hardLight(cs, cb),
  Darken: Math.min,
  Lighten: Math.max,
  ColorDodge: (cb, cs) => {
    if (cb <= 0) return 0;
    if (cs >= 1) return 1;
    return Math.min(1, cb / (1 - cs));
  },
  ColorBurn: (cb, cs) => {
    if (cb >= 1) return 1;
    if (cs <= 0) return 0;
    return 1 - Math.min(1, (1 - cb) / cs);
  },
  HardLight: hardLight,
  SoftLight: (cb, cs) => {
    if (cs <= 0.5) return cb - (1 - 2 * cs) * cb * (1 - cb);
    const d = cb <= 0.25 ? ((16 * cb - 12) * cb + 4) * cb : Math.sqrt(Math.max(cb, 0));
    return cb + (2 * cs - 1) * (d - cb);
  },
  Difference: (cb, cs) => Math.abs(cb - cs),
  Exclusion: (cb, cs) => cb + cs - 2 * cb * cs,
};

const NON_SEPARABLE: Record<string, (cb: V3, cs: V3) => V3> = {
  Hue: (cb, cs) => setLum(setSat(cs, sat(cb)), lum(cb)),
  Saturation: (cb, cs) => setLum(setSat(cb, sat(cs)), lum(cb)),
  Color: (cb, cs) => setLum(cs, lum(cb)),
  Luminosity: (cb, cs) => setLum(cb, lum(cs)),
};

export const MODES = [...Object.keys(SEPARABLE), ...Object.keys(NON_SEPARABLE)];

export function mode(name: string): (cb: V3, cs: V3) => V3 {
  const f = SEPARABLE[name];
  if (f) return (cb, cs) => cb.map((v, i) => f(v, cs[i]!)) as V3;
  return NON_SEPARABLE[name]!;
}

const straight = (c: V4): V3 => (c[3] > 0 ? (c.slice(0, 3).map((v) => v / c[3]) as V3) : [0, 0, 0]);

/** §5.8 general formula: blend with source-over, premultiplied in and out. */
export function blend(name: string, below: V4, above: V4): V4 {
  const [ab, as] = [below[3], above[3]];
  const b = mode(name)(straight(below), straight(above));
  const rgb = [0, 1, 2].map((i) => above[i]! * (1 - ab) + below[i]! * (1 - as) + as * ab * b[i]!);
  return [...rgb, as + ab * (1 - as)] as V4;
}

const pd = (fa: (ab: number, as: number) => number, fb: (ab: number, as: number) => number) => (b: V4, a: V4) =>
  a.map((v, i) => v * fa(b[3], a[3]) + b[i]! * fb(b[3], a[3])) as V4;

export const OPERATORS: Record<string, (below: V4, above: V4) => V4> = {
  Clear: pd(
    () => 0,
    () => 0,
  ),
  Copy: pd(
    () => 1,
    () => 0,
  ),
  Destination: pd(
    () => 0,
    () => 1,
  ),
  SourceOver: pd(
    () => 1,
    (_ab, as) => 1 - as,
  ),
  DestinationOver: pd(
    (ab) => 1 - ab,
    () => 1,
  ),
  SourceIn: pd(
    (ab) => ab,
    () => 0,
  ),
  DestinationIn: pd(
    () => 0,
    (_ab, as) => as,
  ),
  SourceOut: pd(
    (ab) => 1 - ab,
    () => 0,
  ),
  DestinationOut: pd(
    () => 0,
    (_ab, as) => 1 - as,
  ),
  SourceAtop: pd(
    (ab) => ab,
    (_ab, as) => 1 - as,
  ),
  DestinationAtop: pd(
    (ab) => 1 - ab,
    (_ab, as) => as,
  ),
  Xor: pd(
    (ab) => 1 - ab,
    (_ab, as) => 1 - as,
  ),
  Lighter: (b, a) => a.map((v, i) => v + b[i]!) as V4,
  PlusLighter: (b, a) => a.map((v, i) => Math.min(1, v + b[i]!)) as V4,
  PlusDarker: (b, a) => {
    const alpha = Math.min(1, a[3] + b[3]);
    return [...[0, 1, 2].map((i) => Math.max(0, alpha - (a[3] - a[i]!) - (b[3] - b[i]!))), alpha] as V4;
  },
};

const premultiplied = (rgb: V3, a: number): V4 => [...(rgb.map((v) => v * a) as V3), a];

/** Premultiplied sRGB-encoded pairs: opaque, translucent, transparent, HDR and the dodge/burn edges. */
const PAIRS: Array<[string, V4, V4]> = [
  ["opaque", premultiplied([0.2, 0.5, 0.8], 1), premultiplied([0.9, 0.4, 0.1], 1)],
  ["translucent", premultiplied([0.7, 0.3, 0.6], 0.8), premultiplied([0.25, 0.75, 0.5], 0.6)],
  ["dark over light", premultiplied([0.95, 0.9, 0.85], 1), premultiplied([0.05, 0.3, 0.45], 0.5)],
  ["transparent above", premultiplied([0.3, 0.6, 0.9], 0.7), [0, 0, 0, 0]],
  ["transparent below", [0, 0, 0, 0], premultiplied([0.3, 0.6, 0.9], 0.7)],
  ["edges", premultiplied([0, 1, 0.5], 1), premultiplied([1, 0, 0.5], 1)],
  ["greys", premultiplied([0.5, 0.5, 0.5], 1), premultiplied([0.2, 0.2, 0.2], 0.9)],
  ["hdr", premultiplied([1.6, 0.4, 0.2], 1), premultiplied([0.3, 1.25, 0.6], 0.75)],
];

const f4 = (c: V4) => live(vec(c));

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol = 2e-5) =>
    out.push({ name, slang, expect, tol: expect.map((v) => Math.max(tol, Math.abs(v) * 2e-6)) });

  for (const m of MODES) {
    for (const [what, below, above] of PAIRS) {
      add(`blend<${m}> ${what}`, `blend<Blend.${m}>(${f4(below)}, ${f4(above)})`, blend(m, below, above));
    }
    const cb: V3 = [0.15, 0.55, 0.95];
    const cs: V3 = [0.8, 0.35, 0.05];
    add(`Blend.${m}.apply`, `float4(Blend.${m}.apply(${live(vec(cb))}, ${live(vec(cs))}), 0.0)`, [
      ...mode(m)(cb, cs),
      0,
    ]);
  }
  const [, below, above] = PAIRS[1]!;
  add(
    "blend with opacity",
    `blend<Blend.Multiply>(${f4(below)}, ${f4(above)}, ${live(num(0.4))})`,
    blend("Multiply", below, above.map((v) => v * 0.4) as V4),
  );

  const enc = SPACES.Srgb.fromLinear;
  const dec = SPACES.Srgb.toLinear;
  const color = (rgb: V3, a: number) => `Color.fromSrgb(${live(vec([...rgb, a]))})`;
  for (const m of ["Multiply", "Overlay", "Hue", "Luminosity"]) {
    const b: [V3, number] = [[0.7, 0.3, 0.6], 0.8];
    const a: [V3, number] = [[0.25, 0.75, 0.5], 0.6];
    const r = blend(m, premultiplied(...b), premultiplied(...a));
    add(`blend<${m}> on Color`, `blend<Blend.${m}>(${color(...b)}, ${color(...a)}).toLinear()`, [
      ...dec(straight(r)),
      r[3],
    ]);
    const lin = blend(m, premultiplied(dec(b[0]), b[1]), premultiplied(dec(a[0]), a[1]));
    add(`blendLinear<${m}>`, `blendLinear<Blend.${m}>(${color(...b)}, ${color(...a)}).toLinear()`, [
      ...straight(lin),
      lin[3],
    ]);
  }

  for (const [op, f] of Object.entries(OPERATORS)) {
    for (const [what, below, above] of PAIRS.slice(0, 5)) {
      add(`composite<${op}> ${what}`, `composite<Composite.${op}>(${f4(below)}, ${f4(above)})`, f(below, above));
    }
  }
  const b: V3 = [0.2, 0.9, 0.4];
  const r = OPERATORS.DestinationOut!(premultiplied(b, 0.9), premultiplied([1, 1, 1], 0.5));
  add(
    "composite<DestinationOut> on Color",
    `composite<Composite.DestinationOut>(${color(b, 0.9)}, ${color([1, 1, 1], 0.5)}).toSrgb()`,
    [...enc(dec(straight(r))), r[3]],
  );
  return withRunTime(out);
}
