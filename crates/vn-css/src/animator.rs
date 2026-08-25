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
use std::time::{Duration, Instant};

use gpui::{
    AbsoluteLength, DefiniteLength, Fill, Hsla, Interpolate, Length, Pixels, Rems, StyleRefinement,
};

use crate::transition::{covered, spec_for, Easing, TransitionProperty};
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
}

/// Why a slot cannot be interpolated (kept comparable so an *unchanged*
/// non-animatable value never triggers an advisory).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Opaque {
    /// `auto` / `none`
    Auto,
    /// A percentage (gpui `DefiniteLength::Fraction`)
    Fraction(f32),
    /// A gradient or pattern fill
    NonSolidFill,
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
            Some(Fill::Color(bg)) => match bg.as_solid() {
                Some(c) => Slot::Value(Animatable::Color(c)),
                None => Slot::Opaque(Opaque::NonSolidFill),
            },
        },
        BorderColor => slot_color(r.border_color),
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
        All | Other(_) => Slot::Missing,
    }
}

/// Writes one interpolated value into an overlay refinement.
fn install(r: &mut StyleRefinement, p: &TransitionProperty, v: Animatable) {
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
        BackgroundColor => r.background = color().map(|c| Fill::Color(c.into())),
        BorderColor => r.border_color = color(),
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
        All | Other(_) => {}
    }
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
            // cancelled and the value snaps, as in CSS.
            self.nodes.remove(&node);
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
                None => slot_of(&old.base, property),
            };
            let to_slot = slot_of(&new.base, property);

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

    /// The interpolated values for `node` this frame, to be refined ON TOP of
    /// its computed base. `None` when the node has nothing running.
    pub fn overlay(&self, node: u32, now: Instant) -> Option<StyleRefinement> {
        let tracks = self.nodes.get(&node)?;
        if tracks.is_empty() {
            return None;
        }
        let mut r = StyleRefinement::default();
        for track in tracks {
            install(&mut r, &track.property, track.value(now));
        }
        Some(r)
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
    pub fn finish_settled(&mut self, now: Instant) {
        self.nodes.retain(|_, tracks| {
            tracks.retain(|t| !t.finished(now));
            !tracks.is_empty()
        });
    }

    /// Drops state for nodes that no longer exist.
    pub fn gc(&mut self, alive: &dyn Fn(u32) -> bool) {
        self.nodes.retain(|&id, _| alive(id));
    }

    /// Forgets everything (e.g. on a stylesheet swap that resets the tree).
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Whether any node has transition state at all.
    pub fn is_idle(&self) -> bool {
        self.nodes.is_empty()
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
                self.warn_once(format!(
                    "transition of {} between different length units is not animatable yet (v1) \
                     — snapping",
                    property.name()
                ));
                Endpoints::Snap
            }
        }
    }
}

enum Endpoints {
    Pair(Animatable, Animatable),
    /// Not interpolatable (or unchanged): the value jumps.
    Snap,
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
            hover: None,
            active: None,
            transitions: Rc::new(transitions),
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
        assert!(a.overlay(1, now).is_none());
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
        assert!((opacity_of(&a.overlay(1, now).unwrap()) - 0.0).abs() < 1e-6);

        let half = now + Duration::from_millis(500);
        assert!((opacity_of(&a.overlay(1, half).unwrap()) - 0.5).abs() < 1e-3);

        let end = now + Duration::from_millis(1000);
        assert!((opacity_of(&a.overlay(1, end).unwrap()) - 1.0).abs() < 1e-6);
        assert!(!a.any_active(end));

        a.finish_settled(end);
        assert!(a.overlay(1, end).is_none());
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
            let v = opacity_of(&a.overlay(1, now + Duration::from_millis(at)).unwrap());
            assert!(v.abs() < 1e-6, "value moved during the delay at {at}ms: {v}");
        }
        let v = opacity_of(&a.overlay(1, now + Duration::from_millis(1000)).unwrap());
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
        assert!(a.overlay(1, now).is_none());
    }

    #[test]
    fn zero_duration_with_delay_jumps_after_the_delay() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 0s 300ms");
        let new = states(with_opacity(1.0), "transition: opacity 0s 300ms");
        a.on_computed(1, Some(&old), &new, now);
        assert!(opacity_of(&a.overlay(1, now + Duration::from_millis(100)).unwrap()) < 1e-6);
        assert!(
            (opacity_of(&a.overlay(1, now + Duration::from_millis(400)).unwrap()) - 1.0).abs()
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
        let at_interrupt = opacity_of(&a.overlay(1, mid).unwrap());
        assert!((at_interrupt - 0.4).abs() < 1e-3, "{at_interrupt}");

        // Reverse mid-flight: the very next sample must equal what was shown.
        a.on_computed(1, Some(&one), &zero, mid);
        let resumed = opacity_of(&a.overlay(1, mid).unwrap());
        assert!(
            (resumed - at_interrupt).abs() < 1e-4,
            "jumped on retarget: {at_interrupt} -> {resumed}"
        );

        // …and then it travels toward the new target, not the old one.
        let later = mid + Duration::from_millis(200);
        let v = opacity_of(&a.overlay(1, later).unwrap());
        assert!(v < at_interrupt, "{v} should be below {at_interrupt}");
        assert!((v - 0.32).abs() < 1e-2, "{v}");
        let done = mid + Duration::from_millis(1000);
        assert!(opacity_of(&a.overlay(1, done).unwrap()).abs() < 1e-6);
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
        let v = opacity_of(&a.overlay(1, mid).unwrap());
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
            r.background = Some(Fill::Color(c.into()));
            r
        }
        let old = states(mk(red), "transition: background-color 1s linear");
        let new = states(mk(blue), "transition: background-color 1s linear");
        a.on_computed(1, Some(&old), &new, now);

        let half = a.overlay(1, now + Duration::from_millis(500)).unwrap();
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
        let overlay = a.overlay(1, now + Duration::from_millis(250)).unwrap();
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
        let overlay = a.overlay(1, now + Duration::from_millis(500)).unwrap();
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
        assert!(a.overlay(1, now).is_none());
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
        assert!(a.overlay(1, now).is_none());
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
        let old = states(with_opacity(0.0), "transition: transform 1s");
        let new = states(with_opacity(1.0), "transition: transform 1s");
        a.on_computed(1, Some(&old), &new, now);
        a.on_computed(2, Some(&old), &new, now);
        let diags = a.take_diagnostics();
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].contains("transform"), "{diags:?}");
        // …and nothing animates.
        assert!(a.overlay(1, now).is_none());
    }

    #[test]
    fn all_covers_every_animatable_property() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity_and_radius(0.0, 0.0), "transition: all 1s linear");
        let new = states(with_opacity_and_radius(1.0, 8.0), "transition: all 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay(1, now + Duration::from_millis(500)).unwrap();
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
        assert!(a.overlay(1, mid).is_none());
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
        assert!(a.overlay(1, mid).unwrap().corner_radii.top_left.is_some());

        let narrowed = states(
            with_opacity_and_radius(1.0, 8.0),
            "transition: opacity 1s linear",
        );
        a.on_computed(1, Some(&new), &narrowed, mid);
        let overlay = a.overlay(1, mid).unwrap();
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
        assert!(a.overlay(2, now).is_some());

        a.gc(&|id| id == 1);
        assert!(a.overlay(2, now).is_none());
        assert!(a.overlay(1, now).is_some());

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
        assert!(a.overlay(1, now).is_none());

        a.set_reduce_motion(false);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, now).is_some());
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
        let overlay = a.overlay(1, now + Duration::from_millis(500)).unwrap();
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
        assert!(a.overlay(1, now).is_none());
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
        painted.refine(&a.overlay(1, now + Duration::from_millis(250)).unwrap());
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
        let overlay = a.overlay(1, at).unwrap();
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
        let v = opacity_of(&a.overlay(1, now + Duration::from_millis(500)).unwrap());
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
                &a.overlay(1, now + Duration::from_millis(step * 50))
                    .unwrap(),
            );
            assert!((0.0..=1.0).contains(&v), "opacity left 0..1: {v}");
        }
    }
}
