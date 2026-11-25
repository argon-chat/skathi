// sk.tone — tone curves, exposure and HDR transfer functions.
//
//     import sk;
//     import sk.tone;
//
//     float3 light = exposure(sk_frame_to_linear(input.backdrop).rgb, -0.5);
//     float3 sdr = byLuminance(AcesFitted(), light);              // 0..1
//     float3 kept = softClip(light, 1.0, 4.0);                    // HDR, rolled off at 4x SDR white
//
// Scene light is linear with 1.0 at SDR white, as `sk_frame_to_linear` and a `[SkBackdropLinear]`
// backdrop give it. Curves take straight (not premultiplied) light; negative input counts as black
// unless a function says otherwise. Nits are absolute cd/m².

module "sk.tone";

/// BT.709 / sRGB luminance weights.
public static const float3 SK_LUMA_709 = float3(0.2126, 0.7152, 0.0722);
/// Nits of SDR white in scRGB (Windows HDR): 1.0 is 80 nits.
public static const float SK_SCRGB_WHITE_NITS = 80.0;
/// Nits of SDR white BT.2408 recommends for HDR video (HDR reference white).
public static const float SK_HDR_REFERENCE_WHITE_NITS = 203.0;

/// A tone curve: scene light (0..inf) in, display light out.
public interface ITone
{
    float at(float x);
};

/// `x / (1 + x)`: 0..inf to 0..1, never reaching 1.
public struct Reinhard : ITone
{
    public __init() {}
    public float at(float x)
    {
        float v = max(x, 0.0);
        return v / (1.0 + v);
    }
};

/// Reinhard with a white point: `x (1 + x / white²) / (1 + x)`, 0 to 0 and `white` to exactly 1;
/// light above `white` goes above 1. `white` > 0.
public struct ReinhardExtended : ITone
{
    public float white;
    public __init(float white) { this.white = max(white, 1e-4); }
    public float at(float x)
    {
        float v = max(x, 0.0);
        return v * (1.0 + v / (white * white)) / (1.0 + v);
    }
};

/// A filmic curve with a toe and a shoulder (the Hable form with its usual constants), scaled so
/// `white` maps to 1. 0 maps to 0; above `white` it rises slowly towards about 1.29 (white 11.2).
public struct Hable : ITone
{
    public float white;
    public __init(float white = 11.2) { this.white = max(white, 1e-4); }

    static float curve(float x)
    {
        const float A = 0.15;
        const float B = 0.50;
        const float C = 0.10;
        const float D = 0.20;
        const float E = 0.02;
        const float F = 0.30;
        return (x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F) - E / F;
    }

    public float at(float x) { return curve(max(x, 0.0)) / curve(white); }
};

/// A rational fit of the ACES film look (RRT + sRGB ODT): 0..inf to 0..1, 1 reached near x = 10.
/// The fit expects scene light already scaled by 0.6 relative to ACES; pass `exposure(x, -0.737)`
/// to match ACES exactly.
public struct AcesFitted : ITone
{
    public __init() {}
    public float at(float x)
    {
        float v = max(x, 0.0);
        return saturate(v * (2.51 * v + 0.03) / (v * (2.43 * v + 0.59) + 0.14));
    }
};

/// `softClip` as a tone curve.
public struct SoftClip : ITone
{
    public float knee;
    public float limit;
    public __init(float knee, float limit)
    {
        this.knee = knee;
        this.limit = limit;
    }
    public float at(float x) { return softClip(x, knee, limit); }
};

/// The curve on each channel. Hue shifts and saturation drops towards white in highlights.
public float3 perChannel<T : ITone>(T curve, float3 rgb)
{
    return float3(curve.at(rgb.r), curve.at(rgb.g), curve.at(rgb.b));
}

/// The curve on BT.709 luminance, the colour scaled to match: hue and saturation kept, so a
/// channel can end above the curve's range. Zero or negative luminance gives black.
public float3 byLuminance<T : ITone>(T curve, float3 rgb)
{
    float y = dot(rgb, SK_LUMA_709);
    return y > 0.0 ? rgb * (curve.at(y) / y) : float3(0.0);
}

/// BT.709 luminance of linear light, 1.0 at SDR white. Any sign.
public float luminance709(float3 rgb)
{
    return dot(rgb, SK_LUMA_709);
}

/// Light scaled by `2^stops`: +1 doubles it, -1 halves it. Any sign, any stops.
public float exposure(float x, float stops)
{
    return x * exp2(stops);
}

/// Per component `exposure`.
public vector<float, N> exposure<let N : int>(vector<float, N> x, float stops)
{
    return x * exp2(stops);
}

/// Identity up to `knee`, then an exponential roll-off that meets it with slope 1 and approaches
/// `limit` without reaching it. Below `knee` any sign passes through. With `limit` <= `knee` it is
/// `min(x, limit)`.
public float softClip(float x, float knee, float limit)
{
    float span = limit - knee;
    if (span <= 0.0)
        return min(x, limit);
    return x <= knee ? x : knee + span * (1.0 - exp(-(x - knee) / span));
}

/// Per component `softClip`.
public vector<float, N> softClip<let N : int>(vector<float, N> x, float knee, float limit)
{
    vector<float, N> r;
    [ForceUnroll]
    for (int i = 0; i < N; i++)
        r[i] = softClip(x[i], knee, limit);
    return r;
}

/// Absolute nits from scene light with SDR white at `sdrWhiteNits` (80 in scRGB, 203 for HDR
/// video). Any sign.
public vector<float, N> sceneToNits<let N : int>(vector<float, N> x, float sdrWhiteNits)
{
    return x * sdrWhiteNits;
}

/// Scene light (1.0 at SDR white) from absolute nits. `sdrWhiteNits` > 0.
public vector<float, N> nitsToScene<let N : int>(vector<float, N> nits, float sdrWhiteNits)
{
    return nits / sdrWhiteNits;
}

/// Scalar `sceneToNits`.
public float sceneToNits(float x, float sdrWhiteNits)
{
    return x * sdrWhiteNits;
}

/// Scalar `nitsToScene`.
public float nitsToScene(float nits, float sdrWhiteNits)
{
    return nits / sdrWhiteNits;
}

/// SMPTE ST 2084 (PQ) signal 0..1 from absolute nits 0..10000 (clamped to that range).
public vector<float, N> pqEncode<let N : int>(vector<float, N> nits)
{
    const float m1 = 2610.0 / 16384.0;
    const float m2 = 2523.0 / 4096.0 * 128.0;
    const float c1 = 3424.0 / 4096.0;
    const float c2 = 2413.0 / 4096.0 * 32.0;
    const float c3 = 2392.0 / 4096.0 * 32.0;
    vector<float, N> y = pow(clamp(nits / 10000.0, 0.0, 1.0), vector<float, N>(m1));
    return pow((c1 + c2 * y) / (1.0 + c3 * y), vector<float, N>(m2));
}

/// Absolute nits 0..10000 from a PQ signal 0..1 (clamped to that range).
public vector<float, N> pqDecode<let N : int>(vector<float, N> signal)
{
    const float m1 = 2610.0 / 16384.0;
    const float m2 = 2523.0 / 4096.0 * 128.0;
    const float c1 = 3424.0 / 4096.0;
    const float c2 = 2413.0 / 4096.0 * 32.0;
    const float c3 = 2392.0 / 4096.0 * 32.0;
    vector<float, N> p = pow(clamp(signal, 0.0, 1.0), vector<float, N>(1.0 / m2));
    return 10000.0 * pow(max(p - c1, 0.0) / (c2 - c3 * p), vector<float, N>(1.0 / m1));
}

/// BT.2100 HLG OETF: signal 0..1 from normalised scene light 0..1 (clamped to that range).
public float hlgEncode(float e)
{
    const float a = 0.17883277;
    const float b = 1.0 - 4.0 * a;
    const float c = 0.5 - a * log(4.0 * a);
    float v = saturate(e);
    return v <= 1.0 / 12.0 ? sqrt(3.0 * v) : a * log(12.0 * v - b) + c;
}

/// BT.2100 HLG inverse OETF: normalised scene light 0..1 from a signal 0..1 (clamped).
public float hlgDecode(float signal)
{
    const float a = 0.17883277;
    const float b = 1.0 - 4.0 * a;
    const float c = 0.5 - a * log(4.0 * a);
    float v = saturate(signal);
    return v <= 0.5 ? v * v / 3.0 : (exp((v - c) / a) + b) / 12.0;
}

/// Per component `hlgEncode`.
public vector<float, N> hlgEncode<let N : int>(vector<float, N> e)
{
    vector<float, N> r;
    [ForceUnroll]
    for (int i = 0; i < N; i++)
        r[i] = hlgEncode(e[i]);
    return r;
}

/// Per component `hlgDecode`.
public vector<float, N> hlgDecode<let N : int>(vector<float, N> signal)
{
    vector<float, N> r;
    [ForceUnroll]
    for (int i = 0; i < N; i++)
        r[i] = hlgDecode(signal[i]);
    return r;
}
