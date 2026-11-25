// `sk.tone` golden table: the curves, PQ (SMPTE ST 2084) and HLG (BT.2100) in float64, and the cases the
// shaders must reproduce.

import { type Case, live, num, withRunTime } from "./shader-table";

export const IMPORTS = ["sk.tone"];

const LUMA = [0.2126, 0.7152, 0.0722];
const pos = (x: number) => Math.max(x, 0);

export const reinhard = (x: number) => pos(x) / (1 + pos(x));
export const reinhardExtended = (x: number, white: number) => (pos(x) * (1 + pos(x) / (white * white))) / (1 + pos(x));
const hableCurve = (x: number) => {
  const [A, B, C, D, E, F] = [0.15, 0.5, 0.1, 0.2, 0.02, 0.3];
  return (x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F) - E / F;
};
export const hable = (x: number, white = 11.2) => hableCurve(pos(x)) / hableCurve(white);
export const acesFitted = (x: number) => {
  const v = pos(x);
  return Math.min(Math.max((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14), 0), 1);
};
export const softClip = (x: number, knee: number, limit: number) => {
  const span = limit - knee;
  if (span <= 0) return Math.min(x, limit);
  return x <= knee ? x : knee + span * (1 - Math.exp(-(x - knee) / span));
};

const M1 = 2610 / 16384;
const M2 = (2523 / 4096) * 128;
const C1 = 3424 / 4096;
const C2 = (2413 / 4096) * 32;
const C3 = (2392 / 4096) * 32;
export const pqEncode = (nits: number) => {
  const y = Math.min(Math.max(nits / 10000, 0), 1) ** M1;
  return ((C1 + C2 * y) / (1 + C3 * y)) ** M2;
};
export const pqDecode = (signal: number) => {
  const p = Math.min(Math.max(signal, 0), 1) ** (1 / M2);
  return 10000 * (Math.max(p - C1, 0) / (C2 - C3 * p)) ** (1 / M1);
};
const HA = 0.17883277;
const HB = 1 - 4 * HA;
const HC = 0.5 - HA * Math.log(4 * HA);
export const hlgEncode = (e: number) => {
  const v = Math.min(Math.max(e, 0), 1);
  return v <= 1 / 12 ? Math.sqrt(3 * v) : HA * Math.log(12 * v - HB) + HC;
};
export const hlgDecode = (s: number) => {
  const v = Math.min(Math.max(s, 0), 1);
  return v <= 0.5 ? (v * v) / 3 : (Math.exp((v - HC) / HA) + HB) / 12;
};

/** Values the standards and the curves' definitions publish. */
export const KNOWN: Array<{ what: string; got: () => number; want: number; tol: number }> = [
  { what: "PQ of 100 nits", got: () => pqEncode(100), want: 0.5081, tol: 1e-4 },
  { what: "PQ of 1000 nits", got: () => pqEncode(1000), want: 0.7518, tol: 1e-4 },
  { what: "PQ of 10000 nits", got: () => pqEncode(10000), want: 1, tol: 1e-12 },
  { what: "PQ round trip at 203 nits", got: () => pqDecode(pqEncode(203)), want: 203, tol: 1e-9 },
  { what: "HLG at 1/12", got: () => hlgEncode(1 / 12), want: 0.5, tol: 1e-12 },
  { what: "HLG at 1", got: () => hlgEncode(1), want: 1, tol: 1e-6 },
  { what: "HLG inverse at 0.75", got: () => hlgEncode(hlgDecode(0.75)), want: 0.75, tol: 1e-12 },
  { what: "extended Reinhard at its white", got: () => reinhardExtended(4, 4), want: 1, tol: 1e-12 },
  { what: "Hable at its white", got: () => hable(11.2), want: 1, tol: 1e-12 },
];

export function cases(): Case[] {
  const out: Case[] = [];
  const f = (x: number) => live(num(x));
  const F = Math.fround;
  const quad = (name: string, call: (x: string) => string, ref: (x: number) => number, xs: number[], rel = 1e-5) => {
    for (let i = 0; i < xs.length; i += 4) {
      const chunk = xs.slice(i, i + 4);
      while (chunk.length < 4) chunk.push(chunk[chunk.length - 1]!);
      const expect = chunk.map((x) => ref(F(x)));
      out.push({
        name: `${name} at ${chunk}`,
        slang: `float4(${chunk.map((x) => call(f(x))).join(", ")})`,
        expect,
        tol: expect.map((v) => Math.max(2e-6, Math.abs(v) * rel)),
      });
    }
  };
  const light = [-1, 0, 0.01, 0.18, 0.5, 1, 2, 4, 8, 11.2, 40, 1000];

  quad("Reinhard", (x) => `Reinhard().at(${x})`, reinhard, light);
  quad(
    "ReinhardExtended(4)",
    (x) => `ReinhardExtended(4.0).at(${x})`,
    (x) => reinhardExtended(x, 4),
    light,
  );
  quad(
    "Hable()",
    (x) => `Hable().at(${x})`,
    (x) => hable(x),
    light,
  );
  quad(
    "Hable(6)",
    (x) => `Hable(6.0).at(${x})`,
    (x) => hable(x, 6),
    light,
  );
  quad("AcesFitted", (x) => `AcesFitted().at(${x})`, acesFitted, light);
  quad(
    "softClip(x, 1, 4)",
    (x) => `softClip(${x}, 1.0, 4.0)`,
    (x) => softClip(x, 1, 4),
    [...light, -3],
  );
  quad(
    "softClip(x, 2, 1)",
    (x) => `softClip(${x}, 2.0, 1.0)`,
    (x) => softClip(x, 2, 1),
    [-1, 0.5, 1, 3],
  );
  quad(
    "SoftClip(0.5, 2)",
    (x) => `SoftClip(0.5, 2.0).at(${x})`,
    (x) => softClip(x, 0.5, 2),
    light,
  );
  quad(
    "exposure(x, 1.5)",
    (x) => `exposure(${x}, 1.5)`,
    (x) => x * 2 ** 1.5,
    light,
  );
  quad(
    "exposure(x, -2)",
    (x) => `exposure(${x}, -2.0)`,
    (x) => x / 4,
    light,
  );
  quad(
    "sceneToNits(x, 203)",
    (x) => `sceneToNits(${x}, 203.0)`,
    (x) => x * 203,
    light,
  );
  quad(
    "nitsToScene(x, 80)",
    (x) => `nitsToScene(${x}, 80.0)`,
    (x) => x / 80,
    [0, 80, 203, 1000, 10000, -40, 1, 0.5],
  );

  const nits = [-5, 0, 0.005, 0.1, 1, 10, 80, 100, 203, 1000, 4000, 10000, 12000];
  quad("pqEncode", (x) => `pqEncode(float2(${x}, 0.0)).x`, pqEncode, nits, 1e-4);
  const signals = [-0.1, 0, 0.05, 0.25, 0.5, 0.508, 0.58, 0.75, 0.9, 1, 1.2, 0.3];
  quad("pqDecode", (x) => `pqDecode(float2(${x}, 0.0)).x`, pqDecode, signals, 2e-4);
  const scene = [-0.1, 0, 0.01, 1 / 12, 0.1, 0.25, 0.5, 0.75, 1, 1.5, 0.05, 0.3];
  quad("hlgEncode", (x) => `hlgEncode(${x})`, hlgEncode, scene);
  quad("hlgDecode", (x) => `hlgDecode(${x})`, hlgDecode, signals);

  const rgb3 = (r: number, g: number, b: number) => `float3(${f(r)}, ${f(g)}, ${f(b)})`;
  const colours: Array<[number, number, number]> = [
    [0.2, 0.5, 0.9],
    [4, 2, 0.5],
    [12, 0.1, 0.1],
    [-0.2, -0.1, -0.3],
  ];
  const byLuma = (fn: (x: number) => number, c: number[]) => {
    const y = c.reduce((s, v, i) => s + v * LUMA[i]!, 0);
    return y > 0 ? c.map((v) => v * (fn(y) / y)) : [0, 0, 0];
  };
  const tones: Array<[string, (x: number) => number]> = [
    ["Reinhard()", reinhard],
    ["ReinhardExtended(4.0)", (x) => reinhardExtended(x, 4)],
    ["Hable()", (x) => hable(x)],
    ["AcesFitted()", acesFitted],
    ["SoftClip(1.0, 4.0)", (x) => softClip(x, 1, 4)],
  ];
  const push = (name: string, slang: string, expect: number[], rel = 1e-5) =>
    out.push({ name, slang, expect, tol: expect.map((v) => Math.max(2e-6, Math.abs(v) * rel)) });
  for (const [tone, fn] of tones) {
    for (const c of colours) {
      const fc = c.map(F) as [number, number, number];
      push(`byLuminance(${tone}, ${c})`, `float4(byLuminance(${tone}, ${rgb3(...c)}), 0.0)`, [...byLuma(fn, fc), 0]);
      push(`perChannel(${tone}, ${c})`, `float4(perChannel(${tone}, ${rgb3(...c)}), 0.0)`, [...fc.map(fn), 0]);
    }
  }
  for (const c of colours) {
    const fc = c.map(F);
    push(
      `luminance709 and float3 forms at ${c}`,
      `float4(luminance709(${rgb3(...c)}), softClip(${rgb3(...c)}, 1.0, 4.0).y, hlgEncode(${rgb3(...c)} * 0.1).z, pqEncode(${rgb3(...c)} * 100.0).x)`,
      [
        fc.reduce((s, v, i) => s + v * LUMA[i]!, 0),
        softClip(fc[1]!, 1, 4),
        hlgEncode(F(fc[2]! * F(0.1))),
        pqEncode(F(fc[0]! * 100)),
      ],
      1e-4,
    );
  }
  return withRunTime(out);
}
