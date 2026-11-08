//! Modal native element picking; consumed gestures keep their release latches
//! across stop, lease expiry and restart until the physical gesture completes.
use crate::json::Json;
use gpui::{App, MouseButton, PlatformInput, Window};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

const LEASE: Duration = Duration::from_secs(15);
#[derive(Default)]
struct Picker {
    active: bool,
    generation: u64,
    until: Option<Instant>,
    window: Option<u64>,
    point: (f32, f32),
    path: Vec<u32>,
    selected: bool,
    cancelled: bool,
    mouse_releases: Vec<MouseButton>,
    key_releases: Vec<String>,
}
impl Picker {
    fn begin(&mut self, now: Instant) {
        let generation = self.generation.wrapping_add(1);
        let mouse_releases = std::mem::take(&mut self.mouse_releases);
        let key_releases = std::mem::take(&mut self.key_releases);
        *self = Self {
            active: true,
            generation,
            until: Some(now + LEASE),
            mouse_releases,
            key_releases,
            ..Default::default()
        };
    }
    fn cancel(&mut self) {
        self.active = false;
        self.cancelled = !self.selected;
        self.until = None;
    }
    fn expire(&mut self, now: Instant) -> bool {
        if self.active && self.until.is_some_and(|until| now >= until) {
            self.cancel();
            return true;
        }
        false
    }
    fn track_mouse(&mut self, button: MouseButton) {
        if !self.mouse_releases.contains(&button) {
            self.mouse_releases.push(button);
        }
    }
    fn release_mouse(&mut self, button: MouseButton) -> bool {
        if let Some(index) = self
            .mouse_releases
            .iter()
            .position(|pending| *pending == button)
        {
            self.mouse_releases.swap_remove(index);
            true
        } else {
            false
        }
    }
    fn track_key(&mut self, key: &str) {
        if self.key_releases.len() < 256 && !self.key_releases.iter().any(|pending| pending == key)
        {
            self.key_releases.push(key.to_owned());
        }
    }
    fn release_key(&mut self, key: &str) -> bool {
        if let Some(index) = self.key_releases.iter().position(|pending| pending == key) {
            self.key_releases.swap_remove(index);
            true
        } else {
            false
        }
    }
    fn pending(&self) -> bool {
        !self.mouse_releases.is_empty() || !self.key_releases.is_empty()
    }
}
thread_local! { static STATE: RefCell<Picker> = RefCell::new(Picker::default()); }

pub(super) fn stop() {
    STATE.with(|state| state.borrow_mut().cancel());
    // Escape runs inside Window::dispatch_event: never recursively update a
    // Window here. Inactive filters remove themselves after their last release.
    *super::HIGHLIGHT
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
}
fn refresh_all() {
    for id in super::window_ids() {
        let _ = super::with_window_id(id, |window, _| window.refresh());
    }
}
fn arm() {
    for id in super::window_ids() {
        let _ = super::with_window_id(id, |window, _| {
            window.set_input_interceptor(Some(Rc::new(intercept)));
            window.refresh();
        });
    }
}
pub(super) fn command(op: &str) -> String {
    match op {
        "start" => {
            stop();
            STATE.with(|state| state.borrow_mut().begin(Instant::now()));
            arm();
        }
        "stop" => {
            stop();
            refresh_all();
        }
        "status" => {
            let (active, expired) = STATE.with(|slot| {
                let mut state = slot.borrow_mut();
                let now = Instant::now();
                let expired = state.expire(now);
                if state.active {
                    state.until = Some(now + LEASE);
                }
                (state.active, expired)
            });
            if expired {
                *super::HIGHLIGHT
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = None;
                refresh_all();
            }
            if active {
                arm();
            }
        }
        _ => return super::err("pick op must be start, status or stop"),
    }
    super::ok(STATE.with(|slot| {
        let state = slot.borrow();
        let mut fields = vec![
            ("generation", Json::n(state.generation as f64)),
            ("active", Json::Bool(state.active)),
            ("selected", Json::Bool(state.selected)),
            ("cancelled", Json::Bool(state.cancelled)),
            ("x", Json::n(state.point.0)),
            ("y", Json::n(state.point.1)),
            ("insideWindow", Json::Bool(!state.path.is_empty())),
            (
                "path",
                Json::arr(
                    state
                        .path
                        .iter()
                        .map(|&id| Json::n(f64::from(id)))
                        .collect(),
                ),
            ),
            (
                "node",
                state
                    .path
                    .last()
                    .map_or(Json::Null, |&id| super::node_json(id, false)),
            ),
        ];
        if let Some(id) = state.window {
            fields.push(("window", Json::n(id as f64)));
        }
        Json::obj(fields)
    }))
}

fn intercept(event: &PlatformInput, window: &mut Window, _: &mut App) -> bool {
    let expired = STATE.with(|slot| slot.borrow_mut().expire(Instant::now()));
    if expired {
        *super::HIGHLIGHT
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        window.refresh();
    }
    // Release identity follows the physical button/key, even if native capture
    // or focus moved to another window in this process while it was held.
    let consumed_release = STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        match event {
            PlatformInput::MouseUp(up) => state.release_mouse(up.button),
            PlatformInput::KeyUp(up) => state.release_key(&up.keystroke.key),
            _ => false,
        }
    });
    let (active, pending, dragging, repeating_key) = STATE.with(|slot| {
        let state = slot.borrow();
        (state.active, state.pending(), !state.mouse_releases.is_empty(), matches!(event, PlatformInput::KeyDown(down) if state.key_releases.iter().any(|key| *key == down.keystroke.key)))
    });
    if consumed_release {
        if !active && !pending {
            window.set_input_interceptor(None);
        }
        return true;
    }
    if !active {
        if repeating_key {
            return true;
        }
        if dragging {
            match event {
                PlatformInput::MouseDown(down) => {
                    STATE.with(|slot| slot.borrow_mut().track_mouse(down.button));
                    return true;
                }
                PlatformInput::MouseMove(_)
                | PlatformInput::MouseUp(_)
                | PlatformInput::Scroll(_) => return true,
                _ => {}
            }
        }
        if !pending {
            window.set_input_interceptor(None);
        }
        return false;
    }
    match event {
        PlatformInput::KeyDown(key) if key.keystroke.key == "escape" => {
            STATE.with(|slot| slot.borrow_mut().track_key(&key.keystroke.key));
            stop();
            window.refresh();
            true
        }
        PlatformInput::MouseMove(event) => {
            hover(event.position, window);
            true
        }
        PlatformInput::MouseDown(event) => {
            hover(event.position, window);
            STATE.with(|slot| {
                let mut state = slot.borrow_mut();
                state.active = false;
                state.selected = !state.path.is_empty();
                state.cancelled = !state.selected;
                state.track_mouse(event.button);
                state.until = None;
            });
            true
        }
        PlatformInput::KeyDown(key) => {
            STATE.with(|slot| slot.borrow_mut().track_key(&key.keystroke.key));
            true
        }
        PlatformInput::MouseUp(_)
        | PlatformInput::Scroll(_)
        | PlatformInput::KeyUp(_)
        | PlatformInput::Touch(_) => true,
        _ => false,
    }
}
fn hover(point: gpui::Point<gpui::Pixels>, window: &mut Window) {
    let id = vn_tree::windows::key(window);
    let (x, y) = (f32::from(point.x), f32::from(point.y));
    let path = vn_tree::devtools::hit_test_in(super::window_root(id), x, y, window);
    let node = path.last().copied();
    STATE.with(|slot| {
        let mut state = slot.borrow_mut();
        state.window = Some(id);
        state.point = (x, y);
        state.path = path;
    });
    *super::HIGHLIGHT
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = node.map(|node| super::Highlight {
        node,
        window: Some(id),
        until: Some(Instant::now() + LEASE),
        label: None,
    });
    window.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_or_restarted_picker_keeps_complete_mouse_gesture() {
        let now = Instant::now();
        let mut state = Picker::default();
        state.begin(now);
        state.track_mouse(MouseButton::Left);
        assert!(state.expire(now + LEASE + Duration::from_secs(1)));
        assert!(state.pending(), "expiry forwarded an orphan release");
        state.begin(now + LEASE + Duration::from_secs(2));
        assert!(state.pending(), "restart forgot the preceding gesture");
        assert!(state.release_mouse(MouseButton::Left));
        assert!(state.active, "old release cancelled the newly armed picker");
        assert!(!state.pending());
    }
    #[test]
    fn escape_cancel_waits_for_key_release_and_deduplicates_repeat() {
        let mut state = Picker::default();
        state.begin(Instant::now());
        state.track_key("escape");
        state.track_key("escape");
        state.cancel();
        assert!(!state.active);
        assert!(state.pending());
        assert_eq!(state.key_releases.len(), 1);
        assert!(state.release_key("escape"));
        assert!(!state.pending());
        assert!(!state.release_key("escape"));
    }
    #[test]
    fn selected_state_survives_held_button_and_stop() {
        let now = Instant::now();
        let mut state = Picker::default();
        state.begin(now);
        state.track_mouse(MouseButton::Left);
        state.active = false;
        state.selected = true;
        state.until = None;
        assert!(!state.expire(now + LEASE * 2));
        state.cancel();
        assert!(state.selected && !state.cancelled);
        assert!(state.pending());
        assert!(state.release_mouse(MouseButton::Left));
    }
}
