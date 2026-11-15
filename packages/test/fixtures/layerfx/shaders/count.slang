import sk;

// Red that builds up a step at a time in the history, so it only shows if the state carries over.
[SkEffect("count")]
[SkHistory]
struct Count : IEffect
{
    [SkAnimated] [SkDefault("1")] [SkRange(0.0, 1.0)]
    float speed;

    override float4 update(EffectInput input)
    {
        return input.history + float4(0.02, 0.0, 0.0, 0.0);
    }

    float4 shade(EffectInput input)
    {
        return float4(saturate(input.history.r), 0.0, 0.0, 1.0);
    }
};
