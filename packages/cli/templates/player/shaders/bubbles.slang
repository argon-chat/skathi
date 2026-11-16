import sk;

// Soap bubbles rising through a violet haze, three layers deep, each with a thin rainbow skin and a
// window highlight. Same arguments as `aurora` and `stardust`.
[SkEffect("bubbles")]
[SkDoc("iridescent soap bubbles drifting up through a pastel haze")]
struct Bubbles : IEffect
{
    [SkAnimated] [SkDefault("0.05")] [SkRange(0.0, 1.0)]
    float speed;

    [SkDefault("1")] [SkRange(0.0, 1.0)]
    float mood;

    [SkColor] [SkDefault("#f7a8d0")]
    float4 first;

    [SkColor] [SkDefault("#9fcdf5")]
    float4 second;

    [SkPointer]
    float2 at;

    [SkPointerTimes]
    float3 seen;

    float3 film(float x)
    {
        return sk_palette(x, float3(0.78, 0.74, 0.86), float3(0.22, 0.24, 0.16), float3(1.0, 1.0, 1.0), float3(0.0, 0.33, 0.67));
    }

    float4 shade(EffectInput input)
    {
        float t = input.time * speed * 20.0;
        float2 p = input.centered();
        float show = 0.4 + 0.6 * mood;
        float3 rgb = lerp(float3(0.090, 0.086, 0.118), float3(0.125, 0.100, 0.170), smoothstep(-1.0, 1.2, p.y));

        float2 h1 = p - float2(-0.7 + 0.3 * sin(t * 0.07), -0.4 + 0.2 * cos(t * 0.05));
        float2 h2 = p - float2(0.8 + 0.25 * cos(t * 0.06), 0.5 + 0.2 * sin(t * 0.08));
        rgb += first.rgb * 0.26 * exp(-dot(h1, h1) * 1.2) * show;
        rgb += second.rgb * 0.24 * exp(-dot(h2, h2) * 1.2) * show;

        float near = sk_pointer_presence(seen, input.time, 0.6);
        float2 pointer = at * input.size;

        for (int layer = 0; layer < 3; layer++)
        {
            float depth = float(layer);
            float cell = (90.0 + 60.0 * depth) * input.scale;
            float rise = (10.0 + 9.0 * depth) * input.scale * (0.5 + 0.5 * mood);
            float2 q = input.px + float2(0.0, t * rise);
            // Bubbles near the pointer are nudged aside.
            float2 off = input.px - pointer;
            q += off * 0.12 * near * exp(-dot(off, off) / (140.0 * 140.0 * input.scale * input.scale));
            float2 id = floor(q / cell) + depth * 31.0;
            float2 jitter = sk_hash22(id);
            if (sk_hash12(id + 3.0) > 0.32)
                continue;
            float2 centre = (floor(q / cell) + 0.5 + (jitter - 0.5) * 0.3) * cell;
            centre.x += sin(t * (0.08 + 0.05 * jitter.y) + jitter.x * 6.28) * cell * 0.05;
            float radius = (0.1 + 0.17 * jitter.y) * cell;
            float2 d = q - centre;
            float r = length(d) / radius;
            if (r > 1.25)
                continue;
            float skin = exp(-pow((r - 0.96) / 0.05, 2.0));
            float body = smoothstep(1.0, 0.0, r) * 0.05 + smoothstep(0.7, 1.0, r) * 0.12 * step(r, 1.0);
            float3 rainbow = film(atan2(d.y, d.x) * 0.16 + r * 0.6 + t * 0.01 + jitter.x);
            float2 hl = d / radius - float2(-0.38, -0.42);
            float highlight = exp(-dot(hl, hl) * 26.0);
            float fade = 0.45 + 0.25 * depth;
            rgb += (rainbow * (skin * 0.65 + body) + highlight * 0.55) * fade * show;
        }

        rgb += (sk_hash12(input.px + frac(input.time) * 97.0) - 0.5) / 255.0;
        return float4(rgb, 1.0);
    }
};
