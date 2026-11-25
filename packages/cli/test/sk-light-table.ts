// `sk.light` golden table: closed forms in float64 (Fresnel's equations in their angle form, Snell's law as a
// tangent construction, erf by its series, bevel normals from the numeric slope of the height profile) and the
// cases the shaders must reproduce.

import { type Case, live, num, tableEffect, vec, withRunTime } from "./shader-table";

type V2 = [number, number];
type V3 = [number, number, number];

const dot = (a: number[], b: number[]) => a.reduce((s, v, i) => s + v * b[i]!, 0);
const norm = <T extends number[]>(v: T): T => {
  const l = Math.hypot(...v);
  return v.map((x) => x / l) as T;
};
const clamp01 = (x: number) => Math.min(Math.max(x, 0), 1);
const VIEW: V3 = [0, 0, 1];

export const f0FromIor = (ior: number, outside = 1) => ((outside - ior) / (outside + ior)) ** 2;
export const schlick = (c: number, f0: number) => f0 + (1 - f0) * (1 - clamp01(c)) ** 5;

/** Fresnel's equations in their angle form: Rs = sin²(i - t) / sin²(i + t), Rp = tan²(i - t) / tan²(i + t). */
export function fresnel(cosI: number, eta: number): number {
  const i = Math.acos(clamp01(cosI));
  const s = Math.sin(i) / eta;
  if (s >= 1) return 1;
  const t = Math.asin(s);
  if (i === 0) return ((eta - 1) / (eta + 1)) ** 2;
  const rs = Math.sin(i - t) ** 2 / Math.sin(i + t) ** 2;
  const rp = Math.tan(i - t) ** 2 / Math.tan(i + t) ** 2;
  return (rs + rp) / 2;
}

/** Snell's law as a construction: the tangential part scales by eta, the normal part fills the rest. */
export function refract(incident: V3, normal: V3, eta: number): V3 {
  const c = dot(incident, normal);
  const tangential = incident.map((v, k) => (v - c * normal[k]!) * eta) as V3;
  const t2 = dot(tangential, tangential);
  if (t2 > 1) return [0, 0, 0];
  const along = Math.sqrt(1 - t2);
  return tangential.map((v, k) => v - along * normal[k]!) as V3;
}

export const refractionOffset = (n: V3, thickness: number, ior: number): V2 => {
  const t = refract([0, 0, -1], n, 1 / ior);
  return [(t[0] / -t[2]) * thickness, (t[1] / -t[2]) * thickness];
};

type Profile = "Circle" | "Squircle" | "Linear";
export function bevelHeight(sd: number, width: number, profile: Profile): number {
  const s = clamp01(-sd / width);
  const u = 1 - s;
  if (profile === "Circle") return Math.sqrt(1 - u * u);
  if (profile === "Squircle") return (1 - u ** 4) ** 0.25;
  return s;
}

/** The normal of the surface z = height * width * bevelHeight, from its numeric slope across the rim. */
export function bevelNormal(sd: number, gradient: V2, width: number, profile: Profile, height = 1): V3 {
  const z = (d: number) => height * width * bevelHeight(d, width, profile);
  const h = 1e-6 * width;
  const slope = (z(sd - h) - z(sd + h)) / (2 * h);
  const out = norm([...gradient] as V2);
  return norm([out[0] * slope, out[1] * slope, 1] as V3);
}

/** erf by its Maclaurin series (|x| < 3) and continued fraction beyond, to 1e-12. */
export function erf(x: number): number {
  const a = Math.abs(x);
  let r: number;
  if (a < 3) {
    let term = a;
    let sum = a;
    for (let n = 1; n < 80; n++) {
      term *= (-a * a) / n;
      sum += term / (2 * n + 1);
    }
    r = (2 / Math.sqrt(Math.PI)) * sum;
  } else {
    let f = 0;
    for (let n = 60; n >= 1; n--) f = n / 2 / (a + f);
    r = 1 - Math.exp(-a * a) / Math.sqrt(Math.PI) / (a + f);
  }
  return Math.sign(x) * r;
}
const phi = (d: number, sigma: number) => 0.5 + 0.5 * erf(d / sigma / Math.SQRT2);

export function ggx(n: V3, l: V3, roughness: number, f0: number, v: V3 = VIEW): number {
  const h = norm(l.map((x, k) => x + v[k]!) as V3);
  const nl = clamp01(dot(n, l));
  const nv = Math.max(dot(n, v), 1e-4);
  const nh = clamp01(dot(n, h));
  const a = Math.max(roughness * roughness, 0.002);
  const a2 = a * a;
  const D = a2 / (Math.PI * (nh * nh * (a2 - 1) + 1) ** 2);
  const V = 0.5 / (nl * Math.sqrt(nv * nv * (1 - a2) + a2) + nv * Math.sqrt(nl * nl * (1 - a2) + a2));
  return D * V * schlick(dot(l, h), f0) * nl;
}

const blinn = (n: V3, l: V3, s: number, v: V3 = VIEW) => clamp01(dot(n, norm(l.map((x, k) => x + v[k]!) as V3))) ** s;
const kajiya = (n: V3, l: V3, t: V3, e: number) => {
  const th = dot(t, norm(l.map((x, k) => x + VIEW[k]!) as V3));
  return Math.sqrt(clamp01(1 - th * th)) ** e * clamp01(dot(n, l));
};

const linearize = (v: number) =>
  Math.abs(v) <= 0.04045 ? v / 12.92 : Math.sign(v) * ((Math.abs(v) + 0.055) / 1.055) ** 2.4;

const NORMALS: V3[] = [norm([0.3, -0.2, 0.93]), norm([-0.6, 0.1, 0.5]), [0, 0, 1], norm([0.05, 0.7, 0.35])];
const LIGHTS: V3[] = [norm([-0.5, -0.7, 0.6]), norm([0.2, 0.1, 1]), norm([1, -0.3, 0.2])];

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (name: string, slang: string, expect: number[], tol: number[]) => out.push({ name, slang, expect, tol });
  const v = (x: number[]) => live(vec(x));
  const s = (x: number) => live(num(x));
  const t4 = (t: number) => [t, t, t, t];

  for (const ior of [1, 1.33, 1.5, 2.4])
    add(
      `f0FromIor(${ior}), and from water`,
      `float4(f0FromIor(${s(ior)}), f0FromIor(${s(ior)}, 1.33), 0.0, 0.0)`,
      [f0FromIor(ior), f0FromIor(ior, 1.33), 0, 0],
      t4(1e-7),
    );
  for (const c of [0, 0.2, 0.5, 0.87, 1]) {
    add(
      `Fresnel at cos ${c}`,
      `float4(fresnelSchlick(${s(c)}, 0.04), fresnelDielectric(${s(c)}, 1.5), fresnelDielectric(${s(c)}, 1.0 / 1.5), fresnelDielectric(${s(c)}, 1.33))`,
      [schlick(c, 0.04), fresnel(c, 1.5), fresnel(c, 1 / 1.5), fresnel(c, 1.33)],
      t4(2e-6),
    );
  }
  for (const n of NORMALS) {
    for (const eta of [1 / 1.5, 1.5, 1 / 1.33]) {
      const inc = norm([0.3, 0.2, -1] as V3);
      add(
        `refractDirection through ${n.map((x) => x.toFixed(2))}, eta ${eta.toFixed(3)}`,
        `float4(refractDirection(${v(inc)}, ${v(n)}, ${s(eta)}), 0.0)`,
        [...refract(inc, n, eta), 0],
        t4(2e-6),
      );
    }
    add(
      `refractionOffset of ${n.map((x) => x.toFixed(2))}`,
      `float4(refractionOffset(${v(n)}, ${s(12)}, 1.5), refractionOffset(${v(n)}, 20.0, 1.33))`,
      [...refractionOffset(n, 12, 1.5), ...refractionOffset(n, 20, 1.33)],
      t4(2e-5),
    );
  }
  add(
    "total internal reflection is zero",
    `float4(refractDirection(${v(norm([0.9, 0, -0.2]))}, float3(0.0, 0.0, 1.0), ${s(1.5)}), 0.0)`,
    [0, 0, 0, 0],
    t4(0),
  );

  const grads: V2[] = [
    [1, 0],
    [0.6, -0.8],
    [-3, 4],
  ];
  for (const profile of ["Circle", "Squircle", "Linear"] as Profile[]) {
    for (const sd of [-0.5, -6, -13.7, -23, -30]) {
      for (const g of grads.slice(0, profile === "Circle" ? 3 : 2)) {
        const want = bevelNormal(sd, g, 24, profile, profile === "Squircle" ? 0.6 : 1);
        add(
          `bevelNormal ${profile} at ${sd} along ${g}`,
          `float4(bevelNormal(${s(sd)}, ${v(g)}, ${s(24)}, BevelProfile.${profile}, ${profile === "Squircle" ? "0.6" : "1.0"}), bevelHeight(${s(sd)}, 24.0, BevelProfile.${profile}))`,
          [...want, bevelHeight(sd, 24, profile)],
          t4(2e-4),
        );
      }
    }
  }
  add(
    "bevelNormal at the edge is horizontal",
    `float4(bevelNormal(${s(0)}, ${v([0, -2])}, 24.0), 0.0)`,
    [0, -1, 0, 0],
    t4(1e-6),
  );
  add(
    "bevelNormal outside the shape is the edge's",
    `float4(bevelNormal(${s(5)}, ${v([2, 0])}, 24.0), 0.0)`,
    [1, 0, 0, 0],
    t4(1e-6),
  );
  add(
    "bevelNormal without a gradient faces the viewer",
    `float4(bevelNormal(${s(-3)}, ${v([0, 0])}, 24.0), 0.0)`,
    [0, 0, 1, 0],
    t4(1e-6),
  );

  for (const slope of [
    [0.1, 0.2],
    [-2, 0.5],
  ] as V2[]) {
    add(
      `normalFromSlope(${slope})`,
      `float4(normalFromSlope(${v(slope)}, 1.5), 0.0)`,
      [...norm([-slope[0] * 1.5, -slope[1] * 1.5, 1] as V3), 0],
      t4(1e-6),
    );
  }
  // The dome's analytic gradient; central differences one pixel apart differ by O(h^2).
  for (const p of [
    [40, 30],
    [70, 52],
  ] as V2[]) {
    const c: V2 = [50, 40];
    const e = Math.exp(-((p[0] - c[0]) ** 2 + (p[1] - c[1]) ** 2) * 0.001) * 20;
    const slope: V2 = [-0.002 * (p[0] - c[0]) * e, -0.002 * (p[1] - c[1]) * e];
    add(
      `heightNormal of a dome at ${p}`,
      `float4(heightNormal(sk_dome(), ${v(p)}), 0.0)`,
      [...norm([-slope[0], -slope[1], 1] as V3), 0],
      t4(1e-3),
    );
  }
  add(
    "unpackNormal y-down and y-up",
    `float4(unpackNormal(${v([0.75, 0.25, 0.9, 1])}).xy, unpackNormal(float4(0.75, 0.25, 0.9, 1.0), true).xy)`,
    (() => {
      const n = norm([0.5, -0.5, 0.8] as V3);
      return [n[0], n[1], n[0], -n[1]];
    })(),
    t4(1e-6),
  );
  add(
    "packNormal",
    `packNormal(${v(norm([0.3, -0.2, 0.93]))})`,
    [...norm([0.3, -0.2, 0.93] as V3).map((x) => x * 0.5 + 0.5), 1],
    t4(1e-6),
  );

  for (const n of NORMALS) {
    for (const l of LIGHTS) {
      add(
        `diffuse terms, n ${n.map((x) => x.toFixed(2))}, l ${l.map((x) => x.toFixed(2))}`,
        `float4(lambert(${v(n)}, ${v(l)}), wrapLambert(${v(n)}, ${v(l)}, 0.5), lobe(${v(n)}, ${v(l)}, 12.0), rim(${v(n)}, 3.0))`,
        [clamp01(dot(n, l)), clamp01((dot(n, l) + 0.5) / 1.5), clamp01(dot(n, l)) ** 12, (1 - clamp01(n[2])) ** 3],
        t4(2e-6),
      );
      add(
        `specular terms, n ${n.map((x) => x.toFixed(2))}, l ${l.map((x) => x.toFixed(2))}`,
        `float4(blinnPhong(${v(n)}, ${v(l)}, 40.0), ggx(${v(n)}, ${v(l)}, 0.35, 0.04), ggx(${v(n)}, ${v(l)}, 0.8, 0.9), anisotropic(${v(n)}, ${v(l)}, float3(1.0, 0.0, 0.0), 20.0))`,
        [blinn(n, l, 40), ggx(n, l, 0.35, 0.04), ggx(n, l, 0.8, 0.9), kajiya(n, l, [1, 0, 0], 20)],
        [2e-5, 2e-5 * Math.max(1, ggx(n, l, 0.35, 0.04)), 2e-5, 2e-5],
      );
    }
  }
  const mirror = ggx([0, 0, 1], [0, 0, 1], 0, 0.04);
  add(
    "ggx of a mirror facing its light",
    `float4(ggx(float3(0.0, 0.0, 1.0), ${v([0, 0, 1])}, 0.0, 0.04), 0.0, 0.0, 0.0)`,
    [mirror, 0, 0, 0],
    [mirror * 2e-5, 0, 0, 0],
  );

  for (const sd of [-20, -6, -1.5, 0, 0.7, 4, 15]) {
    for (const blur of [0, 4, 10]) {
      const hard = clamp01(0.5 - sd);
      add(
        `shadows at ${sd} px, blur ${blur}`,
        `float4(dropShadow(${s(sd)}, ${s(blur)}), innerShadow(${s(sd)}, ${s(blur)}), edgeOcclusion(${s(sd)}, 6.0), 0.0)`,
        [
          blur ? phi(-sd, blur / 2) : hard,
          blur ? phi(sd, blur / 2) : clamp01(0.5 + sd),
          Math.exp(Math.min(sd, 0) / 6),
          0,
        ],
        t4(1e-6),
      );
    }
  }

  const sun = norm([-1, -1, 1] as V3);
  add(
    "Light.directional",
    `float4(Light.directional(${v([-1, -1, 1])}, 2.0).towards(float3(5.0, 5.0, 0.0)), Light.directional(float3(-1.0, -1.0, 1.0), 2.0).radiance().g)`,
    [...sun, 2],
    t4(1e-6),
  );
  const lamp: V3 = [40, 10, 60];
  const at: V3 = [10, 30, 2];
  add(
    "Light.point towards a surface point",
    `float4(Light.point(${v(lamp)}, Color.fromHex(0xff8000u), 1.5).towards(${v(at)}), Light.point(float3(40.0, 10.0, 60.0), Color.fromHex(0xff8000u), 1.5).radiance().g)`,
    [...norm(lamp.map((x, k) => x - at[k]!) as V3), linearize(0x80 / 255) * 1.5],
    t4(1e-5),
  );
  for (const n of NORMALS.slice(0, 3)) {
    const off = refractionOffset(n, 14, 1.45);
    const spec = ggx(n, sun, 0.25, f0FromIor(1.45)) * 2;
    add(
      `Surface of ${n.map((x) => x.toFixed(2))}`,
      `sk_surface_case(${v(n)})`,
      [...off, schlick(n[2], f0FromIor(1.45)), spec],
      [2e-5, 2e-5, 1e-6, 2e-5 * Math.max(1, spec)],
    );
  }

  const px = [0.3, 0.2, 0.1, 0.5];
  add(
    "addLight to a premultiplied pixel",
    `addLight(${v(px)}, ${s(0.25)})`,
    [0.55, 0.45, 0.35, 0.5 + 0.25 * 0.5],
    t4(1e-6),
  );
  {
    const c = { linear: [0.2, 0.4, 0.05], alpha: 0.6 };
    const light = [0.3, 0.1, 0.5];
    const a = c.alpha + 0.5 * (1 - c.alpha);
    add(
      "addLight to a Color, in light",
      `addLight(Color.fromLinear(${v([...c.linear, c.alpha])}), ${v(light)}).toLinear()`,
      [...c.linear.map((x, k) => (x * c.alpha + light[k]!) / a), a],
      t4(1e-6),
    );
    add(
      "addLight to transparent black stays finite",
      `addLight(Color.fromLinear(${v([0, 0, 0, 0])}), float3(0.0, 0.0, 0.0)).toLinear()`,
      [0, 0, 0, 0],
      t4(0),
    );
  }
  const up = v([0, 0, 1]);
  add(
    "highlights where the base of the power is 0",
    `float4(rim(${up}, ${live("3.0")}), lobe(${up}, ${v([1, 0, 0])}, ${live("8.0")}), lobe(${up}, ${v([0, 0, -1])}, ${live("0.0")}), anisotropic(${up}, ${up}, ${up}, ${live("40.0")}))`,
    [0, 0, 1, 0],
    t4(0),
  );
  add(
    "squircle bevel normal on the edge",
    `float4(bevelNormal(${live("0.0")}, ${v([1, 0])}, ${live("10.0")}, BevelProfile.Squircle), 0.0)`,
    [1, 0, 0, 0],
    t4(1e-6),
  );
  return withRunTime(out);
}

export const DECLARATIONS = `
struct SkDome : IHeightField
{
    float height(float2 p) { float2 d = p - float2(50.0, 40.0); return exp(-dot(d, d) * 0.001) * 20.0; }
};
SkDome sk_dome() { SkDome d; return d; }
float4 sk_surface_case(float3 n)
{
    Surface s = Surface.create(n, 14.0, 1.45, 0.25);
    Light sun = Light.directional(float3(-1.0, -1.0, 1.0), 2.0);
    return float4(s.refraction(), s.fresnel(), s.specular(sun).r);
}
`;

export const IMPORTS = ["sk.color", "sk.light"];

export const caseEffect = (name: string, chunk: Case[]) => tableEffect(name, chunk, IMPORTS, DECLARATIONS);
