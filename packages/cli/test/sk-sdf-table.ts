// `sk.sdf` golden table: distances computed in float64 from each shape's outline (segments and
// circular arcs measured directly, an ellipse by dense search and Newton refinement), never from the
// shader's closed forms, and the cases the shader must reproduce.

import { type Case, live, num, vec, withRunTime } from "./shader-table";

export const IMPORTS = ["sk.sdf"];

/** Structs the cases use as `ISdf` fields. */
export const DECLARATIONS = `
struct SkCaseCircle : ISdf
{
    float2 centre;
    float radius;
    float distance(float2 p) { return Sdf.circle(p - centre, radius); }
};

struct SkCaseBox : ISdf
{
    float2 halfSize;
    float4 radii;
    float distance(float2 p) { return Sdf.roundedBox(p, halfSize, radii); }
};
`;

type P = [number, number];

const sub = (a: P, b: P): P => [a[0] - b[0], a[1] - b[1]];
const dot = (a: P, b: P) => a[0] * b[0] + a[1] * b[1];
const len = (a: P) => Math.hypot(a[0], a[1]);

function segmentDistance(p: P, a: P, b: P): number {
  const ab = sub(b, a);
  const l2 = dot(ab, ab);
  const h = l2 === 0 ? 0 : Math.min(1, Math.max(0, dot(sub(p, a), ab) / l2));
  return len(sub(p, [a[0] + ab[0] * h, a[1] + ab[1] * h]));
}

/** An arc of the circle at `c` of radius `r`, from angle `a0` counter-clockwise (in y-down maths) to `a1`. */
function arcDistance(p: P, c: P, r: number, a0: number, a1: number): number {
  const q = sub(p, c);
  let a = Math.atan2(q[1], q[0]);
  while (a < a0) a += 2 * Math.PI;
  while (a >= a0 + 2 * Math.PI) a -= 2 * Math.PI;
  if (a <= a1) return Math.abs(len(q) - r);
  const end = (t: number): P => [c[0] + r * Math.cos(t), c[1] + r * Math.sin(t)];
  return Math.min(len(sub(p, end(a0))), len(sub(p, end(a1))));
}

function insidePolygon(p: P, vs: P[]): boolean {
  let inside = false;
  for (let i = 0, j = vs.length - 1; i < vs.length; j = i++) {
    const [a, b] = [vs[i]!, vs[j]!];
    if (a[1] > p[1] !== b[1] > p[1] && p[0] < ((b[0] - a[0]) * (p[1] - a[1])) / (b[1] - a[1]) + a[0]) inside = !inside;
  }
  return inside;
}

export function polygon(p: P, vs: P[]): number {
  let d = Infinity;
  for (let i = 0; i < vs.length; i++) d = Math.min(d, segmentDistance(p, vs[i]!, vs[(i + 1) % vs.length]!));
  return insidePolygon(p, vs) ? -d : d;
}

const box = (p: P, h: P) =>
  polygon(p, [
    [-h[0], -h[1]],
    [h[0], -h[1]],
    [h[0], h[1]],
    [-h[0], h[1]],
  ]);

/** Radii in CSS order: top-left, top-right, bottom-right, bottom-left, y down. */
export function roundedBox(p: P, h: P, radii: number[]): number {
  const [tl, tr, br, bl] = radii.map((r) => Math.min(Math.max(r, 0), h[0], h[1])) as [number, number, number, number];
  const [w, v] = h;
  const pieces = [
    segmentDistance(p, [-w + tl, -v], [w - tr, -v]),
    segmentDistance(p, [w, -v + tr], [w, v - br]),
    segmentDistance(p, [w - br, v], [-w + bl, v]),
    segmentDistance(p, [-w, v - bl], [-w, -v + tl]),
    arcDistance(p, [-w + tl, -v + tl], tl, Math.PI, 1.5 * Math.PI),
    arcDistance(p, [w - tr, -v + tr], tr, 1.5 * Math.PI, 2 * Math.PI),
    arcDistance(p, [w - br, v - br], br, 0, 0.5 * Math.PI),
    arcDistance(p, [-w + bl, v - bl], bl, 0.5 * Math.PI, Math.PI),
  ];
  const d = Math.min(...pieces);
  const corners: Array<[P, number, P]> = [
    [[-w + tl, -v + tl], tl, [-1, -1]],
    [[w - tr, -v + tr], tr, [1, -1]],
    [[w - br, v - br], br, [1, 1]],
    [[-w + bl, v - bl], bl, [-1, 1]],
  ];
  let inside = Math.abs(p[0]) <= w && Math.abs(p[1]) <= v;
  for (const [c, r, outward] of corners) {
    const q = sub(p, c);
    if (q[0] * outward[0] > 0 && q[1] * outward[1] > 0 && len(q) > r) inside = false;
  }
  return inside ? -d : d;
}

/** Distance to an ellipse by a dense scan of its perimeter refined with Newton on the angle. */
export function ellipse(p: P, [a, b]: P): number {
  const f = (t: number) => (a * Math.cos(t) - p[0]) ** 2 + (b * Math.sin(t) - p[1]) ** 2;
  let best = 0;
  let bestD = Infinity;
  const n = 20000;
  for (let i = 0; i < n; i++) {
    const t = (i / n) * 2 * Math.PI;
    const d = f(t);
    if (d < bestD) [best, bestD] = [t, d];
  }
  let lo = best - (2 * Math.PI) / n;
  let hi = best + (2 * Math.PI) / n;
  for (let i = 0; i < 200; i++) {
    const m1 = lo + (hi - lo) / 3;
    const m2 = hi - (hi - lo) / 3;
    if (f(m1) < f(m2)) hi = m2;
    else lo = m1;
  }
  const d = Math.sqrt(f((lo + hi) / 2));
  return (p[0] / a) ** 2 + (p[1] / b) ** 2 < 1 ? -d : d;
}

const at = (r: number, a: number): P => [r * Math.sin(a), r * Math.cos(a)];

/** A regular polygon's corners, one straight up (−y), `radius` from the centre. */
export function regularPolygon(radius: number, sides: number): P[] {
  return Array.from({ length: sides }, (_, i) => at(radius, Math.PI + ((2 * Math.PI) / sides) * i));
}

export function starPolygon(radius: number, points: number, inner: number): P[] {
  return Array.from({ length: 2 * points }, (_, i) => at(i % 2 ? inner : radius, Math.PI + (Math.PI / points) * i));
}

export function crossPolygon(l: number, w: number): P[] {
  return [
    [-w, -l],
    [w, -l],
    [w, -w],
    [l, -w],
    [l, w],
    [w, w],
    [w, l],
    [-w, l],
    [-w, w],
    [-l, w],
    [-l, -w],
    [-w, -w],
  ];
}

/** The arc around straight up (−y), `half` radians either side, as an unsigned curve distance. */
function upArc(p: P, r: number, half: number): number {
  return arcDistance(p, [0, 0], r, -Math.PI / 2 - half, -Math.PI / 2 + half);
}

export function pie(p: P, r: number, half: number): number {
  const e1 = at(r, Math.PI - half);
  const e2 = at(r, Math.PI + half);
  const d = Math.min(upArc(p, r, half), segmentDistance(p, [0, 0], e1), segmentDistance(p, [0, 0], e2));
  let off = Math.abs(Math.atan2(p[0], -p[1]));
  if (len(p) === 0) off = 0;
  return len(p) < r && off < half ? -d : d;
}

const circle = (p: P, c: P, r: number) => len(sub(p, c)) - r;

const PROBES: P[] = [
  [0, 0],
  [3, -2],
  [40, 0],
  [0, -16],
  [55, 30],
  [-41, 17],
  [-300, 250],
  [12.5, 7.25],
];

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol = 2e-3) =>
    out.push({ name, slang, expect, tol: expect.map((v) => Math.max(tol, Math.abs(v) * 4e-6)) });
  const pt = (p: P) => live(vec(p));
  const f = (x: number) => live(num(x));
  const distance = (name: string, call: (p: string) => string, ref: (p: P) => number, probes: P[], tol?: number) => {
    for (const p of probes) add(`${name} at ${p}`, `float4(${call(pt(p))}, 0.0, 0.0, 0.0)`, [ref(p), 0, 0, 0], tol);
  };

  distance(
    "circle",
    (p) => `Sdf.circle(${p}, ${f(20)})`,
    (p) => len(p) - 20,
    [...PROBES, [20, 0], [0, 20]],
  );
  distance(
    "box",
    (p) => `Sdf.box(${p}, ${vec([40, 16])})`,
    (p) => box(p, [40, 16]),
    [...PROBES, [40, 16], [40, 10]],
  );
  const radii = [4, 12, 0, 16];
  distance(
    "roundedBox per corner",
    (p) => `Sdf.roundedBox(${p}, ${vec([40, 16])}, ${live(vec(radii))})`,
    (p) => roundedBox(p, [40, 16], radii),
    [...PROBES, [-40, -16], [40, -16], [40, 16], [-40, 16], [-37, -13], [36, -12], [-30, 10]],
  );
  distance(
    "roundedBox uniform",
    (p) => `Sdf.roundedBox(${p}, ${vec([40, 16])}, ${f(10)})`,
    (p) => roundedBox(p, [40, 16], [10, 10, 10, 10]),
    [...PROBES, [40, 16], [33, -9]],
  );
  distance(
    "roundedBox with a radius past half the side",
    (p) => `Sdf.roundedBox(${p}, ${vec([40, 16])}, ${f(30)})`,
    (p) => roundedBox(p, [40, 16], [30, 30, 30, 30]),
    [
      [40, 16],
      [0, 0],
      [30, 30],
    ],
  );
  for (const axes of [
    [60, 20],
    [20, 60],
    [25, 25],
    [400, 40],
  ] as P[]) {
    distance(
      `ellipse ${axes}`,
      (p) => `Sdf.ellipse(${p}, ${vec(axes)})`,
      (p) => ellipse(p, axes),
      [...PROBES, [axes[0], 0], [0, axes[1]], [axes[0] * 0.6, axes[1] * 0.8], [1, 0.5]],
    );
  }
  const [a, b, c]: [P, P, P] = [
    [-30, 20],
    [35, 25],
    [5, -30],
  ];
  distance(
    "segment",
    (p) => `Sdf.segment(${p}, ${vec(a)}, ${vec(b)})`,
    (p) => segmentDistance(p, a, b),
    [...PROBES, a, b],
  );
  distance(
    "capsule",
    (p) => `Sdf.capsule(${p}, ${vec(a)}, ${vec(b)}, ${f(6)})`,
    (p) => segmentDistance(p, a, b) - 6,
    PROBES,
  );
  distance(
    "segment of zero length",
    (p) => `Sdf.segment(${p}, ${vec(a)}, ${vec(a)})`,
    (p) => len(sub(p, a)),
    PROBES.slice(0, 3),
  );
  distance(
    "triangle",
    (p) => `Sdf.triangle(${p}, ${pt(a)}, ${pt(b)}, ${pt(c)})`,
    (p) => polygon(p, [a, b, c]),
    [...PROBES, a, b, c],
  );
  distance(
    "triangle, other winding",
    (p) => `Sdf.triangle(${p}, ${vec(a)}, ${vec(c)}, ${vec(b)})`,
    (p) => polygon(p, [a, b, c]),
    PROBES.slice(0, 5),
  );
  for (const sides of [3, 5, 6]) {
    const vs = regularPolygon(30, sides);
    distance(
      `polygon(${sides})`,
      (p) => `Sdf.polygon(${p}, ${f(30)}, ${sides})`,
      (p) => polygon(p, vs),
      [...PROBES, vs[0]!, [0, -30], [0, 31]],
    );
  }
  for (const inner of [12, 24]) {
    const vs = starPolygon(30, 5, inner);
    distance(
      `star(5, ${inner})`,
      (p) => `Sdf.star(${p}, ${f(30)}, 5, ${f(inner)})`,
      (p) => polygon(p, vs),
      [...PROBES, vs[0]!, vs[1]!, [0, -30]],
    );
  }
  for (const half of [0.6, 2.2]) {
    distance(
      `arc(${half})`,
      (p) => `Sdf.arc(${p}, ${f(25)}, ${f(half)}, ${f(4)})`,
      (p) => upArc(p, 25, half) - 2,
      [...PROBES, [0, -25], at(25, Math.PI - half), [0, 25]],
    );
    distance(
      `pie(${half})`,
      (p) => `Sdf.pie(${p}, ${f(25)}, ${f(half)})`,
      (p) => pie(p, 25, half),
      [...PROBES, [0, -25], at(25, Math.PI - half), [0, 25], [0, -10]],
    );
  }
  distance(
    "ring",
    (p) => `Sdf.ring(${p}, ${f(20)}, ${f(6)})`,
    (p) => Math.abs(len(p) - 20) - 3,
    [...PROBES, [23, 0]],
  );
  distance(
    "halfPlane",
    (p) => `Sdf.halfPlane(${p}, ${vec([5, 5])}, ${pt([3, 4])})`,
    (p) => (3 * (p[0] - 5) + 4 * (p[1] - 5)) / 5,
    PROBES,
  );
  const cross = crossPolygon(30, 8);
  distance(
    "cross",
    (p) => `Sdf.cross(${p}, ${f(30)}, ${f(8)})`,
    (p) => polygon(p, cross),
    [...PROBES, [8, 8], [9, 9], [30, 8], [20, 20]],
  );

  const pairs: P[] = [
    [-3, 5],
    [4, 4.5],
    [10, -2],
    [-1, -7],
    [2, 2],
  ];
  const smin = (x: number, y: number, k: number) => {
    const h = Math.max(k - Math.abs(x - y), 0) / k;
    return Math.min(x, y) - (h * h * k) / 4;
  };
  for (const [x, y] of pairs) {
    const xs = `${f(x)}, ${f(y)}`;
    add(
      `union etc of ${x}, ${y}`,
      `float4(Sdf.union(${xs}), Sdf.intersection(${xs}), Sdf.subtraction(${xs}), Sdf.xor(${xs}))`,
      [Math.min(x, y), Math.max(x, y), Math.max(x, -y), Math.max(Math.min(x, y), -Math.max(x, y))],
    );
    add(
      `smooth ops of ${x}, ${y}`,
      `float4(Sdf.smoothUnion(${xs}, ${f(4)}), Sdf.smoothIntersection(${xs}, ${f(4)}), Sdf.smoothSubtraction(${xs}, ${f(4)}), Sdf.smoothUnion(${xs}, 0.0))`,
      [smin(x, y, 4), -smin(-x, -y, 4), -smin(-x, y, 4), Math.min(x, y)],
      1e-5,
    );
  }
  add(
    "smoothUnion pulls an equal seam in by radius / 4",
    `float4(Sdf.smoothUnion(${f(3)}, ${f(3)}, ${f(8)}), 0.0, 0.0, 0.0)`,
    [1, 0, 0, 0],
    1e-5,
  );

  const disc = (centre: P, r: number) => (p: P) => circle(p, centre, r);
  const moved: Array<[string, (p: string) => string, (p: P) => number, P[]]> = [
    ["translate", (p) => `Sdf.circle(Sdf.translate(${p}, ${pt([10, -5])}), 6.0)`, disc([10, -5], 6), PROBES],
    [
      "rotate a quarter turn moves +x to +y",
      (p) => `Sdf.circle(Sdf.rotate(${p}, ${f(Math.PI / 2)}) - float2(20.0, 0.0), 5.0)`,
      disc([0, 20], 5),
      [...PROBES, [0, 20]],
    ],
    [
      "mirror across x",
      (p) => `Sdf.circle(Sdf.mirror(${p}, float2(1.0, 0.0)) - float2(15.0, 3.0), 5.0)`,
      (p) => Math.min(circle(p, [15, 3], 5), circle(p, [-15, 3], 5)),
      PROBES,
    ],
    [
      "repeat endless",
      (p) => `Sdf.circle(Sdf.repeat(${p}, ${pt([30, 20])}), 6.0)`,
      (p) => {
        let d = Infinity;
        for (let i = -20; i <= 20; i++) for (let j = -20; j <= 20; j++) d = Math.min(d, circle(p, [30 * i, 20 * j], 6));
        return d;
      },
      [...PROBES, [-15, 10]],
    ],
    [
      "repeat 3 x 2",
      (p) => `Sdf.circle(Sdf.repeat(${p}, ${pt([30, 20])}, int2(3, 2)), 6.0)`,
      (p) => {
        let d = Infinity;
        for (const i of [-1, 0, 1]) for (const j of [-0.5, 0.5]) d = Math.min(d, circle(p, [30 * i, 20 * j], 6));
        return d;
      },
      PROBES,
    ],
    [
      "polarRepeat 6",
      (p) => `Sdf.circle(Sdf.polarRepeat(${p}, 6) - float2(0.0, -20.0), 4.0)`,
      (p) => Math.min(...Array.from({ length: 6 }, (_, i) => circle(p, at(20, Math.PI + (i * Math.PI) / 3), 4))),
      [...PROBES, [0, 20], at(20, Math.PI / 3)],
    ],
    ["scaled", (p) => `Sdf.scaled(SkCaseCircle(float2(5.0, 0.0), 4.0), ${p}, ${f(2.5)})`, disc([12.5, 0], 10), PROBES],
  ];
  for (const [name, call, ref, probes] of moved) distance(name, call, ref, probes);

  for (const d of [-3, -1.5, -0.5, 0, 0.25, 1, 2.5]) {
    const stroke = (lo: number, hi: number) => Math.min(Math.max(Math.min(d - lo, hi - d) + 0.5, 0), 1);
    add(
      `coverage and strokes at ${d}`,
      `float4(Sdf.coverage(${f(d)}), Sdf.stroke(${f(d)}, ${f(2)}, StrokeAlign.Inside), Sdf.stroke(${f(d)}, ${f(2)}), Sdf.stroke(${f(d)}, ${f(2)}, StrokeAlign.Outside))`,
      [Math.min(Math.max(0.5 - d, 0), 1), stroke(-2, 0), stroke(-1, 1), stroke(0, 2)],
      1e-6,
    );
    const ss = (x: number) => (x <= 0 ? 0 : x >= 1 ? 1 : x * x * (3 - 2 * x));
    const g = 1 - Math.min(Math.max(Math.max(d, 0) / 2, 0), 1);
    add(
      `feather and glow at ${d}`,
      `float4(Sdf.feather(${f(d)}, ${f(4)}), Sdf.glow(${f(d)}, ${f(2)}), Sdf.inflate(${f(d)}, ${f(1.5)}), Sdf.outline(${f(d)}, ${f(3)}))`,
      [1 - ss((d + 2) / 4), g * g, d - 1.5, Math.abs(d) - 1.5],
      1e-6,
    );
  }

  const numeric = (fn: (p: P) => number, p: P): P => {
    const h = 1e-5;
    const g: P = [
      (fn([p[0] + h, p[1]]) - fn([p[0] - h, p[1]])) / (2 * h),
      (fn([p[0], p[1] + h]) - fn([p[0], p[1] - h])) / (2 * h),
    ];
    return g;
  };
  const r4 = [4, 12, 0, 16];
  for (const p of [
    [3, -2],
    [55, 30],
    [-41, 17],
    [-38, -15],
    [10, 2],
    [0, 14],
  ] as P[]) {
    const rb = numeric((q) => roundedBox(q, [40, 16], r4), p);
    const bx = numeric((q) => box(q, [40, 16]), p);
    add(
      `gradients at ${p}`,
      `float4(Sdf.roundedBoxGradient(${pt(p)}, ${vec([40, 16])}, ${vec(r4)}), Sdf.boxGradient(${pt(p)}, ${vec([40, 16])}))`,
      [...rb, ...bx],
      1e-4,
    );
    const cg = numeric((q) => circle(q, [5, 0], 4), p);
    add(
      `normal of a field at ${p}`,
      `float4(Sdf.normal(SkCaseCircle(float2(5.0, 0.0), 4.0), ${pt(p)}, 0.05), Sdf.circleGradient(${pt(p)} - float2(5.0, 0.0)))`,
      [...cg, ...cg],
      2e-4,
    );
    add(
      `gradient of a box field at ${p}`,
      `float4(Sdf.gradient(SkCaseBox(${vec([40, 16])}, ${vec(r4)}), ${pt(p)}, 0.01), 0.0, 0.0)`,
      [...rb, 0, 0],
      5e-3,
    );
  }
  return withRunTime(out);
}
