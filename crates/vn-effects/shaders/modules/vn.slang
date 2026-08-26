// vn — the engine prelude every effect imports.
//
//     import vn;
//
//     [VnEffect("ripple")]
//     struct Ripple : IEffect
//     {
//         [VnDefault("1.5")] [VnRange(0.0, 8.0)] float speed;
//         [VnLength] [VnDefault("24")]           float wavelength;
//         [VnColor]  [VnDefault("#7fd4ff")]      float4 tint;
//
//         float4 shade(EffectInput input) { ... }
//     };
//
// That is the WHOLE authoring surface. No entry point, no semantics, no
// register bindings, no vertex stage — all of that lives below, and
// `packages/vue-native/shaders.ts` appends a four-line wrapper that calls
// `vn_begin` / `vn_finish` around your `shade`.
//
// The attributes are not decoration. slangc reflects them
// (`-reflection-json`), and shaders.ts turns that reflection into the CSS
// schema: `[VnEffect("ripple")]` is the function name, the FIELD ORDER is the
// argument order and the parameter-slot order, and the per-field attributes
// give the CSS type, the default and the clamp range. Nothing is registered
// twice; there is no Rust or JS list of your effect's parameters to keep in
// sync.
//
// Rules the engine cannot enforce for you:
//   * Never call `Sample` inside a loop or a non-uniform branch — implicit
//     derivatives are undefined there. Use `SampleLevel(..., 0)`.
//   * `shade()` returns a PREMULTIPLIED colour by default. Either premultiply
//     yourself (`vn_premultiply`) or opt out with
//     `static override bool straightAlpha() { return true; }`.
//   * Effects compile at `ps_5_0`; the rest of gpui stays at `ps_4_1`.
//
// The full constraint list is docs/SHADERS-AUTHORING.md.

module vn;

// ===========================================================================
//  Authoring attributes — reflected by slangc, consumed by shaders.ts
// ===========================================================================
//
// Slang user-defined attributes are declared as structs tagged
// `[__AttributeUsage]`; their arguments become the `arguments` array under
// `userAttribs` in `-reflection-json`. This is the ONLY channel by which
// information travels from a `.slang` file to the CPU side, which is why every
// piece of CSS-facing metadata is spelled as an attribute rather than as, say,
// a `static const` (not reflected) or a field initialiser (also not reflected).

/// The CSS function name: `[VnEffect("ripple")]` -> `--shading: ripple(...)`.
/// Required, exactly once, on the effect struct.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct VnEffectAttribute
{
    public string name;
};

/// This effect reads `input.backdrop`, so the renderer must snapshot the
/// render target and draw the quad with blending disabled. Costs a full
/// render-target copy per effect group — do not add it unless `shade` really
/// samples the backdrop. Implied by `[VnBackdropRadius]`.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct VnBackdropAttribute
{
};

/// One line of author-facing documentation, echoed in CSS advisories.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct VnDocAttribute
{
    public string text;
};

/// The parameter's default, written the way it would be written in CSS:
/// `"1.5"`, `"0 0.5"`, `"#7fd4ff"`, `"#ffffff28"`.
///
/// A STRING rather than floats on purpose: Slang attribute structs have a
/// fixed field list, so a numeric spelling would need one attribute per arity
/// (`VnDefault`, `VnDefault2`, `VnDefault3`, `VnDefault4`). One string covers
/// every arity AND lets a colour default be written as a hex literal.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnDefaultAttribute
{
    public string value;
};

/// Inclusive clamp for a scalar parameter. Out-of-range CSS values are
/// clamped WITH an advisory, never dropped.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnRangeAttribute
{
    public float lo;
    public float hi;
};

/// The CSS argument is a `<length>` (`24px`), not a `<number>`. Lengths are
/// authored in LOGICAL pixels and multiplied by the device scale factor before
/// they reach the shader, so `input.px` and a length parameter are in the same
/// units on every monitor.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnLengthAttribute
{
};

/// The CSS argument is a `<color>`. Only legal on a `float4`; the four
/// channels arrive as straight-alpha RGBA in 0..1.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnColorAttribute
{
};

/// The CSS argument may be omitted. Matching is positional but type-directed,
/// so an omitted optional parameter is skipped when the argument in that
/// position does not have its type — which is how `glow(12px, 0.8)` and
/// `glow(#7fd4ff, 12px, 0.8)` both parse. An optional parameter must therefore
/// differ in CSS type from the parameter that follows it; shaders.ts rejects
/// the effect at compile time otherwise.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnOptionalAttribute
{
};

/// Pins this parameter to an explicit slot in the 8-float parameter block,
/// instead of letting the packer choose. Unpinned fields then fill the gaps in
/// declaration order.
///
/// Needed only when the packing is ALREADY a published contract — `glow`'s is,
/// because CSS transitions interpolate slot by slot and its layout predates
/// reflection-driven packing. A new effect should never need this: declare the
/// fields in the order you want the CSS arguments and let the packer work.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnSlotAttribute
{
    public int index;
};

/// This length parameter DILATES the drawn quad by its value, giving an outer
/// glow somewhere to land. At most one per effect.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnBleedAttribute
{
};

/// This length parameter is the radius the backdrop is blurred to before
/// `input.backdrop` is sampled. Implies `[VnBackdrop]`. At most one per
/// effect.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnBackdropRadiusAttribute
{
};

/// While this parameter is > 0 the effect is treated as animated and the
/// window keeps repainting. Put it on the parameter that actually drives
/// `input.time` (a speed), so `speed: 0` costs nothing. At most one per
/// effect; an effect with no `[VnAnimated]` parameter never repaints on its
/// own.
[__AttributeUsage(_AttributeTargets.Var)]
public struct VnAnimatedAttribute
{
};

// ===========================================================================
//  The engine's binding contract
// ===========================================================================
//
// These registers are gpui's, not ours to choose: t1 is the instance buffer
// for EVERY gpui pipeline, t0 is the processed backdrop (the Dual-Kawase
// pyramid's top level), t2 the untouched render-target snapshot, b0 the shared
// globals. s1 clamps — s0 wraps, which bleeds the opposite screen edge into
// anything sampling near the border.
//
// They are declared HERE, in the module, rather than in a generated wrapper:
// slangc preserves explicit `register()` annotations verbatim through the HLSL
// backend (measured, docs/SHADERS.md §3.1), so the module is the right home
// for them and an effect can never redeclare or move one.

public struct VnBounds
{
    public float2 origin;
    public float2 size;
};

public struct VnCorners
{
    public float top_left;
    public float top_right;
    public float bottom_right;
    public float bottom_left;
};

/// Keep in sync with `gpui::EffectQuad` (vendor/zed/crates/gpui/src/scene.rs)
/// and `struct EffectQuad` in gpui_windows/src/effects.hlsl.
/// 32 words / 128 bytes.
public struct VnEffectQuad
{
    public uint order;
    public uint effect_id;
    public VnBounds bounds;
    public VnBounds content_mask;
    public VnCorners corner_radii;
    public float4 params0;
    public float4 params1;
    public float opacity;
    public float time;
    public float bleed;
    public float backdrop_blur_radius;
    public float scale;
    public uint flags;
    public uint4 pad;
};

public StructuredBuffer<VnEffectQuad> vn_effect_quads : register(t1);
public Texture2D<float4> vn_backdrop_input : register(t0);
public Texture2D<float4> vn_backdrop_original : register(t2);
public SamplerState vn_clamp_sampler : register(s1);

public cbuffer GlobalParams : register(b0)
{
    public float4 vn_gamma_ratios;
    public float2 vn_viewport_size;
    public float vn_grayscale_enhanced_contrast;
    public float vn_subpixel_enhanced_contrast;
    public uint vn_is_bgr;
    public uint3 vn_global_pad;
};

/// Bit 0 of `VnEffectQuad::flags`.
public static const uint VN_FLAG_NEEDS_BACKDROP = 1u;

/// The varyings the engine-owned vertex stage
/// (gpui_windows/src/effects.hlsl) emits. `SV_ClipDistance` is consumed by the
/// rasterizer and never reaches the fragment stage, which is the only reason
/// this struct can be spelled in Slang at all — the vertex output cannot.
public struct VnFragmentInput
{
    public nointerpolation uint id : TEXCOORD0;
    public float4 position : SV_Position;
};

// ===========================================================================
//  The authoring contract
// ===========================================================================

/// Everything the engine hands an effect, in one struct.
public struct EffectInput
{
    /// Screen-space UV of this pixel (0..1 across the whole window). This is
    /// the coordinate `backdrop` was sampled at.
    public float2 uv;
    /// Position within the element box, normalised 0..1. Values outside 0..1
    /// occur when the effect declares a `[VnBleed]` parameter.
    public float2 local;
    /// Position within the element box in DEVICE pixels (top-left origin).
    public float2 px;
    /// Element box size in device pixels.
    public float2 size;
    /// The backdrop behind this quad, premultiplied. Blurred to
    /// `[VnBackdropRadius]` when the effect declares one, otherwise the raw
    /// snapshot. All zero for effects that did not ask for it.
    public float4 backdrop;
    /// Signed distance to the ELEMENT's rounded rect in device pixels:
    /// negative inside, 0 on the edge, positive outside.
    public float sd;
    /// Antialiased coverage of the element's rounded rect. 1 well inside,
    /// 0 well outside.
    public float coverage;
    /// Seconds since the process started — the clock an animated effect uses.
    public float time;
    /// Device pixels per logical pixel (the DPI scale factor).
    public float scale;
    /// How far outside the element's border box the drawn quad reaches, in
    /// device pixels (the resolved `[VnBleed]` parameter, 0 without one).
    public float bleed;
    /// The element's `opacity`, already folded into the coverage the engine
    /// applies after `shade` — exposed because an effect may want to fade a
    /// component of its own by the same amount.
    public float opacity;
    /// Whether this effect asked for the backdrop.
    public bool hasBackdrop;
    /// Parameter slots 0..3 — the raw packing behind the named fields on your
    /// effect struct. Reading these directly works but bypasses the schema;
    /// prefer the named fields.
    public float4 params0;
    /// Parameter slots 4..7.
    public float4 params1;

    /// Antialiased 0..1 coverage from a signed distance in device pixels.
    /// One device pixel of feather, which is what the rest of gpui uses.
    public float aa(float distance)
    {
        return saturate(0.5 - distance);
    }

    /// `local`, remapped so that (0,0) is the element centre and the SHORTER
    /// axis spans -1..1 — the coordinate system almost every procedural
    /// pattern actually wants, because it does not stretch with the element.
    public float2 centered()
    {
        float2 c = this.local * 2.0 - 1.0;
        float2 s = this.size / max(min(this.size.x, this.size.y), 1.0);
        return c * s;
    }
};

/// The one interface an effect implements.
public interface IEffect
{
    /// Returns the colour for this pixel — PREMULTIPLIED, unless the effect
    /// overrides `straightAlpha`.
    float4 shade(EffectInput input);

    /// Opt into authoring in STRAIGHT alpha:
    ///
    ///     static override bool straightAlpha() { return true; }
    ///
    /// The engine then premultiplies whatever `shade` returns. Slang resolves
    /// this at specialisation time, so the branch is folded away — it costs
    /// nothing at runtime and removes the single most common way to get an
    /// effect wrong.
    static bool straightAlpha() { return false; }
};

// ===========================================================================
//  Engine composition — generic over the effect type
// ===========================================================================
//
// `vn_begin` and `vn_finish` are what a generated wrapper calls; between them
// it does nothing but unpack the parameter slots into the effect struct's
// named fields (which is why shaders.ts needs reflection at all). Because
// `vn_finish` is generic over `T : IEffect`, Slang specialises the whole
// composition at compile time: there is no indirect call, no vtable, and the
// engine-owned coverage/premultiply/backdrop-restore logic lives in this
// module instead of being pasted into every effect as templated text.

float vn_pick_corner_radius(float2 center_to_point, VnCorners radii)
{
    // Branchless on purpose: the obvious nested-if spelling makes Slang emit a
    // temporary that fxc then reports as `warning X4000: use of potentially
    // uninitialized variable`, which would surface in every effect author's
    // build log for no reason.
    float top = center_to_point.x < 0.0 ? radii.top_left : radii.top_right;
    float bottom = center_to_point.x < 0.0 ? radii.bottom_left : radii.bottom_right;
    return center_to_point.y < 0.0 ? top : bottom;
}

/// Signed distance to a rounded rect. Negative inside, 0 on the edge.
public float vn_quad_sdf(float2 point, VnBounds bounds, VnCorners radii)
{
    float2 half_size = bounds.size / 2.0;
    float2 center_to_point = point - (bounds.origin + half_size);
    float radius = vn_pick_corner_radius(center_to_point, radii);
    float2 corner_center_to_point = abs(center_to_point) - half_size + radius;
    if (radius == 0.0)
    {
        return max(corner_center_to_point.x, corner_center_to_point.y);
    }
    return length(max(float2(0.0, 0.0), corner_center_to_point))
        + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))
        - radius;
}

/// The DirectComposition swap chain is premultiplied; keep rgb <= a so the
/// compositor never sees an out-of-gamut pixel. Applied by `vn_finish`, never
/// by effect code, precisely so an effect cannot skip it.
public float4 vn_clamp_premultiplied(float4 color)
{
    // Written as one construction rather than two component writes: the
    // mutating spelling makes Slang emit an indexed store (`_S[int(3)] = …`)
    // that fxc then reports as `warning X4000: use of potentially
    // uninitialized variable` in every effect that links this function.
    float a = saturate(color.a);
    return float4(clamp(color.rgb, 0.0, float3(a, a, a)), a);
}

/// Builds the `EffectInput` for this pixel. Called by the generated wrapper.
public EffectInput vn_begin(VnFragmentInput frag)
{
    VnEffectQuad q = vn_effect_quads[frag.id];
    float2 position = frag.position.xy;
    // SV_Position is the pixel centre, so this lands on a texel centre of the
    // full-resolution snapshot.
    float2 uv = position / vn_viewport_size;

    // `bounds`/`corner_radii` describe the DRAW quad, which is the element's
    // border box dilated by `bleed`. Dilating a rounded rect by b is a
    // Minkowski sum with a disc of radius b, so its SDF is exactly the
    // element's minus b — recover one from the other with a single add
    // instead of carrying a second bounds pair through the instance buffer.
    float sd_draw = vn_quad_sdf(position, q.bounds, q.corner_radii);
    float sd_element = sd_draw + q.bleed;

    float2 element_origin = q.bounds.origin + q.bleed;
    float2 element_size = max(q.bounds.size - 2.0 * q.bleed, float2(1.0, 1.0));

    bool has_backdrop = (q.flags & VN_FLAG_NEEDS_BACKDROP) != 0u;

    EffectInput i;
    i.uv = uv;
    i.px = position - element_origin;
    i.size = element_size;
    i.local = i.px / element_size;
    i.backdrop = has_backdrop
        ? vn_backdrop_input.SampleLevel(vn_clamp_sampler, uv, 0.0)
        : float4(0.0, 0.0, 0.0, 0.0);
    i.sd = sd_element;
    i.coverage = saturate(0.5 - sd_element);
    i.time = q.time;
    i.scale = q.scale;
    i.bleed = q.bleed;
    i.opacity = q.opacity;
    i.hasBackdrop = has_backdrop;
    i.params0 = q.params0;
    i.params1 = q.params1;
    return i;
}

/// Runs the effect and applies everything the engine owns: the straight-alpha
/// convention, the premultiplied clamp, the drawn quad's antialiased coverage,
/// the element opacity, and — for a backdrop effect, which draws with blending
/// disabled and therefore owns every pixel of its quad — restoring the
/// untouched snapshot outside that coverage.
public float4 vn_finish<T : IEffect>(T material, EffectInput i)
{
    float4 shaded = material.shade(i);
    if (T.straightAlpha())
    {
        shaded = float4(shaded.rgb * shaded.a, shaded.a);
    }
    shaded = vn_clamp_premultiplied(shaded);

    float sd_draw = i.sd - i.bleed;
    float coverage = saturate(0.5 - sd_draw) * saturate(i.opacity);

    // Single exit on purpose — an early `return` inside the branch makes fxc
    // lose track of `shaded` and warn about it.
    float4 result;
    if (i.hasBackdrop)
    {
        float4 original =
            vn_backdrop_original.SampleLevel(vn_clamp_sampler, i.uv, 0.0);
        result = lerp(original, shaded, coverage);
    }
    else
    {
        result = shaded * coverage;
    }
    return vn_clamp_premultiplied(result);
}

// ===========================================================================
//  Helpers
// ===========================================================================

/// Straight alpha -> premultiplied.
public float4 vn_premultiply(float4 color)
{
    return float4(color.rgb * color.a, color.a);
}

/// Premultiplied `above` over premultiplied `below`.
public float4 vn_over(float4 below, float4 above)
{
    return above + below * (1.0 - above.a);
}

/// Deterministic hash of a 2D point -> 0..1. Cheap, no texture needed.
public float vn_hash12(float2 p)
{
    float3 p3 = frac(float3(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, float3(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return frac((p3.x + p3.y) * p3.z);
}

/// Deterministic hash of a 2D point -> two independent 0..1 values.
public float2 vn_hash22(float2 p)
{
    float3 p3 = frac(float3(p.x, p.y, p.x) * float3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, float3(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return frac((p3.xx + p3.yz) * p3.zy);
}

/// Value noise on a unit lattice, smoothstep-interpolated.
public float vn_value_noise(float2 p)
{
    float2 i = floor(p);
    float2 f = frac(p);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = vn_hash12(i);
    float b = vn_hash12(i + float2(1.0, 0.0));
    float c = vn_hash12(i + float2(0.0, 1.0));
    float d = vn_hash12(i + float2(1.0, 1.0));
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

/// Three octaves of value noise. Unrolled: no `Sample` is involved, but a
/// fixed trip count keeps the generated HLSL flat.
public float vn_fbm(float2 p)
{
    float sum = vn_value_noise(p) * 0.5;
    sum += vn_value_noise(p * 2.03) * 0.25;
    sum += vn_value_noise(p * 4.01) * 0.125;
    return sum / 0.875;
}

/// Rotates a 2D point by `radians` about the origin.
public float2 vn_rotate(float2 p, float radians)
{
    float s = sin(radians);
    float c = cos(radians);
    return float2(p.x * c - p.y * s, p.x * s + p.y * c);
}

/// Relative luminance of a linear-ish RGB triple.
public float vn_luma(float3 rgb)
{
    return dot(rgb, float3(0.2126, 0.7152, 0.0722));
}

/// Inigo Quilez's cosine palette: `a + b * cos(2pi * (c * t + d))`. Four
/// float3 knobs that sweep a whole family of smooth, well-behaved gradients.
public float3 vn_palette(float t, float3 a, float3 b, float3 c, float3 d)
{
    return a + b * cos(6.28318530718 * (c * t + d));
}
