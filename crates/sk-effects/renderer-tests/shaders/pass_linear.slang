// Passes its backdrop through in linear light; the D3D12 renderer tests check
// that this is an identity, with and without a blur radius.

import sk;

[SkEffect("pass-linear")]
[SkBackdropLinear]
struct PassLinear : IEffect
{
    [SkLength] [SkBackdropRadius] [SkDefault("0")]
    float radius;

    float4 shade(EffectInput input)
    {
        return input.backdrop;
    }
};
