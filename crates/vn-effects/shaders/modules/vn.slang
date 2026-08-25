// vn — the engine prelude every built-in effect imports.
//
// An effect author writes ONE thing: a `struct <Name> : IEffect` with a
// `shade()` method. No entry point, no semantics, no register bindings — the
// engine appends a generated wrapper after the user source (see
// `packages/vue-native/shaders.ts`), which owns the instance fetch, the
// rounded-rect coverage and the premultiplied clamp.
//
// Rules that the wrapper cannot enforce for you:
//   * Never call `Sample` inside a loop or any non-uniform branch — implicit
//     derivatives are undefined there. Use `SampleLevel(..., 0)`.
//   * `shade()` returns a PREMULTIPLIED colour (rgb already multiplied by a).
//     `vn_premultiply()` converts a straight-alpha colour for you.
//   * Effects compile at `ps_5_0`; the rest of gpui stays at `ps_4_1`.

module vn;

/// Everything the wrapper hands an effect, in one struct.
public struct EffectInput
{
    /// Screen-space UV of this pixel (0..1 across the whole window). This is
    /// the coordinate `backdrop` was sampled at.
    public float2 uv;
    /// Position within the element box, normalised 0..1. Values outside 0..1
    /// occur when the effect asked for a `bleed` (e.g. an outer glow).
    public float2 local;
    /// Position within the element box in device pixels (top-left origin).
    public float2 px;
    /// Element box size in device pixels.
    public float2 size;
    /// The backdrop behind this quad, premultiplied. Blurred when the effect
    /// declares a backdrop blur radius, otherwise the raw snapshot. All zero
    /// for effects that do not request the backdrop.
    public float4 backdrop;
    /// Signed distance to the element's rounded rect in device pixels:
    /// negative inside, 0 on the edge, positive outside.
    public float sd;
    /// Antialiased coverage of the element's rounded rect (`0.5 - sd`,
    /// saturated). 1 well inside, 0 well outside.
    public float coverage;
    /// Seconds since the process started — the clock an animated effect uses.
    public float time;
    /// Device pixels per logical pixel (the DPI scale factor).
    public float scale;
    /// Effect parameters 0..3, in the order the CSS schema declares them.
    public float4 params0;
    /// Effect parameters 4..7.
    public float4 params1;
};

/// The one interface a built-in effect implements.
public interface IEffect
{
    /// Returns the PREMULTIPLIED colour for this pixel.
    float4 shade(EffectInput input);
};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Straight alpha -> premultiplied.
public float4 vn_premultiply(float4 color)
{
    return float4(color.rgb * color.a, color.a);
}

/// Premultiplied `above` over premultiplied `below`.
public float4 vn_over(float4 below, float4 above)
{
    return above + below * (1.0 - above.a);
}

/// Deterministic hash of a 2D point -> 0..1. Cheap, no texture needed.
public float vn_hash12(float2 p)
{
    float3 p3 = frac(float3(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, float3(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return frac((p3.x + p3.y) * p3.z);
}

/// Value noise on a unit lattice, smoothstep-interpolated.
public float vn_value_noise(float2 p)
{
    float2 i = floor(p);
    float2 f = frac(p);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = vn_hash12(i);
    float b = vn_hash12(i + float2(1.0, 0.0));
    float c = vn_hash12(i + float2(0.0, 1.0));
    float d = vn_hash12(i + float2(1.0, 1.0));
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

/// Three octaves of value noise. Unrolled: no `Sample` is involved, but a
/// fixed trip count keeps the generated HLSL flat.
public float vn_fbm(float2 p)
{
    float sum = vn_value_noise(p) * 0.5;
    sum += vn_value_noise(p * 2.03) * 0.25;
    sum += vn_value_noise(p * 4.01) * 0.125;
    return sum / 0.875;
}

/// Relative luminance of a linear-ish RGB triple.
public float vn_luma(float3 rgb)
{
    return dot(rgb, float3(0.2126, 0.7152, 0.0722));
}
