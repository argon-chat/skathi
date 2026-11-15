import sk;

[SkEffect("stamp")]
struct Stamp : IEffect
{
    [SkDefault("#00ff00")]
    SkTexture image;

    float4 shade(EffectInput input)
    {
        return float4(image.sample(input.local).rgb, 1.0);
    }
};
