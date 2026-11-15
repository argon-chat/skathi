import sk;

// Two comets chasing each other around the element's rounded border. `halo` is the `[SkBleed]`
// parameter: the drawn quad grows by it, so the light spills past the border box instead of being cut
// off at it.
[SkEffect("orbit")]
[SkDoc("light running around the border, glowing past it")]
struct Orbit : IEffect
{
    // Laps per second.
    [SkAnimated] [SkDefault("0.35")] [SkRange(0.0, 4.0)]
    float speed;

    [SkLength] [SkDefault("2")] [SkRange(0.5, 16.0)]
    float width;

    [SkLength] [SkBleed] [SkDefault("18")] [SkRange(0.0, 96.0)]
    float halo;

    // How much of the lap the tail covers, 0..1.
    [SkDefault("0.35")] [SkRange(0.02, 1.0)]
    float tail;

    [SkColor] [SkDefault("#7fd4ff")]
    float4 color;

    static override bool straightAlpha() { return true; }

    // A long tail behind the head and a short soft front, so no edge of the light is a hard cut.
    float comet(float around, float head)
    {
        float ahead = frac(around - head + 0.5) - 0.5;
        float front = ahead / 0.02;
        return ahead > 0.0 ? exp(-front * front) : pow(saturate(1.0 + ahead / tail), 3.0);
    }

    float4 shade(EffectInput input)
    {
        float2 c = input.local * 2.0 - 1.0;
        float around = atan2(c.y, c.x) / 6.28318530718 + 0.5;
        float head = frac(input.time * speed);
        float light = comet(around, head) + 0.65 * comet(around, head + 0.5);

        float stroke = input.aa(abs(input.sd + width * 0.5) - width * 0.5);
        float outside = max(input.sd, 0.0) / max(halo, 1.0);
        float glow = exp(-outside * 4.0) * (input.sd > 0.0 ? 1.0 : saturate(1.0 + input.sd / (width * 4.0)));

        float alpha = stroke * (0.14 + 0.86 * light) + glow * light * 0.55;
        float3 rgb = lerp(color.rgb, float3(1.0, 1.0, 1.0), stroke * light * light * 0.6);
        return float4(rgb, saturate(alpha * color.a));
    }
};
