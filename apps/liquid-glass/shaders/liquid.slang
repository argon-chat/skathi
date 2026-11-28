// `--shading: liquid(<blur>, <refraction>, <dispersion>, <edge>, <rim>, <fresnel>)`
//
// A port of the glass pass of github.com/arya3d/liquid-glass-webgpu (MIT, Arya3D,
// src/glassPassShader.ts), with its numbers as the defaults. It works in "glass space": the origin
// at the element's centre and the element's height as the unit, so the look scales with the
// element. The picture is scaled about the centre by the sine of a bend that goes from nothing at
// the edge to a quarter turn `refraction` inside it, red and blue bent 2 % less and more.
//
// `blur` is not in the original, which shows the page sharp; 0 leaves it out.

import sk;

[SkEffect("liquid")]
[SkDoc("liquid glass: the picture bent toward the centre near the edge, a colour split and a lit rim")]
struct Liquid : IEffect
{
    [SkLength] [SkBackdropRadius] [SkDefault("0")] [SkRange(0.0, 64.0)]
    float blur;

    /// How far inside the edge the lens reaches, in element heights.
    [SkDefault("0.3")] [SkRange(0.01, 1.0)]
    float refraction;

    [SkDefault("0.008")] [SkRange(0.0, 0.2)]
    float dispersion;

    [SkDefault("2")] [SkRange(0.0, 8.0)]
    float edgeHighlight;

    [SkDefault("0.15")] [SkRange(0.0, 2.0)]
    float rimStrength;

    [SkDefault("0.3")] [SkRange(0.0, 2.0)]
    float fresnel;

    float edge(float2 point, float2 half, float radius)
    {
        float2 fromCorner = abs(point) - half + radius;
        return length(max(fromCorner, 0.0)) + min(max(fromCorner.x, fromCorner.y), 0.0) - radius;
    }

    float coverage(float distance, float sharpness)
    {
        return saturate(-distance / 0.5 * sharpness);
    }

    float4 shade(EffectInput input)
    {
        float unit = max(input.size.y, 1.0);
        float2 half = input.size * 0.5 / unit;
        float2 position = (input.px - input.size * 0.5) / unit;

        SkCorners corners = sk_effect_quads[input.quad].corner_radii;
        float top = position.x < 0.0 ? corners.top_left : corners.top_right;
        float bottom = position.x < 0.0 ? corners.bottom_left : corners.bottom_right;
        float radius = min((position.y < 0.0 ? top : bottom) / unit, min(half.x, half.y));

        float distance = edge(position, half, radius);
        float covered = coverage(distance, 32.0);

        float2 probeX = float2(0.001, 0.0);
        float2 probeY = float2(0.0, 0.001);
        float3 normal = normalize(float3(
            edge(position + probeX, half, radius) - edge(position - probeX, half, radius),
            edge(position + probeY, half, radius) - edge(position - probeY, half, radius),
            1.0));
        float facing = max(normal.z, 0.0);
        float grazing = 1.0 - facing;
        float reflectance = 0.04 + 0.96 * pow(grazing, 5.0);

        // A flat-topped dome: the bend is a quarter turn everywhere deeper than `refraction`.
        float bend = pow(saturate(-distance / refraction), 0.25) * 1.57;
        float spread = dispersion * (1.0 - facing * 0.909);
        float2 bent = position * sin(bend);
        float2 offset = bent * spread;
        float2 red = (position - offset) * sin(bend * 0.98);
        float2 blue = (position + offset) * sin(bend * 1.02);

        float2 toUv = unit / sk_viewport_size;
        float3 refracted = float3(
            input.backdropAt(input.uv + (red - position) * toUv).r,
            input.backdropAt(input.uv + (bent - position) * toUv).g,
            input.backdropAt(input.uv + (blue - position) * toUv).b);

        // The ring is the difference between the box grown by a pixel and the box.
        float ring = coverage(distance - 1.0 / unit, 16.0) - coverage(distance, 16.0);
        float3 halfway = normalize(normalize(float3(0.3, -0.5, 1.0)) + float3(0.0, 0.0, 1.0));
        float shine = pow(max(dot(normal, halfway), 0.0), 32.0) * ring * 0.5;
        float rim = pow(grazing, 3.0) * covered * rimStrength;

        float3 lit = refracted + ring * (1.0 + reflectance * edgeHighlight) + shine + rim;
        float3 glass = lit * (1.0 + reflectance * fresnel);

        float4 behind = input.rawBackdropAt(input.uv);
        return lerp(behind, float4(glass, 1.0), smoothstep(0.0, 1.0, covered));
    }
};
