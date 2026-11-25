import sk;
import sk.light;
import sk.sample;

// The showcase template's `glass`, written with sk.light and sk.sample.
[SkEffect("glasslib")]
[SkBackdrop]
struct GlassLib : IEffect
{
    [SkLinear] [SkDefault("#8080ffff")]
    SkTexture normals;

    [SkLength] [SkBackdropReach] [SkDefault("10")] [SkRange(0.0, 64.0)]
    float depth;

    [SkDefault("1")] [SkRange(0.25, 8.0)]
    float scale;

    [SkColor] [SkOptional] [SkDefault("#ffffff10")]
    float4 tint;

    float4 shade(EffectInput input)
    {
        float3 n = unpackNormal(Image(normals, true).at(input.px / scale));
        float4 seen = displace(Backdrop(input), input.px, n, depth);
        float glint = 0.2 * lobe(n, normalize(float3(-0.4, -0.6, 0.7)), 12.0);
        return addLight(sk_over(seen, sk_premultiply(tint)), glint);
    }
};
