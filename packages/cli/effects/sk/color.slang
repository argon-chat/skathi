// sk.color — colours with their encoding in the type.
//
//     import sk;
//     import sk.color;
//
//     Color tint = Color.fromSrgb(tintParam);            // an [SkColor] field
//     Color back = Color.fromPremultiplied(input.backdrop);
//     return back.mix<Oklch>(tint, 0.3).toPremultiplied();
//
// A `Color` holds linear sRGB light with straight alpha. Linear values are not
// clamped: 1.0 is SDR white, above it is HDR, below 0 is outside the sRGB
// gamut. Conversions into a space carry such values wherever the space can
// (linear sRGB, XYZ, Lab, OKLab and their polar forms); the engine clamps what
// an effect returns, not this module.

module "sk.color";

// ===========================================================================
//  Colour spaces
// ===========================================================================

/// A colour space a `Color` converts to and from. Coordinates are float3 in
/// the space's own units (see each space); hue is in degrees, 0..360.
public interface IColorSpace
{
    /// Coordinates in this space from linear sRGB (D65).
    static float3 fromLinearSrgb(float3 rgb);
    /// Linear sRGB (D65) from coordinates in this space.
    static float3 toLinearSrgb(float3 c);
    /// Which coordinate is the hue: 0 (h, s, l), 2 (L, C, h), or -1 for none.
    static int hueIndex();
    /// Whether the hue of `c` is powerless (CSS Color 4): a grey has none.
    static bool achromatic(float3 c);
};

// CSS Color 4, sample code: linear sRGB <-> XYZ (D65), and Bradford D65 <-> D50.
static const float3 SK_SRGB_TO_XYZ0 = float3(0.41239079926595934, 0.357584339383878, 0.1804807884018343);
static const float3 SK_SRGB_TO_XYZ1 = float3(0.21263900587151027, 0.715168678767756, 0.07219231536073371);
static const float3 SK_SRGB_TO_XYZ2 = float3(0.01933081871559182, 0.11919477979462598, 0.9505321522496607);
static const float3 SK_XYZ_TO_SRGB0 = float3(3.2409699419045226, -1.537383177570094, -0.4986107602930034);
static const float3 SK_XYZ_TO_SRGB1 = float3(-0.9692436362808796, 1.8759675015077202, 0.04155505740717559);
static const float3 SK_XYZ_TO_SRGB2 = float3(0.05563007969699366, -0.20397695888897652, 1.0569715142428786);
static const float3 SK_D65_TO_D500 = float3(1.0479297925449969, 0.022946870601609652, -0.05019226628920524);
static const float3 SK_D65_TO_D501 = float3(0.02962780877005599, 0.9904344267538799, -0.017073799063418826);
static const float3 SK_D65_TO_D502 = float3(-0.009243040646204504, 0.015055191490298152, 0.7518742814281371);
static const float3 SK_D50_TO_D650 = float3(0.955473421488075, -0.02309845494876471, 0.06325924320057072);
static const float3 SK_D50_TO_D651 = float3(-0.0283697093338637, 1.0099953980813041, 0.021041441191917323);
static const float3 SK_D50_TO_D652 = float3(0.012314014864481998, -0.020507649298898964, 1.330365926242124);
static const float3 SK_D50 = float3(0.9642956764295677, 1.0, 0.8251046025104602);
// OKLab, CSS Color 4 sample code (XYZ D65 -> LMS -> OKLab and back).
static const float3 SK_XYZ_TO_LMS0 = float3(0.8190224379967030, 0.3619062600528904, -0.1288737815209879);
static const float3 SK_XYZ_TO_LMS1 = float3(0.0329836539323885, 0.9292868615863434, 0.0361446663506424);
static const float3 SK_XYZ_TO_LMS2 = float3(0.0481771893596242, 0.2642395317527308, 0.6335478284694309);
static const float3 SK_LMS_TO_OKLAB0 = float3(0.2104542683093140, 0.7936177747023054, -0.0040720430116193);
static const float3 SK_LMS_TO_OKLAB1 = float3(1.9779985324311684, -2.4285922420485799, 0.4505937096174110);
static const float3 SK_LMS_TO_OKLAB2 = float3(0.0259040424655478, 0.7827717124575296, -0.8086757549230774);
static const float3 SK_LMS_TO_XYZ0 = float3(1.2268798758459243, -0.5578149944602171, 0.2813910456659647);
static const float3 SK_LMS_TO_XYZ1 = float3(-0.0405757452148008, 1.1122868032803170, -0.0717110580655164);
static const float3 SK_LMS_TO_XYZ2 = float3(-0.0763729366746601, -0.4214933324022432, 1.5869240198367816);
static const float3 SK_OKLAB_TO_LMS0 = float3(1.0, 0.3963377773761749, 0.2158037573099136);
static const float3 SK_OKLAB_TO_LMS1 = float3(1.0, -0.1055613458156586, -0.0638541728258133);
static const float3 SK_OKLAB_TO_LMS2 = float3(1.0, -0.0894841775298119, -1.2914855480194092);

float3 sk_rows(float3 r0, float3 r1, float3 r2, float3 v)
{
    return float3(dot(r0, v), dot(r1, v), dot(r2, v));
}

/// Hue in degrees wrapped into [0, 360); a hue just below 0 can round to 360.
float sk_wrap_hue(float h)
{
    float w = h - 360.0 * floor(h / 360.0);
    return w < 360.0 ? w : 0.0;
}

/// Signed power: `pow(|x|, e)` with the sign of `x`, 0 at 0, never NaN.
float3 sk_signed_pow(float3 x, float e)
{
    float3 m = abs(x);
    float3 p = exp2(e * log2(max(m, float3(1e-30, 1e-30, 1e-30))));
    return sign(x) * select(m > 0.0, p, float3(0.0, 0.0, 0.0));
}

/// (L, C, h) from rectangular (L, a, b); a chroma at or below `grey` gets hue 0.
float3 sk_to_polar(float3 lab, float grey)
{
    float c = length(lab.yz);
    bool hued = c > grey;
    float h = hued ? sk_wrap_hue(degrees(atan2(lab.z, hued ? lab.y : 1.0))) : 0.0;
    return float3(lab.x, c, h);
}

float3 sk_from_polar(float3 lch)
{
    float h = radians(lch.z);
    return float3(lch.x, lch.y * cos(h), lch.y * sin(h));
}

/// (hue, max - min, max) of sRGB-encoded rgb; hue 0 when grey.
float3 sk_hue_of(float3 c)
{
    float hi = max(max(c.r, c.g), c.b);
    float lo = min(min(c.r, c.g), c.b);
    float d = hi - lo;
    float safe = d != 0.0 ? d : 1.0;
    float h = hi == c.r ? (c.g - c.b) / safe + (c.g < c.b ? 6.0 : 0.0)
        : (hi == c.g ? (c.b - c.r) / safe + 2.0 : (c.r - c.g) / safe + 4.0);
    return float3(d != 0.0 ? sk_wrap_hue(h * 60.0) : 0.0, d, hi);
}

/// sRGB-encoded rgb from hue (degrees), saturation and lightness, CSS Color 4 `hslToRgb`.
float3 sk_hsl_to_rgb(float h, float s, float l)
{
    float a = s * min(l, 1.0 - l);
    float3 k = float3(0.0, 8.0, 4.0) + sk_wrap_hue(h) / 30.0;
    k = k - 12.0 * floor(k / 12.0);
    return l - a * max(min(min(k - 3.0, 9.0 - k), 1.0), -1.0);
}

/// The sRGB transfer function, CSS Color 4: (r, g, b) in sRGB-encoded values,
/// 0..1 for SDR. Mirrored below 0 and extended above 1, as for scRGB.
public struct Srgb : IColorSpace
{
    /// Linear light -> sRGB-encoded, per channel, extended (sign-preserving).
    public static float3 encode(float3 linear)
    {
        float3 m = abs(linear);
        float3 e = select(m > 0.0031308, 1.055 * sk_signed_pow(m, 1.0 / 2.4) - 0.055, 12.92 * m);
        return sign(linear) * e;
    }

    /// sRGB-encoded -> linear light, per channel, extended (sign-preserving).
    public static float3 decode(float3 encoded)
    {
        float3 m = abs(encoded);
        float3 l = select(m > 0.04045, sk_signed_pow((m + 0.055) / 1.055, 2.4), m / 12.92);
        return sign(encoded) * l;
    }

    public static float3 fromLinearSrgb(float3 rgb) { return Srgb.encode(rgb); }
    public static float3 toLinearSrgb(float3 c) { return Srgb.decode(c); }
    public static int hueIndex() { return -1; }
    public static bool achromatic(float3 c) { return false; }
};

/// Linear-light sRGB (CSS `srgb-linear`): (r, g, b), 1.0 is SDR white.
public struct LinearSrgb : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb) { return rgb; }
    public static float3 toLinearSrgb(float3 c) { return c; }
    public static int hueIndex() { return -1; }
    public static bool achromatic(float3 c) { return false; }
};

/// CSS `hsl()` over sRGB-encoded values: (h degrees, s 0..1, l 0..1).
/// Saturation is made positive by turning the hue, as CSS Color 4 does.
public struct Hsl : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        float3 c = Srgb.encode(rgb);
        float3 hue = sk_hue_of(c);
        float l = hue.z - hue.y * 0.5;
        float m = min(l, 1.0 - l);
        bool chromatic = hue.y != 0.0 && m != 0.0;
        float s = chromatic ? (hue.z - l) / (chromatic ? m : 1.0) : 0.0;
        return float3(s < 0.0 ? sk_wrap_hue(hue.x + 180.0) : hue.x, abs(s), l);
    }

    public static float3 toLinearSrgb(float3 c)
    {
        return Srgb.decode(sk_hsl_to_rgb(c.x, c.y, c.z));
    }

    public static int hueIndex() { return 0; }
    public static bool achromatic(float3 c) { return abs(c.y) <= 0.001; }
};

/// HSV (HSB) over sRGB-encoded values: (h degrees, s 0..1, v 0..1).
public struct Hsv : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        float3 hue = sk_hue_of(Srgb.encode(rgb));
        float v = hue.z != 0.0 ? hue.z : 1.0;
        return float3(hue.x, hue.z != 0.0 ? hue.y / v : 0.0, hue.z);
    }

    public static float3 toLinearSrgb(float3 c)
    {
        float3 k = float3(5.0, 3.0, 1.0) + sk_wrap_hue(c.x) / 60.0;
        k = k - 6.0 * floor(k / 6.0);
        return Srgb.decode(c.z - c.z * c.y * max(min(min(k, 4.0 - k), 1.0), 0.0));
    }

    public static int hueIndex() { return 0; }
    public static bool achromatic(float3 c) { return abs(c.y) <= 0.001; }
};

/// CSS `hwb()` over sRGB-encoded values: (h degrees, whiteness 0..1, blackness 0..1).
/// Whiteness + blackness >= 1 is a grey, `w / (w + b)`, as in CSS.
public struct Hwb : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        float3 c = Srgb.encode(rgb);
        float3 hue = sk_hue_of(c);
        return float3(hue.x, hue.z - hue.y, 1.0 - hue.z);
    }

    public static float3 toLinearSrgb(float3 c)
    {
        float sum = c.y + c.z;
        float grey = c.y / max(sum, 1e-6);
        float3 pure = sk_hsl_to_rgb(c.x, 1.0, 0.5) * (1.0 - sum) + c.y;
        return Srgb.decode(sum >= 1.0 ? float3(grey, grey, grey) : pure);
    }

    public static int hueIndex() { return 0; }
    public static bool achromatic(float3 c) { return c.y + c.z >= 0.99999; }
};

/// CIE XYZ relative to D65 (CSS `xyz-d65`): Y 1.0 is SDR white.
public struct XyzD65 : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        return sk_rows(SK_SRGB_TO_XYZ0, SK_SRGB_TO_XYZ1, SK_SRGB_TO_XYZ2, rgb);
    }

    public static float3 toLinearSrgb(float3 c)
    {
        return sk_rows(SK_XYZ_TO_SRGB0, SK_XYZ_TO_SRGB1, SK_XYZ_TO_SRGB2, c);
    }

    public static int hueIndex() { return -1; }
    public static bool achromatic(float3 c) { return false; }
};

/// CIE XYZ relative to D50 (CSS `xyz-d50`), Bradford-adapted: Y 1.0 is SDR white.
public struct XyzD50 : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        return sk_rows(SK_D65_TO_D500, SK_D65_TO_D501, SK_D65_TO_D502, XyzD65.fromLinearSrgb(rgb));
    }

    public static float3 toLinearSrgb(float3 c)
    {
        return XyzD65.toLinearSrgb(sk_rows(SK_D50_TO_D650, SK_D50_TO_D651, SK_D50_TO_D652, c));
    }

    public static int hueIndex() { return -1; }
    public static bool achromatic(float3 c) { return false; }
};

/// CIE Lab (CSS `lab()`, D50): (L 0..100, a, b), a and b about -125..125.
/// L above 100 is brighter than SDR white.
public struct Lab : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        static const float e = 216.0 / 24389.0;
        static const float k = 24389.0 / 27.0;
        float3 xyz = XyzD50.fromLinearSrgb(rgb) / SK_D50;
        float3 f = select(xyz > e, sk_signed_pow(xyz, 1.0 / 3.0), (k * xyz + 16.0) / 116.0);
        return float3(116.0 * f.y - 16.0, 500.0 * (f.x - f.y), 200.0 * (f.y - f.z));
    }

    public static float3 toLinearSrgb(float3 c)
    {
        static const float e = 216.0 / 24389.0;
        static const float k = 24389.0 / 27.0;
        float fy = (c.x + 16.0) / 116.0;
        float3 f = float3(c.y / 500.0 + fy, fy, fy - c.z / 200.0);
        float3 cube = f * f * f;
        float3 xyz = float3(
            cube.x > e ? cube.x : (116.0 * f.x - 16.0) / k,
            c.x > k * e ? cube.y : c.x / k,
            cube.z > e ? cube.z : (116.0 * f.z - 16.0) / k);
        return XyzD50.toLinearSrgb(xyz * SK_D50);
    }

    public static int hueIndex() { return -1; }
    public static bool achromatic(float3 c) { return false; }
};

/// CIE LCH (CSS `lch()`, D50): (L 0..100, C 0..~150, h degrees).
/// A grey (C <= 0.0015, as in CSS) has hue 0.
public struct Lch : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb) { return sk_to_polar(Lab.fromLinearSrgb(rgb), 0.0015); }
    public static float3 toLinearSrgb(float3 c) { return Lab.toLinearSrgb(sk_from_polar(c)); }
    public static int hueIndex() { return 2; }
    public static bool achromatic(float3 c) { return c.y <= 0.0015; }
};

/// OKLab (CSS `oklab()`): (L 0..1, a, b), a and b about -0.4..0.4.
/// L above 1 is brighter than SDR white.
public struct Oklab : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb)
    {
        float3 lms = sk_rows(SK_XYZ_TO_LMS0, SK_XYZ_TO_LMS1, SK_XYZ_TO_LMS2, XyzD65.fromLinearSrgb(rgb));
        return sk_rows(SK_LMS_TO_OKLAB0, SK_LMS_TO_OKLAB1, SK_LMS_TO_OKLAB2, sk_signed_pow(lms, 1.0 / 3.0));
    }

    public static float3 toLinearSrgb(float3 c)
    {
        float3 lms = sk_rows(SK_OKLAB_TO_LMS0, SK_OKLAB_TO_LMS1, SK_OKLAB_TO_LMS2, c);
        return XyzD65.toLinearSrgb(sk_rows(SK_LMS_TO_XYZ0, SK_LMS_TO_XYZ1, SK_LMS_TO_XYZ2, lms * lms * lms));
    }

    public static int hueIndex() { return -1; }
    public static bool achromatic(float3 c) { return false; }
};

/// OKLCH (CSS `oklch()`): (L 0..1, C 0..~0.37, h degrees).
/// A grey (C <= 0.0001, above CSS's 0.000004 for float precision) has hue 0.
public struct Oklch : IColorSpace
{
    public static float3 fromLinearSrgb(float3 rgb) { return sk_to_polar(Oklab.fromLinearSrgb(rgb), 0.0001); }
    public static float3 toLinearSrgb(float3 c) { return Oklab.toLinearSrgb(sk_from_polar(c)); }
    public static int hueIndex() { return 2; }
    public static bool achromatic(float3 c) { return c.y <= 0.0001; }
};

/// Coordinates in `From` converted to `To`, through linear sRGB.
public float3 convert<From : IColorSpace, To : IColorSpace>(float3 c)
{
    return To.fromLinearSrgb(From.toLinearSrgb(c));
}

/// How `Color.mix` interpolates a hue, as in CSS `color-mix(in oklch shorter hue, ...)`.
public enum HueInterpolation
{
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// CSS Color 4 hue interpolation, as closed forms of the hue difference rather than its case
/// split, so no comparison can see a hue near 0/360 rounded differently from the arithmetic.
float sk_mix_hue(float from, float to, float t, HueInterpolation method)
{
    float d = to - from;
    // Half turns stay as CSS leaves them (+180 and -180), the same on every backend's `round`.
    float shorter = d - 360.0 * sign(d) * ceil(abs(d) / 360.0 - 0.5);
    float delta = shorter;
    if (method == HueInterpolation.Longer)
    {
        delta = shorter > 0.0 ? shorter - 360.0 : shorter + 360.0;
    }
    else if (method == HueInterpolation.Increasing)
    {
        delta = d - 360.0 * floor(d / 360.0);
    }
    else if (method == HueInterpolation.Decreasing)
    {
        delta = d - 360.0 * ceil(d / 360.0);
    }
    return sk_wrap_hue(from + delta * t);
}

/// A value nudged by `delta` without being pushed outside 0..1, nor pulled
/// back into it when it already lies outside (an HDR lightness stays).
float sk_nudge(float value, float delta)
{
    return clamp(value + delta, min(value, 0.0), max(value, 1.0));
}

// ===========================================================================
//  Color
// ===========================================================================

/// A colour: linear-light sRGB with straight (not premultiplied) alpha.
/// Build one with a `from...` constructor named after what you hold.
public struct Color
{
    /// Linear sRGB (D65). 1.0 is SDR white; values above it are HDR.
    public float3 linear;
    /// Straight alpha, 0..1.
    public float alpha;

    /// From straight-alpha sRGB-encoded RGBA: an `[SkColor]` field, a colour
    /// written in the shader, or what a `straightAlpha` effect returns.
    public static Color fromSrgb(float4 rgba)
    {
        Color c;
        c.linear = Srgb.decode(rgba.rgb);
        c.alpha = rgba.a;
        return c;
    }

    /// From sRGB-encoded rgb, opaque.
    public static Color fromSrgb(float3 rgb)
    {
        return Color.fromSrgb(float4(rgb, 1.0));
    }

    /// From premultiplied sRGB-encoded RGBA, what the engine samples:
    /// `input.backdrop`, `backdropAt`, `content`, `contentAt`, an `SkTexture`.
    /// Fully transparent reads as transparent black.
    public static Color fromPremultiplied(float4 rgba)
    {
        float3 straight = rgba.a > 0.0 ? rgba.rgb / (rgba.a > 0.0 ? rgba.a : 1.0) : float3(0.0, 0.0, 0.0);
        return Color.fromSrgb(float4(straight, rgba.a));
    }

    /// From straight-alpha linear-light sRGB.
    public static Color fromLinear(float4 rgba)
    {
        Color c;
        c.linear = rgba.rgb;
        c.alpha = rgba.a;
        return c;
    }

    /// From linear-light sRGB, opaque.
    public static Color fromLinear(float3 rgb)
    {
        return Color.fromLinear(float4(rgb, 1.0));
    }

    /// From a CSS hex colour written as a number: `Color.fromHex(0x7fd4ff)`.
    public static Color fromHex(uint rrggbb, float alpha = 1.0)
    {
        float3 rgb = float3(float((rrggbb >> 16) & 0xffu), float((rrggbb >> 8) & 0xffu), float(rrggbb & 0xffu));
        return Color.fromSrgb(float4(rgb / 255.0, alpha));
    }

    /// From coordinates in a colour space: `Color.from<Oklch>(float3(0.7, 0.1, 200.0))`.
    public static Color from<S : IColorSpace>(float3 coords, float alpha = 1.0)
    {
        Color c;
        c.linear = S.toLinearSrgb(coords);
        c.alpha = alpha;
        return c;
    }

    /// Straight-alpha sRGB-encoded RGBA: return it from a `straightAlpha` effect.
    public float4 toSrgb()
    {
        return float4(Srgb.encode(this.linear), this.alpha);
    }

    /// Premultiplied sRGB-encoded RGBA: return it from `shade`.
    public float4 toPremultiplied()
    {
        return float4(Srgb.encode(this.linear) * this.alpha, this.alpha);
    }

    /// Straight-alpha linear-light RGBA.
    public float4 toLinear()
    {
        return float4(this.linear, this.alpha);
    }

    /// This colour's coordinates in a colour space: `c.to<Oklch>()`.
    public float3 to<S : IColorSpace>()
    {
        return S.fromLinearSrgb(this.linear);
    }

    /// The same colour with alpha `a`.
    public Color withAlpha(float a)
    {
        return Color.fromLinear(float4(this.linear, a));
    }

    /// The same colour with its HSL hue set to `degrees`.
    public Color withHue(float degrees)
    {
        float3 hsl = this.to<Hsl>();
        return Color.from<Hsl>(float3(degrees, hsl.y, hsl.z), this.alpha);
    }

    /// HSL lightness raised by `amount` (0..1), stopping at white like colord.
    public Color lighten(float amount)
    {
        float3 hsl = this.to<Hsl>();
        return Color.from<Hsl>(float3(hsl.x, hsl.y, sk_nudge(hsl.z, amount)), this.alpha);
    }

    /// HSL lightness lowered by `amount` (0..1), stopping at black.
    public Color darken(float amount)
    {
        return this.lighten(-amount);
    }

    /// HSL saturation raised by `amount` (0..1), stopping at 1. A grey stays grey.
    public Color saturate(float amount)
    {
        float3 hsl = this.to<Hsl>();
        return Color.from<Hsl>(float3(hsl.x, sk_nudge(hsl.y, amount), hsl.z), this.alpha);
    }

    /// HSL saturation lowered by `amount` (0..1), stopping at grey.
    public Color desaturate(float amount)
    {
        return this.saturate(-amount);
    }

    /// The grey of the same HSL lightness (colord's `grayscale`), not of the same luminance.
    public Color grayscale()
    {
        float l = this.to<Hsl>().z;
        return Color.fromSrgb(float4(l, l, l, this.alpha));
    }

    /// Each sRGB-encoded channel replaced by `1 - c`; an HDR channel goes negative.
    public Color invert()
    {
        return Color.fromSrgb(float4(1.0 - Srgb.encode(this.linear), this.alpha));
    }

    /// The HSL hue turned by `degrees`.
    public Color rotate(float degrees)
    {
        float3 hsl = this.to<Hsl>();
        return Color.from<Hsl>(float3(hsl.x + degrees, hsl.y, hsl.z), this.alpha);
    }

    /// CSS `color-mix(in S, this, other t)`: `t` 0 is this colour, 1 is `other`.
    /// Premultiplied interpolation; a powerless hue takes the other colour's.
    public Color mix<S : IColorSpace>(Color other, float t, HueInterpolation hue = HueInterpolation.Shorter)
    {
        float3 a = this.to<S>();
        float3 b = other.to<S>();
        int h = S.hueIndex();
        float3 keep = h == 0 ? float3(0.0, 1.0, 1.0) : (h == 2 ? float3(1.0, 1.0, 0.0) : float3(1.0, 1.0, 1.0));
        float alpha = lerp(this.alpha, other.alpha, t);
        float3 mixed = lerp(a * (keep * this.alpha + 1.0 - keep), b * (keep * other.alpha + 1.0 - keep), t);
        mixed = alpha > 0.0 ? mixed / (keep * alpha + 1.0 - keep) : mixed;

        bool greyA = S.achromatic(a);
        bool greyB = S.achromatic(b);
        float hueA = h == 0 ? a.x : a.z;
        float hueB = h == 0 ? b.x : b.z;
        float mixedHue = sk_mix_hue(greyA ? hueB : hueA, greyB ? hueA : hueB, t, hue);
        mixedHue = greyA && greyB ? 0.0 : mixedHue;
        float3 result = h == 0 ? float3(mixedHue, mixed.y, mixed.z) : (h == 2 ? float3(mixed.x, mixed.y, mixedHue) : mixed);
        return Color.from<S>(result, alpha);
    }

    /// `mix<Oklab>`: CSS `color-mix(in oklab, ...)`, the default interpolation space.
    public Color mix(Color other, float t)
    {
        return this.mix<Oklab>(other, t);
    }

    /// WCAG relative luminance: linear Y, 0 black, 1 SDR white, above for HDR. Ignores alpha.
    public float luminance()
    {
        return dot(this.linear, float3(0.2126, 0.7152, 0.0722));
    }

    /// WCAG contrast ratio with `other`, 1..21 for SDR colours. Ignores alpha:
    /// composite first. Negative luminance counts as black.
    public float contrast(Color other)
    {
        float a = max(this.luminance(), 0.0) + 0.05;
        float b = max(other.luminance(), 0.0) + 0.05;
        return max(a, b) / min(a, b);
    }

    /// Perceived brightness (YIQ luma of the sRGB-encoded channels), 0..1 for SDR.
    public float brightness()
    {
        return dot(Srgb.encode(this.linear), float3(0.299, 0.587, 0.114));
    }

    /// `brightness() < 0.5`, as colord.
    public bool isDark()
    {
        return this.brightness() < 0.5;
    }

    /// `brightness() >= 0.5`, as colord.
    public bool isLight()
    {
        return !this.isDark();
    }

    /// ΔE OK (CSS Color 4 `deltaEOK`): Euclidean distance in OKLab. About 0.02 is just noticeable.
    public float deltaEOK(Color other)
    {
        return distance(this.to<Oklab>(), other.to<Oklab>());
    }

    /// CIEDE2000 (CSS Color 4 `deltaE2000`) in D50 Lab. About 2 is just noticeable.
    public float deltaE2000(Color other)
    {
        float3 l1 = this.to<Lab>();
        float3 l2 = other.to<Lab>();
        static const float pow25_7 = 6103515625.0;
        float cBar = (length(l1.yz) + length(l2.yz)) * 0.5;
        float c7 = pow(max(cBar, 0.0), 7.0);
        float g = 0.5 * (1.0 - sqrt(c7 / (c7 + pow25_7)));
        float a1 = l1.y * (1.0 + g);
        float a2 = l2.y * (1.0 + g);
        float c1 = length(float2(a1, l1.z));
        float c2 = length(float2(a2, l2.z));
        float h1 = c1 > 0.0 ? degrees(atan2(l1.z, c1 > 0.0 ? a1 : 1.0)) : 0.0;
        float h2 = c2 > 0.0 ? degrees(atan2(l2.z, c2 > 0.0 ? a2 : 1.0)) : 0.0;
        float dl = l2.x - l1.x;
        float dc = c2 - c1;
        float cc = c1 * c2;
        // The hue difference as the signed angle between (a', b') vectors, and the mean hue
        // from it: the published case split on wrapped hues, without its seam at 0/360, where a
        // GPU may round one hue to ~0 for a comparison and ~360 for the arithmetic after it.
        float dh = cc == 0.0 ? 0.0
            : degrees(atan2(a1 * l2.z - l1.z * a2, cc == 0.0 ? 1.0 : a1 * a2 + l1.z * l2.z));
        float dH = 2.0 * sqrt(cc) * sin(radians(dh) * 0.5);
        float lBar = (l1.x + l2.x) * 0.5;
        float cpBar = (c1 + c2) * 0.5;
        float hBar = sk_wrap_hue(cc == 0.0 ? h1 + h2 : h1 + dh * 0.5);
        float t = 1.0 - 0.17 * cos(radians(hBar - 30.0)) + 0.24 * cos(radians(2.0 * hBar))
            + 0.32 * cos(radians(3.0 * hBar + 6.0)) - 0.20 * cos(radians(4.0 * hBar - 63.0));
        float l50 = (lBar - 50.0) * (lBar - 50.0);
        float sl = 1.0 + 0.015 * l50 / sqrt(20.0 + l50);
        float sc = 1.0 + 0.045 * cpBar;
        float sh = 1.0 + 0.015 * cpBar * t;
        float cp7 = pow(max(cpBar, 0.0), 7.0);
        float rc = 2.0 * sqrt(cp7 / (cp7 + pow25_7));
        float dTheta = 30.0 * exp(-((hBar - 275.0) / 25.0) * ((hBar - 275.0) / 25.0));
        float rt = -sin(radians(2.0 * dTheta)) * rc;
        float3 terms = float3(dl / sl, dc / sc, dH / sh);
        return sqrt(max(dot(terms, terms) + rt * terms.y * terms.z, 0.0));
    }

    /// This colour and its HSL opposite (colord harmony `complementary`: 0, 180).
    public Color[2] complementary()
    {
        Color r[2] = { this, this.rotate(180.0) };
        return r;
    }

    /// colord harmony `analogous`: hue -30, 0, +30.
    public Color[3] analogous()
    {
        Color r[3] = { this.rotate(-30.0), this, this.rotate(30.0) };
        return r;
    }

    /// colord harmony `triadic`: hue 0, 120, 240.
    public Color[3] triadic()
    {
        Color r[3] = { this, this.rotate(120.0), this.rotate(240.0) };
        return r;
    }

    /// colord harmony `tetradic`: hue 0, 90, 180, 270.
    public Color[4] tetradic()
    {
        Color r[4] = { this, this.rotate(90.0), this.rotate(180.0), this.rotate(270.0) };
        return r;
    }

    /// colord harmony `split-complementary`: hue 0, 150, 210.
    public Color[3] splitComplementary()
    {
        Color r[3] = { this, this.rotate(150.0), this.rotate(210.0) };
        return r;
    }
};
