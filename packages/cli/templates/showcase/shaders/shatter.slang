import sk;

// Breaks whatever is behind the element into Voronoi shards that crack from `origin`, fall away and,
// as `progress` goes back to 0, fly back into place. Each pixel finds the shard now covering it by
// undoing every nearby shard's motion and testing which cell the result lands in; only then does it
// read the backdrop, once, at that point.
[SkEffect("shatter")]
[SkDoc("cracks the backdrop into falling shards; progress 0..1")]
[SkBackdrop]
struct Shatter : IEffect
{
    [SkDefault("0")] [SkRange(0.0, 1.0)]
    float progress;

    // Where the crack starts, 0..1 across the element.
    [SkDefault("0.5 0.5")]
    float2 origin;

    [SkLength] [SkDefault("110")] [SkRange(24.0, 512.0)]
    float shard;

    // What shows where the shards have gone.
    [SkColor] [SkOptional] [SkDefault("#050508")]
    float4 behind;

    float2 seed(float2 cell)
    {
        return (cell + 0.5 + (sk_hash22(cell) - 0.5) * 0.7) * shard;
    }

    // Distance from `q` to the border of `cell`'s Voronoi region: positive inside it.
    float inside(float2 q, float2 cell)
    {
        float2 own = seed(cell);
        float nearest = 1e9;
        for (int y = -1; y <= 1; y++)
        {
            for (int x = -1; x <= 1; x++)
            {
                if (x == 0 && y == 0)
                    continue;
                float2 other = seed(cell + float2(x, y));
                float2 across = other - own;
                nearest = min(nearest, -dot(q - (own + other) * 0.5, across) / length(across));
            }
        }
        return nearest;
    }

    float4 shade(EffectInput input)
    {
        float2 from = origin * input.size;
        float reach = length(input.size);
        float front = saturate(progress / 0.25) * reach * 1.1;
        float2 home = floor(input.px / shard);

        bool found = false;
        float2 source = input.px;
        float edge = 0.0;
        float fall = 0.0;
        float cracked = 0.0;
        float spin = 0.0;
        // A shard falls at most two cells and drifts at most one sideways, so these are the only ones
        // that can cover this pixel. Falling shards recede, so the least fallen one is in front.
        for (int dy = 1; dy >= -2; dy--)
        {
            for (int dx = -1; dx <= 1; dx++)
            {
                float2 cell = home + float2(dx, dy);
                float2 centre = seed(cell);
                float delay = length(centre - from) / reach;
                float u = saturate((progress - 0.22 - delay * 0.3) / 0.45);
                float e = u * u;
                float hit = step(length(centre - from), front);
                float h1 = sk_hash12(cell + 3.1);
                float h2 = sk_hash12(cell + 7.7);

                float2 away = normalize(centre - from + 1e-3);
                float2 shift = (sk_hash22(cell + 1.3) - 0.5) * 3.0 * input.scale * hit;
                shift += float2((h1 - 0.5) * 0.6 + away.x * 0.3, 1.8) * e * shard;
                float angle = (h2 - 0.5) * 3.0 * e;
                float size = 1.0 - 0.7 * e;

                float2 q = centre + sk_rotate(input.px - centre - shift, -angle) / size;
                float d = inside(q, cell);
                if (d >= 0.0 && u < 0.999 && (!found || u < fall))
                {
                    found = true;
                    source = q;
                    edge = d * size;
                    fall = u;
                    cracked = hit;
                    spin = angle;
                }
            }
        }

        float4 seen = sk_backdrop_input.SampleLevel(sk_clamp_sampler, input.uv + (source - input.px) / sk_viewport_size, 0.0);
        float4 gone = sk_premultiply(behind);
        if (!found)
            return gone;

        // A tilted shard catches or loses the light; its broken edge glints.
        float light = 1.0 - 0.3 * fall + 0.25 * fall * sin(spin * 3.0);
        float px = input.scale;
        float crack = cracked * (1.0 - saturate(edge / (0.9 * px)));
        float glint = cracked * (1.0 - saturate(abs(edge - 1.6 * px) / px)) * (0.35 + 0.4 * fall);

        float4 glass = float4(seen.rgb * light, seen.a);
        glass = lerp(glass, float4(0.0, 0.0, 0.0, 1.0), crack * 0.85);
        glass = float4(glass.rgb + glint * glass.a, glass.a);
        return lerp(glass, gone, smoothstep(0.6, 1.0, fall));
    }
};
