import { type Draft, section } from "./shared";

const since = "Unreleased" as const;

const drafts: Draft[] = [
  {
    slug: "sk-pi",
    name: "SK_PI",
    kind: "type",
    summary: "π and 2π (`SK_TAU`, one turn in radians) as `float` constants.",
    since,
    syntax: `import sk.math;

static const float SK_PI;    // 3.14159265358979
static const float SK_TAU;   // 6.28318530717959`,
    notes: [
      "Prefixed so they never clash with a `PI` an effect declares for itself: Slang reports a name that two imports both declare as ambiguous.",
    ],
    aliases: ["SK_TAU"],
  },
  {
    slug: "floor-mod",
    name: "floorMod",
    kind: "function",
    summary: "`x` wrapped into [0, y) for a positive `y`, as GLSL `mod`: a negative `x` wraps upwards.",
    since,
    syntax: `float floorMod(float x, float y);
vector<float, N> floorMod<let N : int>(vector<float, N> x, vector<float, N> y);`,
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
    kind: "function",
    summary: "`a / b`, or `fallback` where `b` is 0: no infinity or NaN from a zero divisor.",
    since,
    syntax: `float safeDiv(float a, float b, float fallback = 0.0);
vector<float, N> safeDiv<let N : int>(vector<float, N> a, vector<float, N> b, float fallback = 0.0);`,
    params: [
      { name: "a", type: "float", description: "The dividend." },
      { name: "b", type: "float", description: "The divisor; checked against exactly 0." },
      { name: "fallback", type: "float", description: "The result where `b` is 0.", optional: true, default: "0.0" },
    ],
    notes: ["The vector form checks each component on its own."],
  },
  {
    slug: "remap",
    name: "remap",
    kind: "function",
    summary:
      "`x` mapped linearly from one range to another, unclamped; `remapClamped` holds the result in the target range.",
    since,
    syntax: `float remap(float x, float from0, float from1, float to0, float to1);
float remapClamped(float x, float from0, float from1, float to0, float to1);
// and per component on float2, float3 and float4`,
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
