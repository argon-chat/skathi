import { type Draft, section } from "./shared";

const since = "Unreleased" as const;

const normal = {
  name: "normal",
  type: "float3",
  description: "Unit surface normal in pixel space: x right, y down, z towards the viewer.",
};
const toLight = {
  name: "toLight",
  type: "float3",
  description: "Unit direction from the surface towards the light: `Light.towards`.",
};
const toViewer = {
  name: "toViewer",
  type: "float3",
  description: "Unit direction towards the viewer.",
  optional: true,
  default: "(0, 0, 1)",
};

const drafts: Draft[] = [
  {
    slug: "screen-gradient",
    name: "screenGradient",
    kind: "function",
    summary:
      "The gradient of a value per device pixel from the GPU's pixel-quad derivatives: of `input.sd` it points out of the element with length about 1.",
    since,
    syntax: `import sk.light;

float2 screenGradient(float f);   // float2(ddx(f), ddy(f))`,
    params: [{ name: "f", type: "float", description: "Any value computed per pixel: `input.sd`, a height, a noise." }],
    returns: "Units of `f` per pixel. Pass it as the `gradient` of `bevelNormal`; it need not be normalised.",
    notes: [
      "Derivatives come from the 2 × 2 pixel quad, so call it where every pixel of the quad runs it: not inside a branch or loop that differs between neighbours.",
      "`Sdf.gradient` from `sk.sdf` gives the same for a field of your own without that constraint, at four evaluations.",
    ],
    examples: [
      {
        title: "Light the element's edge from the top left",
        lang: "slang",
        code: `float2 away = normalize(screenGradient(input.sd) + 1e-6);
float lit = saturate(dot(away, float2(-0.7, -0.7))) * saturate(1.0 + input.sd / 4.0);
return float4(1.0, 1.0, 1.0, 1.0) * lit * input.coverage;`,
      },
    ],
    related: ["bevel-normal", "screen-normal"],
  },
  {
    slug: "screen-normal",
    name: "normalFromSlope",
    kind: "function",
    summary:
      "A unit normal from a height field's slope, or from a height known only per pixel through its screen-space derivatives.",
    since,
    syntax: `float3 normalFromSlope(float2 slope, float strength = 1.0);
float3 screenNormal(float height, float strength = 1.0);`,
    params: [
      {
        name: "slope",
        type: "float2",
        description: "(dh/dx, dh/dy): height units per pixel, `fbmDerivatives(...).yz` scaled to pixels.",
      },
      { name: "height", type: "float", description: "For `screenNormal`: the height at this pixel." },
      {
        name: "strength",
        type: "float",
        description: "Multiplies the slope: steeper relief.",
        optional: true,
        default: "1.0",
      },
    ],
    returns: "A unit normal, tilted down the slope; (0, 0, 1) on flat ground.",
    notes: ["`screenNormal` uses `screenGradient`, with its restriction to quad-uniform code."],
    examples: [
      {
        title: "Embossed noise",
        lang: "slang",
        code: `float3 h = fbmDerivatives<Simplex>(input.px / 50.0, 4.0);   // sk.noise
float3 n = normalFromSlope(h.yz / 50.0, 20.0);
float lit = lambert(n, normalize(float3(-1.0, -1.0, 1.0)));
return float4(lit, lit, lit, 1.0) * input.coverage;`,
      },
    ],
    aliases: ["screenNormal"],
    related: ["height-normal", "screen-gradient"],
  },
  {
    slug: "height-normal",
    name: "heightNormal",
    kind: "function",
    summary:
      "The unit normal of a height field you can evaluate anywhere, by central differences: for a relief computed in the shader.",
    since,
    syntax: `interface IHeightField { float height(float2 p); }

float3 heightNormal<H : IHeightField>(H field, float2 p, float step = 1.0, float strength = 1.0);`,
    params: [
      {
        name: "field",
        type: "H : IHeightField",
        description: "A struct with `height(p)`, in height units at a point in pixels.",
      },
      { name: "p", type: "float2", description: "The point, in pixels." },
      { name: "step", type: "float", description: "The difference step, in pixels.", optional: true, default: "1.0" },
      { name: "strength", type: "float", description: "Multiplies the slope.", optional: true, default: "1.0" },
    ],
    returns: "A unit normal.",
    notes: ["Cost: 4 calls of `height`. Safe in branches and loops, unlike `screenNormal`."],
    examples: [
      {
        title: "A dome lit from the side",
        lang: "slang",
        code: `import sk;
import sk.light;

struct Dome : IHeightField
{
    float2 center;
    float radius;
    float height(float2 p) { float d = length(p - center) / radius; return d < 1.0 ? sqrt(1.0 - d * d) * radius : 0.0; }
};

[SkEffect("dome")]
struct DomeEffect : IEffect
{
    float4 shade(EffectInput input)
    {
        Dome dome = { input.size * 0.5, min(input.size.x, input.size.y) * 0.4 };
        float3 n = heightNormal(dome, input.px);
        float lit = wrapLambert(n, normalize(float3(-1.0, -0.5, 1.0)), 0.3);
        return float4(lit, lit, lit, 1.0) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["IHeightField"],
    related: ["screen-normal"],
  },
  {
    slug: "unpack-normal",
    name: "unpackNormal",
    kind: "function",
    summary:
      "A unit normal from a normal-map texel (rgb 0..1 holding −1..1), y down as pixel space by default; `packNormal` is the inverse.",
    since,
    syntax: `float3 unpackNormal(float4 texel, bool yUp = false);
float4 packNormal(float3 n);`,
    params: [
      {
        name: "texel",
        type: "float4",
        description: "A read of a `[SkLinear]` texture: normal maps hold vectors, not colours.",
      },
      {
        name: "yUp",
        type: "bool",
        description: "For maps baked with y up (OpenGL convention).",
        optional: true,
        default: "false",
      },
    ],
    returns: "A unit normal, normalised after decoding so filtering and 8-bit steps do not shorten it.",
    examples: [
      {
        title: "Frosted glass from a normal map",
        lang: "slang",
        code: `import sk;
import sk.light;
import sk.sample;

[SkEffect("ribbed-glass")]
struct RibbedGlass : IEffect
{
    [SkLinear] SkTexture normals;
    [SkLength] [SkBackdropReach] [SkDefault("9")] float depth;

    float4 shade(EffectInput input)
    {
        float3 n = unpackNormal(Image(normals, true).at(input.px));
        return displace(Backdrop(input), input.px, n, depth - 1.0);
    }
};`,
      },
    ],
    aliases: ["packNormal"],
    related: ["displace", "sk-linear"],
  },
  {
    slug: "bevel-normal",
    name: "bevelNormal",
    kind: "function",
    summary:
      "The normal of a bevelled rim along any signed distance: round, squircle or flat chamfer, rising over `width` pixels from the edge.",
    since,
    syntax: `enum BevelProfile { Circle, Squircle, Linear }

float3 bevelNormal(float sd, float2 gradient, float width, BevelProfile profile = BevelProfile.Circle, float height = 1.0);
float bevelHeight(float sd, float width, BevelProfile profile = BevelProfile.Circle);`,
    params: [
      {
        name: "sd",
        type: "float",
        description: "Signed distance in pixels, negative inside: `input.sd` or a field from `sk.sdf`.",
      },
      {
        name: "gradient",
        type: "float2",
        description: "Its gradient, any length: `screenGradient(input.sd)` or `Sdf.gradient`.",
      },
      { name: "width", type: "float", description: "How far in from the edge the rim reaches, in pixels." },
      {
        name: "profile",
        type: "BevelProfile",
        description: "`Circle` (quarter round), `Squircle` (flatter top, sharper fall) or `Linear` (chamfer).",
        optional: true,
        default: "Circle",
      },
      {
        name: "height",
        type: "float",
        description: "The rise relative to the width: 0.5 is a lower rim.",
        optional: true,
        default: "1.0",
      },
    ],
    returns:
      "A unit normal: (0, 0, 1) on the flat top, tilting outwards towards the edge; the circle reaches horizontal on it. `bevelHeight` returns 0 at the edge to 1 at `width` in.",
    notes: [
      "Written against a plain distance and gradient, so it takes `input.sd`, any `sk.sdf` shape or a distance of your own alike.",
      "Never divides by the slope, so the vertical edge of the circle profile is finite.",
    ],
    examples: [
      {
        title: "A pillow-shaded button",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 14.0, BevelProfile.Squircle);
float lit = 0.6 + 0.4 * lambert(n, normalize(float3(-0.5, -0.8, 0.6)));
return float4(0.2, 0.45, 0.9, 1.0) * lit * input.coverage;`,
      },
    ],
    aliases: ["BevelProfile", "bevelHeight"],
    related: ["screen-gradient", "refraction-offset", "sdf"],
  },
  {
    slug: "fresnel-schlick",
    name: "fresnelSchlick",
    kind: "function",
    summary:
      "How much light a surface reflects at an angle: Schlick's approximation, the exact dielectric formula, and the normal-incidence reflectance from an index of refraction.",
    since,
    syntax: `float fresnelSchlick(float cosTheta, float f0);
float fresnelDielectric(float cosI, float eta);
float f0FromIor(float ior, float outside = 1.0);`,
    params: [
      {
        name: "cosTheta, cosI",
        type: "float",
        description:
          "The cosine between the normal and the view (or light) direction: `n.z` for the viewer straight ahead.",
      },
      { name: "f0", type: "float", description: "The reflectance head-on: 0.04 for glass, `f0FromIor(1.5)`." },
      { name: "eta", type: "float", description: "The ratio n2 / n1 of the indices: 1.5 from air into glass." },
      {
        name: "ior, outside",
        type: "float",
        description: "For `f0FromIor`: the material's index and the surrounding medium's.",
        optional: true,
        default: "outside 1.0",
      },
    ],
    returns:
      "`fresnelSchlick` in [f0, 1]; `fresnelDielectric` in [0, 1], 1 past the critical angle; `f0FromIor` is ((n1 − n2) / (n1 + n2))², 0.04 for 1.5.",
    notes: [
      "Schlick is within about 1 % of the exact formula for glass and cheaper: a few multiplies against a square root and two divisions.",
    ],
    examples: [
      {
        title: "A rim that brightens at grazing angles",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 12.0);
float f = fresnelSchlick(n.z, f0FromIor(1.5));
return float4(1.0, 1.0, 1.0, 1.0) * f * input.coverage;`,
      },
    ],
    aliases: ["fresnelDielectric", "f0FromIor"],
    related: ["rim", "surface"],
  },
  {
    slug: "refract-direction",
    name: "refractDirection",
    kind: "function",
    summary: "Snell's law: the direction a ray continues in after crossing a surface into another medium.",
    since,
    syntax: `float3 refractDirection(float3 incident, float3 normal, float eta);`,
    params: [
      { name: "incident", type: "float3", description: "Unit direction the ray travels in, towards the surface." },
      { name: "normal", type: "float3", description: "Unit normal facing the incident side." },
      { name: "eta", type: "float", description: "n1 / n2: 1 / 1.5 from air into glass." },
    ],
    returns: "The unit refracted direction, or (0, 0, 0) on total internal reflection.",
    notes: ["HLSL's `refract`, with the total internal reflection case defined the same on every backend."],
    examples: [
      {
        title: "How far a ray through 20 px of water moves sideways",
        lang: "slang",
        code: `float3 n = normalize(float3(0.3, 0.0, 1.0));
float3 t = refractDirection(float3(0.0, 0.0, -1.0), n, 1.0 / 1.33);
float2 shift = t.xy / -t.z * 20.0;
return RawBackdrop(input).at(input.px + shift);   // sk.sample`,
      },
    ],
    related: ["refraction-offset"],
  },
  {
    slug: "refraction-offset",
    name: "refractionOffset",
    kind: "function",
    summary:
      "Where to read what lies behind glass of this normal and thickness: the view ray refracted by Snell's law and carried to the back face, as a pixel offset.",
    since,
    syntax: `float2 refractionOffset(float3 normal, float thickness, float ior = 1.5);`,
    params: [
      normal,
      { name: "thickness", type: "float", description: "How deep the glass is, in pixels." },
      {
        name: "ior",
        type: "float",
        description: "Index of refraction: 1.5 glass, 1.33 water.",
        optional: true,
        default: "1.5",
      },
    ],
    returns:
      "A pixel offset to add to `input.px`: 0 on the flat top, at most `thickness × √(ior² − 1)` (1.12 × thickness for glass) where the normal lies flat.",
    notes: [
      "A convex rim reads from further in, as a real lens does; negate the offset for the outward pull of a meniscus.",
      "The one call between `bevelNormal` and the `sk.sample` kernels: `displace(src, px, refractionOffset(n, t))`, or `dispersion` for coloured fringes.",
      "Reach: give `[SkBackdropReach]` the largest offset plus the kernel's own reach.",
    ],
    examples: [
      {
        title: "A lens",
        lang: "slang",
        code: `import sk;
import sk.light;
import sk.sample;

[SkEffect("lens")]
struct Lens : IEffect
{
    [SkLength] [SkDefault("20")] float thickness;
    [SkLength] [SkBackdropReach] [SkDefault("24")] float reach;   // 1.12 x thickness + 1

    float4 shade(EffectInput input)
    {
        float3 n = bevelNormal(input.sd, screenGradient(input.sd), min(input.size.x, input.size.y) * 0.5);
        return displace(RawBackdrop(input), input.px, refractionOffset(n, thickness)) * input.coverage;
    }
};`,
      },
    ],
    related: ["refract-direction", "bevel-normal", "displace", "dispersion"],
  },
  {
    slug: "light",
    name: "Light",
    kind: "struct",
    summary: "A light: from a direction or at a point above the surface, with a linear-light colour and an intensity.",
    since,
    syntax: `struct Light
{
    float3 vector;   // directional: towards the light; point: its position (px, px, height px)
    bool isPoint;
    Color color;     // linear light (sk.color)
    float intensity;

    static Light directional(float3 towards, float intensity = 1.0);
    static Light directional(float3 towards, Color color, float intensity);
    static Light point(float3 position, float intensity = 1.0);
    static Light point(float3 position, Color color, float intensity);
    float3 towards(float3 at);   // unit direction from a surface point to the light
    float3 radiance();           // color.linear * intensity
}`,
    notes: [
      "Directions are normalised for you. A point light's direction changes over the element; a directional light's does not.",
    ],
    examples: [
      {
        title: "A warm point light",
        lang: "slang",
        code: `Light lamp = Light.point(float3(input.size * 0.3, 60.0), Color.fromHex(0xffd08a), 1.2);
float3 n = bevelNormal(input.sd, screenGradient(input.sd), 16.0);
float3 l = lamp.towards(float3(input.px, 0.0));
return Color.fromLinear(lamp.radiance() * lambert(n, l)).toPremultiplied() * input.coverage;`,
      },
    ],
    related: ["surface", "lambert", "ggx"],
  },
  {
    slug: "lambert",
    name: "lambert",
    kind: "function",
    summary:
      "Diffuse terms: `lambert` (max(n·l, 0)), `wrapLambert` that wraps past the terminator, and `lobe`, a cheap glint.",
    since,
    syntax: `float lambert(float3 normal, float3 toLight);
float wrapLambert(float3 normal, float3 toLight, float wrap);
float lobe(float3 normal, float3 toLight, float exponent);   // max(n . l, 0)^exponent`,
    params: [
      normal,
      toLight,
      {
        name: "wrap",
        type: "float",
        description: "0 is Lambert; 1 lights half the back. Soft light for a pseudo-3D rim.",
      },
      { name: "exponent", type: "float", description: "For `lobe`: the higher, the tighter the glint." },
    ],
    returns: "[0, 1].",
    examples: [
      {
        title: "A soft-lit bevel",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 10.0);
float lit = wrapLambert(n, normalize(float3(-0.4, -0.7, 0.6)), 0.5);
return float4(lit, lit, lit, 1.0) * input.coverage;`,
      },
    ],
    aliases: ["wrapLambert", "lobe"],
    related: ["blinn-phong", "light"],
  },
  {
    slug: "blinn-phong",
    name: "blinnPhong",
    kind: "function",
    summary: "Blinn-Phong specular: max(n·h, 0)^shininess, h halfway between the light and the viewer.",
    since,
    syntax: `float blinnPhong(float3 normal, float3 toLight, float shininess, float3 toViewer = SK_VIEW);`,
    params: [
      normal,
      toLight,
      { name: "shininess", type: "float", description: "The exponent: 20 broad, 200 a pin-point." },
      toViewer,
    ],
    returns: "[0, 1]. Not energy-normalised: raise the intensity with the shininess yourself.",
    examples: [
      {
        title: "A highlight on a glass rim",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 16.0);
float h = blinnPhong(n, normalize(float3(-0.5, -0.7, 0.6)), 80.0);
return addLight(float4(0.0, 0.0, 0.0, 0.0), h) * input.coverage;`,
      },
    ],
    related: ["ggx", "lambert"],
  },
  {
    slug: "ggx",
    name: "ggx",
    kind: "function",
    summary:
      "Physically based specular: Cook-Torrance with the GGX distribution, height-correlated Smith visibility and Schlick Fresnel, times n·l.",
    since,
    syntax: `float ggx(float3 normal, float3 toLight, float roughness, float f0, float3 toViewer = SK_VIEW);`,
    params: [
      normal,
      toLight,
      {
        name: "roughness",
        type: "float",
        description: "Perceptual, 0 (mirror) to 1; alpha = roughness², at least 0.002.",
      },
      { name: "f0", type: "float", description: "Reflectance head-on: `f0FromIor(1.5)` for glass." },
      toViewer,
    ],
    returns:
      "≥ 0: the light reflected towards the viewer per unit radiance. Peaks well above 1 for smooth surfaces; multiply by the light's radiance.",
    notes: [
      "Cost: about 25 arithmetic operations, one square root and one division.",
      "Computes 1 − (n·h)² as |n × h|², which keeps the highlight's peak exact in single precision at low roughness.",
    ],
    examples: [
      {
        title: "A glossy chip",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 20.0);
float s = ggx(n, normalize(float3(-0.4, -0.6, 0.7)), 0.25, 0.04);
return addLight(float4(0.05, 0.08, 0.14, 1.0), s * 0.5) * input.coverage;`,
      },
    ],
    related: ["blinn-phong", "surface", "fresnel-schlick"],
  },
  {
    slug: "rim",
    name: "rim",
    kind: "function",
    summary: "Rim light: (1 − n·v)^power, bright where the surface turns away from the viewer.",
    since,
    syntax: `float rim(float3 normal, float power, float3 toViewer = SK_VIEW);`,
    params: [
      normal,
      { name: "power", type: "float", description: "Higher keeps the light to the outermost edge." },
      toViewer,
    ],
    returns: "[0, 1].",
    examples: [
      {
        title: "A thin glowing edge",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 8.0);
return float4(0.5, 0.8, 1.0, 1.0) * rim(n, 3.0) * input.coverage;`,
      },
    ],
    related: ["fresnel-schlick"],
  },
  {
    slug: "anisotropic",
    name: "anisotropic",
    kind: "function",
    summary: "A Kajiya-Kay anisotropic highlight: a streak across the grooves of brushed metal or hair.",
    since,
    syntax: `float anisotropic(float3 normal, float3 toLight, float3 tangent, float exponent, float3 toViewer = SK_VIEW);`,
    params: [
      normal,
      toLight,
      { name: "tangent", type: "float3", description: "Unit direction along the grooves." },
      { name: "exponent", type: "float", description: "The streak's tightness." },
      toViewer,
    ],
    returns: "[0, 1]: sin(t, h)^exponent times n·l.",
    examples: [
      {
        title: "Brushed metal",
        lang: "slang",
        code: `float3 n = float3(0.0, 0.0, 1.0);
float3 l = normalize(float3(input.local.x - 0.5, -0.5, 0.6));
float s = anisotropic(n, l, float3(1.0, 0.0, 0.0), 40.0);
return float4(float3(0.55, 0.57, 0.6) + s * 0.4, 1.0) * input.coverage;`,
      },
    ],
    related: ["blinn-phong"],
  },
  {
    slug: "drop-shadow",
    name: "dropShadow",
    kind: "function",
    summary:
      "The opacity of a blurred shadow at a signed distance, matching CSS `box-shadow` (sigma = blur / 2), outside or inset.",
    since,
    syntax: `float dropShadow(float sd, float blur);
float innerShadow(float sd, float blur);`,
    params: [
      {
        name: "sd",
        type: "float",
        description:
          "Signed distance in pixels from the shape casting it, negative inside. Move or inflate it for offset and spread.",
      },
      {
        name: "blur",
        type: "float",
        description: "The CSS blur radius in pixels; the Gaussian's sigma is half of it.",
      },
    ],
    returns: "[0, 1], 0.5 on the edge. Multiply your shadow colour by it.",
    notes: [
      "Exact along straight edges, as CSS defines it: the Gaussian integrated across the edge (the error function). Rounded corners follow the distance, which is within a few per cent of a browser's.",
      "A drop shadow outside the element needs room: an `[SkBleed]` of `blur` plus the offset.",
    ],
    examples: [
      {
        title: "A shadow under a card",
        lang: "slang",
        code: `import sk;
import sk.light;

[SkEffect("card-shadow")]
struct CardShadow : IEffect
{
    [SkLength] [SkBleed] [SkDefault("24")] float blur;

    float4 shade(EffectInput input)
    {
        float shadow = dropShadow(input.sd - 2.0, blur) * 0.35;   // a 2 px spread
        float4 card = float4(1.0, 1.0, 1.0, 1.0) * input.coverage;
        return sk_over(float4(0.0, 0.0, 0.0, shadow), card);
    }
};`,
      },
    ],
    aliases: ["innerShadow"],
    related: ["edge-occlusion", "sk-bleed"],
  },
  {
    slug: "edge-occlusion",
    name: "edgeOcclusion",
    kind: "function",
    summary: "Ambient-occlusion-like darkening near an edge: 1 on it, fading to 0 about three radii in.",
    since,
    syntax: `float edgeOcclusion(float sd, float radius);`,
    params: [
      { name: "sd", type: "float", description: "Signed distance in pixels, negative inside." },
      { name: "radius", type: "float", description: "The falloff, in pixels." },
    ],
    returns: "[0, 1]: exp(sd / radius) inside, 1 outside.",
    examples: [
      {
        title: "A recessed well",
        lang: "slang",
        code: `float ao = edgeOcclusion(input.sd, 6.0);
return float4(float3(0.9, 0.9, 0.92) * (1.0 - 0.4 * ao), 1.0) * input.coverage;`,
      },
    ],
    related: ["drop-shadow"],
  },
  {
    slug: "surface",
    name: "Surface",
    kind: "struct",
    summary:
      "A surface's normal, thickness, index of refraction and roughness, with the refraction, Fresnel and GGX specular that follow from them.",
    since,
    syntax: `struct Surface
{
    float3 normal;
    float thickness;   // px
    float ior;
    float roughness;   // 0..1

    static Surface create(float3 normal, float thickness = 0.0, float ior = 1.5, float roughness = 0.2);
    float2 refraction();                                    // refractionOffset(normal, thickness, ior)
    float fresnel();                                        // Schlick, f0 from ior
    float3 specular(Light light, float3 at = float3(0.0)); // ggx(...) * light.radiance()
}`,
    notes: [
      "Saves passing the same four values to each call; there is no `shade` that does everything, because a single combinator made the showcase's glass longer, not shorter.",
    ],
    examples: [
      {
        title: "Glass with a coloured light",
        lang: "slang",
        code: `Surface glass = Surface.create(bevelNormal(input.sd, screenGradient(input.sd), 16.0), 20.0);
Light sun = Light.directional(float3(-0.5, -0.7, 0.6), Color.fromHex(0xfff2d6), 2.0);
Color seen = Color.fromPremultiplied(RawBackdrop(input).at(input.px + glass.refraction()));
return addLight(seen, sun.radiance() * glass.fresnel() + glass.specular(sun)).toPremultiplied() * input.coverage;`,
      },
    ],
    related: ["light", "refraction-offset", "ggx", "fresnel-schlick"],
  },
  {
    slug: "add-light",
    name: "addLight",
    kind: "function",
    summary:
      "Light added to a colour: in sRGB-encoded values on a premultiplied pixel (cheap, for highlights), or in linear light on a `Color`.",
    since,
    syntax: `float4 addLight(float4 premultiplied, float amount);
Color addLight(Color c, float3 light);`,
    params: [
      {
        name: "premultiplied",
        type: "float4",
        description: "A premultiplied, sRGB-encoded pixel: a kernel's result, `input.backdrop`.",
      },
      {
        name: "amount",
        type: "float",
        description: "White to add: `rgb + amount`, alpha raised by `amount × (1 − a)`.",
      },
      {
        name: "c, light",
        type: "Color, float3",
        description: "A colour and linear radiance to add to its premultiplied light.",
      },
    ],
    returns: "The brighter pixel or colour; alpha rises so a highlight shows over transparency too.",
    notes: [
      "The `float4` form adds in encoded values, not light: brighter in the shadows than physics says, which is what a glint on glass should look like anyway.",
    ],
    examples: [
      {
        title: "A glint on the backdrop",
        lang: "slang",
        code: `float3 n = bevelNormal(input.sd, screenGradient(input.sd), 12.0);
return addLight(RawBackdrop(input).at(input.px), 0.3 * lobe(n, normalize(float3(-0.4, -0.6, 0.7)), 12.0));`,
      },
    ],
    related: ["surface", "blinn-phong"],
  },
];

export const light = section("sk.light", drafts);
