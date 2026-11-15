import sk;

// Burns the element away: its own painting is eaten by noise from the ragged edge inwards, with a
// glowing ember rim. `progress` is a plain number, so a CSS transition on `--shading` plays the burn.
[SkEffect("dissolve")]
[SkDoc("burns the element's own painting away behind an ember edge")]
[SkContent]
struct Dissolve : IEffect
{
    // 0 is the element untouched, 1 is all of it gone.
    [SkDefault("0")] [SkRange(0.0, 1.0)]
    float progress;

    // The size of the burnt patches.
    [SkLength] [SkDefault("46")] [SkRange(4.0, 1024.0)]
    float grain;

    [SkColor] [SkOptional] [SkDefault("#ff7a1a")]
    float4 ember;

    float4 shade(EffectInput input)
    {
        // Mostly noise, a little of the distance from the top left, so the burn sweeps across.
        float n = sk_fbm(input.px / max(grain, 1.0)) * 0.75 + dot(input.local, float2(0.15, 0.1));
        float edge = progress * 1.12 - 0.06;
        float burnt = n - edge;
        float width = 0.05;

        float4 content = input.contentAtPx(input.px + float2(0.0, -6.0 * input.scale * saturate(1.0 - burnt / width)));
        float alive = smoothstep(0.0, 0.012, burnt);
        float rim = alive * (1.0 - smoothstep(0.0, width, burnt));

        // The rim chars the content to black, then glows through it.
        float3 rgb = lerp(content.rgb, float3(0.0, 0.0, 0.0), rim * 0.8 * content.a);
        float flicker = 0.6 + 0.4 * sk_value_noise(input.px / (6.0 * input.scale) + input.time * 3.0);
        float glow = rim * rim * flicker * content.a;
        float4 shaded = float4(rgb, content.a) * alive;
        return sk_over(shaded, float4(ember.rgb, 1.0) * glow * ember.a);
    }
};
