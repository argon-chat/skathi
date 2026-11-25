// sk.sample — filter kernels over the engine's samplers: blurs, chromatic aberration,
// displacement, bicubic and sharpening, each with a tap count fixed at compile time.
//
//     import sk;
//     import sk.sample;
//
//     float4 soft = gaussian<Gaussian9>(Backdrop(input), input.px);          // 25 taps, sigma 1.41 px
//     float4 bokeh = disc<24>(RawBackdrop(input), input.px, 6.0);              // 24 taps
//     float4 fringe = dispersion(RawBackdrop(input), input.px, offset, 0.15);  // 3 taps
//
// Every kernel takes a source (`Backdrop`, `RawBackdrop`, `Content`, `History`, `Image`, or your own
// `ISampler`) and a point in that source's pixels: `input.px` for the first four. Sources are
// premultiplied and kernels only take weighted sums, so premultiplied in is premultiplied out and
// a transparent source gives exact zeros. Each kernel states how far from `px` it reads: its reach.
// Size an `[SkBackdropReach]` from it; a read beyond the declared reach returns the edge of what
// the engine copied, so a kernel there smears that edge instead of failing.

module "sk.sample";

import sk;

// ===========================================================================
//  Sources
// ===========================================================================

/// Something kernels sample: premultiplied RGBA at a point in its own pixel space, bilinear.
public interface ISampler
{
    /// The source at `px`, filtered bilinearly between texels.
    float4 at(float2 px);
    /// Where texel centres sit: at `texelOffset() + k` for whole k. Bicubic kernels align to it.
    float2 texelOffset();
}

/// Where window texel centres fall in an element's pixel space.
float2 sk_window_offset(EffectInput input)
{
    float2 origin = input.uv * sk_viewport_size - input.px;
    return frac(0.5 - origin);
}

/// The backdrop, blurred to `[SkBackdropRadius]` (`input.backdropAtPx`): a lower-resolution
/// pyramid level when blurred, so bicubic kernels are pointless on it.
public struct Backdrop : ISampler
{
    EffectInput input;

    public __init(EffectInput input)
    {
        this.input = input;
    }

    public float4 at(float2 px)
    {
        return this.input.backdropAtPx(px);
    }

    public float2 texelOffset()
    {
        return sk_window_offset(this.input);
    }
};

/// The untouched backdrop, one texel per device pixel (`input.rawBackdropAtPx`).
public struct RawBackdrop : ISampler
{
    EffectInput input;

    public __init(EffectInput input)
    {
        this.input = input;
    }

    public float4 at(float2 px)
    {
        return this.input.rawBackdropAtPx(px);
    }

    public float2 texelOffset()
    {
        return sk_window_offset(this.input);
    }
};

/// The page alone, without native layers under it (`input.pageBackdropAtPx`), one texel per pixel.
public struct PageBackdrop : ISampler
{
    EffectInput input;

    public __init(EffectInput input)
    {
        this.input = input;
    }

    public float4 at(float2 px)
    {
        return this.input.pageBackdropAtPx(px);
    }

    public float2 texelOffset()
    {
        return sk_window_offset(this.input);
    }
};

/// The element's own painting (`[SkContent]`, `input.contentAtPx`).
public struct Content : ISampler
{
    EffectInput input;

    public __init(EffectInput input)
    {
        this.input = input;
    }

    public float4 at(float2 px)
    {
        return this.input.contentAtPx(px);
    }

    public float2 texelOffset()
    {
        return sk_window_offset(this.input);
    }
};

/// The effect's state as the last step stored it (`[SkHistory]`, `input.historyAtPx`): raw values,
/// not premultiplied, which the kernels filter the same way.
public struct History : ISampler
{
    EffectInput input;

    public __init(EffectInput input)
    {
        this.input = input;
    }

    public float4 at(float2 px)
    {
        return this.input.historyAtPx(px);
    }

    public float2 texelOffset()
    {
        return frac(0.5 - this.input.bleed);
    }
};

/// An `SkTexture` in its own texels: `px` 0..size across the image, clamped to its edge, or
/// repeating with `repeat`. Its `[SkDefault]` colour while it is missing.
public struct Image : ISampler
{
    SkTexture texture;
    bool repeat;

    public __init(SkTexture texture, bool repeat = false)
    {
        this.texture = texture;
        this.repeat = repeat;
    }

    public float4 at(float2 px)
    {
        float2 uv = px / max(this.texture.size, float2(1.0, 1.0));
        return this.repeat ? this.texture.sampleRepeat(uv) : this.texture.sample(uv);
    }

    public float2 texelOffset()
    {
        return float2(0.5, 0.5);
    }
};

/// A window UV (`input.uv` space) as a point in the element's pixels, the space kernels take.
public float2 uvToPx(EffectInput input, float2 uv)
{
    return input.px + (uv - input.uv) * sk_viewport_size;
}

// ===========================================================================
//  Gaussian blur
// ===========================================================================

/// A small Gaussian kernel: binomial weights with neighbouring taps merged into one bilinear read
/// (the linear-sampling trick), `taps` reads per axis.
public interface IGaussian
{
    /// Bilinear reads per axis; the 2D kernel reads taps x taps.
    static const int taps;
    /// The i-th read's offset in texels, symmetric about 0.
    static float offset(int i);
    /// The i-th read's weight; the weights sum to 1.
    static float weight(int i);
    /// The farthest texel read, in texels at scale 1.
    static const int extent;
}

/// Binomial 1 4 6 4 1 (sigma 1 px): 3 reads per axis, 9 in all, reach 2 px.
public struct Gaussian5 : IGaussian
{
    static const int taps = 3;
    static const int extent = 2;
    static const float OFFSETS[3] = { -1.2, 0.0, 1.2 };
    static const float WEIGHTS[3] = { 0.3125, 0.375, 0.3125 };
    static float offset(int i) { return OFFSETS[i]; }
    static float weight(int i) { return WEIGHTS[i]; }
};

/// Binomial row 8 (sigma 1.41 px): 5 reads per axis, 25 in all, reach 4 px.
public struct Gaussian9 : IGaussian
{
    static const int taps = 5;
    static const int extent = 4;
    static const float OFFSETS[5] = { -3.1111111111111111, -1.3333333333333333, 0.0, 1.3333333333333333,
        3.1111111111111111 };
    static const float WEIGHTS[5] = { 0.03515625, 0.328125, 0.2734375, 0.328125, 0.03515625 };
    static float offset(int i) { return OFFSETS[i]; }
    static float weight(int i) { return WEIGHTS[i]; }
};

/// Binomial row 12 (sigma 1.73 px): 7 reads per axis, 49 in all, reach 6 px.
public struct Gaussian13 : IGaussian
{
    static const int taps = 7;
    static const int extent = 6;
    static const float OFFSETS[7] = { -5.0769230769230769, -3.2307692307692308, -1.3846153846153846, 0.0,
        1.3846153846153846, 3.2307692307692308, 5.0769230769230769 };
    static const float WEIGHTS[7] = { 0.003173828125, 0.06982421875, 0.314208984375, 0.2255859375,
        0.314208984375, 0.06982421875, 0.003173828125 };
    static float offset(int i) { return OFFSETS[i]; }
    static float weight(int i) { return WEIGHTS[i]; }
};

/// A Gaussian blur of `src` around `px`: `K` is `Gaussian5`, `Gaussian9` or `Gaussian13`
/// (9, 25 or 49 reads). Exact on a texel centre at `scale` 1 (`input.px` is one); `scale`
/// spreads the reads for a wider, sparser blur. For radii past a few pixels use `[SkBackdropRadius]`.
public float4 gaussian<K : IGaussian, S : ISampler>(S src, float2 px, float scale = 1.0)
{
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    [ForceUnroll]
    for (int y = 0; y < K.taps; y++)
    {
        [ForceUnroll]
        for (int x = 0; x < K.taps; x++)
        {
            float2 o = float2(K.offset(x), K.offset(y)) * scale;
            sum += src.at(px + o) * (K.weight(x) * K.weight(y));
        }
    }
    return sum;
}

/// A one-dimensional Gaussian along `direction` (unit, in pixels): `K.taps` reads. Two of them at
/// right angles are not the 2D blur in one pass; it is a streak, for motion or anamorphic glare.
public float4 gaussianLine<K : IGaussian, S : ISampler>(S src, float2 px, float2 direction, float scale = 1.0)
{
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    [ForceUnroll]
    for (int i = 0; i < K.taps; i++)
    {
        sum += src.at(px + direction * (K.offset(i) * scale)) * K.weight(i);
    }
    return sum;
}

/// How far `gaussian<K>` and `gaussianLine<K>` read from `px` at this scale, in pixels.
public float gaussianReach<K : IGaussian>(float scale = 1.0)
{
    return float(K.extent) * max(scale, 1.0) + 1.0;
}

// ===========================================================================
//  Box, disc, directional and radial blur
// ===========================================================================

/// A box blur of half-width `radius` pixels: 4 x 4 bilinear reads spread over the square.
/// Exact for `radius` 2; a midpoint approximation beyond, which grows grainy past about 6.
public float4 box<S : ISampler>(S src, float2 px, float radius)
{
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    float step = radius * 0.5;
    [ForceUnroll]
    for (int y = 0; y < 4; y++)
    {
        [ForceUnroll]
        for (int x = 0; x < 4; x++)
        {
            sum += src.at(px + (float2(float(x), float(y)) - 1.5) * step);
        }
    }
    return sum * 0.0625;
}

/// How far `box` reads from `px`, in pixels.
public float boxReach(float radius)
{
    return 0.75 * abs(radius) + 1.0;
}

static const float SK_GOLDEN_ANGLE = 2.39996322972865332;

/// A disc (bokeh) blur of `radius` pixels: `N` reads on a golden-angle (Vogel) spiral, even over
/// the disc. 16 to 32 hide the pattern on a smooth source.
public float4 disc<let N : int, S : ISampler>(S src, float2 px, float radius)
{
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    [ForceUnroll]
    for (int i = 0; i < N; i++)
    {
        float r = radius * sqrt((float(i) + 0.5) / float(N));
        float a = float(i) * SK_GOLDEN_ANGLE;
        sum += src.at(px + r * float2(cos(a), sin(a)));
    }
    return sum / float(N);
}

/// How far `disc` reads from `px`, in pixels.
public float discReach(float radius)
{
    return abs(radius) + 1.0;
}

/// A motion blur along `delta` pixels: `N` even reads on the segment centred on `px`.
public float4 directional<let N : int, S : ISampler>(S src, float2 px, float2 delta)
{
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    [ForceUnroll]
    for (int i = 0; i < N; i++)
    {
        sum += src.at(px + delta * ((float(i) + 0.5) / float(N) - 0.5));
    }
    return sum / float(N);
}

/// How far `directional` reads from `px`, in pixels.
public float directionalReach(float2 delta)
{
    return 0.5 * length(delta) + 1.0;
}

/// A zoom blur towards `center` (pixels): `N` reads from `px` to `amount` of the way to the centre.
public float4 radial<let N : int, S : ISampler>(S src, float2 px, float2 center, float amount)
{
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    [ForceUnroll]
    for (int i = 0; i < N; i++)
    {
        sum += src.at(lerp(px, center, amount * float(i) / float(N)));
    }
    return sum / float(N);
}

/// How far outside an element of `size` pixels `radial` reads: none when `center` is inside it.
public float radialReach(float2 center, float amount, float2 size)
{
    float2 outside = max(abs(center - size * 0.5) - size * 0.5, float2(0.0, 0.0));
    return saturate(amount) * length(outside) + 1.0;
}

// ===========================================================================
//  Colour fringes and displacement
// ===========================================================================

/// Premultiplied RGBA from three reads, red from one, green and alpha from the second, blue from
/// the third; alpha is the largest of the three so the result stays premultiplied.
float4 sk_channels(float4 r, float4 g, float4 b)
{
    return float4(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
}

/// Chromatic aberration: red read `shift` pixels one way, blue the other, green at `px`. 3 reads.
public float4 chromatic<S : ISampler>(S src, float2 px, float2 shift)
{
    return sk_channels(src.at(px + shift), src.at(px), src.at(px - shift));
}

/// Lens-like chromatic aberration growing from `center`: the shift is `(px - center) * amount`. 3 reads.
public float4 chromaticRadial<S : ISampler>(S src, float2 px, float2 center, float amount)
{
    return chromatic(src, px, (px - center) * amount);
}

/// How far `chromatic` reads from `px`, in pixels.
public float chromaticReach(float2 shift)
{
    return length(shift) + 1.0;
}

/// Dispersion: a read displaced by `offset` with red bent `1 + spread` times as far and blue
/// `1 - spread`, as glass bends short wavelengths more. 3 reads.
public float4 dispersion<S : ISampler>(S src, float2 px, float2 offset, float spread)
{
    return sk_channels(src.at(px + offset * (1.0 + spread)), src.at(px + offset), src.at(px + offset * (1.0 - spread)));
}

/// How far `dispersion` reads from `px`, in pixels.
public float dispersionReach(float2 offset, float spread)
{
    return length(offset) * (1.0 + abs(spread)) + 1.0;
}

/// The source displaced by `offset` pixels: compose with a field (`curl`, noise) or with
/// `refractionOffset` from `sk.light`. 1 read.
public float4 displace<S : ISampler>(S src, float2 px, float2 offset)
{
    return src.at(px + offset);
}

/// The source displaced by a unit normal's tilt times `depth` pixels, as a normal-mapped pane of
/// glass bends it. 1 read, at most `depth` + 1 away.
public float4 displace<S : ISampler>(S src, float2 px, float3 normal, float depth)
{
    return src.at(px + normal.xy * depth);
}

// ===========================================================================
//  Resampling
// ===========================================================================

/// The whole-texel cell and fraction of `px` on a source's texel grid.
void sk_grid<S : ISampler>(S src, float2 px, out float2 base, out float2 f)
{
    float2 o = src.texelOffset();
    float2 t = px - o;
    float2 i = floor(t);
    base = i + o;
    f = t - i;
}

/// Catmull-Rom bicubic resampling: sharp, for upscaled history or content. 9 bilinear reads (the
/// two middle taps of each axis merged). It can overshoot; negative results are clamped to 0.
public float4 catmullRom<S : ISampler>(S src, float2 px)
{
    float2 base;
    float2 f;
    sk_grid(src, px, base, f);
    float2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    float2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    float2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    float2 w3 = f * f * (-0.5 + 0.5 * f);
    float2 w12 = w1 + w2;
    float2 p0 = base - 1.0;
    float2 p12 = base + w2 / w12;
    float2 p3 = base + 2.0;
    float4 sum = src.at(float2(p0.x, p0.y)) * (w0.x * w0.y) + src.at(float2(p12.x, p0.y)) * (w12.x * w0.y)
        + src.at(float2(p3.x, p0.y)) * (w3.x * w0.y)
        + src.at(float2(p0.x, p12.y)) * (w0.x * w12.y) + src.at(float2(p12.x, p12.y)) * (w12.x * w12.y)
        + src.at(float2(p3.x, p12.y)) * (w3.x * w12.y)
        + src.at(float2(p0.x, p3.y)) * (w0.x * w3.y) + src.at(float2(p12.x, p3.y)) * (w12.x * w3.y)
        + src.at(float2(p3.x, p3.y)) * (w3.x * w3.y);
    return max(sum, float4(0.0, 0.0, 0.0, 0.0));
}

/// Cubic B-spline resampling: smooth, never overshoots, softer than Catmull-Rom. 4 bilinear reads.
public float4 bspline<S : ISampler>(S src, float2 px)
{
    float2 base;
    float2 f;
    sk_grid(src, px, base, f);
    float2 g = 1.0 - f;
    float2 w0 = g * g * g / 6.0;
    float2 w1 = (4.0 - 6.0 * f * f + 3.0 * f * f * f) / 6.0;
    float2 w3 = f * f * f / 6.0;
    float2 w2 = 1.0 - w0 - w1 - w3;
    float2 s0 = w0 + w1;
    float2 s1 = w2 + w3;
    float2 p0 = base - 1.0 + w1 / s0;
    float2 p1 = base + 1.0 + w3 / s1;
    return src.at(float2(p0.x, p0.y)) * (s0.x * s0.y) + src.at(float2(p1.x, p0.y)) * (s1.x * s0.y)
        + src.at(float2(p0.x, p1.y)) * (s0.x * s1.y) + src.at(float2(p1.x, p1.y)) * (s1.x * s1.y);
}

/// Unsharp masking: `px` pushed away from its 3x3 tent blur by `amount` (0.5 subtle, 2 strong).
/// 5 reads, reach 2 px. Negative results are clamped to 0.
public float4 sharpen<S : ISampler>(S src, float2 px, float amount)
{
    float4 c = src.at(px);
    float4 blur = (src.at(px + float2(-0.5, -0.5)) + src.at(px + float2(0.5, -0.5)) + src.at(px + float2(-0.5, 0.5))
        + src.at(px + float2(0.5, 0.5))) * 0.25;
    return max(c + (c - blur) * amount, float4(0.0, 0.0, 0.0, 0.0));
}

// ===========================================================================
//  Coordinates
// ===========================================================================

/// The centre of the `cell`-pixel square holding `px`: read there for a mosaic.
public float2 cellCenter(float2 px, float2 cell)
{
    float2 c = max(cell, float2(1e-4, 1e-4));
    return (floor(px / c) + 0.5) * c;
}

/// A pixelated (mosaic) source: the colour at the centre of each `cell`-pixel square. 1 read.
public float4 pixelate<S : ISampler>(S src, float2 px, float2 cell)
{
    return src.at(cellCenter(px, cell));
}

/// `p` folded into [0, size] by mirroring at each edge, as a mirrored-repeat sampler addresses.
public float2 mirrorRepeat(float2 p, float2 size)
{
    float2 s = max(size, float2(1e-4, 1e-4));
    float2 t = p / s;
    float2 m = t - 2.0 * floor(t * 0.5);
    return (1.0 - abs(1.0 - m)) * s;
}

/// `p` folded into one wedge of a kaleidoscope of `segments` mirrored wedges about `center`, the
/// first starting at `rotation` radians: sample there.
public float2 kaleidoscope(float2 p, float2 center, int segments, float rotation = 0.0)
{
    float2 d = p - center;
    float wedge = 6.28318530717958648 / float(max(segments, 1));
    float a = atan2(d.y, d.x) - rotation;
    a = a - wedge * floor(a / wedge);
    a = abs(a - 0.5 * wedge);
    float r = length(d);
    return center + r * float2(cos(a + rotation), sin(a + rotation));
}
