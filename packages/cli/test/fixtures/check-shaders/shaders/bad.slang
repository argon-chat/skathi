import sk;

[SkEffect("bad")]
struct Bad : IEffect
{
    [SkDefault("3")]
    int rings;

    float4 shade(EffectInput input)
    {
        return float4(input.local, 0.0, 1.0);
    }
};
