import sk;

// Patterned glass. A tiling normal map bends the backdrop pixel by pixel and catches a little light,
// one texel of the map per `scale` pixels.
[SkEffect("glass")]
[SkDoc("textured glass: a repeating normal map refracts what is behind")]
[SkBackdrop]
struct Glass : IEffect
{
    [SkLinear] [SkDefault("#8080ffff")]
    SkTexture normals;

    // A normal's x and y lie in -1..1, so no read lands farther than `depth` outside the element.
    [SkLength] [SkBackdropReach] [SkDefault("10")] [SkRange(0.0, 64.0)]
    float depth;

    [SkDefault("1")] [SkRange(0.25, 8.0)]
    float scale;

    [SkColor] [SkOptional] [SkDefault("#ffffff10")]
    float4 tint;

    float4 shade(EffectInput input)
    {
        float2 tile = max(normals.size, float2(1.0, 1.0)) * scale;
        float3 n = normals.sampleRepeat(input.px / tile).xyz * 2.0 - 1.0;
        float4 seen = input.backdropAtPx(input.px + n.xy * depth);
        float light = 0.2 * pow(saturate(dot(normalize(n), normalize(float3(-0.4, -0.6, 0.7)))), 12.0);
        float4 glass = sk_over(seen, sk_premultiply(tint));
        return float4(glass.rgb + light, saturate(glass.a + light * (1.0 - glass.a)));
    }
};
