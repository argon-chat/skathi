//! What a pen adds to the mouse events gpui dispatches for it (`Window::vn_pen`): the runtime
//! builds `pointerType: "pen"` pointer events from it. A move carries every reading (for a mouse,
//! every position) since the previous delivered move, each with its time: its
//! `getCoalescedEvents()`; and where the pointer is predicted next: its `getPredictedEvents()`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use gpui::{PenSample, Window};

use crate::predict::ink::KalmanPredictor;
use crate::predict::Predictor;
use crate::tree;
use crate::windows;

/// Numbers per reading: x, y, pressure, tangentialPressure, tiltX, tiltY, twist, width, height,
/// time.
pub const SAMPLE_LEN: usize = 10;
/// Numbers per predicted reading: x, y, time.
pub const PREDICTED_LEN: usize = 3;
/// Flags leading the packet.
pub const ERASER: f64 = 1.0;
pub const BARREL: f64 = 2.0;
pub const CONTACT: f64 = 4.0;
/// The readings are a mouse's positions.
pub const MOUSE: f64 = 8.0;
const MOUSE_PRESSURE: f64 = 16.0;

/// A pen that never gets a frame (a hidden window) must not grow without bound.
const TRAIL_CAP: usize = 1024;

/// A pointer's predictor key: `None` for the mouse, else the pen's platform id.
type Key = Option<u32>;

/// One window's readings not yet delivered, and its pointers' predictors.
#[derive(Default)]
pub struct PenState {
    primary_down: bool,
    mouse_pressure: Option<f32>,
    trail: Vec<PenSample>,
    /// Strokes that ended (a press, release or exit) after the first `usize` readings of `trail`.
    ends: Vec<(usize, Key)>,
    predictors: HashMap<Key, KalmanPredictor>,
}

/// A touch contact's packet: [`TOUCH`], [`PRIMARY`] for the primary contact, its `pointerId`
/// times [`ID_SHIFT`].
pub const TOUCH: f64 = 32.0;
pub const PRIMARY: f64 = 64.0;
pub const ID_SHIFT: f64 = 256.0;

thread_local! {
    static NEXT: RefCell<Option<Vec<f64>>> = const { RefCell::new(None) };
    /// The packet every pointer-family dispatch carries while it is set ([`with_packet`]).
    static FORCED: RefCell<Option<Vec<f64>>> = const { RefCell::new(None) };
}

/// Runs `f` with `packet` armed for every pointer-family dispatch in it, in place of the
/// mouse's or pen's.
pub(crate) fn with_packet<R>(packet: Vec<f64>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Vec<f64>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FORCED.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(FORCED.with(|c| c.borrow_mut().replace(packet)));
    f()
}

/// One touch reading as a packet: the contact's `pointerId`, whether it is primary, and its
/// position, pressure and size.
pub(crate) fn touch_packet(pointer_id: u32, primary: bool, sample: &PenSample) -> Vec<f64> {
    let mut flags = TOUCH + f64::from(pointer_id) * ID_SHIFT;
    if primary {
        flags += PRIMARY;
    }
    packet(flags, &[(*sample, Clock::now().ms(sample.time))], &[])
}

fn key(window: &Window) -> Key {
    window.vn_pen().map(|p| p.pointer_id)
}

pub(crate) fn mouse_button(window: &Window, down: bool) {
    windows::here(window, |w| {
        w.pen.primary_down = down;
        w.pen.mouse_pressure = None;
    });
}

fn normalized_pressure(stage: gpui::PressureStage, value: f32) -> Option<f32> {
    if !value.is_finite() {
        return None;
    }
    let half = value.clamp(0.0, 1.0) * 0.5;
    Some(match stage {
        gpui::PressureStage::Zero => 0.0,
        gpui::PressureStage::Normal => half,
        gpui::PressureStage::Force => 0.5 + half,
    })
}

pub(crate) fn note_pressure(window: &Window, event: &gpui::MousePressureEvent) -> bool {
    let Some(value) = normalized_pressure(event.stage, event.pressure) else {
        return false;
    };
    windows::here(window, |w| {
        let pen = &mut w.pen;
        if !pen.primary_down || pen.mouse_pressure == Some(value) {
            return false;
        }
        pen.mouse_pressure = Some(value);
        pen.append([PenSample {
            position: event.position,
            pressure: value,
            time: Some(Instant::now()),
            ..Default::default()
        }]);
        true
    })
}

impl PenState {
    fn append(&mut self, readings: impl IntoIterator<Item = PenSample>) {
        self.trail.extend(readings);
        let over = self.trail.len().saturating_sub(TRAIL_CAP);
        self.trail.drain(..over);
        for end in &mut self.ends {
            end.0 = end.0.saturating_sub(over);
        }
    }
}

/// A move's readings join the trail the next `mousemove` reports; a mouse's are its positions
/// (none while the pointer is locked, when every move is at the lock's position). A reading
/// without a platform time takes its delivery's.
pub(crate) fn note_move(window: &mut Window, pressed_button: Option<gpui::MouseButton>) {
    let now = Instant::now();
    let readings = match window.vn_pen() {
        Some(pen) => {
            let mut readings = pen.history.clone();
            readings.push(pen.sample);
            for r in &mut readings {
                r.time.get_or_insert(now);
            }
            readings
        }
        None if window.vn_pointer_locked() => return,
        None => vec![PenSample {
            position: window.mouse_position(),
            pressure: windows::here(window, |w| {
                let down = pressed_button == Some(gpui::MouseButton::Left);
                if !down {
                    w.pen.mouse_pressure = None;
                }
                w.pen.primary_down = down;
                w.pen.mouse_pressure.unwrap_or(0.5)
            }),
            time: Some(now),
            ..Default::default()
        }],
    };
    windows::here(window, |w| w.pen.append(readings));
}

/// A press, release or exit: the pointer's prediction starts over after the readings so far.
pub(crate) fn end_stroke(window: &mut Window) {
    let key = key(window);
    windows::here(window, |w| {
        let at = w.pen.trail.len();
        w.pen.ends.push((at, key));
    });
}

pub(crate) fn clear_trail(window: &mut Window) {
    windows::here(window, |w| {
        let pen = &mut w.pen;
        pen.trail.clear();
        for (_, key) in pen.ends.drain(..) {
            pen.predictors.remove(&key);
        }
    });
}

/// The runtime's clock (`performance.now()`, `event.timeStamp`) read with an `Instant`.
struct Clock {
    at: Instant,
    ms: f64,
}

impl Clock {
    fn now() -> Self {
        let ms = sk_runtime::redjs::web::now_ms();
        Self {
            at: Instant::now(),
            ms,
        }
    }

    fn ms(&self, t: Option<Instant>) -> f64 {
        t.map_or(self.ms, |t| {
            self.ms - self.at.saturating_duration_since(t).as_secs_f64() * 1000.0
        })
    }
}

/// The trail with its times, oldest first and never decreasing, and the predicted readings after
/// it; the trail feeds `key`'s predictor.
fn take_move(window: &mut Window, key: Key) -> (Vec<(PenSample, f64)>, Vec<(f64, f64, f64)>) {
    let clock = Clock::now();
    windows::here(window, |w| {
        let pen = &mut w.pen;
        let trail = std::mem::take(&mut pen.trail);
        let mut ends = std::mem::take(&mut pen.ends).into_iter().peekable();
        if key.is_some() {
            pen.predictors.remove(&None);
        }
        let mut readings = Vec::with_capacity(trail.len());
        let mut prev = f64::NEG_INFINITY;
        for (i, s) in trail.into_iter().enumerate() {
            while let Some((_, ended)) = ends.next_if(|&(at, _)| at <= i) {
                pen.predictors.remove(&ended);
            }
            let t = clock.ms(s.time).max(prev);
            prev = t;
            pen.predictors.entry(key).or_default().update(
                f64::from(f32::from(s.position.x)),
                f64::from(f32::from(s.position.y)),
                t,
            );
            readings.push((s, t));
        }
        let predicted = match pen.predictors.get(&key) {
            Some(p) if !readings.is_empty() => p.predictions(),
            _ => Vec::new(),
        };
        for (_, ended) in ends {
            pen.predictors.remove(&ended);
        }
        (readings, predicted)
    })
}

fn mouse_family(event: u32) -> bool {
    matches!(
        event,
        tree::EV_MOUSEDOWN
            | tree::EV_MOUSEUP
            | tree::EV_MOUSEMOVE
            | tree::EV_MOUSEENTER
            | tree::EV_MOUSELEAVE
            | tree::EV_MOUSEOVER
            | tree::EV_MOUSEOUT
            | tree::EV_POINTERCANCEL
            | tree::EV_CLICK
            | tree::EV_AUXCLICK
            | tree::EV_CONTEXTMENU
    )
}

/// Arms the next dispatch with the pen's data when `event` comes from one; a `mousemove` takes
/// the trail and its predictions, a mouse's too.
pub(crate) fn arm(window: &mut Window, event: u32) {
    NEXT.with(|n| n.borrow_mut().take());
    if !mouse_family(event) {
        return;
    }
    if let Some(packet) = FORCED.with(|c| c.borrow().clone()) {
        NEXT.with(|n| *n.borrow_mut() = Some(packet));
        return;
    }
    if let Some(packet) = crate::touch::mouse_packet(window) {
        if event == tree::EV_MOUSEMOVE {
            clear_trail(window);
        }
        NEXT.with(|n| *n.borrow_mut() = Some(packet));
        return;
    }
    let key = key(window);
    let Some(pen) = window.vn_pen() else {
        if event == tree::EV_MOUSEMOVE {
            let (readings, predicted) = take_move(window, key);
            if !readings.is_empty() {
                NEXT.with(|n| {
                    *n.borrow_mut() = Some(packet(MOUSE + MOUSE_PRESSURE, &readings, &predicted))
                });
            }
        } else if let Some(pressure) = windows::here(window, |w| w.pen.mouse_pressure) {
            let sample = PenSample {
                position: window.mouse_position(),
                pressure,
                ..Default::default()
            };
            NEXT.with(|n| {
                *n.borrow_mut() = Some(packet(
                    MOUSE + MOUSE_PRESSURE,
                    &[(sample, Clock::now().ms)],
                    &[],
                ))
            });
        }
        return;
    };
    let mut flags = 0.0;
    if pen.eraser {
        flags += ERASER;
    }
    if pen.barrel {
        flags += BARREL;
    }
    if pen.in_contact {
        flags += CONTACT;
    }
    let latest = pen.sample;
    let (mut readings, predicted) = if event == tree::EV_MOUSEMOVE {
        take_move(window, key)
    } else {
        (Vec::new(), Vec::new())
    };
    if readings.is_empty() {
        readings.push((latest, Clock::now().ms(latest.time)));
    }
    NEXT.with(|n| *n.borrow_mut() = Some(packet(flags, &readings, &predicted)));
}

/// `[flags, readings.len(), ...readings, ...predicted]`, oldest first; the last reading is the
/// event's own.
pub fn packet(
    flags: f64,
    readings: &[(PenSample, f64)],
    predicted: &[(f64, f64, f64)],
) -> Vec<f64> {
    let mut out =
        Vec::with_capacity(2 + readings.len() * SAMPLE_LEN + predicted.len() * PREDICTED_LEN);
    out.push(flags);
    out.push(readings.len() as f64);
    for (s, time) in readings {
        out.extend_from_slice(&[
            f64::from(f32::from(s.position.x)),
            f64::from(f32::from(s.position.y)),
            f64::from(s.pressure),
            f64::from(s.tangential_pressure),
            f64::from(s.tilt_x),
            f64::from(s.tilt_y),
            f64::from(s.twist),
            f64::from(s.width),
            f64::from(s.height),
            *time,
        ]);
    }
    for &(x, y, time) in predicted {
        out.extend_from_slice(&[x, y, time]);
    }
    out
}

pub(crate) fn take() -> Option<Vec<f64>> {
    NEXT.with(|n| n.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    #[test]
    fn trackpad_pressure_is_continuous_across_force_click_stages() {
        use gpui::PressureStage::*;
        assert_eq!(normalized_pressure(Zero, 1.0), Some(0.0));
        assert_eq!(normalized_pressure(Normal, -1.0), Some(0.0));
        assert_eq!(normalized_pressure(Normal, 0.5), Some(0.25));
        assert_eq!(
            normalized_pressure(Normal, 1.0),
            normalized_pressure(Force, 0.0)
        );
        assert_eq!(normalized_pressure(Force, 0.5), Some(0.75));
        assert_eq!(normalized_pressure(Force, 2.0), Some(1.0));
        assert_eq!(normalized_pressure(Force, f32::NAN), None);
    }

    fn at(x: f32, pressure: f32) -> PenSample {
        PenSample {
            position: point(px(x), px(2.0)),
            pressure,
            tilt_x: 10.0,
            ..Default::default()
        }
    }

    #[test]
    fn a_packet_holds_the_readings_oldest_first_then_the_predictions() {
        let p = packet(
            ERASER + CONTACT,
            &[(at(1.0, 0.1), 5.0), (at(3.0, 0.3), 9.0)],
            &[(4.0, 2.0, 13.0)],
        );
        assert_eq!(p.len(), 2 + 2 * SAMPLE_LEN + PREDICTED_LEN);
        assert_eq!(&p[..5], &[5.0, 2.0, 1.0, 2.0, f64::from(0.1f32)]);
        assert_eq!(p[2 + SAMPLE_LEN - 1], 5.0);
        assert_eq!(p[2 + SAMPLE_LEN], 3.0);
        assert_eq!(&p[2 + 2 * SAMPLE_LEN..], &[4.0, 2.0, 13.0]);
    }
}
