// Golden tables for the shader standard library: each case is a float4 Slang expression and the
// value a float64 reference expects, and `tableEffect` builds the effect that encodes every result of
// a chunk into pixels, so 8-bit output carries each float bit for bit. `one` is in scope in a case:
// 1.0 known only at run time, to keep the compiler from folding a case's inputs.

export interface Case {
  name: string;
  /** A float4 Slang expression. */
  slang: string;
  expect: number[];
  /** Absolute tolerance per component. */
  tol: number[];
  /** Components that are hues in degrees, compared around the circle. */
  hues?: number[];
}

/** Device pixels per row. Two pixels encode one float. */
export const COLUMNS = 64;
const MARK = [0x5a, 0xa5];

export const num = (x: number) => (Number.isInteger(x) ? `${x}.0` : String(x));
export const vec = (v: number[]) => `float${v.length}(${v.map(num).join(", ")})`;

/** One effect per chunk of cases, so no single fragment shader grows past what fxc compiles quickly. */
export function tableEffect(name: string, chunk: Case[], imports: string[], declarations = ""): string {
  const branches = chunk.map((c, i) => `    case ${i}u: return ${c.slang}; // ${c.name}`).join("\n");
  return `import sk;
${imports.map((m) => `import ${m};\n`).join("")}${declarations}
float4 sk_case(uint index, float one)
{
    switch (index)
    {
${branches}
    default: return float4(0.0, 0.0, 0.0, 0.0);
    }
}

// Each result component is two cells: the high and low 16 bits of its float, in red and green,
// with a marker in blue, so 8-bit output carries every bit.
[SkEffect("${name}")]
struct Table : IEffect
{
    float4 shade(EffectInput input)
    {
        uint2 cell = uint2(input.px);
        uint index = cell.y * ${COLUMNS}u + cell.x;
        if (cell.x >= ${COLUMNS}u || index >= ${chunk.length * 8}u)
            return float4(0.0, 0.0, 0.0, 1.0);
        uint scalar = index / 2u;
        uint part = index % 2u;
        float4 v = sk_case(scalar / 4u, input.opacity);
        uint component = scalar % 4u;
        float x = component == 0u ? v.x : (component == 1u ? v.y : (component == 2u ? v.z : v.w));
        uint bits = asuint(x);
        uint word = part == 0u ? bits >> 16u : bits & 0xffffu;
        float3 rgb = float3(float(word >> 8u), float(word & 0xffu), part == 0u ? ${MARK[0]}.0 : ${MARK[1]}.0) / 255.0;
        return float4(rgb, 1.0);
    }
};
`;
}

/** Device-pixel rows an effect of `count` cases needs. */
export const effectHeight = (count: number) => Math.ceil((count * 8) / COLUMNS);

/** The results back from a screenshot: `pixel(x, y)` reads RGB at a device pixel of the element. */
export function decode(count: number, pixel: (x: number, y: number) => [number, number, number]): number[][] {
  const words: number[] = [];
  for (let i = 0; i < count * 8; i += 1) {
    const [r, g, b] = pixel(i % COLUMNS, Math.floor(i / COLUMNS));
    if (b !== MARK[i % 2]) throw new Error(`probe cell ${i} has marker ${b}, not ${MARK[i % 2]}: misaligned read`);
    words.push((r << 8) | g);
  }
  const view = new DataView(new ArrayBuffer(4));
  const values: number[][] = [];
  for (let c = 0; c < count; c += 1) {
    const v: number[] = [];
    for (let k = 0; k < 4; k += 1) {
      view.setUint32(0, ((words[(c * 4 + k) * 2]! << 16) | words[(c * 4 + k) * 2 + 1]!) >>> 0);
      v.push(view.getFloat32(0));
    }
    values.push(v);
  }
  return values;
}

/** The components of `got` that miss `c`; empty when it passes. */
export function misses(c: Case, got: number[]): string[] {
  const bad: string[] = [];
  c.expect.forEach((want, i) => {
    const value = got[i]!;
    let diff = Math.abs(value - want);
    if (c.hues?.includes(i)) diff = Math.min(diff, 360 - (diff % 360));
    if (!Number.isFinite(value) || diff > c.tol[i]!) bad.push(`[${i}] ${value} vs ${want} (±${c.tol[i]})`);
  });
  return bad;
}

const OPEN = "⟨";
const CLOSE = "⟩";

/** Marks a float (or float vector) input for `withRunTime`: a literal in one variant, a run-time value in the other. */
export const live = (expr: string) => `${OPEN}${expr}${CLOSE}`;

/** Each case with `live` inputs twice: as written, and with those inputs multiplied by `one`. */
export function withRunTime(cases: Case[]): Case[] {
  const marked = new RegExp(`${OPEN}([^${OPEN}${CLOSE}]*)${CLOSE}`, "g");
  return cases.flatMap((c) =>
    c.slang.includes(OPEN)
      ? [
          { ...c, slang: c.slang.replace(marked, "$1") },
          { ...c, name: `${c.name}, at run time`, slang: c.slang.replace(marked, "(($1) * one)") },
        ]
      : [c],
  );
}
