import sk;

// Rings of water spreading from up to eight points, refracting whatever is behind the element. The
// rings add up as heights, so where two meet they interfere: crests stack, crest and trough cancel.
//
// A ring is where it started and when, on the effect clock (`performance.now() / 1000` in JS), so
// the page sets it once per click and the effect works out the age itself. The window repaints for
// three seconds after the newest ring and then the effect is still.
[SkEffect("ripples")]
[SkDoc("interfering water rings that refract the backdrop")]
[SkBackdrop]
struct Ripples : IEffect
{
    // Where each ring starts, 0..1 across the element.
    float2 at0;
    float2 at1;
    float2 at2;
    float2 at3;
    float2 at4;
    float2 at5;
    float2 at6;
    float2 at7;

    // When each ring started; long ago is no ring.
    [SkAnimatedFor(3.0)] [SkDefault("-1000")]
    float4 born0;
    [SkAnimatedFor(3.0)] [SkDefault("-1000")]
    float4 born1;

    // Height of one ring at `px` and its slope along the ring's radius.
    float2 ring(float2 at, float born, EffectInput input, out float2 outward)
    {
        outward = float2(0.0, 0.0);
        float age = input.time - born;
        if (age < 0.0 || age > 3.0)
            return float2(0.0, 0.0);
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
        float2 ats[8] = { at0, at1, at2, at3, at4, at5, at6, at7 };
        float borns[8] = { born0.x, born0.y, born0.z, born0.w, born1.x, born1.y, born1.z, born1.w };
        float2 slope = float2(0.0, 0.0);
        float height = 0.0;
        for (int i = 0; i < 8; i++)
        {
            float2 outward;
            float2 h = ring(ats[i], borns[i], input, outward);
            height += h.x;
            slope += outward * h.y;
        }

        // The surface's slope bends the view; its tilt toward a light at the top left makes highlights.
        float4 seen = input.backdropAtPx(input.px + slope * 70.0 * input.scale);
        float glint = saturate(dot(slope, float2(-0.6, -0.8)) * 5.0 * input.scale);
        float shade = 1.0 + height * 0.06;
        return float4(seen.rgb * shade + glint * glint * 0.35 * seen.a, seen.a);
    }
};
