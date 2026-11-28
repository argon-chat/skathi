// `--shading: measured(<blur>, <detail>, <lens>, <dispersion>, <frost>, <dark>, <rim>, <shadow>)`
//
// Liquid Glass after the measurements published in github.com/shayann07/liquidglass (Apache-2.0,
// docs/research/measured-model.md), which read the material off an iOS 27 device at the default
// Tint Amount. The numbers below are theirs; lengths are in points, which are CSS pixels here.
//
//   * The body is not a clear lens. It is the backdrop blurred to a sigma of 15.5 pt, with about
//     27 % of the sharp picture laid back over it, more of it on the glass's own side of the
//     surroundings (their section 2e).
//   * A tone curve per appearance, applied to luma, with the chroma kept and boosted (2f).
//   * A lens only in the rim: over a band 0.6 R wide every ring shows the picture from about
//     0.33 R inside (2b, 6).
//   * A bead of light a third of a point inside the rim along the top and bottom, a dark contour
//     on the outline itself, and a colour fringe well under a pixel (2a, 2b).

import sk;

[SkEffect("measured")]
[SkDoc("Liquid Glass: blurred body with sharp detail, measured tone, a lens in the rim and a lit edge")]
struct Measured : IEffect
{
    /// Sigma of the wide kernel.
    [SkLength] [SkBackdropRadius] [SkDefault("15.5")] [SkRange(0.0, 64.0)]
    float blur;

    /// The share of the sharp picture kept over the blurred one.
    [SkDefault("0.275")] [SkRange(0.0, 1.0)]
    float detail;

    /// Scales the rim's displacement; 1 is the measured map.
    [SkDefault("1")] [SkRange(0.0, 2.0)]
    float lens;

    /// How much further blue is displaced than red, as a share of the displacement. The device
    /// measures under 0.01; Apple's own renders of the material show far more.
    [SkDefault("0.14")] [SkRange(0.0, 0.5)]
    float dispersion;

    /// How much of the measured tone the body takes: 0 is clear glass, 1 the regular material.
    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float frost;

    /// 0 is the light appearance, 1 the dark one.
    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float dark;

    /// Scales the bead of light on the edge.
    [SkDefault("1")] [SkRange(0.0, 3.0)]
    float rim;

    /// The reach of the shadow, and the dilation of the drawn quad.
    [SkLength] [SkBleed] [SkDefault("18")] [SkRange(0.0, 128.0)]
    float shadow;

    float edge(float2 p, float2 half, float radius)
    {
        float2 q = abs(p) - (half - radius);
        return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    }

    /// The bead's profile across the rim, `depth` in points inside the outline: +60 levels at its
    /// peak, 40, 22 and 10 over the next point, then a tail of 6 and 3.
    float bead(float depth, float peak)
    {
        float core = (depth - peak) / 0.75;
        return 0.235 * exp(-core * core) + 0.05 * exp(-max(depth - 0.9, 0.0) / 1.33);
    }

    float4 shade(EffectInput input)
    {
        // The corner radii in the instance buffer are the drawn quad's, dilated by the bleed.
        SkCorners corners = sk_effect_quads[input.quad].corner_radii;
        float2 half = input.size * 0.5;
        float shortHalf = min(half.x, half.y);
        float2 p = input.px - half;
        float top = p.x < 0.0 ? corners.top_left : corners.top_right;
        float bottom = p.x < 0.0 ? corners.bottom_left : corners.bottom_right;
        float radius = clamp((p.y < 0.0 ? top : bottom) - input.bleed, 0.0, shortHalf);

        float2 q = abs(p) - (half - radius);
        float2 beyond = max(q, 0.0);
        float reach = length(beyond);
        float dist = reach + min(max(q.x, q.y), 0.0) - radius;
        float2 side = float2(p.x < 0.0 ? -1.0 : 1.0, p.y < 0.0 ? -1.0 : 1.0);
        float2 axis = q.x > q.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
        float2 outward = (reach > 1e-4 ? beyond / max(reach, 1e-4) : axis) * side;
        float inside = max(-dist, 0.0);

        // The rest lens. With x the depth as a fraction of the band, the ring shown is a plateau at
        // 0.548 that eases into the identity by 0.76, and the outermost few pixels reach deeper
        // again; the smooth maximum and the exponential reproduce the measured table within 0.04.
        float band = 0.6 * radius;
        float x = inside / max(band, 1.0);
        float blend = max(0.25 - abs(x - 0.548), 0.0) / 0.25;
        float shown = max(x, 0.548) + blend * blend * 0.0625 + 0.15 * exp(-x / 0.07);
        // The device keeps a straight run at rest flat; the lens is drawn all the way round here, as
        // in Apple's renders. Each channel is displaced by its own amount, blue the furthest.
        float2 shift = -outward * (shown - x) * band * lens / sk_viewport_size;
        float2 atRed = input.uv + shift * (1.0 - dispersion);
        float2 atGreen = input.uv + shift;
        float2 atBlue = input.uv + shift * (1.0 + dispersion);

        float3 wide = float3(
            input.backdropAt(atRed).r, input.backdropAt(atGreen).g, input.backdropAt(atBlue).b);
        float3 fine = float3(
            input.rawBackdropAt(atRed).r, input.rawBackdropAt(atGreen).g, input.rawBackdropAt(atBlue).b);

        // The scatter tapers to nothing at the rim: what the lens shows there is sharp and keeps
        // its own colour, and the body's blur and tone come in over the band.
        float body = saturate(x / 0.8);
        body = 1.0 - (1.0 - body) * (1.0 - body);
        float share = lerp(1.0, detail, body);

        // lin = B + k (D - B) + c |D - B|, c positive on light glass and negative on dark.
        float3 delta = fine - wide;
        float3 lin = wide + share * delta + lerp(0.22, -0.22, dark) * (detail / 0.275) * body * abs(delta);

        // Tone on luma, in display code values as measured.
        float y = sk_luma(lin);
        float toneDark = 0.1244 + 0.9976 * y - 0.3961 * y * y;
        float toneLight = min(0.5196 + 0.502 * y, 1.0);
        float tone = lerp(toneLight, toneDark, dark);
        float3 gain = lerp(float3(0.988, 0.952, 0.979), float3(1.317, 1.256, 1.314), dark)
            + lerp(float3(-0.076, 0.028, -0.045), float3(-0.427, -0.351, -0.429), dark) * y;
        float3 color = lerp(lin, saturate(tone + gain * (lin - y)), frost * lerp(0.35, 1.0, body));

        // The bead lies along the top and bottom runs and dies out round the ends: 0.68, 0.27 and
        // 0.13 of its peak at 15, 30 and 45 degrees, nothing from 60. The bottom one sits a third
        // of a point deeper. Red's copy is 0.15 pt inside the line and blue's 0.15 pt outside.
        float depth = inside / max(input.scale, 1e-3);
        float upright = abs(outward.y);
        float lobe = 0.85 * pow(upright, 12.0) + 0.15 * smoothstep(0.5, 1.0, upright);
        float peak = outward.y > 0.0 ? 0.67 : 0.33;
        float3 light = float3(bead(depth - 0.15, peak), bead(depth, peak), bead(depth + 0.15, peak));
        color = saturate(color + rim * lobe * light);

        // The outline itself is a dark contour a third of a point wide.
        float contour = depth / 0.3;
        color *= 1.0 - 0.6 * exp(-contour * contour);

        float4 behind = input.rawBackdropAt(input.uv);
        float dropped = edge(p - float2(0.0, shadow * 0.22), half, radius);
        float soft = 1.0 - smoothstep(-shadow * 0.4, shadow * 0.9, dropped);
        float shade = shadow > 0.0 ? 0.14 * soft * soft : 0.0;
        float4 ground = float4(behind.rgb * (1.0 - shade), behind.a + (1.0 - behind.a) * shade);

        float4 result = lerp(ground, float4(color, 1.0), saturate(0.5 - dist));
        // Half a code value of noise keeps the wide kernel and the shadow from banding.
        result.rgb += (sk_hash12(input.px + 0.5) - 0.5) / 255.0;
        return result;
    }
};
