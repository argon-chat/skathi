// A tiny Lottie (Bodymovin 5.x) builder for hand-authored demo animations. Every animated
// property's last keyframe equals its first, so each animation loops without a seam.

export type V = number | number[];
type Key = { t: number; v: V; ease?: [number, number, number, number] };

const arr = (v: V) => (Array.isArray(v) ? v : [v]);
export const S = (v: V) => ({ a: 0, k: v });
const SMOOTH: [number, number, number, number] = [0.45, 0, 0.55, 1];
const LIN: [number, number, number, number] = [0, 0, 1, 1];

/** Keyframes as [frame, value] pairs, eased in-out unless `linear`. */
export function A(keys: [number, V][], linear = false) {
  const ease = linear ? LIN : SMOOTH;
  const ks: Key[] = keys.map(([t, v]) => ({ t, v, ease }));
  return {
    a: 1,
    k: ks.map((k, i) =>
      i === ks.length - 1
        ? { t: k.t, s: arr(k.v) }
        : {
            t: k.t,
            s: arr(k.v),
            o: { x: [k.ease![0]], y: [k.ease![1]] },
            i: { x: [k.ease![2]], y: [k.ease![3]] },
          },
    ),
  };
}

export const hex = (h: string, a = 1) => {
  const n = parseInt(h.replace("#", ""), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, a];
};

type P = ReturnType<typeof S> | ReturnType<typeof A>;
const p = (v: V | P): P => (typeof v === "object" && !Array.isArray(v) ? (v as P) : S(v as V));

export const tr = (o: { p?: V | P; a?: V | P; s?: V | P; r?: V | P; o?: V | P } = {}) => ({
  ty: "tr",
  p: p(o.p ?? [0, 0]),
  a: p(o.a ?? [0, 0]),
  s: p(o.s ?? [100, 100]),
  r: p(o.r ?? 0),
  o: p(o.o ?? 100),
  sk: S(0),
  sa: S(0),
});

export const group = (items: object[], t: Parameters<typeof tr>[0] = {}, nm = "g") => ({
  ty: "gr",
  nm,
  it: [...items, tr(t)],
});
export const ellipse = (size: V | P, at: V | P = [0, 0]) => ({ ty: "el", p: p(at), s: p(size), d: 1 });
export const rect = (size: V | P, radius: V | P = 0, at: V | P = [0, 0]) => ({
  ty: "rc",
  p: p(at),
  s: p(size),
  r: p(radius),
  d: 1,
});
export const star = (points: number, outer: V | P, inner: V | P, roundness = 0, at: V | P = [0, 0]) => ({
  ty: "sr",
  sy: 1,
  d: 1,
  pt: S(points),
  p: p(at),
  r: S(0),
  ir: p(inner),
  is: S(roundness),
  or: p(outer),
  os: S(roundness),
});
export const path = (v: number[][], i: number[][], o: number[][], closed = true) => ({
  ty: "sh",
  ks: S({ v, i, o, c: closed } as unknown as V),
});
export const fill = (c: number[] | P, opacity: V | P = 100) => ({
  ty: "fl",
  c: Array.isArray(c) ? S(c) : c,
  o: p(opacity),
  r: 1,
});
export const stroke = (c: number[] | P, w: V | P, opacity: V | P = 100) => ({
  ty: "st",
  c: Array.isArray(c) ? S(c) : c,
  o: p(opacity),
  w: p(w),
  lc: 2,
  lj: 2,
  ml: 4,
});
/** Linear (t=1) or radial (t=2) gradient; stops as [offset, hex]. */
export const gradient = (stops: [number, string][], start: number[], end: number[], type: 1 | 2 = 1) => ({
  ty: "gf",
  o: S(100),
  r: 1,
  t: type,
  s: S(start),
  e: S(end),
  g: { p: stops.length, k: S(stops.flatMap(([o, h]) => [o, ...hex(h).slice(0, 3)])) },
});
export const trim = (s: V | P, e: V | P, o: V | P = 0) => ({ ty: "tm", s: p(s), e: p(e), o: p(o), m: 1 });

export function layer(nm: string, shapes: object[], frames: number, ks: Parameters<typeof tr>[0] = {}, ind = 1) {
  const t = tr(ks);
  return {
    ddd: 0,
    ind,
    ty: 4,
    nm,
    sr: 1,
    ks: {
      o: t.o,
      r: t.r,
      p: { ...t.p, k: Array.isArray(t.p.k) && (t.p.k as number[]).length === 2 ? [...(t.p.k as number[]), 0] : t.p.k },
      a: { ...t.a, k: Array.isArray(t.a.k) && (t.a.k as number[]).length === 2 ? [...(t.a.k as number[]), 0] : t.a.k },
      s: t.s,
    },
    ao: 0,
    shapes: [...shapes].reverse(),
    ip: 0,
    op: frames,
    st: 0,
    bm: 0,
  };
}

export function doc(nm: string, frames: number, layers: object[], size = 512, fps = 60) {
  // Lottie draws the first layer on top: callers list back to front, so reverse here.
  const ordered = [...layers].reverse().map((l, i) => ({ ...(l as object), ind: i + 1 }));
  return { v: "5.7.4", fr: fps, ip: 0, op: frames, w: size, h: size, nm, ddd: 0, assets: [], layers: ordered };
}

/** A heart outline centred on 0,0, about `w` wide. */
export function heart(w: number) {
  const k = w / 200;
  const v = [
    [0, 70],
    [-100, -20],
    [-50, -80],
    [0, -40],
    [50, -80],
    [100, -20],
  ].map(([x, y]) => [x! * k, y! * k]);
  const i = [
    [40, -40],
    [0, 40],
    [-35, 0],
    [-15, -25],
    [-15, 0],
    [0, -35],
  ].map(([x, y]) => [x! * k, y! * k]);
  const o = [
    [-40, -40],
    [0, -35],
    [15, 0],
    [15, -25],
    [35, 0],
    [0, 40],
  ].map(([x, y]) => [x! * k, y! * k]);
  return path(v, i, o);
}
