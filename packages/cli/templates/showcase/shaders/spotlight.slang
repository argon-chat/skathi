import sk;

// A dot grid that swells and lights up around a point, pushing the dots aside as it passes. The point
// comes from JS: the page writes `--shading: spotlight(x, y, ...)` on every pointer move, so the
// shader needs no input of its own.
[SkEffect("spotlight")]
[SkDoc("a dot grid lit and displaced around a point")]
struct Spotlight : IEffect
{
    // The lit point, 0..1 across the element.
    [SkDefault("0.5 0.5")]
    float2 at;

    [SkLength] [SkDefault("140")] [SkRange(8.0, 1024.0)]
    float radius;

    // 0 is a dim, even grid; 1 is the full light.
    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float strength;

    [SkColor] [SkOptional] [SkDefault("#8b9dff")]
    float4 color;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        float2 offset = input.px - at * input.size;
        float reach = max(radius, 1.0);
        float near = exp(-dot(offset, offset) / (reach * reach)) * strength;

        // Near the light each pixel looks up the grid closer to it, which magnifies the dots there.
        float zoom = 1.0 - 0.45 * near;
        float2 p = at * input.size + offset * zoom;

        float spacing = 16.0 * input.scale;
        float2 cell = (frac(p / spacing) - 0.5) * spacing / zoom;
        float size = (0.9 + 1.6 * near) * input.scale;
        float disc = input.aa(length(cell) - size);

        float3 rgb = lerp(float3(0.62, 0.64, 0.72), lerp(color.rgb, float3(1.0, 1.0, 1.0), near * near * 0.5), near);
        float alpha = disc * (0.16 + 0.84 * near) + near * near * 0.22;
        return float4(rgb, saturate(alpha * color.a));
    }
};
