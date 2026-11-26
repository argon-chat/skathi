import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const src = {
  name: "src",
  type: "S : ISampler",
  description:
    "What to read: `Backdrop(input)`, `RawBackdrop(input)`, `Content(input)`, `History(input)` or `Image(texture)`.",
};
const px = {
  name: "px",
  type: "float2",
  description: "Where, in the source's pixels: `input.px` for the element's samplers.",
};

const drafts: Draft[] = [
  {
    slug: "i-sampler",
    name: "ISampler",
    kind: "interface",
    summary:
      "What a kernel reads: premultiplied RGBA at a point in its own pixel space, bilinear. One struct per engine source and one for textures.",
    since,
    syntax: `import sk.sample;

interface ISampler
{
    float4 at(float2 px);
    float2 texelOffset();
}

struct Backdrop : ISampler     { __init(EffectInput input); }  // blurred to [SkBackdropRadius]
struct RawBackdrop : ISampler  { __init(EffectInput input); }  // unblurred, one texel per pixel
struct PageBackdrop : ISampler { __init(EffectInput input); }  // the page without native layers
struct Content : ISampler      { __init(EffectInput input); }  // [SkContent]
struct History : ISampler      { __init(EffectInput input); }  // [SkHistory], raw values
struct Image : ISampler        { __init(SkTexture texture, bool repeat = false); }`,
    notes: [
      "Element samplers take device pixels of the element, `input.px` space, whose texel centres sit at half pixels; `Image` takes texels of the image, 0 to its size.",
      "Every kernel is linear in premultiplied colour: transparent pixels contribute nothing, and a fully transparent neighbourhood gives exactly 0, never a NaN.",
      "`History` holds whatever the effect stored, not premultiplied colour; the kernels filter it the same way.",
      "A read past `[SkBackdropReach]` is clamped to the edge of what the engine copied, so a blur at the rim smears the edge texel outwards instead of fading to black. Give the reach from the kernel's `…Reach` helper to read the real pixels.",
    ],
    examples: [
      {
        title: "One kernel over two sources",
        lang: "slang",
        code: `import sk;
import sk.sample;

[SkEffect("soft-copy")]
[SkContent]
struct SoftCopy : IEffect
{
    [SkLength] [SkBackdropReach] [SkDefault("6")] float reach;

    float4 shade(EffectInput input)
    {
        float4 under = gaussian<Gaussian9>(RawBackdrop(input), input.px);
        float4 over = gaussian<Gaussian5>(Content(input), input.px);
        return sk_over(under, over);
    }
};`,
      },
    ],
    aliases: ["Backdrop", "RawBackdrop", "PageBackdrop", "Content", "History", "Image"],
    related: ["gaussian", "uv-to-px", "sk-backdrop-reach"],
  },
  {
    slug: "uv-to-px",
    name: "uvToPx",
    kind: "function",
    summary:
      "A window UV, the space of `input.uv` and `backdropAt`, as a point in the element's pixels, the space kernels take.",
    since,
    syntax: `float2 uvToPx(EffectInput input, float2 uv);`,
    params: [
      { name: "input", type: "EffectInput", description: "The effect's input." },
      { name: "uv", type: "float2", description: "A window UV." },
    ],
    returns: "The same point in device pixels of the element: `uvToPx(input, input.uv)` is `input.px`.",
    examples: [
      {
        title: "Blur around a point given as a UV",
        lang: "slang",
        code: `float2 at = uvToPx(input, input.uv + float2(0.01, 0.0));
return gaussian<Gaussian5>(RawBackdrop(input), at);`,
      },
    ],
    related: ["i-sampler"],
  },
  {
    slug: "gaussian",
    name: "gaussian",
    kind: "function",
    summary:
      "A small Gaussian blur of any sampler, 5, 9 or 13 taps wide, with neighbouring taps merged into one bilinear read.",
    since,
    syntax: `float4 gaussian<K : IGaussian, S : ISampler>(S src, float2 px, float scale = 1.0);
float4 gaussianLine<K : IGaussian, S : ISampler>(S src, float2 px, float2 direction, float scale = 1.0);
float gaussianReach<K : IGaussian>(float scale = 1.0);

struct Gaussian5 : IGaussian;   // 1 4 6 4 1, sigma 1 px, 3 reads per axis
struct Gaussian9 : IGaussian;   // binomial 8, sigma 1.41 px, 5 reads per axis
struct Gaussian13 : IGaussian;  // binomial 12, sigma 1.73 px, 7 reads per axis`,
    params: [
      src,
      px,
      {
        name: "scale",
        type: "float",
        description: "Spreads the reads: 2 doubles the width with the same reads, sparser.",
        optional: true,
        default: "1.0",
      },
      { name: "direction", type: "float2", description: "For `gaussianLine`: a unit direction in pixels." },
    ],
    returns: "Premultiplied RGBA, the weighted mean of the source; `gaussianReach` returns the reach in pixels.",
    notes: [
      "Cost: 9, 25 or 49 reads for `Gaussian5`, `Gaussian9` and `Gaussian13`; `gaussianLine` is 3, 5 or 7. `Gaussian13` takes fxc about 0.8 s to compile, the others under 0.4 s.",
      "Reach: `gaussianReach<K>(scale)`, 3, 5 or 7 px at scale 1: 2, 4 or 6 texels of kernel plus one for the bilinear footprint.",
      "Exact on texel centres at scale 1, which `input.px` always is. Off-centre reads and `scale` above 1 add the bilinear filter's own small blur.",
      "`gaussianLine` along an oblique direction is a streak, not the blur rotated; two lines at right angles are not the 2D blur.",
      "For a blur wider than a few pixels use `[SkBackdropRadius]`: the engine blurs the backdrop once through a downsampled pyramid, which costs the same at any radius. These kernels are for what the pyramid cannot do: a blur of the content or the history, a few pixels of softening on top of the pyramid, or a blur that changes from pixel to pixel.",
    ],
    examples: [
      {
        title: "A blur that grows towards the bottom of the element",
        lang: "slang",
        code: `import sk;
import sk.sample;

[SkEffect("tilt-blur")]
struct TiltBlur : IEffect
{
    [SkLength] [SkBackdropReach] [SkDefault("15")] float reach;

    float4 shade(EffectInput input)
    {
        float scale = 1.0 + 1.5 * input.local.y;   // gaussianReach<Gaussian13>(2.5) is 15 px
        return gaussian<Gaussian13>(RawBackdrop(input), input.px, scale);
    }
};`,
      },
    ],
    aliases: ["gaussianLine", "gaussianReach", "IGaussian", "Gaussian5", "Gaussian9", "Gaussian13"],
    related: ["box", "disc", "sk-backdrop-radius", "sk-backdrop-reach"],
  },
  {
    slug: "box",
    name: "box",
    kind: "function",
    summary: "A box (mean) blur over a square of half-width `radius` pixels, from 4 × 4 bilinear reads.",
    since,
    syntax: `float4 box<S : ISampler>(S src, float2 px, float radius);
float boxReach(float radius);`,
    params: [src, px, { name: "radius", type: "float", description: "Half the square's side, in pixels." }],
    returns: "Premultiplied RGBA; `boxReach` is `0.75 × radius + 1` px.",
    notes: [
      "Cost: 16 reads at any radius.",
      "Exact at `radius` 2; beyond, each read stands for a larger square, and past about 6 px the result turns grainy. Use the pyramid for wide blurs.",
    ],
    examples: [
      {
        title: "A 4 px box blur of the backdrop",
        lang: "slang",
        code: `return box(RawBackdrop(input), input.px, 4.0) * input.coverage;`,
      },
    ],
    aliases: ["boxReach"],
    related: ["gaussian", "disc"],
  },
  {
    slug: "disc",
    name: "disc",
    kind: "function",
    summary: "A disc (bokeh) blur: `N` reads on a golden-angle spiral, evenly covering a disc of `radius` pixels.",
    since,
    syntax: `float4 disc<let N : int, S : ISampler>(S src, float2 px, float radius);
float discReach(float radius);`,
    params: [
      { name: "N", type: "int", description: "Reads: 16 to 32 hide the spiral on a smooth source." },
      src,
      px,
      { name: "radius", type: "float", description: "The disc's radius, in pixels." },
    ],
    returns: "Premultiplied RGBA; `discReach` is `radius + 1` px.",
    notes: [
      "Cost: `N` reads; `disc<32>` takes fxc about 0.5 s to compile.",
      "Bright points spread into discs rather than Gaussian blobs: the look of a camera lens out of focus.",
    ],
    examples: [
      {
        title: "Out of focus",
        lang: "slang",
        code: `import sk;
import sk.sample;

[SkEffect("bokeh")]
struct Bokeh : IEffect
{
    [SkLength] [SkDefault("8")] float radius;
    [SkLength] [SkBackdropReach] [SkDefault("9")] float reach;   // discReach(radius)

    float4 shade(EffectInput input)
    {
        return disc<24>(RawBackdrop(input), input.px, radius) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["discReach"],
    related: ["gaussian", "box"],
  },
  {
    slug: "directional",
    name: "directional",
    kind: "function",
    summary: "A motion blur: `N` even reads along a segment of `delta` pixels centred on the point.",
    since,
    syntax: `float4 directional<let N : int, S : ISampler>(S src, float2 px, float2 delta);
float directionalReach(float2 delta);`,
    params: [
      { name: "N", type: "int", description: "Reads along the segment." },
      src,
      px,
      { name: "delta", type: "float2", description: "The whole segment, in pixels: half of it each way." },
    ],
    returns: "Premultiplied RGBA; `directionalReach` is `|delta| / 2 + 1` px.",
    notes: ["Cost: `N` reads. Keep the spacing `|delta| / N` near 1 px or below for a smooth streak."],
    examples: [
      {
        title: "A horizontal whoosh",
        lang: "slang",
        code: `return directional<12>(RawBackdrop(input), input.px, float2(12.0, 0.0));`,
      },
    ],
    aliases: ["directionalReach"],
    related: ["radial", "gaussian"],
  },
  {
    slug: "radial",
    name: "radial",
    kind: "function",
    summary: "A zoom blur: `N` reads from the point towards `center`, `amount` of the way there.",
    since,
    syntax: `float4 radial<let N : int, S : ISampler>(S src, float2 px, float2 center, float amount);
float radialReach(float2 center, float amount, float2 size);`,
    params: [
      { name: "N", type: "int", description: "Reads along the ray." },
      src,
      px,
      { name: "center", type: "float2", description: "The point the blur streaks towards, in pixels." },
      { name: "amount", type: "float", description: "0 is no blur, 0.1 streaks a tenth of the way to the centre." },
    ],
    returns:
      "Premultiplied RGBA; `radialReach` is how far outside an element of `size` the reads go: 0 when `center` is inside it.",
    notes: ["Cost: `N` reads."],
    examples: [
      {
        title: "A zoom towards the element's centre",
        lang: "slang",
        code: `return radial<16>(RawBackdrop(input), input.px, input.size * 0.5, 0.12);`,
      },
    ],
    aliases: ["radialReach"],
    related: ["directional"],
  },
  {
    slug: "chromatic",
    name: "chromatic",
    kind: "function",
    summary:
      "Chromatic aberration: red read shifted one way, blue the other, green in place; a fixed shift or one growing from a centre.",
    since,
    syntax: `float4 chromatic<S : ISampler>(S src, float2 px, float2 shift);
float4 chromaticRadial<S : ISampler>(S src, float2 px, float2 center, float amount);
float chromaticReach(float2 shift);`,
    params: [
      src,
      px,
      { name: "shift", type: "float2", description: "Red's offset in pixels; blue gets the opposite." },
      {
        name: "center, amount",
        type: "float2, float",
        description: "For `chromaticRadial`: the shift is `(px - center) * amount`.",
      },
    ],
    returns:
      "Premultiplied RGBA. Alpha is the largest of the three reads' alphas, so a fringe keeps its colour over transparency.",
    notes: ["Cost: 3 reads. Reach: `chromaticReach(shift)` is `|shift| + 1` px."],
    examples: [
      {
        title: "A lens fringe",
        lang: "slang",
        code: `return chromaticRadial(RawBackdrop(input), input.px, input.size * 0.5, 0.02);`,
      },
    ],
    aliases: ["chromaticRadial", "chromaticReach"],
    related: ["dispersion"],
  },
  {
    slug: "dispersion",
    name: "dispersion",
    kind: "function",
    summary:
      "Dispersion: a read displaced by `offset`, with red bent `1 + spread` times as far and blue `1 − spread`, as glass bends short wavelengths more.",
    since,
    syntax: `float4 dispersion<S : ISampler>(S src, float2 px, float2 offset, float spread);
float dispersionReach(float2 offset, float spread);`,
    params: [
      src,
      px,
      {
        name: "offset",
        type: "float2",
        description: "The displacement of green, in pixels: `refractionOffset` from `sk.light`.",
      },
      { name: "spread", type: "float", description: "The relative difference between red and blue: 0.1 is subtle." },
    ],
    returns: "Premultiplied RGBA; alpha the largest of the three reads'.",
    notes: ["Cost: 3 reads. Reach: `dispersionReach(offset, spread)` is `|offset| × (1 + spread) + 1` px."],
    examples: [
      {
        title: "A rainbow rim from the element's edge",
        lang: "slang",
        code: `float2 inward = -screenGradient(input.sd);   // sk.light
float rim = saturate(1.0 + input.sd / 10.0);
return dispersion(RawBackdrop(input), input.px, inward * 8.0 * rim, 0.15);`,
      },
    ],
    aliases: ["dispersionReach"],
    related: ["chromatic", "displace", "refraction-offset"],
  },
  {
    slug: "displace",
    name: "displace",
    kind: "function",
    summary:
      "The source read at a displaced point: by an offset in pixels, or by a normal's tilt times a depth, as a normal-mapped pane of glass bends it.",
    since,
    syntax: `float4 displace<S : ISampler>(S src, float2 px, float2 offset);
float4 displace<S : ISampler>(S src, float2 px, float3 normal, float depth);`,
    params: [
      src,
      px,
      {
        name: "offset",
        type: "float2",
        description: "Pixels to move the read by: a `curl` field, `refractionOffset`.",
      },
      {
        name: "normal, depth",
        type: "float3, float",
        description: "A unit normal (z towards the viewer) and the pixels a full tilt moves the read.",
      },
    ],
    returns: "Premultiplied RGBA.",
    notes: ["Cost: 1 read. Reach: the largest offset plus 1 px; for the normal form, `depth + 1`."],
    examples: [
      {
        title: "Ripples from a noise",
        lang: "slang",
        code: `import sk;
import sk.noise;
import sk.sample;

[SkEffect("heat-haze")]
struct HeatHaze : IEffect
{
    [SkLength] [SkBackdropReach] [SkDefault("5")] float reach;
    [SkAnimated] [SkDefault("1")] float speed;

    float4 shade(EffectInput input)
    {
        float3 n = Simplex.derivatives(input.px / 30.0 + float2(0.0, input.time * speed));
        return displace(RawBackdrop(input), input.px, n.yz * 2.0) * input.coverage;
    }
};`,
      },
    ],
    related: ["dispersion", "refraction-offset", "curl"],
  },
  {
    slug: "catmull-rom",
    name: "catmullRom",
    kind: "function",
    summary:
      "Bicubic resampling: `catmullRom` is sharp (9 bilinear reads), `bspline` smooth and never overshoots (4 reads).",
    since,
    syntax: `float4 catmullRom<S : ISampler>(S src, float2 px);
float4 bspline<S : ISampler>(S src, float2 px);`,
    params: [
      src,
      {
        name: "px",
        type: "float2",
        description: "Any point, not only a texel centre: the use is resampling between texels.",
      },
    ],
    returns: "Premultiplied RGBA. Catmull-Rom can overshoot near edges; its negative results are clamped to 0.",
    notes: [
      "Cost: 9 and 4 reads. Reach: 2 px.",
      "Worth it on sources with one texel per pixel: `RawBackdrop`, `Content`, `History`, `Image`. The blurred `Backdrop` comes from a smaller pyramid level, so a bicubic read of it adds nothing a bilinear one lacks.",
      "Catmull-Rom on a texel centre returns that texel exactly; B-spline softens it slightly by design.",
    ],
    examples: [
      {
        title: "An image scaled up sharply",
        lang: "slang",
        code: `import sk;
import sk.sample;

[SkEffect("upscale")]
struct Upscale : IEffect
{
    SkTexture image;

    float4 shade(EffectInput input)
    {
        float2 texel = input.local * float2(64.0, 64.0);   // a 64 x 64 image across the element
        return catmullRom(Image(image), texel) * input.coverage;
    }
};`,
      },
    ],
    aliases: ["bspline"],
    related: ["sharpen", "i-sampler"],
  },
  {
    slug: "sharpen",
    name: "sharpen",
    kind: "function",
    summary: "Unsharp masking: each pixel pushed away from its 3 × 3 tent blur by `amount`.",
    since,
    syntax: `float4 sharpen<S : ISampler>(S src, float2 px, float amount);`,
    params: [src, px, { name: "amount", type: "float", description: "0.5 is subtle, 2 strong." }],
    returns: "Premultiplied RGBA; negative results are clamped to 0.",
    notes: [
      "Cost: 5 reads (the tent from four bilinear reads). Reach: 2 px.",
      "It amplifies noise as much as detail: dithered gradients get grainier.",
    ],
    examples: [
      {
        title: "Crisper content",
        lang: "slang",
        code: `return sharpen(RawBackdrop(input), input.px, 0.8);`,
      },
    ],
    related: ["catmull-rom", "gaussian"],
  },
  {
    slug: "pixelate",
    name: "pixelate",
    kind: "function",
    summary:
      "A mosaic: each `cell`-pixel square shows the colour at its centre. `cellCenter` gives that centre for reads of your own.",
    since,
    syntax: `float4 pixelate<S : ISampler>(S src, float2 px, float2 cell);
float2 cellCenter(float2 px, float2 cell);`,
    params: [src, px, { name: "cell", type: "float2", description: "The square's size, in pixels." }],
    returns: "Premultiplied RGBA.",
    notes: ["Cost: 1 read. Reach: half a cell plus 1 px."],
    examples: [
      {
        title: "A 12 px mosaic",
        lang: "slang",
        code: `return pixelate(RawBackdrop(input), input.px, float2(12.0, 12.0)) * input.coverage;`,
      },
    ],
    aliases: ["cellCenter"],
    related: ["mirror-repeat"],
  },
  {
    slug: "mirror-repeat",
    name: "mirrorRepeat",
    kind: "function",
    summary:
      "Point folding for mirrored effects: `mirrorRepeat` folds into a rectangle like a mirrored-repeat sampler, `kaleidoscope` into one wedge of `segments`.",
    since,
    syntax: `float2 mirrorRepeat(float2 p, float2 size);
float2 kaleidoscope(float2 p, float2 center, int segments, float rotation = 0.0);`,
    params: [
      { name: "p", type: "float2", description: "The point, in pixels." },
      { name: "size", type: "float2", description: "The rectangle to fold into, from the origin." },
      {
        name: "center, segments, rotation",
        type: "float2, int, float",
        description: "The kaleidoscope's centre, its count of mirrored wedges, and where the first starts, in radians.",
      },
    ],
    returns: "The folded point: read the source there.",
    notes: [
      "No reads of their own: pass the result to `src.at` or a kernel.",
      "`kaleidoscope` returns the centre itself for the centre, on every backend.",
    ],
    examples: [
      {
        title: "A six-fold kaleidoscope of the content",
        lang: "slang",
        code: `import sk;
import sk.sample;

[SkEffect("kaleido")]
[SkContent]
struct Kaleido : IEffect
{
    float4 shade(EffectInput input)
    {
        float2 p = kaleidoscope(input.px, input.size * 0.5, 6);
        return Content(input).at(p);
    }
};`,
      },
    ],
    aliases: ["kaleidoscope"],
    related: ["pixelate"],
  },
];

export const sample = section("sk.sample", drafts);
