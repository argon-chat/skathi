import { type Draft, type Kind, section } from "./shared";

const since = "0.9.0" as const;

const fn = (
  slug: string,
  name: string,
  summary: string,
  syntax: string,
  extra: Partial<Pick<Draft, "params" | "returns" | "notes" | "examples" | "related" | "aliases">> = {},
  kind: Kind = "function",
): Draft => ({ slug: `sdf-${slug}`, name: `Sdf.${name}`, kind, summary, since, syntax, ...extra });

const P = { name: "p", type: "float2", description: "The point, in pixels from the shape's centre (y down)." };

const BADGE = `import sk;
import sk.sdf;

/// A rounded body with a dot welded to its top-right corner.
float badge(float2 p, float2 extent)
{
    float body = Sdf.roundedBox(p, extent, 10.0);
    float dot = Sdf.circle(p - float2(extent.x - 4.0, -extent.y + 4.0), 7.0);
    return Sdf.smoothUnion(body, dot, 4.0);
}

[SkEffect("sdf-badge")]
struct SdfBadge : IEffect
{
    [SkColor] [SkDefault("#2b6cff")] float4 fill;
    [SkLength] [SkBleed] [SkDefault("12")] float shadow;

    float4 shade(EffectInput input)
    {
        float2 p = input.px - input.size * 0.5;
        float2 extent = input.size * 0.5 - 8.0;
        float d = badge(p, extent);
        float drop = badge(Sdf.translate(p, float2(0.0, 3.0)), extent);
        float4 under = float4(0.0, 0.0, 0.0, 0.35) * Sdf.glow(drop, shadow);
        float4 body = sk_premultiply(fill) * Sdf.coverage(d);
        float4 ring = float4(1.0, 1.0, 1.0, 1.0) * (0.6 * Sdf.stroke(d, 1.5, StrokeAlign.Inside));
        return sk_over(sk_over(under, body), ring);
    }
};`;

const drafts: Draft[] = [
  {
    slug: "sdf",
    name: "Sdf",
    kind: "type",
    summary:
      "2D signed distance fields in pixels: shapes, ways to combine and repeat them, and antialiased fills, strokes and glows from a distance.",
    since,
    syntax: `import sk.sdf;

float d = Sdf.roundedBox(p, halfSize, 12.0);   // < 0 inside, 0 on the edge, > 0 outside
float a = Sdf.coverage(d);                      // 0..1, antialiased`,
    notes: [
      "Distances follow `input.sd`: device pixels, negative inside, y down. Shapes sit at the origin; move the point (`p - centre`), not the shape.",
      "Every shape is exact, the true Euclidean distance inside and out, unless its page says otherwise. Combinations are exact outside a union and inside an intersection, which keeps coverage and strokes right at the edge.",
      "Everything is a plain function in the `Sdf` namespace: the same code on every backend, and only what an effect calls reaches its shader.",
    ],
    examples: [{ title: "A badge with a dot, an inner stroke and a soft shadow", lang: "slang", code: BADGE }],
    related: ["i-sdf", "sdf-coverage", "effect-input-aa", "sk-quad-sdf"],
  },
  {
    slug: "i-sdf",
    name: "ISdf",
    kind: "interface",
    summary:
      "A whole distance field as a value, for the helpers that sample it more than once: `gradient`, `normal`, `scaled`.",
    since,
    syntax: `interface ISdf
{
    float distance(float2 p);
};`,
    notes: [
      "Shapes and combinators are functions, not `ISdf` types: a composed shape as nested generic types compiled to about twice the code of the same calls, and read worse. Wrap a composed shape in one struct when a helper needs it.",
    ],
    examples: [
      {
        title: "Light a shape by its normal",
        lang: "slang",
        code: `import sk;
import sk.sdf;

struct Pill : ISdf
{
    float2 halfSize;
    float distance(float2 p) { return Sdf.roundedBox(p, halfSize, halfSize.y); }
};

[SkEffect("lit-pill")]
struct LitPill : IEffect
{
    float4 shade(EffectInput input)
    {
        Pill pill = { input.size * 0.5 - 1.0 };
        float2 p = input.px - input.size * 0.5;
        float2 n = Sdf.normal(pill, p);
        float light = 0.6 + 0.4 * dot(n, normalize(float2(-1.0, -1.0)));
        return float4(light, light, light, 1.0) * Sdf.coverage(pill.distance(p));
    }
};`,
      },
    ],
    related: ["sdf-gradient", "sdf-scaled"],
  },
  {
    slug: "stroke-align",
    name: "StrokeAlign",
    kind: "enum",
    summary: "Where `Sdf.stroke` puts its band: `Inside`, `Center` or `Outside` the edge.",
    since,
    syntax: `enum StrokeAlign { Inside, Center, Outside }`,
    related: ["sdf-stroke"],
  },

  fn("circle", "circle", "A circle of `radius` px.", `float Sdf.circle(float2 p, float radius);`, {
    params: [P, { name: "radius", type: "float", description: "Pixels." }],
  }),
  fn(
    "box",
    "box",
    "An axis-aligned box `halfSize` px from centre to edge.",
    `float Sdf.box(float2 p, float2 halfSize);`,
    {
      params: [P, { name: "halfSize", type: "float2", description: "Half the width and height, pixels." }],
    },
  ),
  fn(
    "rounded-box",
    "roundedBox",
    "A box with rounded corners, one radius for all or one per corner in CSS order.",
    `float Sdf.roundedBox(float2 p, float2 halfSize, float radius);
float Sdf.roundedBox(float2 p, float2 halfSize, float4 radii);   // top-left, top-right, bottom-right, bottom-left`,
    {
      params: [
        P,
        { name: "halfSize", type: "float2", description: "Half the width and height, pixels." },
        { name: "radii", type: "float4", description: "Corner radii, pixels, in `border-radius` order (y down)." },
      ],
      notes: [
        "Each radius is clamped to 0..min(halfSize): a pill is `Sdf.roundedBox(p, h, h.y)`.",
        "Unlike `sk_quad_sdf` it takes its corners in CSS order directly and sits at the origin.",
      ],
      related: ["sk-quad-sdf", "sdf-rounded-box-gradient"],
    },
  ),
  fn(
    "ellipse",
    "ellipse",
    "An axis-aligned ellipse of semi-axes `radii` px.",
    `float Sdf.ellipse(float2 p, float2 radii);`,
    {
      params: [P, { name: "radii", type: "float2", description: "The semi-axes, pixels." }],
      notes: [
        "Not closed form: six fixed steps towards the closest point. Within 1e-3 px of exact up to an aspect of 10:1 and 4000 px across; at 40:1 about 0.1 px. Equal axes are a circle exactly.",
      ],
    },
  ),
  fn(
    "segment",
    "segment",
    "The distance to the segment from `a` to `b`: 0 on it, never negative.",
    `float Sdf.segment(float2 p, float2 a, float2 b);`,
    {
      notes: ["A segment of zero length is its point."],
      related: ["sdf-capsule"],
    },
  ),
  fn(
    "capsule",
    "capsule",
    "A capsule: the segment from `a` to `b` thickened by `radius` px each side, with round ends.",
    `float Sdf.capsule(float2 p, float2 a, float2 b, float radius);`,
    { related: ["sdf-segment"] },
  ),
  fn(
    "triangle",
    "triangle",
    "The triangle with corners `a`, `b` and `c`, in either winding.",
    `float Sdf.triangle(float2 p, float2 a, float2 b, float2 c);`,
  ),
  fn(
    "polygon",
    "polygon",
    "A regular polygon of `sides`, corners `radius` px from the centre, one corner straight up.",
    `float Sdf.polygon(float2 p, float radius, int sides);`,
    {
      params: [
        P,
        { name: "radius", type: "float", description: "Centre to corner, pixels." },
        { name: "sides", type: "int", description: "3 or more." },
      ],
      notes: ["A triangle points up and a hexagon stands on a corner. Rotate the point for another orientation."],
    },
  ),
  fn(
    "star",
    "star",
    "A star of `points` tips `radius` px out and inner corners `innerRadius` px out, one tip straight up.",
    `float Sdf.star(float2 p, float radius, int points, float innerRadius);`,
    {
      params: [
        P,
        { name: "radius", type: "float", description: "Centre to tip, pixels." },
        { name: "points", type: "int", description: "3 or more." },
        { name: "innerRadius", type: "float", description: "Centre to inner corner, pixels, up to `radius`." },
      ],
      notes: [
        "At an inner radius of `radius · cos(π / points)` the inner corners lie on the edges of `Sdf.polygon(p, radius, points)`, and the star is that polygon.",
      ],
    },
  ),
  fn(
    "arc",
    "arc",
    "An arc of a circle, `halfAngle` radians either side of straight up, `thickness` px wide with round ends.",
    `float Sdf.arc(float2 p, float radius, float halfAngle, float thickness);`,
    {
      params: [
        P,
        { name: "radius", type: "float", description: "To the middle of the band, pixels." },
        { name: "halfAngle", type: "float", description: "Radians, 0..π; π is a whole ring." },
        { name: "thickness", type: "float", description: "Band width, pixels." },
      ],
      notes: [
        "A progress ring: `Sdf.arc(Sdf.rotate(p, progress * SK_PI), r, progress * SK_PI, w)` fills clockwise from the top.",
      ],
      related: ["sdf-ring", "sdf-pie"],
    },
  ),
  fn(
    "ring",
    "ring",
    "A ring (annulus): `radius` px to the middle of its band, `thickness` px wide.",
    `float Sdf.ring(float2 p, float radius, float thickness);`,
    {
      related: ["sdf-arc", "sdf-outline"],
    },
  ),
  fn(
    "pie",
    "pie",
    "A pie slice of `radius` px, `halfAngle` radians either side of straight up.",
    `float Sdf.pie(float2 p, float radius, float halfAngle);`,
    {
      related: ["sdf-arc"],
    },
  ),
  fn(
    "half-plane",
    "halfPlane",
    "Everything behind the line through `point` facing `normal`: negative on the side `normal` points away from.",
    `float Sdf.halfPlane(float2 p, float2 point, float2 normal);`,
    { notes: ["`normal` need not be unit length. Intersect half-planes for any convex polygon."] },
  ),
  fn(
    "cross",
    "cross",
    "A plus sign: two bars `halfLength` px from the centre to their ends, `halfWidth` px half thick.",
    `float Sdf.cross(float2 p, float halfLength, float halfWidth);`,
    {
      notes: ["Exact inside too: its inner corners are measured, not the minimum of two boxes."],
    },
  ),

  fn("union", "union", "Both shapes: `min(a, b)`.", `float Sdf.union(float a, float b);`, {
    notes: ["Exact outside both shapes; inside where they overlap the value is a bound, not the distance."],
    related: ["sdf-smooth-union"],
  }),
  fn(
    "intersection",
    "intersection",
    "Where both shapes are: `max(a, b)`.",
    `float Sdf.intersection(float a, float b);`,
  ),
  fn(
    "subtraction",
    "subtraction",
    "`a` with `b` cut out of it: `max(a, −b)`.",
    `float Sdf.subtraction(float a, float b);`,
  ),
  fn("xor", "xor", "Where exactly one of the shapes is.", `float Sdf.xor(float a, float b);`),
  fn(
    "smooth-union",
    "smoothUnion",
    "A union with the seam filleted over `radius` px; `smoothIntersection` and `smoothSubtraction` round theirs the same way.",
    `float Sdf.smoothUnion(float a, float b, float radius);
float Sdf.smoothIntersection(float a, float b, float radius);
float Sdf.smoothSubtraction(float a, float b, float radius);   // a minus b`,
    {
      notes: [
        "The quadratic smooth minimum: the shapes blend where their distances are within `radius` of each other, and the seam moves by at most `radius / 4`. A radius of 0 is the sharp operation.",
      ],
      aliases: ["Sdf.smoothIntersection", "Sdf.smoothSubtraction"],
    },
  ),
  fn(
    "inflate",
    "inflate",
    "The shape grown by `amount` px (negative shrinks); corners round as they grow.",
    `float Sdf.inflate(float d, float amount);`,
    {
      notes: ["Growing is exact outside the shape and shrinking inside it; a negative amount is an inset."],
    },
  ),
  fn(
    "outline",
    "outline",
    "A band `width` px wide centred on the edge, as a shape of its own.",
    `float Sdf.outline(float d, float width);`,
    {
      notes: ["Feed it to any other operation; to just draw a stroke, `Sdf.stroke` is shorter."],
      related: ["sdf-stroke", "sdf-ring"],
    },
  ),

  fn(
    "translate",
    "translate",
    "The point as a shape moved by `offset` px sees it: `p − offset`.",
    `float2 Sdf.translate(float2 p, float2 offset);`,
  ),
  fn(
    "rotate",
    "rotate",
    "The point as a shape turned by `radians` (clockwise on screen) sees it.",
    `float2 Sdf.rotate(float2 p, float radians);`,
    {
      notes: [
        "Rotation keeps distances, so the shape's distance needs no correction. `sk_rotate` turns a point the other way; this one turns the shape.",
      ],
      related: ["sk-rotate"],
    },
  ),
  fn(
    "mirror",
    "mirror",
    "The point folded across the line through the origin with `normal`: a shape on one side appears mirrored on the other.",
    `float2 Sdf.mirror(float2 p, float2 normal);`,
    { notes: ["`Sdf.mirror(p, float2(1.0, 0.0))` mirrors left and right. Exact while the shape stays on its side."] },
  ),
  fn(
    "repeat",
    "repeat",
    "The point in its cell of a grid of `spacing` px, endless or `count` cells centred on the origin.",
    `float2 Sdf.repeat(float2 p, float2 spacing);
float2 Sdf.repeat(float2 p, float2 spacing, int2 count);`,
    {
      notes: [
        "Exact while the shape stays inside its cell; a shape that spills over is cut at the cell edge.",
        "With `count`, a point outside the grid sees the nearest copy at the edge, so the field stays right around it.",
      ],
      related: ["sdf-polar-repeat"],
    },
  ),
  fn(
    "polar-repeat",
    "polarRepeat",
    "The point turned into the nearest of `count` equal sectors around the origin, the first straight up.",
    `float2 Sdf.polarRepeat(float2 p, int count);`,
    {
      notes: [
        "Draw one copy above the origin (`Sdf.circle(q - float2(0.0, -r), 4.0)`) to get `count` of them round it.",
      ],
      examples: [
        {
          title: "A ring of dots",
          lang: "slang",
          code: `import sk;
import sk.sdf;

[SkEffect("dot-ring")]
struct DotRing : IEffect
{
    [SkLength] [SkDefault("4")] float dot;

    float4 shade(EffectInput input)
    {
        float2 p = input.px - input.size * 0.5;
        float r = min(input.size.x, input.size.y) * 0.5 - dot - 1.0;
        float2 q = Sdf.polarRepeat(p, 12);
        float d = Sdf.circle(q - float2(0.0, -r), dot);
        return float4(1.0, 1.0, 1.0, 1.0) * Sdf.coverage(d);
    }
};`,
        },
      ],
    },
  ),
  fn(
    "scaled",
    "scaled",
    "A field scaled by `factor` about the origin, its distance scaled with it so it stays in pixels.",
    `float Sdf.scaled<S : ISdf>(S shape, float2 p, float factor);`,
    { notes: ["Uniform only: a non-uniform scale does not keep distances."], related: ["i-sdf"] },
  ),

  fn(
    "coverage",
    "coverage",
    "Antialiased fill, 0..1: one pixel of feather centred on the edge.",
    `float Sdf.coverage(float d);`,
    {
      notes: [
        "The same ramp as `input.aa` and the engine's own coverage, so a shape drawn with it lines up with the element's edge.",
      ],
      related: ["effect-input-aa", "sdf-stroke"],
    },
  ),
  fn(
    "stroke",
    "stroke",
    "Antialiased coverage of a stroke `width` px wide inside, centred on or outside the edge.",
    `float Sdf.stroke(float d, float width, StrokeAlign align = StrokeAlign.Center);`,
    {
      params: [
        { name: "d", type: "float", description: "The distance." },
        { name: "width", type: "float", description: "Pixels." },
        {
          name: "align",
          type: "StrokeAlign",
          description: "`Inside`, `Center` or `Outside`.",
          optional: true,
          default: "Center",
        },
      ],
      notes: ["An inside stroke with the same `d` as the fill sits flush with it, like a CSS `border`."],
      related: ["stroke-align", "sdf-outline"],
    },
  ),
  fn(
    "feather",
    "feather",
    "A soft edge: 1 inside to 0 outside across `width` px centred on the edge, smoothstep-shaped.",
    `float Sdf.feather(float d, float width);`,
    {
      related: ["sdf-coverage", "sdf-glow"],
    },
  ),
  fn(
    "glow",
    "glow",
    "A glow outside the shape: 1 on and inside the edge, falling quadratically to exactly 0 at `radius` px.",
    `float Sdf.glow(float d, float radius);`,
    {
      notes: ["It reaches 0 within `radius`, so a `[SkBleed]` of the same length holds all of it."],
      related: ["sk-bleed"],
    },
  ),

  fn(
    "gradient",
    "gradient",
    "The gradient of any field by central differences `h` px apart; `normal` is it normalised.",
    `float2 Sdf.gradient<S : ISdf>(S field, float2 p, float h = 0.5);
float2 Sdf.normal<S : ISdf>(S field, float2 p, float h = 0.5);`,
    {
      notes: [
        "Four evaluations of the field. For an exact field the gradient has length 1 and points away from the nearest edge.",
        "`normal` returns (0, 0) where the gradient vanishes (the centre of a circle).",
        "Prefer the exact `circleGradient`, `boxGradient` and `roundedBoxGradient` for those shapes.",
      ],
      aliases: ["Sdf.normal"],
      related: ["i-sdf", "sdf-rounded-box-gradient"],
    },
  ),
  fn(
    "rounded-box-gradient",
    "roundedBoxGradient",
    "The exact gradients of `circle`, `box` and `roundedBox`, without sampling the field.",
    `float2 Sdf.circleGradient(float2 p);
float2 Sdf.boxGradient(float2 p, float2 halfSize);
float2 Sdf.roundedBoxGradient(float2 p, float2 halfSize, float4 radii);`,
    {
      notes: ["Inside a box the gradient points to the nearest side; on a diagonal where two sides tie it picks one."],
      aliases: ["Sdf.circleGradient", "Sdf.boxGradient"],
      related: ["sdf-gradient"],
    },
  ),
];

export const sdf = section("sk.sdf", drafts);
