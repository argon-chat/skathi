//! Script animations: what `Element.animate()` runs (Web Animations 1).
//!
//! # The split
//!
//! The JS half (packages/vue-native/src/waapi.ts) owns the `Animation` object
//! and its whole state machine: play, pause, seek, reverse, finish, the
//! playback rate, the `ready` / `finished` promises and the events. What it
//! tells this side is only what PAINTING a frame needs:
//!
//! * the effect — the keyframes and the timing (delay, duration, iterations,
//!   direction, fill, easing), via [`Animator::script_set`];
//! * the clock, exactly as Web Animations defines it — a start time on the
//!   document timeline, or a held current time, and a playback rate — via
//!   [`Animator::script_clock`], every time the state machine moves it.
//!
//! Each frame the animator turns that clock into the animation's current time
//! and samples the effect at it, the way it samples a `@keyframes` run. So a
//! running script animation costs JS nothing per frame, and the two halves
//! cannot drift: they read the same clock (`performance.now()`, whose origin
//! the host hands over as [`Animator::set_timeline_origin`]) through the same
//! timing model, written twice and tested on both sides.
//!
//! # Where this differs from a `@keyframes` run
//!
//! * The effect's `easing` applies to the whole iteration and a keyframe's own
//!   easing (default `linear`) to its interval — the reverse of CSS, where
//!   `animation-timing-function` is the per-keyframe default.
//! * An eased progress outside 0..1 (a back-out curve's overshoot)
//!   EXTRAPOLATES the first or last interval, as the spec says; the CSS path
//!   clamps. `Element.animate()` users rely on that overshoot.
//! * Nothing here cancels on `display: none` or on `prefers-reduced-motion`,
//!   and a stylesheet swap keeps it: none of those touch a script animation in
//!   a browser either.

use std::rc::Rc;
use std::time::Instant;

use gpui::{Hsla, StyleRefinement};

use super::{initial, install, lerp, slot_of, unify, Animatable, Animator, Slot};
use crate::keyframes::{Direction, FillMode, Keyframe, Keyframes};
use crate::transition::{Easing, TransitionProperty};

/// An `AnimationEffect`'s timing, resolved (`duration: "auto"` is 0, `fill:
/// "auto"` is `none`). Times in milliseconds, as the DOM has them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptTiming {
    pub delay: f64,
    pub end_delay: f64,
    pub duration: f64,
    /// `f64::INFINITY` for `Infinity`.
    pub iterations: f64,
    pub iteration_start: f64,
    pub direction: Direction,
    pub fill: FillMode,
    pub easing: Easing,
}

impl Default for ScriptTiming {
    fn default() -> Self {
        ScriptTiming {
            delay: 0.0,
            end_delay: 0.0,
            duration: 0.0,
            iterations: 1.0,
            iteration_start: 0.0,
            direction: Direction::Normal,
            fill: FillMode::None,
            easing: Easing::Linear,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Before,
    Active,
    After,
}

impl ScriptTiming {
    /// Web Animations 1 §4.6.3, "the active duration".
    pub fn active_duration(&self) -> f64 {
        if self.duration == 0.0 || self.iterations == 0.0 {
            0.0
        } else {
            self.duration * self.iterations
        }
    }

    /// §4.6.4, "the end time".
    pub fn end_time(&self) -> f64 {
        (self.delay + self.active_duration() + self.end_delay).max(0.0)
    }

    /// The eased iteration progress at `local` ms of the animation's current
    /// time, or `None` when the effect is not in effect there (outside its
    /// active interval and not filling). `backwards` is the animation
    /// direction (a negative playback rate), which decides which phase owns a
    /// local time sitting exactly on a boundary. §4.8–§4.10.
    pub fn progress(&self, local: f64, backwards: bool) -> Option<f64> {
        let active = self.active_duration();
        let end = self.end_time();
        let before_active = self.delay.min(end).max(0.0);
        let active_after = (self.delay + active).min(end).max(0.0);
        let phase = if local < before_active || (backwards && local == before_active) {
            Phase::Before
        } else if local > active_after || (!backwards && local == active_after) {
            Phase::After
        } else {
            Phase::Active
        };
        let active_time = match phase {
            Phase::Before => matches!(self.fill, FillMode::Backwards | FillMode::Both)
                .then(|| (local - self.delay).max(0.0)),
            Phase::Active => Some(local - self.delay),
            Phase::After => matches!(self.fill, FillMode::Forwards | FillMode::Both)
                .then(|| (local - self.delay).min(active).max(0.0)),
        }?;
        let overall = if self.duration == 0.0 {
            if phase == Phase::Before {
                self.iteration_start
            } else {
                self.iteration_start + self.iterations
            }
        } else {
            active_time / self.duration + self.iteration_start
        };
        let mut simple = if overall.is_infinite() {
            self.iteration_start % 1.0
        } else {
            overall % 1.0
        };
        // The end of an iteration is 1, not the 0 of the next one — but only
        // where the active interval actually ends.
        if simple == 0.0 && phase != Phase::Before && active_time == active && self.iterations != 0.0 {
            simple = 1.0;
        }
        let iteration = if phase == Phase::After && self.iterations.is_infinite() {
            f64::INFINITY
        } else if simple == 1.0 {
            overall.floor() - 1.0
        } else {
            overall.floor()
        };
        let forwards = match self.direction {
            Direction::Normal => true,
            Direction::Reverse => false,
            Direction::Alternate | Direction::AlternateReverse => {
                let d = if self.direction == Direction::AlternateReverse { iteration + 1.0 } else { iteration };
                !d.is_finite() || d % 2.0 == 0.0
            }
        };
        let directed = if forwards { simple } else { 1.0 - simple };
        Some(self.easing.eval(directed as f32) as f64)
    }
}

/// An `Animation`'s clock, in document-timeline milliseconds: its current
/// time is `hold` when there is one, else `(timeline − start) × rate`, else
/// unresolved (an idle animation, which paints nothing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptClock {
    pub start: Option<f64>,
    pub hold: Option<f64>,
    pub rate: f64,
}

impl Default for ScriptClock {
    fn default() -> Self {
        ScriptClock { start: None, hold: None, rate: 1.0 }
    }
}

impl ScriptClock {
    pub fn current(&self, timeline: Option<f64>) -> Option<f64> {
        self.hold
            .or_else(|| Some((timeline? - self.start?) * self.rate))
    }
}

/// One script animation's painting half.
pub(super) struct ScriptAnim {
    pub(super) id: u32,
    pub(super) frames: Rc<Keyframes>,
    pub(super) timing: ScriptTiming,
    pub(super) clock: ScriptClock,
}

impl ScriptAnim {
    /// Will its value change on a later frame? Playing, not held, and not yet
    /// past the end it is heading for. The delay counts: the value is about
    /// to move.
    pub(super) fn running(&self, timeline: Option<f64>) -> bool {
        if self.clock.hold.is_some() || self.clock.rate == 0.0 {
            return false;
        }
        let Some(current) = self.clock.current(timeline) else {
            return false;
        };
        if self.clock.rate > 0.0 {
            current < self.timing.end_time()
        } else {
            current > 0.0
        }
    }

    fn progress(&self, timeline: Option<f64>) -> Option<f64> {
        let current = self.clock.current(timeline)?;
        self.timing.progress(current, self.clock.rate < 0.0)
    }
}

impl Animator {
    /// Where the document timeline's zero is: the instant `performance.now()`
    /// reads 0 at. The host sets it once; until then no script animation is
    /// sampled.
    pub fn set_timeline_origin(&mut self, origin: Instant) {
        self.timeline_origin = Some(origin);
    }

    pub(super) fn timeline_ms(&self, now: Instant) -> Option<f64> {
        self.timeline_origin
            .map(|origin| now.saturating_duration_since(origin).as_secs_f64() * 1000.0)
    }

    /// Creates script animation `id` on `node`, or replaces its effect (new
    /// keyframes, new timing, or a new target) keeping its clock. A new one
    /// starts idle until [`Animator::script_clock`] gives it a time.
    pub fn script_set(&mut self, id: u32, node: u32, frames: Rc<Keyframes>, timing: ScriptTiming) {
        let mut clock = ScriptClock::default();
        if let Some(old) = self.script_nodes.get(&id).copied() {
            if let Some(list) = self.scripts.get_mut(&old) {
                if let Some(a) = list.iter_mut().find(|a| a.id == id) {
                    if old == node {
                        a.frames = frames;
                        a.timing = timing;
                        return;
                    }
                    clock = a.clock;
                }
                list.retain(|a| a.id != id);
                if list.is_empty() {
                    self.scripts.remove(&old);
                }
            }
        }
        self.script_nodes.insert(id, node);
        let list = self.scripts.entry(node).or_default();
        // Composite order is creation order; ids grow with it, so a target
        // change keeps the animation where it belongs in the new stack.
        let at = list.iter().position(|a| a.id > id).unwrap_or(list.len());
        list.insert(at, ScriptAnim { id, frames, timing, clock });
    }

    /// The node animation `id` paints, if it has one here.
    pub fn script_node(&self, id: u32) -> Option<u32> {
        self.script_nodes.get(&id).copied()
    }

    /// Moves animation `id`'s clock. Answers the node it paints, so the host
    /// can repaint it, or `None` for an id it does not know.
    pub fn script_clock(&mut self, id: u32, clock: ScriptClock) -> Option<u32> {
        let node = *self.script_nodes.get(&id)?;
        let a = self.scripts.get_mut(&node)?.iter_mut().find(|a| a.id == id)?;
        a.clock = clock;
        Some(node)
    }

    /// Forgets animation `id` (cancelled, replaced, or garbage). Answers the
    /// node it painted.
    pub fn script_drop(&mut self, id: u32) -> Option<u32> {
        let node = self.script_nodes.remove(&id)?;
        if let Some(list) = self.scripts.get_mut(&node) {
            list.retain(|a| a.id != id);
            if list.is_empty() {
                self.scripts.remove(&node);
            }
        }
        Some(node)
    }

    /// Animation `id`'s own values right now, as CSS `name: value` pairs —
    /// `commitStyles()`. Empty when it is not in effect; `None` for an id it
    /// does not know.
    pub fn script_values(&self, id: u32, base: &StyleRefinement, now: Instant) -> Option<Vec<(String, String)>> {
        let node = *self.script_nodes.get(&id)?;
        let a = self.scripts.get(&node)?.iter().find(|a| a.id == id)?;
        let mut out = Vec::new();
        let Some(progress) = a.progress(self.timeline_ms(now)) else {
            return Some(out);
        };
        let mut r = StyleRefinement::default();
        apply_script_keyframes(&mut r, base, &a.frames, progress);
        for property in TransitionProperty::animatable() {
            if let Slot::Value(v) = slot_of(&r, &property) {
                if let Some(css) = css_value(v) {
                    out.push((property.name().to_string(), css));
                }
            }
        }
        Some(out)
    }

    /// The script layer of [`Animator::overlay`].
    pub(super) fn script_overlay(
        &self,
        node: u32,
        base: &StyleRefinement,
        now: Instant,
        out: &mut Option<StyleRefinement>,
    ) {
        let Some(list) = self.scripts.get(&node) else {
            return;
        };
        let timeline = self.timeline_ms(now);
        for a in list {
            let Some(progress) = a.progress(timeline) else {
                continue;
            };
            let r = out.get_or_insert_with(StyleRefinement::default);
            apply_script_keyframes(r, base, &a.frames, progress);
        }
    }
}

/// Samples a script animation's keyframes at an (eased) iteration progress —
/// Web Animations 1 §5.3.4, "the effect value of a keyframe effect", per
/// animatable property.
fn apply_script_keyframes(out: &mut StyleRefinement, base: &StyleRefinement, frames: &Keyframes, progress: f64) {
    let p = progress as f32;
    for property in TransitionProperty::animatable() {
        let mut list: Vec<(f32, Option<&Keyframe>)> = Vec::new();
        for k in frames.steps.iter().filter(|k| slot_of(&k.style, &property) != Slot::Missing) {
            list.push((k.offset, Some(k)));
        }
        let (Some(&(first, _)), Some(&(last, _))) = (list.first(), list.last()) else {
            continue;
        };
        // The neutral keyframes: a property no keyframe sets at 0 or at 1
        // takes its underlying value there.
        if first != 0.0 {
            list.insert(0, (0.0, None));
        }
        if last != 1.0 {
            list.push((1.0, None));
        }
        let implicit = || match slot_of(base, &property) {
            Slot::Missing => initial(&property).map_or(Slot::Missing, Slot::Value),
            slot => slot,
        };
        let value = |k: Option<&Keyframe>| k.map_or_else(implicit, |k| slot_of(&k.style, &property));
        let zeros = list.iter().filter(|(o, _)| *o == 0.0).count();
        let ones = list.iter().filter(|(o, _)| *o == 1.0).count();
        // One keyframe stands alone where several share the end the progress
        // has run past; otherwise the interval holding it, or the first or
        // last one extrapolated.
        let lone = if p < 0.0 && zeros > 1 {
            Some(list[0].1)
        } else if p >= 1.0 && ones > 1 {
            Some(list[list.len() - 1].1)
        } else {
            None
        };
        if let Some(k) = lone {
            if let Slot::Value(v) = value(k) {
                install(out, base, &property, v);
            }
            continue;
        }
        let start = if p < 0.0 {
            0
        } else if p >= 1.0 {
            list.len() - 2
        } else {
            list.iter().rposition(|(o, _)| *o <= p && *o < 1.0).unwrap_or(0)
        };
        let ((o0, k0), (o1, k1)) = (list[start], list[start + 1]);
        let distance = if o1 > o0 { (p - o0) / (o1 - o0) } else { 0.0 };
        let eased = k0.and_then(|k| k.easing).unwrap_or(Easing::Linear).eval(distance);
        let (Slot::Value(a), Slot::Value(b)) = (value(k0), value(k1)) else {
            // One end is `auto`, a percentage or unset: hold whichever end
            // does have a value, as the `@keyframes` path does.
            if let Slot::Value(v) = value(k0) {
                install(out, base, &property, v);
            } else if let Slot::Value(v) = value(k1) {
                install(out, base, &property, v);
            }
            continue;
        };
        match unify(a, b) {
            Some((a, b)) => install(out, base, &property, lerp(a, b, eased)),
            // Discrete: the start value for the first half, the end after.
            None => install(out, base, &property, if eased < 0.5 { a } else { b }),
        }
    }
}

/// One animated value as CSS text, for `commitStyles()`.
fn css_value(v: Animatable) -> Option<String> {
    Some(match v {
        Animatable::Number(n) => format!("{n}"),
        Animatable::Color(c) => css_color(c),
        Animatable::BorderColors(colors) => colors.map(css_color).join(" "),
        Animatable::Pixels(p) => format!("{}px", p.as_f32()),
        Animatable::Rems(r) => format!("{}rem", r.0),
        Animatable::Transform(t) => {
            if t.is_identity() {
                return Some("none".into());
            }
            let len = |l: gpui::TransformLength| match l {
                gpui::TransformLength::Pixels(p) => format!("{p}px"),
                gpui::TransformLength::Fraction(f) => format!("{}%", f * 100.0),
            };
            let mut parts = Vec::new();
            parts.push(format!("translate({}, {})", len(t.translate.0), len(t.translate.1)));
            if t.rotate != 0.0 {
                parts.push(format!("rotate({}deg)", t.rotate.to_degrees()));
            }
            if t.scale != (1.0, 1.0) {
                parts.push(format!("scale({}, {})", t.scale.0, t.scale.1));
            }
            parts.join(" ")
        }
        Animatable::Effect(_) => return None,
    })
}

fn css_color(c: Hsla) -> String {
    let c = gpui::Rgba::from(c);
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (ch(c.r), ch(c.g), ch(c.b));
    if c.a >= 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        format!("rgba({r}, {g}, {b}, {})", (c.a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0)
    }
}

// ---------------------------------------------------------------------------
// the keyframes
// ---------------------------------------------------------------------------

/// One keyframe as the script wrote it, after the JS half has computed the
/// offsets and turned the IDL names into CSS ones.
pub struct ScriptFrame {
    pub offset: f32,
    /// The easing of the interval that starts here; `None` is `linear`.
    pub easing: Option<Easing>,
    /// `(css-name, value)`, in the order the script gave them.
    pub decls: Vec<(String, String)>,
}

/// What [`script_keyframes`] could not use, for the JS half's advisories.
#[derive(Default, Debug)]
pub struct ScriptKeyframeNotes {
    /// Property names that parse but do not animate here.
    pub ignored: Vec<String>,
    /// The parser's own diagnostics (an invalid value, an unresolved var()).
    pub diagnostics: Vec<String>,
}

/// Compiles a script's keyframes against the TARGET's custom properties — a
/// keyframe value may say `var(--x)`, and the element it animates is the one
/// whose `--x` that is.
pub fn script_keyframes(
    frames: &[ScriptFrame],
    vars: &crate::Vars,
    env: &crate::Env,
) -> (Rc<Keyframes>, ScriptKeyframeNotes) {
    let mut notes = ScriptKeyframeNotes::default();
    let mut steps = Vec::with_capacity(frames.len());
    for frame in frames {
        let mut body = String::new();
        for (name, value) in &frame.decls {
            if !crate::transition::animates(name) {
                if !notes.ignored.contains(name) {
                    notes.ignored.push(name.clone());
                }
                continue;
            }
            body.push_str(name);
            body.push_str(": ");
            body.push_str(value);
            body.push_str("; ");
        }
        let parsed = crate::mapper::parse_inline_style(&body, &mut notes.diagnostics);
        let mut style = parsed.refinement.map(|r| (*r).clone()).unwrap_or_default();
        let mut extras = crate::Extras::default();
        for (name, raw) in &parsed.dynamic {
            crate::mapper::apply_dynamic(name, raw, vars, env, &mut style, &mut extras, &mut notes.diagnostics);
        }
        steps.push(Keyframe {
            offset: frame.offset,
            easing: frame.easing,
            style: Rc::new(style),
            dynamic: None,
            custom: None,
        });
    }
    // The JS half hands them over in order already (it refuses offsets that
    // go backwards, as the DOM does); a stable sort keeps equal offsets in
    // the order given, which is what decides between them.
    steps.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    (Rc::new(Keyframes { steps, dynamic: false }), notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transition::StepPosition;
    use std::time::Duration;

    fn timing(duration: f64) -> ScriptTiming {
        ScriptTiming { duration, ..ScriptTiming::default() }
    }

    fn frames(list: &[(f32, &str)]) -> Rc<Keyframes> {
        let frames: Vec<ScriptFrame> = list
            .iter()
            .map(|(offset, body)| ScriptFrame {
                offset: *offset,
                easing: None,
                decls: body
                    .split(';')
                    .filter_map(|d| d.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect(),
            })
            .collect();
        script_keyframes(&frames, &crate::Vars::default(), &crate::Env::default()).0
    }

    /// An animator whose timeline zero is `t0`, so `at(ms)` is that timeline time.
    fn animator() -> (Animator, Instant) {
        let mut a = Animator::new();
        let t0 = Instant::now();
        a.set_timeline_origin(t0);
        (a, t0)
    }

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn playing(start: f64) -> ScriptClock {
        ScriptClock { start: Some(start), hold: None, rate: 1.0 }
    }

    fn opacity(a: &Animator, node: u32, now: Instant) -> Option<f32> {
        a.overlay(node, &StyleRefinement::default(), now).and_then(|r| r.opacity)
    }

    fn translate_y(a: &Animator, node: u32, now: Instant) -> f32 {
        let r = a.overlay(node, &StyleRefinement::default(), now).expect("in effect");
        match r.vn_transform.expect("a transform").translate.1 {
            gpui::TransformLength::Pixels(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_timing_model_follows_the_spec() {
        let t = timing(1000.0);
        assert_eq!(t.progress(-1.0, false), None, "before, no fill");
        assert_eq!(t.progress(0.0, false), Some(0.0));
        assert_eq!(t.progress(250.0, false), Some(0.25));
        // Exactly at the end, playing forwards: the after phase, no fill.
        assert_eq!(t.progress(1000.0, false), None);
        // …but playing backwards the end belongs to the active phase.
        assert_eq!(t.progress(1000.0, true), Some(1.0));

        let filled = ScriptTiming { fill: FillMode::Both, delay: 100.0, ..t };
        assert_eq!(filled.progress(0.0, false), Some(0.0), "backwards fill holds the start");
        assert_eq!(filled.progress(5000.0, false), Some(1.0), "forwards fill holds the end");

        // Three iterations, alternating: 0 → 1, 1 → 0, 0 → 1, and the end
        // value is the end of the last (forwards) one.
        let alt = ScriptTiming { iterations: 3.0, direction: Direction::Alternate, fill: FillMode::Forwards, ..t };
        assert_eq!(alt.progress(250.0, false), Some(0.25));
        assert_eq!(alt.progress(1250.0, false), Some(0.75));
        assert_eq!(alt.progress(2250.0, false), Some(0.25));
        assert_eq!(alt.progress(9000.0, false), Some(1.0));
        // Two: the last one ran backwards, so it ends where it began.
        let two = ScriptTiming { iterations: 2.0, ..alt };
        assert_eq!(two.progress(9000.0, false), Some(0.0));

        // A fractional iteration start shifts the whole thing.
        let shifted = ScriptTiming { iteration_start: 0.5, ..t };
        assert_eq!(shifted.progress(250.0, false), Some(0.75));

        // A zero duration jumps straight to the end once the delay is over.
        let instant = ScriptTiming { duration: 0.0, fill: FillMode::Forwards, ..t };
        assert_eq!(instant.progress(10.0, false), Some(1.0));

        // The effect's own easing shapes the whole iteration.
        let stepped = ScriptTiming { easing: Easing::Steps(4, StepPosition::JumpEnd), ..t };
        assert_eq!(stepped.progress(600.0, false), Some(0.5));
        assert_eq!(t.end_time(), 1000.0);
        assert_eq!(ScriptTiming { end_delay: 500.0, delay: 200.0, ..t }.end_time(), 1700.0);
        assert_eq!(ScriptTiming { end_delay: -5000.0, ..t }.end_time(), 0.0);
    }

    #[test]
    fn a_playing_animation_paints_from_its_clock_and_stops_at_the_end() {
        let (mut a, t0) = animator();
        a.script_set(1, 7, frames(&[(0.0, "opacity: 0"), (1.0, "opacity: 1")]), timing(1000.0));
        assert_eq!(opacity(&a, 7, at(t0, 100)), None, "idle until it has a clock");
        assert!(!a.any_animation(at(t0, 100)));

        a.script_clock(1, playing(100.0));
        let v = opacity(&a, 7, at(t0, 600)).unwrap();
        assert!((v - 0.5).abs() < 1e-3, "{v}");
        assert!(a.any_animation(at(t0, 600)));
        assert_eq!(a.moving_nodes(at(t0, 600)).collect::<Vec<_>>(), vec![7]);
        // Finished with no fill: nothing painted, nothing moving.
        assert_eq!(opacity(&a, 7, at(t0, 1200)), None);
        assert!(!a.any_animation(at(t0, 1200)));
        assert_eq!(a.moving_nodes(at(t0, 1200)).count(), 0);
    }

    #[test]
    fn a_held_clock_paints_its_time_and_is_not_moving() {
        let (mut a, t0) = animator();
        a.script_set(1, 7, frames(&[(0.0, "opacity: 0"), (1.0, "opacity: 1")]), timing(1000.0));
        a.script_clock(1, ScriptClock { start: None, hold: Some(250.0), rate: 1.0 });
        let v = opacity(&a, 7, at(t0, 5000)).unwrap();
        assert!((v - 0.25).abs() < 1e-3, "{v}");
        assert!(!a.any_animation(at(t0, 5000)), "paused: no reason to repaint");
        // Played backwards from the end.
        a.script_clock(1, ScriptClock { start: Some(2000.0), hold: None, rate: -1.0 });
        let v = opacity(&a, 7, at(t0, 1250)).unwrap();
        assert!((v - 0.75).abs() < 1e-3, "{v}");
        assert!(a.any_animation(at(t0, 1250)));
    }

    #[test]
    fn an_overshooting_easing_extrapolates_the_last_interval() {
        // Digit.vue's roll: translateY from -100px to -120px on a back-out
        // curve, which overshoots past -120 before settling.
        let (mut a, t0) = animator();
        let back_out = Easing::CubicBezier(0.34, 1.56, 0.64, 1.0);
        a.script_set(
            1,
            3,
            frames(&[(0.0, "transform: translateY(-100px)"), (1.0, "transform: translateY(-120px)")]),
            ScriptTiming { easing: back_out, ..timing(900.0) },
        );
        a.script_clock(1, playing(0.0));
        let deepest = (1..90).map(|i| translate_y(&a, 3, at(t0, i * 10))).fold(f32::MAX, f32::min);
        assert!(deepest < -121.0, "no overshoot: {deepest}");
    }

    #[test]
    fn implicit_ends_take_the_underlying_value_and_later_animations_win() {
        let (mut a, t0) = animator();
        let mut base = StyleRefinement::default();
        base.opacity = Some(0.2);
        // `{ opacity: [1] }` — a single value is the 100 % keyframe; 0 % is
        // the element's own opacity.
        a.script_set(1, 9, frames(&[(1.0, "opacity: 1")]), timing(1000.0));
        a.script_clock(1, playing(0.0));
        let v = a.overlay(9, &base, at(t0, 500)).unwrap().opacity.unwrap();
        assert!((v - 0.6).abs() < 1e-3, "{v}");
        // A later animation on the same property replaces it.
        a.script_set(2, 9, frames(&[(0.0, "opacity: 0"), (1.0, "opacity: 0")]), timing(1000.0));
        a.script_clock(2, playing(0.0));
        assert_eq!(a.overlay(9, &base, at(t0, 500)).unwrap().opacity, Some(0.0));
        // Dropping it shows the first again.
        assert_eq!(a.script_drop(2), Some(9));
        let v = a.overlay(9, &base, at(t0, 500)).unwrap().opacity.unwrap();
        assert!((v - 0.6).abs() < 1e-3, "{v}");
    }

    #[test]
    fn script_animations_sit_above_css_ones_and_survive_a_sheet_swap() {
        let (mut a, t0) = animator();
        a.script_set(1, 4, frames(&[(0.0, "opacity: 0.5"), (1.0, "opacity: 0.5")]), timing(1000.0));
        a.script_clock(1, playing(0.0));
        a.clear();
        assert_eq!(opacity(&a, 4, at(t0, 100)), Some(0.5));
        a.gc(&|id| id != 4);
        assert_eq!(opacity(&a, 4, at(t0, 100)), None, "its node is gone");
        assert!(a.is_idle());
    }

    #[test]
    fn a_new_target_keeps_the_clock() {
        let (mut a, t0) = animator();
        let f = frames(&[(0.0, "opacity: 0"), (1.0, "opacity: 1")]);
        a.script_set(1, 4, f.clone(), timing(1000.0));
        a.script_clock(1, playing(0.0));
        a.script_set(1, 5, f, timing(1000.0));
        assert_eq!(opacity(&a, 4, at(t0, 500)), None);
        let v = opacity(&a, 5, at(t0, 500)).unwrap();
        assert!((v - 0.5).abs() < 1e-3, "{v}");
    }

    #[test]
    fn keyframe_easing_shapes_its_own_interval() {
        let (mut a, t0) = animator();
        let f = vec![
            ScriptFrame { offset: 0.0, easing: Some(Easing::Steps(2, StepPosition::JumpEnd)), decls: vec![("opacity".into(), "0".into())] },
            ScriptFrame { offset: 0.5, easing: None, decls: vec![("opacity".into(), "1".into())] },
            ScriptFrame { offset: 1.0, easing: None, decls: vec![("opacity".into(), "0".into())] },
        ];
        let (k, _) = script_keyframes(&f, &crate::Vars::default(), &crate::Env::default());
        a.script_set(1, 2, k, timing(1000.0));
        a.script_clock(1, playing(0.0));
        // 0..0.5 stepped in two: 0 until a quarter, 0.5 after.
        assert_eq!(opacity(&a, 2, at(t0, 100)), Some(0.0));
        assert_eq!(opacity(&a, 2, at(t0, 300)), Some(0.5));
        // 0.5..1 is linear back down.
        let v = opacity(&a, 2, at(t0, 750)).unwrap();
        assert!((v - 0.5).abs() < 1e-2, "{v}");
    }

    #[test]
    fn unanimatable_properties_are_reported_not_painted() {
        let f = vec![ScriptFrame {
            offset: 1.0,
            easing: None,
            decls: vec![
                ("filter".into(), "blur(4px)".into()),
                ("opacity".into(), "0.5".into()),
                ("display".into(), "none".into()),
            ],
        }];
        let (k, notes) = script_keyframes(&f, &crate::Vars::default(), &crate::Env::default());
        assert_eq!(notes.ignored, vec!["filter".to_string(), "display".to_string()]);
        assert_eq!(k.steps[0].style.opacity, Some(0.5));
        assert_eq!(k.steps[0].style.display, None);
    }

    #[test]
    fn a_keyframe_value_resolves_against_the_target_s_custom_properties() {
        let env = crate::Env::default();
        let own = ("--to".to_string(), "0.25".to_string());
        let vars = crate::vars::resolve(&Rc::new(crate::Vars::default()), &[&own], &env, &mut Vec::new());
        let f = vec![ScriptFrame { offset: 1.0, easing: None, decls: vec![("opacity".into(), "var(--to)".into())] }];
        let (k, _) = script_keyframes(&f, &vars, &env);
        assert_eq!(k.steps[0].style.opacity, Some(0.25));
    }

    #[test]
    fn commit_styles_serialises_the_current_values() {
        let (mut a, t0) = animator();
        a.script_set(
            1,
            2,
            frames(&[(0.0, "opacity: 0; transform: translateY(0px)"), (1.0, "opacity: 1; transform: translateY(-20px)")]),
            ScriptTiming { fill: FillMode::Forwards, ..timing(1000.0) },
        );
        a.script_clock(1, playing(0.0));
        let values = a.script_values(1, &StyleRefinement::default(), at(t0, 2000)).unwrap();
        assert!(values.contains(&("opacity".to_string(), "1".to_string())), "{values:?}");
        assert!(values.contains(&("transform".to_string(), "translate(0px, -20px)".to_string())), "{values:?}");
        assert_eq!(a.script_values(9, &StyleRefinement::default(), t0), None);
    }

    #[test]
    fn easing_text_parses_like_the_cascade_and_refuses_linear_functions() {
        use crate::transition::parse_easing;
        assert_eq!(parse_easing("ease-out"), Some(Easing::EaseOut));
        assert_eq!(parse_easing("cubic-bezier(0.34, 1.56, 0.64, 1)"), Some(Easing::CubicBezier(0.34, 1.56, 0.64, 1.0)));
        assert_eq!(parse_easing("steps(4, end)"), Some(Easing::Steps(4, StepPosition::JumpEnd)));
        assert_eq!(parse_easing("linear(0, 1)"), None);
        assert_eq!(parse_easing("bogus"), None);
        assert!(crate::transition::animates("transform"));
        assert!(crate::transition::animates("padding"));
        assert!(!crate::transition::animates("filter"));
        assert!(!crate::transition::animates("--shading"));
    }
}

#[cfg(test)]
mod border_css_regression {
    use super::*;
    #[test]
    fn committed_border_animation_keeps_four_side_colours() {
        let colors=[gpui::red(), gpui::green(), gpui::blue(), Hsla::transparent_black()];
        assert_eq!(css_value(Animatable::BorderColors(colors)), Some(colors.map(css_color).join(" ")));
    }
}
