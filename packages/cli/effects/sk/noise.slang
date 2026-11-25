// sk.noise — hashes, gradient and cellular noise, fractal sums and dither.
//
//     import sk;
//     import sk.noise;
//
//     float n = Simplex.at(input.px / 48.0);                  // -1..1
//     float clouds = fbm<Perlin>(input.px / 96.0, 5.0) * 0.5 + 0.5;
//     Cell c = worley(input.px / 32.0);                       // c.f1, c.f2, c.id
//     float3 rgb = dither(color.rgb, input.px);               // no banding
//
// Hashes are integer (PCG) and give the same bits on every backend. Noise ranges are stated per
// function and never clamped: a value may approach a bound without reaching it. Coordinates are in
// lattice cells: divide `input.px` by the feature size in pixels.

module "sk.noise";

// ===========================================================================
//  Hashes
// ===========================================================================

/// PCG hash of one word (O'Neill's PCG output permutation, one round): uint in, uint out.
/// About 3 multiplies. Bit-exact on every backend.
public uint pcg(uint v)
{
    uint state = v * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

/// PCG of two words, nested: `pcg(v.x + pcg(v.y))`.
public uint pcg(uint2 v)
{
    return pcg(v.x + pcg(v.y));
}

/// PCG of three words, nested: `pcg(v.x + pcg(v.y + pcg(v.z)))`.
public uint pcg(uint3 v)
{
    return pcg(v.x + pcg(v.y + pcg(v.z)));
}

/// Three independent words from three (Jarzynski and Olano's `pcg3d`). About 9 multiplies.
public uint3 pcg3d(uint3 v)
{
    uint3 s = v * 1664525u + 1013904223u;
    uint x = s.x + s.y * s.z;
    uint y = s.y + s.z * x;
    uint z = s.z + x * y;
    x = x ^ (x >> 16u);
    y = y ^ (y >> 16u);
    z = z ^ (z >> 16u);
    uint x2 = x + y * z;
    uint y2 = y + z * x2;
    uint z2 = z + x2 * y2;
    return uint3(x2, y2, z2);
}

/// The top 24 bits of a hash as a float in [0, 1).
float sk_unit(uint h)
{
    return float(h >> 8u) * (1.0 / 16777216.0);
}

float2 sk_unit(uint2 h)
{
    return float2(h >> 8u) * (1.0 / 16777216.0);
}

float3 sk_unit(uint3 h)
{
    return float3(h >> 8u) * (1.0 / 16777216.0);
}

/// The integer cell holding p as a two's complement word. Clamped into int range first: a float
/// out of it converts to an undefined int on Metal and WGSL.
uint sk_cell(float p)
{
    return uint(int(clamp(floor(p), -2147483648.0, 2147483520.0)));
}

uint2 sk_cell(float2 p)
{
    return uint2(int2(clamp(floor(p), -2147483648.0, 2147483520.0)));
}

uint3 sk_cell(float3 p)
{
    return uint3(int3(clamp(floor(p), -2147483648.0, 2147483520.0)));
}

/// A float in [0, 1) for the cell `floor(p)`: the same bits on every backend.
/// Unlike `sk_hash12`, whose float arithmetic rounds differently from one GPU to another.
public float hash(float p)
{
    return sk_unit(pcg(sk_cell(p)));
}

/// A float in [0, 1) for the cell `floor(p)`. 2 `pcg`.
public float hash(float2 p)
{
    return sk_unit(pcg(sk_cell(p)));
}

/// A float in [0, 1) for the cell `floor(p)`. 3 `pcg`.
public float hash(float3 p)
{
    return sk_unit(pcg(sk_cell(p)));
}

/// Two independent floats in [0, 1) for the cell `floor(p)`. One `pcg3d`.
public float2 hash2(float p)
{
    return sk_unit(pcg3d(uint3(sk_cell(p), 0u, 0u)).xy);
}

/// Two independent floats in [0, 1) for the cell `floor(p)`. One `pcg3d`.
public float2 hash2(float2 p)
{
    return sk_unit(pcg3d(uint3(sk_cell(p), 0u)).xy);
}

/// Two independent floats in [0, 1) for the cell `floor(p)`. One `pcg3d`.
public float2 hash2(float3 p)
{
    return sk_unit(pcg3d(sk_cell(p)).xy);
}

/// Three independent floats in [0, 1) for the cell `floor(p)`. One `pcg3d`.
public float3 hash3(float p)
{
    return sk_unit(pcg3d(uint3(sk_cell(p), 0u, 0u)));
}

/// Three independent floats in [0, 1) for the cell `floor(p)`. One `pcg3d`.
public float3 hash3(float2 p)
{
    return sk_unit(pcg3d(uint3(sk_cell(p), 0u)));
}

/// Three independent floats in [0, 1) for the cell `floor(p)`. One `pcg3d`.
public float3 hash3(float3 p)
{
    return sk_unit(pcg3d(sk_cell(p)));
}

// ===========================================================================
//  Gradient and value noise
// ===========================================================================

/// One of eight unit directions, 45 degrees apart, from the top three bits of a hash.
float2 sk_grad2(uint h)
{
    uint k = h >> 29u;
    uint m = (k + 2u) & 7u;
    float dy = float(k > 0u && k < 4u) - float(k > 4u);
    float dx = float(m > 0u && m < 4u) - float(m > 4u);
    return float2(dx, dy) * ((k & 1u) != 0u ? 0.70710678118654752 : 1.0);
}

/// One of the twelve cube edge directions, unit length.
float3 sk_grad3(uint h)
{
    uint k = (h >> 4u) % 12u;
    uint axis = k >> 2u;
    float a = (k & 1u) != 0u ? -0.70710678118654752 : 0.70710678118654752;
    float b = (k & 2u) != 0u ? -0.70710678118654752 : 0.70710678118654752;
    return axis == 0u ? float3(a, b, 0.0) : (axis == 1u ? float3(a, 0.0, b) : float3(0.0, a, b));
}

/// The quintic fade 6t^5 - 15t^4 + 10t^3 and its derivative.
float2 sk_fade(float t)
{
    return float2(t * t * t * (t * (t * 6.0 - 15.0) + 10.0), 30.0 * t * t * (t - 1.0) * (t - 1.0));
}

/// Bilinear blend of four corner values with fade weights, and its gradient, given each
/// corner's own gradient (zero for value noise).
float3 sk_blend2(float4 v, float2 g00, float2 g10, float2 g01, float2 g11, float2 f)
{
    float2 ux = sk_fade(f.x);
    float2 uy = sk_fade(f.y);
    float k1 = v.y - v.x;
    float k2 = v.z - v.x;
    float k3 = v.x - v.y - v.z + v.w;
    float n = v.x + k1 * ux.x + k2 * uy.x + k3 * ux.x * uy.x;
    float2 g = g00 + ux.x * (g10 - g00) + uy.x * (g01 - g00) + ux.x * uy.x * (g00 - g10 - g01 + g11)
        + float2(ux.y * (k1 + k3 * uy.x), uy.y * (k2 + k3 * ux.x));
    return float3(n, g);
}

/// Perlin gradient noise over cells `i` (corner hashes already wrapped) at fraction `f`.
float3 sk_perlin2(uint2 c00, uint2 c11, float2 f)
{
    float2 g00 = sk_grad2(pcg(c00));
    float2 g10 = sk_grad2(pcg(uint2(c11.x, c00.y)));
    float2 g01 = sk_grad2(pcg(uint2(c00.x, c11.y)));
    float2 g11 = sk_grad2(pcg(c11));
    float4 v = float4(dot(g00, f), dot(g10, f - float2(1.0, 0.0)), dot(g01, f - float2(0.0, 1.0)),
        dot(g11, f - float2(1.0, 1.0)));
    return sk_blend2(v, g00, g10, g01, g11, f) * 1.41421356237309505;
}

float3 sk_value2(uint2 c00, uint2 c11, float2 f)
{
    float4 v = float4(sk_unit(pcg(c00)), sk_unit(pcg(uint2(c11.x, c00.y))), sk_unit(pcg(uint2(c00.x, c11.y))),
        sk_unit(pcg(c11))) * 2.0 - 1.0;
    float2 z = float2(0.0, 0.0);
    return sk_blend2(v, z, z, z, z, f);
}

/// Cell coordinates wrapped into [0, period).
uint2 sk_wrap(float2 cell, float2 period)
{
    float2 p = max(round(period), float2(1.0, 1.0));
    return sk_cell(cell - p * floor(cell / p));
}

/// Trilinear blend with fade weights of eight corner values (v0: x00, x10, x01, x11 at z0;
/// v1 the same at z1) and the corners' gradients; returns (value, gradient).
float4 sk_blend3(float4 v0, float4 v1, float3 g[8], float3 f)
{
    float2 ux = sk_fade(f.x);
    float2 uy = sk_fade(f.y);
    float2 uz = sk_fade(f.z);
    float u = ux.x;
    float v = uy.x;
    float w = uz.x;
    float k0 = v0.x;
    float k1 = v0.y - v0.x;
    float k2 = v0.z - v0.x;
    float k3 = v1.x - v0.x;
    float k4 = v0.x - v0.y - v0.z + v0.w;
    float k5 = v0.x - v0.z - v1.x + v1.z;
    float k6 = v0.x - v0.y - v1.x + v1.y;
    float k7 = -v0.x + v0.y + v0.z - v0.w + v1.x - v1.y - v1.z + v1.w;
    float n = k0 + k1 * u + k2 * v + k3 * w + k4 * u * v + k5 * v * w + k6 * w * u + k7 * u * v * w;
    float3 g = g[0] + u * (g[1] - g[0]) + v * (g[2] - g[0]) + w * (g[4] - g[0])
        + u * v * (g[0] - g[1] - g[2] + g[3]) + v * w * (g[0] - g[2] - g[4] + g[6])
        + w * u * (g[0] - g[1] - g[4] + g[5])
        + u * v * w * (-g[0] + g[1] + g[2] - g[3] + g[4] - g[5] - g[6] + g[7]);
    float3 d = float3(ux.y * (k1 + k4 * v + k6 * w + k7 * v * w), uy.y * (k2 + k5 * w + k4 * u + k7 * w * u),
        uz.y * (k3 + k6 * u + k5 * v + k7 * u * v));
    return float4(n, g + d);
}

uint sk_corner(uint3 c, uint i)
{
    return pcg(c + uint3(i & 1u, (i >> 1u) & 1u, (i >> 2u) & 1u));
}

/// A noise over the plane: `at` in [-1, 1], and its value with its gradient.
/// `fbm`, `turbulence`, `ridged`, `warp` and `curl` take one as a type: `fbm<Simplex>(p)`.
public interface INoise
{
    /// The noise at p, in [-1, 1].
    static float at(float2 p);
    /// (value, d/dx, d/dy) at p; the value as `at`.
    static float3 derivatives(float2 p);
}

/// A noise over space; `fbm<Perlin>(float3(p, t))` animates a 2D pattern without sliding.
public interface INoise3
{
    /// The noise at p, in [-1, 1].
    static float at(float3 p);
    /// (value, d/dx, d/dy, d/dz) at p.
    static float4 derivatives(float3 p);
}

/// Perlin's improved gradient noise: quintic fade, 0 at every integer lattice point.
/// 2D: 4 corners, 8 `pcg`; 3D: 8 corners, 24 `pcg`. Range [-1, 1].
public struct Perlin : INoise, INoise3
{
    /// Perlin noise at p, in [-1, 1]; 0 at integer points.
    public static float at(float2 p)
    {
        return Perlin.derivatives(p).x;
    }

    /// (value, d/dx, d/dy), analytic.
    public static float3 derivatives(float2 p)
    {
        float2 i = floor(p);
        uint2 c = sk_cell(i);
        return sk_perlin2(c, c + 1u, p - i);
    }

    /// Perlin noise that repeats every `period` cells (rounded, at least 1) on each axis.
    public static float tiled(float2 p, float2 period)
    {
        return Perlin.tiledDerivatives(p, period).x;
    }

    /// (value, d/dx, d/dy) of `tiled`.
    public static float3 tiledDerivatives(float2 p, float2 period)
    {
        float2 i = floor(p);
        return sk_perlin2(sk_wrap(i, period), sk_wrap(i + 1.0, period), p - i);
    }

    /// 3D Perlin noise at p, in [-1, 1]; 0 at integer points.
    public static float at(float3 p)
    {
        return Perlin.derivatives(p).x;
    }

    /// (value, d/dx, d/dy, d/dz), analytic.
    public static float4 derivatives(float3 p)
    {
        float3 i = floor(p);
        float3 f = p - i;
        uint3 c = sk_cell(i);
        float3 g[8];
        float v[8];
        [ForceUnroll]
        for (uint k = 0u; k < 8u; k++)
        {
            float3 o = float3(float(k & 1u), float((k >> 1u) & 1u), float((k >> 2u) & 1u));
            g[k] = sk_grad3(sk_corner(c, k));
            v[k] = dot(g[k], f - o);
        }
        return sk_blend3(float4(v[0], v[1], v[2], v[3]), float4(v[4], v[5], v[6], v[7]), g, f) * 1.15470053837925153;
    }
};

/// Value noise: random values at the lattice points, quintic blend. Cheaper than gradient
/// noise and blockier. 2D: 8 `pcg`; 3D: 24. Range [-1, 1].
public struct ValueNoise : INoise, INoise3
{
    /// Value noise at p, in [-1, 1].
    public static float at(float2 p)
    {
        return ValueNoise.derivatives(p).x;
    }

    /// (value, d/dx, d/dy), analytic.
    public static float3 derivatives(float2 p)
    {
        float2 i = floor(p);
        uint2 c = sk_cell(i);
        return sk_value2(c, c + 1u, p - i);
    }

    /// Value noise that repeats every `period` cells (rounded, at least 1) on each axis.
    public static float tiled(float2 p, float2 period)
    {
        return ValueNoise.tiledDerivatives(p, period).x;
    }

    /// (value, d/dx, d/dy) of `tiled`.
    public static float3 tiledDerivatives(float2 p, float2 period)
    {
        float2 i = floor(p);
        return sk_value2(sk_wrap(i, period), sk_wrap(i + 1.0, period), p - i);
    }

    /// 3D value noise at p, in [-1, 1].
    public static float at(float3 p)
    {
        return ValueNoise.derivatives(p).x;
    }

    /// (value, d/dx, d/dy, d/dz), analytic.
    public static float4 derivatives(float3 p)
    {
        float3 i = floor(p);
        uint3 c = sk_cell(i);
        float3 g[8];
        float v[8];
        [ForceUnroll]
        for (uint k = 0u; k < 8u; k++)
        {
            g[k] = float3(0.0, 0.0, 0.0);
            v[k] = sk_unit(sk_corner(c, k)) * 2.0 - 1.0;
        }
        return sk_blend3(float4(v[0], v[1], v[2], v[3]), float4(v[4], v[5], v[6], v[7]), g, p - i);
    }
};

static const float SK_F2 = 0.36602540378443865;
static const float SK_G2 = 0.21132486540518712;

/// One simplex corner: (value, gradient) of t^4 (g . x), t = 0.5 - |x|^2.
float3 sk_simplex_corner(float2 g, float2 x)
{
    float t = max(0.5 - dot(x, x), 0.0);
    float t2 = t * t;
    float gx = dot(g, x);
    return float3(t2 * t2 * gx, t2 * t2 * g - 8.0 * t2 * t * gx * x);
}

/// 2D simplex noise with each corner gradient turned by `cs` (cos, sin), the turn's sign per corner.
float3 sk_simplex2(float2 p, float2 cs)
{
    float2 i = floor(p + (p.x + p.y) * SK_F2);
    float2 x0 = p - i + (i.x + i.y) * SK_G2;
    float2 i1 = x0.x > x0.y ? float2(1.0, 0.0) : float2(0.0, 1.0);
    float2 x1 = x0 - i1 + SK_G2;
    float2 x2 = x0 - 1.0 + 2.0 * SK_G2;
    uint2 c = sk_cell(i);
    uint h0 = pcg(c);
    uint h1 = pcg(c + uint2(i1));
    uint h2 = pcg(c + 1u);
    float3 n = sk_simplex_corner(sk_spin(sk_grad2(h0), cs, h0), x0)
        + sk_simplex_corner(sk_spin(sk_grad2(h1), cs, h1), x1)
        + sk_simplex_corner(sk_spin(sk_grad2(h2), cs, h2), x2);
    return n * 99.0;
}

/// `g` turned by the angle whose (cos, sin) is `cs`, clockwise or anticlockwise by a hash bit.
float2 sk_spin(float2 g, float2 cs, uint h)
{
    float s = (h & 0x100u) != 0u ? -cs.y : cs.y;
    return float2(g.x * cs.x - g.y * s, g.x * s + g.y * cs.x);
}

/// Simplex noise (Perlin 2001) on a triangle (2D) or tetrahedron (3D) lattice: fewer corners
/// than Perlin's, no axis-aligned artefacts. 2D: 3 corners, 6 `pcg`; 3D: 4 corners, 12. Range [-1, 1].
public struct Simplex : INoise, INoise3
{
    /// Simplex noise at p, in [-1, 1].
    public static float at(float2 p)
    {
        return sk_simplex2(p, float2(1.0, 0.0)).x;
    }

    /// (value, d/dx, d/dy), analytic.
    public static float3 derivatives(float2 p)
    {
        return sk_simplex2(p, float2(1.0, 0.0));
    }

    /// Flow noise (Perlin and Neyret): simplex noise whose gradients turn by `angle` radians,
    /// half of them each way, so the pattern swirls in place. Periodic in `angle` over 2π.
    public static float flow(float2 p, float angle)
    {
        return sk_simplex2(p, float2(cos(angle), sin(angle))).x;
    }

    /// (value, d/dx, d/dy) of `flow`.
    public static float3 flowDerivatives(float2 p, float angle)
    {
        return sk_simplex2(p, float2(cos(angle), sin(angle)));
    }

    /// 3D simplex noise at p, in [-1, 1].
    public static float at(float3 p)
    {
        return Simplex.derivatives(p).x;
    }

    /// (value, d/dx, d/dy, d/dz), analytic.
    public static float4 derivatives(float3 p)
    {
        float3 i = floor(p + (p.x + p.y + p.z) * (1.0 / 3.0));
        float3 x0 = p - i + (i.x + i.y + i.z) * (1.0 / 6.0);
        float3 g = step(x0.yzx, x0.xyz);
        float3 l = 1.0 - g;
        float3 i1 = min(g, l.zxy);
        float3 i2 = max(g, l.zxy);
        uint3 c = sk_cell(i);
        float4 n = sk_simplex_corner3(sk_grad3(pcg(c)), x0)
            + sk_simplex_corner3(sk_grad3(pcg(c + uint3(i1))), x0 - i1 + (1.0 / 6.0))
            + sk_simplex_corner3(sk_grad3(pcg(c + uint3(i2))), x0 - i2 + (2.0 / 6.0))
            + sk_simplex_corner3(sk_grad3(pcg(c + 1u)), x0 - 0.5);
        return n * 85.0;
    }
};

float4 sk_simplex_corner3(float3 g, float3 x)
{
    float t = max(0.5 - dot(x, x), 0.0);
    float t2 = t * t;
    float gx = dot(g, x);
    return float4(t2 * t2 * gx, t2 * t2 * g - 8.0 * t2 * t * gx * x);
}

// ===========================================================================
//  Cellular noise
// ===========================================================================

/// The nearest and second-nearest feature points of cellular (Worley) noise at a point.
public struct Cell
{
    /// Distance to the nearest feature point, in cells: 0 at the point, about 0..1.1.
    public float f1;
    /// Distance to the second-nearest, >= f1.
    public float f2;
    /// The lattice cell of the nearest feature point (integer coordinates).
    public float2 cell;
    /// A hash of that cell: the same across the whole Voronoi region, for a colour or a random value.
    public uint id;
    /// From the point to the nearest feature point, in cells.
    public float2 toNearest;
};

/// The feature point of a cell: its centre moved by up to jitter/2 on each axis.
float2 sk_feature(uint2 cell, float jitter)
{
    return 0.5 + (sk_unit(pcg3d(uint3(cell, 0u)).xy) - 0.5) * jitter;
}

Cell sk_worley(float2 p, float jitter, float2 period, bool tiled)
{
    float2 i = floor(p);
    float2 f = p - i;
    float d1 = 8.0;
    float d2 = 8.0;
    float2 best = float2(0.0, 0.0);
    float2 to = float2(0.0, 0.0);
    [ForceUnroll]
    for (int y = -1; y <= 1; y++)
    {
        [ForceUnroll]
        for (int x = -1; x <= 1; x++)
        {
            float2 o = float2(float(x), float(y));
            uint2 key = tiled ? sk_wrap(i + o, period) : sk_cell(i + o);
            float2 r = o + sk_feature(key, jitter) - f;
            float d = dot(r, r);
            bool nearer = d < d1;
            d2 = nearer ? d1 : min(d2, d);
            best = nearer ? i + o : best;
            to = nearer ? r : to;
            d1 = nearer ? d : d1;
        }
    }
    Cell c;
    c.f1 = sqrt(d1);
    c.f2 = sqrt(d2);
    c.cell = best;
    c.id = pcg(tiled ? sk_wrap(best, period) : sk_cell(best));
    c.toNearest = to;
    return c;
}

/// Cellular (Worley) noise: one feature point per cell, searched over the 3x3 cells around p
/// (9 `pcg3d`). `jitter` 0 is a square grid, 1 (default) fully random.
public Cell worley(float2 p, float jitter = 1.0)
{
    return sk_worley(p, jitter, float2(1.0, 1.0), false);
}

/// `worley` that repeats every `period` cells (rounded, at least 1) on each axis.
public Cell worley(float2 p, float jitter, float2 period)
{
    return sk_worley(p, jitter, period, true);
}

/// Distance from p to the nearest edge of its Voronoi cell, in cells: 0 on an edge. Exact
/// bisector distance, not F2 - F1. 18 `pcg3d` (two 3x3 searches).
public float worleyEdge(float2 p, float jitter = 1.0)
{
    Cell near = worley(p, jitter);
    float2 i = floor(p);
    float2 f = p - i;
    float2 home = near.cell - i;
    float edge = 8.0;
    [ForceUnroll]
    for (int y = -1; y <= 1; y++)
    {
        [ForceUnroll]
        for (int x = -1; x <= 1; x++)
        {
            float2 o = home + float2(float(x), float(y));
            float2 r = o + sk_feature(sk_cell(i + o), jitter) - f;
            float2 d = r - near.toNearest;
            float len2 = dot(d, d);
            float bisector = dot(0.5 * (near.toNearest + r), d * rsqrt(max(len2, 1e-12)));
            edge = len2 > 1e-8 ? min(edge, bisector) : edge;
        }
    }
    return edge;
}

/// Cellular noise in 3D, for a pattern that changes with time: `worley(float3(p, t))`.
public struct Cell3
{
    /// Distance to the nearest feature point, in cells.
    public float f1;
    /// Distance to the second-nearest, >= f1.
    public float f2;
    /// The lattice cell of the nearest feature point.
    public float3 cell;
    /// A hash of that cell.
    public uint id;
};

/// 3D cellular noise over the 27 cells around p (27 `pcg3d`).
public Cell3 worley(float3 p, float jitter = 1.0)
{
    float3 i = floor(p);
    float3 f = p - i;
    float d1 = 8.0;
    float d2 = 8.0;
    float3 best = float3(0.0, 0.0, 0.0);
    [ForceUnroll]
    for (int z = -1; z <= 1; z++)
    {
        [ForceUnroll]
        for (int y = -1; y <= 1; y++)
        {
            [ForceUnroll]
            for (int x = -1; x <= 1; x++)
            {
                float3 o = float3(float(x), float(y), float(z));
                float3 r = o + 0.5 + (sk_unit(pcg3d(sk_cell(i + o))) - 0.5) * jitter - f;
                float d = dot(r, r);
                bool nearer = d < d1;
                d2 = nearer ? d1 : min(d2, d);
                best = nearer ? i + o : best;
                d1 = nearer ? d : d1;
            }
        }
    }
    Cell3 c;
    c.f1 = sqrt(d1);
    c.f2 = sqrt(d2);
    c.cell = best;
    c.id = pcg(sk_cell(best));
    return c;
}

// ===========================================================================
//  Fractal sums
// ===========================================================================

/// The most octaves `fbm`, `turbulence` and `ridged` sum; more are ignored.
public static const int MAX_OCTAVES = 8;

/// Each octave turns by atan(3/4) and moves, so the lattices of successive octaves never line up.
float2 sk_next_octave(float2 p, float lacunarity)
{
    return float2(0.8 * p.x - 0.6 * p.y, 0.6 * p.x + 0.8 * p.y) * lacunarity + float2(19.1, -7.3);
}

float3 sk_next_octave(float3 p, float lacunarity)
{
    return float3(0.8 * p.x - 0.6 * p.y, 0.6 * p.x + 0.8 * p.y, p.z) * lacunarity + float3(19.1, -7.3, 3.7);
}

/// Fractal Brownian motion: `octaves` layers of `N`, each `lacunarity` times finer and `gain`
/// times weaker, divided by the sum of the weights so the range stays [-1, 1]. A fractional
/// octave count fades the last layer in. At most `MAX_OCTAVES` (8); cost is octaves x `N`.
public float fbm<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5)
{
    float sum = 0.0;
    float norm = 0.0;
    float amp = 1.0;
    float2 q = p;
    for (int i = 0; i < MAX_OCTAVES; i++)
    {
        float w = saturate(octaves - float(i)) * amp;
        sum += w * N.at(q);
        norm += w;
        amp *= gain;
        q = sk_next_octave(q, lacunarity);
    }
    return sum / max(norm, 1e-6);
}

/// `fbm` with its gradient: (value, d/dx, d/dy), for `curl` of a fractal and for normals.
public float3 fbmDerivatives<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5)
{
    float3 sum = float3(0.0, 0.0, 0.0);
    float norm = 0.0;
    float amp = 1.0;
    float2 q = p;
    float2 jx = float2(1.0, 0.0);
    float2 jy = float2(0.0, 1.0);
    for (int i = 0; i < MAX_OCTAVES; i++)
    {
        float w = saturate(octaves - float(i)) * amp;
        float3 n = N.derivatives(q);
        sum += w * float3(n.x, dot(n.yz, jx), dot(n.yz, jy));
        norm += w;
        amp *= gain;
        q = sk_next_octave(q, lacunarity);
        jx = float2(0.8 * jx.x - 0.6 * jx.y, 0.6 * jx.x + 0.8 * jx.y) * lacunarity;
        jy = float2(0.8 * jy.x - 0.6 * jy.y, 0.6 * jy.x + 0.8 * jy.y) * lacunarity;
    }
    return sum / max(norm, 1e-6);
}

/// `fbm` over space: animate a 2D pattern with `float3(p, time)`. Range [-1, 1].
public float fbm<N : INoise3>(float3 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5)
{
    float sum = 0.0;
    float norm = 0.0;
    float amp = 1.0;
    float3 q = p;
    for (int i = 0; i < MAX_OCTAVES; i++)
    {
        float w = saturate(octaves - float(i)) * amp;
        sum += w * N.at(q);
        norm += w;
        amp *= gain;
        q = sk_next_octave(q, lacunarity);
    }
    return sum / max(norm, 1e-6);
}

/// Turbulence: `fbm` of |N|, billowy. Range [0, 1].
public float turbulence<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5)
{
    float sum = 0.0;
    float norm = 0.0;
    float amp = 1.0;
    float2 q = p;
    for (int i = 0; i < MAX_OCTAVES; i++)
    {
        float w = saturate(octaves - float(i)) * amp;
        sum += w * abs(N.at(q));
        norm += w;
        amp *= gain;
        q = sk_next_octave(q, lacunarity);
    }
    return sum / max(norm, 1e-6);
}

/// Ridged noise: `fbm` of (1 - |N|)^2, sharp crests where N crosses 0. Range [0, 1].
public float ridged<N : INoise>(float2 p, float octaves = 5.0, float lacunarity = 2.0, float gain = 0.5)
{
    float sum = 0.0;
    float norm = 0.0;
    float amp = 1.0;
    float2 q = p;
    for (int i = 0; i < MAX_OCTAVES; i++)
    {
        float w = saturate(octaves - float(i)) * amp;
        float r = 1.0 - abs(N.at(q));
        sum += w * r * r;
        norm += w;
        amp *= gain;
        q = sk_next_octave(q, lacunarity);
    }
    return sum / max(norm, 1e-6);
}

/// Domain warp: p moved by `strength` times two `fbm`s of `N`, for marbled, smoky patterns.
/// Returns the moved point; feed it to any noise. Costs two `fbm`s.
public float2 warp<N : INoise>(float2 p, float strength, float octaves = 4.0)
{
    float2 d = float2(fbm<N>(p, octaves), fbm<N>(p + float2(5.2, 1.3), octaves));
    return p + strength * d;
}

/// Curl noise: the rotated gradient (dN/dy, -dN/dx) of `N`, a divergence-free 2D flow for
/// advecting particles or warping without bunching. Magnitude 0 to about 8 for one octave.
public float2 curl<N : INoise>(float2 p)
{
    float3 n = N.derivatives(p);
    return float2(n.z, -n.y);
}

/// Curl of `fbm<N>`: a divergence-free flow with detail at several scales.
public float2 curl<N : INoise>(float2 p, float octaves, float lacunarity = 2.0, float gain = 0.5)
{
    float3 n = fbmDerivatives<N>(p, octaves, lacunarity, gain);
    return float2(n.z, -n.y);
}

// ===========================================================================
//  Dither
// ===========================================================================

/// Interleaved gradient noise (Jimenez 2014) at a pixel, in [0, 1): a cheap screen-space pattern
/// whose 3x3 neighbourhoods are well spread. Float arithmetic: may differ by an ulp across GPUs.
public float ign(float2 px)
{
    float2 p = floor(px);
    return frac(52.9829189 * frac(dot(p, float2(0.06711056, 0.00583715))));
}

/// Interleaved gradient noise that moves every frame (`frame` a whole frame count), for
/// temporal accumulation.
public float ign(float2 px, float frame)
{
    float f = frame - 64.0 * floor(frame / 64.0);
    return ign(floor(px) + 5.588238 * f);
}

/// The R2 low-discrepancy sequence (Roberts 2018) over pixels, in [0, 1): an even,
/// blue-noise-like threshold mask without a texture.
public float r2(float2 px)
{
    float2 p = floor(px);
    return frac(p.x * 0.75487766624669276 + p.y * 0.56984029099805327);
}

/// The ordered (Bayer) threshold of a pixel in a `size` x `size` matrix, `size` 2, 4 or 8:
/// `(index + 0.5) / size^2`, in (0, 1). Integer maths: the same on every backend.
public float bayer(float2 px, int size = 8)
{
    uint levels = size >= 8 ? 3u : (size >= 4 ? 2u : 1u);
    uint2 c = sk_cell(px) & 7u;
    uint xy = c.x ^ c.y;
    uint index = 0u;
    [ForceUnroll]
    for (uint k = 0u; k < 3u; k++)
    {
        uint shift = 2u * (levels - 1u - min(k, levels - 1u));
        uint bits = (((xy >> k) & 1u) << 1u) | ((c.y >> k) & 1u);
        index = k < levels ? index | (bits << shift) : index;
    }
    float n = float(1u << (2u * levels));
    return (float(index) + 0.5) / n;
}

/// A uniform value in [0, 1) reshaped to a triangular distribution in (-1, 1), mean 0.
public float triangular(float u)
{
    float v = u * 2.0 - 1.0;
    return sign(v) * (1.0 - sqrt(1.0 - abs(v)));
}

/// `rgb` plus triangular (TPDF) noise of +-1 step of 1/levels per channel, so a smooth gradient
/// quantised to `levels` shows no bands. Use on what `shade` returns (sRGB-encoded, 255 steps).
public float3 dither(float3 rgb, float2 px, float levels = 255.0)
{
    float3 u = hash3(px);
    return rgb + float3(triangular(u.x), triangular(u.y), triangular(u.z)) / levels;
}

/// `dither` with a different pattern every frame (`frame` a whole frame count).
public float3 dither(float3 rgb, float2 px, float frame, float levels)
{
    float3 u = sk_unit(pcg3d(uint3(sk_cell(px), sk_cell(frame))));
    return rgb + float3(triangular(u.x), triangular(u.y), triangular(u.z)) / levels;
}
