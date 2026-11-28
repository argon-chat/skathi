//! Touch contacts as `pointerType: "touch"` pointers. The primary contact drives gpui's mouse,
//! carrying its own packet; every other contact is hit-tested, captured and delivered here.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui::{
    App, MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, PenSample,
    Pixels, PlatformInput, Point, TouchEvent, TouchId, TouchPhase, Window,
};

use crate::render::{js_wave_app, mouse_payload};
use crate::{pointer, tree, windows};

/// `pointerId`s for touch contacts: after the mouse's (1) and the pen's (2), one per contact,
/// never two alive at once.
const FIRST_ID: u32 = 3;
const LAST_ID: u32 = 1 << 20;

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(FIRST_ID) };
}

fn next_pointer_id(taken: impl Fn(u32) -> bool) -> u32 {
    NEXT_ID.with(|next| loop {
        let id = next.get();
        next.set(if id >= LAST_ID { FIRST_ID } else { id + 1 });
        if !taken(id) {
            return id;
        }
    })
}

struct Contact {
    id: TouchId,
    pointer_id: u32,
    primary: bool,
    sample: PenSample,
    delivered: Point<Pixels>,
    pending: bool,
    /// Where it is over, target first: the boundary events' last state.
    over: Vec<u32>,
    capture: Option<u32>,
    taps: usize,
}

/// How the primary contact's release reaches the up wave its `MouseUp` opens.
pub(crate) enum Release {
    /// Lifted: the wave leaves the window after `mouseup`, before `click`.
    Up {
        packet: Vec<f64>,
        position: Point<Pixels>,
    },
    Cancel,
}

type Tap = (Instant, Point<Pixels>, usize);

#[derive(Default)]
pub(crate) struct TouchState {
    contacts: Vec<Contact>,
    /// The primary contact gpui's mouse events currently stand for, and its latest reading.
    mouse: Option<(u32, PenSample)>,
    synthesizing: bool,
    release: Option<Release>,
    /// The script released the primary contact's capture during its press.
    released: bool,
    last_tap: Option<Tap>,
}

impl TouchState {
    pub(crate) fn take_release(&mut self) -> Option<Release> {
        self.release.take()
    }

    /// A new contact: primary when no other is down.
    fn begin(&mut self, id: TouchId, sample: PenSample) -> (u32, bool) {
        let primary = self.contacts.is_empty();
        let pointer_id = next_pointer_id(|p| self.contacts.iter().any(|c| c.pointer_id == p));
        self.contacts.push(Contact {
            id,
            pointer_id,
            primary,
            sample,
            delivered: sample.position,
            pending: false,
            over: Vec::new(),
            capture: None,
            taps: 1,
        });
        (pointer_id, primary)
    }

    fn get(&mut self, id: TouchId) -> Option<&mut Contact> {
        self.contacts.iter_mut().find(|c| c.id == id)
    }

    fn finish(&mut self, id: TouchId) -> Option<Contact> {
        let at = self.contacts.iter().position(|c| c.id == id)?;
        Some(self.contacts.remove(at))
    }
}

fn sample_of(ev: &TouchEvent) -> PenSample {
    let (width, height) = ev
        .size
        .map_or((0.0, 0.0), |s| (f32::from(s.width), f32::from(s.height)));
    PenSample {
        position: ev.position,
        pressure: ev.force.unwrap_or(0.5),
        width,
        height,
        time: Some(Instant::now()),
        ..Default::default()
    }
}

/// The packet gpui's mouse events carry while they stand for the primary contact.
pub(crate) fn mouse_packet(window: &Window) -> Option<Vec<f64>> {
    let (pointer_id, sample) = windows::here(window, |w| w.touch.mouse)?;
    Some(crate::pen::touch_packet(pointer_id, true, &sample))
}

/// A mouse or pen event of the platform's own: gpui's mouse is no longer the finger's. A primary
/// finger still down is cancelled first, under its own packet, so the runtime ends its contact.
pub(crate) fn note_mouse_event(window: &mut Window, cx: &mut App) {
    let interrupted = windows::here(window, |w| {
        if w.touch.synthesizing {
            return None;
        }
        w.touch.mouse = None;
        let c = w.touch.contacts.iter().find(|c| c.primary)?;
        w.touch.mouse = Some((c.pointer_id, c.sample));
        Some((c.id, c.pointer_id, c.sample))
    });
    let Some((id, pointer_id, sample)) = interrupted else {
        return;
    };
    let packet = crate::pen::touch_packet(pointer_id, true, &sample);
    crate::render::cancel_touch(window, cx, sample.position);
    pointer::hover_now(window, cx, sample.position, false, &packet);
    windows::here(window, |w| {
        w.touch.finish(id);
        w.touch.mouse = None;
    });
}

/// The count a tap at `at` starts, after the tap `last`, as a click count: taps close enough in
/// time and place make a double tap.
fn next_tap_count(
    last: Option<Tap>,
    now: Instant,
    at: Point<Pixels>,
    interval: Duration,
    slop: (f32, f32),
) -> usize {
    match last {
        Some((t, p, n))
            if now.duration_since(t) < interval
                && f32::from(at.x - p.x).abs() <= slop.0
                && f32::from(at.y - p.y).abs() <= slop.1 =>
        {
            n + 1
        }
        _ => 1,
    }
}

/// The system's double-click time and distance, the distance in window pixels.
#[cfg(windows)]
fn double_tap(scale: f32) -> (Duration, (f32, f32)) {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
    };
    // SAFETY: plain queries.
    let (ms, cx, cy) = unsafe {
        (
            GetDoubleClickTime(),
            GetSystemMetrics(SM_CXDOUBLECLK),
            GetSystemMetrics(SM_CYDOUBLECLK),
        )
    };
    (
        Duration::from_millis(u64::from(ms)),
        (cx as f32 / scale, cy as f32 / scale),
    )
}

#[cfg(not(windows))]
fn double_tap(_scale: f32) -> (Duration, (f32, f32)) {
    (Duration::from_millis(500), (4.0, 4.0))
}

pub(crate) fn handle(ev: &TouchEvent, window: &mut Window, cx: &mut App) -> bool {
    match ev.phase {
        TouchPhase::Started => {
            if windows::here(window, |w| w.touch.get(ev.id).is_some()) {
                end(ev.id, true, window, cx);
            }
            let sample = sample_of(ev);
            let (pointer_id, primary) = windows::here(window, |w| w.touch.begin(ev.id, sample));
            if primary {
                primary_down(ev.id, pointer_id, sample, window, cx);
            } else {
                secondary_down(ev.id, window, cx);
            }
        }
        TouchPhase::Moved => {
            let sample = sample_of(ev);
            let primary = windows::here(window, |w| {
                let pointer_id = w.touch.get(ev.id).map(|c| {
                    c.sample = sample;
                    c.pending = !c.primary;
                    (c.pointer_id, c.primary)
                });
                match pointer_id {
                    Some((pointer_id, true)) => {
                        w.touch.mouse = Some((pointer_id, sample));
                        Some(true)
                    }
                    Some((_, false)) => Some(false),
                    None => None,
                }
            });
            match primary {
                Some(true) => synthesize(window, cx, |modifiers| {
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position: sample.position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers,
                    })
                }),
                Some(false) => {
                    sk_runtime::frames::wake();
                    window.refresh();
                }
                None => {}
            }
        }
        TouchPhase::Ended | TouchPhase::Cancelled => {
            let known = windows::here(window, |w| {
                w.touch.get(ev.id).map(|c| {
                    c.sample = PenSample {
                        position: ev.position,
                        ..c.sample
                    };
                })
            });
            if known.is_some() {
                end(ev.id, ev.phase == TouchPhase::Cancelled, window, cx);
            }
        }
    }
    true
}

/// Ends contact `id`: released, or cancelled because something else took it (the system, or a
/// gesture of the host's own).
pub(crate) fn end(id: TouchId, cancelled: bool, window: &mut Window, cx: &mut App) {
    let primary = windows::here(window, |w| w.touch.get(id).map(|c| c.primary));
    match primary {
        Some(true) => primary_end(id, cancelled, window, cx),
        Some(false) => secondary_end(id, cancelled, window, cx),
        None => {}
    }
}

fn primary_down(
    id: TouchId,
    pointer_id: u32,
    sample: PenSample,
    window: &mut Window,
    cx: &mut App,
) {
    let position = sample.position;
    let (interval, slop) = double_tap(window.scale_factor());
    let taps = windows::here(window, |w| {
        let taps = next_tap_count(w.touch.last_tap, Instant::now(), position, interval, slop);
        w.touch.last_tap = Some((Instant::now(), position, taps));
        if let Some(c) = w.touch.get(id) {
            c.taps = taps;
        }
        w.touch.mouse = Some((pointer_id, sample));
        w.touch.released = false;
        taps
    });
    let packet = crate::pen::touch_packet(pointer_id, true, &sample);
    pointer::hover_now(window, cx, position, true, &packet);
    let target = pointer::target_at(window, position);
    synthesize(window, cx, |modifiers| {
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: taps,
            first_mouse: false,
        })
    });
    // Implicit capture, as on the web, even when no listener armed a press: the finger's
    // `pointerup` goes to where it went down unless the script released it.
    let capturable = pointer::capturable_in(windows::key(window), target);
    windows::here(window, |w| {
        let down = w.touch.get(id).is_some();
        if down && capturable && !w.touch.released && w.pointer_capture.is_none() {
            w.pointer_capture = Some(target);
        }
    });
}

fn primary_end(id: TouchId, cancelled: bool, window: &mut Window, cx: &mut App) {
    let Some(contact) = windows::here(window, |w| w.touch.finish(id)) else {
        return;
    };
    let position = contact.sample.position;
    let packet = crate::pen::touch_packet(contact.pointer_id, true, &contact.sample);
    windows::here(window, |w| {
        w.touch.mouse = Some((contact.pointer_id, contact.sample));
        w.touch.release = Some(if cancelled {
            Release::Cancel
        } else {
            Release::Up {
                packet: packet.clone(),
                position,
            }
        });
    });
    if cancelled {
        crate::render::cancel_touch(window, cx, position);
    }
    // Sent even when cancelled, so gpui ends its press; the wave it opens fires nothing then.
    synthesize(window, cx, |modifiers| {
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: contact.taps,
        })
    });
    synthesize(window, cx, |modifiers| {
        PlatformInput::MouseExited(MouseExitEvent {
            position,
            pressed_button: None,
            modifiers,
        })
    });
    if !crate::render::touch_leave_scheduled(window) {
        pointer::hover_now(window, cx, position, false, &packet);
    }
}

/// Dispatches a mouse event standing for the primary contact through gpui.
fn synthesize(
    window: &mut Window,
    cx: &mut App,
    input: impl FnOnce(gpui::Modifiers) -> PlatformInput,
) {
    let input = input(window.modifiers());
    windows::here(window, |w| w.touch.synthesizing = true);
    window.dispatch_event(input, cx);
    windows::here(window, |w| w.touch.synthesizing = false);
}

fn secondary_target(window: &Window, id: TouchId) -> Option<(u32, Vec<u32>, Point<Pixels>)> {
    let (capture, position) = windows::here(window, |w| {
        w.touch.get(id).map(|c| (c.capture, c.sample.position))
    })?;
    let k = windows::key(window);
    let captured = capture.filter(|&node| pointer::capturable_in(k, node));
    if capture.is_some() && captured.is_none() {
        windows::here(window, |w| {
            if let Some(c) = w.touch.get(id) {
                c.capture = None;
            }
        });
    }
    let path = match captured {
        Some(node) => pointer::path_of(window, node),
        None => pointer::path_at(window, position),
    };
    Some((*path.first()?, path, position))
}

/// A press, release or cancel is always delivered: the runtime holds the contact's capture
/// from its press to its release.
fn send(
    id: TouchId,
    node: u32,
    event: u32,
    button: f64,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some((pointer_id, sample)) = windows::here(window, |w| {
        w.touch.get(id).map(|c| (c.pointer_id, c.sample))
    }) else {
        return true;
    };
    let payload = mouse_payload(sample.position, button, &window.modifiers());
    let packet = crate::pen::touch_packet(pointer_id, false, &sample);
    crate::pen::with_packet(packet, || {
        js_wave_app(window, cx, || {
            crate::haptics::dispatch(node, event, &payload)
        })
    })
}

fn boundary(id: TouchId, new: Vec<u32>, button: f64, window: &mut Window, cx: &mut App) {
    let Some((pointer_id, sample, old)) = windows::here(window, |w| {
        w.touch.get(id).map(|c| {
            let old = std::mem::replace(&mut c.over, new.clone());
            (c.pointer_id, c.sample, old)
        })
    }) else {
        return;
    };
    let packet = crate::pen::touch_packet(pointer_id, false, &sample);
    crate::pen::with_packet(packet, || {
        pointer::boundary_events(&old, &new, sample.position, button, window, cx)
    });
}

fn secondary_down(id: TouchId, window: &mut Window, cx: &mut App) {
    let Some((target, path, _)) = secondary_target(window, id) else {
        return;
    };
    boundary(id, path, -1.0, window, cx);
    // Implicit capture, as on the web: a direct-manipulation pointer stays with the element it
    // went down on until released.
    windows::here(window, |w| {
        if let Some(c) = w.touch.get(id) {
            c.capture = Some(target);
        }
    });
    send(id, target, tree::EV_MOUSEDOWN, 0.0, window, cx);
}

fn secondary_move(id: TouchId, window: &mut Window, cx: &mut App) {
    let Some((target, path, position)) = secondary_target(window, id) else {
        return;
    };
    let from = windows::here(window, |w| {
        w.touch.get(id).map(|c| {
            c.pending = false;
            std::mem::replace(&mut c.delivered, position)
        })
    });
    let Some(from) = from else { return };
    boundary(id, path, 0.0, window, cx);
    if tree::read_tree(|t| crate::render::path_listens(t, target, tree::EV_MOUSEMOVE)) {
        crate::pointer_at_next_dispatch(
            f64::from(position.x - from.x),
            f64::from(position.y - from.y),
        );
        send(id, target, tree::EV_MOUSEMOVE, 0.0, window, cx);
    }
}

fn secondary_end(id: TouchId, cancelled: bool, window: &mut Window, cx: &mut App) {
    if !cancelled && windows::here(window, |w| w.touch.get(id).is_some_and(|c| c.pending)) {
        secondary_move(id, window, cx);
    }
    if let Some((target, _, _)) = secondary_target(window, id) {
        if cancelled {
            send(id, target, tree::EV_POINTERCANCEL, -1.0, window, cx);
        } else {
            send(id, target, tree::EV_MOUSEUP, 0.0, window, cx);
        }
    }
    boundary(id, Vec::new(), -1.0, window, cx);
    windows::here(window, |w| w.touch.finish(id));
}

/// Delivers the moves of the contacts other than the primary, once a frame as for the mouse.
pub(crate) fn flush(window: &mut Window, cx: &mut App) {
    let moved: Vec<TouchId> = windows::here(window, |w| {
        w.touch
            .contacts
            .iter()
            .filter(|c| c.pending)
            .map(|c| c.id)
            .collect()
    });
    for id in moved {
        secondary_move(id, window, cx);
    }
}

/// `setPointerCapture` / `releasePointerCapture` for a touch `pointerId`.
pub(crate) fn capture(node: u32, pointer_id: u32, capture: bool) -> bool {
    if !capture {
        let mut mouse = false;
        windows::each(|_, w| {
            if w.touch.mouse.is_some_and(|(p, _)| p == pointer_id) {
                mouse = true;
                w.touch.released = true;
            }
            for c in &mut w.touch.contacts {
                if c.pointer_id == pointer_id && c.capture == Some(node) {
                    c.capture = None;
                }
            }
        });
        if mouse {
            pointer::capture_mouse(node, false);
        }
        return true;
    }
    let k = windows::window_of_node(node);
    let primary = windows::peek(k, |w| {
        w.touch
            .contacts
            .iter()
            .find(|c| c.pointer_id == pointer_id)
            .map(|c| c.primary)
    })
    .flatten();
    match primary {
        Some(true) => pointer::capture_mouse(node, true),
        Some(false) if pointer::capturable_in(k, node) => {
            windows::of(k, |w| {
                if let Some(c) = w
                    .touch
                    .contacts
                    .iter_mut()
                    .find(|c| c.pointer_id == pointer_id)
                {
                    c.capture = Some(node);
                }
            });
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> PenSample {
        PenSample {
            position: gpui::point(gpui::px(x), gpui::px(0.0)),
            ..Default::default()
        }
    }

    #[test]
    fn ten_contacts_get_their_own_ids_and_only_the_first_is_primary() {
        let mut s = TouchState::default();
        let begun: Vec<(u32, bool)> = (0..10).map(|i| s.begin(TouchId(i), at(i as f32))).collect();
        let mut ids: Vec<u32> = begun.iter().map(|b| b.0).collect();
        assert!(
            ids.iter().all(|&id| id >= FIRST_ID),
            "1 and 2 are the mouse's and the pen's"
        );
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 10);
        assert_eq!(
            begun.iter().filter(|b| b.1).count(),
            1,
            "one primary contact"
        );
        assert!(begun[0].1);
    }

    #[test]
    fn the_primary_lifting_makes_no_other_contact_primary_until_the_sequence_ends() {
        let mut s = TouchState::default();
        s.begin(TouchId(1), at(0.0));
        s.begin(TouchId(2), at(1.0));
        assert!(s.finish(TouchId(1)).is_some_and(|c| c.primary));
        let (_, third) = s.begin(TouchId(3), at(2.0));
        assert!(!third, "another finger is still down");
        assert!(s.contacts.iter().all(|c| !c.primary));
        s.finish(TouchId(2));
        s.finish(TouchId(3));
        let (_, next) = s.begin(TouchId(4), at(3.0));
        assert!(next, "a new touch sequence");
    }

    #[test]
    fn taps_close_in_time_and_place_count_up() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let p = |x: f32, y: f32| gpui::point(gpui::px(x), gpui::px(y));
        let count = |last, dt: u64, at| next_tap_count(last, t0 + ms(dt), at, ms(500), (4.0, 4.0));
        assert_eq!(count(None, 0, p(0.0, 0.0)), 1);
        let first = Some((t0, p(10.0, 10.0), 1));
        assert_eq!(count(first, 200, p(13.0, 6.0)), 2);
        assert_eq!(count(Some((t0, p(10.0, 10.0), 2)), 200, p(10.0, 10.0)), 3);
        assert_eq!(count(first, 600, p(10.0, 10.0)), 1, "too late");
        assert_eq!(count(first, 200, p(15.0, 10.0)), 1, "too far");
    }

    #[test]
    fn an_id_in_use_is_skipped_when_the_counter_wraps() {
        NEXT_ID.with(|n| n.set(LAST_ID));
        let first = next_pointer_id(|_| false);
        assert_eq!(first, LAST_ID);
        let wrapped = next_pointer_id(|id| id == FIRST_ID);
        assert_eq!(wrapped, FIRST_ID + 1);
    }
}
