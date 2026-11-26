import { type Draft, section } from "./shared";

const since = "0.9.0" as const;

const LIGHT = { name: "x", type: "float", description: "Scene light, 1.0 at SDR white; below 0 counts as 0." };

const GLASS = `import sk;
import sk.tone;

[SkEffect("hdr-glass")]
[SkBackdropLinear]
struct HdrGlass : IEffect
{
    [SkLength] [SkBackdropRadius] [SkDefault("16")] float radius;
    [SkDefault("-0.3")] [SkRange(-2.0, 2.0)] float stops;

    float4 shade(EffectInput input)
    {
        float4 light = input.backdrop;
        float3 straight = exposure(light.rgb / max(light.a, 1e-4), stops);
        return float4(softClip(straight, 1.5, 5.0) * light.a, light.a);
    }
};`;

const drafts: Draft[] = [
  {
    slug: "i-tone",
    name: "ITone",
    kind: "interface",
    summary:
      "A tone curve as a value: `float at(float x)`, scene light in, display light out. `Reinhard`, `ReinhardExtended`, `Hable`, `AcesFitted` and `SoftClip` implement it.",
    since,
    syntax: `import sk.tone;

interface ITone
{
    float at(float x);
};`,
    notes: [
      "Apply one to a colour with `perChannel` or `byLuminance`; both specialise at compile time.",
      "Curves take straight light: divide a premultiplied colour by its alpha first.",
    ],
    related: ["tone-per-channel", "reinhard", "aces-fitted"],
  },
  {
    slug: "reinhard",
    name: "Reinhard",
    kind: "struct",
    summary: "`x / (1 + x)`: all of 0..∞ into 0..1, never reaching 1.",
    since,
    syntax: `Reinhard();
float at(float x);`,
    params: [LIGHT],
    returns: "0..1, 0.5 at SDR white.",
    related: ["reinhard-extended", "i-tone"],
  },
  {
    slug: "reinhard-extended",
    name: "ReinhardExtended",
    kind: "struct",
    summary: "Reinhard with a white point, `x (1 + x / white²) / (1 + x)`: `white` maps to exactly 1.",
    since,
    syntax: `ReinhardExtended(float white);
float at(float x);`,
    params: [{ name: "white", type: "float", description: "The light that maps to 1; > 0." }, LIGHT],
    returns: "0 at 0, 1 at `white`, above 1 beyond it.",
    related: ["reinhard", "i-tone"],
  },
  {
    slug: "hable",
    name: "Hable",
    kind: "struct",
    summary:
      "A filmic curve with a toe and a shoulder (the Hable form and its usual constants), scaled so `white` maps to 1.",
    since,
    syntax: `Hable(float white = 11.2);
float at(float x);`,
    params: [
      { name: "white", type: "float", description: "The light that maps to 1; > 0.", optional: true, default: "11.2" },
      LIGHT,
    ],
    returns: "0 at 0, 1 at `white`; beyond it the curve keeps rising slowly, towards about 1.29 for the default white.",
    notes: ["`saturate` the result when the output must stay within 0..1."],
    related: ["aces-fitted", "i-tone"],
  },
  {
    slug: "aces-fitted",
    name: "AcesFitted",
    kind: "struct",
    summary: "A rational fit of the ACES film look (RRT and sRGB ODT) from scene light to SDR display light.",
    since,
    syntax: `AcesFitted();
float at(float x);`,
    params: [LIGHT],
    returns: "0..1; 1 is reached near x = 10.",
    notes: [
      "The fit expects light already scaled by 0.6 relative to ACES' own scale; `exposure(x, -0.737)` first matches it.",
      "Per channel it desaturates bright colours towards white, as film does; `byLuminance` keeps their hue.",
    ],
    related: ["hable", "i-tone"],
  },
  {
    slug: "tone-soft-clip",
    name: "softClip",
    kind: "function",
    summary:
      "Identity up to `knee`, then an exponential roll-off that meets it with slope 1 and approaches `limit` without reaching it.",
    since,
    syntax: `float softClip(float x, float knee, float limit);
vector<float, N> softClip(vector<float, N> x, float knee, float limit);
SoftClip(float knee, float limit);   // the same as an ITone`,
    params: [
      { name: "x", type: "float", description: "Any value; below `knee` every sign passes through." },
      { name: "knee", type: "float", description: "Where the roll-off starts." },
      {
        name: "limit",
        type: "float",
        description: "What the roll-off approaches. With `limit` <= `knee` it is `min(x, limit)`.",
      },
    ],
    returns: "Below `limit`, continuous with a continuous slope.",
    notes: [
      "The curve for HDR output: light up to the knee is untouched, brighter light is compressed into the headroom left, so highlights keep their order instead of flattening at a cut.",
    ],
    examples: [{ title: "Glass over HDR video that keeps its highlights", lang: "slang", code: GLASS }],
    aliases: ["SoftClip"],
    related: ["sk-backdrop-linear", "exposure"],
  },
  {
    slug: "tone-per-channel",
    name: "perChannel",
    kind: "function",
    summary: "A tone curve on each channel, or on BT.709 luminance with the colour scaled to match (`byLuminance`).",
    since,
    syntax: `float3 perChannel<T : ITone>(T curve, float3 rgb);
float3 byLuminance<T : ITone>(T curve, float3 rgb);`,
    params: [
      { name: "curve", type: "ITone", description: "The curve." },
      { name: "rgb", type: "float3", description: "Straight linear light." },
    ],
    notes: [
      "`perChannel` drives bright saturated colours towards white; `byLuminance` keeps hue and saturation, so a channel can end above the curve's range.",
      "`byLuminance` returns black where luminance is 0 or below.",
    ],
    aliases: ["byLuminance"],
    related: ["i-tone", "luminance709"],
  },
  {
    slug: "luminance709",
    name: "luminance709",
    kind: "function",
    summary: "BT.709 luminance of linear light, `dot(rgb, SK_LUMA_709)`: 1.0 at SDR white.",
    since,
    syntax: `static const float3 SK_LUMA_709 = float3(0.2126, 0.7152, 0.0722);
float luminance709(float3 rgb);`,
    params: [{ name: "rgb", type: "float3", description: "Linear light, any sign." }],
    notes: ["On sRGB-encoded values it is not luminance; decode with `sk_frame_to_linear` first."],
    aliases: ["SK_LUMA_709"],
    related: ["tone-per-channel", "sk-luma"],
  },
  {
    slug: "exposure",
    name: "exposure",
    kind: "function",
    summary: "Light scaled by `2^stops`: +1 doubles it, −1 halves it.",
    since,
    syntax: `float exposure(float x, float stops);
vector<float, N> exposure(vector<float, N> x, float stops);`,
    params: [
      { name: "x", type: "float or vector", description: "Linear light, any sign." },
      { name: "stops", type: "float", description: "Any value." },
    ],
    related: ["tone-soft-clip"],
  },
  {
    slug: "scene-to-nits",
    name: "sceneToNits",
    kind: "function",
    summary: "Scene light (1.0 at SDR white) to absolute nits and back, given the nits of SDR white.",
    since,
    syntax: `static const float SK_SCRGB_WHITE_NITS = 80.0;          // Windows scRGB
static const float SK_HDR_REFERENCE_WHITE_NITS = 203.0;  // BT.2408
float sceneToNits(float x, float sdrWhiteNits);
float nitsToScene(float nits, float sdrWhiteNits);       // and vector<float, N> forms`,
    params: [
      { name: "x", type: "float or vector", description: "Scene light, any sign." },
      { name: "sdrWhiteNits", type: "float", description: "Nits of SDR white; > 0." },
    ],
    notes: [
      "The engine does not pass the display's SDR white level to effects; pick the one the content assumes. Video mastered to BT.2408 puts diffuse white at 203 nits.",
    ],
    aliases: ["nitsToScene", "SK_SCRGB_WHITE_NITS", "SK_HDR_REFERENCE_WHITE_NITS"],
    related: ["pq-encode"],
  },
  {
    slug: "pq-encode",
    name: "pqEncode",
    kind: "function",
    summary: "SMPTE ST 2084 (PQ): absolute nits 0..10000 to a signal 0..1, and back with `pqDecode`.",
    since,
    syntax: `vector<float, N> pqEncode(vector<float, N> nits);
vector<float, N> pqDecode(vector<float, N> signal);`,
    params: [
      { name: "nits", type: "vector", description: "0..10000; clamped to that range." },
      { name: "signal", type: "vector", description: "0..1; clamped to that range." },
    ],
    returns: "`pqEncode`: 0..1 (100 nits is 0.508, 1000 nits 0.752). `pqDecode`: 0..10000 nits.",
    notes: ["For a scalar, pass a `float2` and read `.x`."],
    aliases: ["pqDecode"],
    related: ["scene-to-nits", "hlg-encode"],
  },
  {
    slug: "hlg-encode",
    name: "hlgEncode",
    kind: "function",
    summary: "BT.2100 HLG OETF: normalised scene light 0..1 to a signal 0..1, and back with `hlgDecode`.",
    since,
    syntax: `float hlgEncode(float e);
float hlgDecode(float signal);              // and vector<float, N> forms`,
    params: [
      { name: "e", type: "float", description: "Scene light normalised to 0..1; clamped to that range." },
      { name: "signal", type: "float", description: "0..1; clamped to that range." },
    ],
    returns: "0..1; 1/12 maps to 0.5.",
    notes: ["The OETF only: the display-referred OOTF (system gamma) is not applied."],
    aliases: ["hlgDecode"],
    related: ["pq-encode"],
  },
];

export const tone = section("sk.tone", drafts);
