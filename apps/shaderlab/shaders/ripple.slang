import vn;

[VnEffect("ripple")]
[VnDoc("animated concentric rings radiating from a point")]
struct Ripple : IEffect
{
    // `[VnAnimated]` is what tells the host this effect depends on `input.time`, so the window
    // keeps repainting while it is above zero — and stops when it is not.
    [VnAnimated] [VnDefault("1.6")] [VnRange(0.0, 12.0)]
    float speed;

    // `[VnLength]` means the CSS argument is written `26px` and arrives already multiplied by the
    // device scale factor, so the rings are the same physical size on every monitor.
    [VnLength] [VnDefault("26")] [VnRange(2.0, 512.0)]
    float wavelength;

    // A `float2` expands to two CSS arguments, `center.x` and `center.y`, because a CSS function
    // argument is a single value.
    [VnDefault("0.5 0.5")]
    float2 center;

    [VnColor] [VnOptional] [VnDefault("#7fd4ff")]
    float4 tint;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        // Distance from the centre in DEVICE pixels, so the ring spacing is the `wavelength` the
        // author asked for regardless of the element's aspect.
        float2 offset = (input.local - center) * input.size;
        float radius = length(offset);

        float phase = radius / max(wavelength, 1.0) - input.time * speed;
        float crest = 0.5 + 0.5 * cos(phase * 6.28318530718);
        crest = crest * crest * crest;

        // Fade out over roughly the element's half-diagonal so the disturbance dies away instead
        // of tiling to the corners.
        float reach = 0.75 * length(input.size);
        float attenuation = saturate(1.0 - radius / max(reach, 1.0));

        float echo = 0.5 + 0.5 * cos(phase * 3.14159265359 - 0.7);

        float alpha = tint.a * attenuation * crest * (0.55 + 0.45 * echo);
        // Crests are slightly brighter than the tint, troughs slightly cooler: a flat colour would
        // look like a decal.
        float3 rgb = lerp(tint.rgb, float3(1.0, 1.0, 1.0), crest * 0.35);
        return float4(rgb, saturate(alpha));
    }
};
