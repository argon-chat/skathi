//! vn-effect: glow
//! vn-struct: Glow
//! vn-backdrop: false
//
// `--shading: glow(<color>?, <radius>, <strength>)`
//
//   params0.x  radius    device px the halo reaches outside the border box
//   params0.y  strength  0..1 peak alpha at the edge
//   params1    color     straight-alpha RGBA (defaults to white)
//
// One continuous lobe centred on the rounded-rect edge:
//   * OUTWARD it reaches `radius`. The renderer dilates the quad by `radius`
//     (the primitive's `bleed` field) so there is somewhere to draw it.
//   * INWARD it reaches `radius * 0.6`, which is what stops the glow reading
//     as a detached drop shadow.
//
// Blending is ordinary source-over, so the glow lies over whatever is behind
// the element and under the element's own background.

import vn;

struct Glow : IEffect
{
    float4 shade(EffectInput input)
    {
        float radius = max(input.params0.x, 0.001);
        float strength = saturate(input.params0.y);
        float4 color = input.params1;

        // `sd` is signed distance to the ELEMENT rect: <0 inside, >0 outside.
        // Normalising each side by its own reach makes the lobe continuous at
        // the edge (both sides are 0 there) while still being asymmetric.
        float t = input.sd > 0.0
            ? (input.sd / radius)
            : (-input.sd / (radius * 0.6));

        // Quadratic falloff reads much closer to a real gaussian glow than a
        // linear ramp, and costs one multiply.
        float falloff = saturate(1.0 - t);
        float halo = falloff * falloff;

        float a = saturate(halo * strength * color.a);
        return vn_premultiply(float4(color.rgb, a));
    }
};
