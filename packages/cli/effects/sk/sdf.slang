// sk.sdf — 2D signed distance fields, in pixels.
//
//     import sk;
//     import sk.sdf;
//
//     float2 p = input.px - input.size * 0.5;                // centred, device pixels
//     float d = Sdf.roundedBox(p, float2(40.0, 16.0), 16.0);
//     d = Sdf.smoothUnion(d, Sdf.circle(p - float2(36.0, -14.0), 9.0), 6.0);
//     float fill = Sdf.coverage(d);
//     float ring = Sdf.stroke(d, 2.0, StrokeAlign.Inside);
//
// Distances follow `input.sd`: device pixels, negative inside, 0 on the edge, positive outside, and
// y grows downwards. Shapes sit at the origin, so move the point, not the shape (`p - centre`).
// Every shape is exact (the true Euclidean distance inside and out) unless its notes say
// otherwise; the combinators are exact outside a union and inside an intersection only, as for
// any min/max of fields, which still makes coverage and strokes right at the edge.

module "sk.sdf";

import sk.math;

/// A distance field you can hand to the field helpers (`Sdf.gradient`, `Sdf.normal`,
/// `Sdf.scaled`): a struct with `distance`, in pixels, negative inside.
public interface ISdf
{
    float distance(float2 p);
};

/// Where `Sdf.stroke` puts its band relative to the edge.
public enum StrokeAlign
{
    Inside,
    Center,
    Outside,
}

float sk_sdf_cross2(float2 a, float2 b)
{
    return a.x * b.y - a.y * b.x;
}

/// The point folded into one sector of an n-fold symmetric shape: a sector edge straight up (−y).
float2 sk_sdf_sector(float2 p, float wedge)
{
    float a = floorMod(safeAtan2(p.x, -p.y), 2.0 * wedge) - wedge;
    return length(p) * float2(cos(a), abs(sin(a)));
}

public namespace Sdf
{
    // -----------------------------------------------------------------------
    //  Shapes, centred on the origin
    // -----------------------------------------------------------------------

    /// A circle of `radius` px.
    public float circle(float2 p, float radius)
    {
        return length(p) - radius;
    }

    /// An axis-aligned box of `halfSize` px from centre to edge.
    public float box(float2 p, float2 halfSize)
    {
        float2 d = abs(p) - halfSize;
        return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
    }

    /// A box with rounded corners, `radii` in CSS order: top-left, top-right, bottom-right,
    /// bottom-left (y down). Each radius is clamped to 0..min(halfSize).
    public float roundedBox(float2 p, float2 halfSize, float4 radii)
    {
        float top = p.x < 0.0 ? radii.x : radii.y;
        float bottom = p.x < 0.0 ? radii.w : radii.z;
        float r = clamp(p.y < 0.0 ? top : bottom, 0.0, min(halfSize.x, halfSize.y));
        float2 d = abs(p) - halfSize + r;
        return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - r;
    }

    /// A box with every corner rounded by `radius` px.
    public float roundedBox(float2 p, float2 halfSize, float radius)
    {
        return roundedBox(p, halfSize, float4(radius, radius, radius, radius));
    }

    /// An axis-aligned ellipse of semi-axes `radii` px, by six fixed steps towards the closest
    /// point: within 1e-3 px up to 10:1 and 4000 px across; at 40:1 about 0.1 px.
    public float ellipse(float2 p, float2 radii)
    {
        float2 ab = max(radii, 1e-4);
        float2 q = abs(p);
        float2 t = float2(0.70710678, 0.70710678);
        float2 e2 = float2(ab.x * ab.x - ab.y * ab.y, ab.y * ab.y - ab.x * ab.x) / ab;
        [loop]
        for (int i = 0; i < 6; i++)
        {
            float2 onCurve = ab * t;
            float2 evolute = e2 * t * t * t;
            float r = length(onCurve - evolute);
            float2 toPoint = q - evolute;
            float len = length(toPoint);
            float2 next = saturate((toPoint * (r / (len > 0.0 ? len : 1.0)) + evolute) / ab);
            float size = length(next);
            t = size > 0.0 ? next / size : t;
        }
        float d = length(q - ab * t);
        float2 n = q / ab;
        return dot(n, n) < 1.0 ? -d : d;
    }

    /// The distance to the segment from `a` to `b` (0 on it, never negative).
    public float segment(float2 p, float2 a, float2 b)
    {
        float2 pa = p - a;
        float2 ba = b - a;
        float h = saturate(safeDiv(dot(pa, ba), dot(ba, ba)));
        return length(pa - ba * h);
    }

    /// A capsule: the segment from `a` to `b` thickened by `radius` px each side.
    public float capsule(float2 p, float2 a, float2 b, float radius)
    {
        return segment(p, a, b) - radius;
    }

    /// The triangle with corners `a`, `b`, `c`, in either winding.
    public float triangle(float2 p, float2 a, float2 b, float2 c)
    {
        float2 e0 = b - a;
        float2 e1 = c - b;
        float2 e2 = a - c;
        float2 v0 = p - a;
        float2 v1 = p - b;
        float2 v2 = p - c;
        float2 pq0 = v0 - e0 * saturate(safeDiv(dot(v0, e0), dot(e0, e0)));
        float2 pq1 = v1 - e1 * saturate(safeDiv(dot(v1, e1), dot(e1, e1)));
        float2 pq2 = v2 - e2 * saturate(safeDiv(dot(v2, e2), dot(e2, e2)));
        float s = sk_sdf_cross2(e0, e2) >= 0.0 ? 1.0 : -1.0;
        float2 d = min(min(float2(dot(pq0, pq0), s * sk_sdf_cross2(v0, e0)),
            float2(dot(pq1, pq1), s * sk_sdf_cross2(v1, e1))),
            float2(dot(pq2, pq2), s * sk_sdf_cross2(v2, e2)));
        return d.y > 0.0 ? -sqrt(d.x) : sqrt(d.x);
    }

    /// A regular polygon of `sides` (3 or more) with its corners `radius` px from the centre and
    /// one corner straight up (−y): a triangle points up, a hexagon stands on a corner.
    public float polygon(float2 p, float radius, int sides)
    {
        float wedge = SK_PI / float(max(sides, 3));
        float2 corner = radius * float2(cos(wedge), sin(wedge));
        float2 q = sk_sdf_sector(p, wedge) - corner;
        q.y += clamp(-q.y, 0.0, corner.y);
        return length(q) * sign(q.x);
    }

    /// A star of `points` (3 or more) tips `radius` px from the centre and inner corners
    /// `innerRadius` px from it, one tip straight up (−y) like `polygon`.
    public float star(float2 p, float radius, int points, float innerRadius)
    {
        float wedge = SK_PI / float(max(points, 3));
        float2 tip = radius * float2(cos(wedge), sin(wedge));
        float k = clamp(innerRadius / max(radius, 1e-6), 1e-4, 1.0);
        float side = atan2(sin(wedge), cos(wedge) - k);
        float2 dir = float2(cos(side), sin(side));
        float2 q = sk_sdf_sector(p, wedge) - tip;
        q += dir * clamp(-dot(q, dir), 0.0, radius * sin(wedge) / dir.y);
        return length(q) * sign(q.x);
    }

    /// An arc of a circle of `radius` px, `halfAngle` radians either side of straight up (−y),
    /// `thickness` px wide with round ends. A half angle of π is a whole ring.
    public float arc(float2 p, float radius, float halfAngle, float thickness)
    {
        float2 q = float2(abs(p.x), -p.y);
        float2 sc = float2(sin(halfAngle), cos(halfAngle));
        float d = sc.y * q.x > sc.x * q.y ? length(q - sc * radius) : abs(length(q) - radius);
        return d - 0.5 * thickness;
    }

    /// A ring (annulus) of `radius` px to the middle of its band, `thickness` px wide.
    public float ring(float2 p, float radius, float thickness)
    {
        return abs(length(p) - radius) - 0.5 * thickness;
    }

    /// A pie slice of `radius` px, `halfAngle` radians either side of straight up (−y).
    public float pie(float2 p, float radius, float halfAngle)
    {
        float2 q = float2(abs(p.x), -p.y);
        float2 c = float2(sin(halfAngle), cos(halfAngle));
        float l = length(q) - radius;
        float m = length(q - c * clamp(dot(q, c), 0.0, radius));
        return max(l, m * sign(c.y * q.x - c.x * q.y));
    }

    /// The half-plane behind the line through `point` facing `normal`: negative on the side
    /// `normal` points away from.
    public float halfPlane(float2 p, float2 point, float2 normal)
    {
        return dot(p - point, normal * rsqrt(max(dot(normal, normal), 1e-20)));
    }

    /// A plus sign: two bars `halfLength` px from the centre to their ends, `halfWidth` px half
    /// thick.
    public float cross(float2 p, float halfLength, float halfWidth)
    {
        float2 b = float2(halfLength, halfWidth);
        float2 q = abs(p);
        q = q.y > q.x ? q.yx : q;
        float2 d = q - b;
        float k = max(d.y, d.x);
        float2 w = k > 0.0 ? d : float2(b.y - q.x, -k);
        return sign(k) * length(max(w, 0.0));
    }

    // -----------------------------------------------------------------------
    //  Combining distances
    // -----------------------------------------------------------------------

    /// Both shapes.
    public float union(float a, float b) { return min(a, b); }

    /// Where both shapes overlap.
    public float intersection(float a, float b) { return max(a, b); }

    /// `a` with `b` cut out of it.
    public float subtraction(float a, float b) { return max(a, -b); }

    /// Where exactly one of the shapes is.
    public float xor(float a, float b) { return max(min(a, b), -max(a, b)); }

    /// `union` with the seam filleted over `radius` px (quadratic smooth minimum): the fields
    /// blend where they are within `radius` of each other; 0 is `union`.
    public float smoothUnion(float a, float b, float radius)
    {
        float k = max(radius, 1e-6);
        float h = max(k - abs(a - b), 0.0) / k;
        return min(a, b) - h * h * k * 0.25;
    }

    /// `intersection` with the seam rounded over `radius` px.
    public float smoothIntersection(float a, float b, float radius)
    {
        return -smoothUnion(-a, -b, radius);
    }

    /// `subtraction` with the cut's edge rounded over `radius` px.
    public float smoothSubtraction(float a, float b, float radius)
    {
        return -smoothUnion(-a, b, radius);
    }

    /// The shape grown by `amount` px (negative shrinks); corners round as they grow.
    public float inflate(float d, float amount) { return d - amount; }

    /// A band `width` px wide centred on the shape's edge: its outline as a shape of its own.
    public float outline(float d, float width) { return abs(d) - 0.5 * width; }

    // -----------------------------------------------------------------------
    //  Moving the point (domain operations)
    // -----------------------------------------------------------------------

    /// The point as seen by a shape moved by `offset` px.
    public float2 translate(float2 p, float2 offset) { return p - offset; }

    /// The point as seen by a shape turned by `radians` (clockwise on screen, y down).
    public float2 rotate(float2 p, float radians)
    {
        float s = sin(radians);
        float c = cos(radians);
        return float2(c * p.x + s * p.y, c * p.y - s * p.x);
    }

    /// The point folded across the line through the origin with `normal`: a shape drawn on the
    /// side `normal` points to appears mirrored on the other.
    public float2 mirror(float2 p, float2 normal)
    {
        float2 n = normal * rsqrt(max(dot(normal, normal), 1e-20));
        return p - 2.0 * min(dot(p, n), 0.0) * n;
    }

    /// The point in its cell of an endless grid of `spacing` px, a copy centred on the origin.
    /// Exact while the shape stays inside its cell.
    public float2 repeat(float2 p, float2 spacing)
    {
        return p - spacing * round(p / spacing);
    }

    /// The point in its cell of a `count` grid of `spacing` px centred on the origin; outside
    /// it, the point sees the nearest edge copy.
    public float2 repeat(float2 p, float2 spacing, int2 count)
    {
        float2 last = float2(max(count, int2(1, 1)) - 1);
        float2 id = clamp(round(p / spacing + 0.5 * last), 0.0, last);
        return p - spacing * (id - 0.5 * last);
    }

    /// The point turned into the nearest of `count` equal sectors around the origin, the first
    /// centred straight up (−y): draw one copy above the origin to get `count` around it.
    public float2 polarRepeat(float2 p, int count)
    {
        float sector = SK_TAU / float(max(count, 1));
        return rotate(p, sector * round(safeAtan2(p.x, -p.y) / sector));
    }

    /// `shape` scaled by `factor` (> 0) about the origin, the distance scaled with it so it
    /// stays in pixels.
    public float scaled<S : ISdf>(S shape, float2 p, float factor)
    {
        return shape.distance(p / factor) * factor;
    }

    // -----------------------------------------------------------------------
    //  Drawing a distance
    // -----------------------------------------------------------------------

    /// Antialiased fill, 0..1: one pixel of feather centred on the edge, as `input.aa` and the
    /// engine's own coverage.
    public float coverage(float d)
    {
        return saturate(0.5 - d);
    }

    /// Antialiased coverage of a stroke `width` px wide along the edge, inside it, centred on it
    /// or outside it.
    public float stroke(float d, float width, StrokeAlign align = StrokeAlign.Center)
    {
        float centre = align == StrokeAlign.Inside ? -0.5 * width : (align == StrokeAlign.Outside ? 0.5 * width : 0.0);
        return saturate(0.5 - (abs(d - centre) - 0.5 * width));
    }

    /// A soft edge: 1 inside to 0 outside across `width` px centred on the edge, smoothstep-shaped.
    public float feather(float d, float width)
    {
        float w = max(width, 1e-4);
        return 1.0 - smoothstep(-0.5 * w, 0.5 * w, d);
    }

    /// A glow outside the shape: 1 on and inside the edge, falling quadratically to exactly 0 at
    /// `radius` px out.
    public float glow(float d, float radius)
    {
        float t = 1.0 - saturate(max(d, 0.0) / max(radius, 1e-4));
        return t * t;
    }

    // -----------------------------------------------------------------------
    //  Gradients: the direction of steepest increase, unit length for an exact field
    // -----------------------------------------------------------------------

    /// The gradient of any field by central differences `h` px apart (four evaluations).
    public float2 gradient<S : ISdf>(S field, float2 p, float h = 0.5)
    {
        float2 dx = float2(h, 0.0);
        float2 dy = float2(0.0, h);
        return float2(field.distance(p + dx) - field.distance(p - dx),
            field.distance(p + dy) - field.distance(p - dy)) / (2.0 * h);
    }

    /// `gradient` normalised: the outward normal of the nearest edge. (0, 0) where it vanishes.
    public float2 normal<S : ISdf>(S field, float2 p, float h = 0.5)
    {
        float2 g = gradient(field, p, h);
        float l = length(g);
        return l > 0.0 ? g / l : float2(0.0, 0.0);
    }

    /// The exact gradient of `circle`: outward from the centre, (0, 0) at it.
    public float2 circleGradient(float2 p)
    {
        float l = length(p);
        return l > 0.0 ? p / l : float2(0.0, 0.0);
    }

    /// The exact gradient of `roundedBox` with per-corner `radii` (CSS order).
    public float2 roundedBoxGradient(float2 p, float2 halfSize, float4 radii)
    {
        float top = p.x < 0.0 ? radii.x : radii.y;
        float bottom = p.x < 0.0 ? radii.w : radii.z;
        float r = clamp(p.y < 0.0 ? top : bottom, 0.0, min(halfSize.x, halfSize.y));
        float2 w = abs(p) - halfSize + r;
        float2 s = float2(p.x < 0.0 ? -1.0 : 1.0, p.y < 0.0 ? -1.0 : 1.0);
        float g = max(w.x, w.y);
        float2 q = max(w, 0.0);
        float l = length(q);
        float2 inside = w.x > w.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
        return s * (g > 0.0 ? q / (l > 0.0 ? l : 1.0) : inside);
    }

    /// The exact gradient of `box`.
    public float2 boxGradient(float2 p, float2 halfSize)
    {
        return roundedBoxGradient(p, halfSize, float4(0.0, 0.0, 0.0, 0.0));
    }
}
