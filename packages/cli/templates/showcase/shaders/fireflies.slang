import sk;

// Fireflies wandering over the element on paths made of incommensurate sines, so no two repeat,
// each blinking on its own rhythm. With `lure` above 0 the ones near `at` (the pointer) gather round it.
[SkEffect("fireflies")]
[SkDoc("glowing fireflies drifting about, drawn to a point")]
struct Fireflies : IEffect
{
    [SkAnimated] [SkDefault("1")] [SkRange(0.0, 4.0)]
    float speed;

    // The glow's radius.
    [SkLength] [SkDefault("16")] [SkRange(2.0, 128.0)]
    float glow;

    // The pointer, 0..1 across the element.
    [SkDefault("0.5 0.5")]
    float2 at;

    // 0 ignores the pointer; 1 gathers the nearby fireflies round it.
    [SkDefault("0")] [SkRange(0.0, 1.0)]
    float lure;

    float4 shade(EffectInput input)
    {
        float t = input.time * speed;
        float reach = max(glow, 1.0);
        float aspect = input.size.x / max(input.size.y, 1.0);
        float3 light = float3(0.0, 0.0, 0.0);
        for (int i = 0; i < 28; i++)
        {
            float2 h = sk_hash22(float2(i, 3.7));
            float2 k = sk_hash22(float2(i, 9.1));
            float2 wander = 0.5 + float2(
                0.30 * sin(t * (0.07 + 0.09 * h.x) + 6.28 * h.y) + 0.17 * sin(t * (0.19 + 0.13 * k.x) + 6.28 * k.y),
                0.28 * sin(t * (0.05 + 0.08 * k.y) + 6.28 * k.x) + 0.18 * sin(t * (0.23 + 0.11 * h.y) + 6.28 * h.x));

            // Drawn in by distance in the element's own proportions, then kept circling the pointer.
            float2 toward = (at - wander) * float2(aspect, 1.0);
            float pull = lure * exp(-dot(toward, toward) / 0.35);
            float2 circle = float2(cos(t * (0.6 + h.x) + 6.28 * k.x) / aspect, sin(t * (0.6 + h.x) + 6.28 * k.x));
            float2 spot = lerp(wander, at + circle * (0.04 + 0.06 * k.y), pull) * input.size;

            float2 d = input.px - spot;
            float r2 = dot(d, d) / (reach * reach);
            float blink = 0.15 + 0.85 * pow(saturate(sin(t * (0.7 + 1.1 * h.y) + 6.28 * k.x)), 3.0);
            float shine = (exp(-r2 * 24.0) + 0.35 * exp(-r2 * 2.0)) * blink;
            float3 tint = lerp(float3(0.75, 1.0, 0.35), float3(1.0, 0.78, 0.3), k.x);
            light += tint * shine;
        }
        float strength = max(light.r, max(light.g, light.b));
        return float4(light, saturate(strength));
    }
};
