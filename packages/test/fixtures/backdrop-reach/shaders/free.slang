import sk;

// `reach` without a declared reach: it may sample anywhere.
[SkEffect("free")]
[SkBackdrop]
struct Free : IEffect
{
    [SkLength]
    float offset;

    float4 shade(EffectInput input)
    {
        return input.backdropAtPx(input.px + float2(offset, 0.0));
    }
};
