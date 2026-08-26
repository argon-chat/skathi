// aurora — an APP-authored effect.
//
//     --shading: aurora(<intensity>, <angle>?, <tint>?)
//
// A soft, banded gradient sheen laid across the element, brightest away from
// the edges. Every parameter is a plain float, so `transition: --shading 600ms`
// interpolates the whole look — colour included — component by component.

import vn;

[VnEffect("aurora")]
[VnDoc("layered gradient sheen with a soft edge falloff")]
struct Aurora : IEffect
{
    /// Overall opacity of the sheen.
    [VnDefault("0.8")] [VnRange(0.0, 1.0)]
    float intensity;

    /// Direction of the gradient, in TURNS (0 = left-to-right, 0.25 = bottom
    /// to top). Optional, and a `<number>` — the parameter after it is a
    /// `<color>`, so skipping it stays unambiguous.
    [VnOptional] [VnDefault("0.12")] [VnRange(-1.0, 1.0)]
    float angle;

    [VnColor] [VnOptional] [VnDefault("#7f5cff")]
    float4 tint;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        // `centered()` is aspect-corrected, so the bands stay at the requested
        // angle instead of shearing with the element's proportions.
        float2 p = vn_rotate(input.centered(), angle * 6.28318530718);

        // Two octaves of value noise warp the band edges just enough to stop
        // them reading as a linear gradient.
        float warp = vn_fbm(p * 1.3 + float2(0.0, p.y * 0.4)) - 0.5;
        float t = saturate(p.x * 0.5 + 0.5 + warp * 0.35);

        // Inigo Quilez's cosine palette, biased toward the author's tint. The
        // palette supplies the hue travel; the tint supplies the identity.
        float3 sweep = vn_palette(
            t,
            float3(0.58, 0.52, 0.68),
            float3(0.42, 0.44, 0.40),
            float3(1.00, 1.00, 1.00),
            float3(0.00, 0.18, 0.38));
        float3 rgb = lerp(sweep, tint.rgb, 0.40);

        // Fade in over ~20 logical px from the rounded edge, so the sheen sits
        // INSIDE the element rather than painting its border.
        float inset = saturate(-input.sd / max(20.0 * input.scale, 1.0));
        float body = 0.42 + 0.58 * inset;

        // A brighter ridge where the noise-warped bands crowd together.
        float ridge = saturate(1.0 - abs(t * 2.0 - 1.0));
        ridge = ridge * ridge;

        float alpha = intensity * tint.a * body * (0.55 + 0.45 * ridge);
        return float4(rgb, saturate(alpha));
    }
};
