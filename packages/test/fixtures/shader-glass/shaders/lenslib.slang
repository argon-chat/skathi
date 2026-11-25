import sk;
import sk.light;
import sk.sample;

// The showcase template's `lens`, written with sk.light and sk.sample.
[SkEffect("lenslib")]
[SkBackdrop]
struct LensLib : IEffect
{
    [SkLength] [SkDefault("18")] [SkRange(0.0, 96.0)]
    float depth;

    [SkLength] [SkDefault("24")] [SkRange(1.0, 256.0)]
    float bevel;

    [SkDefault("0.12")] [SkRange(0.0, 1.0)]
    float spread;

    [SkColor] [SkOptional] [SkDefault("#ffffff0f")]
    float4 tint;

    float4 shade(EffectInput input)
    {
        float2 gradient = screenGradient(input.sd);
        float2 normal = gradient / max(length(gradient), 1e-4);
        float edge = 1.0 - saturate(-input.sd / max(bevel, 1.0));

        float2 shift = normal * depth * edge * edge + (input.size * 0.5 - input.px) * 0.07;
        float4 glass = sk_over(dispersion(Backdrop(input), input.px, shift, spread), sk_premultiply(tint));

        float facing = dot(normal, float2(-0.6, -0.8));
        float specular = pow(edge, 5.0) * (0.5 * saturate(facing) + 0.16 * saturate(-facing));
        float sheen = 0.05 * saturate(1.0 - input.local.y * 1.6) * (1.0 - edge);
        return addLight(glass, specular + sheen);
    }
};
