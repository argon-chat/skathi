import sk;

// A radial spectrum: 24 bands from bass at the top to treble at the bottom, mirrored left and right, as
// glowing candy bars round a circle with falling peak caps and sparkles off the loudest tips.
//
// The page hands it the raw levels every frame. Smoothing and the peak caps live in the history: each texel
// keeps the state of the band its angle falls in (red the smoothed level, green the peak, blue how long
// the peak still holds, alpha its fall speed), so the bars rise fast, fall slowly and the caps drop under
// gravity on the GPU.
[SkEffect("spectrum")]
[SkDoc("a radial candy spectrum with smoothing and peak caps kept in its history")]
[SkHistory]
struct Spectrum : IEffect
{
    // Band levels 0..1, four per argument group, bass first.
    float4 bands0;
    float4 bands1;
    float4 bands2;
    float4 bands3;
    float4 bands4;
    float4 bands5;

    // Radius of the circle the bars stand on.
    [SkLength] [SkDefault("128")] [SkRange(8.0, 2048.0)]
    float inner;

    // Length of a bar at full level.
    [SkLength] [SkDefault("56")] [SkRange(4.0, 1024.0)]
    float reach;

    // When the levels last changed: the ring keeps animating for a moment after, so bars and caps settle.
    [SkAnimatedFor(2.0)] [SkDefault("-1000")]
    float fed;

    // The colour of the track; its alpha is how far it takes over the candy gradient.
    [SkColor] [SkDefault("#f7a8d04d")]
    float4 accent;

    static const int BANDS = 24;
    static const float PI = 3.14159265359;

    float raw(int j)
    {
        float4 groups[6] = { bands0, bands1, bands2, bands3, bands4, bands5 };
        float4 g = groups[clamp(j, 0, BANDS - 1) / 4];
        int k = clamp(j, 0, BANDS - 1) % 4;
        return k == 0 ? g.x : k == 1 ? g.y : k == 2 ? g.z : g.w;
    }

    // 0 at the top of the ring, 1 at the bottom, the same on both sides.
    float around(float2 v)
    {
        return 1.0 - abs(atan2(v.x, v.y)) / PI;
    }

    int band(float2 v)
    {
        return min(int(around(v) * float(BANDS)), BANDS - 1);
    }

    float2 direction(int j, float side)
    {
        float a = side * (1.0 - (float(j) + 0.5) / float(BANDS)) * PI;
        return float2(sin(a), cos(a));
    }

    float3 candy(float u)
    {
        float3 pink = float3(0.969, 0.659, 0.816);
        float3 lilac = float3(0.788, 0.655, 0.961);
        float3 blue = float3(0.624, 0.804, 0.961);
        float3 yellow = float3(0.984, 0.878, 0.627);
        float s = saturate(u) * 3.0;
        return s < 1.0 ? lerp(pink, lilac, s) : s < 2.0 ? lerp(lilac, blue, s - 1.0) : lerp(blue, yellow, s - 2.0);
    }

    override float4 update(EffectInput input)
    {
        float level = raw(band(input.px - input.size * 0.5));
        float4 last = input.history;
        float eased = level > last.r ? lerp(last.r, level, 0.6) : max(level, last.r * 0.93 - 0.002);

        float peak = last.g;
        float hold = last.b - 1.0 / 30.0;
        float fall = last.a;
        if (eased >= peak)
        {
            peak = eased;
            hold = 1.0;
            fall = 0.0;
        }
        else if (hold <= 0.0)
        {
            fall += 0.0007;
            peak = max(eased, peak - fall);
        }
        return float4(eased, peak, max(hold, 0.0), fall);
    }

    float4 sparkle(float2 d, float size, float3 colour)
    {
        float2 a = abs(d) / size;
        float s = exp(-a.x * 6.0) * exp(-a.y * 1.2) + exp(-a.y * 6.0) * exp(-a.x * 1.2);
        s *= exp(-length(a) * 0.9);
        return float4(colour * s, s);
    }

    float4 shade(EffectInput input)
    {
        float2 centre = input.size * 0.5;
        float2 v = input.px - centre;
        float r = length(v);
        float side = v.x < 0.0 ? -1.0 : 1.0;
        int b = band(v);
        float px = input.scale;

        float slot = 2.0 * PI * inner / (2.0 * float(BANDS));
        float w = 0.27 * slot;
        float base = inner + 6.0 * px;
        float stub = 2.0 * w + 1.0 * px;

        float bass = 0.0;
        for (int i = 0; i < 4; i++)
            bass += input.historyAtPx(centre + direction(i, 1.0) * inner * 0.5).r;
        bass *= 0.25;

        float4 colour = float4(0.0, 0.0, 0.0, 0.0);

        // The halo the bass breathes into, and the thin ring the bars stand on.
        float outside = r - inner;
        float halo = smoothstep(-12.0 * px, 0.0, outside) * exp(-max(outside, 0.0) / (34.0 * px)) * (0.06 + 0.3 * bass);
        float3 haloTint = lerp(float3(0.788, 0.655, 0.961), accent.rgb, saturate(0.3 + bass * 1.5));
        colour += float4(haloTint * halo, halo);
        float ring = exp(-pow((r - inner - 1.0 * px) / (1.3 * px), 2.0)) * (0.35 + 0.5 * bass);
        colour += float4(lerp(float3(0.93, 0.9, 1.0), accent.rgb, accent.a * 0.6) * ring, ring);

        for (int k = -1; k <= 1; k++)
        {
            int j = b + k;
            if (j < 0 || j >= BANDS)
                continue;
            float2 dir = direction(j, side);
            float4 state = input.historyAtPx(centre + dir * inner * 0.5);
            float level = saturate(state.r);
            float peak = saturate(state.g);
            float u = (float(j) + 0.5) / float(BANDS);
            float3 tint = lerp(candy(u), accent.rgb, accent.a);

            float along = dot(v, dir) - base;
            float across = v.x * dir.y - v.y * dir.x;
            float len = stub + level * reach;
            float seg = clamp(along, w, max(len - w, w));
            float d = length(float2(across, along - seg)) - w;

            float tip = saturate(along / max(reach, 1.0));
            float core = exp(-across * across / (w * w) * 2.5);
            float3 bar = lerp(tint * 0.8, lerp(accent.rgb, float3(1.0, 1.0, 1.0), 0.25), tip * tip) + core * 0.22;
            float glow = exp(-max(d, 0.0) / (7.0 * px)) * (0.12 + 0.55 * level) * 0.6;
            colour += float4(tint * glow, glow * 0.8);
            float body = input.aa(d) * 0.95;
            colour = sk_over(colour, float4(bar * body, body));

            float capAt = stub + peak * reach + 5.0 * px;
            float dc = length(float2(across, along - capAt)) - min(w * 0.75, 2.4 * px);
            float cap = input.aa(dc) * smoothstep(0.02, 0.08, peak);
            float capGlow = exp(-max(dc, 0.0) / (4.0 * px)) * 0.35 * peak;
            float3 capTint = lerp(lerp(float3(0.984, 0.878, 0.627), float3(1.0, 1.0, 1.0), 0.4), accent.rgb, accent.a * accent.a);
            colour += float4(capTint * capGlow, capGlow);
            colour = sk_over(colour, float4(capTint * cap, cap));

            float beat = floor(input.time * 2.5 + float(j) * 0.37);
            float2 h = sk_hash22(float2(float(j) * 3.1 + side, beat));
            float spark = level * level * step(0.55, h.x) * (1.0 - frac(input.time * 2.5 + float(j) * 0.37));
            float2 at = centre + dir * (base + len + (8.0 + 12.0 * h.y) * px) + float2(-dir.y, dir.x) * (h.x - 0.75) * slot;
            colour += sparkle(input.px - at, 6.0 * px, lerp(tint, float3(1.0, 1.0, 1.0), 0.6)) * spark;
        }

        return float4(colour.rgb, saturate(colour.a));
    }
};
