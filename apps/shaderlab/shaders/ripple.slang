// ripple — an APP-authored effect.
//
//     --shading: ripple(<speed>, <wavelength>, <center.x>, <center.y>, <tint>?)
//
// Concentric rings travelling outwards from a point, attenuated with distance
// so they read as a disturbance in the surface rather than as wallpaper.
//
// Nothing outside this file knows `ripple` exists. `bun shaders.ts --app … --out …`
// reflects the attributes below into `effects.json`, the host reads that at
// startup, and the CSS parser validates `ripple(…)` against it.

import vn;

[VnEffect("ripple")]
[VnDoc("animated concentric rings radiating from a point")]
struct Ripple : IEffect
{
    /// Rings per second. `[VnAnimated]` is what tells the host this effect
    /// depends on `input.time`, so the window keeps repainting while it is
    /// above zero — and stops when it is not.
    [VnAnimated] [VnDefault("1.6")] [VnRange(0.0, 12.0)]
    float speed;

    /// Distance between rings, in logical pixels. `[VnLength]` means the CSS
    /// argument is written `26px` and arrives already multiplied by the device
    /// scale factor, so the rings are the same physical size on every monitor.
    [VnLength] [VnDefault("26")] [VnRange(2.0, 512.0)]
    float wavelength;

    /// Where the rings come from, in 0..1 element coordinates. A `float2`
    /// expands to two CSS arguments, `center.x` and `center.y`, because a CSS
    /// function argument is a single value.
    [VnDefault("0.5 0.5")]
    float2 center;

    /// `[VnColor]` collapses the four channels back into ONE CSS argument, so
    /// this is written `#7fd4ff` rather than four numbers.
    [VnColor] [VnOptional] [VnDefault("#7fd4ff")]
    float4 tint;

    // Picking a colour and a coverage is how anyone actually thinks about a
    // layer like this; the engine premultiplies on the way out.
    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        // Distance from the centre in DEVICE pixels, so the ring spacing is the
        // `wavelength` the author asked for regardless of the element's aspect.
        float2 offset = (input.local - center) * input.size;
        float radius = length(offset);

        float phase = radius / max(wavelength, 1.0) - input.time * speed;
        // pow() on a 0..1 cosine gives thin bright crests and wide dark
        // troughs, which is what makes this read as ripples and not as a
        // sine grating.
        float crest = 0.5 + 0.5 * cos(phase * 6.28318530718);
        crest = crest * crest * crest;

        // Fade out over roughly the element's half-diagonal so the disturbance
        // dies away instead of tiling to the corners.
        float reach = 0.75 * length(input.size);
        float attenuation = saturate(1.0 - radius / max(reach, 1.0));

        // A second, slower ring train an eighth of a wavelength behind adds the
        // interference that keeps the motion from looking mechanical.
        float echo = 0.5 + 0.5 * cos(phase * 3.14159265359 - 0.7);

        float alpha = tint.a * attenuation * crest * (0.55 + 0.45 * echo);
        // Crests are slightly brighter than the tint, troughs slightly cooler:
        // a flat colour would look like a decal.
        float3 rgb = lerp(tint.rgb, float3(1.0, 1.0, 1.0), crest * 0.35);
        return float4(rgb, saturate(alpha));
    }
};
