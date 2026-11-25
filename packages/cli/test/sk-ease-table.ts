// `sk.ease` golden table: CSS Easing Level 2 and easings.net written out in float64 (cubic-bezier
// solved by bisection, steps() and linear() as the spec's interval and point rules, a spring by
// numerical integration), and the cases the shader must reproduce.

import { type Case, live, num, vec, withRunTime } from "./shader-table";

export const IMPORTS = ["sk.ease"];

const bez = (p1: number, p2: number, t: number) => 3 * (1 - t) ** 2 * t * p1 + 3 * (1 - t) * t * t * p2 + t ** 3;

/** CSS `cubic-bezier()`, outside 0..1 along the end tangents (§2.2.2). */
export function cubicBezier(x1: number, y1: number, x2: number, y2: number, x: number): number {
  if (x < 0) {
    if (x1 > 0) return (y1 / x1) * x;
    if (x2 > 0) return (y2 / x2) * x;
    return 0;
  }
  if (x > 1) {
    if (x2 < 1) return 1 + ((y2 - 1) / (x2 - 1)) * (x - 1);
    if (x1 < 1) return 1 + ((y1 - 1) / (x1 - 1)) * (x - 1);
    return 1;
  }
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 100; i++) {
    const m = (lo + hi) / 2;
    if (bez(x1, x2, m) < x) lo = m;
    else hi = m;
  }
  return bez(y1, y2, (lo + hi) / 2);
}

export type Position = "JumpStart" | "JumpEnd" | "JumpNone" | "JumpBoth";

/** CSS `steps()` (§2.3.2), the before flag unset: the value of the interval `t` falls in. */
export function steps(n: number, position: Position, t: number): number {
  if (t < 0) return 0;
  if (t >= 1) return 1;
  const values: Record<Position, number[]> = {
    JumpStart: Array.from({ length: n }, (_, i) => (i + 1) / n),
    JumpEnd: Array.from({ length: n }, (_, i) => i / n),
    JumpNone: Array.from({ length: n }, (_, i) => i / (n - 1)),
    JumpBoth: Array.from({ length: n }, (_, i) => (i + 1) / (n + 1)),
  };
  const interval = values[position].findLastIndex((_, i) => t >= i / n);
  return values[position][interval]!;
}

/** CSS `linear()` (§2.1.2), points already carrying their inputs. */
export function linear(raw: Array<[number, number]>, t: number): number {
  let highest = -Infinity;
  const points = raw.map(([x, y]) => {
    highest = Math.max(highest, x);
    return [highest, y] as [number, number];
  });
  if (points.length === 1) return points[0]![1];
  const matches = points.filter(([x]) => x === t);
  if (matches.length) return matches[matches.length - 1]![1];
  let a: [number, number];
  let b: [number, number];
  if (points.every(([x]) => t < x)) {
    [a, b] = [points[0]!, points[1]!];
    if (a[0] === b[0]) return a[1];
  } else if (points.every(([x]) => t > x)) {
    [a, b] = [points[points.length - 2]!, points[points.length - 1]!];
    if (a[0] === b[0]) return b[1];
  } else {
    a = points.filter(([x]) => x < t).pop()!;
    b = points.find(([x]) => x > t)!;
  }
  return a[1] + ((b[1] - a[1]) * (t - a[0])) / (b[0] - a[0]);
}

/** x'' = −ω²(x − 1) − 2ζωx' from rest at 0, by RK4. */
export function spring(duration: number, bounce: number, seconds: number): number {
  const w = (2 * Math.PI) / duration;
  const z = bounce >= 0 ? 1 - bounce : 1 / (1 + bounce);
  const acc = (x: number, v: number) => -w * w * (x - 1) - 2 * z * w * v;
  let x = 0;
  let v = 0;
  const n = 20000;
  const h = seconds / n;
  for (let i = 0; i < n; i++) {
    const k1x = v;
    const k1v = acc(x, v);
    const k2x = v + (h / 2) * k1v;
    const k2v = acc(x + (h / 2) * k1x, v + (h / 2) * k1v);
    const k3x = v + (h / 2) * k2v;
    const k3v = acc(x + (h / 2) * k2x, v + (h / 2) * k2v);
    const k4x = v + h * k3v;
    const k4v = acc(x + h * k3x, v + h * k3v);
    x += (h / 6) * (k1x + 2 * k2x + 2 * k3x + k4x);
    v += (h / 6) * (k1v + 2 * k2v + 2 * k3v + k4v);
  }
  return x;
}

const bounceOut = (t: number) => {
  const n1 = 7.5625;
  const d1 = 2.75;
  if (t < 1 / d1) return n1 * t * t;
  if (t < 2 / d1) return n1 * (t -= 1.5 / d1) * t + 0.75;
  if (t < 2.5 / d1) return n1 * (t -= 2.25 / d1) * t + 0.9375;
  return n1 * (t -= 2.625 / d1) * t + 0.984375;
};

const c1 = 1.70158;
const c2 = c1 * 1.525;
const c3 = c1 + 1;
const c4 = (2 * Math.PI) / 3;
const c5 = (2 * Math.PI) / 4.5;

/** easings.net, verbatim. */
export const PENNER: Record<string, (t: number) => number> = {
  easeInQuad: (x) => x * x,
  easeOutQuad: (x) => 1 - (1 - x) * (1 - x),
  easeInOutQuad: (x) => (x < 0.5 ? 2 * x * x : 1 - (-2 * x + 2) ** 2 / 2),
  easeInCubic: (x) => x ** 3,
  easeOutCubic: (x) => 1 - (1 - x) ** 3,
  easeInOutCubic: (x) => (x < 0.5 ? 4 * x ** 3 : 1 - (-2 * x + 2) ** 3 / 2),
  easeInQuart: (x) => x ** 4,
  easeOutQuart: (x) => 1 - (1 - x) ** 4,
  easeInOutQuart: (x) => (x < 0.5 ? 8 * x ** 4 : 1 - (-2 * x + 2) ** 4 / 2),
  easeInQuint: (x) => x ** 5,
  easeOutQuint: (x) => 1 - (1 - x) ** 5,
  easeInOutQuint: (x) => (x < 0.5 ? 16 * x ** 5 : 1 - (-2 * x + 2) ** 5 / 2),
  easeInSine: (x) => 1 - Math.cos((x * Math.PI) / 2),
  easeOutSine: (x) => Math.sin((x * Math.PI) / 2),
  easeInOutSine: (x) => -(Math.cos(Math.PI * x) - 1) / 2,
  easeInExpo: (x) => (x === 0 ? 0 : 2 ** (10 * x - 10)),
  easeOutExpo: (x) => (x === 1 ? 1 : 1 - 2 ** (-10 * x)),
  easeInOutExpo: (x) => (x === 0 ? 0 : x === 1 ? 1 : x < 0.5 ? 2 ** (20 * x - 10) / 2 : (2 - 2 ** (-20 * x + 10)) / 2),
  easeInCirc: (x) => 1 - Math.sqrt(1 - x ** 2),
  easeOutCirc: (x) => Math.sqrt(1 - (x - 1) ** 2),
  easeInOutCirc: (x) => (x < 0.5 ? (1 - Math.sqrt(1 - (2 * x) ** 2)) / 2 : (Math.sqrt(1 - (-2 * x + 2) ** 2) + 1) / 2),
  easeInBack: (x) => c3 * x ** 3 - c1 * x * x,
  easeOutBack: (x) => 1 + c3 * (x - 1) ** 3 + c1 * (x - 1) ** 2,
  easeInOutBack: (x) =>
    x < 0.5 ? ((2 * x) ** 2 * ((c2 + 1) * 2 * x - c2)) / 2 : ((2 * x - 2) ** 2 * ((c2 + 1) * (x * 2 - 2) + c2) + 2) / 2,
  easeInElastic: (x) => (x === 0 ? 0 : x === 1 ? 1 : -(2 ** (10 * x - 10)) * Math.sin((x * 10 - 10.75) * c4)),
  easeOutElastic: (x) => (x === 0 ? 0 : x === 1 ? 1 : 2 ** (-10 * x) * Math.sin((x * 10 - 0.75) * c4) + 1),
  easeInOutElastic: (x) =>
    x === 0
      ? 0
      : x === 1
        ? 1
        : x < 0.5
          ? -(2 ** (20 * x - 10) * Math.sin((20 * x - 11.125) * c5)) / 2
          : (2 ** (-20 * x + 10) * Math.sin((20 * x - 11.125) * c5)) / 2 + 1,
  easeInBounce: (x) => 1 - bounceOut(1 - x),
  easeOutBounce: bounceOut,
  easeInOutBounce: (x) => (x < 0.5 ? (1 - bounceOut(1 - 2 * x)) / 2 : (1 + bounceOut(2 * x - 1)) / 2),
};

/** Published sample values the reference must reproduce. */
export const KNOWN: Array<{ what: string; got: () => number; want: number; tol: number }> = [
  { what: "ease at 0.5", got: () => cubicBezier(0.25, 0.1, 0.25, 1, 0.5), want: 0.8024033877399112, tol: 1e-9 },
  {
    what: "ease-in-out is symmetric",
    got: () => cubicBezier(0.42, 0, 0.58, 1, 0.3) + cubicBezier(0.42, 0, 0.58, 1, 0.7),
    want: 1,
    tol: 1e-12,
  },
  { what: "easeOutBounce at 0.5", got: () => bounceOut(0.5), want: 0.765625, tol: 1e-12 },
  { what: "steps(4, end) at 0.5", got: () => steps(4, "JumpEnd", 0.5), want: 0.5, tol: 0 },
  { what: "steps(4, jump-none) at 0.5", got: () => steps(4, "JumpNone", 0.5), want: 2 / 3, tol: 1e-15 },
  {
    what: "linear(0, 0.25 75%, 1) at 0.5",
    got: () =>
      linear(
        [
          [0, 0],
          [0.75, 0.25],
          [1, 1],
        ],
        0.5,
      ),
    want: 1 / 6,
    tol: 1e-15,
  },
  {
    what: "a critically damped spring at one period",
    got: () => spring(1, 0, 1),
    want: 1 - Math.exp(-2 * Math.PI) * (1 + 2 * Math.PI),
    tol: 1e-9,
  },
];

const sstep = (x: number) => {
  const t = Math.min(Math.max(x, 0), 1);
  return t * t * (3 - 2 * t);
};
const sstep2 = (x: number) => {
  const t = Math.min(Math.max(x, 0), 1);
  return t ** 3 * (t * (6 * t - 15) + 10);
};
const invert = (fn: (x: number) => number, y: number) => {
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 100; i++) {
    const m = (lo + hi) / 2;
    if (fn(m) < y) lo = m;
    else hi = m;
  }
  return (lo + hi) / 2;
};
const F = Math.fround;
const fmod = (x: number, y: number) => x - y * Math.floor(x / y);

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol = 2e-6) =>
    out.push({ name, slang, expect, tol: expect.map((v) => Math.max(tol, Math.abs(v) * 2e-6)) });
  const f = (x: number) => live(num(x));
  const quad = (name: string, call: (t: string) => string, ref: (t: number) => number, ts: number[], tol?: number) => {
    for (let i = 0; i < ts.length; i += 4) {
      const chunk = ts.slice(i, i + 4);
      while (chunk.length < 4) chunk.push(chunk[chunk.length - 1]!);
      add(
        `${name} at ${chunk}`,
        `float4(${chunk.map((t) => call(f(t))).join(", ")})`,
        chunk.map((t) => ref(Math.fround(t))),
        tol,
      );
    }
  };

  const curves: Array<[string, [number, number, number, number], string]> = [
    ["ease", [0.25, 0.1, 0.25, 1], "CubicBezier.ease()"],
    ["ease-in", [0.42, 0, 1, 1], "CubicBezier.easeIn()"],
    ["ease-out", [0, 0, 0.58, 1], "CubicBezier.easeOut()"],
    ["ease-in-out", [0.42, 0, 0.58, 1], "CubicBezier.easeInOut()"],
    ["back overshoot", [0.68, -0.55, 0.265, 1.55], ""],
    ["steep", [0.9, 0.1, 0.1, 0.9], ""],
    ["x past 0..1 clamps", [-0.5, 0.3, 1.5, 0.7], ""],
    ["flat ends", [0, 0.5, 1, 0.5], ""],
  ];
  const ts = [-0.5, -0.1, 0, 0.02, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 0.98, 1, 1.2];
  for (const [name, [x1, y1, x2, y2], slang] of curves) {
    const call = slang || `CubicBezier(${f(x1)}, ${f(y1)}, ${f(x2)}, ${f(y2)})`;
    const [cx1, cx2] = [Math.min(Math.max(x1, 0), 1), Math.min(Math.max(x2, 0), 1)];
    quad(
      `cubic-bezier ${name}`,
      (t) => `${call}.at(${t})`,
      (t) => cubicBezier(cx1, y1, cx2, y2, t),
      ts,
      1e-5,
    );
  }

  const stepTs = [-0.2, 0, 0.1, 0.24, 0.25, 0.26, 0.5, 0.74, 0.75, 0.99, 1, 1.5];
  for (const position of ["JumpStart", "JumpEnd", "JumpNone", "JumpBoth"] as const) {
    quad(
      `steps(4, ${position})`,
      (t) => `Steps(4, StepPosition.${position}).at(${t})`,
      (t) => steps(4, position, t),
      stepTs,
      1e-6,
    );
  }
  quad(
    "steps(3) defaults to end",
    (t) => `Steps(3).at(${t})`,
    (t) => steps(3, "JumpEnd", t),
    [0.2, 0.34, 0.67, 0.9],
  );
  quad(
    "steps(1, start)",
    (t) => `Steps(1, StepPosition.Start).at(${t})`,
    (t) => steps(1, "JumpStart", t),
    [-0.1, 0, 0.5, 1],
  );

  const stops: Array<[string, Array<[number, number]>]> = [
    [
      "linear(0, 0.25 75%, 1)",
      [
        [0, 0],
        [0.75, 0.25],
        [1, 1],
      ],
    ],
    [
      "a held step",
      [
        [0, 0],
        [0.5, 0.2],
        [0.5, 0.8],
        [1, 1],
      ],
    ],
    [
      "an input going backwards",
      [
        [0, 0],
        [0.6, 0.9],
        [0.4, 0.3],
        [1, 1],
      ],
    ],
    [
      "overshoot",
      [
        [0, 0],
        [0.4, 1.2],
        [0.7, 0.9],
        [1, 1],
      ],
    ],
  ];
  for (const [name, pts] of stops) {
    const arr = `{ ${pts.map((p) => vec(p)).join(", ")} }`;
    quad(
      `LinearStops ${name}`,
      (t) => `LinearStops<${pts.length}>(${arr}).at(${t})`,
      (t) => linear(pts, t),
      [-0.5, 0, 0.2, 0.4, 0.5, 0.55, 0.6, 0.75, 0.9, 1, 1.25, 0.45],
      1e-6,
    );
  }
  add("LinearStops<1>", `float4(LinearStops<1>({ float2(0.3, 0.7) }).at(${f(0.9)}), 0.0, 0.0, 0.0)`, [0.7, 0, 0, 0]);

  for (const [duration, bounce] of [
    [0.5, 0],
    [0.5, 0.3],
    [1, 0.7],
    [0.8, -0.5],
    [0.6, 1],
  ]) {
    quad(
      `Spring(${duration}, ${bounce})`,
      (t) => `Spring(${f(duration!)}, ${f(bounce!)}).at(${t})`,
      (t) => (t <= 0 ? 0 : spring(duration!, bounce!, t)),
      [-0.1, 0, 0.05, 0.15, 0.3, 0.5, 0.8, 1.5],
      2e-5,
    );
  }

  const pennerTs = [-0.25, 0, 0.1, 0.3, 0.45, 0.5, 0.55, 0.7, 0.9, 0.97, 1, 1.4];
  for (const [name, fn] of Object.entries(PENNER)) {
    quad(
      name,
      (t) => `${name}(${t})`,
      (t) => fn(Math.min(Math.max(t, 0), 1)),
      pennerTs,
      1e-5,
    );
  }

  const ys = [-0.1, 0, 0.05, 0.3, 0.5, 0.8, 0.999, 1];
  quad(
    "linearstep(2, 6)",
    (t) => `linearstep(2.0, 6.0, ${t} * 8.0)`,
    (t) => Math.min(Math.max((t * 8 - 2) / 4, 0), 1),
    ys,
  );
  quad(
    "linearstep with equal edges",
    (t) => `linearstep(0.5, 0.5, ${t})`,
    (t) => (t < 0.5 ? 0 : 1),
    ys,
  );
  quad(
    "smootherstep(2, 6)",
    (t) => `smootherstep(2.0, 6.0, ${t} * 8.0)`,
    (t) => sstep2((t * 8 - 2) / 4),
    ys,
  );
  quad(
    "inverseSmoothstep",
    (t) => `inverseSmoothstep(${t})`,
    (y) => invert(sstep, Math.min(Math.max(y, 0), 1)),
    ys,
    1e-5,
  );
  quad(
    "inverseSmootherstep",
    (t) => `inverseSmootherstep(${t})`,
    (y) => invert(sstep2, Math.min(Math.max(y, 0), 1)),
    ys,
    1e-5,
  );

  const times = [-1.3, 0, 0.2, 0.5, 0.9, 1, 1.75, 2.6, 7.25, 1000.4];
  quad(
    "progress(t, 0.5, 2)",
    (t) => `progress(${t}, 0.5, 2.0)`,
    (t) => Math.min(Math.max((t - 0.5) / 2, 0), 1),
    times,
  );
  quad(
    "progress over zero seconds",
    (t) => `progress(${t}, 0.5, 0.0)`,
    (t) => (t < 0.5 ? 0 : 1),
    times,
  );
  quad(
    "loop(t, 0.8)",
    (t) => `loop(${t}, 0.8)`,
    (t) => fmod(t, F(0.8)) / F(0.8),
    times,
    2e-4,
  );
  quad(
    "pingPong(t, 0.8)",
    (t) => `pingPong(${t}, 0.8)`,
    (t) => 1 - Math.abs(2 * (fmod(t, 2 * F(0.8)) / (2 * F(0.8))) - 1),
    times,
    2e-4,
  );
  quad(
    "pulse(t, 2, 0.5)",
    (t) => `pulse(${t}, 2.0, 0.5)`,
    (t) => 1 - sstep(Math.abs(fmod(t, 2) - 0.25) / 0.25),
    [...times, 0.1, 2.3],
    2e-4,
  );
  const ahr = (t: number) => {
    const x = t - 0.2;
    if (x < 0) return 0;
    if (x < 0.3) return x / 0.3;
    if (x < 0.8) return 1;
    if (x < 1.2) return 1 - (x - 0.8) / 0.4;
    return 0;
  };
  quad(
    "attackHoldRelease(t, 0.2, 0.3, 0.5, 0.4)",
    (t) => `attackHoldRelease(${t}, 0.2, 0.3, 0.5, 0.4)`,
    ahr,
    [0, 0.2, 0.35, 0.5, 0.9, 1.0, 1.2, 1.3, 1.4, 2],
    1e-5,
  );
  return withRunTime(out);
}
