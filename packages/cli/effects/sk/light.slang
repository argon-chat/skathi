// sk.light — surface normals, Fresnel, refraction, highlights and shadows for glass and material effects.
//
//     import sk;
//     import sk.light;
//
//     float3 n = bevelNormal(input.sd, screenGradient(input.sd), 24.0);   // a rounded glass rim
//     float4 seen = input.backdropAtPx(input.px + refractionOffset(n, 18.0, 1.5));
//     float glint = blinnPhong(n, normalize(float3(-0.5, -0.7, 0.6)), 60.0);
//     return addLight(seen, 0.4 * glint);
//
// Directions are in the element's pixel space: x to the right, y DOWN (as `input.px`), z towards the
// viewer, who looks along -z. So a light from the top left is `float3(-1.0, -1.0, 1.0)`, normalised.
// Every direction argument must be unit length unless its notes say otherwise. Lighting terms are
// linear-light factors; `addLight` on a premultiplied pixel is the one sRGB-encoded shortcut.

module "sk.light";

import sk.color;

static const float3 SK_VIEW = float3(0.0, 0.0, 1.0);

/// `v` normalised, or `fallback` when it is (nearly) zero: never a NaN.
float3 sk_normalize_or(float3 v, float3 fallback)
{
    float len2 = dot(v, v);
    return len2 > 1e-20 ? v * rsqrt(len2 > 1e-20 ? len2 : 1.0) : fallback;
}

float2 sk_normalize_or(float2 v, float2 fallback)
{
    float len2 = dot(v, v);
    return len2 > 1e-20 ? v * rsqrt(len2 > 1e-20 ? len2 : 1.0) : fallback;
}

/// `x^e` for x >= 0 without the built-in's log2(0): 0 at x = 0 for e > 0, 1 for e <= 0.
float sk_pow(float x, float e)
{
    return x > 0.0 ? pow(x > 0.0 ? x : 1.0, e) : (e > 0.0 ? 0.0 : 1.0);
}

// ===========================================================================
//  Normals
// ===========================================================================

/// The screen-space gradient of `f` per device pixel, `float2(ddx(f), ddy(f))`: of `input.sd` it
/// points outwards with length about 1. From the 2x2 pixel quad, so call it outside branches and loops.
public float2 screenGradient(float f)
{
    return float2(ddx(f), ddy(f));
}

/// A unit normal from a height field's slope (dh/dx, dh/dy) in height units per pixel, scaled
/// by `strength`: the surface tilts down the slope.
public float3 normalFromSlope(float2 slope, float strength = 1.0)
{
    return normalize(float3(-slope * strength, 1.0));
}

/// A unit normal from a height known only per pixel, through its screen-space derivatives.
/// Like `screenGradient`, keep it out of branches and loops.
public float3 screenNormal(float height, float strength = 1.0)
{
    return normalFromSlope(screenGradient(height), strength);
}

/// A height function of a point in pixels, for `heightNormal`: a struct with `height`.
public interface IHeightField
{
    float height(float2 p);
};

/// A unit normal of a height field at p by central differences `step` pixels apart (4 calls).
public float3 heightNormal<H : IHeightField>(H field, float2 p, float step = 1.0, float strength = 1.0)
{
    float2 dx = float2(step, 0.0);
    float2 dy = float2(0.0, step);
    float2 slope = float2(field.height(p + dx) - field.height(p - dx), field.height(p + dy) - field.height(p - dy));
    return normalFromSlope(slope / (2.0 * step), strength);
}

/// A tangent-space normal from a `[SkLinear]` normal-map texel (rgb 0..1 holding -1..1), y down
/// as in pixel space; `yUp` for maps baked y-up (OpenGL convention). Normalised.
public float3 unpackNormal(float4 texel, bool yUp = false)
{
    float3 n = texel.xyz * 2.0 - 1.0;
    return sk_normalize_or(float3(n.x, yUp ? -n.y : n.y, n.z), SK_VIEW);
}

/// A unit normal as an rgb texel, 0..1: the inverse of `unpackNormal`.
public float4 packNormal(float3 n)
{
    return float4(n * 0.5 + 0.5, 1.0);
}

/// The cross-section of a bevel, from the edge inwards.
public enum BevelProfile
{
    /// A quarter circle: vertical at the edge, flat `width` in. The usual glass lens rim.
    Circle,
    /// A superellipse of exponent 4 (squircle): flatter on top, steeper at the edge.
    Squircle,
    /// A straight 45-degree chamfer (with `height` 1), flat beyond it.
    Linear,
}

/// The bevel's height, 0 at the edge to 1 at `width` pixels in and beyond; 0 outside the shape.
/// `sd` is a signed distance in pixels, negative inside (`input.sd`).
public float bevelHeight(float sd, float width, BevelProfile profile = BevelProfile.Circle)
{
    float s = saturate(-sd / max(width, 1e-4));
    float u = 1.0 - s;
    float u4 = u * u * u * u;
    float circle = sqrt(saturate(1.0 - u * u));
    float squircle = sqrt(sqrt(saturate(1.0 - u4)));
    return profile == BevelProfile.Circle ? circle : (profile == BevelProfile.Squircle ? squircle : s);
}

/// The unit normal of a bevelled rim: `sd` a signed distance in pixels (negative inside), `gradient`
/// its gradient (any length; `screenGradient(input.sd)`, `Sdf.gradient`), `width` the rim's
/// width in pixels and `height` its rise relative to the width. (0, 0, 1) on the flat top.
public float3 bevelNormal(float sd, float2 gradient, float width, BevelProfile profile = BevelProfile.Circle,
    float height = 1.0)
{
    float s = saturate(-sd / max(width, 1e-4));
    float u = 1.0 - s;
    float u3 = u * u * u;
    float2 outward = sk_normalize_or(gradient, float2(0.0, 0.0));
    // The normal is (outward * slope, 1); written as (outward * a, b) with slope = a / b, so the
    // vertical edge of a circle (b = 0) needs no division.
    float a = profile == BevelProfile.Circle ? u : (profile == BevelProfile.Squircle ? u3 : (s < 1.0 ? 1.0 : 0.0));
    float b = profile == BevelProfile.Circle ? sqrt(saturate(1.0 - u * u))
        : (profile == BevelProfile.Squircle ? sk_pow(saturate(1.0 - u3 * u), 0.75) : 1.0);
    return sk_normalize_or(float3(outward * a * height, b), SK_VIEW);
}

// ===========================================================================
//  Fresnel and refraction
// ===========================================================================

/// The reflectance at normal incidence between two media: ((n1 - n2) / (n1 + n2))^2.
/// 0.04 for glass (1.5) in air.
public float f0FromIor(float ior, float outside = 1.0)
{
    float r = (outside - ior) / max(outside + ior, 1e-6);
    return r * r;
}

/// Schlick's Fresnel approximation: the reflected fraction at an angle whose cosine is `cosTheta`
/// (clamped to 0..1). Range [f0, 1].
public float fresnelSchlick(float cosTheta, float f0)
{
    float m = 1.0 - saturate(cosTheta);
    float m2 = m * m;
    return f0 + (1.0 - f0) * m2 * m2 * m;
}

/// The exact Fresnel reflectance of unpolarised light from the outside medium into one `eta` times
/// as refractive (n2 / n1), at incidence cosine `cosI`. 1 past the critical angle. Range [0, 1].
public float fresnelDielectric(float cosI, float eta)
{
    float c = saturate(cosI);
    float sin2t = (1.0 - c * c) / max(eta * eta, 1e-12);
    float cosT = sqrt(max(1.0 - sin2t, 0.0));
    float rs = (c - eta * cosT) / max(c + eta * cosT, 1e-12);
    float rp = (eta * c - cosT) / max(eta * c + cosT, 1e-12);
    return sin2t >= 1.0 ? 1.0 : 0.5 * (rs * rs + rp * rp);
}

/// Snell's law: the unit direction `incident` (travelling towards the surface) continues in after
/// crossing `normal` (unit, facing the incident side), `eta` = n1 / n2. Zero on total internal reflection.
public float3 refractDirection(float3 incident, float3 normal, float eta)
{
    float cosI = -dot(incident, normal);
    float k = 1.0 - eta * eta * (1.0 - cosI * cosI);
    float3 t = eta * incident + (eta * cosI - sqrt(max(k, 0.0))) * normal;
    return k < 0.0 ? float3(0.0, 0.0, 0.0) : t;
}

/// Where to read what lies behind glass of this normal and `thickness` pixels with index `ior`, as a
/// pixel offset from the point: the view ray refracted by Snell's law, carried to the back face.
/// A convex rim shows what lies further in, as a real lens does; negate it for the outward pull.
public float2 refractionOffset(float3 normal, float thickness, float ior = 1.5)
{
    float3 t = refractDirection(float3(0.0, 0.0, -1.0), normal, 1.0 / max(ior, 1e-4));
    return t.xy / max(-t.z, 0.05) * thickness;
}

// ===========================================================================
//  Lights
// ===========================================================================

/// A light: directional (from a direction) or point (at a position above the surface), with a
/// linear-light colour times intensity.
public struct Light
{
    /// Directional: the unit direction towards the light. Point: the light's position (x, y in
    /// pixels, z its height above the surface in pixels).
    public float3 vector;
    /// Whether `vector` is a position.
    public bool isPoint;
    /// The colour, linear light.
    public Color color;
    /// Multiplies `color`; 1 is a light of that colour at full strength.
    public float intensity;

    /// A light from a direction (normalised here): `Light.directional(float3(-1.0, -1.0, 1.5))`.
    public static Light directional(float3 towards, float intensity = 1.0)
    {
        return Light.directional(towards, Color.fromLinear(float3(1.0, 1.0, 1.0)), intensity);
    }

    /// A coloured light from a direction.
    public static Light directional(float3 towards, Color color, float intensity)
    {
        Light l;
        l.vector = sk_normalize_or(towards, SK_VIEW);
        l.isPoint = false;
        l.color = color;
        l.intensity = intensity;
        return l;
    }

    /// A white light at a position: (x, y) in the element's pixels, z the height above it.
    public static Light point(float3 position, float intensity = 1.0)
    {
        return Light.point(position, Color.fromLinear(float3(1.0, 1.0, 1.0)), intensity);
    }

    /// A coloured light at a position.
    public static Light point(float3 position, Color color, float intensity)
    {
        Light l;
        l.vector = position;
        l.isPoint = true;
        l.color = color;
        l.intensity = intensity;
        return l;
    }

    /// The unit direction from a surface point (x, y pixels, z height) to the light.
    public float3 towards(float3 at)
    {
        return this.isPoint ? sk_normalize_or(this.vector - at, SK_VIEW) : this.vector;
    }

    /// The light's radiance, linear: colour times intensity.
    public float3 radiance()
    {
        return this.color.linear * this.intensity;
    }
};

// ===========================================================================
//  Shading terms
// ===========================================================================

/// Lambert's diffuse term: max(n . l, 0). Range [0, 1].
public float lambert(float3 normal, float3 toLight)
{
    return saturate(dot(normal, toLight));
}

/// Diffuse light that wraps past the terminator by `wrap` (0 is Lambert, 1 lights half the back):
/// a soft light for a pseudo-3D bevel. Range [0, 1].
public float wrapLambert(float3 normal, float3 toLight, float wrap)
{
    return saturate((dot(normal, toLight) + wrap) / (1.0 + wrap));
}

/// A glint where the normal faces the light: max(n . l, 0)^exponent. Range [0, 1].
public float lobe(float3 normal, float3 toLight, float exponent)
{
    return sk_pow(saturate(dot(normal, toLight)), exponent);
}

/// Blinn-Phong specular: max(n . h, 0)^shininess, h halfway between light and viewer. Range [0, 1];
/// not energy-normalised (raise the intensity with the shininess yourself).
public float blinnPhong(float3 normal, float3 toLight, float shininess, float3 toViewer = SK_VIEW)
{
    float3 h = sk_normalize_or(toLight + toViewer, normal);
    return sk_pow(saturate(dot(normal, h)), shininess);
}

/// Cook-Torrance specular with the GGX distribution, Smith height-correlated visibility and
/// Schlick Fresnel, times n . l: the light reflected towards the viewer per unit radiance.
/// `roughness` 0..1 (perceptual; alpha = roughness^2, at least 0.002). >= 0, peaks above 1 when smooth.
public float ggx(float3 normal, float3 toLight, float roughness, float f0, float3 toViewer = SK_VIEW)
{
    float3 h = sk_normalize_or(toLight + toViewer, normal);
    float nl = saturate(dot(normal, toLight));
    float nv = max(dot(normal, toViewer), 1e-4);
    float nh = saturate(dot(normal, h));
    float a = max(roughness * roughness, 0.002);
    float a2 = a * a;
    // 1 - (n.h)^2 as |n x h|^2: the subtraction loses every digit of a smooth surface's peak in float32.
    float3 nxh = cross(normal, h);
    float d = dot(nxh, nxh) + nh * nh * a2;
    float distribution = a2 / (3.14159265358979 * d * d);
    float gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    float gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    float visibility = 0.5 / max(gv + gl, 1e-6);
    return distribution * visibility * fresnelSchlick(dot(toLight, h), f0) * nl;
}

/// Rim light: (1 - n . v)^power, bright where the surface turns away from the viewer. Range [0, 1].
public float rim(float3 normal, float power, float3 toViewer = SK_VIEW)
{
    return sk_pow(1.0 - saturate(dot(normal, toViewer)), power);
}

/// Kajiya-Kay anisotropic highlight: a streak across `tangent` (unit, along the grooves of brushed
/// metal or hair), sin(t, h)^exponent times n . l. Range [0, 1].
public float anisotropic(float3 normal, float3 toLight, float3 tangent, float exponent, float3 toViewer = SK_VIEW)
{
    float3 h = sk_normalize_or(toLight + toViewer, normal);
    float th = dot(tangent, h);
    return sk_pow(sqrt(saturate(1.0 - th * th)), exponent) * saturate(dot(normal, toLight));
}

// ===========================================================================
//  Shadows
// ===========================================================================

/// erf by Abramowitz and Stegun 7.1.26, |error| < 1.5e-7.
float sk_erf(float x)
{
    // erf(4) is 1 in float32; past it fxc, folding a constant argument, warns that 1 - exp(-a^2) is inexact.
    float a = min(abs(x), 4.0);
    float t = 1.0 / (1.0 + 0.3275911 * a);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t
        * exp(-a * a);
    return x < 0.0 ? -y : y;
}

/// The standard normal CDF of d / sigma; a hard antialiased step when sigma is 0.
float sk_gaussian_step(float d, float sigma)
{
    return sigma > 0.0 ? 0.5 + 0.5 * sk_erf(d / (sigma > 0.0 ? sigma : 1.0) * 0.70710678118654752) : saturate(0.5 + d);
}

/// A drop shadow's opacity at signed distance `sd` (pixels, negative inside) from the shape that
/// casts it, blurred like CSS `box-shadow`: sigma = blur / 2. 0.5 on the edge. Range [0, 1].
/// Exact along straight edges; offset or spread the shape by moving or inflating `sd`.
public float dropShadow(float sd, float blur)
{
    return sk_gaussian_step(-sd, 0.5 * blur);
}

/// An inset shadow's opacity at signed distance `sd` inside a shape: what `box-shadow: inset` with
/// this blur casts from the edge, 0.5 on it, fading in. Range [0, 1]; multiply by coverage.
public float innerShadow(float sd, float blur)
{
    return sk_gaussian_step(sd, 0.5 * blur);
}

/// Ambient occlusion near an edge: 1 on it, falling to 0 by about 3 x `radius` inside, as light
/// is shut out of a corner. exp(sd / radius) for sd <= 0. Range [0, 1].
public float edgeOcclusion(float sd, float radius)
{
    return exp(min(sd, 0.0) / max(radius, 1e-4));
}

// ===========================================================================
//  Surfaces
// ===========================================================================

/// A surface to shade: what `refraction`, `fresnel` and `specular` need.
public struct Surface
{
    /// Unit normal, pixel space (z towards the viewer).
    public float3 normal;
    /// How thick the material is, in pixels: how far refraction carries the view ray.
    public float thickness;
    /// Index of refraction: 1.5 glass, 1.33 water.
    public float ior;
    /// Perceptual roughness, 0 (mirror) .. 1.
    public float roughness;

    /// A surface with these properties.
    public static Surface create(float3 normal, float thickness = 0.0, float ior = 1.5, float roughness = 0.2)
    {
        Surface s;
        s.normal = normal;
        s.thickness = thickness;
        s.ior = ior;
        s.roughness = roughness;
        return s;
    }

    /// `refractionOffset(normal, thickness, ior)`: add it to `input.px` to read what is behind.
    public float2 refraction()
    {
        return refractionOffset(this.normal, this.thickness, this.ior);
    }

    /// The Fresnel reflectance towards the viewer, Schlick with f0 from the ior. Range [f0, 1].
    public float fresnel()
    {
        return fresnelSchlick(this.normal.z, f0FromIor(this.ior));
    }

    /// The specular light reflected to the viewer from `light` (GGX), linear radiance, for a
    /// surface point at `at` (x, y pixels, z height; only a point light uses it).
    public float3 specular(Light light, float3 at = float3(0.0, 0.0, 0.0))
    {
        return light.radiance() * ggx(this.normal, light.towards(at), this.roughness, f0FromIor(this.ior));
    }
};

// ===========================================================================
//  Adding light to a pixel
// ===========================================================================

/// `amount` of white light added to a premultiplied, sRGB-encoded pixel, raising its alpha as
/// much: `rgb + amount`, `a + amount (1 - a)`. Adds in encoded values, not light: the cheap form
/// effects use for highlights, where the exact curve does not show.
public float4 addLight(float4 premultiplied, float amount)
{
    return float4(premultiplied.rgb + amount, saturate(premultiplied.a + amount * (1.0 - premultiplied.a)));
}

/// Linear `light` added to a colour, in light: the colour's premultiplied radiance plus `light`,
/// its alpha raised by the brightest channel of it.
public Color addLight(Color c, float3 light)
{
    float a = saturate(c.alpha + saturate(max(max(light.r, light.g), light.b)) * (1.0 - c.alpha));
    float3 premultiplied = c.linear * c.alpha + light;
    return Color.fromLinear(float4(a > 0.0 ? premultiplied / (a > 0.0 ? a : 1.0) : float3(0.0, 0.0, 0.0), a));
}
