import sk;
import sk.color;
import sk.light;
import sk.sample;

// A glass button: a rounded rim bends what is behind it by Snell's law, frosts it a little, and
// catches a highlight. Over a flat colour, reads clamped past the reach change nothing.
[SkEffect("button")]
struct Button : IEffect
{
    [SkLength] [SkDefault("16")] float bevel;
    [SkLength] [SkBackdropReach] [SkDefault("24")] float thickness;
    [SkColor] [SkDefault("#ffffff20")] float4 tint;

    float4 shade(EffectInput input)
    {
        float3 n = bevelNormal(input.sd, screenGradient(input.sd), bevel);
        float2 bend = refractionOffset(n, thickness, 1.5);
        Color seen = Color.fromPremultiplied(gaussian<Gaussian5>(Backdrop(input), input.px + bend));
        Color glass = seen.mix(Color.fromSrgb(tint), tint.a);
        float3 l = normalize(float3(-0.5, -0.7, 0.6));
        float highlight = blinnPhong(n, l, 80.0) + 0.3 * rim(n, 3.0) * fresnelSchlick(n.z, f0FromIor(1.5));
        return addLight(glass, float3(highlight, highlight, highlight)).toPremultiplied();
    }
};
