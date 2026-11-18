//! What a pen adds to the mouse events gpui dispatches for it (`Window::vn_pen`): the runtime
//! builds `pointerType: "pen"` pointer events from it. A move carries every reading (for a mouse,
//! every position) since the previous delivered move: its `getCoalescedEvents()`.

use std::cell::RefCell;

use gpui::{PenSample, Window};

use crate::tree;
use crate::windows;

/// Numbers per reading: x, y, pressure, tangentialPressure, tiltX, tiltY, twist, width, height.
pub const SAMPLE_LEN: usize = 9;
/// Flags leading the packet.
pub const ERASER: f64 = 1.0;
pub const BARREL: f64 = 2.0;
pub const CONTACT: f64 = 4.0;
/// The readings are a mouse's positions.
pub const MOUSE: f64 = 8.0;

/// A pen that never gets a frame (a hidden window) must not grow without bound.
const TRAIL_CAP: usize = 1024;

thread_local! {
    static NEXT: RefCell<Option<Vec<f64>>> = const { RefCell::new(None) };
}

/// A move's readings join the trail the next `mousemove` reports; a mouse's are its positions
/// (none while the pointer is locked, when every move is at the lock's position).
pub(crate) fn note_move(window: &mut Window) {
    let readings = match window.vn_pen() {
        Some(pen) => {
            let mut readings = pen.history.clone();
            readings.push(pen.sample);
            readings
        }
        None if window.vn_pointer_locked() => return,
        None => vec![PenSample {
            position: window.mouse_position(),
            ..Default::default()
        }],
    };
    windows::here(window, |w| {
        w.pen_trail.extend(readings);
        let over = w.pen_trail.len().saturating_sub(TRAIL_CAP);
        w.pen_trail.drain(..over);
    });
}

pub(crate) fn clear_trail(window: &mut Window) {
    windows::here(window, |w| w.pen_trail.clear());
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
/// the trail.
pub(crate) fn arm(window: &mut Window, event: u32) {
    NEXT.with(|n| n.borrow_mut().take());
    if !mouse_family(event) {
        return;
    }
    let Some(pen) = window.vn_pen() else {
        if event == tree::EV_MOUSEMOVE {
            let trail = windows::here(window, |w| std::mem::take(&mut w.pen_trail));
            if trail.len() > 1 {
                let latest = trail[trail.len() - 1];
                NEXT.with(|n| *n.borrow_mut() = Some(packet(MOUSE, &trail, latest)));
            }
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
    let trail = if event == tree::EV_MOUSEMOVE {
        windows::here(window, |w| std::mem::take(&mut w.pen_trail))
    } else {
        Vec::new()
    };
    NEXT.with(|n| *n.borrow_mut() = Some(packet(flags, &trail, latest)));
}

/// `[flags, ...readings]`, oldest first; the last reading is the event's own.
pub fn packet(flags: f64, trail: &[PenSample], latest: PenSample) -> Vec<f64> {
    let readings: &[PenSample] = if trail.is_empty() {
        std::slice::from_ref(&latest)
    } else {
        trail
    };
    let mut out = Vec::with_capacity(1 + readings.len() * SAMPLE_LEN);
    out.push(flags);
    for s in readings {
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
        ]);
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

    fn at(x: f32, pressure: f32) -> PenSample {
        PenSample {
            position: point(px(x), px(2.0)),
            pressure,
            tilt_x: 10.0,
            ..Default::default()
        }
    }

    #[test]
    fn a_packet_holds_the_trail_oldest_first_or_the_latest_reading() {
        let p = packet(
            ERASER + CONTACT,
            &[at(1.0, 0.1), at(3.0, 0.3)],
            at(3.0, 0.3),
        );
        assert_eq!(p.len(), 1 + 2 * SAMPLE_LEN);
        assert_eq!(p[0], 5.0);
        assert_eq!(&p[1..4], &[1.0, 2.0, f64::from(0.1f32)]);
        assert_eq!(p[1 + SAMPLE_LEN], 3.0);
        let lone = packet(0.0, &[], at(7.0, 0.0));
        assert_eq!(lone.len(), 1 + SAMPLE_LEN);
        assert_eq!((lone[1], lone[5]), (7.0, 10.0));
    }
}
