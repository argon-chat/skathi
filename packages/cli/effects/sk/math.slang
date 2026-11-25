// sk.math — the few scalar helpers the other std modules share.
//
//     import sk.math;
//
//     float t = remapClamped(input.px.x, 0.0, input.size.x, 0.0, 1.0);
//     float a = floorMod(angle, SK_TAU);

module "sk.math";

/// π.
public static const float SK_PI = 3.14159265358979;
/// 2π, one turn in radians.
public static const float SK_TAU = 6.28318530717959;

/// `x` wrapped into [0, y) for y > 0 (GLSL `mod`): unlike HLSL `fmod`, a negative `x` wraps
/// upwards, so -0.25 mod 1 is 0.75.
public float floorMod(float x, float y)
{
    return x - y * floor(x / y);
}

/// Per component `floorMod`.
public vector<float, N> floorMod<let N : int>(vector<float, N> x, vector<float, N> y)
{
    return x - y * floor(x / y);
}

/// `a / b`, or `fallback` where `b` is 0: never an infinity or a NaN from a zero divisor.
public float safeDiv(float a, float b, float fallback = 0.0)
{
    return b != 0.0 ? a / (b != 0.0 ? b : 1.0) : fallback;
}

/// Per component `safeDiv`.
public vector<float, N> safeDiv<let N : int>(vector<float, N> a, vector<float, N> b, float fallback = 0.0)
{
    vector<float, N> one = vector<float, N>(1.0);
    return select(b != 0.0, a / select(b != 0.0, b, one), vector<float, N>(fallback));
}

/// `x` mapped linearly from [from0, from1] to [to0, to1], unclamped. An empty source range
/// (from0 == from1) maps everything to `to0`.
public float remap(float x, float from0, float from1, float to0, float to1)
{
    return to0 + (to1 - to0) * safeDiv(x - from0, from1 - from0);
}

/// `remap`, with the result held inside [to0, to1] (either order).
public float remapClamped(float x, float from0, float from1, float to0, float to1)
{
    return clamp(remap(x, from0, from1, to0, to1), min(to0, to1), max(to0, to1));
}

/// Per component `remap`.
public vector<float, N> remap<let N : int>(vector<float, N> x, vector<float, N> from0, vector<float, N> from1,
    vector<float, N> to0, vector<float, N> to1)
{
    return to0 + (to1 - to0) * safeDiv(x - from0, from1 - from0);
}

/// Per component `remapClamped`.
public vector<float, N> remapClamped<let N : int>(vector<float, N> x, vector<float, N> from0,
    vector<float, N> from1, vector<float, N> to0, vector<float, N> to1)
{
    return clamp(remap(x, from0, from1, to0, to1), min(to0, to1), max(to0, to1));
}
