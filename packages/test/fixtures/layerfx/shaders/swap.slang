import sk;

// The element's own painting with its channels rotated: green shows blue, red shows green.
[SkEffect("swap")]
[SkContent]
struct Swap : IEffect
{
    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float amount;

    float4 shade(EffectInput input)
    {
        float4 c = input.content;
        return lerp(c, float4(c.b, c.r, c.g, c.a), amount);
    }
};
