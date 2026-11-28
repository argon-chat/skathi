import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const space = (
  slug: string,
  name: string,
  summary: string,
  coords: string,
  notes: string[],
  example?: string,
): Draft => ({
  slug,
  name,
  summary,
  since,
  usage: `float3 c = color.to<${name}>();       // ${coords}
Color back = Color.from<${name}>(c, alpha);`,
  notes,
  ...(example ? { examples: [{ title: `Work in ${name}`, lang: "slang" as const, code: example }] } : {}),
  related: ["i-color-space", "color-to", "color-from"],
});

export const color = section("sk.color", [
  {
    slug: "color",
    name: "Color",
    since,
    notes: [
      "The encoding is in the type: a `Color` is always linear light and straight alpha, whatever it was built from. Build one with `fromSrgb`, `fromPremultiplied`, `fromLinear`, `fromHex` or `from<S>`, and turn it back with the matching `toSrgb`, `toPremultiplied`, `toLinear` or `to<S>`.",
      "`linear` is not clamped. Values above 1.0 (an HDR backdrop) and below 0 (outside the sRGB gamut) survive every conversion that can carry them: linear sRGB, XYZ, Lab, LCH, OKLab and OKLCH. The engine clamps what `shade` returns.",
      "Every method returns a new `Color`; none changes the one it is called on.",
      "Only what an effect calls reaches its compiled shader: an unused method costs nothing.",
    ],
    examples: [
      {
        title: "Tint the backdrop and keep text readable",
        lang: "slang",
        code: `import sk;
import sk.color;

[SkEffect("tinted-glass")]
struct TintedGlass : IEffect
{
    [SkColor] [SkDefault("#3d7fff")] float4 tint;
    [SkLength] [SkBackdropRadius] [SkDefault("12")] float blur;

    float4 shade(EffectInput input)
    {
        Color back = Color.fromPremultiplied(input.backdrop);
        Color glass = back.mix<Oklch>(Color.fromSrgb(tint), 0.35);
        // White text sits on this glass: darken until it reads at 4.5:1.
        Color white = Color.fromHex(0xffffff);
        for (int i = 0; i < 4 && glass.contrast(white) < 4.5; i++)
            glass = glass.darken(0.08);
        return glass.withAlpha(1.0).toPremultiplied();
    }
};`,
      },
    ],
    related: ["color-from-srgb", "color-from-premultiplied", "color-to-premultiplied", "color-mix"],
  },
  {
    slug: "color-from-srgb",
    name: "Color.fromSrgb",
    since,
    notes: [
      "This is the encoding of an `[SkColor]` parameter, of a `#hex` or `rgb()` written as numbers, and of what a `straightAlpha` effect returns.",
      "Channels outside 0..1 are decoded with the sRGB curve mirrored below 0 and extended above 1, as scRGB does.",
    ],
    related: ["sk-color", "color-to-srgb", "color-from-premultiplied"],
  },
  {
    slug: "color-from-premultiplied",
    name: "Color.fromPremultiplied",
    since,
    notes: [
      "Use it for `input.backdrop`, `backdropAt`, `rawBackdropAt`, `input.content`, `contentAt` and an `SkTexture` without `[SkLinear]`.",
      "The engine premultiplies in sRGB-encoded space, so this divides by alpha before decoding. A fully transparent sample reads as transparent black.",
      "In an HDR window a sample can exceed 1.0; the value is kept.",
    ],
    related: ["effect-input-backdrop-at", "color-to-premultiplied"],
  },
  {
    slug: "color-from-linear",
    name: "Color.fromLinear",
    summary:
      "Build a colour from straight-alpha, linear-light sRGB, such as values computed in light or sampled from a `[SkLinear]` texture.",
    since,
    related: ["color-to-linear"],
  },
  {
    slug: "color-from-hex",
    name: "Color.fromHex",
    since,
    params: [
      { name: "rrggbb", type: "uint", description: "Red, green and blue bytes, sRGB-encoded." },
      { name: "alpha", type: "float", description: "Straight alpha.", optional: true, default: "1.0" },
    ],
  },
  {
    slug: "color-from",
    name: "Color.from",
    since,
    params: [
      {
        name: "S",
        type: "IColorSpace",
        description:
          "The space: `Srgb`, `LinearSrgb`, `Hsl`, `Hsv`, `Hwb`, `XyzD65`, `XyzD50`, `Lab`, `Lch`, `Oklab` or `Oklch`.",
      },
      { name: "coords", type: "float3", description: "Coordinates in the space's own units; hues in degrees." },
      { name: "alpha", type: "float", description: "Straight alpha.", optional: true, default: "1.0" },
    ],
    related: ["color-to", "i-color-space"],
  },
  {
    slug: "color-to-srgb",
    name: "Color.toSrgb",
    since,
    related: ["color-from-srgb", "i-effect"],
  },
  {
    slug: "color-to-premultiplied",
    name: "Color.toPremultiplied",
    since,
    notes: ["Values above SDR white stay in the result; the engine's clamp after `shade` limits them, not this."],
    related: ["color-from-premultiplied", "i-effect"],
  },
  {
    slug: "color-to-linear",
    name: "Color.toLinear",
    summary: "Return straight-alpha, linear-light RGBA. Channels above 1 retain their HDR values.",
    since,
  },
  {
    slug: "color-to",
    name: "Color.to",
    since,
    notes: ["Alpha is not part of the coordinates; read `c.alpha`."],
    related: ["color-from", "i-color-space"],
  },
  {
    slug: "color-lighten",
    name: "Color.lighten",
    summary: "HSL lightness raised by `amount` (0..1); `darken` lowers it. As colord, stopping at white and black.",
    since,
    notes: [
      "Works on the sRGB-encoded HSL lightness, so `lighten(0.1)` is what colord, Sass and CSS preprocessors do.",
      "The result stops at 0 and 1, but a lightness already outside them (an HDR colour) is not pulled back in.",
      "For a perceptually even step, change L in OKLCH instead: `Color.from<Oklch>(c.to<Oklch>() + float3(0.05, 0.0, 0.0), c.alpha)`.",
    ],
    aliases: ["Color.darken"],
  },
  {
    slug: "color-saturate",
    name: "Color.saturate",
    summary: "HSL saturation raised by `amount` (0..1); `desaturate` lowers it. A grey stays grey.",
    since,
    aliases: ["Color.desaturate"],
  },
  {
    slug: "color-grayscale",
    name: "Color.grayscale",
    since,
    notes: [
      "This keeps HSL lightness, not luminance: a saturated blue turns a lighter grey than its luminance. For a luminance grey use `Color.fromLinear(float3(c.luminance()))`.",
    ],
  },
  {
    slug: "color-invert",
    name: "Color.invert",
    since,
    notes: ["An SDR operation: a channel above 1.0 turns negative."],
  },
  {
    slug: "color-rotate",
    name: "Color.rotate",
    summary: "The HSL hue turned by `degrees`; `withHue` sets it.",
    since,
    aliases: ["Color.withHue"],
  },
  {
    slug: "color-with-alpha",
    name: "Color.withAlpha",
    since,
  },
  {
    slug: "color-mix",
    name: "Color.mix",
    since,
    params: [
      { name: "S", type: "IColorSpace", description: "The interpolation space. Without it, `Oklab`, the CSS default." },
      { name: "other", type: "Color", description: "The colour at `t = 1`." },
      { name: "t", type: "float", description: "0 is this colour, 1 is `other`; outside 0..1 extrapolates." },
      {
        name: "hue",
        type: "HueInterpolation",
        description: "For a polar space: which way round the hue goes.",
        optional: true,
        default: "Shorter",
      },
    ],
    notes: [
      "As CSS Color 4: components are premultiplied by alpha before interpolating and divided by the mixed alpha after; in a polar space the hue is not premultiplied.",
      "A powerless hue, that of a grey, takes the other colour's, so mixing white into red in OKLCH stays red, not a detour through hue 0.",
    ],
    examples: [
      {
        title: "A gradient that turns the hue the long way",
        lang: "slang",
        code: `Color start = Color.fromSrgb(a);
Color end = Color.fromSrgb(b);
Color mixed = start.mix<Oklch>(end, input.local.x, HueInterpolation.Longer);
return mixed.toPremultiplied();`,
      },
    ],
    related: ["hue-interpolation", "i-color-space"],
  },
  {
    slug: "hue-interpolation",
    name: "HueInterpolation",
    since,
    notes: ["Half turns stay as CSS leaves them: `Shorter` from 0° to 180° goes up, from 180° to 0° goes down."],
    related: ["color-mix"],
  },
  {
    slug: "color-luminance",
    name: "Color.luminance",
    since,
    related: ["color-contrast"],
  },
  {
    slug: "color-contrast",
    name: "Color.contrast",
    since,
    notes: [
      "WCAG 2 asks for 4.5 for body text and 3 for large text.",
      "Alpha is ignored: composite a translucent colour over what is behind it first.",
      "Negative luminance, a colour outside the sRGB gamut, counts as black.",
    ],
    examples: [
      {
        title: "Pick black or white text for a background",
        lang: "slang",
        code: `Color bg = Color.fromPremultiplied(input.backdrop);
Color ink = bg.contrast(Color.fromHex(0x000000)) >= bg.contrast(Color.fromHex(0xffffff))
    ? Color.fromHex(0x000000)
    : Color.fromHex(0xffffff);`,
      },
    ],
    related: ["color-luminance"],
  },
  {
    slug: "color-brightness",
    name: "Color.brightness",
    summary: "Perceived brightness, YIQ luma of the sRGB-encoded channels, 0..1 for SDR; `isDark` is below 0.5.",
    since,
    aliases: ["Color.isDark", "Color.isLight"],
  },
  {
    slug: "color-delta-e",
    name: "Color.deltaEOK",
    summary: "Colour differences: ΔE OK (Euclidean in OKLab) and CIEDE2000 (in D50 Lab), as CSS Color 4 defines them.",
    since,
    notes: [
      "`deltaEOK` is cheap and is what CSS gamut mapping uses. `deltaE2000` is the industry metric; it is several times the arithmetic.",
      "colord's `delta()` is CIEDE2000 divided by 100.",
    ],
    aliases: ["Color.deltaE2000"],
  },
  {
    slug: "color-harmonies",
    name: "Color.triadic",
    summary: "Colour harmonies as fixed arrays, by HSL hue rotation as colord's harmonies plugin.",
    since,
    aliases: ["Color.complementary", "Color.analogous", "Color.tetradic", "Color.splitComplementary"],
  },
  {
    slug: "i-color-space",
    name: "IColorSpace",
    since,
    notes: [
      "Every space converts through linear sRGB. Generic code over `S : IColorSpace` is specialised by the compiler: there is no dynamic dispatch, and only the spaces used are compiled in.",
      "A space of your own is a struct that implements the four functions.",
    ],
    related: ["convert", "color-from", "color-to", "color-mix"],
  },
  {
    slug: "convert",
    name: "convert",
    since,
  },
  space(
    "srgb",
    "Srgb",
    "sRGB-encoded (r, g, b), 0..1 for SDR; also the transfer functions `Srgb.encode` and `Srgb.decode`.",
    "sRGB-encoded r, g, b",
    [
      "`Srgb.encode(linear)` and `Srgb.decode(encoded)` are the CSS Color 4 transfer functions, per channel, mirrored below 0 and extended above 1.",
      "Interpolating here (`mix<Srgb>`) is what CSS `color-mix(in srgb, …)` and a CSS transition do.",
    ],
  ),
  space(
    "linear-srgb",
    "LinearSrgb",
    "Linear-light sRGB (r, g, b), CSS `srgb-linear`: 1.0 is SDR white.",
    "linear r, g, b",
    ["Mixing here is physically additive light: what a blur or a cross-fade of light does."],
  ),
  space("hsl", "Hsl", "CSS `hsl()` over sRGB-encoded values: (hue°, saturation 0..1, lightness 0..1).", "h°, s, l", [
    "Saturation and lightness are fractions, not percentages: `hsl(150 77.78% 55%)` is `float3(150.0, 0.7778, 0.55)`.",
    "A negative saturation from an out-of-gamut colour is made positive by turning the hue 180°, as CSS Color 4 does. A grey (saturation ≤ 0.001) has hue 0.",
  ]),
  space("hsv", "Hsv", "HSV (HSB) over sRGB-encoded values: (hue°, saturation 0..1, value 0..1).", "h°, s, v", [
    "Not a CSS space, but what colour pickers show. A grey has hue 0.",
  ]),
  space("hwb", "Hwb", "CSS `hwb()` over sRGB-encoded values: (hue°, whiteness 0..1, blackness 0..1).", "h°, w, b", [
    "Whiteness plus blackness of 1 or more is a grey of `w / (w + b)`, as in CSS.",
  ]),
  space("xyz-d65", "XyzD65", "CIE XYZ relative to D65, CSS `xyz-d65`: Y 1.0 is SDR white.", "X, Y, Z", [
    "The hub of the CSS conversions; values above 1 are kept.",
  ]),
  space("xyz-d50", "XyzD50", "CIE XYZ relative to D50 by the Bradford transform, CSS `xyz-d50`.", "X, Y, Z", [
    "D50 is the white point CIE Lab and LCH are defined against.",
  ]),
  space("lab", "Lab", "CIE Lab, CSS `lab()`, D50: (L 0..100, a, b), a and b about -125..125.", "L, a, b", [
    "L above 100 is brighter than SDR white.",
  ]),
  space("lch", "Lch", "CIE LCH, CSS `lch()`: (L 0..100, chroma 0..~150, hue°).", "L, C, h°", [
    "A chroma of 0.0015 or less is a grey, whose hue is powerless (CSS's threshold); `to<Lch>()` gives it hue 0.",
  ]),
  space("oklab", "Oklab", "OKLab, CSS `oklab()`: (L 0..1, a, b), a and b about -0.4..0.4.", "L, a, b", [
    "The CSS default for `color-mix()` and gradients, and `Color.mix`'s default.",
    "L above 1 is brighter than SDR white.",
  ]),
  space(
    "oklch",
    "Oklch",
    "OKLCH, CSS `oklch()`: (L 0..1, chroma 0..~0.37, hue°).",
    "L, C, h°",
    [
      "The space for changing one perceptual property at a time: lightness without a hue shift, a hue turn at even lightness.",
      "A chroma of 0.0001 or less is a grey, with hue 0. CSS uses 0.000004; the larger threshold is float precision, and is far below what an eye sees.",
    ],
    `float3 lch = c.to<Oklch>();
lch.z += input.time * 30.0;   // turn the hue, keep lightness and chroma
return Color.from<Oklch>(lch, c.alpha).toPremultiplied();`,
  ),
]);
