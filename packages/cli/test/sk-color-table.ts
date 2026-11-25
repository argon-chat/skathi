// `sk.color` golden table: a float64 reference written from the CSS Color 4 sample code and colord's
// published semantics, and the cases the shaders must reproduce. Used by effects.test.ts (reference
// against published values) and by packages/test/specs/shader-color.spec.ts (the same cases on the GPU).

import { type Case, num, tableEffect, vec } from "./shader-table";

export { type Case, decode, effectHeight, misses } from "./shader-table";

export const caseEffect = (name: string, chunk: Case[]) => tableEffect(name, chunk, ["sk.color"]);

export type V3 = [number, number, number];
type M3 = [V3, V3, V3];

const mul = (m: M3, v: V3): V3 => m.map((r) => r[0] * v[0] + r[1] * v[1] + r[2] * v[2]) as V3;

const SRGB_TO_XYZ: M3 = [
  [506752 / 1228815, 87881 / 245763, 12673 / 70218],
  [87098 / 409605, 175762 / 245763, 12673 / 175545],
  [7918 / 409605, 87881 / 737289, 1001167 / 1053270],
];
const XYZ_TO_SRGB: M3 = [
  [12831 / 3959, -329 / 214, -1974 / 3959],
  [-851781 / 878810, 1648619 / 878810, 36519 / 878810],
  [705 / 12673, -2585 / 12673, 705 / 667],
];
const D65_TO_D50: M3 = [
  [1.0479297925449969, 0.022946870601609652, -0.05019226628920524],
  [0.02962780877005599, 0.9904344267538799, -0.017073799063418826],
  [-0.009243040646204504, 0.015055191490298152, 0.7518742814281371],
];
const D50_TO_D65: M3 = [
  [0.955473421488075, -0.02309845494876471, 0.06325924320057072],
  [-0.0283697093338637, 1.0099953980813041, 0.021041441191917323],
  [0.012314014864481998, -0.020507649298898964, 1.330365926242124],
];
const XYZ_TO_LMS: M3 = [
  [0.819022437996703, 0.3619062600528904, -0.1288737815209879],
  [0.0329836539323885, 0.9292868615863434, 0.0361446663506424],
  [0.0481771893596242, 0.2642395317527308, 0.6335478284694309],
];
const LMS_TO_OKLAB: M3 = [
  [0.210454268309314, 0.7936177747023054, -0.0040720430116193],
  [1.9779985324311684, -2.42859224204858, 0.450593709617411],
  [0.0259040424655478, 0.7827717124575296, -0.8086757549230774],
];
const LMS_TO_XYZ: M3 = [
  [1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
  [-0.0405757452148008, 1.112286803280317, -0.0717110580655164],
  [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816],
];
const OKLAB_TO_LMS: M3 = [
  [1, 0.3963377773761749, 0.2158037573099136],
  [1, -0.1055613458156586, -0.0638541728258133],
  [1, -0.0894841775298119, -1.2914855480194092],
];
const D50: V3 = [0.3457 / 0.3585, 1, (1 - 0.3457 - 0.3585) / 0.3585];

const linearize = (v: number) => {
  const a = Math.abs(v);
  return a <= 0.04045 ? v / 12.92 : Math.sign(v) * ((a + 0.055) / 1.055) ** 2.4;
};
const gamma = (v: number) => {
  const a = Math.abs(v);
  return a > 0.0031308 ? Math.sign(v) * (1.055 * a ** (1 / 2.4) - 0.055) : 12.92 * v;
};
const wrap = (h: number) => h - 360 * Math.floor(h / 360);

function rgbToHsl([r, g, b]: V3): V3 {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (min + max) / 2;
  const d = max - min;
  let h = 0;
  let s = 0;
  if (d !== 0) {
    s = l === 0 || l === 1 ? 0 : (max - l) / Math.min(l, 1 - l);
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
  }
  if (s < 0) {
    h += 180;
    s = Math.abs(s);
  }
  return [wrap(h), s, l];
}

function hslToRgb([h, s, l]: V3): V3 {
  const f = (n: number) => {
    const k = (n + wrap(h) / 30) % 12;
    const a = s * Math.min(l, 1 - l);
    return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return [f(0), f(8), f(4)];
}

/** HSV by the classic sector construction, unlike the shader's closed form. */
function rgbToHsv([r, g, b]: V3): V3 {
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  return [rgbToHsl([r, g, b])[0] * (d === 0 ? 0 : 1), max === 0 ? 0 : d / max, max];
}

function hsvToRgb([h, s, v]: V3): V3 {
  const hh = wrap(h) / 60;
  const i = Math.floor(hh);
  const f = hh - i;
  const p = v * (1 - s);
  const q = v * (1 - s * f);
  const t = v * (1 - s * (1 - f));
  const table: V3[] = [
    [v, t, p],
    [q, v, p],
    [p, v, t],
    [p, q, v],
    [t, p, v],
    [v, p, q],
  ];
  return table[i % 6]!;
}

function hwbToRgb([h, w, b]: V3): V3 {
  if (w + b >= 1) {
    const grey = w / (w + b);
    return [grey, grey, grey];
  }
  return hslToRgb([h, 1, 0.5]).map((c) => c * (1 - w - b) + w) as V3;
}

const E = 216 / 24389;
const K = 24389 / 27;

function xyzToLab(xyz: V3): V3 {
  const f = xyz.map((v, i) => v / D50[i]!).map((v) => (v > E ? Math.cbrt(v) : (K * v + 16) / 116));
  return [116 * f[1]! - 16, 500 * (f[0]! - f[1]!), 200 * (f[1]! - f[2]!)];
}

function labToXyz([L, a, b]: V3): V3 {
  const f1 = (L + 16) / 116;
  const f0 = a / 500 + f1;
  const f2 = f1 - b / 200;
  const xyz: V3 = [
    f0 ** 3 > E ? f0 ** 3 : (116 * f0 - 16) / K,
    L > K * E ? ((L + 16) / 116) ** 3 : L / K,
    f2 ** 3 > E ? f2 ** 3 : (116 * f2 - 16) / K,
  ];
  return xyz.map((v, i) => v * D50[i]!) as V3;
}

const polar = ([L, a, b]: V3, grey: number): V3 => {
  const c = Math.hypot(a, b);
  return [L, c, c > grey ? wrap((Math.atan2(b, a) * 180) / Math.PI) : 0];
};
const rect = ([L, c, h]: V3): V3 => [L, c * Math.cos((h * Math.PI) / 180), c * Math.sin((h * Math.PI) / 180)];

export interface Space {
  slang: string;
  fromLinear: (rgb: V3) => V3;
  toLinear: (c: V3) => V3;
  hue: -1 | 0 | 2;
  grey: (c: V3) => boolean;
}

const enc = (rgb: V3) => rgb.map(gamma) as V3;
const dec = (rgb: V3) => rgb.map(linearize) as V3;
const no = () => false;
const oklab = (rgb: V3): V3 => mul(LMS_TO_OKLAB, mul(XYZ_TO_LMS, mul(SRGB_TO_XYZ, rgb)).map(Math.cbrt) as V3);
const fromOklab = (c: V3): V3 => mul(XYZ_TO_SRGB, mul(LMS_TO_XYZ, mul(OKLAB_TO_LMS, c).map((v) => v ** 3) as V3));
const lab = (rgb: V3) => xyzToLab(mul(D65_TO_D50, mul(SRGB_TO_XYZ, rgb)));
const fromLab = (c: V3) => mul(XYZ_TO_SRGB, mul(D50_TO_D65, labToXyz(c)));

export const SPACES = {
  Srgb: { slang: "Srgb", fromLinear: enc, toLinear: dec, hue: -1, grey: no },
  LinearSrgb: { slang: "LinearSrgb", fromLinear: (c: V3) => c, toLinear: (c: V3) => c, hue: -1, grey: no },
  Hsl: {
    slang: "Hsl",
    fromLinear: (c: V3) => rgbToHsl(enc(c)),
    toLinear: (c: V3) => dec(hslToRgb(c)),
    hue: 0,
    grey: (c: V3) => Math.abs(c[1]) <= 0.001,
  },
  Hsv: {
    slang: "Hsv",
    fromLinear: (c: V3) => rgbToHsv(enc(c)),
    toLinear: (c: V3) => dec(hsvToRgb(c)),
    hue: 0,
    grey: (c: V3) => Math.abs(c[1]) <= 0.001,
  },
  Hwb: {
    slang: "Hwb",
    fromLinear: (c: V3) => {
      const e = enc(c);
      return [rgbToHsl(e)[0], Math.min(...e), 1 - Math.max(...e)] as V3;
    },
    toLinear: (c: V3) => dec(hwbToRgb(c)),
    hue: 0,
    grey: (c: V3) => c[1] + c[2] >= 0.99999,
  },
  XyzD65: {
    slang: "XyzD65",
    fromLinear: (c: V3) => mul(SRGB_TO_XYZ, c),
    toLinear: (c: V3) => mul(XYZ_TO_SRGB, c),
    hue: -1,
    grey: no,
  },
  XyzD50: {
    slang: "XyzD50",
    fromLinear: (c: V3) => mul(D65_TO_D50, mul(SRGB_TO_XYZ, c)),
    toLinear: (c: V3) => mul(XYZ_TO_SRGB, mul(D50_TO_D65, c)),
    hue: -1,
    grey: no,
  },
  Lab: { slang: "Lab", fromLinear: lab, toLinear: fromLab, hue: -1, grey: no },
  Lch: {
    slang: "Lch",
    fromLinear: (c: V3) => polar(lab(c), 0.0015),
    toLinear: (c: V3) => fromLab(rect(c)),
    hue: 2,
    grey: (c: V3) => c[1] <= 0.0015,
  },
  Oklab: { slang: "Oklab", fromLinear: oklab, toLinear: fromOklab, hue: -1, grey: no },
  Oklch: {
    slang: "Oklch",
    fromLinear: (c: V3) => polar(oklab(c), 0.0001),
    toLinear: (c: V3) => fromOklab(rect(c)),
    hue: 2,
    grey: (c: V3) => c[1] <= 0.0001,
  },
} satisfies Record<string, Space>;

export type SpaceName = keyof typeof SPACES;

/** A colour as the reference holds it: linear sRGB and straight alpha, like `Color`. */
export interface Ref {
  linear: V3;
  alpha: number;
  /** The Slang expression that builds the same colour. */
  slang: string;
}

export const from = (space: SpaceName, c: V3, alpha = 1): Ref => ({
  linear: SPACES[space].toLinear(c),
  alpha,
  slang: `Color.from<${space}>(${vec(c)}, ${num(alpha)})`,
});

export const to = (c: Ref, space: SpaceName): V3 => SPACES[space].fromLinear(c.linear);

const withLinear = (c: Ref, linear: V3, alpha = c.alpha): Ref => ({ linear, alpha, slang: "" });

/** colord's clamp, made HDR-safe: never pushes into nor pulls out of 0..1. */
const nudge = (v: number, d: number) => Math.min(Math.max(v + d, Math.min(v, 0)), Math.max(v, 1));

const hslAdjust = (c: Ref, f: (hsl: V3) => V3) => withLinear(c, SPACES.Hsl.toLinear(f(to(c, "Hsl"))));

export const manipulate = {
  lighten: (c: Ref, a: number) => hslAdjust(c, ([h, s, l]) => [h, s, nudge(l, a)]),
  darken: (c: Ref, a: number) => hslAdjust(c, ([h, s, l]) => [h, s, nudge(l, -a)]),
  saturate: (c: Ref, a: number) => hslAdjust(c, ([h, s, l]) => [h, nudge(s, a), l]),
  desaturate: (c: Ref, a: number) => hslAdjust(c, ([h, s, l]) => [h, nudge(s, -a), l]),
  grayscale: (c: Ref) => {
    const l = to(c, "Hsl")[2];
    return withLinear(c, dec([l, l, l]));
  },
  invert: (c: Ref) => withLinear(c, dec(enc(c.linear).map((v) => 1 - v) as V3)),
  rotate: (c: Ref, d: number) => hslAdjust(c, ([h, s, l]) => [h + d, s, l]),
  withHue: (c: Ref, d: number) => hslAdjust(c, ([, s, l]) => [d, s, l]),
  withAlpha: (c: Ref, a: number) => withLinear(c, c.linear, a),
};

export type HueMethod = "Shorter" | "Longer" | "Increasing" | "Decreasing";

/** CSS Color 4 §12 interpolation, as `color-mix(in <space> <hue> hue, a, b t)`. */
export function mix(a: Ref, b: Ref, t: number, space: SpaceName, method: HueMethod = "Shorter"): Ref {
  const s = SPACES[space];
  const ca = s.fromLinear(a.linear);
  const cb = s.fromLinear(b.linear);
  const alpha = a.alpha + (b.alpha - a.alpha) * t;
  const out = [0, 1, 2].map((i) => {
    if (i === s.hue) return 0;
    const v = ca[i]! * a.alpha + (cb[i]! * b.alpha - ca[i]! * a.alpha) * t;
    return alpha > 0 ? v / alpha : v;
  }) as V3;
  const hue: number = s.hue;
  if (hue >= 0) {
    const ga = s.grey(ca);
    const gb = s.grey(cb);
    let h1 = wrap(ga ? cb[hue]! : ca[hue]!);
    let h2 = wrap(gb ? ca[hue]! : cb[hue]!);
    const d = h2 - h1;
    if (method === "Shorter") {
      if (d > 180) h1 += 360;
      else if (d < -180) h2 += 360;
    } else if (method === "Longer") {
      if (0 < d && d < 180) h1 += 360;
      else if (-180 < d && d <= 0) h2 += 360;
    } else if (method === "Increasing") {
      if (h2 < h1) h2 += 360;
    } else if (h1 < h2) h1 += 360;
    out[hue] = ga && gb ? 0 : wrap(h1 + (h2 - h1) * t);
  }
  return withLinear(a, s.toLinear(out), alpha);
}

export const luminance = (c: Ref) => 0.2126 * c.linear[0] + 0.7152 * c.linear[1] + 0.0722 * c.linear[2];

export function contrast(a: Ref, b: Ref): number {
  const la = Math.max(luminance(a), 0) + 0.05;
  const lb = Math.max(luminance(b), 0) + 0.05;
  return Math.max(la, lb) / Math.min(la, lb);
}

export const brightness = (c: Ref) => {
  const [r, g, b] = enc(c.linear);
  return (r * 299 + g * 587 + b * 114) / 1000;
};

export const deltaEOK = (a: Ref, b: Ref) => {
  const [x, y] = [to(a, "Oklab"), to(b, "Oklab")];
  return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
};

/** CIEDE2000 (Sharma, Wu and Dalal 2005), on Lab values. */
export function deltaE2000Lab([L1, a1, b1]: V3, [L2, a2, b2]: V3): number {
  const rad = Math.PI / 180;
  const cBar = (Math.hypot(a1, b1) + Math.hypot(a2, b2)) / 2;
  const g = 0.5 * (1 - Math.sqrt(cBar ** 7 / (cBar ** 7 + 25 ** 7)));
  const ap1 = a1 * (1 + g);
  const ap2 = a2 * (1 + g);
  const c1 = Math.hypot(ap1, b1);
  const c2 = Math.hypot(ap2, b2);
  const h = (b: number, a: number) => (a === 0 && b === 0 ? 0 : wrap(Math.atan2(b, a) / rad));
  const h1 = h(b1, ap1);
  const h2 = h(b2, ap2);
  let dh = 0;
  if (c1 * c2 !== 0) {
    dh = h2 - h1;
    if (dh > 180) dh -= 360;
    else if (dh < -180) dh += 360;
  }
  const dH = 2 * Math.sqrt(c1 * c2) * Math.sin((dh * rad) / 2);
  const lBar = (L1 + L2) / 2;
  const cpBar = (c1 + c2) / 2;
  let hBar = h1 + h2;
  if (c1 * c2 !== 0) {
    if (Math.abs(h1 - h2) <= 180) hBar /= 2;
    else hBar = h1 + h2 < 360 ? (h1 + h2 + 360) / 2 : (h1 + h2 - 360) / 2;
  }
  const t =
    1 -
    0.17 * Math.cos((hBar - 30) * rad) +
    0.24 * Math.cos(2 * hBar * rad) +
    0.32 * Math.cos((3 * hBar + 6) * rad) -
    0.2 * Math.cos((4 * hBar - 63) * rad);
  const sl = 1 + (0.015 * (lBar - 50) ** 2) / Math.sqrt(20 + (lBar - 50) ** 2);
  const sc = 1 + 0.045 * cpBar;
  const sh = 1 + 0.015 * cpBar * t;
  const rt =
    -2 *
    Math.sqrt(cpBar ** 7 / (cpBar ** 7 + 25 ** 7)) *
    Math.sin(2 * 30 * Math.exp(-(((hBar - 275) / 25) ** 2)) * rad);
  const [x, y, z] = [(L2 - L1) / sl, (c2 - c1) / sc, dH / sh];
  return Math.sqrt(x * x + y * y + z * z + rt * y * z);
}

const HARMONIES = {
  complementary: [0, 180],
  analogous: [-30, 0, 30],
  triadic: [0, 120, 240],
  tetradic: [0, 90, 180, 270],
  splitComplementary: [0, 150, 210],
} as const;

/** Published conversions the reference must reproduce: CSS Color 4's worked examples. */
export const CSS_SAMPLES: Array<{ what: string; from: [SpaceName, V3]; to: [SpaceName, V3]; tol: number }> = [
  {
    what: "lch() and lab() of the §2 leaf",
    from: ["Lch", [51.2345, 21.2, 130]],
    to: ["Lab", [51.2345, -13.6271, 16.2401]],
    tol: 1e-4,
  },
  {
    what: "the §2 leaf in sRGB",
    from: ["Lch", [51.2345, 21.2, 130]],
    to: ["Srgb", [0.41587, 0.50367, 0.36664]],
    tol: 5e-5,
  },
  {
    what: "hwb(150 20% 10%) is rgb(20% 90% 55%)",
    from: ["Hwb", [150, 0.2, 0.1]],
    to: ["Srgb", [0.2, 0.9, 0.55]],
    tol: 1e-12,
  },
  {
    what: "hwb(150 20% 10%) is hsl(150 77.78% 55%)",
    from: ["Hwb", [150, 0.2, 0.1]],
    to: ["Hsl", [150, 0.7778, 0.55]],
    tol: 1e-4,
  },
  { what: "hwb(45 40% 80%) is a grey", from: ["Hwb", [45, 0.4, 0.8]], to: ["Srgb", [1 / 3, 1 / 3, 1 / 3]], tol: 1e-12 },
  { what: "sRGB blue in lab()", from: ["Srgb", [0, 0, 1]], to: ["Lab", [29.567, 68.298, -112.0294]], tol: 0.015 },
  { what: "sRGB yellow in lab()", from: ["Srgb", [1, 1, 0]], to: ["Lab", [97.607, -15.753, 93.388]], tol: 0.015 },
  {
    what: "hsl(38.824 100% 50%) is orange",
    from: ["Hsl", [38.824, 1, 0.5]],
    to: ["Srgb", [1, 165 / 255, 0]],
    tol: 1e-4,
  },
  {
    what: "#7654CD in lab()",
    from: ["Srgb", [0x76 / 255, 0x54 / 255, 0xcd / 255]],
    to: ["Lab", [44.35772, 36.0479, -58.9859]],
    tol: 1e-4,
  },
  {
    what: "#7654CD in oklab()",
    from: ["Srgb", [0x76 / 255, 0x54 / 255, 0xcd / 255]],
    to: ["Oklab", [0.5443237, 0.0681665, -0.1656703]],
    tol: 1e-6,
  },
  {
    what: "#7654CD in xyz-d50",
    from: ["Srgb", [0x76 / 255, 0x54 / 255, 0xcd / 255]],
    to: ["XyzD50", [0.2004941, 0.140872, 0.4470838]],
    tol: 1e-6,
  },
  { what: "white in xyz-d50", from: ["Srgb", [1, 1, 1]], to: ["XyzD50", [0.9642957, 1, 0.8251046]], tol: 1e-6 },
  {
    what: "oklch(65% 0.15 270) in lab()",
    from: ["Oklch", [0.65, 0.15, 270]],
    to: ["Lab", [57.9, 11.4, -53.7]],
    tol: 0.06,
  },
  {
    what: "hsl(240 100% 50%) in oklch()",
    from: ["Hsl", [240, 1, 0.5]],
    to: ["Oklch", [0.452, 0.313, 264.1]],
    tol: 0.06,
  },
];

/** CIEDE2000 test pairs from Sharma, Wu and Dalal (2005), table 1. */
export const SHARMA_PAIRS: Array<[V3, V3, number]> = [
  [[50, 2.6772, -79.7751], [50, 0, -82.7485], 2.0425],
  [[50, 2.5, 0], [73, 25, -18], 27.1492],
  [[50, 2.5, 0], [61, -5, 29], 22.8977],
  [[50, 2.5, 0], [56, -27, -3], 31.903],
  [[50, 2.5, 0], [58, 24, 15], 19.4535],
];

const TOL: Record<SpaceName, V3> = {
  Srgb: [2e-4, 2e-4, 2e-4],
  LinearSrgb: [2e-4, 2e-4, 2e-4],
  Hsl: [0.02, 2e-4, 2e-4],
  Hsv: [0.02, 2e-4, 2e-4],
  Hwb: [0.02, 2e-4, 2e-4],
  XyzD65: [2e-4, 2e-4, 2e-4],
  XyzD50: [2e-4, 2e-4, 2e-4],
  Lab: [0.01, 0.01, 0.01],
  Lch: [0.01, 0.01, 0.02],
  Oklab: [2e-4, 2e-4, 2e-4],
  Oklch: [2e-4, 2e-4, 0.05],
};

const swatches: Record<string, Ref> = {
  red: from("Srgb", [1, 0, 0]),
  blue: from("Srgb", [0, 0, 1]),
  white: from("Srgb", [1, 1, 1]),
  black: from("Srgb", [0, 0, 0]),
  grey: from("Srgb", [0.5, 0.5, 0.5]),
  violet: from("Srgb", [0x76 / 255, 0x54 / 255, 0xcd / 255]),
  leaf: from("Lch", [51.2345, 21.2, 130]),
  orange: from("Hsl", [30, 0.8, 0.6], 0.75),
  teal: from("Oklch", [0.7, 0.12, 190], 0.5),
  hdr: from("LinearSrgb", [2.5, 1.4, 0.8]),
  outside: from("LinearSrgb", [-0.08, 0.6, 0.25]),
};

/** Every case the shaders must reproduce. */
export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol: number[], hues?: number[]) =>
    out.push({
      name,
      slang,
      expect,
      tol: tol.map((t, i) => Math.max(t, Math.abs(expect[i]!) * 2e-7)),
      ...(hues ? { hues } : {}),
    });
  const hueOf = (space: SpaceName) => (SPACES[space].hue >= 0 ? [SPACES[space].hue] : undefined);

  for (const [name, c] of Object.entries(swatches)) {
    for (const space of Object.keys(SPACES) as SpaceName[]) {
      if ((name === "hdr" || name === "outside") && SPACES[space].hue === 0) continue;
      add(`${name} in ${space}`, `float4(${c.slang}.to<${space}>(), 0.0)`, to(c, space), TOL[space], hueOf(space));
      if (SPACES[space].hue === 0) {
        const back = SPACES[space].toLinear(to(c, space));
        add(
          `${name} through ${space}`,
          `Color.from<${space}>(${c.slang}.to<${space}>(), 1.0).toLinear()`,
          [...back, 1],
          [...TOL.LinearSrgb, 0],
        );
      }
    }
    add(
      `${name} back from Oklab`,
      `Color.from<Oklab>(${c.slang}.to<Oklab>(), 1.0).toLinear()`,
      [...c.linear, 1],
      [...TOL.LinearSrgb, 0],
    );
    add(
      `${name} back from Lab`,
      `Color.from<Lab>(${c.slang}.to<Lab>(), 1.0).toLinear()`,
      [...c.linear, 1],
      [5e-4, 5e-4, 5e-4, 0],
    );
  }

  add(
    "fromPremultiplied",
    "Color.fromPremultiplied(float4(0.25, 0.1, 0.05, 0.5)).toLinear()",
    [...dec([0.5, 0.2, 0.1]), 0.5],
    [2e-5, 2e-5, 2e-5, 0],
  );
  add(
    "fromPremultiplied of transparent",
    "Color.fromPremultiplied(float4(0.0, 0.0, 0.0, 0.0)).toLinear()",
    [0, 0, 0, 0],
    [0, 0, 0, 0],
  );
  add(
    "fromPremultiplied above SDR white",
    "Color.fromPremultiplied(float4(1.6, 0.8, -0.04, 0.8)).toLinear()",
    [...dec([2, 1, -0.05]), 0.8],
    [1e-4, 2e-5, 2e-5, 0],
  );
  add(
    "fromSrgb extended",
    "Color.fromSrgb(float4(-0.2, 0.5, 1.3, 1.0)).toLinear()",
    [...dec([-0.2, 0.5, 1.3]), 1],
    [2e-5, 2e-5, 2e-5, 0],
  );
  add(
    "fromSrgb(float3)",
    "Color.fromSrgb(float3(0.2, 0.4, 0.6)).toLinear()",
    [...dec([0.2, 0.4, 0.6]), 1],
    [2e-5, 2e-5, 2e-5, 0],
  );
  add(
    "toPremultiplied",
    "Color.fromLinear(float4(0.5, 2.0, -0.1, 0.5)).toPremultiplied()",
    [...enc([0.5, 2, -0.1]).map((v) => v * 0.5), 0.5],
    [2e-5, 2e-5, 2e-5, 0],
  );
  add(
    "toSrgb",
    "Color.fromLinear(float3(0.5, 0.0031308, 0.001)).toSrgb()",
    [...enc([0.5, 0.0031308, 0.001]), 1],
    [2e-5, 2e-5, 2e-5, 0],
  );
  add(
    "fromHex",
    "Color.fromHex(0x7654cdu, 0.5).toSrgb()",
    [0x76 / 255, 0x54 / 255, 0xcd / 255, 0.5],
    [2e-6, 2e-6, 2e-6, 0],
  );
  add(
    "convert<Lab, Oklch>",
    "float4(convert<Lab, Oklch>(float3(57.9, 11.4, -53.7)), 0.0)",
    SPACES.Oklch.fromLinear(SPACES.Lab.toLinear([57.9, 11.4, -53.7])),
    TOL.Oklch,
    [2],
  );
  add(
    "convert<Hsl, Hwb>",
    "float4(convert<Hsl, Hwb>(float3(150.0, 0.7778, 0.55)), 0.0)",
    SPACES.Hwb.fromLinear(SPACES.Hsl.toLinear([150, 0.7778, 0.55])),
    TOL.Hwb,
    [0],
  );

  const srgb = (c: Ref) => [...enc(c.linear), c.alpha];
  const ops: Array<[string, (c: Ref) => Ref]> = [
    ["lighten(0.1)", (c) => manipulate.lighten(c, 0.1)],
    ["lighten(0.8)", (c) => manipulate.lighten(c, 0.8)],
    ["darken(0.2)", (c) => manipulate.darken(c, 0.2)],
    ["saturate(0.2)", (c) => manipulate.saturate(c, 0.2)],
    ["desaturate(0.3)", (c) => manipulate.desaturate(c, 0.3)],
    ["grayscale()", (c) => manipulate.grayscale(c)],
    ["invert()", (c) => manipulate.invert(c)],
    ["rotate(45.0)", (c) => manipulate.rotate(c, 45)],
    ["rotate(-200.0)", (c) => manipulate.rotate(c, -200)],
    ["withHue(200.0)", (c) => manipulate.withHue(c, 200)],
    ["withAlpha(0.25)", (c) => manipulate.withAlpha(c, 0.25)],
  ];
  for (const name of ["violet", "orange", "grey", "leaf"]) {
    const c = swatches[name]!;
    for (const [op, f] of ops) add(`${name}.${op}`, `${c.slang}.${op}.toSrgb()`, srgb(f(c)), [3e-4, 3e-4, 3e-4, 0]);
  }

  const pairs: Array<[string, string, number]> = [
    ["red", "blue", 0.5],
    ["violet", "orange", 0.25],
    ["white", "red", 0.5],
    ["orange", "teal", 0.6],
  ];
  for (const [a, b, t] of pairs) {
    for (const space of Object.keys(SPACES) as SpaceName[]) {
      const m = mix(swatches[a]!, swatches[b]!, t, space);
      add(
        `${a}.mix<${space}>(${b}, ${t})`,
        `${swatches[a]!.slang}.mix<${space}>(${swatches[b]!.slang}, ${num(t)}).toLinear()`,
        [...m.linear, m.alpha],
        [3e-4, 3e-4, 3e-4, 1e-6],
      );
    }
  }
  for (const method of ["Shorter", "Longer", "Increasing", "Decreasing"] as const) {
    for (const space of ["Oklch", "Hsl"] as const) {
      const m = mix(swatches.violet!, swatches.leaf!, 0.3, space, method);
      add(
        `violet.mix<${space}>(leaf, ${method})`,
        `${swatches.violet!.slang}.mix<${space}>(${swatches.leaf!.slang}, 0.3, HueInterpolation.${method}).toLinear()`,
        [...m.linear, m.alpha],
        [3e-4, 3e-4, 3e-4, 1e-6],
      );
    }
  }
  // Hues either side of 0/360, as constants and as values only known at run time.
  const below = from("Oklch", [0.6, 0.1, 359.5]);
  const above = from("Oklch", [0.6, 0.1, 0.5]);
  const live = (c: V3) => `Color.from<Oklch>(${vec(c)} * one, 1.0)`;
  for (const method of ["Shorter", "Longer"] as const) {
    const m = mix(below, above, 0.25, "Oklch", method);
    const expect = [...m.linear, m.alpha];
    const call = (a: string, b: string) => `${a}.mix<Oklch>(${b}, 0.25, HueInterpolation.${method}).toLinear()`;
    add(`across 0/360, ${method}`, call(below.slang, above.slang), expect, [3e-4, 3e-4, 3e-4, 1e-6]);
    add(
      `across 0/360, ${method}, at run time`,
      call(live([0.6, 0.1, 359.5]), live([0.6, 0.1, 0.5])),
      expect,
      [3e-4, 3e-4, 3e-4, 1e-6],
    );
  }
  const dm = mix(swatches.red!, swatches.blue!, 0.5, "Oklab");
  add(
    "red.mix(blue) is Oklab",
    `${swatches.red!.slang}.mix(${swatches.blue!.slang}, 0.5).toLinear()`,
    [...dm.linear, dm.alpha],
    [3e-4, 3e-4, 3e-4, 1e-6],
  );

  for (const [name, c] of Object.entries(swatches)) {
    add(
      `${name} luminance, brightness, isDark, isLight`,
      `float4(${c.slang}.luminance(), ${c.slang}.brightness(), ${c.slang}.isDark() ? 1.0 : 0.0, ${c.slang}.isLight() ? 1.0 : 0.0)`,
      [luminance(c), brightness(c), brightness(c) < 0.5 ? 1 : 0, brightness(c) < 0.5 ? 0 : 1],
      [2e-5, 2e-5, 0, 0],
    );
  }
  for (const [a, b] of [
    ["white", "black"],
    ["violet", "white"],
    ["orange", "teal"],
    ["hdr", "grey"],
    ["outside", "black"],
  ] as const) {
    const ca = swatches[a]!;
    const cb = swatches[b]!;
    add(
      `${a} against ${b}: contrast, deltaEOK, deltaE2000`,
      `float4(${ca.slang}.contrast(${cb.slang}), ${ca.slang}.deltaEOK(${cb.slang}), ${ca.slang}.deltaE2000(${cb.slang}), 0.0)`,
      [contrast(ca, cb), deltaEOK(ca, cb), deltaE2000Lab(to(ca, "Lab"), to(cb, "Lab")), 0],
      [1e-3, 2e-4, 0.01, 0],
    );
  }
  SHARMA_PAIRS.forEach(([l1, l2], i) => {
    const a = from("Lab", l1);
    const b = from("Lab", l2);
    add(
      `Sharma pair ${i + 1}`,
      `float4(${a.slang}.deltaE2000(${b.slang}), 0.0, 0.0, 0.0)`,
      [deltaE2000Lab(l1, l2), 0, 0, 0],
      [5e-3, 0, 0, 0],
    );
    const liveLab = (l: V3) => `Color.from<Lab>(${vec(l)} * one, 1.0)`;
    add(
      `Sharma pair ${i + 1} at run time`,
      `float4(${liveLab(l1)}.deltaE2000(${liveLab(l2)}), 0.0, 0.0, 0.0)`,
      [deltaE2000Lab(l1, l2), 0, 0, 0],
      [5e-3, 0, 0, 0],
    );
  });

  for (const [harmony, shifts] of Object.entries(HARMONIES)) {
    shifts.forEach((shift, i) => {
      const c = swatches.violet!;
      add(
        `violet.${harmony}()[${i}]`,
        `${c.slang}.${harmony}()[${i}].toSrgb()`,
        srgb(shift ? manipulate.rotate(c, shift) : c),
        [3e-4, 3e-4, 3e-4, 0],
      );
    });
  }
  return out;
}
