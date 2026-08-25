//! vn-effect: frost
//! vn-struct: Frost
//! vn-backdrop: true
//
// `--shading: frost(<strength>, <radius>?, <tint>?)`
//
//   params0.x  strength  0..1   how completely the frost replaces the backdrop
//   params0.y  radius    device px, the backdrop blur radius (also applied by
//                        the renderer, which builds the Dual-Kawase pyramid)
//   params1    tint      straight-alpha RGBA laid over the frosted backdrop
//
// The backdrop arrives already blurred to `radius`. On top of it this adds a
// faint, static frozen-glass grain and a rim highlight, which is what makes
// frost read as *frost* rather than as plain `backdrop-filter: blur()`.

import vn;

struct Frost : IEffect
{
    float4 shade(EffectInput input)
    {
        float strength = saturate(input.params0.x);
        float4 tint = input.params1;

        float4 backdrop = input.backdrop;

        // Frozen-glass grain: two lattices at different scales, one of them
        // stretched, so it looks crystalline rather than like film grain.
        float2 grain_uv = input.px / max(input.scale, 0.001);
        float crystal = vn_fbm(grain_uv * 0.18);
        crystal += 0.5 * vn_fbm(float2(grain_uv.x * 0.9, grain_uv.y * 0.31) * 0.55);
        crystal = crystal / 1.5 - 0.5;

        // A rim highlight hugging the rounded edge, brightest right at it.
        float rim = saturate(1.0 + input.sd / max(2.5 * input.scale, 1.0));
        rim = rim * rim * 0.16;

        float3 frosted = backdrop.rgb;
        // Grain and rim act on the premultiplied colour, so they scale with
        // whatever alpha the backdrop already carries.
        frosted += backdrop.a * (crystal * 0.055 + rim) * strength;

        float4 result = float4(frosted, backdrop.a);
        if (tint.a > 0.0)
        {
            result = vn_over(result, vn_premultiply(float4(tint.rgb, tint.a * strength)));
        }

        // `strength` also fades the whole treatment back toward the untouched
        // backdrop, so `frost(0)` is a no-op even with a large radius.
        return lerp(backdrop, result, strength);
    }
};
