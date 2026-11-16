import sk;

// A slow swirl of candy-coloured stars in three depths around the middle of the window, with pastel
// nebula glow and a shooting star now and then. Same arguments as `aurora` and `bubbles`.
[SkEffect("stardust")]
[SkDoc("a swirling field of candy stars with the odd shooting star")]
struct Stardust : IEffect
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

    float3 candy(float h)
    {
        float3 pink = float3(0.969, 0.659, 0.816);
        float3 lilac = float3(0.788, 0.655, 0.961);
        float3 blue = float3(0.624, 0.804, 0.961);
        float3 yellow = float3(0.984, 0.878, 0.627);
        return h < 0.25 ? pink : h < 0.5 ? lilac : h < 0.75 ? blue : yellow;
    }

    float4 shade(EffectInput input)
    {
        float t = input.time * speed;
        float2 p = input.centered();
        float show = 0.4 + 0.6 * mood;
        float3 rgb = float3(0.090, 0.086, 0.118);

        float n = sk_fbm(p * 1.3 + float2(t * 0.8, -t * 0.5));
        float n2 = sk_fbm(p * 2.1 - float2(t * 0.6, t * 0.4) + 4.0);
        rgb += first.rgb * 0.16 * smoothstep(0.35, 0.9, n) * show;
        rgb += second.rgb * 0.13 * smoothstep(0.4, 0.95, n2) * show;
        rgb += float3(0.788, 0.655, 0.961) * 0.06 * exp(-dot(p, p) * 1.2) * show;

        float near = sk_pointer_presence(seen, input.time, 0.6);
        float2 toward = (at * 2.0 - 1.0) * input.size / max(min(input.size.x, input.size.y), 1.0);

        for (int layer = 0; layer < 3; layer++)
        {
            float depth = float(layer);
            float swirl = t * (0.25 + 0.2 * depth) * (0.4 + 0.6 * mood);
            float2 q = sk_rotate(p, swirl) + (toward - p) * 0.04 * near * (depth + 1.0) * 0.3;
            float lattice = (0.11 - 0.025 * depth);
            float2 cell = floor(q / lattice);
            float2 spot = (cell + 0.2 + 0.6 * sk_hash22(cell + depth * 13.0)) * lattice;
            float h = sk_hash12(cell + depth * 7.0);
            float r = length(q - spot) / lattice;
            float size = 0.035 + 0.045 * (2.0 - depth) * sk_hash12(cell + 19.0);
            float twinkle = 0.5 + 0.5 * sin(input.time * (1.2 + 2.0 * h) + h * 50.0);
            float core = smoothstep(size, size * 0.3, r);
            float halo = exp(-r * r * 90.0) * 0.3;
            float present = step(0.74 + 0.08 * depth, sk_hash12(cell + 29.0));
            rgb += candy(sk_hash12(cell + 41.0)) * (core + halo) * present * (0.45 + 0.55 * twinkle) * (0.9 - 0.2 * depth);
        }

        // A shooting star every few seconds, on a path picked from the period's number.
        float period = 5.5;
        float k = floor(input.time / period);
        float age = frac(input.time / period) * period;
        float2 h2 = sk_hash22(float2(k, 3.0));
        float2 from = float2(-1.4 + h2.x * 1.6, -1.0 + h2.y * 0.6);
        float2 dir = normalize(float2(1.0, 0.45 + 0.3 * h2.y));
        float2 head = from + dir * age * 1.6;
        float2 rel = p - head;
        float along = dot(rel, -dir);
        float across = dot(rel, float2(-dir.y, dir.x));
        float trail = (along > 0.0 ? exp(-along * 5.0) : exp(along * 80.0)) * exp(-across * across * 9000.0);
        float alive = smoothstep(0.0, 0.15, age) * smoothstep(1.4, 0.8, age) * step(0.4, sk_hash12(float2(k, 9.0)));
        rgb += float3(1.0, 0.94, 0.85) * trail * alive * 0.9;

        rgb += (sk_hash12(input.px + frac(input.time) * 97.0) - 0.5) / 255.0;
        return float4(rgb, 1.0);
    }
};
