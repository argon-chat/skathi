import sk;

// A water surface over the element's own painting. The pointer drags through it; the waves it makes
// run outwards, bounce off the edges and die down, and they bend and light what is under them.
//
// The surface is a wave equation stepped in the history: red is the height now, green the height one
// step ago. The window keeps repainting for four seconds after the pointer last moved, which is long
// enough for the water to settle.
[SkEffect("pond")]
[SkDoc("a rippling water surface the pointer stirs, over the element's own painting")]
[SkContent]
[SkHistory]
struct Pond : IEffect
{
    // How long a wave lasts: the share of its height it keeps each step.
    [SkDefault("0.985")] [SkRange(0.8, 0.999)]
    float damping;

    // How far the slope of the water shifts what is under it.
    [SkLength] [SkDefault("40")] [SkRange(0.0, 400.0)]
    float depth;

    [SkColor] [SkOptional] [SkDefault("#bfe3ff")]
    float4 light;

    [SkPointer]
    float2 at;

    [SkPointerTimes] [SkAnimatedFor(4.0)]
    float3 seen;

    // The grid the waves travel on, in device pixels: coarser than a pixel so they move at a
    // readable speed with the history stepping at most 75 times a second.
    float spacing(EffectInput input)
    {
        return 2.0 * input.scale;
    }

    float height(EffectInput input, float2 px)
    {
        return input.historyAtPx(px).r;
    }

    override float4 update(EffectInput input)
    {
        float s = spacing(input);
        float around = height(input, input.px + float2(s, 0.0)) + height(input, input.px - float2(s, 0.0))
            + height(input, input.px + float2(0.0, s)) + height(input, input.px - float2(0.0, s));
        float next = (around * 0.5 - input.history.g) * damping;

        // The pointer pushes the water down while it moves inside the element.
        float2 pointer = at * input.size;
        float moving = 1.0 - saturate((input.time - seen.x) / 0.05);
        bool inside = all(at >= 0.0) && all(at <= 1.0) && seen.y > seen.z;
        float radius = 9.0 * input.scale;
        float2 d = input.px - pointer;
        next -= inside ? moving * 0.9 * exp(-dot(d, d) / (radius * radius)) : 0.0;

        return float4(clamp(next, -4.0, 4.0), input.history.r, 0.0, 0.0);
    }

    float4 shade(EffectInput input)
    {
        float s = spacing(input);
        float2 slope = float2(
            height(input, input.px + float2(s, 0.0)) - height(input, input.px - float2(s, 0.0)),
            height(input, input.px + float2(0.0, s)) - height(input, input.px - float2(0.0, s)));

        // Kept inside the box: past its edge the element painted nothing to show.
        float4 under = input.contentAtPx(clamp(input.px + slope * depth, 0.5, input.size - 0.5));
        // Light from the top left catches the faces of the waves that tilt towards it.
        float glint = pow(saturate(dot(slope, float2(-0.6, -0.8)) * 1.6), 2.0);
        float3 rgb = under.rgb * (1.0 + input.history.r * 0.08) + light.rgb * glint * light.a * under.a;
        return float4(rgb, under.a);
    }
};
