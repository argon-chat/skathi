// `--shading: noise(<amount>, <scale>?, <speed>?)`
//
// Film grain drawn OVER the element (the element's own background paints
// underneath it). Blending is ordinary source-over, so this composes with any
// background, gradient or image.

import sk;

[SkEffect("noise")]
[SkDoc("film grain over the element")]
struct Noise : IEffect
{
    /// Peak alpha of the grain.
    [SkDefault("0.25")] [SkRange(0.0, 1.0)]
    float amount;

    /// Device pixels per grain cell (1 = one cell per pixel).
    [SkLength] [SkOptional] [SkDefault("1.5")] [SkRange(0.05, 64.0)]
    float scale;

    /// Grain regenerations per second; 0 = a still image, and the only value
    /// that does not keep the window repainting.
    [SkOptional] [SkAnimated] [SkDefault("0")] [SkRange(0.0, 120.0)]
    float speed;

    // Straight alpha is the natural way to write a grain layer: pick a colour,
    // pick a coverage. The engine premultiplies.
    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        float peak = saturate(amount);
        float cell_size = max(scale, 0.05);
        float rate = max(speed, 0.0);

        // Quantising time to whole frames is what makes grain read as film
        // rather than as a smooth shimmer.
        float frame = floor(input.time * rate);
        float2 cell = floor(input.px / cell_size);

        // Two decorrelated samples: a monochrome grain plus a weaker chroma
        // wobble, which is what real film grain looks like.
        float g = sk_hash12(cell + float2(frame * 17.13, frame * 7.31));
        float r = sk_hash12(cell + float2(frame * 17.13 + 91.7, frame * 7.31 + 13.1));
        float b = sk_hash12(cell + float2(frame * 17.13 + 41.3, frame * 7.31 + 57.9));

        // Centre on zero so the grain darkens as much as it lightens, then
        // fold the signed value into a single source-over layer.
        float3 signed_grain = float3(r, g, b) - 0.5;
        float3 chroma = lerp(float3(g, g, g) - 0.5, signed_grain, 0.35);

        float a = peak * saturate(abs(chroma.g) * 2.0 + 0.15);
        float3 rgb = saturate(0.5 + chroma * 2.0);

        return float4(rgb, a);
    }
};
