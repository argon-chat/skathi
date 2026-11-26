import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const octaveParams = [
  {
    name: "p",
    type: "float2",
    description: "The point, in noise cells: scale `input.px` by one over the feature size.",
  },
  {
    name: "octaves",
    type: "float",
    description: "Layers summed, up to `MAX_OCTAVES` (8); a fraction fades the last one in.",
    optional: true,
    default: "5.0",
  },
  {
    name: "lacunarity",
    type: "float",
    description: "How much finer each layer is than the last.",
    optional: true,
    default: "2.0",
  },
  {
    name: "gain",
    type: "float",
    description: "How much weaker each layer is than the last.",
    optional: true,
    default: "0.5",
  },
];

const drafts: Draft[] = [
  {
    slug: "pcg",
    name: "pcg",
    kind: "function",
    summary: "An integer hash: one round of the PCG output permutation, uint in, uint out, the same bits on every GPU.",
    since,
    syntax: `import sk.noise;

uint pcg(uint v);
uint pcg(uint2 v);    // pcg(v.x + pcg(v.y))
uint pcg(uint3 v);    // pcg(v.x + pcg(v.y + pcg(v.z)))
uint3 pcg3d(uint3 v); // three independent words at once`,
    params: [
      { name: "v", type: "uint, uint2, uint3", description: "The words to hash: a pixel, a cell, a frame count." },
    ],
    returns: "A uint whose every bit is about equally likely to be set; `pcg3d` returns three.",
    notes: [
      "Integer multiplies, adds, shifts and xors only, so DirectX 12, Metal and wgpu agree bit for bit; the test suite checks it on each.",
      "Cost: about 3 multiplies for `pcg`, 9 for `pcg3d`. `pcg3d` is the cheaper way to get several values for one cell.",
      "`sk_hash12` from `sk` hashes with float arithmetic, which rounds differently from one GPU to another: a pattern built on it shifts between machines. Use `hash` for anything that must match.",
    ],
    examples: [
      {
        title: "A random colour per 16 px tile",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("tile-confetti")]
struct TileConfetti : IEffect
{
    float4 shade(EffectInput input)
    {
        uint3 bits = pcg3d(uint3(uint2(input.px / 16.0), 7u));
        float3 rgb = float3(bits >> 8u) / 16777216.0;
        return float4(rgb, 1.0) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["pcg3d"],
    related: ["hash"],
  },
  {
    slug: "hash",
    name: "hash",
    kind: "function",
    summary:
      "Random floats in [0, 1) for the cell `floor(p)`, from `pcg`: one, two or three values per cell, identical on every backend.",
    since,
    syntax: `float hash(float p);   float hash(float2 p);   float hash(float3 p);
float2 hash2(float p); float2 hash2(float2 p); float2 hash2(float3 p);
float3 hash3(float p); float3 hash3(float2 p); float3 hash3(float3 p);`,
    params: [
      {
        name: "p",
        type: "float, float2, float3",
        description: "Any point of the cell; the cell is `floor(p)`, negative cells included.",
      },
    ],
    returns: "[0, 1) in steps of 2⁻²⁴: the top 24 bits of the hash, which a float holds exactly.",
    notes: [
      "Cost: `hash` of a float2 is 2 `pcg`, of a float3 3; `hash2` and `hash3` are one `pcg3d` whatever the dimension.",
      "Bit-exact across DirectX 12, Metal and wgpu, so a scatter of sparkles or a dissolve sits on the same pixels on every machine.",
      "Cells beyond ±2³¹ clamp to the last one, where a plain float-to-int conversion would be undefined.",
    ],
    examples: [
      {
        title: "A dissolve that eats the element cell by cell",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("dissolve")]
struct Dissolve : IEffect
{
    [SkRange(0.0, 1.0)] [SkDefault("0.5")] float amount;

    float4 shade(EffectInput input)
    {
        float keep = step(amount, hash(input.px / 6.0));
        return input.content * keep;
    }
};`,
      },
    ],
    aliases: ["hash2", "hash3"],
    related: ["pcg", "value-noise"],
  },
  {
    slug: "i-noise",
    name: "INoise",
    kind: "interface",
    summary:
      "A noise over the plane, passed as a type to `fbm`, `turbulence`, `ridged`, `warp` and `curl`: `Perlin`, `Simplex`, `ValueNoise` or your own.",
    since,
    syntax: `interface INoise
{
    static float at(float2 p);           // [-1, 1]
    static float3 derivatives(float2 p); // (value, d/dx, d/dy)
}

interface INoise3
{
    static float at(float3 p);
    static float4 derivatives(float3 p); // (value, d/dx, d/dy, d/dz)
}`,
    notes: [
      "A noise is a type, not a value, so `fbm<Simplex>(p)` compiles to the same code as a loop written by hand around `Simplex.at`: the measured output differs by under 1 %.",
      "A struct of your own with these two static functions works with every fractal; `derivatives` only has to be right if you call `fbmDerivatives` or `curl` with it.",
    ],
    examples: [
      {
        title: "A noise of your own: stripes bent by simplex",
        lang: "slang",
        code: `import sk;
import sk.noise;

struct Stripes : INoise
{
    static float at(float2 p) { return sin(p.x * 3.0 + 2.0 * Simplex.at(p)); }
    static float3 derivatives(float2 p)
    {
        float3 s = Simplex.derivatives(p);
        float c = cos(p.x * 3.0 + 2.0 * s.x);
        return float3(sin(p.x * 3.0 + 2.0 * s.x), c * (3.0 + 2.0 * s.y), c * 2.0 * s.z);
    }
};

[SkEffect("marble")]
struct Marble : IEffect
{
    float4 shade(EffectInput input)
    {
        float v = fbm<Stripes>(input.px / 80.0, 4.0) * 0.5 + 0.5;
        return float4(v, v, v, 1.0) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["INoise3"],
    related: ["fbm", "perlin", "simplex", "value-noise"],
  },
  {
    slug: "perlin",
    name: "Perlin",
    kind: "struct",
    summary: "Perlin's improved gradient noise in 2D and 3D, with analytic derivatives and a tiling form.",
    since,
    syntax: `struct Perlin : INoise, INoise3
{
    static float at(float2 p);
    static float3 derivatives(float2 p);                     // (value, d/dx, d/dy)
    static float tiled(float2 p, float2 period);
    static float3 tiledDerivatives(float2 p, float2 period);
    static float at(float3 p);
    static float4 derivatives(float3 p);
}`,
    params: [
      { name: "p", type: "float2, float3", description: "The point, in lattice cells." },
      {
        name: "period",
        type: "float2",
        description: "Cells after which `tiled` repeats on each axis, rounded, at least 1.",
      },
    ],
    returns: "[-1, 1], 0 at every integer point. 2D reaches about ±1; 3D stays within about ±0.8 in practice.",
    notes: [
      "Cost: 2D reads 4 corners (8 `pcg`), 3D 8 corners (24 `pcg`).",
      "The quintic fade makes the second derivative continuous, so normals from `derivatives` show no grid creases.",
      "The values are not clamped: they stay inside the range by construction.",
    ],
    examples: [
      {
        title: "A seamless 4 x 4-cell tile",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("perlin-tile")]
struct PerlinTile : IEffect
{
    float4 shade(EffectInput input)
    {
        float v = Perlin.tiled(input.local * 4.0, float2(4.0, 4.0)) * 0.5 + 0.5;
        return float4(v, v, v, 1.0) * input.coverage;
    }
};`,
      },
    ],
    related: ["simplex", "value-noise", "i-noise", "fbm"],
  },
  {
    slug: "simplex",
    name: "Simplex",
    kind: "struct",
    summary:
      "Simplex noise in 2D and 3D: fewer corners than Perlin, no axis-aligned artefacts, analytic derivatives, and flow noise that swirls in place.",
    since,
    syntax: `struct Simplex : INoise, INoise3
{
    static float at(float2 p);
    static float3 derivatives(float2 p);
    static float flow(float2 p, float angle);
    static float3 flowDerivatives(float2 p, float angle);
    static float at(float3 p);
    static float4 derivatives(float3 p);
}`,
    params: [
      { name: "p", type: "float2, float3", description: "The point, in lattice cells." },
      {
        name: "angle",
        type: "float",
        description: "How far `flow` turns the gradients, in radians; periodic over 2π. Drive it with `input.time`.",
      },
    ],
    returns: "[-1, 1]. 2D reaches about ±1; 3D stays within about ±0.8 in practice.",
    notes: [
      "Cost: 2D reads 3 corners (6 `pcg`), 3D 4 corners (12 `pcg`): the cheapest gradient noise here.",
      "`flow` (Perlin and Neyret's flow noise) turns half the gradients one way and half the other, so the pattern churns without drifting; animating `p` instead slides it.",
    ],
    examples: [
      {
        title: "A churning surface",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("churn")]
struct Churn : IEffect
{
    [SkColor] [SkDefault("#3a7bd5")] float4 tint;
    [SkAnimated] [SkDefault("1.5")] float speed;

    float4 shade(EffectInput input)
    {
        float v = Simplex.flow(input.px / 40.0, input.time * speed);
        return sk_premultiply(float4(tint.rgb * (0.8 + 0.2 * v), tint.a)) * input.coverage;
    }
};`,
      },
    ],
    related: ["perlin", "value-noise", "curl", "fbm"],
  },
  {
    slug: "value-noise",
    name: "ValueNoise",
    kind: "struct",
    summary:
      "Value noise: random heights at the lattice points blended smoothly. Blockier than gradient noise, and the cheapest base for a fractal.",
    since,
    syntax: `struct ValueNoise : INoise, INoise3
{
    static float at(float2 p);
    static float3 derivatives(float2 p);
    static float tiled(float2 p, float2 period);
    static float3 tiledDerivatives(float2 p, float2 period);
    static float at(float3 p);
    static float4 derivatives(float3 p);
}`,
    params: [
      { name: "p", type: "float2, float3", description: "The point, in lattice cells." },
      { name: "period", type: "float2", description: "Cells after which `tiled` repeats, rounded, at least 1." },
    ],
    returns: "[-1, 1], reaching about ±1 in 2D and 3D.",
    notes: ["Cost: 2D 4 corners (8 `pcg`), 3D 8 corners (24 `pcg`)."],
    examples: [
      {
        title: "Soft clouds",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("clouds")]
struct Clouds : IEffect
{
    float4 shade(EffectInput input)
    {
        float v = fbm<ValueNoise>(input.px / 120.0, 6.0) * 0.5 + 0.5;
        return float4(v, v, v, v) * input.coverage;
    }
};`,
      },
    ],
    related: ["perlin", "simplex", "hash"],
  },
  {
    slug: "worley",
    name: "worley",
    kind: "function",
    summary:
      "Cellular (Worley) noise: distances to the nearest and second-nearest feature points, and the nearest one's cell and id, in 2D, tiling 2D and 3D.",
    since,
    syntax: `struct Cell { float f1; float f2; float2 cell; uint id; float2 toNearest; };
struct Cell3 { float f1; float f2; float3 cell; uint id; };

Cell worley(float2 p, float jitter = 1.0);
Cell worley(float2 p, float jitter, float2 period);
Cell3 worley(float3 p, float jitter = 1.0);`,
    params: [
      { name: "p", type: "float2, float3", description: "The point, in cells: one feature point per cell." },
      {
        name: "jitter",
        type: "float",
        description: "0 puts each point at its cell's centre (a square grid), 1 anywhere in the cell.",
        optional: true,
        default: "1.0",
      },
      { name: "period", type: "float2", description: "Cells after which the pattern repeats, rounded, at least 1." },
    ],
    returns:
      "`f1` in cells, 0 on a point and below about 1.1; `f2` ≥ `f1`, below about 1.45. `id` is the same over the whole Voronoi region of a point.",
    notes: [
      "Cost: 2D searches 3 × 3 cells (9 `pcg3d`), 3D 27 cells (27 `pcg3d`).",
      "`f2 - f1` is 0 on the cell boundaries but is not a distance; `worleyEdge` is the exact distance to them.",
      "`float(cell.id) / 4294967296.0` turns the id into a value in [0, 1) for a colour per cell.",
    ],
    examples: [
      {
        title: "Stained glass",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("stained-glass")]
struct StainedGlass : IEffect
{
    float4 shade(EffectInput input)
    {
        float2 p = input.px / 36.0;
        Cell c = worley(p);
        float3 rgb = hash3(c.cell + 0.5) * 0.8 + 0.2;
        float lead = smoothstep(0.03, 0.06, worleyEdge(p));
        return float4(rgb * lead, 1.0) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["Cell", "Cell3"],
    related: ["worley-edge", "hash"],
  },
  {
    slug: "worley-edge",
    name: "worleyEdge",
    kind: "function",
    summary: "The exact distance from a point to the nearest edge of its Voronoi cell, in cells: 0 on an edge.",
    since,
    syntax: `float worleyEdge(float2 p, float jitter = 1.0);`,
    params: [
      { name: "p", type: "float2", description: "The point, in cells." },
      { name: "jitter", type: "float", description: "As for `worley`.", optional: true, default: "1.0" },
    ],
    returns: "≥ 0, at most about 0.62 at the middle of a cell.",
    notes: [
      "Measured to the bisector between the nearest point and each neighbour, so lines drawn with it keep one width; `f2 - f1` does not.",
      "Cost: two 3 × 3 searches, 18 `pcg3d`.",
    ],
    examples: [
      {
        title: "Cell walls 2 px wide",
        lang: "slang",
        code: `float cell = 40.0;
float d = worleyEdge(input.px / cell) * cell;
return float4(1.0, 1.0, 1.0, 1.0) * (1.0 - smoothstep(1.0, 2.0, d)) * input.coverage;`,
      },
    ],
    related: ["worley"],
  },
  {
    slug: "fbm",
    name: "fbm",
    kind: "function",
    summary:
      "Fractal Brownian motion: layers of a noise, each finer and weaker, normalised so the range stays that of the noise. 2D and 3D.",
    since,
    syntax: `float fbm<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5);
float fbm<N : INoise3>(float3 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5);
float3 fbmDerivatives<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5);
static const int MAX_OCTAVES = 8;`,
    params: octaveParams,
    returns: "[-1, 1], the sum divided by its weights; `fbmDerivatives` returns (value, d/dx, d/dy).",
    notes: [
      "Cost: `octaves` × the noise, at most 8. The loop has a fixed bound, so the compiler unrolls it and the shader never branches on the count.",
      "Each layer is rotated by about 37° and shifted, so the lattices of the layers never line up into a grid.",
      "A fractional `octaves` fades the last layer in: animate the detail without popping.",
      "`fbm<Perlin>(float3(p, input.time * 0.2))` animates a 2D pattern without sliding it.",
    ],
    examples: [
      {
        title: "Drifting smoke",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("smoke")]
struct Smoke : IEffect
{
    [SkColor] [SkDefault("#ffffff")] float4 tint;
    [SkAnimated] [SkDefault("0.15")] float speed;

    float4 shade(EffectInput input)
    {
        float v = fbm<Simplex>(float3(input.px / 90.0, input.time * speed), 5.0) * 0.5 + 0.5;
        return sk_premultiply(float4(tint.rgb, tint.a * v * v)) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["fbmDerivatives", "MAX_OCTAVES"],
    related: ["turbulence", "ridged", "warp", "i-noise"],
  },
  {
    slug: "turbulence",
    name: "turbulence",
    kind: "function",
    summary: "`fbm` of the noise's absolute value: billowy, with creases where it crosses zero.",
    since,
    syntax: `float turbulence<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5);`,
    params: octaveParams,
    returns: "[0, 1].",
    notes: ["Cost: as `fbm`."],
    examples: [
      {
        title: "Fire-like billows",
        lang: "slang",
        code: `float v = turbulence<Perlin>(input.px / 60.0, 5.0);
return float4(1.0, 0.45, 0.1, 1.0) * v * input.coverage;`,
      },
    ],
    related: ["fbm", "ridged"],
  },
  {
    slug: "ridged",
    name: "ridged",
    kind: "function",
    summary:
      "Ridged noise: `fbm` of (1 − |N|)², sharp crests where the noise crosses zero, for mountain ranges and lightning.",
    since,
    syntax: `float ridged<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5);`,
    params: octaveParams,
    returns: "[0, 1], 1 on a crest.",
    notes: ["Cost: as `fbm`."],
    examples: [
      {
        title: "Veins",
        lang: "slang",
        code: `float v = pow(ridged<Simplex>(input.px / 70.0, 4.0), 6.0);
return float4(0.6, 0.9, 1.0, 1.0) * v * input.coverage;`,
      },
    ],
    related: ["fbm", "turbulence"],
  },
  {
    slug: "warp",
    name: "warp",
    kind: "function",
    summary:
      "Domain warping: the point moved by two `fbm`s of a noise, for marbled, smoky patterns. Feed the result to any noise.",
    since,
    syntax: `float2 warp<N : INoise>(float2 p, float strength, float octaves = 4.0);`,
    params: [
      { name: "p", type: "float2", description: "The point, in noise cells." },
      {
        name: "strength",
        type: "float",
        description: "How far the point moves, in cells, at the extremes of the `fbm`s.",
      },
      { name: "octaves", type: "float", description: "Octaves of each `fbm`.", optional: true, default: "4.0" },
    ],
    returns: "The moved point: `p` plus up to `strength` on each axis.",
    notes: ["Cost: two `fbm`s; the noise you read at the result comes on top."],
    examples: [
      {
        title: "Marble",
        lang: "slang",
        code: `float2 q = warp<Simplex>(input.px / 100.0, 1.5);
float v = fbm<Simplex>(q, 4.0) * 0.5 + 0.5;
return float4(v, v * 0.95, v * 0.9, 1.0) * input.coverage;`,
      },
    ],
    related: ["fbm", "curl"],
  },
  {
    slug: "curl",
    name: "curl",
    kind: "function",
    summary:
      "Curl noise: the rotated gradient of a noise or of its `fbm`, a divergence-free 2D flow for advection and warps that never bunch up.",
    since,
    syntax: `float2 curl<N : INoise>(float2 p);
float2 curl<N : INoise>(float2 p, float octaves, float lacunarity = 2.0, float gain = 0.5);`,
    params: octaveParams.map((p) => (p.name === "octaves" ? { ...p, optional: false } : p)),
    returns: "(dN/dy, −dN/dx), per noise cell. Magnitude 0 to about 8 for one octave; scale it to taste.",
    notes: [
      "Divergence-free by construction: the flow neither gathers nor thins out what it moves. The test suite checks the divergence is zero to rounding.",
      "Cost: the noise's `derivatives`, once per octave; no finite differences.",
      "Its scale is per noise cell: divide by the cell size in pixels before using it as a pixel offset, or normalise it.",
    ],
    examples: [
      {
        title: "Content stirred by a flow",
        lang: "slang",
        code: `import sk;
import sk.noise;
import sk.sample;

[SkEffect("stir")]
[SkContent]
struct Stir : IEffect
{
    [SkLength] [SkDefault("6")] float strength;
    [SkAnimated] [SkDefault("0.2")] float speed;

    float4 shade(EffectInput input)
    {
        float2 flow = curl<Simplex>(input.px / 80.0 + float2(input.time * speed, 0.0), 3.0);
        return displace(Content(input), input.px, flow * strength / 8.0);
    }
};`,
      },
    ],
    related: ["warp", "simplex", "displace"],
  },
  {
    slug: "ign",
    name: "ign",
    kind: "function",
    summary:
      "Interleaved gradient noise (Jimenez 2014): a cheap per-pixel threshold whose 3 × 3 neighbourhoods are well spread, optionally moving every frame.",
    since,
    syntax: `float ign(float2 px);
float ign(float2 px, float frame);`,
    params: [
      { name: "px", type: "float2", description: "The pixel, `input.px`." },
      {
        name: "frame",
        type: "float",
        description: "A whole frame count: the pattern moves by a fixed step each frame.",
        optional: true,
      },
    ],
    returns: "[0, 1).",
    notes: [
      "Float arithmetic: a value may differ by an ulp between GPUs. Use `bayer` or `hash` where exact agreement matters.",
      "Cost: a dot product and two `frac`s.",
    ],
    examples: [
      {
        title: "A soft-edge threshold",
        lang: "slang",
        code: `float edge = saturate(-input.sd / 12.0);
return float4(1.0, 1.0, 1.0, 1.0) * step(ign(input.px), edge) * input.coverage;`,
      },
    ],
    related: ["r2", "bayer", "dither"],
  },
  {
    slug: "r2",
    name: "r2",
    kind: "function",
    summary:
      "The R2 low-discrepancy sequence (Roberts 2018) over pixels: an even, blue-noise-like threshold mask without a texture.",
    since,
    syntax: `float r2(float2 px);`,
    params: [{ name: "px", type: "float2", description: "The pixel, `input.px`." }],
    returns: "[0, 1).",
    notes: ["Cost: a dot product and a `frac`. Float arithmetic, like `ign`."],
    examples: [
      {
        title: "Stippled shading",
        lang: "slang",
        code: `float shade = input.local.x;
return float4(0.0, 0.0, 0.0, 1.0) * step(r2(input.px), shade) * input.coverage;`,
      },
    ],
    related: ["ign", "bayer"],
  },
  {
    slug: "bayer",
    name: "bayer",
    kind: "function",
    summary: "The ordered (Bayer) dither threshold of a pixel in a 2 × 2, 4 × 4 or 8 × 8 matrix, in integer maths.",
    since,
    syntax: `float bayer(float2 px, int size = 8);`,
    params: [
      { name: "px", type: "float2", description: "The pixel, `input.px`." },
      { name: "size", type: "int", description: "The matrix: 2, 4 or 8.", optional: true, default: "8" },
    ],
    returns: "`(index + 0.5) / size²`, in (0, 1): the 64 values of the 8 × 8 matrix are 1/128 to 127/128.",
    notes: ["Integer maths: the same on every backend. Cost: a few bit operations."],
    examples: [
      {
        title: "A one-bit retro fade",
        lang: "slang",
        code: `import sk;
import sk.noise;

[SkEffect("one-bit")]
[SkBackdrop]
struct OneBit : IEffect
{
    float4 shade(EffectInput input)
    {
        float l = dot(input.backdrop.rgb, float3(0.2126, 0.7152, 0.0722));
        float on = step(bayer(input.px, 4), l);
        return float4(on, on, on, 1.0) * input.coverage;
    }
};`,
      },
    ],
    related: ["ign", "r2", "dither"],
  },
  {
    slug: "dither",
    name: "dither",
    kind: "function",
    summary:
      "Triangular-PDF dither: ±1 quantisation step of noise per channel, so a smooth gradient written to an 8-bit target shows no bands.",
    since,
    syntax: `float3 dither(float3 rgb, float2 px, float levels = 255.0);
float3 dither(float3 rgb, float2 px, float frame, float levels);
float triangular(float u);   // [0, 1) uniform -> (-1, 1) triangular`,
    params: [
      {
        name: "rgb",
        type: "float3",
        description: "The colour as it will be stored: sRGB-encoded, what `shade` returns.",
      },
      { name: "px", type: "float2", description: "The pixel, `input.px`." },
      { name: "frame", type: "float", description: "A whole frame count, for a pattern that changes every frame." },
      {
        name: "levels",
        type: "float",
        description: "Steps of the target: 255 for 8 bits.",
        optional: true,
        default: "255.0",
      },
    ],
    returns: "`rgb` plus noise in (−1, 1) / `levels` per channel, mean 0; not clamped.",
    notes: [
      "Triangular noise of two steps' width makes the quantisation error independent of the signal, which uniform noise of one step does not: no bands and no brightness-dependent grain.",
      "Apply it last, to premultiplied rgb, after any maths that could stretch a gradient.",
      "Cost: one `pcg3d`.",
    ],
    examples: [
      {
        title: "A long, band-free gradient",
        lang: "slang",
        code: `float3 rgb = lerp(float3(0.10, 0.11, 0.14), float3(0.16, 0.17, 0.21), input.local.y);
return float4(dither(rgb, input.px), 1.0) * input.coverage;`,
      },
    ],
    aliases: ["triangular"],
    related: ["ign", "bayer"],
  },
];

export const noise = section("sk.noise", drafts);
