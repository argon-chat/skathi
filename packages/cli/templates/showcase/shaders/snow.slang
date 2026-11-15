import sk;

// Four layers of snow, the nearer ones bigger, softer and faster. The flakes near `at` (the pointer,
// from JS) are pushed gently aside; the rest of the snow does not notice it.
[SkEffect("snow")]
[SkDoc("parallax snowfall that parts around a point")]
struct Snow : IEffect
{
    [SkAnimated] [SkDefault("1")] [SkRange(0.0, 4.0)]
    float speed;

    // The pointer, 0..1 across the element.
    [SkDefault("0.5 0.5")]
    float2 at;

    // How wide a berth the flakes give the pointer; 0 turns it off.
    [SkLength] [SkDefault("90")] [SkRange(0.0, 1024.0)]
    float berth;

    [SkDefault("0.6")] [SkRange(0.0, 1.0)]
    float amount;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        float t = input.time * speed;
        float2 pointer = at * input.size;
        float2 offset = input.px - pointer;
        float reach = max(berth, 1.0);
        float part = berth > 0.0 ? 0.3 * exp(-dot(offset, offset) / (reach * reach)) : 0.0;
        float2 p = pointer + offset * (1.0 - part);

        float alpha = 0.0;
        for (int layer = 0; layer < 4; layer++)
        {
            float depth = float(layer);
            float cell = (38.0 + 30.0 * depth) * input.scale;
            float fall = (26.0 + 30.0 * depth) * input.scale;
            float2 q = p - float2(0.0, t * fall);
            float2 id = floor(q / cell) + depth * 17.0;
            float2 jitter = sk_hash22(id);
            float2 spot = (floor(q / cell) + 0.25 + 0.5 * jitter) * cell;
            spot.x += sin(t * (0.8 + jitter.y) + jitter.x * 6.28) * cell * 0.18;

            float radius = (0.8 + 0.9 * depth + jitter.y * 0.8) * input.scale;
            float soft = (0.6 + 0.9 * depth) * input.scale;
            float flake = 1.0 - smoothstep(radius - soft, radius + soft, length(q - spot));
            float present = step(sk_hash12(id + 5.0), amount);
            alpha = max(alpha, flake * present * (0.45 + 0.15 * depth));
        }
        return float4(0.93, 0.96, 1.0, alpha);
    }
};
