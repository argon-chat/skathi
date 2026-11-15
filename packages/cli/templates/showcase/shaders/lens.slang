import sk;

// Liquid glass. It reads the backdrop at displaced coordinates rather than under the pixel: the
// middle magnifies a little, and the curved rim gathers in what lies just outside the element, each
// colour channel from a slightly different place.
[SkEffect("lens")]
[SkDoc("liquid glass: refraction, dispersion and a specular rim over whatever is behind")]
[SkBackdrop]
struct Lens : IEffect
{
    // How far past the edge the rim reaches for what it shows.
    [SkLength] [SkDefault("18")] [SkRange(0.0, 96.0)]
    float depth;

    // The width of the curved rim; inside it the glass is flat.
    [SkLength] [SkDefault("24")] [SkRange(1.0, 256.0)]
    float bevel;

    // How far apart red and blue refract, as a fraction of the bend.
    [SkDefault("0.12")] [SkRange(0.0, 1.0)]
    float dispersion;

    [SkColor] [SkOptional] [SkDefault("#ffffff0f")]
    float4 tint;

    float4 shade(EffectInput input)
    {
        // `sd` is a distance field, so its screen-space gradient is the outward normal of the rim.
        float2 gradient = float2(ddx(input.sd), ddy(input.sd));
        float2 normal = gradient / max(length(gradient), 1e-4);

        float edge = 1.0 - saturate(-input.sd / max(bevel, 1.0));
        float curve = edge * edge;

        float2 magnify = (input.size * 0.5 - input.px) * 0.07;
        float2 shift = normal * depth * curve + magnify;
        float r = input.backdropAtPx(input.px + shift * (1.0 + dispersion)).r;
        float4 g = input.backdropAtPx(input.px + shift);
        float b = input.backdropAtPx(input.px + shift * (1.0 - dispersion)).b;
        float4 glass = sk_over(float4(r, g.g, b, g.a), sk_premultiply(tint));

        // A key light from the top left catches that rim, a weaker internal reflection the opposite one.
        float facing = dot(normal, float2(-0.6, -0.8));
        float rim = pow(edge, 5.0);
        float specular = rim * (0.5 * saturate(facing) + 0.16 * saturate(-facing));
        float sheen = 0.05 * saturate(1.0 - input.local.y * 1.6) * (1.0 - edge);

        float light = specular + sheen;
        return float4(glass.rgb + light, saturate(glass.a + light * (1.0 - glass.a)));
    }
};
