import sk;

// Shows the backdrop `offset` to the right, having declared it reads no farther than `reach`.
[SkEffect("reach")]
struct Reach : IEffect
{
    [SkLength] [SkBackdropReach]
    float reach;

    [SkLength]
    float offset;

    float4 shade(EffectInput input)
    {
        return input.backdropAtPx(input.px + float2(offset, 0.0));
    }
};
