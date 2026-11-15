import sk;

// Rings of water spreading from up to four points, refracting whatever is behind the element. The
// rings add up as heights, so where two meet they interfere: crests stack, crest and trough cancel.
//
// The parameter block holds eight floats and a ring needs a position and an age, so each ring is a
// `float2` of (x and y packed into one float, age in seconds): x * 2048 + y, both in 1/2047ths of the
// element. Every packed value is an integer below 2^24, which a float holds exactly. An age below 0
// is a ring that is not there.
[SkEffect("ripples")]
[SkDoc("interfering water rings that refract the backdrop")]
[SkBackdrop]
struct Ripples : IEffect
{
    [SkDefault("0 -1")]
    float2 first;
    [SkDefault("0 -1")]
    float2 second;
    [SkDefault("0 -1")]
    float2 third;
    [SkDefault("0 -1")]
    float2 fourth;

    // Height of one ring at `px` and its slope along the ring's radius.
    float2 ring(float2 packed, EffectInput input, out float2 outward)
    {
        outward = float2(0.0, 0.0);
        float age = packed.y;
        if (age < 0.0 || age > 3.0)
            return float2(0.0, 0.0);
        float2 at = float2(floor(packed.x / 2048.0), fmod(packed.x, 2048.0)) / 2047.0;
        float2 d = input.px - at * input.size;
        float r = length(d);
        outward = d / max(r, 1.0);

        float s = input.scale;
        float wavelength = 34.0 * s;
        float front = age * 300.0 * s;
        float width = (40.0 + 90.0 * age) * s;
        float x = (r - front) / width;
        float packet = exp(-x * x) * pow(1.0 - age / 3.0, 2.0) / sqrt(1.0 + r / (160.0 * s));
        float phase = (r - front) / wavelength * 6.28318530718;
        return float2(sin(phase), cos(phase) * 6.28318530718 / wavelength) * packet;
    }

    float4 shade(EffectInput input)
    {
        float2 slope = float2(0.0, 0.0);
        float height = 0.0;
        float2 outward;
        float2 h = ring(first, input, outward);
        height += h.x;
        slope += outward * h.y;
        h = ring(second, input, outward);
        height += h.x;
        slope += outward * h.y;
        h = ring(third, input, outward);
        height += h.x;
        slope += outward * h.y;
        h = ring(fourth, input, outward);
        height += h.x;
        slope += outward * h.y;

        // The surface's slope bends the view; its tilt toward a light at the top left makes highlights.
        float2 bend = slope * 70.0 * input.scale;
        float4 seen = sk_backdrop_input.SampleLevel(sk_clamp_sampler, input.uv + bend / sk_viewport_size, 0.0);
        float glint = saturate(dot(slope, float2(-0.6, -0.8)) * 5.0 * input.scale);
        float shade = 1.0 + height * 0.06;
        return float4(seen.rgb * shade + glint * glint * 0.35 * seen.a, seen.a);
    }
};
