import { section } from "./shared";

const attribute = (target: "struct" | "field", name: string, args = "") =>
  target === "struct" ? `[${name}${args}]\nstruct MyEffect : IEffect { ... };` : `[${name}${args}] float value;`;

export const sk = section("sk", [
  {
    slug: "sk-effect",
    name: "[SkEffect]",
    kind: "attribute",
    summary: 'Names the effect: `[SkEffect("ripple")]` is `--shading: ripple(...)` in CSS.',
    syntax: `[SkEffect("ripple")]
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
    kind: "attribute",
    summary: "The effect reads what is behind its element: `input.backdrop` and `backdropAt`.",
    syntax: attribute("struct", "SkBackdrop"),
    notes: [
      "The renderer copies the render target for the effect and draws its quad with blending disabled: the effect owns every pixel of it, and the engine restores the untouched backdrop outside the element's coverage.",
      "Backdrop samples are premultiplied and sRGB-encoded; `Color.fromPremultiplied` reads them.",
      "Without `[SkBackdropReach]` each frame copies the whole window. Implied by `[SkBackdropRadius]` and `[SkBackdropReach]`.",
    ],
    related: ["sk-backdrop-reach", "sk-backdrop-radius", "effect-input-backdrop-at"],
  },
  {
    slug: "sk-content",
    name: "[SkContent]",
    kind: "attribute",
    summary: "The effect shades its element's own painting, which it reads through `input.content` and `contentAt`.",
    syntax: attribute("struct", "SkContent"),
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
    kind: "attribute",
    summary:
      "The effect keeps state per element from frame to frame: two RGBA 16-bit float textures the size of its quad.",
    syntax: attribute("struct", "SkHistory"),
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
    kind: "attribute",
    summary: "One line of documentation, echoed in CSS advisories about the effect.",
    syntax: attribute("struct", "SkDoc", '("Ripples the backdrop")'),
  },
  {
    slug: "sk-default",
    name: "[SkDefault]",
    kind: "attribute",
    summary: 'A parameter\'s default, written as in CSS: `"1.5"`, `"0 0.5"`, `"#7fd4ff"`.',
    syntax: `[SkDefault("1.5")] float speed;
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
    kind: "attribute",
    summary: "An inclusive range for a scalar parameter; a CSS value outside it is clamped with an advisory.",
    syntax: attribute("field", "SkRange", "(0.0, 8.0)"),
  },
  {
    slug: "sk-length",
    name: "[SkLength]",
    kind: "attribute",
    summary: "The CSS argument is a `<length>`, multiplied by the device scale before it reaches the shader.",
    syntax: attribute("field", "SkLength"),
    notes: ["A length parameter and `input.px` are in the same units, device pixels, on every monitor."],
  },
  {
    slug: "sk-color",
    name: "[SkColor]",
    kind: "attribute",
    summary: "The CSS argument is a `<color>`: a `float4` of straight-alpha, sRGB-encoded RGBA in 0..1.",
    syntax: `[SkColor] [SkDefault("#7fd4ff")] float4 tint;`,
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
    kind: "attribute",
    summary: "The CSS argument may be left out; matching is positional but type-directed.",
    syntax: attribute("field", "SkOptional"),
    notes: [
      "An optional parameter must differ in CSS type from the one after it, or `ripple(1)` would be ambiguous; the compiler refuses the effect otherwise.",
      "Not allowed on a bare `float2`..`float4`, which spreads over several CSS arguments.",
    ],
  },
  {
    slug: "sk-slot",
    name: "[SkSlot]",
    kind: "attribute",
    summary: "Pins a parameter to a slot of the 32-float block instead of letting the packer choose.",
    syntax: attribute("field", "SkSlot", "(4)"),
    notes: [
      "Only for a packing that is already a published contract: CSS transitions interpolate slot by slot. A new effect never needs it.",
      "The slot must be free, inside 0..31, and a vector must not straddle a four-slot boundary.",
    ],
  },
  {
    slug: "sk-bleed",
    name: "[SkBleed]",
    kind: "attribute",
    summary: "A length parameter that dilates the drawn quad, giving an outer glow somewhere to land.",
    syntax: `[SkLength] [SkBleed] [SkDefault("12")] float radius;`,
    notes: [
      "At most one per effect. `input.bleed` is its resolved value, and `input.local` runs outside 0..1 in the bleed.",
    ],
  },
  {
    slug: "sk-backdrop-radius",
    name: "[SkBackdropRadius]",
    kind: "attribute",
    summary: "A length parameter that is the radius the backdrop is blurred to before the effect samples it.",
    syntax: `[SkLength] [SkBackdropRadius] [SkDefault("14")] float radius;`,
    notes: ["Implies `[SkBackdrop]`. At most one per effect. `rawBackdropAt` still reads the unblurred snapshot."],
  },
  {
    slug: "sk-backdrop-reach",
    name: "[SkBackdropReach]",
    kind: "attribute",
    summary: "A length parameter that bounds how far outside its quad the effect reads the backdrop.",
    since: "Unreleased",
    syntax: `[SkLength] [SkBackdropReach] [SkDefault("12")] float depth;`,
    notes: [
      "The renderer then copies and blurs only the drawn quad grown by it, instead of the whole window. A read beyond it is clamped to that edge.",
      "Implies `[SkBackdrop]`. At most one per effect.",
    ],
  },
  {
    slug: "sk-animated",
    name: "[SkAnimated]",
    kind: "attribute",
    summary: "While this parameter is above 0 the effect is animated and the window keeps repainting.",
    syntax: `[SkAnimated] [SkDefault("1")] float speed;`,
    notes: [
      "Put it on the speed that multiplies `input.time`: the host integrates the speed per element, so a change of speed carries on from where the motion is.",
      "At most one per effect. Without one, the effect never repaints on its own.",
    ],
  },
  {
    slug: "sk-animated-for",
    name: "[SkAnimatedFor]",
    kind: "attribute",
    summary: "Every component is a time on the effect clock; the window repaints until `seconds` after the latest.",
    syntax: `[SkAnimatedFor(0.6)] float4 started;`,
    params: [{ name: "seconds", type: "float", description: "How long after each time the effect keeps repainting." }],
    notes: ["Hand the effect the time something started rather than rewriting an age every frame."],
  },
  {
    slug: "sk-pointer",
    name: "[SkPointer]",
    kind: "attribute",
    summary: "A `float2` the host fills with the pointer's position in `input.local` units; not a CSS argument.",
    syntax: `[SkPointer] float2 pointer;`,
    notes: [
      "0..1 across the element, beyond that outside it, anywhere in the window. The last position stays when the pointer leaves.",
      "The element repaints when the pointer moves, and only elements whose effect has a pointer parameter do.",
    ],
    related: ["sk-pointer-times", "sk-pointer-presence"],
  },
  {
    slug: "sk-pointer-times",
    name: "[SkPointerTimes]",
    kind: "attribute",
    summary: "A `float3` of effect-clock times: when the pointer last moved, entered and left the window.",
    syntax: `[SkPointerTimes] [SkAnimatedFor(0.5)] float3 seen;`,
    notes: [
      "The pointer is in the window while `y > z`. About -1e4 until each happens. `sk_pointer_presence` turns the three into a fade.",
    ],
    related: ["sk-pointer-presence"],
  },
  {
    slug: "sk-linear",
    name: "[SkLinear]",
    kind: "attribute",
    summary: "On an `SkTexture`: the image is data, sampled as its file stores it, without premultiplying.",
    syntax: `[SkLinear] SkTexture normals;`,
    notes: [
      "For normal maps, noise and lookup tables. Alpha is just a fourth channel.",
      "Without it an image is colour, sampled premultiplied in the same sRGB-encoded space as the backdrop.",
    ],
    related: ["sk-texture"],
  },
  {
    slug: "sk-quality",
    name: "[SkQuality]",
    kind: "attribute",
    summary: "A host-written float: 0 for full detail, 1 for reduced, 2 for minimal; opts into adaptive quality.",
    since: "0.7.0",
    syntax: `[SkQuality] float quality;`,
    notes: [
      "The tier follows CPU pressure and the power profile, or the element's `effect-quality`. See [Adaptive effects](/guides/adaptive-effects/).",
    ],
  },
  {
    slug: "i-effect",
    name: "IEffect",
    kind: "interface",
    summary: "The interface an effect struct implements: `shade`, and optionally `straightAlpha` and `update`.",
    syntax: `interface IEffect
{
    float4 shade(EffectInput input);
    static bool straightAlpha();        // false unless overridden
    float4 update(EffectInput input);   // [SkHistory]; defaults to shade
};`,
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
    kind: "struct",
    summary:
      "Everything the engine hands `shade` about this pixel: coordinates, coverage, time and the samples the effect asked for.",
    syntax: `struct EffectInput
{
    float2 uv;          // window UV, 0..1 across the window
    float2 local;       // 0..1 across the element; outside it in the bleed
    float2 px;          // device pixels from the element's top-left
    float2 size;        // the element's size in device pixels
    float4 backdrop;    // backdropAt(uv): premultiplied, sRGB-encoded
    float sd;           // signed distance to the element's rounded rect, device px
    float coverage;     // antialiased coverage of that rect
    float time;         // seconds, performance.now() / 1000
    float scale;        // device pixels per CSS pixel
    float bleed;        // the [SkBleed] value in device pixels
    float opacity;      // the element's opacity, applied by the engine
    float clip;         // coverage of a rounded overflow clip, applied by the engine
    bool hasBackdrop;
    float4 content;     // contentAt(uv), with [SkContent]
    bool hasContent;
    float4 history;     // historyAtPx(px), with [SkHistory]
    bool hasHistory;
    bool historyPass;   // true while update() runs
    uint quad;
};`,
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
    kind: "method",
    summary:
      "The backdrop at another point: blurred to `[SkBackdropRadius]`, or the untouched snapshot with `rawBackdropAt`.",
    syntax: `float4 backdropAt(float2 uv);
float4 backdropAtPx(float2 px);
float4 rawBackdropAt(float2 uv);
float4 rawBackdropAtPx(float2 px);`,
    returns: "Premultiplied, sRGB-encoded RGBA; zero without `[SkBackdrop]`.",
    notes: [
      "The backdrop covers the whole window as it was before this effect; outside the window it clamps to the edge. With `[SkBackdropReach]` a read clamps to the drawn quad grown by the reach.",
      "It holds neither the element's own background nor another backdrop effect of the same group.",
      "Sampled at mip 0, so safe in loops and branches.",
    ],
    related: ["sk-backdrop", "color-from-premultiplied"],
  },
  {
    slug: "effect-input-content-at",
    name: "EffectInput.contentAt",
    kind: "method",
    summary: "The element's own painting at another point, with `[SkContent]`.",
    syntax: `float4 contentAt(float2 uv);
float4 contentAtPx(float2 px);
float2 contentSize();`,
    returns:
      "Premultiplied, sRGB-encoded RGBA, transparent where the element painted nothing; zero without `[SkContent]`.",
    notes: ["The content covers the whole window; `contentSize()` is its size in texels, one per device pixel."],
    related: ["sk-content"],
  },
  {
    slug: "effect-input-history-at",
    name: "EffectInput.historyAt",
    kind: "method",
    summary: "The effect's state as the last step stored it, with `[SkHistory]`.",
    syntax: `float4 historyAt(float2 uv);
float4 historyAtPx(float2 px);
float2 historySize();`,
    returns: "What `update` (or `shade`) returned, raw: not premultiplied, not clamped, 16-bit float.",
    notes: ["One texel per device pixel of the drawn quad, clamped at its edge. Zero on the first frame."],
    related: ["sk-history"],
  },
  {
    slug: "effect-input-px-to-uv",
    name: "EffectInput.pxToUv",
    kind: "method",
    summary: "A point in the element's device pixels as a window UV.",
    syntax: `float2 pxToUv(float2 px);`,
  },
  {
    slug: "effect-input-centered",
    name: "EffectInput.centered",
    kind: "method",
    summary: "`local` with (0, 0) at the centre and the shorter axis spanning -1..1, so a pattern does not stretch.",
    syntax: `float2 centered();`,
  },
  {
    slug: "effect-input-aa",
    name: "EffectInput.aa",
    kind: "method",
    summary: "Antialiased 0..1 coverage from a signed distance in device pixels, one pixel of feather.",
    syntax: `float aa(float distance);`,
  },
  {
    slug: "effect-input-param",
    name: "EffectInput.param",
    kind: "method",
    summary: "Raw parameter slot 0..31, bypassing the schema. Prefer the named fields.",
    syntax: `float param(uint slot);`,
  },
  {
    slug: "sk-texture",
    name: "SkTexture",
    kind: "struct",
    summary: "An image parameter, `url(...)` in CSS. At most four per effect.",
    syntax: `struct SkTexture
{
    float2 size;                    // texels; (0, 0) while missing
    bool loaded();
    float4 sample(float2 uv);       // clamped to the edge
    float4 sampleRepeat(float2 uv); // repeating
};`,
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
    kind: "function",
    summary: "Straight alpha to premultiplied: `float4(c.rgb * c.a, c.a)`.",
    syntax: `float4 sk_premultiply(float4 color);`,
    related: ["color-to-premultiplied"],
  },
  {
    slug: "sk-over",
    name: "sk_over",
    kind: "function",
    summary: "Premultiplied `above` composited over premultiplied `below`, as the engine blends.",
    syntax: `float4 sk_over(float4 below, float4 above);`,
  },
  {
    slug: "sk-luma",
    name: "sk_luma",
    kind: "function",
    summary: "Rec. 709 luma of an RGB triple: `dot(rgb, (0.2126, 0.7152, 0.0722))`.",
    syntax: `float sk_luma(float3 rgb);`,
    notes: [
      "It weights whatever it is given; on sRGB-encoded values it is not luminance. `Color.luminance()` decodes first.",
    ],
    related: ["color-luminance"],
  },
  {
    slug: "sk-palette",
    name: "sk_palette",
    kind: "function",
    summary: "Inigo Quilez's cosine palette, `a + b * cos(2π (c t + d))`.",
    syntax: `float3 sk_palette(float t, float3 a, float3 b, float3 c, float3 d);`,
  },
  {
    slug: "sk-rotate",
    name: "sk_rotate",
    kind: "function",
    summary: "A 2D point rotated by `radians` about the origin.",
    syntax: `float2 sk_rotate(float2 p, float radians);`,
  },
  {
    slug: "sk-hash",
    name: "sk_hash12",
    kind: "function",
    summary: "Deterministic hashes of a 2D point: one or two values in 0..1, no texture needed.",
    syntax: `float sk_hash12(float2 p);
float2 sk_hash22(float2 p);`,
    aliases: ["sk_hash22"],
  },
  {
    slug: "sk-value-noise",
    name: "sk_value_noise",
    kind: "function",
    summary: "Value noise on a unit lattice, smoothstep-interpolated, 0..1; `sk_fbm` sums three octaves of it.",
    syntax: `float sk_value_noise(float2 p);
float sk_fbm(float2 p);`,
    aliases: ["sk_fbm"],
  },
  {
    slug: "sk-pointer-presence",
    name: "sk_pointer_presence",
    kind: "function",
    summary: "0..1: how present the pointer is in the window, fading in and out over `seconds`.",
    syntax: `float sk_pointer_presence(float3 times, float time, float seconds);`,
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
    kind: "function",
    summary: "Signed distance to a rounded rect, negative inside; `sk_shaped_quad_sdf` adds CSS `corner-shape`.",
    syntax: `float sk_quad_sdf(float2 point, SkBounds bounds, SkCorners radii);
float sk_shaped_quad_sdf(float2 point, SkBounds bounds, SkCorners radii, SkCornerShapes shapes);`,
    aliases: ["sk_shaped_quad_sdf"],
  },
]);
