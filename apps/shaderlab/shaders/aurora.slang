import sk;

[SkEffect("aurora")]
[SkDoc("layered gradient sheen with a soft edge falloff")]
struct Aurora : IEffect
{
    [SkDefault("0.8")] [SkRange(0.0, 1.0)]
    float intensity;

    [SkOptional] [SkDefault("0.12")] [SkRange(-1.0, 1.0)]
    float angle;

    [SkColor] [SkOptional] [SkDefault("#7f5cff")]
    float4 tint;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        // `centered()` is aspect-corrected, so the bands stay at the requested angle instead of
        // shearing with the element's proportions.
        float2 p = sk_rotate(input.centered(), angle * 6.28318530718);

        float warp = sk_fbm(p * 1.3 + float2(0.0, p.y * 0.4)) - 0.5;
        float t = saturate(p.x * 0.5 + 0.5 + warp * 0.35);

        float3 sweep = sk_palette(
            t,
            float3(0.58, 0.52, 0.68),
            float3(0.42, 0.44, 0.40),
            float3(1.00, 1.00, 1.00),
            float3(0.00, 0.18, 0.38));
        float3 rgb = lerp(sweep, tint.rgb, 0.40);

        // Fade in over ~20 logical px from the rounded edge, so the sheen sits INSIDE the element
        // rather than painting its border.
        float inset = saturate(-input.sd / max(20.0 * input.scale, 1.0));
        float body = 0.42 + 0.58 * inset;

        float ridge = saturate(1.0 - abs(t * 2.0 - 1.0));
        ridge = ridge * ridge;

        float alpha = intensity * tint.a * body * (0.55 + 0.45 * ridge);
        return float4(rgb, saturate(alpha));
    }
};
