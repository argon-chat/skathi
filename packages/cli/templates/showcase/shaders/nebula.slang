import sk;

// Domain warping: noise sampled at coordinates bent by more noise, twice over, so the field folds into
// gas-like filaments. It paints the whole element; `hue` is a plain number, so a CSS transition on
// `--shading` cross-fades the palette while the gas keeps moving.
[SkEffect("nebula")]
[SkDoc("drifting domain-warped gas with twinkling stars")]
struct Nebula : IEffect
{
    [SkAnimated] [SkDefault("0.12")] [SkRange(0.0, 2.0)]
    float speed;

    // Features per shorter side of the element.
    [SkDefault("2.2")] [SkRange(0.2, 12.0)]
    float scale;

    // Shifts the palette around the colour wheel, 0..1.
    [SkDefault("0")] [SkRange(-4.0, 4.0)]
    float hue;

    // The brightest filaments and the stars.
    [SkColor] [SkOptional] [SkDefault("#ffe6c7")]
    float4 glow;

    float4 shade(EffectInput input)
    {
        float t = input.time * speed;
        float2 p = input.centered() * scale;

        float2 q = float2(sk_fbm(p + float2(0.0, t)), sk_fbm(p + float2(5.2, 1.3) - t * 0.7));
        float2 r = float2(
            sk_fbm(p + 2.8 * q + float2(1.7, 9.2) + t * 1.3),
            sk_fbm(p + 2.8 * q + float2(8.3, 2.8) - t * 0.9));
        float f = sk_fbm(p + 2.8 * r);

        float3 rgb = sk_palette(
            f + length(q) * 0.35 + hue,
            float3(0.42, 0.30, 0.52),
            float3(0.42, 0.30, 0.40),
            float3(1.0, 1.0, 1.0),
            float3(0.62, 0.78, 0.95));
        float density = smoothstep(0.25, 0.95, f * f * 1.6 + r.x * 0.45);
        rgb *= 0.18 + 0.95 * density;
        rgb = lerp(rgb, glow.rgb, pow(saturate(f * 1.25 - 0.35), 3.0) * glow.a);

        // A sparse star field on a lattice in logical pixels, so the stars keep their size on any display.
        float lattice = 7.0 * input.scale;
        float2 cell = floor(input.px / lattice);
        float2 spot = (cell + 0.2 + 0.6 * sk_hash22(cell + 11.0)) * lattice;
        float size = (0.5 + 0.9 * sk_hash12(cell + 5.0)) * input.scale;
        float star = step(0.975, sk_hash12(cell)) * input.aa(length(input.px - spot) - size);
        float twinkle = 0.5 + 0.5 * sin(input.time * (1.5 + 3.0 * sk_hash12(cell + 7.0)) + 40.0 * sk_hash12(cell + 3.0));
        rgb += glow.rgb * star * twinkle * (1.0 - density * 0.5);

        return float4(rgb, 1.0);
    }
};
