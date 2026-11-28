import { section } from "./shared";

const attribute = (target: "struct" | "field", name: string, args = "") =>
  target === "struct" ? `[${name}${args}]\nstruct MyEffect : IEffect { ... };` : `[${name}${args}] float value;`;

export const sk = section("sk", [
  {
    slug: "sk-effect",
    name: "[SkEffect]",
    usage: `[SkEffect("ripple")]
struct Ripple : IEffect
{
    float4 shade(EffectInput input) { ... }
};`,
    params: [{ name: "name", type: "string", description: "A lowercase CSS ident, `[a-z][a-z0-9-]*`." }],
    notes: [
      "Required, exactly once, on the one struct in a file that conforms to `IEffect`. A file without it is a module other effects can import.",
      "The struct's field order is the CSS argument order and the packing order of the 32-float parameter block.",
      "An app effect cannot take the name of a built-in (`frost`, `noise`, `glow`) or of another app effect.",
    ],
    examples: [
      {
        title: "The smallest effect",
        lang: "slang",
        code: `import sk;

[SkEffect("tint")]
struct Tint : IEffect
{
    [SkColor] [SkDefault("#7fd4ff")] float4 color;

    float4 shade(EffectInput input)
    {
        return sk_premultiply(color);
    }
};`,
      },
    ],
    related: ["i-effect", "sk-doc"],
  },
  {
    slug: "sk-backdrop",
    name: "[SkBackdrop]",
    summary: "The effect reads what is behind its element: `input.backdrop` and `backdropAt`.",
    usage: attribute("struct", "SkBackdrop"),
    notes: [
      "The renderer copies the render target for the effect and draws its quad with blending disabled: the effect owns every pixel of it, and the engine restores the untouched backdrop outside the element's coverage.",
      "Backdrop samples are premultiplied and sRGB-encoded; `Color.fromPremultiplied` reads them. `[SkBackdropLinear]` hands them over in linear light instead.",
      "In an HDR window the result may keep the HDR light it read: it is cut at the brightest value under the pixel rather than at SDR white, so glass over HDR video does not dim its highlights. In an SDR window it is cut at SDR white as before.",
      "Without `[SkBackdropReach]` each frame copies the whole window. Implied by `[SkBackdropRadius]`, `[SkBackdropReach]` and `[SkBackdropLinear]`.",
    ],
    related: ["sk-backdrop-reach", "sk-backdrop-radius", "sk-backdrop-linear", "effect-input-backdrop-at"],
  },
  {
    slug: "sk-content",
    name: "[SkContent]",
    summary: "The effect shades its element's own painting, which it reads through `input.content` and `contentAt`.",
    usage: attribute("struct", "SkContent"),
    notes: [
      "The element and its subtree paint into an offscreen group the size of the window, at full opacity, and the effect is drawn in their place.",
      "The drawn quad is the border box dilated by a `[SkBleed]` parameter. `opacity` applies once, to the result.",
      "A rotation or skew cannot be drawn by an effect; the element then shows unshaded. Hit testing is unchanged.",
      "The group is a Backdrop Root: a backdrop effect inside it sees the element's painting, not the page.",
      "Content samples are premultiplied and sRGB-encoded, like the backdrop.",
    ],
    related: ["effect-input-content-at", "sk-bleed"],
  },
  {
    slug: "sk-history",
    name: "[SkHistory]",
    summary:
      "The effect keeps state per element from frame to frame: two RGBA 16-bit float textures the size of its quad.",
    usage: attribute("struct", "SkHistory"),
    notes: [
      "Each step stores what `update` returns for every texel, unclamped; without an `update` it stores what `shade` returned.",
      "A history steps at most once per painted frame and about 75 times a second, and only while the window repaints: keep the effect animated for as long as the state should change.",
      "No history on a quad longer than 4096 device pixels on a side, and at most 256 MiB of histories per window; past them it reads zeros.",
    ],
    related: ["effect-input-history-at", "i-effect"],
  },
  {
    slug: "sk-doc",
    name: "[SkDoc]",
    usage: attribute("struct", "SkDoc", '("Ripples the backdrop")'),
  },
  {
    slug: "sk-default",
    name: "[SkDefault]",
    usage: `[SkDefault("1.5")] float speed;
[SkDefault("0 0.5")] float2 origin;
[SkColor] [SkDefault("#ffffff28")] float4 tint;`,
    notes: [
      "A string, so one attribute covers every width and a colour can be a hex literal (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`).",
      "One number fills every component of a vector; otherwise the count must match.",
      "On an `SkTexture` it is the colour the image reads as while it is missing.",
    ],
  },
  {
    slug: "sk-range",
    name: "[SkRange]",
    usage: attribute("field", "SkRange", "(0.0, 8.0)"),
  },
  {
    slug: "sk-length",
    name: "[SkLength]",
    summary: "The CSS argument is a `<length>`, multiplied by the device scale before it reaches the shader.",
    usage: attribute("field", "SkLength"),
    notes: ["A length parameter and `input.px` are in the same units, device pixels, on every monitor."],
  },
  {
    slug: "sk-color",
    name: "[SkColor]",
    summary: "The CSS argument is a `<color>`: a `float4` of straight-alpha, sRGB-encoded RGBA in 0..1.",
    usage: `[SkColor] [SkDefault("#7fd4ff")] float4 tint;`,
    notes: [
      "Only legal on a `float4`.",
      "`lab()`, `oklch()`, `color()` and the rest are converted to sRGB and clipped to 0..1 per channel, as a browser paints them on an sRGB display; `color-mix()` and relative colours arrive computed.",
      "A transition between two values interpolates the four channels as they are, in sRGB-encoded straight alpha.",
      "Read it with `Color.fromSrgb(tint)` from `sk.color`; `sk_premultiply(tint)` gives what `shade` returns.",
    ],
    related: ["color-from-srgb", "sk-premultiply"],
  },
  {
    slug: "sk-optional",
    name: "[SkOptional]",
    summary: "The CSS argument may be left out; matching is positional but type-directed.",
    usage: attribute("field", "SkOptional"),
    notes: [
      "An optional parameter must differ in CSS type from the one after it, or `ripple(1)` would be ambiguous; the compiler refuses the effect otherwise.",
      "Not allowed on a bare `float2`..`float4`, which spreads over several CSS arguments.",
    ],
  },
  {
    slug: "sk-slot",
    name: "[SkSlot]",
    usage: attribute("field", "SkSlot", "(4)"),
    notes: [
      "Only for a packing that is already a published contract: CSS transitions interpolate slot by slot. A new effect never needs it.",
      "The slot must be free, inside 0..31, and a vector must not straddle a four-slot boundary.",
    ],
  },
  {
    slug: "sk-bleed",
    name: "[SkBleed]",
    usage: `[SkLength] [SkBleed] [SkDefault("12")] float radius;`,
    notes: [
      "At most one per effect. `input.bleed` is its resolved value, and `input.local` runs outside 0..1 in the bleed.",
    ],
  },
  {
    slug: "sk-backdrop-radius",
    name: "[SkBackdropRadius]",
    usage: `[SkLength] [SkBackdropRadius] [SkDefault("14")] float radius;`,
    notes: ["Implies `[SkBackdrop]`. At most one per effect. `rawBackdropAt` still reads the unblurred snapshot."],
  },
  {
    slug: "sk-backdrop-reach",
    name: "[SkBackdropReach]",
    summary: "A length parameter that bounds how far outside its quad the effect reads the backdrop.",
    since: "0.9.0",
    usage: `[SkLength] [SkBackdropReach] [SkDefault("12")] float depth;`,
    notes: [
      "The renderer then copies and blurs only the drawn quad grown by it, instead of the whole window. A read beyond it is clamped to that edge.",
      "Implies `[SkBackdrop]`. At most one per effect.",
    ],
  },
  {
    slug: "sk-backdrop-linear",
    name: "[SkBackdropLinear]",
    summary:
      "The effect works in linear light: its backdrop arrives linear and extended, and its result is read back so.",
    since: "0.9.0",
    usage: attribute("struct", "SkBackdropLinear"),
    notes: [
      "`input.backdrop`, `backdropAt`, `rawBackdropAt` and `pageBackdropAt` return premultiplied linear light: 1.0 is SDR white, an HDR highlight keeps its value above it, and values below 0 are colours outside sRGB. On macOS with a Display P3 window the primaries are Display P3.",
      "The `[SkBackdropRadius]` blur is taken in linear light in half float, so a bright highlight spreads as light does instead of as sRGB codes.",
      "What `shade` returns is premultiplied linear light, not cut at SDR white; alpha is still clamped to 0..1. `return input.backdrop;` is an exact identity in SDR and HDR windows.",
      "`input.content`, `history` and `SkTexture` images stay sRGB-encoded; `sk_frame_to_linear` converts them.",
      "Implies `[SkBackdrop]`.",
    ],
    examples: [
      {
        title: "Frosted glass that keeps HDR highlights",
        lang: "slang",
        code: `import sk;
import sk.tone;

[SkEffect("hdr-glass")]
[SkBackdropLinear]
struct HdrGlass : IEffect
{
    [SkLength] [SkBackdropRadius] [SkDefault("16")] float radius;

    float4 shade(EffectInput input)
    {
        // Highlights up to 2x SDR white pass as they are, brighter ones roll off below 6x.
        float4 light = input.backdrop;
        float3 straight = light.rgb / max(light.a, 1e-4);
        return float4(softClip(straight, 2.0, 6.0) * light.a, light.a);
    }
};`,
      },
    ],
    related: ["sk-backdrop", "sk-frame-to-linear", "tone-soft-clip"],
  },
  {
    slug: "sk-animated",
    name: "[SkAnimated]",
    summary: "While this parameter is above 0 the effect is animated and the window keeps repainting.",
    usage: `[SkAnimated] [SkDefault("1")] float speed;`,
    notes: [
      "Put it on the speed that multiplies `input.time`: the host integrates the speed per element, so a change of speed carries on from where the motion is.",
      "At most one per effect. Without one, the effect never repaints on its own.",
    ],
  },
  {
    slug: "sk-animated-for",
    name: "[SkAnimatedFor]",
    summary: "Every component is a time on the effect clock; the window repaints until `seconds` after the latest.",
    usage: `[SkAnimatedFor(0.6)] float4 started;`,
    params: [{ name: "seconds", type: "float", description: "How long after each time the effect keeps repainting." }],
    notes: ["Hand the effect the time something started rather than rewriting an age every frame."],
  },
  {
    slug: "sk-pointer",
    name: "[SkPointer]",
    summary: "A `float2` the host fills with the pointer's position in `input.local` units; not a CSS argument.",
    usage: `[SkPointer] float2 pointer;`,
    notes: [
      "0..1 across the element, beyond that outside it, anywhere in the window. The last position stays when the pointer leaves.",
      "The element repaints when the pointer moves, and only elements whose effect has a pointer parameter do.",
    ],
    related: ["sk-pointer-times", "sk-pointer-presence"],
  },
  {
    slug: "sk-pointer-times",
    name: "[SkPointerTimes]",
    summary: "A `float3` of effect-clock times: when the pointer last moved, entered and left the window.",
    usage: `[SkPointerTimes] [SkAnimatedFor(0.5)] float3 seen;`,
    notes: [
      "The pointer is in the window while `y > z`. About -1e4 until each happens. `sk_pointer_presence` turns the three into a fade.",
    ],
    related: ["sk-pointer-presence"],
  },
  {
    slug: "sk-linear",
    name: "[SkLinear]",
    summary: "On an `SkTexture`: the image is data, sampled as its file stores it, without premultiplying.",
    usage: `[SkLinear] SkTexture normals;`,
    notes: [
      "For normal maps, noise and lookup tables. Alpha is just a fourth channel.",
      "Without it an image is colour, sampled premultiplied in the same sRGB-encoded space as the backdrop.",
    ],
    related: ["sk-texture"],
  },
  {
    slug: "sk-quality",
    name: "[SkQuality]",
    since: "0.7.0",
    usage: `[SkQuality] float quality;`,
    notes: [
      "The tier follows CPU pressure and the power profile, or the element's `effect-quality`. See [Adaptive effects](/guides/adaptive-effects/).",
    ],
  },
  {
    slug: "i-effect",
    name: "IEffect",
    summary:
      "Implement `shade` to colour a pixel. Override `straightAlpha` for straight-alpha output or `update` to write history.",
    notes: [
      "`shade` returns this pixel's colour, premultiplied and sRGB-encoded, unless `straightAlpha` says otherwise.",
      "`static override bool straightAlpha() { return true; }` makes the engine premultiply what `shade` returns; it is resolved at compile time and costs nothing.",
      "`update` computes the next `[SkHistory]` state for a texel; it runs with `input.historyPass` set and never reads the backdrop.",
      "After `shade` the engine clamps the colour to valid premultiplied 0..1, applies the element's coverage, opacity and clip, and for a backdrop effect restores the untouched backdrop outside the coverage. An effect cannot skip it.",
    ],
    examples: [
      {
        title: "Author in straight alpha",
        lang: "slang",
        code: `import sk;
import sk.color;

[SkEffect("wash")]
struct Wash : IEffect
{
    [SkColor] [SkDefault("#ff8a3d80")] float4 color;

    static override bool straightAlpha() { return true; }

    float4 shade(EffectInput input)
    {
        return Color.fromSrgb(color).lighten(input.local.y * 0.3).toSrgb();
    }
};`,
      },
    ],
    related: ["effect-input", "sk-effect"],
  },
  {
    slug: "effect-input",
    name: "EffectInput",
    summary:
      "Everything the engine hands `shade` about this pixel: coordinates, coverage, time and the samples the effect requested.",
    notes: [
      "`backdrop`, `content` and `history` are zero for an effect that did not ask for them.",
      "Samples of the backdrop, the content and colour images are premultiplied and sRGB-encoded. In an HDR window their channels can exceed 1.0 (and go below 0); `Color.fromPremultiplied` keeps those values.",
      "`coverage`, `opacity` and `clip` are applied by the engine after `shade`; they are exposed for an effect that wants to fade a part of its own by the same amount.",
    ],
    related: [
      "effect-input-backdrop-at",
      "effect-input-content-at",
      "effect-input-history-at",
      "effect-input-centered",
    ],
  },
  {
    slug: "effect-input-backdrop-at",
    name: "EffectInput.backdropAt",
    summary:
      "The backdrop at another point: blurred to `[SkBackdropRadius]`, or the untouched snapshot with `rawBackdropAt`.",
    aliases: ["EffectInput.backdropAtPx", "EffectInput.rawBackdropAt", "EffectInput.rawBackdropAtPx"],
    returns:
      "Premultiplied, sRGB-encoded RGBA (premultiplied linear light with `[SkBackdropLinear]`); zero without `[SkBackdrop]`.",
    notes: [
      "The backdrop covers the whole window as it was before this effect; outside the window it clamps to the edge. With `[SkBackdropReach]` a read clamps to the drawn quad grown by the reach.",
      "It holds neither the element's own background nor another backdrop effect of the same group.",
      "Sampled at mip 0, so safe in loops and branches.",
    ],
    related: ["sk-backdrop", "color-from-premultiplied"],
  },
  {
    slug: "effect-input-page-backdrop-at",
    name: "EffectInput.pageBackdropAt",
    summary: "The page alone at a point: `rawBackdropAt` without the pictures of `<sk-layer>` elements under it.",
    aliases: ["EffectInput.pageBackdropAtPx"],
    returns: "Premultiplied, sRGB-encoded RGBA with the page's own alpha; zero without `[SkBackdrop]`.",
    notes: [
      "`backdropAt` and `rawBackdropAt` hold the page over the pictures of `<sk-layer>` elements under it, as the viewer sees it; this holds the page alone. Without such pictures the two are equal.",
      "The engine writes this back outside the element's shape, so the live layer keeps showing through a transparent page there. An effect that draws in its bleed on purpose composites over it.",
    ],
    related: ["effect-input-backdrop-at", "sk-backdrop"],
  },
  {
    slug: "effect-input-content-at",
    name: "EffectInput.contentAt",
    summary: "The element's own painting at another point, with `[SkContent]`.",
    aliases: ["EffectInput.contentAtPx", "EffectInput.contentSize"],
    returns:
      "Premultiplied, sRGB-encoded RGBA, transparent where the element painted nothing; zero without `[SkContent]`.",
    notes: ["The content covers the whole window; `contentSize()` is its size in texels, one per device pixel."],
    related: ["sk-content"],
  },
  {
    slug: "effect-input-history-at",
    name: "EffectInput.historyAt",
    summary: "The effect's state as the last step stored it, with `[SkHistory]`.",
    aliases: ["EffectInput.historyAtPx", "EffectInput.historySize"],
    returns: "What `update` (or `shade`) returned, raw: not premultiplied, not clamped, 16-bit float.",
    notes: ["One texel per device pixel of the drawn quad, clamped at its edge. Zero on the first frame."],
    related: ["sk-history"],
  },
  {
    slug: "effect-input-px-to-uv",
    name: "EffectInput.pxToUv",
  },
  {
    slug: "effect-input-centered",
    name: "EffectInput.centered",
  },
  {
    slug: "effect-input-aa",
    name: "EffectInput.aa",
  },
  {
    slug: "effect-input-param",
    name: "EffectInput.param",
  },
  {
    slug: "sk-texture",
    name: "SkTexture",
    summary: "An image parameter, `url(...)` in CSS. At most four per effect.",
    notes: [
      "Resolved like an `<img src>`. Until it has loaded, or when it fails or is omitted, every read returns the field's `[SkDefault]` colour, transparent without one.",
      "A colour image samples premultiplied and sRGB-encoded; `[SkLinear]` samples it as stored.",
      "Sampled bilinearly at mip 0, so safe in loops and branches.",
    ],
    related: ["sk-linear", "sk-default"],
  },
  {
    slug: "sk-premultiply",
    name: "sk_premultiply",
    returns: "`float4(c.rgb * c.a, c.a)` for the straight-alpha input `c`.",
    related: ["color-to-premultiplied"],
  },
  {
    slug: "sk-over",
    name: "sk_over",
  },
  {
    slug: "sk-frame-to-linear",
    name: "sk_frame_to_linear",
    summary:
      "A premultiplied, sRGB-encoded sample to premultiplied linear light, 1.0 at SDR white, as the HDR output shows it; `sk_linear_to_frame` is its exact inverse.",
    since: "0.9.0",
    notes: [
      "Up to alpha a channel is decoded as stored; the part above alpha, HDR light, is decoded straight and added. Negative values decode with their sign.",
      "Use it for `input.content` or an `SkTexture` in a `[SkBackdropLinear]` effect, or to work in linear light in any effect and encode the result back.",
    ],
    aliases: ["sk_linear_to_frame"],
    related: ["sk-backdrop-linear", "color-from-premultiplied"],
  },
  {
    slug: "sk-clamp-extended",
    name: "sk_clamp_extended",
    since: "0.9.0",
    notes: [
      "The engine applies it to what `shade` returns in an HDR window, with the brightest value under the pixel as `headroom`; call it to keep an intermediate result in the same bounds. A headroom below 1 counts as 1, and the bound never exceeds the largest half float.",
    ],
  },
  {
    slug: "sk-luma",
    name: "sk_luma",
    returns: "Rec. 709 luma, `dot(rgb, float3(0.2126, 0.7152, 0.0722))`.",
    notes: [
      "It weights whatever it is given; on sRGB-encoded values it is not luminance. `Color.luminance()` decodes first.",
    ],
    related: ["color-luminance"],
  },
  {
    slug: "sk-palette",
    name: "sk_palette",
  },
  {
    slug: "sk-rotate",
    name: "sk_rotate",
  },
  {
    slug: "sk-hash",
    name: "sk_hash12",
    summary: "Deterministic hashes of a 2D point: one or two values in 0..1, no texture needed.",
    aliases: ["sk_hash22"],
  },
  {
    slug: "sk-value-noise",
    name: "sk_value_noise",
    summary: "Value noise on a unit lattice, smoothstep-interpolated, 0..1; `sk_fbm` sums three octaves of it.",
    aliases: ["sk_fbm"],
  },
  {
    slug: "sk-pointer-presence",
    name: "sk_pointer_presence",
    params: [
      { name: "times", type: "float3", description: "An `[SkPointerTimes]` field." },
      { name: "time", type: "float", description: "`input.time`." },
      { name: "seconds", type: "float", description: "The fade; pair the field with `[SkAnimatedFor(seconds)]`." },
    ],
    related: ["sk-pointer-times"],
  },
  {
    slug: "sk-quad-sdf",
    name: "sk_quad_sdf",
    summary: "Signed distance to a rounded rect, negative inside; `sk_shaped_quad_sdf` adds CSS `corner-shape`.",
    aliases: ["sk_shaped_quad_sdf"],
  },
]);
