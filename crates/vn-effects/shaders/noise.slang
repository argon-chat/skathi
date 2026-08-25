//! vn-effect: noise
//! vn-struct: Noise
//! vn-backdrop: false
//
// `--shading: noise(<amount>, <scale>?, <speed>?)`
//
//   params0.x  amount  0..1  peak alpha of the grain
//   params0.y  scale   device px per grain cell (1 = one cell per pixel)
//   params0.z  speed   grain regenerations per second; 0 = a still image
//
// Film grain drawn OVER the element (the element's own background paints
// underneath it). Blending is ordinary source-over, so this composes with any
// background, gradient or image.

import vn;

struct Noise : IEffect
{
    float4 shade(EffectInput input)
    {
        float amount = saturate(input.params0.x);
        float scale = max(input.params0.y, 0.05);
        float speed = max(input.params0.z, 0.0);

        // Quantising time to whole frames is what makes grain read as film
        // rather than as a smooth shimmer.
        float frame = floor(input.time * speed);
        float2 cell = floor(input.px / scale);

        // Two decorrelated samples: a monochrome grain plus a weaker chroma
        // wobble, which is what real film grain looks like.
        float g = vn_hash12(cell + float2(frame * 17.13, frame * 7.31));
        float r = vn_hash12(cell + float2(frame * 17.13 + 91.7, frame * 7.31 + 13.1));
        float b = vn_hash12(cell + float2(frame * 17.13 + 41.3, frame * 7.31 + 57.9));

        // Centre on zero so the grain darkens as much as it lightens, then
        // fold the signed value into a single source-over layer.
        float3 signed_grain = float3(r, g, b) - 0.5;
        float3 chroma = lerp(float3(g, g, g) - 0.5, signed_grain, 0.35);

        float a = amount * saturate(abs(chroma.g) * 2.0 + 0.15);
        float3 rgb = saturate(0.5 + chroma * 2.0);

        return vn_premultiply(float4(rgb, a));
    }
};
