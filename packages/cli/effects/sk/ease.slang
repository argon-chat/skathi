// sk.ease — easing curves and time envelopes.
//
//     import sk;
//     import sk.ease;
//
//     float t = progress(input.time, clickTime, 0.6);        // 0..1 over 0.6 s
//     float r = CubicBezier.easeOut().at(t) * maxRadius;     // CSS `ease-out`
//     float fade = 1.0 - easeInCubic(t);
//
// Easings map progress `t` to eased progress. The CSS ones (`CubicBezier`, `Steps`, `LinearStops`)
// follow CSS Easing Level 2 outside 0..1 too; the Penner set (`easeInQuad` ... `easeInOutBounce`)
// clamps `t` to 0..1. Times are seconds, as `input.time` and `[SkPointerTimes]` are. Every loop
// here has a fixed trip count.

module "sk.ease";

import sk.math;

/// An easing: progress in, eased progress out. 0 maps to 0 and 1 to 1 for every curve here
/// except `Spring`, whose input is seconds.
public interface IEasing
{
    float at(float t);
};

// ===========================================================================
//  CSS easing functions
// ===========================================================================

/// The `t` at which the bezier's x reaches `x`, x in (0, 1): ten safeguarded Newton steps
/// (bisection when a step leaves the bracket), an x residual below 2e-6 for any x1, x2 in 0..1.
float sk_bezier_t(float x1, float x2, float x)
{
    float cx = 3.0 * x1;
    float bx = 3.0 * (x2 - x1) - cx;
    float ax = 1.0 - cx - bx;
    float t = x;
    float lo = 0.0;
    float hi = 1.0;
    [loop]
    for (int i = 0; i < 10; i++)
    {
        float v = ((ax * t + bx) * t + cx) * t - x;
        lo = v > 0.0 ? lo : t;
        hi = v > 0.0 ? t : hi;
        float d = (3.0 * ax * t + 2.0 * bx) * t + cx;
        float next = t - v / (d > 1e-6 ? d : 1.0);
        t = v == 0.0 ? t : (d > 1e-6 && next >= lo && next <= hi ? next : 0.5 * (lo + hi));
    }
    return t;
}

float sk_bezier(float p1, float p2, float t)
{
    float u = 1.0 - t;
    return 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t;
}

/// CSS `cubic-bezier(x1, y1, x2, y2)`. x1 and x2 are clamped to 0..1 as CSS requires; y may
/// overshoot. Outside 0..1 the curve continues along its end tangents, as CSS defines.
public struct CubicBezier : IEasing
{
    public float x1;
    public float y1;
    public float x2;
    public float y2;

    public __init(float x1, float y1, float x2, float y2)
    {
        this.x1 = saturate(x1);
        this.y1 = y1;
        this.x2 = saturate(x2);
        this.y2 = y2;
    }

    /// CSS `ease`: cubic-bezier(0.25, 0.1, 0.25, 1).
    public static CubicBezier ease() { return CubicBezier(0.25, 0.1, 0.25, 1.0); }
    /// CSS `ease-in`: cubic-bezier(0.42, 0, 1, 1).
    public static CubicBezier easeIn() { return CubicBezier(0.42, 0.0, 1.0, 1.0); }
    /// CSS `ease-out`: cubic-bezier(0, 0, 0.58, 1).
    public static CubicBezier easeOut() { return CubicBezier(0.0, 0.0, 0.58, 1.0); }
    /// CSS `ease-in-out`: cubic-bezier(0.42, 0, 0.58, 1).
    public static CubicBezier easeInOut() { return CubicBezier(0.42, 0.0, 0.58, 1.0); }

    public float at(float t)
    {
        float before = this.x1 > 0.0 ? this.y1 / (this.x1 > 0.0 ? this.x1 : 1.0)
            : (this.x2 > 0.0 ? this.y2 / (this.x2 > 0.0 ? this.x2 : 1.0) : 0.0);
        float after = this.x2 < 1.0 ? (this.y2 - 1.0) / (this.x2 < 1.0 ? this.x2 - 1.0 : 1.0)
            : (this.x1 < 1.0 ? (this.y1 - 1.0) / (this.x1 < 1.0 ? this.x1 - 1.0 : 1.0) : 0.0);
        float inside = sk_bezier(this.y1, this.y2, sk_bezier_t(this.x1, this.x2, t));
        return t <= 0.0 ? before * t : (t >= 1.0 ? 1.0 + after * (t - 1.0) : inside);
    }
};

/// The `<step-position>` of CSS `steps()`. `Start` and `End` are `JumpStart` and `JumpEnd`.
public enum StepPosition
{
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
    Start = 0,
    End = 1,
}

/// CSS `steps(count, position)`: 0 before 0, 1 from 1 on, `count` equal intervals between whose
/// values `position` picks. The before flag is taken as unset.
public struct Steps : IEasing
{
    public int count;
    public StepPosition position;

    public __init(int count, StepPosition position = StepPosition.JumpEnd)
    {
        this.count = max(count, 1);
        this.position = position;
    }

    public float at(float t)
    {
        float n = float(this.count);
        float jumps = this.position == StepPosition.JumpBoth ? n + 1.0 : (this.position == StepPosition.JumpNone ? n - 1.0 : n);
        jumps = max(jumps, 1.0);
        float lead = this.position == StepPosition.JumpStart || this.position == StepPosition.JumpBoth ? 1.0 : 0.0;
        float step = min(floor(t * n) + lead, jumps);
        return t < 0.0 ? 0.0 : (t >= 1.0 ? 1.0 : step / jumps);
    }
};

/// CSS `linear()` with `N` stops, each (input, output) with the input given, as in
/// `linear(0, 0.25 75%, 1)` -> `{ float2(0, 0), float2(0.75, 0.25), float2(1, 1) }`. An input
/// below an earlier one is raised to it, as CSS does; outside the stops the end segments extend.
public struct LinearStops<let N : int> : IEasing
{
    public float2 points[N];

    public __init(float2 points[N])
    {
        this.points = points;
    }

    public float at(float t)
    {
        float2 first = this.points[0];
        float2 next = this.points[min(1, N - 1)];
        float2 second = float2(max(next.x, first.x), next.y);
        float2 below = first;
        float2 above = first;
        float2 penultimate = first;
        float2 last = first;
        bool foundBelow = false;
        bool foundAbove = false;
        bool matched = false;
        float matchedOutput = 0.0;
        float highest = first.x;
        [ForceUnroll]
        for (int i = 0; i < N; i++)
        {
            float2 p = float2(max(this.points[i].x, highest), this.points[i].y);
            highest = p.x;
            matched = matched || p.x == t;
            matchedOutput = p.x == t ? p.y : matchedOutput;
            below = p.x < t ? p : below;
            foundBelow = foundBelow || p.x < t;
            above = !foundAbove && p.x > t ? p : above;
            foundAbove = foundAbove || p.x > t;
            penultimate = last;
            last = p;
        }
        float2 a = !foundBelow ? first : (!foundAbove ? penultimate : below);
        float2 b = !foundBelow ? second : (!foundAbove ? last : above);
        float span = b.x - a.x;
        float along = a.y + (b.y - a.y) * (t - a.x) / (span != 0.0 ? span : 1.0);
        float flat = !foundBelow ? a.y : b.y;
        float interpolated = span != 0.0 ? along : flat;
        return N == 1 ? first.y : (matched ? matchedOutput : interpolated);
    }
};

/// A spring from 0 to 1 at rest, by perceptual `duration` (seconds, the undamped period, SwiftUI's
/// `response`) and `bounce` (-1..1: 0 is critically damped, 0.3 a light overshoot, below 0
/// overdamped). `at` takes SECONDS since release, not progress.
public struct Spring : IEasing
{
    public float duration;
    public float bounce;

    public __init(float duration, float bounce = 0.0)
    {
        this.duration = max(duration, 1e-4);
        this.bounce = clamp(bounce, -0.99, 1.0);
    }

    /// The damping ratio: 1 - bounce for bounce >= 0, 1 / (1 + bounce) below.
    public float dampingRatio()
    {
        return this.bounce >= 0.0 ? 1.0 - this.bounce : 1.0 / (1.0 + this.bounce);
    }

    public float at(float seconds)
    {
        float t = max(seconds, 0.0);
        float w = SK_TAU / this.duration;
        float z = this.dampingRatio();
        float decay = exp(-z * w * t);
        float x = 1.0 - decay * (1.0 + w * t);
        if (z < 0.999)
        {
            float wd = w * sqrt(max(1.0 - z * z, 0.0));
            x = 1.0 - decay * (cos(wd * t) + z * w / max(wd, 1e-6) * sin(wd * t));
        }
        else if (z > 1.001)
        {
            // cosh and sinh through the slower exponent alone, so a large w t cannot overflow.
            float r = w * sqrt(max(z * z - 1.0, 0.0));
            float slow = exp((r - z * w) * t);
            float fast = exp((-r - z * w) * t);
            x = 1.0 - 0.5 * (slow + fast) - 0.5 * z * w / max(r, 1e-6) * (slow - fast);
        }
        return x;
    }
};

// ===========================================================================
//  Penner's easings, as named on easings.net. `t` is clamped to 0..1.
// ===========================================================================

float sk_ease_bounce(float t)
{
    static const float n = 7.5625;
    static const float d = 2.75;
    float a = n * t * t;
    float b = n * (t - 1.5 / d) * (t - 1.5 / d) + 0.75;
    float c = n * (t - 2.25 / d) * (t - 2.25 / d) + 0.9375;
    float e = n * (t - 2.625 / d) * (t - 2.625 / d) + 0.984375;
    return t < 1.0 / d ? a : (t < 2.0 / d ? b : (t < 2.5 / d ? c : e));
}

/// t².
public float easeInQuad(float t) { t = saturate(t); return t * t; }
/// 1 − (1 − t)².
public float easeOutQuad(float t) { t = saturate(t); return 1.0 - (1.0 - t) * (1.0 - t); }
/// Quad in, then out, meeting at 0.5.
public float easeInOutQuad(float t)
{
    t = saturate(t);
    return t < 0.5 ? 2.0 * t * t : 1.0 - pow(-2.0 * t + 2.0, 2.0) * 0.5;
}

/// t³.
public float easeInCubic(float t) { t = saturate(t); return t * t * t; }
/// 1 − (1 − t)³.
public float easeOutCubic(float t) { t = saturate(t); float u = 1.0 - t; return 1.0 - u * u * u; }
/// Cubic in, then out.
public float easeInOutCubic(float t)
{
    t = saturate(t);
    float u = -2.0 * t + 2.0;
    return t < 0.5 ? 4.0 * t * t * t : 1.0 - u * u * u * 0.5;
}

/// t⁴.
public float easeInQuart(float t) { t = saturate(t); return t * t * t * t; }
/// 1 − (1 − t)⁴.
public float easeOutQuart(float t) { t = saturate(t); float u = 1.0 - t; return 1.0 - u * u * u * u; }
/// Quart in, then out.
public float easeInOutQuart(float t)
{
    t = saturate(t);
    float u = -2.0 * t + 2.0;
    return t < 0.5 ? 8.0 * t * t * t * t : 1.0 - u * u * u * u * 0.5;
}

/// t⁵.
public float easeInQuint(float t) { t = saturate(t); return t * t * t * t * t; }
/// 1 − (1 − t)⁵.
public float easeOutQuint(float t) { t = saturate(t); float u = 1.0 - t; return 1.0 - u * u * u * u * u; }
/// Quint in, then out.
public float easeInOutQuint(float t)
{
    t = saturate(t);
    float u = -2.0 * t + 2.0;
    return t < 0.5 ? 16.0 * t * t * t * t * t : 1.0 - u * u * u * u * u * 0.5;
}

/// 1 − cos(t·π/2).
public float easeInSine(float t) { return 1.0 - cos(saturate(t) * SK_PI * 0.5); }
/// sin(t·π/2).
public float easeOutSine(float t) { return sin(saturate(t) * SK_PI * 0.5); }
/// (1 − cos(t·π)) / 2.
public float easeInOutSine(float t) { return 0.5 - 0.5 * cos(saturate(t) * SK_PI); }

/// 2^(10t − 10), exactly 0 at 0.
public float easeInExpo(float t) { t = saturate(t); return t <= 0.0 ? 0.0 : exp2(10.0 * t - 10.0); }
/// 1 − 2^(−10t), exactly 1 at 1.
public float easeOutExpo(float t) { t = saturate(t); return t >= 1.0 ? 1.0 : 1.0 - exp2(-10.0 * t); }
/// Expo in, then out; exactly 0 and 1 at the ends.
public float easeInOutExpo(float t)
{
    t = saturate(t);
    float v = t < 0.5 ? exp2(20.0 * t - 10.0) * 0.5 : (2.0 - exp2(-20.0 * t + 10.0)) * 0.5;
    return t <= 0.0 ? 0.0 : (t >= 1.0 ? 1.0 : v);
}

/// 1 − √(1 − t²), a quarter circle.
public float easeInCirc(float t) { t = saturate(t); return 1.0 - sqrt(max(1.0 - t * t, 0.0)); }
/// √(1 − (t − 1)²).
public float easeOutCirc(float t) { t = saturate(t); return sqrt(max(1.0 - (t - 1.0) * (t - 1.0), 0.0)); }
/// Circ in, then out.
public float easeInOutCirc(float t)
{
    t = saturate(t);
    float a = (1.0 - sqrt(max(1.0 - 4.0 * t * t, 0.0))) * 0.5;
    float b = (sqrt(max(1.0 - (-2.0 * t + 2.0) * (-2.0 * t + 2.0), 0.0)) + 1.0) * 0.5;
    return t < 0.5 ? a : b;
}

/// Pulls back below 0 (about −0.1) before going to 1; Penner's overshoot 1.70158.
public float easeInBack(float t)
{
    static const float c1 = 1.70158;
    t = saturate(t);
    return (c1 + 1.0) * t * t * t - c1 * t * t;
}

/// Overshoots past 1 (about 1.1) and settles.
public float easeOutBack(float t)
{
    static const float c1 = 1.70158;
    float u = saturate(t) - 1.0;
    return 1.0 + (c1 + 1.0) * u * u * u + c1 * u * u;
}

/// Back in, then out, with the overshoot scaled by 1.525.
public float easeInOutBack(float t)
{
    static const float c2 = 1.70158 * 1.525;
    t = saturate(t);
    float a = 4.0 * t * t * ((c2 + 1.0) * 2.0 * t - c2) * 0.5;
    float u = 2.0 * t - 2.0;
    float b = (u * u * ((c2 + 1.0) * u + c2) + 2.0) * 0.5;
    return t < 0.5 ? a : b;
}

/// A growing oscillation into 1; exactly 0 and 1 at the ends.
public float easeInElastic(float t)
{
    t = saturate(t);
    float v = -exp2(10.0 * t - 10.0) * sin((t * 10.0 - 10.75) * (SK_TAU / 3.0));
    return t <= 0.0 ? 0.0 : (t >= 1.0 ? 1.0 : v);
}

/// An oscillation settling at 1; exactly 0 and 1 at the ends.
public float easeOutElastic(float t)
{
    t = saturate(t);
    float v = exp2(-10.0 * t) * sin((t * 10.0 - 0.75) * (SK_TAU / 3.0)) + 1.0;
    return t <= 0.0 ? 0.0 : (t >= 1.0 ? 1.0 : v);
}

/// Elastic in, then out.
public float easeInOutElastic(float t)
{
    t = saturate(t);
    float s = sin((20.0 * t - 11.125) * (SK_TAU / 4.5));
    float v = t < 0.5 ? -(exp2(20.0 * t - 10.0) * s) * 0.5 : exp2(-20.0 * t + 10.0) * s * 0.5 + 1.0;
    return t <= 0.0 ? 0.0 : (t >= 1.0 ? 1.0 : v);
}

/// Bounces up to 1 from below, like a ball dropped in reverse.
public float easeInBounce(float t) { return 1.0 - sk_ease_bounce(1.0 - saturate(t)); }
/// Falls to 1 and bounces three times, each lower.
public float easeOutBounce(float t) { return sk_ease_bounce(saturate(t)); }
/// Bounce in, then out.
public float easeInOutBounce(float t)
{
    t = saturate(t);
    return t < 0.5 ? (1.0 - sk_ease_bounce(1.0 - 2.0 * t)) * 0.5 : (1.0 + sk_ease_bounce(2.0 * t - 1.0)) * 0.5;
}

// ===========================================================================
//  Steps between ranges
// ===========================================================================

/// 0 below `edge0`, 1 above `edge1`, linear between (a clamped `remap` to 0..1). Equal edges
/// give a hard step at `edge0`.
public float linearstep(float edge0, float edge1, float x)
{
    float span = edge1 - edge0;
    return span != 0.0 ? saturate((x - edge0) / (span != 0.0 ? span : 1.0)) : (x < edge0 ? 0.0 : 1.0);
}

/// Ken Perlin's 6x⁵ − 15x⁴ + 10x³ between `edge0` and `edge1`, clamped like `smoothstep`; its
/// first and second derivatives are 0 at both ends.
public float smootherstep(float edge0, float edge1, float x)
{
    float t = linearstep(edge0, edge1, x);
    return t * t * t * (t * (6.0 * t - 15.0) + 10.0);
}

/// The `x` in 0..1 whose `smoothstep(0, 1, x)` is `y` (y clamped to 0..1): the closed form,
/// polished by one Newton step against the GPU's approximate `asin` and `sin`.
public float inverseSmoothstep(float y)
{
    y = saturate(y);
    float x = 0.5 - sin(asin(1.0 - 2.0 * y) / 3.0);
    float d = 6.0 * x * (1.0 - x);
    float polished = saturate(x - (x * x * (3.0 - 2.0 * x) - y) / max(d, 1e-6));
    return d > 1e-3 ? polished : x;
}

/// The `x` in 0..1 whose `smootherstep(0, 1, x)` is `y` (y clamped to 0..1): eight safeguarded
/// Newton steps from `inverseSmoothstep(y)`, within 1e-6.
public float inverseSmootherstep(float y)
{
    y = saturate(y);
    float x = inverseSmoothstep(y);
    float lo = 0.0;
    float hi = 1.0;
    [loop]
    for (int i = 0; i < 8; i++)
    {
        float v = x * x * x * (x * (6.0 * x - 15.0) + 10.0) - y;
        lo = v > 0.0 ? lo : x;
        hi = v > 0.0 ? x : hi;
        float d = 30.0 * x * x * (x - 1.0) * (x - 1.0);
        float next = x - v / (d > 1e-6 ? d : 1.0);
        x = v == 0.0 ? x : (d > 1e-6 && next >= lo && next <= hi ? next : 0.5 * (lo + hi));
    }
    return y <= 0.0 || y >= 1.0 ? y : x;
}

// ===========================================================================
//  Time envelopes, in seconds
// ===========================================================================

/// 0..1 progress through `duration` seconds that began at `start`: 0 before, 1 after. A
/// duration of 0 or less steps from 0 to 1 at `start`.
public float progress(float time, float start, float duration)
{
    return linearstep(start, start + max(duration, 0.0), time);
}

/// 0..1 through each `period`, restarting: a sawtooth, 0 at whole periods.
public float loop(float time, float period)
{
    return period > 0.0 ? floorMod(time, period) / period : 0.0;
}

/// 0 to 1 over `period`, back to 0 over the next, and again: a triangle wave, like a CSS
/// animation with `animation-direction: alternate`.
public float pingPong(float time, float period)
{
    float t = loop(time, 2.0 * period);
    return 1.0 - abs(2.0 * t - 1.0);
}

/// A smooth 0..1..0 bump `width` seconds wide at the start of every `period`; 0 between bumps.
public float pulse(float time, float period, float width)
{
    float w = clamp(width, 0.0, max(period, 0.0));
    float x = period > 0.0 ? floorMod(time, period) : time;
    float half = 0.5 * w;
    return w > 0.0 ? 1.0 - smoothstep(0.0, half, abs(x - half)) : 0.0;
}

/// An envelope from `start`: 0 to 1 linearly over `attack` seconds, 1 for `hold`, back to 0 over
/// `release`, then 0. Ease each phase by passing the result through an easing.
public float attackHoldRelease(float time, float start, float attack, float hold, float release)
{
    float t = time - start;
    float up = linearstep(0.0, max(attack, 0.0), t);
    float down = 1.0 - linearstep(max(attack, 0.0) + max(hold, 0.0), max(attack, 0.0) + max(hold, 0.0) + max(release, 0.0), t);
    return t < 0.0 ? 0.0 : min(up, down);
}
