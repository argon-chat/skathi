use crate::{pointer, render, tree, windows, EventPayload};
use gpui::{App, PinchEvent, TouchPhase, Window};

#[derive(Clone, Copy)]
pub(crate) struct Gesture {
    target: u32,
    scale: f64,
}

fn scale_step(delta: f32) -> Option<(f64, f64)> {
    let delta = f64::from(delta);
    if !delta.is_finite() || delta <= -1.0 || delta == 0.0 {
        return None;
    }
    Some((1.0 + delta, -100.0 * delta.ln_1p()))
}

pub(crate) fn handle(event: &PinchEvent, window: &mut Window, cx: &mut App) {
    let hit = pointer::target_at(window, event.position);
    let ending = matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled);
    let Some((gesture, started, wheel)) = windows::here(window, |w| {
        if ending && w.pinch.is_none() {
            return None;
        }
        let started = w.pinch.is_none();
        let active = w.pinch.get_or_insert(Gesture {
            target: hit,
            scale: 1.0,
        });
        let wheel = scale_step(event.delta)
            .filter(|_| event.phase != TouchPhase::Cancelled)
            .and_then(|(factor, dy)| {
                let next = active.scale * factor;
                if !next.is_finite() || next <= 0.0 {
                    return None;
                }
                active.scale = next;
                Some(dy)
            });
        let gesture = *active;
        if ending {
            w.pinch = None;
        }
        Some((gesture, started, wheel))
    }) else {
        return;
    };
    let mods = render::mods_bits(&event.modifiers);
    let point = (f64::from(event.position.x), f64::from(event.position.y));
    let proceed = render::js_wave_app(window, cx, || {
        let send = |target, kind, a, b, d| {
            crate::pointer_at_next_dispatch(point.0, point.1);
            crate::dispatch_event(
                target,
                kind,
                &EventPayload {
                    a,
                    b,
                    d,
                    ..Default::default()
                },
            )
        };
        let mut proceed = true;
        if started {
            proceed &= send(gesture.target, tree::EV_GESTURESTART, 1.0, 0.0, mods);
        }
        if ending || wheel.is_some() {
            let kind = if ending {
                tree::EV_GESTUREEND
            } else {
                tree::EV_GESTURECHANGE
            };
            proceed &= send(gesture.target, kind, gesture.scale, 0.0, mods);
        }
        if let Some(dy) = wheel.filter(|_| proceed) {
            let wheel_mods = gpui::Modifiers {
                control: true,
                ..event.modifiers
            };
            proceed = send(hit, tree::EV_WHEEL, 0.0, dy, render::mods_bits(&wheel_mods));
        }
        proceed
    });
    if !proceed {
        window.prevent_default();
    }
    cx.stop_propagation();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reciprocal_scales_produce_opposite_deltas() {
        let (_, zoom_in) = scale_step(1.0).unwrap();
        let (_, zoom_out) = scale_step(-0.5).unwrap();
        assert!(zoom_in < 0.0);
        assert!((zoom_in + zoom_out).abs() < 1e-10);
        assert!(((-zoom_in / 100.0).exp() - 2.0).abs() < 1e-10);
        for invalid in [0.0, -1.0, -1.2, f32::NAN, f32::INFINITY] {
            assert!(scale_step(invalid).is_none());
        }
    }
}
