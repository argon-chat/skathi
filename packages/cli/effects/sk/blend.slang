// sk.blend — CSS blend modes and Porter-Duff compositing.
//
//     import sk;
//     import sk.blend;
//
//     float4 tinted = blend<Blend.Overlay>(input.backdrop, sk_premultiply(tint));
//     float4 cut = composite<Composite.DestinationOut>(input.content, mask);
//
// The float4 forms take and return premultiplied colours in the encoding they arrive in. What the
// engine samples (`input.backdrop`, `input.content`, an `SkTexture`) and what `shade` returns are
// sRGB-encoded, which is the space CSS `mix-blend-mode` blends in, so passing them through gives
// CSS's result. The `Color` forms blend sRGB-encoded too; `blendLinear` blends linear light.
//
// The formulas are W3C Compositing and Blending Level 1 (Level 2 for plus-lighter and
// plus-darker). A fully transparent colour blends as transparent black. Values above 1 (HDR)
// pass through every formula except where it is bounded: color-dodge and color-burn stop at 0 and
// 1, the four non-separable modes clip into 0..1, plus-lighter stops at 1 and plus-darker at 0.
// Soft-light reads a backdrop below 0 as 0 under its square root.

module "sk.blend";

import sk.color;

/// A separable or non-separable blend function, B(Cb, Cs) in the spec: straight (not
/// premultiplied) colours in, the blended colour out.
public interface IBlendMode
{
    /// The blended colour of `above` (the source) over `below` (the backdrop), straight rgb.
    static float3 apply(float3 below, float3 above);
};

/// A compositing operator: premultiplied `above` (the source) with premultiplied `below` (the
/// backdrop), premultiplied out.
public interface ICompositeOp
{
    static float4 apply(float4 below, float4 above);
};

float sk_blend_lum(float3 c) { return dot(c, float3(0.3, 0.59, 0.11)); }

/// ClipColor: pulls a colour with a channel outside 0..1 back towards its luminance.
float3 sk_blend_clip(float3 c)
{
    float l = sk_blend_lum(c);
    float n = min(min(c.r, c.g), c.b);
    float x = max(max(c.r, c.g), c.b);
    float below = l - n;
    float above = x - l;
    c = n < 0.0 && below > 0.0 ? l + (c - l) * l / (below > 0.0 ? below : 1.0) : c;
    c = x > 1.0 && above > 0.0 ? l + (c - l) * (1.0 - l) / (above > 0.0 ? above : 1.0) : c;
    return c;
}

float3 sk_blend_lum_set(float3 c, float l)
{
    return sk_blend_clip(c + (l - sk_blend_lum(c)));
}

float sk_blend_sat(float3 c)
{
    return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

/// SetSat: the spec's sort-and-scale, as one expression: min goes to 0, max to `s`, mid between.
float3 sk_blend_sat_set(float3 c, float s)
{
    float lo = min(min(c.r, c.g), c.b);
    float d = sk_blend_sat(c);
    return d > 0.0 ? (c - lo) * (s / (d > 0.0 ? d : 1.0)) : float3(0.0, 0.0, 0.0);
}

float3 sk_blend_multiply(float3 b, float3 s) { return b * s; }
float3 sk_blend_screen(float3 b, float3 s) { return b + s - b * s; }
float3 sk_blend_hard_light(float3 b, float3 s)
{
    return select(s <= 0.5, sk_blend_multiply(b, 2.0 * s), sk_blend_screen(b, 2.0 * s - 1.0));
}

/// The CSS blend modes. Each is a type for `blend<M>`; `M.apply(below, above)` is the bare formula.
public namespace Blend
{
    /// `normal`: the source.
    public struct Normal : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return above; }
    };

    /// `multiply`: Cb × Cs.
    public struct Multiply : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return sk_blend_multiply(below, above); }
    };

    /// `screen`: Cb + Cs − Cb × Cs.
    public struct Screen : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return sk_blend_screen(below, above); }
    };

    /// `overlay`: hard-light with the layers swapped.
    public struct Overlay : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return sk_blend_hard_light(above, below); }
    };

    /// `darken`: min(Cb, Cs) per channel.
    public struct Darken : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return min(below, above); }
    };

    /// `lighten`: max(Cb, Cs) per channel.
    public struct Lighten : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return max(below, above); }
    };

    /// `color-dodge`: Cb / (1 − Cs), at most 1; 0 where Cb <= 0, 1 where Cs >= 1.
    public struct ColorDodge : IBlendMode
    {
        public static float3 apply(float3 below, float3 above)
        {
            float3 room = 1.0 - above;
            float3 dodged = min(1.0, below / select(room > 0.0, room, float3(1.0, 1.0, 1.0)));
            return select(below <= 0.0, float3(0.0, 0.0, 0.0), select(above >= 1.0, float3(1.0, 1.0, 1.0), dodged));
        }
    };

    /// `color-burn`: 1 − (1 − Cb) / Cs, at least 0; 1 where Cb >= 1, 0 where Cs <= 0.
    public struct ColorBurn : IBlendMode
    {
        public static float3 apply(float3 below, float3 above)
        {
            float3 burned = 1.0 - min(1.0, (1.0 - below) / select(above > 0.0, above, float3(1.0, 1.0, 1.0)));
            return select(below >= 1.0, float3(1.0, 1.0, 1.0), select(above <= 0.0, float3(0.0, 0.0, 0.0), burned));
        }
    };

    /// `hard-light`: multiply where Cs <= 0.5, screen above.
    public struct HardLight : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return sk_blend_hard_light(below, above); }
    };

    /// `soft-light`, the W3C formula; Cb below 0 reads as 0 under its square root.
    public struct SoftLight : IBlendMode
    {
        public static float3 apply(float3 below, float3 above)
        {
            float3 b = below;
            float3 d = select(b <= 0.25, ((16.0 * b - 12.0) * b + 4.0) * b, sqrt(max(b, 0.0)));
            return select(above <= 0.5, b - (1.0 - 2.0 * above) * b * (1.0 - b), b + (2.0 * above - 1.0) * (d - b));
        }
    };

    /// `difference`: |Cb − Cs|.
    public struct Difference : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return abs(below - above); }
    };

    /// `exclusion`: Cb + Cs − 2 × Cb × Cs.
    public struct Exclusion : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return below + above - 2.0 * below * above; }
    };

    /// `hue`: the source's hue with the backdrop's saturation and luminosity.
    public struct Hue : IBlendMode
    {
        public static float3 apply(float3 below, float3 above)
        {
            return sk_blend_lum_set(sk_blend_sat_set(above, sk_blend_sat(below)), sk_blend_lum(below));
        }
    };

    /// `saturation`: the source's saturation with the backdrop's hue and luminosity.
    public struct Saturation : IBlendMode
    {
        public static float3 apply(float3 below, float3 above)
        {
            return sk_blend_lum_set(sk_blend_sat_set(below, sk_blend_sat(above)), sk_blend_lum(below));
        }
    };

    /// `color`: the source's hue and saturation with the backdrop's luminosity.
    public struct Color : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return sk_blend_lum_set(above, sk_blend_lum(below)); }
    };

    /// `luminosity`: the source's luminosity with the backdrop's hue and saturation.
    public struct Luminosity : IBlendMode
    {
        public static float3 apply(float3 below, float3 above) { return sk_blend_lum_set(below, sk_blend_lum(above)); }
    };
}

/// Porter-Duff: `above` × Fa + `below` × Fb, colour and alpha alike.
float4 sk_porter_duff(float4 below, float4 above, float2 f)
{
    return above * f.x + below * f.y;
}

/// The Porter-Duff operators of CSS `mask-composite` and canvas `globalCompositeOperation`,
/// plus Level 2's `plus-lighter` and `plus-darker`. Each is a type for `composite<Op>`.
public namespace Composite
{
    /// `clear`: nothing.
    public struct Clear : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(0.0, 0.0)); }
    };

    /// `copy`: the source alone.
    public struct Copy : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(1.0, 0.0)); }
    };

    /// `destination`: the backdrop alone.
    public struct Destination : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(0.0, 1.0)); }
    };

    /// `source-over`: the source over the backdrop, the default everywhere.
    public struct SourceOver : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(1.0, 1.0 - above.a)); }
    };

    /// `destination-over`: the backdrop over the source.
    public struct DestinationOver : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(1.0 - below.a, 1.0)); }
    };

    /// `source-in`: the source where the backdrop is.
    public struct SourceIn : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(below.a, 0.0)); }
    };

    /// `destination-in`: the backdrop where the source is.
    public struct DestinationIn : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(0.0, above.a)); }
    };

    /// `source-out`: the source where the backdrop is not.
    public struct SourceOut : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(1.0 - below.a, 0.0)); }
    };

    /// `destination-out`: the backdrop where the source is not.
    public struct DestinationOut : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(0.0, 1.0 - above.a)); }
    };

    /// `source-atop`: the source over the backdrop, only where the backdrop is.
    public struct SourceAtop : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(below.a, 1.0 - above.a)); }
    };

    /// `destination-atop`: the backdrop over the source, only where the source is.
    public struct DestinationAtop : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(1.0 - below.a, above.a)); }
    };

    /// `xor`: each where the other is not.
    public struct Xor : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above)
        {
            return sk_porter_duff(below, above, float2(1.0 - below.a, 1.0 - above.a));
        }
    };

    /// `lighter`: the sum, unclamped (alpha too).
    public struct Lighter : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return sk_porter_duff(below, above, float2(1.0, 1.0)); }
    };

    /// `plus-lighter`: the sum, colour and alpha at most 1.
    public struct PlusLighter : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above) { return min(above + below, 1.0); }
    };

    /// `plus-darker`: alpha min(1, αs + αb), colour max(0, αo − (αs − cs) − (αb − cb)), Core
    /// Graphics' formula extended to alpha, so a transparent source leaves the backdrop as it was.
    public struct PlusDarker : ICompositeOp
    {
        public static float4 apply(float4 below, float4 above)
        {
            float a = min(1.0, above.a + below.a);
            return float4(max(0.0, a - (above.a - above.rgb) - (below.a - below.rgb)), a);
        }
    };
}

float3 sk_blend_straight(float4 premultiplied)
{
    float a = premultiplied.a;
    return a > 0.0 ? premultiplied.rgb / (a > 0.0 ? a : 1.0) : float3(0.0, 0.0, 0.0);
}

/// CSS `mix-blend-mode: M` of premultiplied `above` over premultiplied `below`: blend, then
/// source-over. Premultiplied out, in the encoding the inputs share.
public float4 blend<M : IBlendMode>(float4 below, float4 above)
{
    float ab = below.a;
    float as = above.a;
    float3 mixed = M.apply(sk_blend_straight(below), sk_blend_straight(above));
    float3 rgb = above.rgb * (1.0 - ab) + below.rgb * (1.0 - as) + as * ab * mixed;
    return float4(rgb, as + ab * (1.0 - as));
}

/// `blend<M>` with `opacity` (0..1) applied to `above` first, like CSS `opacity` on the blended layer.
public float4 blend<M : IBlendMode>(float4 below, float4 above, float opacity)
{
    return blend<M>(below, above * opacity);
}

/// `blend<M>` on `Color`s, in sRGB-encoded values as CSS blends.
public Color blend<M : IBlendMode>(Color below, Color above)
{
    return Color.fromPremultiplied(blend<M>(below.toPremultiplied(), above.toPremultiplied()));
}

/// `blend<M>` on `Color`s in linear light: physically additive modes (screen, multiply) look
/// right there, but the result differs from CSS.
public Color blendLinear<M : IBlendMode>(Color below, Color above)
{
    float4 b = float4(below.linear * below.alpha, below.alpha);
    float4 a = float4(above.linear * above.alpha, above.alpha);
    float4 r = blend<M>(b, a);
    return Color.fromLinear(float4(sk_blend_straight(r), r.a));
}

/// Porter-Duff `Op` of premultiplied `above` (the source) with premultiplied `below` (the backdrop).
public float4 composite<Op : ICompositeOp>(float4 below, float4 above)
{
    return Op.apply(below, above);
}

/// `composite<Op>` on `Color`s, in sRGB-encoded values.
public Color composite<Op : ICompositeOp>(Color below, Color above)
{
    return Color.fromPremultiplied(composite<Op>(below.toPremultiplied(), above.toPremultiplied()));
}
