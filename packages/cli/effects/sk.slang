// sk — the engine prelude every effect imports.
//
//     import sk;
//
//     [SkEffect("ripple")]
//     struct Ripple : IEffect
//     {
//         [SkDefault("1.5")] [SkRange(0.0, 8.0)] float speed;
//         [SkLength] [SkDefault("24")]           float wavelength;
//         [SkColor]  [SkDefault("#7fd4ff")]      float4 tint;
//
//         float4 shade(EffectInput input) { ... }
//     };
//
// That is the WHOLE authoring surface. No entry point, no semantics, no
// register bindings, no vertex stage — all of that lives below, and the
// compiler appends a short wrapper that calls `sk_begin` / `sk_finish`
// around your `shade`. One effect per file, in the app's `shaders/`:
// `sk build`, `sk run` and `sk publish` compile them into the app for every
// OS, `sk dev` recompiles on save, and `sk check` reports what fails.
// The attributes are not decoration. The compiler reflects them and turns
// that reflection into the CSS schema: `[SkEffect("ripple")]` is the
// function name, the FIELD ORDER is the argument order and the
// parameter-slot order, and the per-field attributes give the CSS type, the
// default and the clamp range. Nothing is registered twice; there is no Rust
// or JS list of your effect's parameters to keep in sync.
//
// Rules the engine cannot enforce for you:
//   * Never call `Sample` inside a loop or a non-uniform branch — implicit
//     derivatives are undefined there. Use `SampleLevel(..., 0)`.
//   * `shade()` returns a PREMULTIPLIED colour by default. Either premultiply
//     yourself (`sk_premultiply`) or opt out with
//     `static override bool straightAlpha() { return true; }`. Either way
//     the engine draws a premultiplied result (see `sk_finish`).
//   * Effects compile at `ps_5_0`; the rest of gpui stays at `ps_4_1`.
//   * An effect declares no textures, samplers or buffers: it reads
//     `EffectInput` and its own fields, nothing else. An image is a field
//     too: `SkTexture lut;`, `url(...)` in CSS.

module sk;

// ===========================================================================
//  Authoring attributes — reflected by the compiler, read into the schema
// ===========================================================================
//
// Slang user-defined attributes are declared as structs tagged
// `[__AttributeUsage]`; their arguments become the `arguments` array under
// `userAttribs` in `-reflection-json`. This is the ONLY channel by which
// information travels from a `.slang` file to the CPU side, which is why every
// piece of CSS-facing metadata is spelled as an attribute rather than as, say,
// a `static const` (not reflected) or a field initialiser (also not reflected).

/// The CSS function name: `[SkEffect("ripple")]` -> `--shading: ripple(...)`.
/// Required, exactly once, on the effect struct.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct SkEffectAttribute
{
    public string name;
};

/// This effect reads `input.backdrop`, so the renderer must snapshot the
/// render target and draw the quad with blending disabled. Costs a full
/// render-target copy per effect group — do not add it unless `shade` really
/// samples the backdrop. Implied by `[SkBackdropRadius]`.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct SkBackdropAttribute
{
};

/// This effect shades its element's OWN painting: `input.content`,
/// `contentAt`, `contentAtPx`. The element and its whole subtree (background,
/// border, shadows, outline, children, text, video, carets, animations) paint
/// into an offscreen group the size of the window, at full opacity, and the
/// effect is drawn in their place; what it returns is all that shows of them.
///
///   * The drawn quad is the border box, dilated by a `[SkBleed]` parameter:
///     content can be moved out to it (a box-shadow outside the box shows only
///     there). Sampling anywhere else in the window is allowed and reads
///     transparent where the element painted nothing.
///   * `opacity` applies once, to the result; the content is opaque in the
///     group. `overflow` clips of ancestors cut the result like any effect's.
///   * A transform moves the content and the quad together; a rotation or skew
///     cannot be drawn by an effect, and the element then shows unshaded.
///   * Hit testing is unchanged: it follows layout, never pixels.
///   * The group is a Backdrop Root: a `backdrop-filter` or backdrop effect
///     inside it sees the element's own painting, not the page. The element's
///     own `backdrop-filter` still reads the page. With `[SkBackdrop]` too, the
///     effect owns its quad and composites the content over the backdrop
///     itself (`sk_over(input.backdrop, input.content)`).
///   * Cost: one window-sized offscreen target per nesting depth (pooled,
///     released after 90 idle frames) and the subtree drawn into it every
///     frame the window paints.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct SkContentAttribute
{
};

/// This effect keeps state from frame to frame: `input.history`,
/// `historyAt`, `historyAtPx` read what it stored the last time it stepped,
/// per element and effect. The history is two RGBA 16-bit float textures the
/// size of the drawn quad in device pixels (16 bytes a pixel), zero when the
/// element first draws, started over when that size changes and freed the
/// first frame the element is not painted.
///
/// Each step stores what `update` returns for every texel, unclamped; an
/// effect that does not declare `override float4 update(EffectInput input)`
/// stores what `shade` returned. Both
/// `shade` and `update` read the state as the LAST step left it, so the first
/// frame reads zeros. `update` runs with `input.historyPass` set and reads
/// content and history but never the backdrop.
///
///   * A history steps at most once per painted frame and at most about 75
///     times a second, so a simulation runs at the same speed at 60 and 144
///     Hz. It only steps while the window repaints: animate the effect
///     (`[SkAnimated]`, `[SkAnimatedFor]`, or `[SkPointer]`, which repaints
///     on pointer moves) for as long as the state should keep changing.
///   * Limits: no history on a quad longer than 4096 device pixels on a side,
///     and at most 256 MiB of histories per window; past them it reads zeros.
///   * The element needs an identity to key its state by (every Skathi element
///     has one).
[__AttributeUsage(_AttributeTargets.Struct)]
public struct SkHistoryAttribute
{
};

/// One line of author-facing documentation, echoed in CSS advisories.
[__AttributeUsage(_AttributeTargets.Struct)]
public struct SkDocAttribute
{
    public string text;
};

/// The parameter's default, written the way it would be written in CSS:
/// `"1.5"`, `"0 0.5"`, `"#7fd4ff"`, `"#ffffff28"`.
///
/// A STRING rather than floats on purpose: Slang attribute structs have a
/// fixed field list, so a numeric spelling would need one attribute per arity
/// (`SkDefault`, `SkDefault2`, `SkDefault3`, `SkDefault4`). One string covers
/// every arity AND lets a colour default be written as a hex literal.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkDefaultAttribute
{
    public string value;
};

/// Inclusive clamp for a scalar parameter. Out-of-range CSS values are
/// clamped WITH an advisory, never dropped.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkRangeAttribute
{
    public float lo;
    public float hi;
};

/// The CSS argument is a `<length>` (`24px`), not a `<number>`. Lengths are
/// authored in LOGICAL pixels and multiplied by the device scale factor before
/// they reach the shader, so `input.px` and a length parameter are in the same
/// units on every monitor.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkLengthAttribute
{
};

/// The CSS argument is a `<color>`. Only legal on a `float4`; the four
/// channels arrive as straight-alpha RGBA in 0..1.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkColorAttribute
{
};

/// The CSS argument may be omitted. Matching is positional but type-directed,
/// so an omitted optional parameter is skipped when the argument in that
/// position does not have its type — which is how `glow(12px, 0.8)` and
/// `glow(#7fd4ff, 12px, 0.8)` both parse. An optional parameter must therefore
/// differ in CSS type from the parameter that follows it; the compiler rejects
/// the effect at compile time otherwise.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkOptionalAttribute
{
};

/// Pins this parameter to an explicit slot in the 32-float parameter block,
/// instead of letting the packer choose. Unpinned fields then fill the gaps in
/// declaration order.
///
/// Needed only when the packing is ALREADY a published contract — `glow`'s is,
/// because CSS transitions interpolate slot by slot and its layout predates
/// reflection-driven packing. A new effect should never need this: declare the
/// fields in the order you want the CSS arguments and let the packer work.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkSlotAttribute
{
    public int index;
};

/// This length parameter DILATES the drawn quad by its value, giving an outer
/// glow somewhere to land. At most one per effect.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkBleedAttribute
{
};

/// This length parameter is the radius the backdrop is blurred to before
/// `input.backdrop` is sampled. Implies `[SkBackdrop]`. At most one per
/// effect.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkBackdropRadiusAttribute
{
};

/// While this parameter is > 0 the effect is treated as animated and the
/// window keeps repainting. Put it on the parameter that actually drives
/// `input.time` (a speed), so `speed: 0` costs nothing. At most one per
/// effect; an effect with no `[SkAnimated]` parameter never repaints on its
/// own.
///
/// Use it only as a factor of `input.time` (`input.time * speed`): the host
/// integrates the speed per element and hands the shader the value that makes
/// that product the phase reached so far, so a change of speed (or a
/// transition of it) carries on from where the motion is instead of jumping.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkAnimatedAttribute
{
};

/// Every component of this parameter is a time on the effect clock (the
/// clock `input.time` reads, which is `performance.now() / 1000` in JS). The
/// window keeps repainting until `seconds` after the latest of them, then the
/// effect costs nothing until a parameter changes: hand it the time something
/// started instead of rewriting an age every frame.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkAnimatedForAttribute
{
    public float seconds;
};

/// A `float2` the host fills with the pointer's position in `input.local`
/// units: 0..1 across the element, beyond that outside it, anywhere in the
/// window. The last position stays when the pointer leaves the window. Not a
/// CSS argument. The element repaints when the pointer moves, and only
/// elements whose effect has a pointer parameter do.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkPointerAttribute
{
};

/// On an `SkTexture`: the image is data (a normal map, noise, a LUT), sampled
/// as its file stores it — no premultiplication, alpha is just a fourth
/// channel. Without it the image is colour, sampled premultiplied in the same
/// sRGB-encoded space as `input.backdrop`.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkLinearAttribute
{
};

/// A `float3` the host fills with times on the effect clock: when the pointer
/// last moved in the window, when it last entered the window and when it last
/// left it. It is in the window while `y > z`; `sk_pointer_presence` turns
/// the three into a fade. Long ago (about -1e4) until it happens. Not a CSS
/// argument; repaints like `[SkPointer]`, and with `[SkAnimatedFor]` keeps
/// repainting for a fade after each of them.
[__AttributeUsage(_AttributeTargets.Var)]
public struct SkPointerTimesAttribute
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
// anything sampling near the border. t3..t6 are an effect's `SkTexture`
// images and s2 the sampler that repeats them.
//
// They are declared HERE, in the module, rather than in a generated wrapper:
// Slang keeps explicit `register()` annotations through the HLSL and Metal
// backends (Metal: b0 buffer(0), t1 buffer(1), t0/t2 texture(0/2), s1
// sampler(1), t3..t6 texture(3..6), s2 sampler(2)), so the module is the right
// home and an effect cannot move one.
//
// `[[vk::binding(binding, group)]]` is the same contract for the WGSL target,
// matching the pipeline layout in gpui_wgpu: group 0 the globals, group 1 the
// instance buffer, group 2 the backdrop textures and their sampler, then the
// images at 3..6 and their sampler at 9.
//
// t7 is the element's content (`[SkContent]`), t8 its history (`[SkHistory]`):
// WGSL group 2 bindings 7 and 8, Metal texture(7) and texture(8). Every effect
// pipeline binds them, transparent when the effect has neither.

public struct SkBounds
{
    public float2 origin;
    public float2 size;
};

public struct SkCorners
{
    public float top_left;
    public float top_right;
    public float bottom_right;
    public float bottom_left;
};

/// Keep in sync with `gpui::EffectQuad` (vendor/gpui/crates/gpui/src/scene.rs),
/// `struct EffectQuad` in gpui_windows/src/effects.hlsl and in
/// gpui_wgpu/src/shaders_effects.wgsl. 56 words / 224 bytes.
///
/// The parameter block is `float[]`, not `float4`s: a WGSL storage buffer
/// aligns a `vec4` to 16 bytes, which would move it from offset 72 to 80.
public struct SkEffectQuad
{
    public uint order;
    public uint effect_id;
    public SkBounds bounds;
    public SkBounds content_mask;
    /// The content mask's rounded corners; see `sk_content_mask_coverage`.
    public SkCorners content_mask_radii;
    public SkCorners corner_radii;
    public float params[32];
    public float opacity;
    public float time;
    public float bleed;
    public float backdrop_blur_radius;
    public float scale;
    public uint flags;
};

[[vk::binding(0, 1)]]
public StructuredBuffer<SkEffectQuad> sk_effect_quads : register(t1);
[[vk::binding(0, 2)]]
public Texture2D<float4> sk_backdrop_input : register(t0);
[[vk::binding(2, 2)]]
public Texture2D<float4> sk_backdrop_original : register(t2);
[[vk::binding(1, 2)]]
public SamplerState sk_clamp_sampler : register(s1);
[[vk::binding(7, 2)]]
public Texture2D<float4> sk_content : register(t7);
[[vk::binding(8, 2)]]
public Texture2D<float4> sk_history : register(t8);

// The images of `SkTexture` parameters, in declaration order, and the sampler
// that repeats them.
[[vk::binding(3, 2)]]
Texture2D<float4> sk_texture0 : register(t3);
[[vk::binding(4, 2)]]
Texture2D<float4> sk_texture1 : register(t4);
[[vk::binding(5, 2)]]
Texture2D<float4> sk_texture2 : register(t5);
[[vk::binding(6, 2)]]
Texture2D<float4> sk_texture3 : register(t6);
[[vk::binding(9, 2)]]
SamplerState sk_repeat_sampler : register(s2);

/// Scalar padding rather than a `uint3`: WGSL aligns a `vec3` to 16 bytes and
/// would grow the block from 48 to 64.
[[vk::binding(0, 0)]]
public cbuffer GlobalParams : register(b0)
{
    public float4 sk_gamma_ratios;
    public float2 sk_viewport_size;
    public float sk_grayscale_enhanced_contrast;
    public float sk_subpixel_enhanced_contrast;
    public uint sk_is_bgr;
    public uint sk_global_pad0;
    public uint sk_global_pad1;
    public uint sk_global_pad2;
};

/// Bits of `SkEffectQuad::flags`, `gpui::EFFECT_FLAG_*`.
public static const uint SK_FLAG_NEEDS_BACKDROP = 1u;
public static const uint SK_FLAG_NEEDS_CONTENT = 2u;
public static const uint SK_FLAG_HISTORY = 4u;
public static const uint SK_FLAG_HISTORY_PASS = 8u;


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
    /// occur when the effect declares a `[SkBleed]` parameter.
    public float2 local;
    /// Position within the element box in DEVICE pixels (top-left origin).
    public float2 px;
    /// Element box size in device pixels.
    public float2 size;
    /// The backdrop behind this pixel, premultiplied: `backdropAt(uv)`. All
    /// zero for effects that did not ask for it.
    public float4 backdrop;
    /// Signed distance to the ELEMENT's rounded rect in device pixels:
    /// negative inside, 0 on the edge, positive outside.
    public float sd;
    /// Antialiased coverage of the element's rounded rect. 1 well inside,
    /// 0 well outside.
    public float coverage;
    /// Seconds on the effect clock, which shares its zero with JS:
    /// `input.time == performance.now() / 1000` (rAF timestamps too).
    public float time;
    /// Device pixels per logical pixel (the DPI scale factor).
    public float scale;
    /// How far outside the element's border box the drawn quad reaches, in
    /// device pixels (the resolved `[SkBleed]` parameter, 0 without one).
    public float bleed;
    /// The element's `opacity`, already folded into the coverage the engine
    /// applies after `shade` — exposed because an effect may want to fade a
    /// component of its own by the same amount.
    public float opacity;
    /// The coverage of the content mask, the rounded clip of an `overflow`
    /// ancestor with a `border-radius`: 1 inside it, and everywhere for a
    /// square one. Folded into the coverage the engine applies after `shade`,
    /// like `opacity`.
    public float clip;
    /// Whether this effect asked for the backdrop.
    public bool hasBackdrop;
    /// The element's own painting at this pixel, premultiplied:
    /// `contentAt(uv)`. All zero without `[SkContent]`.
    public float4 content;
    /// Whether this effect reads its content (`[SkContent]`).
    public bool hasContent;
    /// This effect's state at this pixel as the last step stored it:
    /// `historyAtPx(px)`. All zero without `[SkHistory]`, on the first frame,
    /// and where the engine keeps no history (see `[SkHistory]`).
    public float4 history;
    /// Whether this quad has a history to read.
    public bool hasHistory;
    /// Whether this run computes the next history state (`update`) rather
    /// than the element's pixels.
    public bool historyPass;
    /// This quad's index in the instance buffer.
    public uint quad;

    /// Parameter slot `slot` (0..31) — the raw packing behind the named
    /// fields on your effect struct. Bypasses the schema; prefer the fields.
    public float param(uint slot)
    {
        return sk_effect_quads[this.quad].params[slot];
    }

    // The backdrop. Both textures cover the whole window as it was painted
    // before this effect, so any point may be sampled, not only the element's
    // own area; outside the window they clamp to its edge. Neither holds the
    // element's own background (a backdrop effect paints beneath it) nor
    // another backdrop effect drawn in the same group (they never overlap).
    // The blurred one is a lower-resolution pyramid level: blurred to
    // `[SkBackdropRadius]`, or the raw snapshot when the effect has none.
    // All four return zero for an effect without `[SkBackdrop]`.

    /// The backdrop at a window UV (`input.uv` space), blurred to
    /// `[SkBackdropRadius]` when the effect declares one.
    public float4 backdropAt(float2 uv)
    {
        return this.hasBackdrop
            ? sk_backdrop_input.SampleLevel(sk_clamp_sampler, uv, 0.0)
            : float4(0.0, 0.0, 0.0, 0.0);
    }

    /// The backdrop at a point in this element's device pixels (`input.px`
    /// space), blurred like `backdropAt`.
    public float4 backdropAtPx(float2 px)
    {
        return this.backdropAt(this.pxToUv(px));
    }

    /// The untouched backdrop at a window UV, never blurred.
    public float4 rawBackdropAt(float2 uv)
    {
        return this.hasBackdrop
            ? sk_backdrop_original.SampleLevel(sk_clamp_sampler, uv, 0.0)
            : float4(0.0, 0.0, 0.0, 0.0);
    }

    /// The untouched backdrop at a point in this element's device pixels.
    public float4 rawBackdropAtPx(float2 px)
    {
        return this.rawBackdropAt(this.pxToUv(px));
    }

    /// A point in this element's device pixels as a window UV.
    public float2 pxToUv(float2 px)
    {
        return this.uv + (px - this.px) / sk_viewport_size;
    }

    // The content. It covers the whole window, like the backdrop: the
    // element's painting where it is, transparent elsewhere; outside the
    // window it clamps to the edge. Zero without `[SkContent]`.

    /// The element's painting at a window UV (`input.uv` space).
    public float4 contentAt(float2 uv)
    {
        return this.hasContent
            ? sk_content.SampleLevel(sk_clamp_sampler, uv, 0.0)
            : float4(0.0, 0.0, 0.0, 0.0);
    }

    /// The element's painting at a point in its device pixels (`input.px`).
    public float4 contentAtPx(float2 px)
    {
        return this.contentAt(this.pxToUv(px));
    }

    /// The size of the content texture in texels: the window's, in device
    /// pixels. One texel is one `px` unit.
    public float2 contentSize()
    {
        return sk_viewport_size;
    }

    // The history: the drawn quad (the border box dilated by `bleed`) in
    // device pixels, one texel each, clamped at its edge. Zero without
    // `[SkHistory]`.

    /// The state at a point in this element's device pixels (`input.px`).
    public float4 historyAtPx(float2 px)
    {
        float2 size = this.historySize();
        return this.hasHistory
            ? sk_history.SampleLevel(sk_clamp_sampler, (px + this.bleed) / size, 0.0)
            : float4(0.0, 0.0, 0.0, 0.0);
    }

    /// The state at a window UV (`input.uv` space).
    public float4 historyAt(float2 uv)
    {
        return this.historyAtPx(this.px + (uv - this.uv) * sk_viewport_size);
    }

    /// The size of the history in texels: the drawn quad's, in device pixels.
    public float2 historySize()
    {
        return max(round(sk_effect_quads[this.quad].bounds.size), float2(1.0, 1.0));
    }

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

/// An image parameter, written `url(...)` in CSS and resolved like an
/// `<img src>`. At most four per effect. Sampled bilinearly at mip 0, so it is
/// safe in loops and branches. Until the image has loaded, or when it fails or
/// is omitted, every read returns the field's `[SkDefault]` colour
/// (transparent without one) and `size` is zero.
public struct SkTexture
{
    /// The image's size in texels; (0, 0) while it is missing.
    public float2 size;
    uint slot;
    float4 fallback;

    public bool loaded()
    {
        return this.size.x > 0.0;
    }

    /// The image at a 0..1 UV, clamped to its edge texels outside it.
    public float4 sample(float2 uv)
    {
        return this.loaded() ? sk_texture_at(this.slot, sk_clamp_sampler, uv) : this.fallback;
    }

    /// The image at a UV, repeating outside 0..1.
    public float4 sampleRepeat(float2 uv)
    {
        return this.loaded() ? sk_texture_at(this.slot, sk_repeat_sampler, uv) : this.fallback;
    }
};

/// Built by the generated wrapper; an effect receives its `SkTexture` fields filled in.
public SkTexture sk_texture(uint slot, float2 size, float4 fallback)
{
    SkTexture t;
    t.size = size;
    t.slot = slot;
    t.fallback = fallback;
    return t;
}

float4 sk_texture_at(uint slot, SamplerState s, float2 uv)
{
    float4 texel = float4(0.0, 0.0, 0.0, 0.0);
    if (slot == 0u)
    {
        texel = sk_texture0.SampleLevel(s, uv, 0.0);
    }
    else if (slot == 1u)
    {
        texel = sk_texture1.SampleLevel(s, uv, 0.0);
    }
    else if (slot == 2u)
    {
        texel = sk_texture2.SampleLevel(s, uv, 0.0);
    }
    else if (slot == 3u)
    {
        texel = sk_texture3.SampleLevel(s, uv, 0.0);
    }
    return texel;
}

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

    /// With `[SkHistory]`: the state to store for this texel, read back next
    /// step through `input.history`. Raw: not premultiplied, clamped or
    /// covered, kept at 16-bit float precision. Defaults to `shade`.
    float4 update(EffectInput input) { return this.shade(input); }
};

// ===========================================================================
//  Engine composition — generic over the effect type
// ===========================================================================
//
// `sk_begin` and `sk_finish` are what a generated wrapper calls; between them
// it does nothing but unpack the parameter slots into the effect struct's
// named fields (which is why the compiler needs reflection at all). Because
// `sk_finish` is generic over `T : IEffect`, Slang specialises the whole
// composition at compile time: there is no indirect call, no vtable, and the
// engine-owned coverage/premultiply/backdrop-restore logic lives in this
// module instead of being pasted into every effect as templated text.

float sk_pick_corner_radius(float2 center_to_point, SkCorners radii)
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
public float sk_quad_sdf(float2 point, SkBounds bounds, SkCorners radii)
{
    float2 half_size = bounds.size / 2.0;
    float2 center_to_point = point - (bounds.origin + half_size);
    float radius = sk_pick_corner_radius(center_to_point, radii);
    float2 corner_center_to_point = abs(center_to_point) - half_size + radius;
    if (radius == 0.0)
    {
        return max(corner_center_to_point.x, corner_center_to_point.y);
    }
    return length(max(float2(0.0, 0.0), corner_center_to_point))
        + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))
        - radius;
}

/// The coverage of the quad's content mask, the rounded clip an `overflow`
/// ancestor with a `border-radius` pushes. All-zero radii are the plain rect
/// the clip distances already cut, so they return 1 before any SDF math.
/// Mirrors `content_mask_coverage` in gpui_windows/src/shaders.hlsl.
public float sk_content_mask_coverage(float2 position, SkBounds mask, SkCorners radii)
{
    if (radii.top_left == 0.0 && radii.top_right == 0.0 &&
        radii.bottom_right == 0.0 && radii.bottom_left == 0.0)
    {
        return 1.0;
    }
    return saturate(0.5 - sk_quad_sdf(position, mask, radii));
}

/// The DirectComposition swap chain is premultiplied; keep rgb <= a so the
/// compositor never sees an out-of-gamut pixel. Applied by `sk_finish`, never
/// by effect code, precisely so an effect cannot skip it.
public float4 sk_clamp_premultiplied(float4 color)
{
    // Written as one construction rather than two component writes: the
    // mutating spelling makes Slang emit an indexed store (`_S[int(3)] = …`)
    // that fxc then reports as `warning X4000: use of potentially
    // uninitialized variable` in every effect that links this function.
    float a = saturate(color.a);
    return float4(clamp(color.rgb, 0.0, float3(a, a, a)), a);
}

/// Builds the `EffectInput` for this pixel. Called by the generated wrapper.
public EffectInput sk_begin(uint id, float4 frag_position)
{
    SkEffectQuad q = sk_effect_quads[id];
    bool history_pass = (q.flags & SK_FLAG_HISTORY_PASS) != 0u;
    // A history pass draws into the history texture, whose texel (0,0) is
    // the drawn quad's top-left device pixel.
    float2 position = history_pass ? q.bounds.origin + frag_position.xy : frag_position.xy;
    // SV_Position is the pixel centre, so this lands on a texel centre of the
    // full-resolution snapshot.
    float2 uv = position / sk_viewport_size;

    // `bounds`/`corner_radii` describe the DRAW quad, which is the element's
    // border box dilated by `bleed`. Dilating a rounded rect by b is a
    // Minkowski sum with a disc of radius b, so its SDF is exactly the
    // element's minus b — recover one from the other with a single add
    // instead of carrying a second bounds pair through the instance buffer.
    float sd_draw = sk_quad_sdf(position, q.bounds, q.corner_radii);
    float sd_element = sd_draw + q.bleed;

    float2 element_origin = q.bounds.origin + q.bleed;
    float2 element_size = max(q.bounds.size - 2.0 * q.bleed, float2(1.0, 1.0));

    bool has_backdrop = (q.flags & SK_FLAG_NEEDS_BACKDROP) != 0u;

    EffectInput i;
    i.uv = uv;
    i.px = position - element_origin;
    i.size = element_size;
    i.local = i.px / element_size;
    i.hasBackdrop = has_backdrop;
    i.hasContent = (q.flags & SK_FLAG_NEEDS_CONTENT) != 0u;
    i.hasHistory = (q.flags & SK_FLAG_HISTORY) != 0u;
    i.historyPass = history_pass;
    i.quad = id;
    i.bleed = q.bleed;
    i.backdrop = i.backdropAt(uv);
    i.content = i.contentAt(uv);
    i.history = i.historyAtPx(i.px);
    i.sd = sd_element;
    i.coverage = saturate(0.5 - sd_element);
    i.time = q.time;
    i.scale = q.scale;
    i.opacity = q.opacity;
    i.clip = sk_content_mask_coverage(position, q.content_mask, q.content_mask_radii);
    return i;
}

/// Runs the effect and applies everything the engine owns: the straight-alpha
/// convention, the premultiplied clamp, the drawn quad's antialiased coverage,
/// the element opacity, and — for a backdrop effect, which draws with blending
/// disabled and therefore owns every pixel of its quad — restoring the
/// untouched snapshot outside that coverage.
///
/// The result is PREMULTIPLIED whatever `straightAlpha` says. Every renderer
/// blends a non-backdrop effect as premultiplied source-over (One,
/// OneMinusSrcAlpha); blending it by source alpha would apply alpha twice.
public float4 sk_finish<T : IEffect>(T material, EffectInput i)
{
    if (i.historyPass)
    {
        return material.update(i);
    }
    float4 shaded = material.shade(i);
    if (T.straightAlpha())
    {
        shaded = float4(shaded.rgb * shaded.a, shaded.a);
    }
    shaded = sk_clamp_premultiplied(shaded);

    float sd_draw = i.sd - i.bleed;
    float coverage = saturate(0.5 - sd_draw) * saturate(i.opacity) * i.clip;

    // Single exit on purpose — an early `return` inside the branch makes fxc
    // lose track of `shaded` and warn about it.
    float4 result;
    if (i.hasBackdrop)
    {
        float4 original =
            sk_backdrop_original.SampleLevel(sk_clamp_sampler, i.uv, 0.0);
        result = lerp(original, shaded, coverage);
    }
    else
    {
        result = shaded * coverage;
    }
    return sk_clamp_premultiplied(result);
}

// ===========================================================================
//  Helpers
// ===========================================================================

/// 0..1: how present the pointer is in the window, from `[SkPointerTimes]`
/// and `input.time`, fading in over `seconds` after it enters and out over
/// `seconds` after it leaves. Pair the field with `[SkAnimatedFor(seconds)]`.
public float sk_pointer_presence(float3 times, float time, float seconds)
{
    float fade = max(seconds, 1e-4);
    float entered = saturate((time - times.y) / fade);
    float atLeave = saturate((times.z - times.y) / fade);
    float leaving = atLeave * (1.0 - saturate((time - times.z) / fade));
    return times.y > times.z ? entered : leaving;
}

/// Straight alpha -> premultiplied.
public float4 sk_premultiply(float4 color)
{
    return float4(color.rgb * color.a, color.a);
}

/// Premultiplied `above` over premultiplied `below`.
public float4 sk_over(float4 below, float4 above)
{
    return above + below * (1.0 - above.a);
}

/// Deterministic hash of a 2D point -> 0..1. Cheap, no texture needed.
public float sk_hash12(float2 p)
{
    float3 p3 = frac(float3(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, float3(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return frac((p3.x + p3.y) * p3.z);
}

/// Deterministic hash of a 2D point -> two independent 0..1 values.
public float2 sk_hash22(float2 p)
{
    float3 p3 = frac(float3(p.x, p.y, p.x) * float3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, float3(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return frac((p3.xx + p3.yz) * p3.zy);
}

/// Value noise on a unit lattice, smoothstep-interpolated.
public float sk_value_noise(float2 p)
{
    float2 i = floor(p);
    float2 f = frac(p);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = sk_hash12(i);
    float b = sk_hash12(i + float2(1.0, 0.0));
    float c = sk_hash12(i + float2(0.0, 1.0));
    float d = sk_hash12(i + float2(1.0, 1.0));
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

/// Three octaves of value noise. Unrolled: no `Sample` is involved, but a
/// fixed trip count keeps the generated HLSL flat.
public float sk_fbm(float2 p)
{
    float sum = sk_value_noise(p) * 0.5;
    sum += sk_value_noise(p * 2.03) * 0.25;
    sum += sk_value_noise(p * 4.01) * 0.125;
    return sum / 0.875;
}

/// Rotates a 2D point by `radians` about the origin.
public float2 sk_rotate(float2 p, float radians)
{
    float s = sin(radians);
    float c = cos(radians);
    return float2(p.x * c - p.y * s, p.x * s + p.y * c);
}

/// Relative luminance of a linear-ish RGB triple.
public float sk_luma(float3 rgb)
{
    return dot(rgb, float3(0.2126, 0.7152, 0.0722));
}

/// Inigo Quilez's cosine palette: `a + b * cos(2pi * (c * t + d))`. Four
/// float3 knobs that sweep a whole family of smooth, well-behaved gradients.
public float3 sk_palette(float t, float3 a, float3 b, float3 c, float3 d)
{
    return a + b * cos(6.28318530718 * (c * t + d));
}
