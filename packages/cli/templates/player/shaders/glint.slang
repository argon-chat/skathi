import sk;

// The play button's candy shine: a glossy cap of light on the button, a halo past its edge and three little
// sparkles circling it. `lit` is 0 while paused and 1 while playing; a transition on `--shading` brings the
// halo and the sparkles up and down.
[SkEffect("glint")]
[SkDoc("glossy shine, halo and orbiting sparkles for a round button")]
struct Glint : IEffect
{
    [SkAnimated] [SkDefault("0.3")] [SkRange(0.0, 4.0)]
    float speed;

    [SkDefault("0")] [SkRange(0.0, 1.0)]
    float lit;

    [SkLength] [SkBleed] [SkDefault("24")] [SkRange(0.0, 96.0)]
    float halo;

    [SkColor] [SkDefault("#f7a8d0")]
    float4 color;

    static override bool straightAlpha() { return true; }

    float star(float2 d, float size)
    {
        float2 a = abs(d) / size;
        return (exp(-a.x * 6.0) * exp(-a.y * 1.1) + exp(-a.y * 6.0) * exp(-a.x * 1.1)) * exp(-length(a));
    }

    float4 shade(EffectInput input)
    {
        float px = input.scale;
        float2 centre = input.size * 0.5;
        float2 v = input.px - centre;
        float radius = min(input.size.x, input.size.y) * 0.5;
        float t = input.time * speed;

        float inside = input.coverage;
        float2 cap = (input.local - float2(0.5, 0.3)) * float2(1.0, 2.1);
        float gloss = smoothstep(0.42, 0.18, length(cap)) * 0.4 * inside;
        float sweep = frac(t * 0.6) * 2.4 - 0.7;
        float sheen = saturate(1.0 - abs(input.local.x + input.local.y * 0.5 - sweep) * 7.0);
        gloss += sheen * sheen * 0.22 * inside;

        float outside = max(input.sd, 0.0) / max(halo, 1.0);
        float pulse = 0.85 + 0.15 * sin(input.time * 3.2);
        float glow = exp(-outside * 3.2) * step(0.0, input.sd) * (0.18 + 0.5 * lit * pulse);

        float sparkles = 0.0;
        for (int i = 0; i < 3; i++)
        {
            float a = t * 6.28318530718 + float(i) * 2.09439510239;
            float2 at = float2(cos(a), sin(a)) * (radius + halo * 0.45);
            float twinkle = 0.6 + 0.4 * sin(input.time * 5.0 + float(i) * 2.0);
            sparkles += star(v - at, (5.0 + 1.5 * float(i)) * px) * twinkle;
        }
        sparkles *= lit;

        float3 rgb = lerp(color.rgb, float3(1.0, 1.0, 1.0), saturate(gloss * 2.0 + sparkles * 0.7));
        float alpha = saturate(gloss + glow * color.a + sparkles);
        return float4(rgb, alpha);
    }
};
