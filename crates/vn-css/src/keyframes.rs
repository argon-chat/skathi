//! CSS animations: `@keyframes` blocks and the `animation` shorthand.
//!
//! # Why this is not just "transitions with more steps"
//!
//! A transition is a diff: the cascade produces a new value, the animator
//! interpolates from the old one to it, and the ELEMENT'S OWN computed value is
//! the target. An animation has no diff at all — the keyframes ARE the values,
//! the element's computed style is only the fallback for a property a keyframe
//! does not name, and the clock is a loop rather than a one-shot.
//!
//! So it rides its own channel end to end: `@keyframes` blocks compile into
//! [`Keyframes`] on the [`crate::Engine`] (they are document-scoped, not
//! per-element), the `animation-*` declarations fold into
//! [`AnimationSpec`]s on the element (they are per-element), and the animator
//! joins the two at run time.
//!
//! # The parts that are easy to get subtly wrong
//!
//! * **Per-property keyframe lists.** CSS says a keyframe only participates in
//!   the timeline of the properties it actually declares. `0% { opacity: 0 }
//!   50% { transform: … } 100% { opacity: 1 }` interpolates `opacity` straight
//!   from 0 % to 100 % — the 50 % frame is not a stop for it. So the timeline is
//!   resolved per property, not per keyframe.
//! * **The implicit 0 % / 100 % frames.** A property missing from the first or
//!   last keyframe takes the element's own computed value there.
//! * **`animation-timing-function` inside a keyframe** applies to the segment
//!   that STARTS at that keyframe, not to the whole animation.
//! * **`direction` and `iteration-count` compose**: `alternate` reverses on odd
//!   iterations, `alternate-reverse` on even ones, and a fractional final
//!   iteration stops part way through whichever direction it is on.

use std::rc::Rc;
use std::time::Duration;

use gpui::StyleRefinement;

use crate::transition::Easing;

/// One resolved `@keyframes` step.
pub struct Keyframe {
    /// Position on the timeline, 0..=1 (`from` is 0, `to` is 1).
    pub offset: f32,
    /// `animation-timing-function` declared INSIDE the block — the easing for
    /// the segment starting here, overriding the animation's own.
    pub easing: Option<Easing>,
    /// The declarations, already mapped.
    pub style: Rc<StyleRefinement>,
    /// The declarations that referenced `var()` or `env()`, kept as raw text.
    ///
    /// A `@keyframes` block is DOCUMENT-scoped but a custom property is
    /// PER-ELEMENT, so these cannot be mapped when the block is compiled — they
    /// are substituted against the animating element's own variable map by
    /// [`crate::Engine::keyframes_for`] when a run starts. This is not exotic:
    /// the whole of `tailwindcss-animate` is written this way
    /// (`@keyframes enter { from { opacity: var(--tw-enter-opacity, 1);
    /// transform: … scale3d(var(--tw-enter-scale, 1), …) } }`), so dropping
    /// them made every `animate-in` / `animate-out` a correctly-timed no-op.
    pub dynamic: Option<Rc<crate::mapper::Decls>>,
}

/// One compiled `@keyframes` rule: its steps, sorted by offset.
pub struct Keyframes {
    pub steps: Vec<Keyframe>,
    /// Does any step carry [`Keyframe::dynamic`]? One bool so the common
    /// var()-free block is shared by `Rc` with no per-element work at all.
    pub dynamic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillMode {
    #[default]
    None,
    Forwards,
    Backwards,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Running,
    Paused,
}

/// One entry of the folded `animation` list on an element.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationSpec {
    /// The `@keyframes` name. An empty name means `animation-name: none`.
    pub name: Rc<str>,
    pub duration: Duration,
    pub delay: Duration,
    /// `f32::INFINITY` for `infinite`.
    pub iterations: f32,
    pub direction: Direction,
    pub fill: FillMode,
    /// The animation's own `animation-timing-function` — the default easing for
    /// every segment a keyframe does not override.
    pub easing: Easing,
    pub play_state: PlayState,
}

impl AnimationSpec {
    /// An animation with no name, no duration and no iterations does nothing —
    /// `animation: none` and a `@keyframes` block that does not exist both land
    /// here, and both are "no animation" rather than an error.
    pub fn is_inert(&self) -> bool {
        self.name.is_empty() || (self.duration.is_zero() && self.delay.is_zero())
    }

    /// When this animation's LAST iteration ends, measured from its start.
    /// `None` for `infinite`.
    pub fn total(&self) -> Option<Duration> {
        if !self.iterations.is_finite() {
            return None;
        }
        Some(self.delay + self.duration.mul_f32(self.iterations.max(0.0)))
    }

    /// Progress within one iteration at `elapsed` since the animation started,
    /// already through `direction` and `iteration-count`, plus which iteration
    /// it is on. `None` while still inside the delay (and the caller decides
    /// what `fill-mode` makes of that).
    ///
    /// After the last iteration the value sticks at the END of the final
    /// iteration — which is what `fill-mode: forwards` paints and what the
    /// unfilled case simply stops using.
    pub fn phase(&self, elapsed: Duration) -> Phase {
        if elapsed < self.delay {
            return Phase::Delayed;
        }
        let running = elapsed - self.delay;
        if self.duration.is_zero() {
            return Phase::Finished {
                progress: self.direct(self.iterations.max(1.0), 1.0),
            };
        }
        let raw = running.as_secs_f64() / self.duration.as_secs_f64();
        if raw >= self.iterations as f64 {
            // CSS: the final value is the end of the LAST iteration, which for
            // a fractional count is part way through it.
            let last = self.iterations.max(0.0);
            let whole = if last.fract() == 0.0 {
                (last - 1.0).max(0.0)
            } else {
                last.floor()
            };
            let frac = if last.fract() == 0.0 {
                1.0
            } else {
                last.fract()
            };
            return Phase::Finished {
                progress: self.direct(whole, frac),
            };
        }
        let iteration = raw.floor() as f32;
        Phase::Running {
            iteration,
            progress: self.direct(iteration, (raw - raw.floor()) as f32),
        }
    }

    /// Apply `animation-direction` to a within-iteration progress.
    fn direct(&self, iteration: f32, progress: f32) -> f32 {
        let odd = (iteration as i64) % 2 != 0;
        let reverse = match self.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => odd,
            Direction::AlternateReverse => !odd,
        };
        if reverse {
            1.0 - progress
        } else {
            progress
        }
    }
}

/// Where an animation is on its own clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Inside `animation-delay` — nothing is applied unless `fill-mode`
    /// includes `backwards`.
    Delayed,
    Running {
        iteration: f32,
        progress: f32,
    },
    /// Past the last iteration; `progress` is the value it settled on, applied
    /// only when `fill-mode` includes `forwards`.
    Finished {
        progress: f32,
    },
}

impl Keyframes {
    /// The two steps that bracket `progress` for one property, plus how far
    /// between them it is and which easing governs the segment.
    ///
    /// `None` when no keyframe declares this property at all. Steps that do not
    /// declare it are skipped rather than treated as a stop — that is CSS's
    /// per-property timeline, and getting it wrong makes a three-keyframe
    /// animation visibly stutter.
    pub fn segment<'a>(
        &'a self,
        declares: &dyn Fn(&StyleRefinement) -> bool,
        progress: f32,
    ) -> Option<Segment<'a>> {
        let mut before: Option<&Keyframe> = None;
        let mut after: Option<&Keyframe> = None;
        for step in self.steps.iter().filter(|s| declares(&s.style)) {
            if step.offset <= progress {
                before = Some(step);
            } else {
                after = Some(step);
                break;
            }
        }
        // The implicit ends sit at 0 % and 100 % and carry the ELEMENT's value,
        // so a segment that runs to one of them still spans a real interval —
        // `@keyframes fade { to { opacity: 1 } }` interpolates across the whole
        // timeline, it does not sit at the start until the last instant.
        let span = |a: f32, b: f32| {
            let width = b - a;
            if width > 0.0 {
                ((progress - a) / width).clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        match (before, after) {
            (None, None) => None,
            (None, Some(to)) => Some(Segment {
                from: None,
                to: Some(to),
                t: span(0.0, to.offset),
                easing: None,
            }),
            // A keyframe AT 100 % is the end of the timeline. CSS adds an
            // implicit 100 % carrying the element's own value only for a
            // property NO keyframe declares there, so there is nothing after
            // this one to interpolate towards — `to: Some(from)` is that, and
            // it says so without depending on an easing curve's value at 0.
            //
            // Without it, `@keyframes fade { to { opacity: 0.2 } }` with
            // `animation-fill-mode: forwards` snapped back to the element's own
            // opacity the instant it finished, which is the one thing
            // `forwards` exists to prevent. It only ever showed with a fill
            // mode that paints the end, which is why it survived: with
            // `fill: none` the overlay is dropped at that exact moment anyway.
            (Some(from), None) if from.offset >= 1.0 => Some(Segment {
                from: Some(from),
                to: Some(from),
                t: 1.0,
                easing: from.easing,
            }),
            (Some(from), None) => Some(Segment {
                from: Some(from),
                to: None,
                t: span(from.offset, 1.0),
                easing: from.easing,
            }),
            (Some(from), Some(to)) => Some(Segment {
                from: Some(from),
                to: Some(to),
                t: span(from.offset, to.offset),
                easing: from.easing,
            }),
        }
    }
}

/// The `animation-*` LONGHANDS a block declares without naming an animation.
///
/// `animation-play-state: paused` on its own must not delete the animation the
/// stylesheet started — but the folded list is one value, so a block that says
/// only "paused" has no list of its own to contribute. It contributes a PATCH
/// instead, applied over whichever list won the cascade.
///
/// This is not a corner case: reka-ui's `Presence` writes
/// `style.animationFillMode = "forwards"` on an element mid-exit-animation to
/// pin its last frame, and without the patch that single inline longhand
/// cancelled the very animation it was trying to freeze.
/// Aggregate callers may still construct a scalar patch with struct update:
/// ```
/// use std::time::Duration;
/// use vn_css::{AnimationDeclarations, AnimationPatch};
/// let patch = AnimationPatch {
///     duration: Some(Duration::from_millis(200)),
///     ..Default::default()
/// };
/// assert_eq!(patch.duration, Some(Duration::from_millis(200)));
/// let _: AnimationDeclarations = Default::default();
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationPatch {
    pub duration: Option<Duration>,
    pub delay: Option<Duration>,
    pub easing: Option<Easing>,
    pub iterations: Option<f32>,
    pub direction: Option<Direction>,
    pub fill: Option<FillMode>,
    pub play_state: Option<PlayState>,
    // Preserve independently declared lists until the complete cascade has
    // folded. A later animation-name must not reset an earlier duration, and
    // duration lists must keep cycling even when a later rule changes names.
    #[doc(hidden)]
    pub declarations: Option<Rc<AnimationDeclarations>>,
}

impl AnimationPatch {
    pub fn is_empty(&self) -> bool {
        *self == AnimationPatch::default()
    }

    /// Merges this patch into `into`, later-wins per field — the same fold the
    /// refinements get, so two rules each setting one longhand compose.
    pub fn apply_over(&self, into: &mut AnimationPatch) {
        if self.duration.is_some() {
            into.duration = self.duration;
        }
        if self.delay.is_some() {
            into.delay = self.delay;
        }
        if self.easing.is_some() {
            into.easing = self.easing;
        }
        if self.iterations.is_some() {
            into.iterations = self.iterations;
        }
        if self.direction.is_some() {
            into.direction = self.direction;
        }
        if self.fill.is_some() {
            into.fill = self.fill;
        }
        if self.play_state.is_some() {
            into.play_state = self.play_state;
        }
        if let Some(declarations) = &self.declarations {
            let mut merged = into.declarations.as_deref().cloned().unwrap_or_default();
            merged.merge(declarations);
            into.declarations = Some(Rc::new(merged));
        } else if self.duration.is_some()
            || self.delay.is_some()
            || self.easing.is_some()
            || self.iterations.is_some()
            || self.direction.is_some()
            || self.fill.is_some()
            || self.play_state.is_some()
        {
            let legacy = AnimationCollector {
                durations: self.duration.map(|value| vec![value]),
                delays: self.delay.map(|value| vec![value]),
                easings: self.easing.map(|value| vec![value]),
                iterations: self.iterations.map(|value| vec![value]),
                directions: self.direction.map(|value| vec![value]),
                fills: self.fill.map(|value| vec![value]),
                play_states: self.play_state.map(|value| vec![value]),
                seen: true,
                ..Default::default()
            };
            let mut merged = into.declarations.as_deref().cloned().unwrap_or_default();
            merged.merge(&legacy);
            into.declarations = Some(Rc::new(merged));
        }
    }

    pub(crate) fn computed_list(&self) -> Option<Vec<AnimationSpec>> {
        self.declarations
            .as_ref()
            .map(|declarations| declarations.specs())
    }

    pub fn apply(&self, spec: &mut AnimationSpec) {
        if let Some(v) = self.duration {
            spec.duration = v;
        }
        if let Some(v) = self.delay {
            spec.delay = v;
        }
        if let Some(v) = self.easing {
            spec.easing = v;
        }
        if let Some(v) = self.iterations {
            spec.iterations = v;
        }
        if let Some(v) = self.direction {
            spec.direction = v;
        }
        if let Some(v) = self.fill {
            spec.fill = v;
        }
        if let Some(v) = self.play_state {
            spec.play_state = v;
        }
    }
}

/// One property's position between two keyframes.
pub struct Segment<'a> {
    /// `None` means "the element's own computed value" (the implicit 0 %).
    pub from: Option<&'a Keyframe>,
    /// `None` means "the element's own computed value" (the implicit 100 %).
    pub to: Option<&'a Keyframe>,
    /// 0..=1 between them, BEFORE easing.
    pub t: f32,
    /// The easing declared on the segment's starting keyframe, if any.
    pub easing: Option<Easing>,
}

// ---------------------------------------------------------------------------
// the `animation-*` declarations
// ---------------------------------------------------------------------------

/// Collects a rule's `animation` shorthand and longhands into a list, the same
/// shape [`crate::transition::Collector`] has for `transition` — and for the
/// same reason: the four (here eight) component lists are independent and CSS
/// zips them by CYCLING the shorter ones, with `animation-name` deciding how
/// many animations there are.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationDeclarations {
    names: Option<Vec<Rc<str>>>,
    durations: Option<Vec<Duration>>,
    delays: Option<Vec<Duration>>,
    easings: Option<Vec<Easing>>,
    iterations: Option<Vec<f32>>,
    directions: Option<Vec<Direction>>,
    fills: Option<Vec<FillMode>>,
    play_states: Option<Vec<PlayState>>,
    seen: bool,
    /// Did the block name an animation (`animation` shorthand or
    /// `animation-name`)? Without one it has no LIST to contribute, only a
    /// patch — see [`AnimationPatch`].
    named: bool,
}

pub(crate) type AnimationCollector = AnimationDeclarations;

impl AnimationDeclarations {
    /// Consumes `p` when it is an animation declaration; returns whether it did.
    pub(crate) fn take(
        &mut self,
        p: &lightningcss::properties::Property,
        diagnostics: &mut Vec<String>,
    ) -> bool {
        use lightningcss::properties::animation as a;
        use lightningcss::properties::Property as P;

        fn name_of(n: &a::AnimationName) -> Rc<str> {
            match n {
                a::AnimationName::None => "".into(),
                a::AnimationName::Ident(i) => i.0.as_ref().into(),
                a::AnimationName::String(s) => s.as_ref().into(),
            }
        }
        fn count_of(c: &a::AnimationIterationCount) -> f32 {
            match c {
                a::AnimationIterationCount::Infinite => f32::INFINITY,
                a::AnimationIterationCount::Number(n) => n.max(0.0),
            }
        }
        fn dir_of(d: &a::AnimationDirection) -> Direction {
            match d {
                a::AnimationDirection::Normal => Direction::Normal,
                a::AnimationDirection::Reverse => Direction::Reverse,
                a::AnimationDirection::Alternate => Direction::Alternate,
                a::AnimationDirection::AlternateReverse => Direction::AlternateReverse,
            }
        }
        fn fill_of(f: &a::AnimationFillMode) -> FillMode {
            match f {
                a::AnimationFillMode::None => FillMode::None,
                a::AnimationFillMode::Forwards => FillMode::Forwards,
                a::AnimationFillMode::Backwards => FillMode::Backwards,
                a::AnimationFillMode::Both => FillMode::Both,
            }
        }
        fn play_of(s: &a::AnimationPlayState) -> PlayState {
            match s {
                a::AnimationPlayState::Running => PlayState::Running,
                a::AnimationPlayState::Paused => PlayState::Paused,
            }
        }

        match p {
            P::Animation(list, _) => {
                self.seen = true;
                self.named = true;
                // The shorthand resets every longhand.
                self.names = Some(list.iter().map(|e| name_of(&e.name)).collect());
                self.durations = Some(
                    list.iter()
                        .map(|e| {
                            crate::transition::duration_of(
                                &e.duration,
                                "animation-duration",
                                diagnostics,
                            )
                        })
                        .collect(),
                );
                self.delays = Some(
                    list.iter()
                        .map(|e| {
                            crate::transition::duration_of(&e.delay, "animation-delay", diagnostics)
                        })
                        .collect(),
                );
                self.easings = Some(
                    list.iter()
                        .map(|e| crate::transition::easing_of(&e.timing_function, diagnostics))
                        .collect(),
                );
                self.iterations = Some(list.iter().map(|e| count_of(&e.iteration_count)).collect());
                self.directions = Some(list.iter().map(|e| dir_of(&e.direction)).collect());
                self.fills = Some(list.iter().map(|e| fill_of(&e.fill_mode)).collect());
                self.play_states = Some(list.iter().map(|e| play_of(&e.play_state)).collect());
                if list
                    .iter()
                    .any(|e| !matches!(e.timeline, a::AnimationTimeline::Auto))
                {
                    crate::mapper::once(
                        diagnostics,
                        "`animation-timeline` is not supported (v1) — scroll/view timelines have \
                         no clock here; the animation runs on the document timeline instead"
                            .into(),
                    );
                }
                true
            }
            P::AnimationName(list, _) => {
                self.seen = true;
                self.named = true;
                self.names = Some(list.iter().map(name_of).collect());
                true
            }
            P::AnimationDuration(list, _) => {
                self.seen = true;
                self.durations = Some(
                    list.iter()
                        .map(|t| {
                            crate::transition::duration_of(t, "animation-duration", diagnostics)
                        })
                        .collect(),
                );
                true
            }
            P::AnimationDelay(list, _) => {
                self.seen = true;
                self.delays = Some(
                    list.iter()
                        .map(|t| crate::transition::duration_of(t, "animation-delay", diagnostics))
                        .collect(),
                );
                true
            }
            P::AnimationTimingFunction(list, _) => {
                self.seen = true;
                self.easings = Some(
                    list.iter()
                        .map(|e| crate::transition::easing_of(e, diagnostics))
                        .collect(),
                );
                true
            }
            P::AnimationIterationCount(list, _) => {
                self.seen = true;
                self.iterations = Some(list.iter().map(count_of).collect());
                true
            }
            P::AnimationDirection(list, _) => {
                self.seen = true;
                self.directions = Some(list.iter().map(dir_of).collect());
                true
            }
            P::AnimationFillMode(list, _) => {
                self.seen = true;
                self.fills = Some(list.iter().map(fill_of).collect());
                true
            }
            P::AnimationPlayState(list, _) => {
                self.seen = true;
                self.play_states = Some(list.iter().map(play_of).collect());
                true
            }
            P::AnimationTimeline(..) => {
                self.seen = true;
                crate::mapper::once(
                    diagnostics,
                    "`animation-timeline` is not supported (v1) — scroll/view timelines have no \
                     clock here; the animation runs on the document timeline instead"
                        .into(),
                );
                true
            }
            P::Unparsed(u) if u.property_id.name().starts_with("animation") => {
                self.seen = true;
                crate::mapper::once(
                    diagnostics,
                    format!(
                        "unresolved value for {} (var()/env() in animations is not resolved yet, \
                         v1)",
                        u.property_id.name()
                    ),
                );
                true
            }
            _ => false,
        }
    }

    /// Cascade each declared longhand independently. A shorthand populated
    /// every list in take(), so it resets all eight; a name-only rule replaces
    /// names alone. Keep original list lengths until the final zip.
    pub(crate) fn merge(&mut self, later: &Self) {
        macro_rules! field {
            ($field:ident) => {
                if later.$field.is_some() {
                    self.$field = later.$field.clone();
                }
            };
        }
        field!(names);
        field!(durations);
        field!(delays);
        field!(easings);
        field!(iterations);
        field!(directions);
        field!(fills);
        field!(play_states);
        self.seen |= later.seen;
        self.named |= later.named;
    }

    pub(crate) fn from_specs(specs: &[AnimationSpec]) -> Self {
        Self {
            names: Some(specs.iter().map(|spec| spec.name.clone()).collect()),
            durations: Some(specs.iter().map(|spec| spec.duration).collect()),
            delays: Some(specs.iter().map(|spec| spec.delay).collect()),
            easings: Some(specs.iter().map(|spec| spec.easing).collect()),
            iterations: Some(specs.iter().map(|spec| spec.iterations).collect()),
            directions: Some(specs.iter().map(|spec| spec.direction).collect()),
            fills: Some(specs.iter().map(|spec| spec.fill).collect()),
            play_states: Some(specs.iter().map(|spec| spec.play_state).collect()),
            seen: true,
            named: true,
        }
    }

    /// Per-block list remains available to keyframe compilation and diagnostics;
    /// the patch also carries declared lists for the per-element cascade.
    pub(crate) fn finish(self) -> (Option<Vec<AnimationSpec>>, Option<AnimationPatch>) {
        if !self.seen {
            return (None, None);
        }
        let list = self.named.then(|| self.specs());
        let patch = AnimationPatch {
            duration: self
                .durations
                .as_ref()
                .and_then(|list| list.first().copied()),
            delay: self.delays.as_ref().and_then(|list| list.first().copied()),
            easing: self.easings.as_ref().and_then(|list| list.first().copied()),
            iterations: self
                .iterations
                .as_ref()
                .and_then(|list| list.first().copied()),
            direction: self
                .directions
                .as_ref()
                .and_then(|list| list.first().copied()),
            fill: self.fills.as_ref().and_then(|list| list.first().copied()),
            play_state: self
                .play_states
                .as_ref()
                .and_then(|list| list.first().copied()),
            declarations: Some(Rc::new(self)),
        };
        (list, Some(patch))
    }

    /// Zip after the cascade, cycling each shorter component list. Names alone
    /// determine the number of animations, including a name that is none.
    fn specs(&self) -> Vec<AnimationSpec> {
        let names = self.names.as_deref().unwrap_or_default();
        let mut out = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            out.push(AnimationSpec {
                name: name.clone(),
                duration: cycle(&self.durations, i, Duration::ZERO),
                delay: cycle(&self.delays, i, Duration::ZERO),
                iterations: cycle(&self.iterations, i, 1.0),
                direction: cycle(&self.directions, i, Direction::Normal),
                fill: cycle(&self.fills, i, FillMode::None),
                easing: cycle(&self.easings, i, Easing::Ease),
                play_state: cycle(&self.play_states, i, PlayState::Running),
            });
        }
        out
    }
}

fn cycle<T: Copy>(list: &Option<Vec<T>>, i: usize, default: T) -> T {
    match list {
        Some(l) if !l.is_empty() => l[i % l.len()],
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(iterations: f32, direction: Direction) -> AnimationSpec {
        AnimationSpec {
            name: "spin".into(),
            duration: Duration::from_millis(1000),
            delay: Duration::ZERO,
            iterations,
            direction,
            fill: FillMode::None,
            easing: Easing::Linear,
            play_state: PlayState::Running,
        }
    }

    #[test]
    fn direction_reverses_the_right_iterations() {
        let alt = spec(4.0, Direction::Alternate);
        // Iteration 0 forwards, 1 backwards, 2 forwards…
        assert_eq!(
            alt.phase(Duration::from_millis(250)),
            Phase::Running {
                iteration: 0.0,
                progress: 0.25
            }
        );
        assert_eq!(
            alt.phase(Duration::from_millis(1250)),
            Phase::Running {
                iteration: 1.0,
                progress: 0.75
            }
        );
        assert_eq!(
            alt.phase(Duration::from_millis(2250)),
            Phase::Running {
                iteration: 2.0,
                progress: 0.25
            }
        );

        let rev = spec(2.0, Direction::Reverse);
        assert_eq!(
            rev.phase(Duration::from_millis(250)),
            Phase::Running {
                iteration: 0.0,
                progress: 0.75
            }
        );

        let altrev = spec(2.0, Direction::AlternateReverse);
        assert_eq!(
            altrev.phase(Duration::from_millis(250)),
            Phase::Running {
                iteration: 0.0,
                progress: 0.75
            }
        );
        assert_eq!(
            altrev.phase(Duration::from_millis(1250)),
            Phase::Running {
                iteration: 1.0,
                progress: 0.25
            }
        );
    }

    #[test]
    fn the_end_value_is_the_end_of_the_last_iteration() {
        // An even count on `alternate` ends where it STARTED, because the last
        // iteration ran backwards. This is the classic fill-mode surprise.
        let alt = spec(2.0, Direction::Alternate);
        assert_eq!(
            alt.phase(Duration::from_millis(5000)),
            Phase::Finished { progress: 0.0 }
        );
        let alt3 = spec(3.0, Direction::Alternate);
        assert_eq!(
            alt3.phase(Duration::from_millis(5000)),
            Phase::Finished { progress: 1.0 }
        );
        // A fractional count stops part way.
        let half = spec(1.5, Direction::Normal);
        assert_eq!(
            half.phase(Duration::from_millis(5000)),
            Phase::Finished { progress: 0.5 }
        );
    }

    #[test]
    fn infinite_never_finishes() {
        let forever = spec(f32::INFINITY, Direction::Normal);
        assert_eq!(forever.total(), None);
        assert!(matches!(
            forever.phase(Duration::from_secs(10_000)),
            Phase::Running { .. }
        ));
    }

    #[test]
    fn the_delay_holds_the_animation_off() {
        let mut s = spec(1.0, Direction::Normal);
        s.delay = Duration::from_millis(500);
        assert_eq!(s.phase(Duration::from_millis(200)), Phase::Delayed);
        assert_eq!(
            s.phase(Duration::from_millis(750)),
            Phase::Running {
                iteration: 0.0,
                progress: 0.25
            }
        );
        assert_eq!(s.total(), Some(Duration::from_millis(1500)));
    }

    /// The per-property timeline: a keyframe that does not declare the property
    /// is NOT a stop for it.
    #[test]
    fn a_property_only_stops_at_the_keyframes_that_declare_it() {
        let mut mid = StyleRefinement::default();
        mid.size.width = Some(gpui::px(10.0).into());
        let mut zero = StyleRefinement::default();
        zero.opacity = Some(0.0);
        let mut one = StyleRefinement::default();
        one.opacity = Some(1.0);

        let frames = Keyframes {
            steps: vec![
                Keyframe {
                    offset: 0.0,
                    easing: None,
                    style: Rc::new(zero),
                    dynamic: None,
                },
                Keyframe {
                    offset: 0.5,
                    easing: None,
                    style: Rc::new(mid),
                    dynamic: None,
                },
                Keyframe {
                    offset: 1.0,
                    easing: None,
                    style: Rc::new(one),
                    dynamic: None,
                },
            ],
            dynamic: false,
        };
        let declares_opacity: &dyn Fn(&StyleRefinement) -> bool = &|r| r.opacity.is_some();
        let seg = frames.segment(declares_opacity, 0.5).unwrap();
        // Straight through the 50% frame: 0 -> 1 across the whole timeline, so
        // halfway is t = 0.5 between the 0% and 100% steps.
        assert_eq!(seg.from.unwrap().offset, 0.0);
        assert_eq!(seg.to.unwrap().offset, 1.0);
        assert_eq!(seg.t, 0.5);

        let declares_width: &dyn Fn(&StyleRefinement) -> bool = &|r| r.size.width.is_some();
        let seg = frames.segment(declares_width, 0.75).unwrap();
        // Only one declaring keyframe, at 50%: it holds from there on.
        assert_eq!(seg.from.unwrap().offset, 0.5);
        assert!(seg.to.is_none());

        let declares_height: &dyn Fn(&StyleRefinement) -> bool = &|r| r.size.height.is_some();
        assert!(frames.segment(declares_height, 0.5).is_none());
    }
}
