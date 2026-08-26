// `--shading: frost(<strength>, <radius>?, <tint>?)`
//
// The backdrop arrives already blurred to `radius` (the renderer builds the
// Dual-Kawase pyramid before the effect runs). On top of it this adds a faint,
// static frozen-glass grain and a rim highlight, which is what makes frost read
// as *frost* rather than as plain `backdrop-filter: blur()`.
//
// The only built-in that reads the backdrop, so the only one that costs a
// render-target snapshot.

import vn;

[VnEffect("frost")]
[VnDoc("blurred backdrop + crystalline grain + rim highlight")]
struct Frost : IEffect
{
    /// How completely the frost replaces the backdrop. `frost(0)` is a no-op
    /// even with a large radius.
    [VnDefault("0.5")] [VnRange(0.0, 1.0)]
    float strength;

    /// The backdrop blur radius. Applied by the renderer, not read here.
    /// The Dual-Kawase pyramid saturates at 5 levels; past that a larger
    /// radius changes nothing, so the range makes the ceiling visible in an
    /// advisory instead of silently.
    [VnLength] [VnOptional] [VnBackdropRadius] [VnDefault("12")] [VnRange(0.0, 25.0)]
    float radius;

    /// Straight-alpha RGBA laid over the frosted backdrop.
    /// `[VnDefault]` takes CSS-ish text, so a colour default can be written
    /// either as a hex literal (`"#ffffff10"`) or as four channels — which is
    /// the only way to spell an alpha that is not a multiple of 1/255.
    [VnColor] [VnOptional] [VnDefault("1 1 1 0.06")]
    float4 tint;

    float4 shade(EffectInput input)
    {
        float amount = saturate(strength);
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
        frosted += backdrop.a * (crystal * 0.055 + rim) * amount;

        float4 result = float4(frosted, backdrop.a);
        if (tint.a > 0.0)
        {
            result = vn_over(result, vn_premultiply(float4(tint.rgb, tint.a * amount)));
        }

        // `strength` also fades the whole treatment back toward the untouched
        // backdrop.
        return lerp(backdrop, result, amount);
    }
};
