import sk;

// A colour grade through a 3D lookup table: 16 slices of 16x16 side by side, red across a slice,
// green down it, blue from slice to slice. Until the image arrives it shows the backdrop as it is.
[SkEffect("grade")]
[SkDoc("re-colours the backdrop through a 16x16x16 LUT image")]
[SkBackdrop]
struct Grade : IEffect
{
    [SkLinear]
    SkTexture lut;

    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float amount;

    float3 lookup(float3 c)
    {
        float blue = c.b * 15.0;
        float slice = floor(blue);
        float2 cell = float2(c.r * 15.0 + 0.5, c.g * 15.0 + 0.5);
        float3 lo = lut.sample((cell + float2(slice * 16.0, 0.0)) / lut.size).rgb;
        float3 hi = lut.sample((cell + float2(min(slice + 1.0, 15.0) * 16.0, 0.0)) / lut.size).rgb;
        return lerp(lo, hi, blue - slice);
    }

    float4 shade(EffectInput input)
    {
        float4 seen = input.backdrop;
        if (!lut.loaded())
            return seen;
        float3 rgb = saturate(seen.rgb / max(seen.a, 1e-4));
        return float4(lerp(rgb, lookup(rgb), amount) * seen.a, seen.a);
    }
};
