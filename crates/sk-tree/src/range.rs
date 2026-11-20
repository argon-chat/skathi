//! Native HTML range controls.
use std::cell::RefCell;
use std::collections::HashMap;

use crate::{
    tree::{self, Tree},
    windows, EventPayload,
};
use gpui::prelude::*;
use gpui::{
    canvas, div, px, relative, AnyElement, App, Bounds, KeyDownEvent, Length, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, StyleRefinement, Window,
};

pub(crate) fn is_range(t: &Tree, id: u32) -> bool {
    t.node(id).is_some_and(|n| t.name(n.tag) == "input")
        && t.prop(id, "type")
            .is_some_and(|s| s.eq_ignore_ascii_case("range"))
}

fn number(s: Option<&str>) -> Option<f64> {
    let s = s?;
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let mut parts = unsigned.split(['e', 'E']);
    let mantissa = parts.next()?;
    if let Some(exponent) = parts.next() {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || parts.next().is_some()
        {
            return None;
        }
    }
    let valid = if let Some((whole, fraction)) = mantissa.split_once('.') {
        whole.bytes().all(|b| b.is_ascii_digit())
            && !fraction.is_empty()
            && fraction.bytes().all(|b| b.is_ascii_digit())
    } else {
        !mantissa.is_empty() && mantissa.bytes().all(|b| b.is_ascii_digit())
    };
    if !valid {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[derive(Clone, Copy, Debug)]
struct Spec {
    min: f64,
    max: f64,
    step: Option<f64>,
    base: f64,
}
impl Spec {
    fn read(t: &Tree, id: u32) -> Self {
        let min = number(t.prop(id, "min"));
        Self {
            min: min.unwrap_or(0.),
            max: number(t.prop(id, "max")).unwrap_or(100.),
            step: if t
                .prop(id, "step")
                .is_some_and(|s| s.eq_ignore_ascii_case("any"))
            {
                None
            } else {
                Some(number(t.prop(id, "step")).filter(|v| *v > 0.).unwrap_or(1.))
            },
            base: min
                .or_else(|| number(t.prop(id, ":range-base")))
                .unwrap_or(0.),
        }
    }
    fn sanitize(self, value: Option<f64>) -> f64 {
        if self.max < self.min {
            return self.min;
        }
        let value = value
            .unwrap_or(self.min / 2. + self.max / 2.)
            .clamp(self.min, self.max);
        let Some(step) = self.step else { return value };
        let lo = ((self.min - self.base) / step - 1e-10).ceil();
        let hi = ((self.max - self.base) / step + 1e-10).floor();
        if lo > hi {
            return value;
        }
        let index = (((value - self.base) / step + 0.5 + 1e-10).floor()).clamp(lo, hi);
        let value = (self.base + index * step).clamp(self.min, self.max);
        // Strip arithmetic noise without discarding small or exponential values.
        format!("{value:.14e}").parse().unwrap_or(value)
    }
    fn value(self, t: &Tree, id: u32) -> f64 {
        self.sanitize(number(t.prop(id, "value")))
    }
    fn fraction(self, value: f64) -> f32 {
        if self.max <= self.min {
            0.
        } else {
            ((value - self.min) / (self.max - self.min)) as f32
        }
    }
    fn key(self, current: f64, key: &str) -> Option<f64> {
        let step = self
            .step
            .unwrap_or((self.max - self.min).abs() / 100.)
            .max(f64::EPSILON);
        let page = ((self.max - self.min).abs() / 10.).max(step);
        Some(self.sanitize(Some(match key {
            "left" | "down" => current - step,
            "right" | "up" => current + step,
            "pageup" => current + page,
            "pagedown" => current - page,
            "home" => self.min,
            "end" => self.max,
            _ => return None,
        })))
    }
}

fn usable(t: &Tree, id: u32) -> bool {
    is_range(t, id) && !crate::render::actually_disabled(t, id)
}
pub(crate) fn value(t: &Tree, id: u32) -> String {
    Spec::read(t, id).value(t, id).to_string()
}

#[derive(Clone, Copy)]
struct Drag {
    id: u32,
    initial: f64,
    grab: f32,
    bounds: Bounds<Pixels>,
    thumb: f32,
}
thread_local! {
    static PRESSED: RefCell<Option<(windows::WinKey,u32,Point<Pixels>)>> = const { RefCell::new(None) };
    static DRAGS: RefCell<HashMap<windows::WinKey,Drag>> = RefCell::new(HashMap::new());
}

pub(crate) fn bind_press<E: InteractiveElement>(e: E, id: u32) -> E {
    e.on_mouse_down(MouseButton::Left, move |ev, window, _| {
        PRESSED.with(|p| *p.borrow_mut() = Some((windows::key(window), id, ev.position)));
    })
}

pub(crate) fn press_settled(cancelled: bool, window: &mut Window, cx: &mut App) {
    let Some((key, id, point)) = PRESSED.with(|p| p.borrow_mut().take()) else {
        return;
    };
    if cancelled || key != windows::key(window) {
        return;
    }
    let Some((spec, current, bounds, thumb)) = tree::read_tree(|t| {
        if !usable(t, id) {
            return None;
        }
        let (x, y, w, h) = crate::render::painted_bounds(id)?;
        let spec = Spec::read(t, id);
        let thumb = thumb_size(t, id, window, w).0;
        Some((
            spec,
            spec.value(t, id),
            Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(w), px(h))),
            thumb,
        ))
    }) else {
        return;
    };
    let x = f32::from(point.x - bounds.origin.x);
    let travel = (f32::from(bounds.size.width) - thumb).max(0.);
    let center = thumb / 2. + spec.fraction(current) * travel;
    let grab = if (x - center).abs() <= thumb / 2. {
        x - center
    } else {
        0.
    };
    let drag = Drag {
        id,
        initial: current,
        grab,
        bounds,
        thumb,
    };
    DRAGS.with(|d| {
        d.borrow_mut().insert(key, drag);
    });
    move_to(drag, point, window, cx);
    window.refresh();
}

fn move_to(drag: Drag, point: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(next) = tree::read_tree(|t| {
        usable(t, drag.id).then(|| {
            let spec = Spec::read(t, drag.id);
            let travel = (f32::from(drag.bounds.size.width) - drag.thumb).max(0.);
            let along = f32::from(point.x - drag.bounds.origin.x) - drag.thumb / 2. - drag.grab;
            let fraction = if travel > 0. {
                (along / travel).clamp(0., 1.)
            } else {
                0.
            };
            spec.sanitize(Some(spec.min + (spec.max - spec.min) * fraction as f64))
        })
    }) else {
        DRAGS.with(|d| {
            d.borrow_mut().remove(&windows::key(window));
        });
        return;
    };
    update(drag.id, next, false, window, cx);
}

fn update(id: u32, next: f64, committed: bool, window: &mut Window, cx: &mut App) -> bool {
    let changed = tree::with_tree(|t| {
        if !usable(t, id) {
            return false;
        }
        let changed = Spec::read(t, id).value(t, id) != next;
        if changed {
            let key = t.intern("value");
            t.set_prop(id, key, Some(next.to_string()));
        }
        changed
    });
    if changed || committed {
        use sk_css::haptics::Trigger;
        let source = crate::haptics::input_target(window);
        if changed {
            let spec = tree::read_tree(|t| Spec::read(t, id));
            let edge =
                next == spec.sanitize(Some(spec.min)) || next == spec.sanitize(Some(spec.max));
            if edge {
                crate::haptics::trigger(id, Trigger::Boundary, source, window, cx);
            }
            if spec.step.is_some() && (!edge || !crate::haptics::has_boundary(id, window)) {
                crate::haptics::trigger(id, Trigger::Step, source, window, cx);
            }
        }
        if committed {
            crate::haptics::trigger(id, Trigger::Change, source, window, cx);
        }
        let text = next.to_string();
        crate::render::js_dispatch_app(
            id,
            tree::EV_INPUT,
            &EventPayload {
                key: Some(&text),
                b: if committed { 1. } else { 0. },
                ..Default::default()
            },
            window,
            cx,
        );
        window.refresh();
    }
    changed
}

pub(crate) fn bind_root<E: InteractiveElement>(e: E) -> E {
    e.on_key_down(|ev: &KeyDownEvent, window, cx| {
        if windows::here(window, |w| w.key_prevented) {
            return;
        }
        let modifiers = &ev.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let id = windows::focused_node(windows::key(window));
        let next = tree::read_tree(|t| {
            if !usable(t, id) {
                return None;
            }
            let spec = Spec::read(t, id);
            spec.key(spec.value(t, id), ev.keystroke.key.as_str())
        });
        if let Some(next) = next {
            window.prevent_default();
            if update(id, next, false, window, cx) {
                let current =
                    tree::read_tree(|t| usable(t, id).then(|| Spec::read(t, id).value(t, id)));
                if let Some(value) = current {
                    update(id, value, true, window, cx);
                }
            }
        }
    })
}

fn part_style(t: &Tree, id: u32, thumb: bool, window: &Window) -> Option<StyleRefinement> {
    let cs = t.node(id)?.computed.as_ref()?;
    let part = if thumb {
        cs.range_thumb.as_ref()?
    } else {
        cs.range_track.as_ref()?
    };
    let (active, focus) = windows::here(window, |w| (w.active == Some(id), w.focused_node == id));
    let (style, extras) = part.resolve(
        active,
        tree::is_hovered(id),
        focus && window.last_input_was_keyboard(),
        focus,
        focus,
    );
    let mut style = style.clone();
    if extras.background_images.is_some() {
        style.background_layers = Some(extras.resolve_background_layers());
    }
    Some(style)
}
fn pixels(length: Option<Length>, fallback: f32, base: f32, window: &Window) -> f32 {
    match length {
        Some(Length::Definite(v)) => f32::from(v.to_pixels(px(base).into(), window.rem_size())),
        _ => fallback,
    }
    .max(0.)
}
fn thumb_size(t: &Tree, id: u32, window: &Window, width: f32) -> (f32, f32) {
    let style = part_style(t, id, true, window).unwrap_or_default();
    (
        pixels(style.size.width, 16., width, window),
        pixels(style.size.height, 16., 16., window),
    )
}

pub(crate) fn children(t: &Tree, id: u32, window: &Window) -> AnyElement {
    let spec = Spec::read(t, id);
    let fraction = spec.fraction(spec.value(t, id));
    let width = crate::render::painted_bounds(id).map_or(129., |r| r.2);
    let (thumb_width, thumb_height) = thumb_size(t, id, window, width);
    let dark = crate::scheme::declared(t, id, window) == Some(true);
    let accent = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.extras.accent_color.flatten())
        .unwrap_or_else(|| gpui::rgb(if dark { 0x99c8ff } else { 0x0075ff }).into());
    let skin = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .is_none_or(|cs| cs.extras.appearance_none != Some(true));
    let mut track = div()
        .absolute()
        .left(px(0.))
        .right(px(0.))
        .h(px(4.))
        .rounded(px(2.));
    if skin {
        track = track.bg(gpui::rgb(if dark { 0x545454 } else { 0x777777 }));
    }
    if let Some(style) = part_style(t, id, false, window) {
        track.style().refine(&style);
    }
    let track_height = pixels(track.style().size.height, 4., 14., window);
    track = track.top(relative(0.5)).mt(px(-track_height / 2.));
    let styled_track = t
        .node(id)
        .and_then(|n| n.computed.as_ref())
        .is_some_and(|cs| cs.range_track.is_some());
    if !styled_track && skin {
        track = track.child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .bottom(px(0.))
                .w(relative(fraction))
                .rounded(px(2.))
                .bg(accent),
        );
    }
    let mut thumb = div()
        .absolute()
        .w(px(thumb_width))
        .h(px(thumb_height))
        .rounded(px(999.))
        .bg(accent);
    if let Some(style) = part_style(t, id, true, window) {
        thumb.style().refine(&style);
    }
    thumb = thumb
        .left(relative(fraction))
        .ml(px(-fraction * thumb_width))
        .top(relative(0.5))
        .mt(px(-thumb_height / 2.));
    let listener = canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() {
                    return;
                }
                let drag = DRAGS.with(|d| {
                    let mut map = d.borrow_mut();
                    let drag = map.get_mut(&windows::key(window))?;
                    if drag.id != id {
                        return None;
                    }
                    drag.bounds = bounds;
                    drag.thumb = thumb_width;
                    Some(*drag)
                });
                if let Some(drag) = drag {
                    move_to(drag, ev.position, window, cx);
                }
            });
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                if !phase.bubble() || ev.button != MouseButton::Left {
                    return;
                }
                let drag = DRAGS.with(|d| {
                    let mut d = d.borrow_mut();
                    if d.get(&windows::key(window)).is_none_or(|d| d.id != id) {
                        return None;
                    }
                    d.remove(&windows::key(window))
                });
                if let Some(drag) = drag {
                    move_to(drag, ev.position, window, cx);
                    let current =
                        tree::read_tree(|t| usable(t, id).then(|| Spec::read(t, id).value(t, id)));
                    if let Some(value) = current.filter(|v| *v != drag.initial) {
                        update(id, value, true, window, cx);
                    }
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .size_full();
    div()
        .relative()
        .size_full()
        .child(track)
        .child(thumb)
        .child(listener)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(min: f64, max: f64, step: Option<f64>) -> Spec {
        Spec {
            min,
            max,
            step,
            base: min,
        }
    }
    #[test]
    fn defaults_bounds_and_positive_ties() {
        let s = spec(0., 100., Some(1.));
        assert_eq!(s.sanitize(None), 50.);
        assert_eq!(s.sanitize(Some(0.5)), 1.);
        assert_eq!(s.sanitize(Some(-10.)), 0.);
        assert_eq!(s.sanitize(Some(110.)), 100.);
        assert_eq!(spec(10., 5., Some(1.)).sanitize(None), 10.);
        assert_eq!(spec(-10., 50., Some(1.)).sanitize(Some(-0.5)), 0.);
    }
    #[test]
    fn application_gauges_fractional_steps_and_keyboard() {
        let weight = spec(100., 900., Some(100.));
        assert_eq!(weight.sanitize(Some(849.)), 800.);
        assert_eq!(weight.key(900., "left"), Some(800.));
        assert_eq!(weight.key(100., "home"), Some(100.));
        assert_eq!(weight.key(100., "end"), Some(900.));
        let spacing = spec(-10., 50., Some(1.));
        assert_eq!(spacing.key(26., "right"), Some(27.));
        assert_eq!(spacing.key(26., "pageup"), Some(32.));
        assert_eq!(spec(0., 1., Some(0.1)).sanitize(Some(0.3)), 0.3);
        assert_eq!(spec(0., 1., Some(0.3)).sanitize(Some(1.)), 0.9);
        assert_eq!(spec(0., 1., None).sanitize(Some(0.333)), 0.333);
        assert_eq!(number(Some("NaN")), None);
        assert_eq!(number(Some(" 2")), None);
    }
}
