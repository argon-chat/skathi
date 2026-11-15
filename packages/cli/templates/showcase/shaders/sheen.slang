import sk;

// This app's own `--shading` effect. Every shaders/<name>.slang is compiled into the app by sk build
// and sk dev; the comments in the `sk` module are the authoring rules.
[SkEffect("sheen")]
[SkDoc("a band of light sweeping across the element")]
struct Sheen : IEffect
{
    [SkAnimated] [SkDefault("0.5")] [SkRange(0.0, 4.0)]
    float speed;

    [SkColor] [SkOptional] [SkDefault("#ffffff")]
    float4 tint;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        float sweep = frac(input.time * speed) * 1.6 - 0.3;
        float band = saturate(1.0 - abs((input.local.x + input.local.y) * 0.5 - sweep) * 8.0);
        return float4(tint.rgb, tint.a * band * band * 0.6);
    }
};
