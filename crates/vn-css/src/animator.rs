//! The transition animator: diff the cascade's output, run the clock, produce
//! a per-frame overlay `StyleRefinement`.
//!
//! Shape (research/reports/css-layer.md §4.4): the cascade runs once per tree
//! mutation and hands us the new `ComputedStates`; we diff its `base` against
//! the previous one over exactly the properties `new.transitions` names, start
//! or *retarget* a per-property track, and every frame emit the interpolated
//! values as a refinement the renderer applies ON TOP of the computed base.
//!
//! Retargeting preserves visual continuity: an interrupted transition restarts
//! from the value it is showing right now, never from the old computed value.
//!
//! Interpolation itself is `gpui::Interpolate` (f32 / Pixels / Rems / Hsla with
//! the shortest hue arc) — the same toolkit gpui's own springs use.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AbsoluteLength, DefiniteLength, EffectSpec, Hsla, Interpolate, Length, Pixels, Rems,
    StyleRefinement, TransformSpec,
};

use crate::keyframes::{AnimationSpec, FillMode, Keyframes, Phase, PlayState};
use crate::transition::{covered, spec_for, Easing, TransitionProperty, TransitionSpec};
use crate::ComputedStates;

// ---------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------

/// An interpolatable value pulled out of a `StyleRefinement` slot.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Animatable {
    Number(f32),
    Color(Hsla),
    Pixels(Pixels),
    Rems(Rems),
    /// `--shading`. The whole spec travels together because only the eight
    /// `params` interpolate — the id, the flags and the schema metadata must
    /// MATCH for a transition to exist at all (see [`unify`]).
    Effect(EffectSpec),
    /// `transform`. The whole spec travels together for the same reason CSS
    /// interpolates matched function lists componentwise rather than through
    /// the matrix: lerping the matrix entries of two rotations shears.
    Transform(TransformSpec),
}

/// Why a slot cannot be interpolated (kept comparable so an *unchanged*
/// non-animatable value never triggers an advisory).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Opaque {
    /// `auto` / `none`
    Auto,
    /// A percentage (gpui `DefiniteLength::Fraction`)
    Fraction(f32),
    /// A `calc(<percentage> + <length>)` (gpui `DefiniteLength::Calc`):
    /// `(fraction, px)`.
    Calc(f32, f32),
    /// A gradient or pattern fill
    NonSolidFill,
    /// `fill: none` / `stroke: none` — this half of an SVG's geometry is not
    /// painted at all. CSS does not interpolate `none` with a paint either, so
    /// the pair is discrete and snaps.
    UnpaintedSvg,
}

/// What one property looks like in one `StyleRefinement`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    /// The refinement does not set the property.
    Missing,
    Value(Animatable),
    Opaque(Opaque),
}

fn from_absolute(a: AbsoluteLength) -> Animatable {
    match a {
        AbsoluteLength::Pixels(p) => Animatable::Pixels(p),
        AbsoluteLength::Rems(r) => Animatable::Rems(r),
    }
}

fn to_absolute(v: Animatable) -> Option<AbsoluteLength> {
    match v {
        Animatable::Pixels(p) => Some(AbsoluteLength::Pixels(p)),
        Animatable::Rems(r) => Some(AbsoluteLength::Rems(r)),
        _ => None,
    }
}

fn slot_absolute(v: Option<AbsoluteLength>) -> Slot {
    v.map_or(Slot::Missing, |a| Slot::Value(from_absolute(a)))
}

fn slot_definite(v: Option<DefiniteLength>) -> Slot {
    match v {
        None => Slot::Missing,
        Some(DefiniteLength::Absolute(a)) => Slot::Value(from_absolute(a)),
        Some(DefiniteLength::Fraction(f)) => Slot::Opaque(Opaque::Fraction(f)),
        Some(DefiniteLength::Calc { fraction, pixels }) => {
            Slot::Opaque(Opaque::Calc(fraction, f32::from(pixels)))
        }
    }
}

fn slot_length(v: Option<Length>) -> Slot {
    match v {
        None => Slot::Missing,
        Some(Length::Auto) => Slot::Opaque(Opaque::Auto),
        Some(Length::Definite(d)) => slot_definite(Some(d)),
    }
}

fn slot_color(v: Option<Hsla>) -> Slot {
    v.map_or(Slot::Missing, |c| Slot::Value(Animatable::Color(c)))
}

/// Reads one property out of a refinement.
fn slot_of(r: &StyleRefinement, p: &TransitionProperty) -> Slot {
    use TransitionProperty::*;
    match p {
        Opacity => r
            .opacity
            .map_or(Slot::Missing, |o| Slot::Value(Animatable::Number(o))),
        Color => slot_color(r.text.color),
        BackgroundColor => match &r.background {
            None => Slot::Missing,
            Some(gpui::Fill::Color(bg)) => match bg.as_solid() {
                Some(c) => Slot::Value(Animatable::Color(c)),
                None => Slot::Opaque(Opaque::NonSolidFill),
            },
        },
        BorderColor => slot_color(r.border_color),
        // CSS applies one colour to every decoration line the element has, so
        // whichever line is present answers and both are written.
        TextDecorationColor => slot_color(
            r.text
                .underline
                .as_ref()
                .and_then(|u| u.color)
                .or_else(|| r.text.strikethrough.as_ref().and_then(|st| st.color)),
        ),
        // A refinement carries no SVG paint — [`slot_inherited`] answers these
        // from the node's `Extras`, and a `@keyframes` block (the one caller
        // that reaches here without any) genuinely cannot set them.
        Fill | Stroke => Slot::Missing,
        Width => slot_length(r.size.width),
        Height => slot_length(r.size.height),
        MinWidth => slot_length(r.min_size.width),
        MinHeight => slot_length(r.min_size.height),
        MaxWidth => slot_length(r.max_size.width),
        MaxHeight => slot_length(r.max_size.height),
        PaddingTop => slot_definite(r.padding.top),
        PaddingRight => slot_definite(r.padding.right),
        PaddingBottom => slot_definite(r.padding.bottom),
        PaddingLeft => slot_definite(r.padding.left),
        MarginTop => slot_length(r.margin.top),
        MarginRight => slot_length(r.margin.right),
        MarginBottom => slot_length(r.margin.bottom),
        MarginLeft => slot_length(r.margin.left),
        BorderTopLeftRadius => slot_absolute(r.corner_radii.top_left),
        BorderTopRightRadius => slot_absolute(r.corner_radii.top_right),
        BorderBottomRightRadius => slot_absolute(r.corner_radii.bottom_right),
        BorderBottomLeftRadius => slot_absolute(r.corner_radii.bottom_left),
        // gpui's `gap` is a Size: row-gap is the height, column-gap the width.
        RowGap => slot_definite(r.gap.height),
        ColumnGap => slot_definite(r.gap.width),
        FontSize => slot_absolute(r.text.font_size),
        Shading => r
            .vn_effect
            .map_or(Slot::Missing, |e| Slot::Value(Animatable::Effect(e))),
        Transform => r
            .vn_transform
            .map_or(Slot::Missing, |t| Slot::Value(Animatable::Transform(t))),
        All | Other(_) => Slot::Missing,
    }
}

/// [`slot_of`], with the INHERITED `color` standing in for an undeclared one.
///
/// `color` is the one animatable property in the set that inherits, so "this
/// rung declares none" is not "unset": it is the parent's colour, a real value,
/// and it is what a browser interpolates from. The cascade already threads it
/// down (`ComputedStates::color`), so the animator only has to ask instead of
/// giving up with "transition of color from an unset value is not animatable".
///
/// This is the common shape, not a corner: Tailwind's `transition-colors` sits
/// on a wrapper whose children inherit their colour, and `group-hover:text-…`
/// then changes it on exactly those children.
fn slot_inherited(
    r: &StyleRefinement,
    p: &TransitionProperty,
    inherited: Option<Hsla>,
    extras: &crate::Extras,
) -> Slot {
    use TransitionProperty::*;
    match p {
        // The one animatable property that INHERITS: "undeclared" means the
        // parent's colour, not "unset".
        Color => match slot_of(r, p) {
            Slot::Missing => slot_color(inherited),
            slot => slot,
        },
        // `fill` / `stroke` live in `Extras`, and what they animate is the
        // colour the sprite is tinted with — which is the element's own
        // `color`, because that is the only colour an SVG has here.
        Fill => slot_paint(extras.fill, r.text.color.or(inherited)),
        Stroke => slot_paint(extras.stroke, r.text.color.or(inherited)),
        // `text-decoration-color` defaults to `currentColor`, so an element
        // with a decoration and no explicit colour animates with its text.
        TextDecorationColor => match slot_of(r, p) {
            Slot::Missing if r.text.underline.is_some() || r.text.strikethrough.is_some() => {
                slot_color(r.text.color.or(inherited))
            }
            slot => slot,
        },
        _ => slot_of(r, p),
    }
}

/// One SVG paint as an interpolable endpoint.
///
/// * undeclared — nothing to animate, and nothing was asked for;
/// * `none` — an unpainted half of the geometry. CSS does not interpolate
///   `none` with a colour either (SVG paint is a discrete type at that
///   boundary), so this is deliberately opaque and snaps;
/// * anything painted — `currentColor`, a concrete colour, a paint server:
///   `vn_tree::svg` flattens all of them onto the element's `color`, because
///   gpui rasterises an SVG into a single-colour alpha mask. So the value that
///   is actually on screen, and therefore the value to interpolate, is that
///   colour.
fn slot_paint(paint: Option<crate::SvgPaint>, color: Option<Hsla>) -> Slot {
    match paint {
        None => Slot::Missing,
        Some(crate::SvgPaint::None) => Slot::Opaque(Opaque::UnpaintedSvg),
        Some(_) => slot_color(color),
    }
}

/// Writes one interpolated value into an overlay refinement.
fn install(r: &mut StyleRefinement, base: &StyleRefinement, p: &TransitionProperty, v: Animatable) {
    use TransitionProperty::*;
    let definite = || to_absolute(v).map(DefiniteLength::Absolute);
    let length = || to_absolute(v).map(|a| Length::Definite(DefiniteLength::Absolute(a)));
    let color = || match v {
        Animatable::Color(c) => Some(c),
        _ => None,
    };
    match p {
        Opacity => {
            if let Animatable::Number(n) = v {
                // CSS clamps the used value; an overshooting easing must not
                // paint a negative or >1 alpha.
                r.opacity = Some(n.clamp(0.0, 1.0));
            }
        }
        Color => r.text.color = color(),
        BackgroundColor => r.background = color().map(|c| gpui::Fill::Color(c.into())),
        BorderColor => r.border_color = color(),
        // The decoration is a whole struct in gpui, so the overlay has to carry
        // a COMPLETE one — thickness, waviness and offset copied from the base,
        // colour replaced. Writing a bare `UnderlineStyle { color }` would
        // refine the base's thickness away and paint a hairline.
        TextDecorationColor => {
            if let Some(c) = color() {
                if let Some(mut u) = r.text.underline.clone().or_else(|| base.text.underline.clone())
                {
                    u.color = Some(c);
                    r.text.underline = Some(u);
                }
                if let Some(mut st) =
                    r.text.strikethrough.clone().or_else(|| base.text.strikethrough.clone())
                {
                    st.color = Some(c);
                    r.text.strikethrough = Some(st);
                }
            }
        }
        // The sprite's tint IS the element's `color` (see [`slot_paint`]), so
        // that is the slot an interpolated paint is written into.
        Fill | Stroke => r.text.color = color().or(r.text.color),
        Width => r.size.width = length(),
        Height => r.size.height = length(),
        MinWidth => r.min_size.width = length(),
        MinHeight => r.min_size.height = length(),
        MaxWidth => r.max_size.width = length(),
        MaxHeight => r.max_size.height = length(),
        PaddingTop => r.padding.top = definite(),
        PaddingRight => r.padding.right = definite(),
        PaddingBottom => r.padding.bottom = definite(),
        PaddingLeft => r.padding.left = definite(),
        MarginTop => r.margin.top = length(),
        MarginRight => r.margin.right = length(),
        MarginBottom => r.margin.bottom = length(),
        MarginLeft => r.margin.left = length(),
        BorderTopLeftRadius => r.corner_radii.top_left = to_absolute(v),
        BorderTopRightRadius => r.corner_radii.top_right = to_absolute(v),
        BorderBottomRightRadius => r.corner_radii.bottom_right = to_absolute(v),
        BorderBottomLeftRadius => r.corner_radii.bottom_left = to_absolute(v),
        RowGap => r.gap.height = definite(),
        ColumnGap => r.gap.width = definite(),
        FontSize => r.text.font_size = to_absolute(v),
        Shading => {
            if let Animatable::Effect(spec) = v {
                r.vn_effect = Some(spec);
            }
        }
        Transform => {
            if let Animatable::Transform(spec) = v {
                r.vn_transform = Some(spec);
            }
        }
        All | Other(_) => {}
    }
}

/// Unsets one property in a refinement — the inverse of [`install`].
///
/// Used to take the transitioned properties OUT of a `:hover`/`group-hover`
/// rung before it is handed to gpui. gpui's ladder refines the rung over the
/// computed style at paint time, unconditionally: anything left in it snaps the
/// instant the pointer crosses the hitbox, which is exactly the behaviour a
/// `transition` declaration exists to replace. What stays in the rung is every
/// property the author did NOT name in `transition` — those keep snapping,
/// which is also what CSS says.
fn clear(r: &mut StyleRefinement, p: &TransitionProperty) {
    use TransitionProperty::*;
    match p {
        Opacity => r.opacity = None,
        Color => r.text.color = None,
        // Take the colour out of the decoration, not the decoration out of the
        // rung: `None` means `currentColor`, which is what an undeclared
        // decoration colour is anyway.
        TextDecorationColor => {
            if let Some(u) = r.text.underline.as_mut() {
                u.color = None;
            }
            if let Some(st) = r.text.strikethrough.as_mut() {
                st.color = None;
            }
        }
        // Deliberately NOTHING. `clear` takes a transitioned property out of
        // the `:hover` rung so gpui's ladder cannot snap it; the animator then
        // owns it. But `fill`/`stroke` are not IN a refinement — what they
        // paint is `color`, and stripping `color` here would strip a hover
        // colour that only `transition: color` is entitled to animate, leaving
        // it painted by nobody. A browser makes the same distinction: with
        // only `fill` in the transition list, a `currentColor` icon whose
        // `color` changes on hover snaps, because `fill`'s own computed value
        // never moved.
        Fill | Stroke => {}
        BackgroundColor => r.background = None,
        BorderColor => r.border_color = None,
        Width => r.size.width = None,
        Height => r.size.height = None,
        MinWidth => r.min_size.width = None,
        MinHeight => r.min_size.height = None,
        MaxWidth => r.max_size.width = None,
        MaxHeight => r.max_size.height = None,
        PaddingTop => r.padding.top = None,
        PaddingRight => r.padding.right = None,
        PaddingBottom => r.padding.bottom = None,
        PaddingLeft => r.padding.left = None,
        MarginTop => r.margin.top = None,
        MarginRight => r.margin.right = None,
        MarginBottom => r.margin.bottom = None,
        MarginLeft => r.margin.left = None,
        BorderTopLeftRadius => r.corner_radii.top_left = None,
        BorderTopRightRadius => r.corner_radii.top_right = None,
        BorderBottomRightRadius => r.corner_radii.bottom_right = None,
        BorderBottomLeftRadius => r.corner_radii.bottom_left = None,
        RowGap => r.gap.height = None,
        ColumnGap => r.gap.width = None,
        FontSize => r.text.font_size = None,
        Shading => r.vn_effect = None,
        Transform => r.vn_transform = None,
        All | Other(_) => {}
    }
}

/// Copies one property from `src` to `dst`, whatever shape its value has.
///
/// [`install`] can only write values the animator can interpolate; this moves
/// the ones it cannot (a gradient `background`, `width: auto`, a percentage)
/// so that a stripped rung's non-interpolable properties still reach the paint.
fn copy_property(dst: &mut StyleRefinement, src: &StyleRefinement, p: &TransitionProperty) {
    use TransitionProperty::*;
    match p {
        Opacity => dst.opacity = src.opacity,
        Color => dst.text.color = src.text.color,
        // The decoration travels whole — its colour is a field of it.
        TextDecorationColor => {
            dst.text.underline = src.text.underline.clone();
            dst.text.strikethrough = src.text.strikethrough.clone();
        }
        // The one non-interpolable SVG paint is `none`, and `none` is not in a
        // refinement at all (`Extras` carries it); the renderer reads it from
        // there, so there is nothing to copy.
        Fill | Stroke => {}
        BackgroundColor => dst.background = src.background.clone(),
        BorderColor => dst.border_color = src.border_color,
        Width => dst.size.width = src.size.width,
        Height => dst.size.height = src.size.height,
        MinWidth => dst.min_size.width = src.min_size.width,
        MinHeight => dst.min_size.height = src.min_size.height,
        MaxWidth => dst.max_size.width = src.max_size.width,
        MaxHeight => dst.max_size.height = src.max_size.height,
        PaddingTop => dst.padding.top = src.padding.top,
        PaddingRight => dst.padding.right = src.padding.right,
        PaddingBottom => dst.padding.bottom = src.padding.bottom,
        PaddingLeft => dst.padding.left = src.padding.left,
        MarginTop => dst.margin.top = src.margin.top,
        MarginRight => dst.margin.right = src.margin.right,
        MarginBottom => dst.margin.bottom = src.margin.bottom,
        MarginLeft => dst.margin.left = src.margin.left,
        BorderTopLeftRadius => dst.corner_radii.top_left = src.corner_radii.top_left,
        BorderTopRightRadius => dst.corner_radii.top_right = src.corner_radii.top_right,
        BorderBottomRightRadius => dst.corner_radii.bottom_right = src.corner_radii.bottom_right,
        BorderBottomLeftRadius => dst.corner_radii.bottom_left = src.corner_radii.bottom_left,
        RowGap => dst.gap.height = src.gap.height,
        ColumnGap => dst.gap.width = src.gap.width,
        FontSize => dst.text.font_size = src.text.font_size,
        Shading => dst.vn_effect = src.vn_effect,
        Transform => dst.vn_transform = src.vn_transform,
        All | Other(_) => {}
    }
}

/// The rung with every property `transitions` covers taken out of it, or `None`
/// when nothing had to be removed (so the caller can keep sharing the `Rc`).
///
/// This is the half of the state-transition design that lives on the paint
/// side: what gpui's ladder still applies instantly, and what the animator now
/// owns. Splitting it here rather than in the renderer keeps the two halves
/// reading off ONE list — [`covered`] — so a property can never be both
/// snapped by gpui and animated by us.
pub fn strip_transitioned(
    rung: &StyleRefinement,
    transitions: &[TransitionSpec],
) -> Option<StyleRefinement> {
    if transitions.is_empty() {
        return None;
    }
    let properties = covered(transitions);
    let mut out = rung.clone();
    let mut touched = false;
    for p in &properties {
        if !matches!(slot_of(rung, p), Slot::Missing) {
            clear(&mut out, p);
            touched = true;
        }
    }
    touched.then_some(out)
}

/// The CSS initial value, when it is well defined *without* inheritance or
/// layout. Used when one endpoint of a transition is simply absent.
fn initial(p: &TransitionProperty) -> Option<Animatable> {
    use TransitionProperty::*;
    match p {
        Opacity => Some(Animatable::Number(1.0)),
        // `transparent`
        BackgroundColor => Some(Animatable::Color(Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        })),
        PaddingTop | PaddingRight | PaddingBottom | PaddingLeft | MarginTop | MarginRight
        | MarginBottom | MarginLeft | BorderTopLeftRadius | BorderTopRightRadius
        | BorderBottomRightRadius | BorderBottomLeftRadius | RowGap | ColumnGap => {
            Some(Animatable::Pixels(Pixels::from(0.0)))
        }
        // `transform: none`, which is a real, interpolable value — and the
        // reason `@keyframes spin { to { transform: rotate(360deg) } }` sweeps
        // the whole turn on an element that declares no transform of its own.
        Transform => Some(Animatable::Transform(TransformSpec::IDENTITY)),
        // `auto` / `none` (sizes) and the inherited or currentColor-derived
        // properties (color, border-color, font-size) have no resolvable
        // initial here — those endpoints snap instead.
        _ => None,
    }
}

/// Reconciles px/rem endpoints: `0px` and `0rem` are the same length, so a
/// zero on one side adopts the other's unit rather than failing the diff.
fn unify(a: Animatable, b: Animatable) -> Option<(Animatable, Animatable)> {
    use Animatable::*;
    match (a, b) {
        (Number(_), Number(_)) | (Color(_), Color(_)) | (Pixels(_), Pixels(_)) | (Rems(_), Rems(_)) => {
            Some((a, b))
        }
        // Two `--shading` endpoints interpolate only when they are the SAME
        // effect: `frost(0.2)` -> `frost(0.8)` is a parameter sweep, but
        // `frost(…)` -> `glow(…)` is a different fragment pipeline with a
        // different parameter meaning per slot, and lerping between them would
        // paint nonsense. Different ids snap, with an advisory naming both.
        (Effect(x), Effect(y)) if x.id == y.id => Some((a, b)),
        // Two transforms always interpolate: the spec is a fixed
        // translate/rotate/scale triple, so there is no "different function
        // list" case to reject — an absent component is its identity.
        (Transform(_), Transform(_)) => Some((a, b)),
        (Pixels(p), Rems(r)) if r.0 == 0.0 => Some((Pixels(p), Pixels(gpui::Pixels::from(0.0)))),
        (Pixels(p), Rems(r)) if p.as_f32() == 0.0 => Some((Rems(gpui::Rems(0.0)), Rems(r))),
        (Rems(r), Pixels(p)) if p.as_f32() == 0.0 => Some((Rems(r), Rems(gpui::Rems(0.0)))),
        (Rems(r), Pixels(p)) if r.0 == 0.0 => Some((Pixels(gpui::Pixels::from(0.0)), Pixels(p))),
        _ => None,
    }
}

fn lerp(from: Animatable, to: Animatable, t: f32) -> Animatable {
    use Animatable::*;
    match (from, to) {
        (Number(a), Number(b)) => Number(f32::interpolate(a, b, t)),
        (Color(a), Color(b)) => Color(clamp_hsla(Hsla::interpolate(a, b, t))),
        (Pixels(a), Pixels(b)) => Pixels(gpui::Pixels::interpolate(a, b, t)),
        (Rems(a), Rems(b)) => Rems(gpui::Rems::interpolate(a, b, t)),
        (Effect(a), Effect(b)) => {
            // `unify` guarantees the ids match, so every slot means the same
            // thing on both sides and sweeps componentwise. Colour parameters
            // therefore interpolate in straight sRGB rather than through HSLA
            // the way `background-color` does — the params vector is opaque
            // floats to everything but the shader, and sRGB is what the CSS
            // default interpolation for legacy colours is anyway.
            let mut spec = b;
            for (slot, value) in spec.params.iter_mut().enumerate() {
                *value = f32::interpolate(a.params[slot], b.params[slot], t);
            }
            // Keep repainting for as long as EITHER endpoint wants to, or an
            // animated grain would freeze the moment a transition starts.
            spec.animated = a.animated || b.animated;
            Effect(spec)
        }
        (Transform(a), Transform(b)) => Transform(a.interpolate(b, t)),
        // Unreachable: tracks are only created from unified endpoints.
        _ => to,
    }
}

/// An overshooting easing can push a channel out of range; browsers clamp the
/// used color rather than painting garbage.
fn clamp_hsla(c: Hsla) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s.clamp(0.0, 1.0),
        l: c.l.clamp(0.0, 1.0),
        a: c.a.clamp(0.0, 1.0),
    }
}

// ---------------------------------------------------------------------------
// tracks
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Track {
    property: TransitionProperty,
    from: Animatable,
    to: Animatable,
    /// When the delay period began.
    start: Instant,
    delay: Duration,
    duration: Duration,
    easing: Easing,
}

impl Track {
    /// Input progress in 0..=1 — 0 for the whole delay, 1 once finished.
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed < self.delay {
            return 0.0;
        }
        if self.duration.is_zero() {
            return 1.0;
        }
        let running = elapsed - self.delay;
        (running.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    fn value(&self, now: Instant) -> Animatable {
        lerp(self.from, self.to, self.easing.eval(self.progress(now)))
    }

    fn end(&self) -> Instant {
        self.start + self.delay + self.duration
    }

    fn finished(&self, now: Instant) -> bool {
        now >= self.end()
    }
}

// ---------------------------------------------------------------------------
// the animator
// ---------------------------------------------------------------------------

/// Per-node transition state. A plain struct — the host owns one and threads it
/// through the restyle and render walks (no thread-locals, so it is testable
/// and can be swapped per window).
#[derive(Default)]
pub struct Animator {
    nodes: HashMap<u32, Vec<Track>>,
    /// Running `@keyframes` animations, per node. A separate map from the
    /// transition tracks because the two have different endpoints, different
    /// clocks and different lifetimes — see [`AnimRun`].
    anims: HashMap<u32, Vec<AnimRun>>,
    /// Queued `transitionend` / `animation*` events, drained once per frame by
    /// the host (which is the only place that has a `Window` to dispatch with).
    events: Vec<AnimEvent>,
    /// The SETTLED value of an interaction-state transition, per node.
    ///
    /// A base-driven transition can drop its track the moment it finishes,
    /// because the new base already carries the target — the track was only ever
    /// covering the gap. A state-driven one cannot: its target lives in a
    /// `:hover` rung that [`crate::ComputedStates`] keeps beside the base, and
    /// the host STRIPS the transitioned properties out of the rung it hands
    /// gpui (otherwise gpui's paint-time ladder would snap them and there would
    /// be nothing left to animate). So once the track settles, this is the only
    /// thing still holding the hovered value on screen. Cleared when the state
    /// goes back to base, where the base itself is the answer again.
    states: HashMap<u32, StyleRefinement>,
    warned: HashSet<String>,
    diagnostics: Vec<String>,
    reduce_motion: bool,
}

impl Animator {
    /// A fresh animator with no running transitions.
    pub fn new() -> Animator {
        Animator::default()
    }

    /// Mirrors `gpui::App::reduce_motion()`. While set, values snap instead of
    /// animating (gpui's own animations behave the same way); turning it on
    /// also cancels whatever is running.
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        if reduce_motion && !self.reduce_motion {
            self.nodes.clear();
            // A cancelled animation still reports `animationcancel`, which is
            // what lets a component that waits for the event stop waiting.
            let now = Instant::now();
            for (node, runs) in std::mem::take(&mut self.anims) {
                for run in runs {
                    self.cancel_event(node, &run, now);
                }
            }
        }
        self.reduce_motion = reduce_motion;
    }

    /// Diffs one node's cascade output and starts/retargets its transitions.
    ///
    /// Call once per node whose `ComputedStates` the restyle pass produced,
    /// with the states from *before* that pass as `old`. `old = None` means the
    /// node was just styled for the first time: CSS does not transition an
    /// element's initial style, so any leftover tracks are dropped.
    pub fn on_computed(
        &mut self,
        node: u32,
        old: Option<&ComputedStates>,
        new: &ComputedStates,
        now: Instant,
    ) {
        if self.reduce_motion {
            self.nodes.remove(&node);
            return;
        }
        let Some(old) = old else {
            self.nodes.remove(&node);
            return;
        };

        for spec in new.transitions.iter() {
            if let TransitionProperty::Other(name) = &spec.property {
                self.warn_once(format!("transition of {name} not animatable yet"));
            }
        }

        if new.transitions.is_empty() {
            // No `transition` declaration any more — running transitions are
            // cancelled and the value snaps, as in CSS. The state hold goes with
            // them: with nothing left to strip, the whole rung is gpui's again
            // and a leftover hold would double-apply it.
            self.nodes.remove(&node);
            self.states.remove(&node);
            return;
        }

        let properties = covered(&new.transitions);
        let mut tracks = self.nodes.remove(&node).unwrap_or_default();
        // A property that dropped out of `transition-property` is cancelled.
        tracks.retain(|t| properties.contains(&t.property));

        for property in &properties {
            let Some(spec) = spec_for(&new.transitions, property) else {
                continue;
            };
            let index = tracks.iter().position(|t| t.property == *property);
            let running = index.map(|i| tracks[i].value(now));

            let from_slot = match running {
                Some(v) => Slot::Value(v),
                None => slot_inherited(&old.base, property, old.color, &old.extras),
            };
            let to_slot = slot_inherited(&new.base, property, new.color, &new.extras);

            match self.endpoints(from_slot, to_slot, property) {
                Endpoints::Pair(from, to) => {
                    if let Some(i) = index {
                        if tracks[i].to == to {
                            // Already heading there: let it run undisturbed.
                            continue;
                        }
                    }
                    // Nothing to animate, or a zero combined duration (CSS
                    // requires a positive combined duration to transition).
                    if from == to || (spec.duration.is_zero() && spec.delay.is_zero()) {
                        if let Some(i) = index {
                            tracks.remove(i);
                        }
                        continue;
                    }
                    let track = Track {
                        property: property.clone(),
                        from,
                        to,
                        start: now,
                        delay: spec.delay,
                        duration: spec.duration,
                        easing: spec.easing,
                    };
                    match index {
                        Some(i) => tracks[i] = track,
                        None => tracks.push(track),
                    }
                }
                Endpoints::Snap => {
                    if let Some(i) = index {
                        tracks.remove(i);
                    }
                }
            }
        }

        if tracks.is_empty() {
            self.nodes.remove(&node);
        } else {
            self.nodes.insert(node, tracks);
        }
    }

    /// An interaction-state flip — the pointer entered or left this element (or
    /// the `.group` ancestor a `group-hover:` rule watches).
    ///
    /// `target` is the refinement the element's ladder produces in its NEW
    /// state: base refined by every rung that is now on. `None` means "back to
    /// the plain base state". Only the properties `transition` covers are
    /// touched; everything else is still gpui's to snap.
    ///
    /// This is deliberately a separate entry point from [`Self::on_computed`]
    /// rather than an extension of it. A base change arrives on the RESTYLE
    /// path, once per pass over the nodes the cascade re-ran; a state flip
    /// arrives on the PAINT path, for one element, from gpui's hover edge. m11's
    /// invariant is that a hover costs no tree work, and folding the flip into
    /// the restyle diff would have cost exactly that.
    pub fn on_state(
        &mut self,
        node: u32,
        computed: &ComputedStates,
        target: Option<&StyleRefinement>,
        now: Instant,
    ) {
        if computed.transitions.is_empty() {
            // Nothing was stripped out of the rung, so gpui is still applying
            // the whole thing and there is nothing for us to hold.
            self.states.remove(&node);
            self.nodes.remove(&node);
            return;
        }
        let properties = covered(&computed.transitions);
        let base = &*computed.base;

        // The settled value, which outlives the tracks. Built first so that the
        // reduce-motion path can take it and skip the animation entirely — the
        // state style still has to APPLY, it just applies at once.
        let mut hold = StyleRefinement::default();
        let mut holds_anything = false;
        if let Some(target) = target {
            for p in &properties {
                let to = slot_of(target, p);
                if matches!(to, Slot::Missing) {
                    continue;
                }
                // Same as the base? Then the base already says it and holding a
                // copy would only make the overlay bigger.
                if to == slot_inherited(base, p, computed.color, &computed.extras) {
                    continue;
                }
                match to {
                    Slot::Value(v) => {
                        install(&mut hold, base, p, v);
                        holds_anything = true;
                    }
                    // A value that cannot be interpolated (a gradient fill,
                    // `auto`, a percentage) still has to be SHOWN — the rung no
                    // longer carries it. It just snaps, which is what CSS does
                    // with a non-interpolable endpoint too.
                    _ => {
                        copy_property(&mut hold, target, p);
                        holds_anything = true;
                    }
                }
            }
        }

        if self.reduce_motion {
            self.nodes.remove(&node);
            if holds_anything {
                self.states.insert(node, hold);
            } else {
                self.states.remove(&node);
            }
            return;
        }

        let previous = self.states.get(&node).cloned();
        let mut tracks = self.nodes.remove(&node).unwrap_or_default();
        tracks.retain(|t| properties.contains(&t.property));

        for property in &properties {
            let Some(spec) = spec_for(&computed.transitions, property) else {
                continue;
            };
            let index = tracks.iter().position(|t| t.property == *property);
            let running = index.map(|i| tracks[i].value(now));
            // Where the property is RIGHT NOW: mid-transition, or settled on
            // the previous state's value, or on the base.
            let from_slot = match running {
                Some(v) => Slot::Value(v),
                None => match previous.as_ref().map(|p| slot_of(p, property)) {
                    Some(Slot::Missing) | None => {
                        slot_inherited(base, property, computed.color, &computed.extras)
                    }
                    Some(s) => s,
                },
            };
            let to_slot = match slot_of(&hold, property) {
                Slot::Missing => slot_inherited(base, property, computed.color, &computed.extras),
                s => s,
            };

            match self.endpoints(from_slot, to_slot, property) {
                Endpoints::Pair(from, to) => {
                    if let Some(i) = index {
                        if tracks[i].to == to {
                            // Already heading there: let it run undisturbed.
                            continue;
                        }
                    }
                    if from == to || (spec.duration.is_zero() && spec.delay.is_zero()) {
                        if let Some(i) = index {
                            tracks.remove(i);
                        }
                        continue;
                    }
                    let track = Track {
                        property: property.clone(),
                        from,
                        to,
                        start: now,
                        delay: spec.delay,
                        duration: spec.duration,
                        easing: spec.easing,
                    };
                    match index {
                        Some(i) => tracks[i] = track,
                        None => tracks.push(track),
                    }
                }
                Endpoints::Snap => {
                    if let Some(i) = index {
                        tracks.remove(i);
                    }
                }
            }
        }

        if tracks.is_empty() {
            self.nodes.remove(&node);
        } else {
            self.nodes.insert(node, tracks);
        }
        if holds_anything {
            self.states.insert(node, hold);
        } else {
            self.states.remove(&node);
        }
    }

    /// Whether `node` is currently holding an interaction-state value — the
    /// host uses it to decide whether a restyle has to re-fold the target.
    pub fn holds_state(&self, node: u32) -> bool {
        self.states.contains_key(&node)
    }

    /// The interpolated values for `node` this frame, to be refined ON TOP of
    /// its computed base. `None` when the node has nothing running.
    pub fn overlay(&self, node: u32, base: &StyleRefinement, now: Instant) -> Option<StyleRefinement> {
        // Animations first, transitions on top: CSS Cascade §6.2 orders
        // transition declarations ABOVE animation declarations.
        let mut r = self.animation_overlay(node, base, now);
        // Then the SETTLED interaction-state values, then the running tracks on
        // top of those: while a track runs it is the authority for its property,
        // and when it finishes the hold is what the value settles onto.
        if let Some(hold) = self.states.get(&node) {
            use gpui::Refineable as _;
            r.get_or_insert_with(StyleRefinement::default).refine(hold);
        }
        if let Some(tracks) = self.nodes.get(&node).filter(|t| !t.is_empty()) {
            let out = r.get_or_insert_with(StyleRefinement::default);
            for track in tracks {
                install(out, base, &track.property, track.value(now));
            }
        }
        r
    }

    /// Whether any node still has an unfinished transition — the host keeps
    /// requesting animation frames while this is true.
    pub fn any_active(&self, now: Instant) -> bool {
        self.nodes
            .values()
            .any(|tracks| tracks.iter().any(|t| !t.finished(now)))
    }

    /// The earliest instant at which every running transition has finished, or
    /// `None` when nothing is running. Useful for a timer-driven repaint.
    pub fn settles_at(&self) -> Option<Instant> {
        self.nodes
            .values()
            .flat_map(|tracks| tracks.iter())
            .map(Track::end)
            .max()
    }

    /// Drops tracks that have reached their target. Safe to call every frame
    /// after painting: a finished track's value equals the computed base, so
    /// removing it is visually a no-op.
    /// Drops tracks that have reached their target, queueing one
    /// `transitionend` for each. Safe to call every frame after painting: a
    /// finished track's value equals the computed base, so removing it is
    /// visually a no-op — and the event is exactly what a browser fires at that
    /// moment (per PROPERTY, with `elapsedTime` excluding the delay).
    pub fn finish_settled(&mut self, now: Instant) {
        let mut events = Vec::new();
        self.nodes.retain(|&node, tracks| {
            tracks.retain(|t| {
                if !t.finished(now) {
                    return true;
                }
                events.push(AnimEvent {
                    node,
                    kind: AnimEventKind::TransitionEnd,
                    name: t.property.name().to_string(),
                    elapsed: t.duration.as_secs_f32(),
                });
                false
            });
            !tracks.is_empty()
        });
        self.events.append(&mut events);
    }

    /// Drops state for nodes that no longer exist.
    pub fn gc(&mut self, alive: &dyn Fn(u32) -> bool) {
        self.nodes.retain(|&id, _| alive(id));
        // A node that stopped existing cannot receive an `animationcancel`
        // (there is nothing to dispatch it to), so its runs are simply dropped.
        self.anims.retain(|&id, _| alive(id));
        self.states.retain(|&id, _| alive(id));
        self.events.retain(|e| alive(e.node));
    }

    /// Forgets everything (e.g. on a stylesheet swap that resets the tree).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.states.clear();
        // A stylesheet swap can redefine or delete any `@keyframes` block, and
        // a run holds the block it started with — so every run is dropped
        // rather than left sampling a rule the sheet no longer has. This is the
        // HMR path; a browser restarts CSS animations on a sheet swap too.
        self.anims.clear();
        self.events.clear();
    }

    /// Whether any node has transition state at all.
    pub fn is_idle(&self) -> bool {
        self.nodes.is_empty() && self.anims.is_empty()
    }

    /// Drains the advisories accumulated since the last call. Each distinct
    /// message is emitted at most once for the lifetime of the animator.
    pub fn take_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn warn_once(&mut self, message: String) {
        if self.warned.insert(message.clone()) {
            self.diagnostics.push(message);
        }
    }

    /// Resolves the two ends of a would-be transition, substituting CSS initial
    /// values for absent endpoints and advising once when the pair cannot be
    /// interpolated.
    fn endpoints(&mut self, from: Slot, to: Slot, property: &TransitionProperty) -> Endpoints {
        if from == to {
            return Endpoints::Snap;
        }
        let (from, to) = match (from, to) {
            (Slot::Value(a), Slot::Value(b)) => (a, b),
            (Slot::Missing, Slot::Value(b)) => match initial(property) {
                Some(a) => (a, b),
                None => {
                    self.warn_once(format!(
                        "transition of {} from an unset value is not animatable yet (v1) — snapping",
                        property.name()
                    ));
                    return Endpoints::Snap;
                }
            },
            (Slot::Value(a), Slot::Missing) => match initial(property) {
                Some(b) => (a, b),
                None => {
                    self.warn_once(format!(
                        "transition of {} to an unset value is not animatable yet (v1) — snapping",
                        property.name()
                    ));
                    return Endpoints::Snap;
                }
            },
            _ => {
                self.warn_once(format!(
                    "transition of {} between percentage/auto endpoints is not animatable yet \
                     (v1) — snapping",
                    property.name()
                ));
                return Endpoints::Snap;
            }
        };
        match unify(from, to) {
            Some((from, to)) => Endpoints::Pair(from, to),
            None => {
                self.warn_once(match (from, to) {
                    // A different effect id is a different fragment pipeline
                    // with a different meaning per parameter slot. This is a
                    // real CSS-visible behaviour, not a gap, so it says what
                    // happened rather than "not supported yet".
                    (Animatable::Effect(a), Animatable::Effect(b)) => format!(
                        "transition of --shading between different effects (`{}` -> `{}`) \
                         cannot interpolate — snapping",
                        effect_name(a.id),
                        effect_name(b.id)
                    ),
                    _ => format!(
                        "transition of {} between different length units is not animatable yet \
                         (v1) — snapping",
                        property.name()
                    ),
                });
                Endpoints::Snap
            }
        }
    }
}

/// The author-facing name of an effect id, for advisories.
fn effect_name(id: u32) -> &'static str {
    vn_effects::EFFECTS
        .get(id as usize)
        .map(|effect| effect.name)
        .unwrap_or("none")
}

enum Endpoints {
    Pair(Animatable, Animatable),
    /// Not interpolatable (or unchanged): the value jumps.
    Snap,
}

impl Animator {
    /// Test convenience: [`Animator::overlay`] against an empty base. The
    /// transition tests all predate `@keyframes`, and none of them needs the
    /// element's own values — only the implicit 0 % / 100 % of an ANIMATION
    /// reads those.
    #[cfg(test)]
    fn overlay_test(&self, node: u32, now: Instant) -> Option<StyleRefinement> {
        self.overlay(node, &StyleRefinement::default(), now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, relative, rems, Rgba};
    use std::rc::Rc;

    fn states(base: StyleRefinement, css: &str) -> ComputedStates {
        let transitions = if css.is_empty() {
            Vec::new()
        } else {
            let mut diagnostics = Vec::new();
            let (_, t) = crate::parse_inline_with_transitions(css, &mut diagnostics);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            t
        };
        ComputedStates {
            base: Rc::new(base),
            transitions: Rc::new(transitions),
            ..Default::default()
        }
    }

    fn with_opacity(v: f32) -> StyleRefinement {
        let mut r = StyleRefinement::default();
        r.opacity = Some(v);
        r
    }

    fn with_width(w: Length) -> StyleRefinement {
        let mut r = StyleRefinement::default();
        r.size.width = Some(w);
        r
    }

    fn with_radius(radius: AbsoluteLength) -> StyleRefinement {
        let mut r = StyleRefinement::default();
        r.corner_radii.top_left = Some(radius);
        r
    }

    fn with_opacity_and_radius(o: f32, radius: f32) -> StyleRefinement {
        let mut r = with_radius(px(radius).into());
        r.opacity = Some(o);
        r
    }

    fn opacity_of(r: &StyleRefinement) -> f32 {
        r.opacity.expect("overlay carries opacity")
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn first_computation_does_not_animate() {
        let mut a = Animator::new();
        let now = t0();
        let new = states(with_opacity(0.0), "transition: opacity 1s linear");
        a.on_computed(1, None, &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert!(!a.any_active(now));
    }

    #[test]
    fn diff_start_interpolate_settle() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear");

        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));
        assert!((opacity_of(&a.overlay(1, &StyleRefinement::default(), now).unwrap()) - 0.0).abs() < 1e-6);

        let half = now + Duration::from_millis(500);
        assert!((opacity_of(&a.overlay(1, &StyleRefinement::default(), half).unwrap()) - 0.5).abs() < 1e-3);

        let end = now + Duration::from_millis(1000);
        assert!((opacity_of(&a.overlay(1, &StyleRefinement::default(), end).unwrap()) - 1.0).abs() < 1e-6);
        assert!(!a.any_active(end));

        a.finish_settled(end);
        assert!(a.overlay(1, &StyleRefinement::default(), end).is_none());
        assert!(a.is_idle());
    }

    #[test]
    fn delay_holds_the_start_value() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear 500ms");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear 500ms");
        a.on_computed(1, Some(&old), &new, now);

        for at in [0, 200, 499] {
            let v = opacity_of(&a.overlay_test(1, now + Duration::from_millis(at)).unwrap());
            assert!(v.abs() < 1e-6, "value moved during the delay at {at}ms: {v}");
        }
        let v = opacity_of(&a.overlay_test(1, now + Duration::from_millis(1000)).unwrap());
        assert!((v - 0.5).abs() < 1e-3, "{v}");
        assert!(a.any_active(now + Duration::from_millis(1000)));
        assert!(!a.any_active(now + Duration::from_millis(1500)));
    }

    #[test]
    fn zero_combined_duration_snaps() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 0s");
        let new = states(with_opacity(1.0), "transition: opacity 0s");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    #[test]
    fn zero_duration_with_delay_jumps_after_the_delay() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 0s 300ms");
        let new = states(with_opacity(1.0), "transition: opacity 0s 300ms");
        a.on_computed(1, Some(&old), &new, now);
        assert!(opacity_of(&a.overlay_test(1, now + Duration::from_millis(100)).unwrap()) < 1e-6);
        assert!(
            (opacity_of(&a.overlay_test(1, now + Duration::from_millis(400)).unwrap()) - 1.0).abs()
                < 1e-6
        );
    }

    #[test]
    fn retarget_preserves_visual_continuity() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let zero = states(with_opacity(0.0), css);
        let one = states(with_opacity(1.0), css);

        a.on_computed(1, Some(&zero), &one, now);
        let mid = now + Duration::from_millis(400);
        let at_interrupt = opacity_of(&a.overlay(1, &StyleRefinement::default(), mid).unwrap());
        assert!((at_interrupt - 0.4).abs() < 1e-3, "{at_interrupt}");

        // Reverse mid-flight: the very next sample must equal what was shown.
        a.on_computed(1, Some(&one), &zero, mid);
        let resumed = opacity_of(&a.overlay(1, &StyleRefinement::default(), mid).unwrap());
        assert!(
            (resumed - at_interrupt).abs() < 1e-4,
            "jumped on retarget: {at_interrupt} -> {resumed}"
        );

        // …and then it travels toward the new target, not the old one.
        let later = mid + Duration::from_millis(200);
        let v = opacity_of(&a.overlay(1, &StyleRefinement::default(), later).unwrap());
        assert!(v < at_interrupt, "{v} should be below {at_interrupt}");
        assert!((v - 0.32).abs() < 1e-2, "{v}");
        let done = mid + Duration::from_millis(1000);
        assert!(opacity_of(&a.overlay(1, &StyleRefinement::default(), done).unwrap()).abs() < 1e-6);
    }

    #[test]
    fn restyle_with_the_same_target_does_not_restart() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let zero = states(with_opacity(0.0), css);
        let one = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&zero), &one, now);

        let mid = now + Duration::from_millis(500);
        // vn-tree recomputes every node on every restyle: old == new here.
        a.on_computed(1, Some(&one), &one, mid);
        let v = opacity_of(&a.overlay(1, &StyleRefinement::default(), mid).unwrap());
        assert!((v - 0.5).abs() < 1e-3, "restarted: {v}");
        assert!(!a.any_active(now + Duration::from_millis(1000)));
    }

    #[test]
    fn colors_interpolate_through_hsla() {
        let mut a = Animator::new();
        let now = t0();
        let red: Hsla = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
        .into();
        let blue: Hsla = Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }
        .into();
        fn mk(c: Hsla) -> StyleRefinement {
            let mut r = StyleRefinement::default();
            r.background = Some(gpui::Fill::Color(c.into()));
            r
        }
        let old = states(mk(red), "transition: background-color 1s linear");
        let new = states(mk(blue), "transition: background-color 1s linear");
        a.on_computed(1, Some(&old), &new, now);

        let half = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        let c = half.background.unwrap().color().unwrap().as_solid().unwrap();
        // Red is hue 0, blue is hue 2/3: the shortest arc runs backwards
        // through magenta, landing on 5/6.
        assert!((c.h - 5.0 / 6.0).abs() < 1e-3, "hue {}", c.h);
        assert!((c.s - 1.0).abs() < 1e-3);
        assert!((c.l - 0.5).abs() < 1e-3);
    }

    #[test]
    fn pixel_lengths_interpolate() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_width(px(100.0).into()), "transition: width 1s linear");
        let new = states(with_width(px(200.0).into()), "transition: width 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(250)).unwrap();
        assert_eq!(overlay.size.width, Some(px(125.0).into()));
    }

    #[test]
    fn padding_from_unset_uses_the_css_initial_zero() {
        let mut a = Animator::new();
        let now = t0();
        let mut to = StyleRefinement::default();
        to.padding.top = Some(px(20.0).into());
        let old = states(StyleRefinement::default(), "transition: padding 1s linear");
        let new = states(to, "transition: padding 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert_eq!(overlay.padding.top, Some(px(10.0).into()));
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn percentage_endpoints_snap_with_one_advisory() {
        let mut a = Animator::new();
        let now = t0();
        let from = with_width(px(100.0).into());
        let to = with_width(Length::Definite(relative(0.5)));
        let old = states(from, "transition: width 1s linear");
        let new = states(to, "transition: width 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        let diags = a.take_diagnostics();
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].contains("width"), "{diags:?}");

        // A second identical failure must stay quiet.
        a.on_computed(2, Some(&old), &new, now);
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn auto_endpoint_snaps() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_width(Length::Auto), "transition: width 1s linear");
        let new = states(with_width(px(200.0).into()), "transition: width 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert_eq!(a.take_diagnostics().len(), 1);
    }

    #[test]
    fn unchanged_non_animatable_value_is_silent() {
        let mut a = Animator::new();
        let now = t0();
        let auto = states(with_width(Length::Auto), "transition: width 1s linear");
        a.on_computed(1, Some(&auto), &auto, now);
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn unsupported_property_warns_once() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: filter 1s");
        let new = states(with_opacity(1.0), "transition: filter 1s");
        a.on_computed(1, Some(&old), &new, now);
        a.on_computed(2, Some(&old), &new, now);
        let diags = a.take_diagnostics();
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].contains("filter"), "{diags:?}");
        // …and nothing animates.
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    #[test]
    fn all_covers_every_animatable_property() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity_and_radius(0.0, 0.0), "transition: all 1s linear");
        let new = states(with_opacity_and_radius(1.0, 8.0), "transition: all 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert!((overlay.opacity.unwrap() - 0.5).abs() < 1e-3);
        assert_eq!(overlay.corner_radii.top_left, Some(px(4.0).into()));
    }

    #[test]
    fn removing_the_transition_cancels_running_tracks() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));

        let mid = now + Duration::from_millis(300);
        let bare = states(with_opacity(1.0), "");
        a.on_computed(1, Some(&new), &bare, mid);
        assert!(a.overlay(1, &StyleRefinement::default(), mid).is_none());
        assert!(!a.any_active(mid));
    }

    #[test]
    fn narrowing_transition_property_cancels_the_dropped_one() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity_and_radius(0.0, 0.0), "transition: all 1s linear");
        let new = states(with_opacity_and_radius(1.0, 8.0), "transition: all 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let mid = now + Duration::from_millis(200);
        assert!(a.overlay(1, &StyleRefinement::default(), mid).unwrap().corner_radii.top_left.is_some());

        let narrowed = states(
            with_opacity_and_radius(1.0, 8.0),
            "transition: opacity 1s linear",
        );
        a.on_computed(1, Some(&new), &narrowed, mid);
        let overlay = a.overlay(1, &StyleRefinement::default(), mid).unwrap();
        assert!(overlay.corner_radii.top_left.is_none());
        assert!(overlay.opacity.is_some());
    }

    #[test]
    fn gc_and_clear_drop_node_state() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        a.on_computed(2, Some(&old), &new, now);
        assert!(a.overlay(2, &StyleRefinement::default(), now).is_some());

        a.gc(&|id| id == 1);
        assert!(a.overlay(2, &StyleRefinement::default(), now).is_none());
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_some());

        a.clear();
        assert!(a.is_idle());
    }

    #[test]
    fn reduce_motion_snaps_and_cancels() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));

        a.set_reduce_motion(true);
        assert!(a.is_idle());
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());

        a.set_reduce_motion(false);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_some());
    }

    #[test]
    fn settles_at_reports_the_last_end() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let settle = a.settles_at().unwrap();
        assert!(settle >= now + Duration::from_millis(999));
        assert!(settle <= now + Duration::from_millis(1001));
    }

    #[test]
    fn zero_px_and_zero_rem_unify() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(
            with_radius(AbsoluteLength::Rems(rems(0.0))),
            "transition: border-radius 1s linear",
        );
        let new = states(
            with_radius(px(10.0).into()),
            "transition: border-radius 1s linear",
        );
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert_eq!(overlay.corner_radii.top_left, Some(px(5.0).into()));
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn mixed_px_rem_endpoints_snap() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(
            with_radius(AbsoluteLength::Rems(rems(1.0))),
            "transition: border-radius 1s linear",
        );
        let new = states(
            with_radius(px(10.0).into()),
            "transition: border-radius 1s linear",
        );
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert_eq!(a.take_diagnostics().len(), 1);
    }

    #[test]
    fn overlay_refines_on_top_of_the_base() {
        use gpui::Refineable;
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear");
        a.on_computed(1, Some(&old), &new, now);

        let mut painted = (*new.base).clone();
        painted.refine(&a.overlay_test(1, now + Duration::from_millis(250)).unwrap());
        assert!((painted.opacity.unwrap() - 0.25).abs() < 1e-3);
    }

    #[test]
    fn per_property_specs_use_their_own_timing() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear, border-top-left-radius 2s linear";
        let old = states(with_opacity_and_radius(0.0, 0.0), css);
        let new = states(with_opacity_and_radius(1.0, 100.0), css);
        a.on_computed(1, Some(&old), &new, now);

        let at = now + Duration::from_millis(1000);
        let overlay = a.overlay(1, &StyleRefinement::default(), at).unwrap();
        assert!((overlay.opacity.unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(overlay.corner_radii.top_left, Some(px(50.0).into()));
        assert!(a.any_active(at));
        assert!(!a.any_active(now + Duration::from_millis(2000)));
    }

    #[test]
    fn easing_shapes_the_curve() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s ease-in");
        let new = states(with_opacity(1.0), "transition: opacity 1s ease-in");
        a.on_computed(1, Some(&old), &new, now);
        let v = opacity_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert!((v - 0.3154).abs() < 2e-3, "{v}");
    }

    #[test]
    fn overshooting_easing_is_clamped_for_opacity() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s cubic-bezier(0.34, 1.8, 0.64, 1)";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        for step in 0..=20 {
            let v = opacity_of(
                &a.overlay_test(1, now + Duration::from_millis(step * 50))
                    .unwrap(),
            );
            assert!((0.0..=1.0).contains(&v), "opacity left 0..1: {v}");
        }
    }

    // -- `--shading` ------------------------------------------------------

    /// Builds a refinement carrying one `--shading` declaration, through the
    /// real mapper — so these tests cover the whole chain, not just the
    /// animator's half of it.
    fn with_shading(value: &str) -> StyleRefinement {
        let mut diagnostics = Vec::new();
        let r = crate::parse_inline(&format!("--shading: {value}"), &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(r.vn_effect.is_some(), "--shading: {value} did not map");
        r
    }

    fn shading_of(r: &StyleRefinement) -> EffectSpec {
        r.vn_effect.expect("overlay carries --shading")
    }

    #[test]
    fn effect_params_interpolate() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("frost(0.2, 8px)"), css);
        let new = states(with_shading("frost(0.8, 20px)"), css);
        a.on_computed(1, Some(&old), &new, now);

        let half = shading_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert_eq!(half.id, gpui::effect_id::FROST);
        assert!((half.params[0] - 0.5).abs() < 1e-3, "{}", half.params[0]);
        assert!((half.params[1] - 14.0).abs() < 1e-3, "{}", half.params[1]);
        // The schema metadata rides along untouched, so the painter still knows
        // which slot is the blur radius.
        assert_eq!(half.backdrop_blur_radius(), half.params[1]);
        assert!(a.take_diagnostics().is_empty());

        let end = shading_of(&a.overlay_test(1, now + Duration::from_millis(1000)).unwrap());
        assert_eq!(end.params[0], 0.8);
        assert_eq!(end.params[1], 20.0);
    }

    #[test]
    fn effect_color_params_interpolate_componentwise() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("glow(#000000, 10px, 0)"), css);
        let new = states(with_shading("glow(#ffffff, 30px, 1)"), css);
        a.on_computed(1, Some(&old), &new, now);
        let half = shading_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert!((half.params[0] - 20.0).abs() < 1e-3);
        assert!((half.params[1] - 0.5).abs() < 1e-3);
        for channel in 4..7 {
            assert!((half.params[channel] - 0.5).abs() < 1e-3, "slot {channel}");
        }
        // …and the dilation follows the interpolated radius, so the halo's quad
        // grows with it instead of popping at the end.
        assert!((half.bleed() - 20.0).abs() < 1e-3);
    }

    #[test]
    fn different_effects_snap_with_an_advisory_naming_both() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("frost(0.5)"), css);
        let new = states(with_shading("glow(10px, 0.5)"), css);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());

        let diags = a.take_diagnostics();
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].contains("frost"), "{diags:?}");
        assert!(diags[0].contains("glow"), "{diags:?}");

        // …and it stays one-shot.
        a.on_computed(2, Some(&old), &new, now);
        assert!(a.take_diagnostics().is_empty());
    }

    /// `all` is the 25 real CSS properties. A custom property is not one of
    /// them — a browser would not animate `--shading` under `all` either —
    /// so it has to be named.
    #[test]
    fn all_does_not_cover_shading_but_naming_it_does() {
        let mut a = Animator::new();
        let now = t0();
        let all = "transition: all 1s linear";
        let old = states(with_shading("frost(0.2)"), all);
        let new = states(with_shading("frost(0.8)"), all);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none(), "`all` must not cover --shading");
        assert!(a.take_diagnostics().is_empty());

        let named = "transition: all 1s linear, --shading 1s linear";
        let old = states(with_shading("frost(0.2)"), named);
        let new = states(with_shading("frost(0.8)"), named);
        a.on_computed(2, Some(&old), &new, now);
        assert!(a.overlay(2, &StyleRefinement::default(), now).is_some());
    }

    /// `--shading: none` is a VALUE, so a transition into it has two present
    /// endpoints with different ids and must snap rather than silently do
    /// nothing.
    #[test]
    fn transitioning_to_none_snaps() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("frost(0.5)"), css);
        let new = states(with_shading("none"), css);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert_eq!(a.take_diagnostics().len(), 1);
    }

    #[test]
    fn an_animated_endpoint_keeps_the_whole_transition_animated() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("noise(0.1, 2px, 0)"), css);
        let new = states(with_shading("noise(0.6, 2px, 24)"), css);
        a.on_computed(1, Some(&old), &new, now);
        let half = shading_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert!(half.animated, "grain must not freeze mid-transition");
        assert!((half.params[0] - 0.35).abs() < 1e-3);
        assert!((half.params[2] - 12.0).abs() < 1e-3);
    }

    // ---- the three colours m22 made animatable (engine-round.md §4.6) ------

    fn rgb(r: f32, g: f32, b: f32) -> Hsla {
        Rgba { r, g, b, a: 1.0 }.into()
    }

    /// A state whose element `color` is `c` and whose SVG paint is `paint`.
    /// Both `fill` and `stroke` get it, because the two are the same question.
    fn svg_states(c: Hsla, paint: Option<crate::SvgPaint>, css: &str) -> ComputedStates {
        let mut base = StyleRefinement::default();
        base.text.color = Some(c);
        let mut s = states(base, css);
        s.extras = crate::Extras { fill: paint, stroke: paint, ..Default::default() };
        s.color = Some(c);
        s
    }

    /// `fill` and `stroke` live in `Extras`, not in a `StyleRefinement`, and
    /// what they animate is the colour the SVG sprite is TINTED with — gpui
    /// rasterises an SVG into a single-colour alpha mask, so the element's own
    /// `color` is the only colour it has. Both ends here declare the same
    /// paint (`currentColor`) and differ only in `color`: a track that did not
    /// follow `color` would find its endpoints equal and never start.
    #[test]
    fn a_fill_track_follows_the_colour_the_sprite_is_tinted_with() {
        for css in ["transition: fill 1s linear", "transition: stroke 1s linear"] {
            let mut a = Animator::new();
            let now = t0();
            let (red, blue) = (rgb(1.0, 0.0, 0.0), rgb(0.0, 0.0, 1.0));
            let old = svg_states(red, Some(crate::SvgPaint::Current), css);
            let new = svg_states(blue, Some(crate::SvgPaint::Current), css);

            a.on_computed(1, Some(&old), &new, now);
            assert!(a.any_active(now), "{css} must start a track");

            // The interpolated paint is written into the tint slot — `text.color`
            // is what the renderer hands the rasteriser.
            let at = |ms: u64| {
                a.overlay_test(1, now + Duration::from_millis(ms))
                    .and_then(|r| r.text.color)
                    .unwrap_or_else(|| panic!("{css}: no tint at {ms}ms"))
            };
            let same = |a: Hsla, b: Hsla| {
                (a.h - b.h).abs() < 1e-4
                    && (a.s - b.s).abs() < 1e-4
                    && (a.l - b.l).abs() < 1e-4
                    && (a.a - b.a).abs() < 1e-4
            };
            assert!(same(at(0), red), "{css} starts where it was: {:?}", at(0));
            assert!(same(at(1000), blue), "{css} ends where it is going: {:?}", at(1000));
            let half = at(500);
            assert!(!same(half, red) && !same(half, blue), "{css} interpolates: {half:?}");
        }
    }

    /// `none` ↔ a paint is a DISCRETE pair — SVG paint is not a continuous type
    /// across that boundary and CSS does not interpolate it either — so the
    /// value snaps and no track is started. (`clear()` staying a no-op for
    /// `fill`/`stroke` is the other half of that: stripping `color` out of a
    /// `:hover` rung on behalf of `fill` would leave a hover colour that only
    /// `transition: color` is entitled to animate painted by nobody.)
    #[test]
    fn an_svg_paint_of_none_snaps_instead_of_interpolating() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: fill 1s linear";
        let old = svg_states(rgb(1.0, 0.0, 0.0), Some(crate::SvgPaint::None), css);
        let new = svg_states(rgb(0.0, 0.0, 1.0), Some(crate::SvgPaint::Current), css);

        a.on_computed(1, Some(&old), &new, now);
        assert!(!a.any_active(now), "an unpainted endpoint has nothing to interpolate");
        assert!(a.overlay_test(1, now).is_none());

        // …and an element that declares no paint at all is not animated either:
        // nothing was asked for, so there is no slot.
        let bare = svg_states(rgb(0.0, 0.0, 1.0), None, css);
        a.on_computed(2, Some(&bare), &bare, now);
        assert!(a.overlay_test(2, now).is_none());
    }

    /// `text-decoration-color` is a FIELD of gpui's `UnderlineStyle` /
    /// `StrikethroughStyle` rather than a slot of its own, so the overlay has
    /// to carry a COMPLETE decoration — thickness, waviness and offset copied
    /// from the base, colour replaced. A bare `UnderlineStyle { color }` would
    /// refine the base's thickness away and paint a hairline.
    #[test]
    fn text_decoration_color_animates_without_refining_the_line_away() {
        fn decorated(c: Hsla) -> StyleRefinement {
            let mut r = StyleRefinement::default();
            r.text.underline = Some(gpui::UnderlineStyle {
                thickness: px(3.0),
                color: Some(c),
                wavy: true,
                offset: px(2.0),
            });
            r
        }
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: text-decoration-color 1s linear";
        let (red, blue) = (rgb(1.0, 0.0, 0.0), rgb(0.0, 0.0, 1.0));
        let old = states(decorated(red), css);
        let new = states(decorated(blue), css);

        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));
        // The element's own base is what the overlay refines — the decoration
        // it must copy thickness, waviness and offset out of lives there.
        let base = decorated(red);
        let half = a.overlay(1, &base, now + Duration::from_millis(500)).unwrap();
        let u = half.text.underline.clone().expect("the overlay carries a decoration");
        assert_eq!(u.thickness, px(3.0), "the base's thickness survives the overlay");
        assert!(u.wavy, "…and so does its waviness");
        assert_eq!(u.offset, px(2.0), "…and its offset");
        let c = u.color.expect("with a colour of its own");
        assert!(c != red && c != blue, "interpolated, not snapped: {c:?}");

        let end = a.overlay(1, &base, now + Duration::from_millis(1000)).unwrap();
        let ended = end.text.underline.and_then(|u| u.color).expect("a colour at the end");
        assert!((ended.h - blue.h).abs() < 1e-4 && (ended.l - blue.l).abs() < 1e-4, "{ended:?}");
    }

    /// All three are in Tailwind's `transition-colors`, so `transition: all`
    /// has to cover them or every use of that utility keeps advising.
    #[test]
    fn the_three_new_properties_are_covered_by_transition_all() {
        let all = crate::transition::TransitionProperty::animatable();
        for p in [
            crate::transition::TransitionProperty::Fill,
            crate::transition::TransitionProperty::Stroke,
            crate::transition::TransitionProperty::TextDecorationColor,
        ] {
            assert!(all.contains(&p), "{p:?} is not in `transition: all`");
        }
    }
}

// ---------------------------------------------------------------------------
// CSS animations (@keyframes)
// ---------------------------------------------------------------------------

/// One running `@keyframes` animation on one node.
///
/// It holds its own `Rc<Keyframes>` rather than a name, because `overlay()` is
/// called from the render walk — which has a node id and a clock and no engine.
/// Resolving the name ONCE, when the run starts, is also what CSS says: an
/// animation samples the `@keyframes` rule that was in effect when it began.
struct AnimRun {
    spec: AnimationSpec,
    frames: Rc<Keyframes>,
    /// When the run's clock started (i.e. when `animation-delay` began).
    start: Instant,
    /// The clock has not been anchored to a FRAME yet.
    ///
    /// CSS Animations §4 starts an animation at the first frame that renders
    /// it, not at the style resolution that created it — `Animation.startTime`
    /// stays null while the animation is "pending" and is set when it becomes
    /// ready. The difference used to be invisible here because the cascade only
    /// ever ran from the frame pump; now that `getComputedStyle` forces a style
    /// flush mid-wave (reka's `<Presence>` needs it), a run can be created a
    /// full mount-and-layout before anything paints, and a 150 ms animation
    /// that starts its clock there is most of the way through by the time it is
    /// first drawn. So the first `tick` re-anchors it, which is unobservable —
    /// nothing has painted and no event has fired yet.
    pending: bool,
    /// `animationstart` has been reported.
    started: bool,
    /// `animationend` has been reported.
    ended: bool,
    /// How long the run has already spent paused, so `animation-play-state`
    /// resumes where it stopped instead of jumping forward.
    paused_for: Duration,
    /// When the current pause began, if it is paused right now.
    paused_at: Option<Instant>,
}

impl AnimRun {
    /// The run's own elapsed time — wall clock minus whatever it spent paused.
    fn elapsed(&self, now: Instant) -> Duration {
        let raw = now.saturating_duration_since(self.start);
        let paused = self.paused_for
            + self
                .paused_at
                .map(|at| now.saturating_duration_since(at))
                .unwrap_or_default();
        raw.saturating_sub(paused)
    }

    fn phase(&self, now: Instant) -> Phase {
        self.spec.phase(self.elapsed(now))
    }

    /// Seconds of animation actually run, for an event's `elapsedTime`.
    fn elapsed_seconds(&self, now: Instant) -> f32 {
        self.elapsed(now)
            .saturating_sub(self.spec.delay)
            .as_secs_f32()
    }
}

/// What the animator has to tell JS about. The host drains these once per frame
/// and dispatches them through the ordinary event path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimEventKind {
    TransitionEnd,
    AnimationStart,
    AnimationEnd,
    AnimationCancel,
}

/// One event to deliver. `name` is the CSS property name for a transition and
/// the `@keyframes` name for an animation — exactly what
/// `TransitionEvent.propertyName` and `AnimationEvent.animationName` carry.
#[derive(Clone, Debug)]
pub struct AnimEvent {
    pub node: u32,
    pub kind: AnimEventKind,
    pub name: String,
    /// `elapsedTime` in SECONDS, excluding the delay — the DOM's definition.
    pub elapsed: f32,
}

impl Animator {
    /// Starts, retargets and cancels this node's `@keyframes` animations.
    ///
    /// Called from the same place `on_computed` is, so it sees the same cascade
    /// output the transitions do. `resolve` turns an `animation-name` into the
    /// sheet's keyframes; it is a callback rather than an `&Engine` so the
    /// animator stays free of a stylesheet dependency and testable without one.
    pub fn on_computed_animations(
        &mut self,
        node: u32,
        new: &ComputedStates,
        now: Instant,
        resolve: &dyn Fn(&str) -> Option<Rc<Keyframes>>,
    ) {
        // `prefers-reduced-motion` suppresses animations exactly as it
        // suppresses transitions: they do not run at all.
        let wanted: Vec<AnimationSpec> = if self.reduce_motion {
            Vec::new()
        } else {
            new.animations.iter().filter(|s| !s.is_inert()).cloned().collect()
        };

        if wanted.is_empty() {
            if let Some(runs) = self.anims.remove(&node) {
                for run in runs {
                    self.cancel_event(node, &run, now);
                }
            }
            return;
        }

        let mut previous = self.anims.remove(&node).unwrap_or_default();
        let mut runs: Vec<AnimRun> = Vec::with_capacity(wanted.len());
        for spec in wanted {
            // CSS restarts an animation when its NAME changes at that position
            // in the list, and only then: changing the duration or the easing
            // retargets a RUNNING animation rather than replaying it. Matching
            // by name (the first unused run carrying it) is what gives that.
            let existing = previous.iter().position(|r| r.spec.name == spec.name);
            match existing {
                Some(i) => {
                    let mut run = previous.remove(i);
                    // `animation-play-state` is a clock edit, not a restart: the
                    // pause debt keeps the phase where it was.
                    match (run.paused_at, spec.play_state) {
                        (None, PlayState::Paused) => run.paused_at = Some(now),
                        (Some(at), PlayState::Running) => {
                            run.paused_for += now.saturating_duration_since(at);
                            run.paused_at = None;
                        }
                        _ => {}
                    }
                    run.spec = spec;
                    // **The block the name refers to can move under a running
                    // animation, and re-resolving is not a restart.** A
                    // `@keyframes` inside an `@media` block is declared only
                    // where that query holds, so a resize across the breakpoint
                    // — or a subtree teleported into a differently-sized
                    // pop-out (docs/MULTIWINDOW.md §6) — changes which block
                    // this name means, and a browser re-resolves it in place.
                    // So does a `var()`-composed block when the element's
                    // custom properties change. Only reached on a pass that
                    // already found this node's computed style different, so a
                    // steady frame pays nothing.
                    if let Some(frames) = resolve(&run.spec.name) {
                        run.frames = frames;
                    }
                    runs.push(run);
                }
                None => {
                    let Some(frames) = resolve(&spec.name) else {
                        // A name with no `@keyframes` block animates nothing —
                        // which is what a browser does too, so this is a likely
                        // author error rather than a divergence.
                        self.warn_once(format!(
                            "`animation-name: {}` names no @keyframes block in the stylesheet — \
                             nothing animates (a browser behaves the same way)",
                            spec.name
                        ));
                        continue;
                    };
                    runs.push(AnimRun {
                        paused_at: (spec.play_state == PlayState::Paused).then_some(now),
                        spec,
                        frames,
                        start: now,
                        pending: true,
                        started: false,
                        ended: false,
                        paused_for: Duration::ZERO,
                    });
                }
            }
        }
        // Anything left in `previous` lost its slot in the list.
        let leftovers: Vec<AnimRun> = previous;
        for run in leftovers {
            self.cancel_event(node, &run, now);
        }
        if runs.is_empty() {
            self.anims.remove(&node);
        } else {
            self.anims.insert(node, runs);
        }
    }

    /// Ends every `@keyframes` animation on `node`, each with an
    /// `animationcancel` (unless it already ended): what CSS Animations §3 does
    /// to an element when it, or an ancestor, becomes `display: none`. A later
    /// [`Animator::on_computed_animations`] for the node starts them afresh,
    /// which is the other half of that rule.
    pub fn cancel_animations(&mut self, node: u32, now: Instant) {
        if let Some(runs) = self.anims.remove(&node) {
            for run in runs {
                self.cancel_event(node, &run, now);
            }
        }
    }

    /// Does `node` hold any `@keyframes` run, running, delayed or filling?
    pub fn has_animations(&self, node: u32) -> bool {
        self.anims.contains_key(&node)
    }

    fn cancel_event(&mut self, node: u32, run: &AnimRun, now: Instant) {
        // A run that already reported `animationend` finished; it was not
        // cancelled, and the DOM fires only one of the two.
        if run.ended {
            return;
        }
        self.events.push(AnimEvent {
            node,
            kind: AnimEventKind::AnimationCancel,
            name: run.spec.name.to_string(),
            elapsed: run.elapsed_seconds(now).max(0.0),
        });
    }

    /// Advances the animation clocks and queues `animationstart`/`animationend`.
    /// Call once per frame, before `overlay`.
    pub fn tick(&mut self, now: Instant) {
        let mut events = Vec::new();
        for (&node, runs) in self.anims.iter_mut() {
            for run in runs.iter_mut() {
                // The first frame is what the clock is measured from — see
                // `AnimRun::pending`. A paused run keeps its pause anchored to
                // the same instant so no debt is invented.
                if run.pending {
                    run.pending = false;
                    run.start = now;
                    if run.paused_at.is_some() {
                        run.paused_at = Some(now);
                    }
                }
                let phase = run.phase(now);
                if !run.started && !matches!(phase, Phase::Delayed) {
                    run.started = true;
                    events.push(AnimEvent {
                        node,
                        kind: AnimEventKind::AnimationStart,
                        name: run.spec.name.to_string(),
                        elapsed: 0.0,
                    });
                }
                if !run.ended && matches!(phase, Phase::Finished { .. }) {
                    run.ended = true;
                    events.push(AnimEvent {
                        node,
                        kind: AnimEventKind::AnimationEnd,
                        name: run.spec.name.to_string(),
                        elapsed: run
                            .spec
                            .total()
                            .map(|t| t.saturating_sub(run.spec.delay).as_secs_f32())
                            .unwrap_or(0.0),
                    });
                }
            }
        }
        self.events.append(&mut events);
    }

    /// Drains the queued transition/animation events.
    pub fn take_events(&mut self) -> Vec<AnimEvent> {
        std::mem::take(&mut self.events)
    }

    /// The nodes whose painted style is moving right now: the ones
    /// [`Animator::any_active`] and [`Animator::any_animation`] answer for, by
    /// node. An unfinished transition, or an unpaused `@keyframes` run that is
    /// delayed or running. A node can be listed twice. The host's
    /// retained-render cache refuses only the subtrees these are in.
    pub fn moving_nodes(&self, now: Instant) -> impl Iterator<Item = u32> + '_ {
        let tracks = self
            .nodes
            .iter()
            .filter(move |(_, tracks)| tracks.iter().any(|t| !t.finished(now)))
            .map(|(&node, _)| node);
        let runs = self
            .anims
            .iter()
            .filter(move |(_, runs)| {
                runs.iter().any(|run| match run.phase(now) {
                    Phase::Finished { .. } => false,
                    Phase::Delayed | Phase::Running { .. } => run.paused_at.is_none(),
                })
            })
            .map(|(&node, _)| node);
        tracks.chain(runs)
    }

    /// Is any animation still running (or holding a `forwards` fill)?
    pub fn any_animation(&self, now: Instant) -> bool {
        self.anims.values().flatten().any(|run| match run.phase(now) {
            Phase::Finished { .. } => false,
            // A paused animation is not a reason to keep repainting; its value
            // is frozen and the frame that froze it already painted.
            Phase::Delayed | Phase::Running { .. } => run.paused_at.is_none(),
        })
    }

    /// The interpolated animation values for `node` this frame.
    ///
    /// The endpoints are the KEYFRAMES rather than the element's previous
    /// computed value, so this is composed with — not merged into — the
    /// transition tracks. CSS Cascade §6.2 puts transitions ABOVE animations,
    /// which is the order [`Animator::overlay`] applies them in.
    fn animation_overlay(
        &self,
        node: u32,
        base: &StyleRefinement,
        now: Instant,
    ) -> Option<StyleRefinement> {
        let runs = self.anims.get(&node)?;
        let mut out: Option<StyleRefinement> = None;
        for run in runs {
            let progress = match run.phase(now) {
                Phase::Running { progress, .. } => progress,
                // `fill-mode` decides whether the ends are painted at all.
                Phase::Delayed => {
                    if !matches!(run.spec.fill, FillMode::Backwards | FillMode::Both) {
                        continue;
                    }
                    // The value the FIRST iteration starts at, direction and
                    // all — that is what `backwards` freezes.
                    run.spec.phase(run.spec.delay).progress_or(0.0)
                }
                Phase::Finished { progress } => {
                    if !matches!(run.spec.fill, FillMode::Forwards | FillMode::Both) {
                        continue;
                    }
                    progress
                }
            };
            let r = out.get_or_insert_with(StyleRefinement::default);
            apply_keyframes(r, base, &run.frames, &run.spec, progress);
        }
        out
    }
}

impl Phase {
    fn progress_or(self, default: f32) -> f32 {
        match self {
            Phase::Running { progress, .. } | Phase::Finished { progress } => progress,
            Phase::Delayed => default,
        }
    }
}

/// Samples every animatable property any keyframe declares, at `progress`.
fn apply_keyframes(
    out: &mut StyleRefinement,
    base: &StyleRefinement,
    frames: &Keyframes,
    spec: &AnimationSpec,
    progress: f32,
) {
    for property in TransitionProperty::animatable() {
        let declares: &dyn Fn(&StyleRefinement) -> bool =
            &|r: &StyleRefinement| slot_of(r, &property) != Slot::Missing;
        let Some(segment) = frames.segment(declares, progress) else {
            continue;
        };
        // An implicit end (no `from`/`to` for this property) is the element's
        // own computed value — CSS Animations §4 — and where the element
        // declares nothing either, the property's INITIAL value. That last
        // fallback is not a nicety: Tailwind's `pulse` is `@keyframes pulse {
        // 50% { opacity: .5 } }` and nothing else, so without it both ends of
        // every segment are unset and the Skeleton sits at 0.5 instead of
        // breathing between 1 and 0.5.
        let implicit = || match slot_of(base, &property) {
            Slot::Missing => initial(&property).map_or(Slot::Missing, Slot::Value),
            slot => slot,
        };
        let from = segment
            .from
            .map(|k| slot_of(&k.style, &property))
            .unwrap_or_else(implicit);
        let to = segment
            .to
            .map(|k| slot_of(&k.style, &property))
            .unwrap_or_else(implicit);
        let (Slot::Value(a), Slot::Value(b)) = (from, to) else {
            // One end is `auto`, a percentage or unset. CSS cannot interpolate
            // those either; the discrete behaviour is to hold whichever end
            // does have a value.
            if let Slot::Value(v) = from {
                install(out, base, &property, v);
            } else if let Slot::Value(v) = to {
                install(out, base, &property, v);
            }
            continue;
        };
        let Some((a, b)) = unify(a, b) else {
            install(out, base, &property, if segment.t < 0.5 { a } else { b });
            continue;
        };
        let easing = segment.easing.unwrap_or(spec.easing);
        install(out, base, &property, lerp(a, b, easing.eval(segment.t.clamp(0.0, 1.0))));
    }
}
