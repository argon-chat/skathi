import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const mode = (css: string, name: string, formula: string, notes?: string[]): Draft => ({
  slug: `blend-${css}`,
  name: `Blend.${name}`,
  kind: "struct",
  summary: `CSS \`mix-blend-mode: ${css}\`: ${formula}`,
  since,
  syntax: `float4 r = blend<Blend.${name}>(below, above);
float3 b = Blend.${name}.apply(cb, cs);   // B(Cb, Cs) on straight colours`,
  ...(notes ? { notes } : {}),
  related: ["blend", "i-blend-mode"],
});

const op = (css: string, name: string, summary: string, factors: string, notes?: string[]): Draft => ({
  slug: `composite-${css}`,
  name: `Composite.${name}`,
  kind: "struct",
  summary: `\`${css}\`: ${summary}`,
  since,
  syntax: `float4 r = composite<Composite.${name}>(below, above);   // ${factors}`,
  ...(notes ? { notes } : {}),
  related: ["composite", "i-composite-op"],
});

const NON_SEPARABLE =
  "Works on whole colours through the spec's `Lum`, `Sat` and `ClipColor` (luminosity 0.3 R + 0.59 G + 0.11 B), and ClipColor brings the result into 0..1, so this mode does not carry HDR values.";

const drafts: Draft[] = [
  {
    slug: "blend",
    name: "blend",
    kind: "function",
    summary:
      "CSS `mix-blend-mode`: `above` blended with `below` by a mode, then composited source-over. Premultiplied in and out.",
    since,
    syntax: `import sk.blend;

float4 blend<M : IBlendMode>(float4 below, float4 above);
float4 blend<M : IBlendMode>(float4 below, float4 above, float opacity);
Color blend<M : IBlendMode>(Color below, Color above);`,
    params: [
      { name: "M", type: "IBlendMode", description: "The mode: `Blend.Multiply`, `Blend.Overlay`, … (see the list)." },
      { name: "below", type: "float4", description: "The backdrop, premultiplied." },
      { name: "above", type: "float4", description: "The source, premultiplied." },
      {
        name: "opacity",
        type: "float",
        description: "Scales `above` first, as CSS `opacity` on the blended layer.",
        optional: true,
      },
    ],
    returns:
      "The W3C Compositing and Blending formula: `cs·(1 − αb) + cb·(1 − αs) + αs·αb·B(Cb, Cs)`, alpha `αs + αb·(1 − αs)`.",
    notes: [
      "The engine's colours are sRGB-encoded and premultiplied, and CSS blends sRGB-encoded values, so passing `input.backdrop`, `input.content` or `sk_premultiply(tint)` straight in gives what a browser shows.",
      "A fully transparent input blends as transparent black, so `blend<M>(below, 0)` is `below`.",
      "The `Color` form blends sRGB-encoded too; `blendLinear` blends linear light instead.",
      "The mode is a type, so the call specialises at compile time: only that mode's formula reaches the shader.",
    ],
    examples: [
      {
        title: "A CSS-accurate overlay tint over the backdrop",
        lang: "slang",
        code: `import sk;
import sk.blend;

[SkEffect("overlay-tint")]
[SkBackdrop]
struct OverlayTint : IEffect
{
    [SkColor] [SkDefault("#ff7a1a")] float4 tint;
    [SkRange(0.0, 1.0)] [SkDefault("0.8")] float strength;

    float4 shade(EffectInput input)
    {
        return blend<Blend.Overlay>(input.backdrop, sk_premultiply(tint), strength);
    }
};`,
      },
    ],
    related: ["i-blend-mode", "blend-linear", "composite", "sk-over"],
  },
  {
    slug: "blend-linear",
    name: "blendLinear",
    kind: "function",
    summary: "`blend<M>` on `Color`s in linear light: physically even for screen and multiply, but not what CSS shows.",
    since,
    syntax: `Color blendLinear<M : IBlendMode>(Color below, Color above);`,
    notes: [
      "Light adds in linear space, so a `Blend.Screen` of two lamps or a `Blend.Multiply` filter looks physically right here. Use `blend` to match a browser.",
    ],
    related: ["blend", "color"],
  },
  {
    slug: "i-blend-mode",
    name: "IBlendMode",
    kind: "interface",
    summary: "A blend function B(Cb, Cs) on straight colours: what `blend<M>` is specialised for.",
    since,
    syntax: `interface IBlendMode
{
    static float3 apply(float3 below, float3 above);
};`,
    notes: [
      "Write your own mode as a struct with a static `apply` and pass it to `blend`: the compositing around it stays the spec's.",
      "Values above 1 pass through the separable formulas unchanged except where the formula is bounded: `color-dodge` and `color-burn` stop at 0 and 1, `soft-light` reads a backdrop below 0 as 0 under its square root, and the four non-separable modes clip into 0..1.",
    ],
    examples: [
      {
        title: "A mode of your own: average",
        lang: "slang",
        code: `import sk;
import sk.blend;

struct Average : IBlendMode
{
    static float3 apply(float3 below, float3 above) { return 0.5 * (below + above); }
};

[SkEffect("average-tint")]
[SkBackdrop]
struct AverageTint : IEffect
{
    [SkColor] [SkDefault("#3d7fff")] float4 tint;

    float4 shade(EffectInput input)
    {
        return blend<Average>(input.backdrop, sk_premultiply(tint));
    }
};`,
      },
    ],
    related: ["blend"],
  },
  mode("normal", "Normal", "the source, B = Cs."),
  mode("multiply", "Multiply", "B = Cb × Cs, never lighter than either layer."),
  mode("screen", "Screen", "B = Cb + Cs − Cb × Cs, never darker than either layer."),
  mode(
    "overlay",
    "Overlay",
    "hard-light with the layers swapped: multiplies the backdrop's darks, screens its lights.",
  ),
  mode("darken", "Darken", "B = min(Cb, Cs) per channel."),
  mode("lighten", "Lighten", "B = max(Cb, Cs) per channel."),
  mode("color-dodge", "ColorDodge", "B = min(1, Cb / (1 − Cs)); 0 where Cb is 0, 1 where Cs is 1.", [
    "A backdrop at or below 0 stays 0 and a source at or above 1 gives 1, so no division by zero reaches the result; the result stops at 1, also for an HDR backdrop.",
  ]),
  mode("color-burn", "ColorBurn", "B = 1 − min(1, (1 − Cb) / Cs); 1 where Cb is 1, 0 where Cs is 0.", [
    "A backdrop at or above 1 stays 1 and a source at or below 0 gives 0; the result stops at 0.",
  ]),
  mode("hard-light", "HardLight", "multiply where Cs ≤ 0.5, screen above."),
  mode("soft-light", "SoftLight", "the W3C soft-light, a gentler hard-light.", [
    "A backdrop below 0 reads as 0 under the formula's square root.",
  ]),
  mode("difference", "Difference", "B = |Cb − Cs|."),
  mode("exclusion", "Exclusion", "B = Cb + Cs − 2 × Cb × Cs, a softer difference."),
  mode("hue", "Hue", "the source's hue with the backdrop's saturation and luminosity.", [NON_SEPARABLE]),
  mode("saturation", "Saturation", "the source's saturation with the backdrop's hue and luminosity.", [NON_SEPARABLE]),
  mode("color", "Color", "the source's hue and saturation with the backdrop's luminosity.", [
    NON_SEPARABLE,
    "Inside `Blend` it is spelled `Blend.Color`, so it never meets `sk.color`'s `Color`.",
  ]),
  mode("luminosity", "Luminosity", "the source's luminosity with the backdrop's hue and saturation.", [NON_SEPARABLE]),
  {
    slug: "composite",
    name: "composite",
    kind: "function",
    summary: "A Porter-Duff operator applied to premultiplied `above` (the source) and `below` (the backdrop).",
    since,
    syntax: `float4 composite<Op : ICompositeOp>(float4 below, float4 above);
Color composite<Op : ICompositeOp>(Color below, Color above);`,
    params: [
      {
        name: "Op",
        type: "ICompositeOp",
        description: "`Composite.SourceOver`, `Composite.DestinationOut`, … (see the list).",
      },
      { name: "below", type: "float4", description: "The backdrop (the destination), premultiplied." },
      { name: "above", type: "float4", description: "The source, premultiplied." },
    ],
    returns: "`above × Fa + below × Fb` for colour and alpha alike, with the operator's factors.",
    notes: [
      "These are the operators of canvas `globalCompositeOperation` and CSS `mask-composite`, plus Level 2's `plus-lighter` and `plus-darker`.",
      "`composite<Composite.SourceOver>` is `sk_over`.",
    ],
    examples: [
      {
        title: "Punch the content out where a mask is",
        lang: "slang",
        code: `import sk;
import sk.blend;

[SkEffect("punch")]
[SkContent]
struct Punch : IEffect
{
    [SkLength] [SkDefault("20")] float radius;

    float4 shade(EffectInput input)
    {
        float hole = saturate(radius - length(input.px - input.size * 0.5));
        return composite<Composite.DestinationOut>(input.content, float4(0.0, 0.0, 0.0, hole));
    }
};`,
      },
    ],
    related: ["i-composite-op", "blend", "sk-over"],
  },
  {
    slug: "i-composite-op",
    name: "ICompositeOp",
    kind: "interface",
    summary: "A compositing operator: premultiplied source and backdrop in, premultiplied result out.",
    since,
    syntax: `interface ICompositeOp
{
    static float4 apply(float4 below, float4 above);
};`,
    related: ["composite"],
  },
  op("clear", "Clear", "nothing.", "Fa 0, Fb 0"),
  op("copy", "Copy", "the source alone.", "Fa 1, Fb 0"),
  op("destination", "Destination", "the backdrop alone.", "Fa 0, Fb 1"),
  op("source-over", "SourceOver", "the source over the backdrop, the default everywhere.", "Fa 1, Fb 1 − αs"),
  op("destination-over", "DestinationOver", "the backdrop over the source.", "Fa 1 − αb, Fb 1"),
  op("source-in", "SourceIn", "the source where the backdrop is.", "Fa αb, Fb 0"),
  op("destination-in", "DestinationIn", "the backdrop where the source is.", "Fa 0, Fb αs"),
  op("source-out", "SourceOut", "the source where the backdrop is not.", "Fa 1 − αb, Fb 0"),
  op("destination-out", "DestinationOut", "the backdrop where the source is not: an eraser.", "Fa 0, Fb 1 − αs"),
  op("source-atop", "SourceAtop", "the source over the backdrop, only inside the backdrop.", "Fa αb, Fb 1 − αs"),
  op(
    "destination-atop",
    "DestinationAtop",
    "the backdrop over the source, only inside the source.",
    "Fa 1 − αb, Fb αs",
  ),
  op("xor", "Xor", "each layer where the other is not.", "Fa 1 − αb, Fb 1 − αs"),
  op("lighter", "Lighter", "the sum of both, colour and alpha unclamped.", "Fa 1, Fb 1", [
    "The engine clamps what `shade` returns; use `PlusLighter` to clamp at each step.",
  ]),
  op("plus-lighter", "PlusLighter", "the sum, colour and alpha at most 1.", "min(1, cs + cb)", [
    "The value of CSS `mix-blend-mode: plus-lighter`, which cross-fades two layers without a dip in the middle.",
  ]),
  op(
    "plus-darker",
    "PlusDarker",
    "a darkening sum: colour max(0, αo − (αs − cs) − (αb − cb)), alpha min(1, αs + αb).",
    "see notes",
    [
      "The draft's printed formula does not leave the backdrop unchanged under a transparent source; this is Core Graphics' `plus-darker` (`1 − ((1 − S) + (1 − D))` for opaque layers) carried to premultiplied alpha, which does.",
    ],
  ),
];

export const blend = section("sk.blend", drafts);
