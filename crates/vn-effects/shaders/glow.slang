// `--shading: glow(<color>?, <radius>, <strength>)`
//
// One continuous lobe centred on the rounded-rect edge:
//   * OUTWARD it reaches `radius`. `radius` is the effect's `[VnBleed]`
//     parameter, so the renderer dilates the drawn quad by it and there is
//     somewhere for the outer half of the halo to land — an effect is
//     otherwise clipped to the element's border box.
//   * INWARD it reaches `radius * 0.6`, which is what stops the glow reading
//     as a detached drop shadow.
//
// Blending is ordinary source-over, so the glow lies over whatever is behind
// the element and under the element's own background.

import vn;

[VnEffect("glow")]
[VnDoc("soft halo centred on the element's rounded edge")]
struct Glow : IEffect
{
    /// Declared FIRST and optional, matching the author-facing signature.
    /// Type-directed matching is what makes that work: `glow(12px, 0.8)` skips
    /// it because `12px` is not a colour.
    ///
    /// `[VnSlot(4)]` keeps the packing this effect shipped with (radius 0,
    /// strength 1, colour 4..7) even though the field order now says otherwise.
    /// CSS transitions interpolate `--shading` slot by slot, so a slot layout
    /// is a published contract once anything animates it.
    [VnColor] [VnOptional] [VnSlot(4)] [VnDefault("#ffffff")]
    float4 color;

    /// How far outside the border box the halo reaches, and the dilation of
    /// the drawn quad.
    [VnLength] [VnBleed] [VnDefault("16")] [VnRange(0.0, 256.0)]
    float radius;

    /// Peak alpha at the edge.
    [VnDefault("0.75")] [VnRange(0.0, 1.0)]
    float strength;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        float reach = max(radius, 0.001);
        float peak = saturate(strength);

        // `sd` is signed distance to the ELEMENT rect: <0 inside, >0 outside.
        // Normalising each side by its own reach makes the lobe continuous at
        // the edge (both sides are 0 there) while still being asymmetric.
        float t = input.sd > 0.0
            ? (input.sd / reach)
            : (-input.sd / (reach * 0.6));

        // Quadratic falloff reads much closer to a real gaussian glow than a
        // linear ramp, and costs one multiply.
        float falloff = saturate(1.0 - t);
        float halo = falloff * falloff;

        return float4(color.rgb, saturate(halo * peak * color.a));
    }
};
