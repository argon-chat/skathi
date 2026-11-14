//! `@keyframes` and `animation-*`. Keyframes compile document-wide onto the `Engine`, the
//! `animation-*` declarations fold per element into `AnimationSpec`s, and the animator joins them.
//! The timeline is per property: a keyframe is a stop only for the properties it declares, a
//! missing 0% / 100% takes the element's own value, and `animation-timing-function` governs the
//! segment starting at its keyframe.

use std::rc::Rc;
use std::time::Duration;

use gpui::StyleRefinement;

use crate::transition::Easing;

pub struct Keyframe {
    pub offset: f32,
    pub easing: Option<Easing>,
    pub style: Rc<StyleRefinement>,
    /// `var()`-composed declarations: the block is document-scoped but custom properties are per
    /// element, so they are substituted by `Engine::keyframes_for` when a run starts.
    pub dynamic: Option<Rc<crate::mapper::Decls>>,
    pub custom: Option<Rc<crate::mapper::Decls>>,
}

pub struct Keyframes {
    pub steps: Vec<Keyframe>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationSpec {
    pub name: Rc<str>,
    pub duration: Duration,
    pub delay: Duration,
    pub iterations: f32,
    pub direction: Direction,
    pub fill: FillMode,
    pub easing: Easing,
    pub play_state: PlayState,
}

impl AnimationSpec {
    pub fn is_inert(&self) -> bool {
        self.name.is_empty() || (self.duration.is_zero() && self.delay.is_zero())
    }

    pub fn total(&self) -> Option<Duration> {
        if !self.iterations.is_finite() {
            return None;
        }
        // A count that carries the end past what a `Duration` holds never ends as far as anyone
        // watching can tell; `mul_f32` would panic.
        let run = self.duration.as_secs_f64() * self.iterations.max(0.0) as f64;
        let run = Duration::try_from_secs_f64(run).ok()?;
        self.delay.checked_add(run)
    }

    pub fn phase(&self, elapsed: Duration) -> Phase {
        if elapsed < self.delay {
            return Phase::Delayed;
        }
        let running = elapsed - self.delay;
        if self.duration.is_zero() {
            return Phase::Finished {
                progress: self.final_progress(),
            };
        }
        let raw = running.as_secs_f64() / self.duration.as_secs_f64();
        if raw >= self.iterations as f64 {
            return Phase::Finished {
                progress: self.final_progress(),
            };
        }
        let iteration = raw.floor() as f32;
        Phase::Running {
            iteration,
            progress: self.direct(iteration, (raw - raw.floor()) as f32),
        }
    }

    fn final_progress(&self) -> f32 {
        let last = if self.iterations.is_finite() {
            self.iterations.max(0.0)
        } else {
            1.0
        };
        if last == 0.0 {
            return self.direct(0.0, 0.0);
        }
        if last.fract() == 0.0 {
            self.direct(last - 1.0, 1.0)
        } else {
            self.direct(last.floor(), last.fract())
        }
    }

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Delayed,
    Running {
        iteration: f32,
        progress: f32,
    },
    Finished {
        progress: f32,
    },
}

impl Keyframes {
    pub fn segment<'a>(
        &'a self,
        declares: &dyn Fn(&StyleRefinement) -> bool,
        progress: f32,
    ) -> Option<Segment<'a>> {
        self.segment_steps(&|step| declares(&step.style), progress)
    }

    pub(crate) fn segment_steps<'a>(&'a self, declares: &dyn Fn(&Keyframe) -> bool, progress: f32) -> Option<Segment<'a>> {
        let mut before: Option<&Keyframe> = None;
        let mut after: Option<&Keyframe> = None;
        for step in self.steps.iter().filter(|s| declares(s)) {
            if step.offset <= progress {
                before = Some(step);
            } else {
                after = Some(step);
                break;
            }
        }
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
            // Nothing after the last declared keyframe: interpolate to itself, so `fill-mode:
            // forwards` holds the end value instead of snapping back to the element's own.
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

/// `animation-*` longhands without a list: a lone `animation-play-state: paused` or
/// `animation-fill-mode: forwards` (reka-ui's `Presence` writes it) must not delete the animation
/// the sheet started, so it applies as a patch over the winning list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationPatch {
    pub duration: Option<Duration>,
    pub delay: Option<Duration>,
    pub easing: Option<Easing>,
    pub iterations: Option<f32>,
    pub direction: Option<Direction>,
    pub fill: Option<FillMode>,
    pub play_state: Option<PlayState>,
    // A later animation-name must not reset an earlier duration, and duration lists must keep
    // cycling even when a later rule changes names.
    #[doc(hidden)]
    pub declarations: Option<Rc<AnimationDeclarations>>,
}

impl AnimationPatch {
    pub fn is_empty(&self) -> bool {
        *self == AnimationPatch::default()
    }

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

pub struct Segment<'a> {
    pub from: Option<&'a Keyframe>,
    pub to: Option<&'a Keyframe>,
    pub t: f32,
    pub easing: Option<Easing>,
}

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
    named: bool,
}

pub(crate) type AnimationCollector = AnimationDeclarations;

impl AnimationDeclarations {
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
        let half = spec(1.5, Direction::Normal);
        assert_eq!(
            half.phase(Duration::from_millis(5000)),
            Phase::Finished { progress: 0.5 }
        );
    }

    #[test]
    fn an_instant_animation_rests_where_a_timed_one_does() {
        let mut one = spec(1.0, Direction::Alternate);
        one.duration = Duration::ZERO;
        assert_eq!(one.phase(Duration::ZERO), Phase::Finished { progress: 1.0 });
        let mut two = spec(2.0, Direction::Alternate);
        two.duration = Duration::ZERO;
        assert_eq!(two.phase(Duration::ZERO), Phase::Finished { progress: 0.0 });
    }

    #[test]
    fn a_count_of_zero_rests_at_the_start() {
        let none = spec(0.0, Direction::Normal);
        assert_eq!(
            none.phase(Duration::from_millis(10)),
            Phase::Finished { progress: 0.0 }
        );
        let reversed = spec(0.0, Direction::Reverse);
        assert_eq!(
            reversed.phase(Duration::from_millis(10)),
            Phase::Finished { progress: 1.0 }
        );
    }

    #[test]
    fn a_total_past_what_a_duration_holds_is_endless() {
        let mut long = spec(1.0e30, Direction::Normal);
        long.duration = Duration::from_secs(1_000_000_000);
        assert_eq!(long.total(), None);
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
                    custom: None,
                },
                Keyframe {
                    offset: 0.5,
                    easing: None,
                    style: Rc::new(mid),
                    dynamic: None,
                    custom: None,
                },
                Keyframe {
                    offset: 1.0,
                    easing: None,
                    style: Rc::new(one),
                    dynamic: None,
                    custom: None,
                },
            ],
            dynamic: false,
        };
        let declares_opacity: &dyn Fn(&StyleRefinement) -> bool = &|r| r.opacity.is_some();
        let seg = frames.segment(declares_opacity, 0.5).unwrap();
        assert_eq!(seg.from.unwrap().offset, 0.0);
        assert_eq!(seg.to.unwrap().offset, 1.0);
        assert_eq!(seg.t, 0.5);

        let declares_width: &dyn Fn(&StyleRefinement) -> bool = &|r| r.size.width.is_some();
        let seg = frames.segment(declares_width, 0.75).unwrap();
        assert_eq!(seg.from.unwrap().offset, 0.5);
        assert!(seg.to.is_none());

        let declares_height: &dyn Fn(&StyleRefinement) -> bool = &|r| r.size.height.is_some();
        assert!(frames.segment(declares_height, 0.5).is_none());
    }
}
