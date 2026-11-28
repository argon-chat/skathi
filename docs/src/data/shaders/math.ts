import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const drafts: Draft[] = [
  {
    slug: "sk-pi",
    name: "SK_PI",
    summary: "π and 2π (`SK_TAU`, one turn in radians) as `float` constants.",
    since,
    notes: [
      "Prefixed so they never clash with a `PI` an effect declares for itself: Slang reports a name that two imports both declare as ambiguous.",
    ],
    aliases: ["SK_TAU"],
  },
  {
    slug: "floor-mod",
    name: "floorMod",
    since,
    params: [
      { name: "x", type: "float", description: "The value to wrap." },
      { name: "y", type: "float", description: "The period, greater than 0." },
    ],
    returns: "`x - y * floor(x / y)`: −0.25 wraps to 0.75 for `y` 1.",
    notes: [
      "HLSL's `fmod` truncates towards zero, so `fmod(-0.25, 1.0)` is −0.25. Angles, tiles and loops want this one.",
      "Precision follows the size of `x`: at `x` around 1000 a float carries about 6e-5, so wrap a time before it grows if you need finer steps.",
    ],
    related: ["safe-div", "loop"],
  },
  {
    slug: "safe-div",
    name: "safeDiv",
    since,
    params: [
      { name: "a", type: "float", description: "The dividend." },
      { name: "b", type: "float", description: "The divisor; checked against exactly 0." },
      { name: "fallback", type: "float", description: "The result where `b` is 0.", optional: true, default: "0.0" },
    ],
    notes: ["The vector form checks each component on its own."],
  },
  {
    slug: "safe-atan2",
    name: "safeAtan2",
    since,
    params: [
      { name: "y", type: "float", description: "The vertical component." },
      { name: "x", type: "float", description: "The horizontal component." },
    ],
    returns: "[−π, π]; 0 at (0, 0). A zero of either sign counts as +0, so (0, −1) is π and (1, ±0) is π/2.",
    notes: [
      "The built-in `atan2(0, 0)` is undefined in HLSL, Metal and WGSL, and Metal's fast maths can flip the sign of the result when `x` is −0. Use this one for angles of points that can sit at a centre or on an axis.",
      "Cost: one `atan` and a few comparisons.",
    ],
    examples: [
      {
        title: "A conic sweep around the element's centre",
        lang: "slang",
        code: `import sk;
import sk.math;

[SkEffect("conic")]
struct Conic : IEffect
{
    float4 shade(EffectInput input)
    {
        float2 d = input.px - input.size * 0.5;
        float t = safeAtan2(d.x, -d.y) / SK_TAU + 0.5;
        return float4(t, t, t, 1.0) * input.coverage;
    }
};`,
      },
    ],
    related: ["floor-mod"],
  },
  {
    slug: "remap",
    name: "remap",
    summary:
      "`x` mapped linearly from one range to another, unclamped; `remapClamped` holds the result in the target range.",
    since,
    params: [
      { name: "x", type: "float", description: "The value." },
      { name: "from0, from1", type: "float", description: "The source range; either order." },
      { name: "to0, to1", type: "float", description: "The target range; either order." },
    ],
    notes: [
      "An empty source range (`from0 == from1`) maps everything to `to0` instead of dividing by zero.",
      "`remapClamped(x, a, b, 0.0, 1.0)` is `linearstep(a, b, x)` from `sk.ease`.",
    ],
    examples: [
      {
        title: "A horizontal fade across the element",
        lang: "slang",
        code: `import sk;
import sk.math;

[SkEffect("edge-fade")]
struct EdgeFade : IEffect
{
    [SkLength] [SkDefault("24")] float fade;

    float4 shade(EffectInput input)
    {
        float a = remapClamped(input.px.x, 0.0, fade, 0.0, 1.0);
        return float4(1.0, 1.0, 1.0, 1.0) * a;
    }
};`,
      },
    ],
    aliases: ["remapClamped"],
    related: ["linearstep"],
  },
];

export const math = section("sk.math", drafts);
