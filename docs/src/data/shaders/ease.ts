import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const T = { name: "t", type: "float", description: "Progress; clamped to 0..1." };

const penner = (family: string, summary: string, notes?: string[]): Draft => ({
  slug: `ease-${family.toLowerCase()}`,
  name: `easeIn${family}`,
  kind: "function",
  summary,
  since,
  syntax: `float easeIn${family}(float t);
float easeOut${family}(float t);
float easeInOut${family}(float t);`,
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
    kind: "interface",
    summary:
      "An easing as a value: `float at(float t)`. `CubicBezier`, `Steps`, `LinearStops` and `Spring` implement it.",
    since,
    syntax: `import sk.ease;

interface IEasing
{
    float at(float t);
};`,
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
    kind: "struct",
    summary: "CSS `cubic-bezier(x1, y1, x2, y2)` and the keywords `ease`, `ease-in`, `ease-out` and `ease-in-out`.",
    since,
    syntax: `CubicBezier(float x1, float y1, float x2, float y2);
static CubicBezier CubicBezier.ease();        // cubic-bezier(0.25, 0.1, 0.25, 1)
static CubicBezier CubicBezier.easeIn();      // cubic-bezier(0.42, 0, 1, 1)
static CubicBezier CubicBezier.easeOut();     // cubic-bezier(0, 0, 0.58, 1)
static CubicBezier CubicBezier.easeInOut();   // cubic-bezier(0.42, 0, 0.58, 1)
float at(float t);`,
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
    kind: "struct",
    summary: "CSS `steps(count, position)`: `count` equal steps from 0 to 1.",
    since,
    syntax: `Steps(int count, StepPosition position = StepPosition.JumpEnd);
float at(float t);`,
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
    kind: "enum",
    summary:
      "The `<step-position>` of `steps()`: `JumpStart`, `JumpEnd`, `JumpNone`, `JumpBoth`, and `Start` and `End`.",
    since,
    syntax: `enum StepPosition { JumpStart, JumpEnd, JumpNone, JumpBoth, Start = JumpStart, End = JumpEnd }`,
    related: ["steps"],
  },
  {
    slug: "linear-stops",
    name: "LinearStops",
    kind: "struct",
    summary: "CSS `linear()` with `N` stops: straight segments between (input, output) points.",
    since,
    syntax: `LinearStops<let N : int>(float2 points[N]);
float at(float t);

// linear(0, 0.25 75%, 1)
LinearStops<3>({ float2(0.0, 0.0), float2(0.75, 0.25), float2(1.0, 1.0) })`,
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
    kind: "struct",
    summary: "A spring from 0 to rest at 1, by perceptual `duration` and `bounce`. `at` takes seconds, not progress.",
    since,
    syntax: `Spring(float duration, float bounce = 0.0);
float at(float seconds);
float dampingRatio();`,
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
    kind: "function",
    summary: "0 below `edge0`, 1 above `edge1`, linear between: `smoothstep` without the curve.",
    since,
    syntax: `float linearstep(float edge0, float edge1, float x);`,
    notes: ["Equal edges make a hard step at `edge0`, never a division by zero."],
    related: ["smootherstep", "remap"],
  },
  {
    slug: "smootherstep",
    name: "smootherstep",
    kind: "function",
    summary: "Perlin's 6x⁵ − 15x⁴ + 10x³ between two edges: `smoothstep` with flat second derivatives at the ends too.",
    since,
    syntax: `float smootherstep(float edge0, float edge1, float x);`,
    related: ["linearstep", "inverse-smoothstep"],
  },
  {
    slug: "inverse-smoothstep",
    name: "inverseSmoothstep",
    kind: "function",
    summary: "The `x` in 0..1 whose `smoothstep(0, 1, x)` is `y`; `inverseSmootherstep` undoes `smootherstep`.",
    since,
    syntax: `float inverseSmoothstep(float y);
float inverseSmootherstep(float y);`,
    notes: [
      "`y` is clamped to 0..1. `inverseSmoothstep` is closed form polished by one Newton step; `inverseSmootherstep` takes eight safeguarded Newton steps from it, within 1e-6.",
    ],
    aliases: ["inverseSmootherstep"],
    related: ["smootherstep"],
  },
  {
    slug: "progress",
    name: "progress",
    kind: "function",
    summary: "0..1 through `duration` seconds that began at `start`: 0 before, 1 after.",
    since,
    syntax: `float progress(float time, float start, float duration);`,
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
    kind: "function",
    summary: "0..1 through each `period` seconds, restarting: a sawtooth.",
    since,
    syntax: `float loop(float time, float period);`,
    notes: ["Precision follows `time`: after about 1000 s a float carries 6e-5 s, still far below a frame."],
    related: ["ping-pong", "floor-mod"],
  },
  {
    slug: "ping-pong",
    name: "pingPong",
    kind: "function",
    summary: "0 to 1 over `period` seconds, back over the next: `animation-direction: alternate` as a triangle wave.",
    since,
    syntax: `float pingPong(float time, float period);`,
    notes: ["Ease it for a smooth turn: `easeInOutSine(pingPong(input.time, 1.2))`."],
    related: ["loop"],
  },
  {
    slug: "pulse",
    name: "pulse",
    kind: "function",
    summary: "A smooth 0..1..0 bump `width` seconds wide at the start of every `period`; 0 between bumps.",
    since,
    syntax: `float pulse(float time, float period, float width);`,
    notes: ["Rises over `width / 2` and falls over `width / 2`, each a smoothstep. A width of 0 never fires."],
    related: ["loop"],
  },
  {
    slug: "attack-hold-release",
    name: "attackHoldRelease",
    kind: "function",
    summary: "An envelope from `start`: up to 1 over `attack` seconds, held for `hold`, back to 0 over `release`.",
    since,
    syntax: `float attackHoldRelease(float time, float start, float attack, float hold, float release);`,
    notes: [
      "The ramps are linear; ease the result (`easeOutCubic(env)`) for a curved one. A phase of 0 seconds is a jump.",
    ],
    related: ["progress"],
  },
];

export const ease = section("sk.ease", drafts);
