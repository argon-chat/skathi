import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const T = { name: "t", type: "float", description: "Progress; clamped to 0..1." };

const penner = (family: string, summary: string, notes?: string[]): Draft => ({
  slug: `ease-${family.toLowerCase()}`,
  name: `easeIn${family}`,
  summary,
  since,
  params: [T],
  notes: ["The formulas of easings.net. `t` is clamped to 0..1, and 0 and 1 map to themselves.", ...(notes ?? [])],
  aliases: [`easeOut${family}`, `easeInOut${family}`],
  related: ["cubic-bezier", "i-easing"],
});

const RIPPLE = `import sk;
import sk.ease;
import sk.sdf;

[SkEffect("click-ripple")]
struct ClickRipple : IEffect
{
    [SkColor] [SkDefault("#ffffff60")] float4 tint;
    // The time and place of the last click, written from JS: performance.now() / 1000 and 0..1.
    [SkAnimatedFor(0.6)] [SkDefault("-100")] float clickedAt;
    [SkDefault("0.5 0.5")] float2 origin;

    float4 shade(EffectInput input)
    {
        float t = progress(input.time, clickedAt, 0.6);
        float reach = length(input.size);
        float radius = CubicBezier.easeOut().at(t) * reach;
        float d = Sdf.circle(input.px - origin * input.size, radius);
        float fade = 1.0 - easeInQuad(t);
        float a = Sdf.coverage(d) * fade * (t < 1.0 ? 1.0 : 0.0);
        return sk_premultiply(tint) * a;
    }
};`;

const drafts: Draft[] = [
  {
    slug: "i-easing",
    name: "IEasing",
    since,
    notes: [
      "Take an `IEasing` in your own helpers to let callers choose the curve; it specialises at compile time.",
      "There is no separate `ease<E>(t)`: an easing with parameters needs a value, and `e.at(t)` is that call.",
    ],
    examples: [
      {
        title: "A fade-in with the curve as a parameter of your own helper",
        lang: "slang",
        code: `import sk;
import sk.ease;

float fadeIn<E : IEasing>(E curve, float time, float start, float seconds)
{
    return curve.at(progress(time, start, seconds));
}

[SkEffect("fade-in")]
struct FadeIn : IEffect
{
    [SkAnimatedFor(0.8)] [SkDefault("0")] float shownAt;

    float4 shade(EffectInput input)
    {
        float a = fadeIn(CubicBezier.ease(), input.time, shownAt, 0.8);
        return float4(1.0, 1.0, 1.0, 1.0) * a;
    }
};`,
      },
    ],
    related: ["cubic-bezier", "steps", "linear-stops", "spring"],
  },
  {
    slug: "cubic-bezier",
    name: "CubicBezier",
    summary: "CSS `cubic-bezier(x1, y1, x2, y2)` and the keywords `ease`, `ease-in`, `ease-out` and `ease-in-out`.",
    since,
    params: [
      { name: "x1, x2", type: "float", description: "Clamped to 0..1, as CSS requires." },
      { name: "y1, y2", type: "float", description: "Any value: outside 0..1 the curve overshoots." },
    ],
    notes: [
      "`at` solves x(s) = t with ten safeguarded Newton steps (bisection where a step would leave its bracket), a fixed count: the x residual is below 2e-6 for any x1, x2, and the result matches a float64 solve within 1e-5.",
      "Outside 0..1 the curve continues along its end tangents, as CSS Easing defines for an overshooting input.",
      "Where x1 or x2 is 0 or 1 the curve is vertical at an end; values there are as exact as float precision of t allows.",
    ],
    examples: [{ title: "A click ripple timed with ease-out", lang: "slang", code: RIPPLE }],
    aliases: ["CubicBezier.ease", "CubicBezier.easeIn", "CubicBezier.easeOut", "CubicBezier.easeInOut"],
    related: ["i-easing", "progress"],
  },
  {
    slug: "steps",
    name: "Steps",
    since,
    params: [
      { name: "count", type: "int", description: "Steps, at least 1 (2 for `JumpNone`)." },
      {
        name: "position",
        type: "StepPosition",
        description: "Where the jumps are.",
        optional: true,
        default: "JumpEnd",
      },
    ],
    notes: [
      "CSS Easing Level 2: 0 before 0, 1 from 1 on, and at a step boundary the higher value. The before flag (an animation's delay phase) is taken as unset.",
    ],
    related: ["step-position"],
  },
  {
    slug: "step-position",
    name: "StepPosition",
    since,
    related: ["steps"],
  },
  {
    slug: "linear-stops",
    name: "LinearStops",
    summary: "CSS `linear()` with `N` stops: straight segments between (input, output) points.",
    since,
    examples: [
      {
        title: "Match CSS linear(0, 0.25 75%, 1)",
        lang: "slang",
        code: `LinearStops<3> curve = LinearStops<3>({ float2(0.0, 0.0), float2(0.75, 0.25), float2(1.0, 1.0) });
float eased = curve.at(t);`,
      },
    ],
    notes: [
      "Each stop carries its input: CSS spreads the inputs you leave out evenly, so write them as CSS would have resolved them.",
      "An input below an earlier one is raised to it, as in CSS; two stops at one input make a jump. Outside the stops the first and last segments extend.",
      "The stops are looped over at compile time, so keep `N` small.",
    ],
    related: ["i-easing"],
  },
  {
    slug: "spring",
    name: "Spring",
    summary: "A spring from 0 to rest at 1, by perceptual `duration` and `bounce`. `at` takes seconds, not progress.",
    since,
    params: [
      {
        name: "duration",
        type: "float",
        description: "Seconds: the period of the undamped spring (SwiftUI's `response`).",
      },
      {
        name: "bounce",
        type: "float",
        description: "−1..1: 0 is critically damped, 0.3 a light overshoot, 1 never settles, below 0 overdamped.",
        optional: true,
        default: "0.0",
      },
    ],
    notes: [
      "Mass 1, stiffness (2π / duration)², damping ratio `1 − bounce` (or `1 / (1 + bounce)` below 0), started at rest: the closed-form solution, the same at any frame rate.",
      "It settles visibly within about `duration` for a bounce of 0.3 or less.",
    ],
    related: ["i-easing", "progress"],
  },
  penner("Quad", "Quadratic: t², and its out and in-out forms."),
  penner("Cubic", "Cubic: t³, and its out and in-out forms."),
  penner("Quart", "Quartic: t⁴, and its out and in-out forms."),
  penner("Quint", "Quintic: t⁵, and its out and in-out forms."),
  penner("Sine", "A quarter of a sine wave, and its out and in-out forms."),
  penner("Expo", "Exponential: 2^(10t − 10), and its out and in-out forms.", [
    "The ends are exactly 0 and 1, not the formula's 2⁻¹⁰.",
  ]),
  penner("Circ", "A quarter circle, and its out and in-out forms."),
  penner("Back", "Pulls back past the start (about −0.1) before going, and its out and in-out forms.", [
    "Penner's overshoot 1.70158; the in-out form scales it by 1.525.",
  ]),
  penner("Elastic", "A spring-like oscillation, and its out and in-out forms.", [
    "Overshoots 1 several times; use `Spring` for a physical one.",
  ]),
  penner("Bounce", "A ball bouncing to rest, and its out and in-out forms."),
  {
    slug: "linearstep",
    name: "linearstep",
    since,
    notes: ["Equal edges make a hard step at `edge0`, never a division by zero."],
    related: ["smootherstep", "remap"],
  },
  {
    slug: "smootherstep",
    name: "smootherstep",
    since,
    related: ["linearstep", "inverse-smoothstep"],
  },
  {
    slug: "inverse-smoothstep",
    name: "inverseSmoothstep",
    summary: "The `x` in 0..1 whose `smoothstep(0, 1, x)` is `y`; `inverseSmootherstep` undoes `smootherstep`.",
    since,
    notes: [
      "`y` is clamped to 0..1. `inverseSmoothstep` is closed form polished by one Newton step; `inverseSmootherstep` takes eight safeguarded Newton steps from it, within 1e-6.",
    ],
    aliases: ["inverseSmootherstep"],
    related: ["smootherstep"],
  },
  {
    slug: "progress",
    name: "progress",
    since,
    params: [
      { name: "time", type: "float", description: "Now: `input.time`." },
      {
        name: "start",
        type: "float",
        description: "When it began, on the effect clock: a `[SkAnimatedFor]` field or a `[SkPointerTimes]` component.",
      },
      { name: "duration", type: "float", description: "Seconds; 0 or less steps from 0 to 1 at `start`." },
    ],
    notes: ["Pass the result through an easing: `CubicBezier.easeOut().at(progress(input.time, clickedAt, 0.4))`."],
    related: ["sk-animated-for", "sk-pointer-times", "attack-hold-release"],
  },
  {
    slug: "loop",
    name: "loop",
    since,
    notes: ["Precision follows `time`: after about 1000 s a float carries 6e-5 s, still far below a frame."],
    related: ["ping-pong", "floor-mod"],
  },
  {
    slug: "ping-pong",
    name: "pingPong",
    since,
    notes: ["Ease it for a smooth turn: `easeInOutSine(pingPong(input.time, 1.2))`."],
    related: ["loop"],
  },
  {
    slug: "pulse",
    name: "pulse",
    since,
    notes: ["Rises over `width / 2` and falls over `width / 2`, each a smoothstep. A width of 0 never fires."],
    related: ["loop"],
  },
  {
    slug: "attack-hold-release",
    name: "attackHoldRelease",
    since,
    notes: [
      "The ramps are linear; ease the result (`easeOutCubic(env)`) for a curved one. A phase of 0 seconds is a jump.",
    ],
    related: ["progress"],
  },
];

export const ease = section("sk.ease", drafts);
