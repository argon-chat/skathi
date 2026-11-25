import sk;
import sk.sample;

// One sk.sample kernel per mode over the engine's backdrop. Every one is symmetric and sums to 1, so a
// linear ramp comes back unchanged away from the edges, and a flat colour unchanged everywhere, even
// where the reads pass the declared reach and clamp.
[SkEffect("kernels")]
struct Kernels : IEffect
{
    [SkDefault("0")] float mode;

    [SkLength] [SkBackdropReach] [SkDefault("2")]
    float reach;

    float4 shade(EffectInput input)
    {
        float2 px = input.px;
        float4 c = RawBackdrop(input).at(px);
        if (mode < 0.5)
            c = gaussian<Gaussian13>(Backdrop(input), px);
        else if (mode < 1.5)
            c = box(RawBackdrop(input), px, 4.0);
        else if (mode < 2.5)
            c = catmullRom(RawBackdrop(input), px);
        else if (mode < 3.5)
            c = bspline(RawBackdrop(input), px);
        else if (mode < 4.5)
            c = sharpen(RawBackdrop(input), px, 1.5);
        else if (mode < 5.5)
            c = directional<9>(RawBackdrop(input), px, float2(12.0, 0.0));
        else
            c = disc<24>(RawBackdrop(input), px, 9.0);
        return c;
    }
};
