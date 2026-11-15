//! The Pointer Lock API. One document, so one lock: an element, held in the window that paints it.
//!
//! JS asks ([`sk_pointer_lock_request`], [`sk_pointer_lock_exit`]); each window's frame settles what
//! was asked of it against the platform ([`tick`]) and tells the document with
//! `pointerlockchange` / `pointerlockerror` (`a` = the request settled, 0 for none). Escape
//! ([`intercept_escape`]), losing focus, removing the element, a reload and closing the window let
//! go. After the user lets go, only a newer gesture locks again (Chromium's rule).
//!
//! `SK_POINTER_LOCK=fake` simulates the lock in gpui without touching the cursor and counts every
//! window as focused (tests); `deny` refuses every request.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use gpui::{App, KeystrokeEvent, PointerLockError, VnPointerLock, Window};
use sk_runtime::host::{self, global, Settler};
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host_fn, JsRuntime};

use crate::native_dom;

use crate::tree::{self, Kind};
use crate::windows::{self, WinKey};
use crate::EventPayload;

/// Transient activation's lifespan (Chromium's).
const ACTIVATION: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    System,
    Simulated,
    Denied,
}

fn mode() -> Mode {
    thread_local! { static MODE: Cell<Option<Mode>> = const { Cell::new(None) }; }
    MODE.with(|m| {
        *m.get().get_or_insert_with(|| {
            match std::env::var("SK_POINTER_LOCK")
                .ok()
                .as_deref()
                .map(str::trim)
            {
                Some(v) if v.eq_ignore_ascii_case("fake") => Mode::Simulated,
                Some(v) if v.eq_ignore_ascii_case("deny") => Mode::Denied,
                _ => Mode::System,
            }
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    Unsupported,
    Refused,
    Denied,
}

impl From<PointerLockError> for Refusal {
    fn from(e: PointerLockError) -> Self {
        match e {
            PointerLockError::Unsupported => Refusal::Unsupported,
            PointerLockError::Refused => Refusal::Refused,
        }
    }
}

/// What the state machine needs from one window and the tree.
trait Host {
    fn window(&self) -> WinKey;
    fn focused(&self) -> bool;
    fn lock(&mut self, unadjusted: bool) -> Result<(), Refusal>;
    fn unlock(&mut self);
    fn locked(&self) -> bool;
    /// The window painting `element`; `None` when it is no connected element.
    fn window_of(&self, element: u32) -> Option<WinKey>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Note {
    /// `pointerlockchange`, settling request `request` (0: none).
    Change { request: u32 },
    Error {
        request: u32,
        name: &'static str,
        message: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    element: u32,
    window: WinKey,
    unadjusted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Request {
        id: u32,
        element: u32,
        unadjusted: bool,
        /// The gesture the request was made under, if one was still active.
        gesture: Option<u64>,
    },
    Exit,
}

#[derive(Default)]
struct State {
    held: Option<Held>,
    ops: Vec<Op>,
    next_id: u32,
    gestures: u64,
    gesture_at: Option<Instant>,
    /// The gesture count when the user let go: a new lock needs a later gesture.
    released_after: Option<u64>,
    /// Windows whose system lock outlived its document (a reload).
    unlock_owed: Vec<WinKey>,
    /// Changes for a window that can no longer tell (it closed).
    owed: Vec<Note>,
}

impl State {
    fn note_gesture(&mut self, now: Instant) {
        self.gestures += 1;
        self.gesture_at = Some(now);
    }

    fn active_gesture(&self, now: Instant) -> Option<u64> {
        let at = self.gesture_at?;
        (now.saturating_duration_since(at) <= ACTIVATION).then_some(self.gestures)
    }

    fn request(&mut self, element: u32, unadjusted: bool, now: Instant) -> u32 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.ops.push(Op::Request {
            id,
            element,
            unadjusted,
            gesture: self.active_gesture(now),
        });
        id
    }

    fn exit(&mut self) {
        self.ops.push(Op::Exit);
    }

    fn held_in(&self, window: WinKey) -> Option<Held> {
        self.held.filter(|h| h.window == window)
    }

    fn release_by_user(&mut self, host: &mut dyn Host) -> Option<Note> {
        self.held_in(host.window())?;
        host.unlock();
        self.held = None;
        self.released_after = Some(self.gestures);
        Some(Note::Change { request: 0 })
    }

    fn tick(&mut self, host: &mut dyn Host) -> Vec<Note> {
        let here = host.window();
        let mut notes = std::mem::take(&mut self.owed);
        if let Some(at) = self.unlock_owed.iter().position(|&w| w == here) {
            self.unlock_owed.swap_remove(at);
            if self.held_in(here).is_none() {
                host.unlock();
            }
        }
        if let Some(held) = self.held_in(here) {
            if !host.locked() || !host.focused() {
                notes.extend(self.release_by_user(host));
            } else if host.window_of(held.element) != Some(here) {
                host.unlock();
                self.held = None;
                notes.push(Note::Change { request: 0 });
            }
        }
        for op in std::mem::take(&mut self.ops) {
            match op {
                Op::Request {
                    id,
                    element,
                    unadjusted,
                    gesture,
                } => match host.window_of(element) {
                    None => notes.push(Note::Error {
                        request: id,
                        name: "WrongDocumentError",
                        message: "The element is not connected to the document.",
                    }),
                    Some(w) if w != here => self.ops.push(op),
                    Some(_) => self.grant(host, id, element, unadjusted, gesture, &mut notes),
                },
                Op::Exit => match self.held {
                    Some(held) if held.window == here => {
                        host.unlock();
                        self.held = None;
                        notes.push(Note::Change { request: 0 });
                    }
                    Some(_) => self.ops.push(op),
                    None => {}
                },
            }
        }
        notes
    }

    fn grant(
        &mut self,
        host: &mut dyn Host,
        id: u32,
        element: u32,
        unadjusted: bool,
        gesture: Option<u64>,
        notes: &mut Vec<Note>,
    ) {
        let here = host.window();
        let error = |name, message| Note::Error {
            request: id,
            name,
            message,
        };
        if !host.focused() {
            return notes.push(error(
                "WrongDocumentError",
                "Pointer lock needs the window to be focused.",
            ));
        }
        let current = self.held_in(here);
        if current.is_none() {
            let Some(gesture) = gesture else {
                return notes.push(error(
                    "NotAllowedError",
                    "requestPointerLock() needs a user gesture: call it from a click or key \
                     press handler.",
                ));
            };
            if self.released_after.is_some_and(|after| gesture <= after) {
                return notes.push(error(
                    "NotAllowedError",
                    "The user released the pointer lock: requesting it again needs a new user \
                     gesture.",
                ));
            }
        }
        if let Some(other) = self.held.filter(|h| h.window != here) {
            self.unlock_owed.push(other.window);
            self.held = None;
            notes.push(Note::Change { request: 0 });
        }
        if current.is_some_and(|held| held.unadjusted == unadjusted) {
            self.held = Some(Held {
                element,
                window: here,
                unadjusted,
            });
            return notes.push(Note::Change { request: id });
        }
        match host.lock(unadjusted) {
            Ok(()) => {
                self.held = Some(Held {
                    element,
                    window: here,
                    unadjusted,
                });
                notes.push(Note::Change { request: id });
            }
            Err(refusal) => {
                if current.is_some() {
                    self.held = None;
                    notes.push(Note::Change { request: 0 });
                }
                notes.push(match refusal {
                    Refusal::Unsupported if unadjusted => error(
                        "NotSupportedError",
                        "unadjustedMovement is not supported on this platform.",
                    ),
                    Refusal::Unsupported => error(
                        "NotSupportedError",
                        "Pointer lock is not supported on this platform.",
                    ),
                    Refusal::Refused => {
                        error("NotAllowedError", "The system refused to lock the pointer.")
                    }
                    Refusal::Denied => error(
                        "NotAllowedError",
                        "Pointer lock is off for this process (SK_POINTER_LOCK=deny).",
                    ),
                });
            }
        }
    }

    fn window_closed(&mut self, window: WinKey) {
        self.unlock_owed.retain(|&w| w != window);
        if self.held_in(window).is_some() {
            self.held = None;
            self.owed.push(Note::Change { request: 0 });
        }
    }

    /// The document went away (a reload): its lock with it, without events.
    fn navigated(&mut self) {
        if let Some(held) = self.held.take() {
            self.unlock_owed.push(held.window);
        }
        self.ops.clear();
        self.owed.clear();
        self.released_after = None;
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The promise each unsettled `requestPointerLock()` returned.
    static PENDING: RefCell<HashMap<u32, Settler>> = RefCell::new(HashMap::new());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

struct WindowHost<'a> {
    window: &'a mut Window,
    mode: Mode,
}

impl Host for WindowHost<'_> {
    fn window(&self) -> WinKey {
        windows::key(self.window)
    }

    fn focused(&self) -> bool {
        self.mode == Mode::Simulated || self.window.is_window_active()
    }

    fn lock(&mut self, unadjusted: bool) -> Result<(), Refusal> {
        let how = match self.mode {
            Mode::Denied => return Err(Refusal::Denied),
            Mode::Simulated => VnPointerLock::Simulated,
            Mode::System => VnPointerLock::System { unadjusted },
        };
        Ok(self.window.vn_lock_pointer(how)?)
    }

    fn unlock(&mut self) {
        self.window.vn_unlock_pointer();
    }

    fn locked(&self) -> bool {
        self.window.vn_pointer_locked()
    }

    fn window_of(&self, element: u32) -> Option<WinKey> {
        tree::read_tree(|t| {
            let element_here = t.node(element).is_some_and(|n| n.kind == Kind::Element)
                && crate::fullscreen::connected(t, element);
            element_here.then(|| windows::window_of_node_in(t, element))
        })
    }
}

/// The element holding the lock in `window`, while connected: where its mouse events go.
pub(crate) fn target_in(window: &Window) -> Option<u32> {
    let held = with_state(|s| s.held_in(windows::key(window)))?;
    let connected = tree::read_tree(|t| crate::fullscreen::connected(t, held.element));
    connected.then_some(held.element)
}

/// A press or a key press: transient activation, as far as the lock is concerned.
pub(crate) fn note_gesture() {
    with_state(|s| s.note_gesture(Instant::now()));
}

/// Settles this window's requests and notices a lock the platform, the focus or the tree let go.
pub(crate) fn tick(window: &mut Window, cx: &mut App) {
    let idle = with_state(|s| {
        s.held.is_none() && s.ops.is_empty() && s.owed.is_empty() && s.unlock_owed.is_empty()
    });
    if idle {
        return;
    }
    let notes = with_state(|s| {
        s.tick(&mut WindowHost {
            window,
            mode: mode(),
        })
    });
    deliver(notes, window, cx);
}

/// Escape releases the lock before anything else hears the key, and nothing on the page does.
/// Every other key press counts as a gesture.
pub fn intercept_escape(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    let key = event.keystroke.key.as_str();
    if key != "escape" {
        if !matches!(
            key,
            "shift" | "control" | "alt" | "platform" | "function" | "capslock"
        ) {
            note_gesture();
        }
        return;
    }
    let released = with_state(|s| {
        s.release_by_user(&mut WindowHost {
            window,
            mode: mode(),
        })
    });
    let Some(note) = released else {
        return;
    };
    crate::render::claim_key_event(window);
    cx.stop_propagation();
    deliver(vec![note], window, cx);
}

pub(crate) fn window_closed(window: WinKey) {
    with_state(|s| s.window_closed(window));
}

/// The document went away: its requests are never answered.
pub fn navigated() {
    with_state(State::navigated);
    let abandoned: Vec<Settler> =
        PENDING.with(|p| p.borrow_mut().drain().map(|(_, s)| s).collect());
    if !abandoned.is_empty() {
        JsRuntime::try_with(|rt| {
            for s in abandoned {
                unsafe { s.abandon(rt.raw()) };
            }
        });
    }
}

/// Fires each note's event at the document, then settles the request it answers.
fn deliver(notes: Vec<Note>, window: &mut Window, cx: &mut App) {
    for note in notes {
        let code = match note {
            Note::Change { .. } => tree::EV_POINTERLOCKCHANGE,
            Note::Error { .. } => tree::EV_POINTERLOCKERROR,
        };
        crate::off_input(|| {
            crate::render::js_dispatch_app(0, code, &EventPayload::default(), window, cx)
        });
        JsRuntime::try_with(|rt| unsafe { settle(rt.raw(), &note) });
    }
}

unsafe fn settle(g: *mut c_void, note: &Note) {
    let request = match *note {
        Note::Change { request } | Note::Error { request, .. } => request,
    };
    let Some(settler) = PENDING.with(|p| p.borrow_mut().remove(&request)) else {
        return;
    };
    match *note {
        Note::Change { .. } => settler.resolve(g, UNDEF),
        Note::Error { name, message, .. } => settler.reject(g, host::error(g, name, message)),
    }
}

host_fn!(fn request_pointer_lock(g, args) {
    let options = args.arg(0).0;
    let unadjusted = is_cell(options) && unsafe { host::truthy(g, get(g, options, "unadjustedMovement")) };
    let element = unsafe { native_dom::node_id(g, args.this().0) }.unwrap_or(0);
    let id = with_state(|s| s.request(element, unadjusted, Instant::now()));
    tree::with_tree(|t| t.mark_dirty());
    let (promise, settler) = unsafe { host::deferred(g) };
    PENDING.with(|p| p.borrow_mut().insert(id, settler));
    promise
});

host_fn!(fn exit_pointer_lock(_g, _args) {
    with_state(State::exit);
    tree::with_tree(|t| t.mark_dirty());
    UNDEF
});

host_fn!(fn pointer_lock_element(g, _args) {
    let held = with_state(|s| s.held);
    let element = held
        .filter(|h| tree::read_tree(|t| crate::fullscreen::connected(t, h.element)))
        .map_or(0, |h| h.element);
    unsafe { native_dom::node_by_id(g, element) }
});

pub(crate) fn install(rt: &JsRuntime) {
    unsafe {
        let g = rt.raw();
        native_dom::method(
            g,
            native_dom::proto(g, "Element"),
            "requestPointerLock",
            0,
            request_pointer_lock,
        );
        let document = get(g, global(g), "document");
        native_dom::method(g, document, "exitPointerLock", 0, exit_pointer_lock);
        native_dom::accessor(
            g,
            document,
            "pointerLockElement",
            pointer_lock_element,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const W: WinKey = 7;
    const OTHER: WinKey = 8;

    struct Fake {
        window: WinKey,
        focused: bool,
        locked: bool,
        refuse: Option<Refusal>,
        unadjusted_supported: bool,
        elements: HashMap<u32, WinKey>,
        calls: Vec<String>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                window: W,
                focused: true,
                locked: false,
                refuse: None,
                unadjusted_supported: true,
                elements: HashMap::from([(1, W), (2, W), (3, OTHER)]),
                calls: Vec::new(),
            }
        }
    }

    impl Host for Fake {
        fn window(&self) -> WinKey {
            self.window
        }
        fn focused(&self) -> bool {
            self.focused
        }
        fn lock(&mut self, unadjusted: bool) -> Result<(), Refusal> {
            self.calls.push(format!("lock {unadjusted}"));
            if let Some(r) = self.refuse {
                return Err(r);
            }
            if unadjusted && !self.unadjusted_supported {
                return Err(Refusal::Unsupported);
            }
            self.locked = true;
            Ok(())
        }
        fn unlock(&mut self) {
            self.calls.push("unlock".into());
            self.locked = false;
        }
        fn locked(&self) -> bool {
            self.locked
        }
        fn window_of(&self, element: u32) -> Option<WinKey> {
            self.elements.get(&element).copied()
        }
    }

    fn gestured() -> (State, Instant) {
        let mut s = State::default();
        let now = Instant::now();
        s.note_gesture(now);
        (s, now)
    }

    fn change(request: u32) -> Note {
        Note::Change { request }
    }

    fn error_name(note: &Note) -> &'static str {
        match note {
            Note::Error { name, .. } => name,
            Note::Change { .. } => panic!("expected an error, got {note:?}"),
        }
    }

    #[test]
    fn a_request_under_a_gesture_locks_and_exit_releases() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        let id = s.request(1, false, now);
        assert!(
            s.held.is_none(),
            "nothing happens before the window's frame"
        );
        assert_eq!(s.tick(&mut host), vec![change(id)]);
        assert_eq!(s.held.map(|h| h.element), Some(1));
        assert!(host.locked);

        s.exit();
        assert_eq!(s.tick(&mut host), vec![change(0)]);
        assert!(s.held.is_none() && !host.locked);
        s.exit();
        assert_eq!(s.tick(&mut host), vec![], "exiting when unlocked is silent");
    }

    #[test]
    fn a_request_without_a_gesture_or_focus_fails() {
        let mut s = State::default();
        let mut host = Fake::new();
        s.request(1, false, Instant::now());
        let notes = s.tick(&mut host);
        assert_eq!(notes.len(), 1);
        assert_eq!(error_name(&notes[0]), "NotAllowedError");

        let (mut s, now) = gestured();
        host.focused = false;
        let id2 = s.request(1, false, now);
        let notes = s.tick(&mut host);
        assert!(
            matches!(notes[..], [Note::Error { request, name: "WrongDocumentError", .. }] if request == id2)
        );
        assert!(host.calls.is_empty(), "the platform is never asked");
    }

    #[test]
    fn a_gesture_expires() {
        let mut s = State::default();
        let then = Instant::now();
        s.note_gesture(then);
        s.request(1, false, then + ACTIVATION + Duration::from_millis(1));
        let notes = s.tick(&mut Fake::new());
        assert_eq!(error_name(&notes[0]), "NotAllowedError");
    }

    #[test]
    fn escape_releases_and_the_next_request_needs_a_new_gesture() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        s.request(1, false, now);
        s.tick(&mut host);

        assert_eq!(s.release_by_user(&mut host), Some(change(0)));
        assert!(!host.locked);
        assert_eq!(s.release_by_user(&mut host), None, "nothing to release");

        s.request(1, false, now);
        let notes = s.tick(&mut host);
        assert_eq!(error_name(&notes[0]), "NotAllowedError");

        s.note_gesture(now);
        let id = s.request(1, false, now);
        assert_eq!(s.tick(&mut host), vec![change(id)]);
    }

    #[test]
    fn losing_focus_or_the_platform_lock_releases_as_the_user() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        s.request(1, false, now);
        s.tick(&mut host);
        host.focused = false;
        assert_eq!(s.tick(&mut host), vec![change(0)]);
        assert!(!host.locked && s.held.is_none());

        host.focused = true;
        s.note_gesture(now);
        s.request(1, false, now);
        s.tick(&mut host);
        host.locked = false;
        assert_eq!(s.tick(&mut host), vec![change(0)]);
        assert_eq!(s.released_after, Some(s.gestures));
    }

    #[test]
    fn removing_the_element_releases_without_needing_a_new_gesture() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        s.request(1, false, now);
        s.tick(&mut host);
        host.elements.remove(&1);
        assert_eq!(s.tick(&mut host), vec![change(0)]);
        assert!(!host.locked);
        assert_eq!(s.released_after, None);
    }

    #[test]
    fn a_detached_element_is_a_wrong_document() {
        let (mut s, now) = gestured();
        s.request(99, false, now);
        let notes = s.tick(&mut Fake::new());
        assert_eq!(error_name(&notes[0]), "WrongDocumentError");
    }

    #[test]
    fn moving_the_lock_to_another_element_needs_no_gesture_or_platform_call() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        s.request(1, false, now);
        s.tick(&mut host);
        let later = now + ACTIVATION * 2;
        let id = s.request(2, false, later);
        assert_eq!(s.tick(&mut host), vec![change(id)]);
        assert_eq!(s.held.map(|h| h.element), Some(2));
        assert_eq!(host.calls, vec!["lock false"]);

        let id = s.request(2, true, later);
        assert_eq!(s.tick(&mut host), vec![change(id)]);
        assert_eq!(host.calls, vec!["lock false", "lock true"]);
    }

    #[test]
    fn platform_refusals_name_their_reason() {
        let cases = [
            (Some(Refusal::Refused), false, "NotAllowedError"),
            (Some(Refusal::Denied), false, "NotAllowedError"),
            (Some(Refusal::Unsupported), false, "NotSupportedError"),
            (None, true, "NotSupportedError"),
        ];
        for (refuse, unadjusted, name) in cases {
            let (mut s, now) = gestured();
            let mut host = Fake::new();
            host.refuse = refuse;
            host.unadjusted_supported = false;
            s.request(1, unadjusted, now);
            let notes = s.tick(&mut host);
            assert_eq!(
                error_name(&notes[0]),
                name,
                "{refuse:?} unadjusted={unadjusted}"
            );
            assert!(s.held.is_none());
        }
    }

    #[test]
    fn a_failed_mode_change_loses_the_lock_it_had() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        host.unadjusted_supported = false;
        s.request(1, false, now);
        s.tick(&mut host);
        s.request(1, true, now);
        let notes = s.tick(&mut host);
        assert_eq!(notes[0], change(0));
        assert_eq!(error_name(&notes[1]), "NotSupportedError");
        assert!(s.held.is_none());
    }

    #[test]
    fn a_request_waits_for_the_window_painting_its_element() {
        let (mut s, now) = gestured();
        let mut here = Fake::new();
        let id = s.request(3, false, now);
        assert_eq!(s.tick(&mut here), vec![]);
        let mut there = Fake::new();
        there.window = OTHER;
        assert_eq!(s.tick(&mut there), vec![change(id)]);
        assert_eq!(s.held.map(|h| h.window), Some(OTHER));

        s.exit();
        assert_eq!(s.tick(&mut here), vec![]);
        assert_eq!(s.tick(&mut there), vec![change(0)]);
    }

    #[test]
    fn closing_the_window_tells_the_document_at_the_next_frame() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        s.request(1, false, now);
        s.tick(&mut host);
        s.window_closed(W);
        assert!(s.held.is_none());
        let mut other = Fake::new();
        other.window = OTHER;
        assert_eq!(s.tick(&mut other), vec![change(0)]);
    }

    fn js(src: &str) -> String {
        JsRuntime::with(|rt| {
            let v = rt
                .ctx()
                .eval(src)
                .unwrap_or_else(|e| panic!("{src}: {e:?}"));
            rt.ctx().to_rust_string(v).unwrap()
        })
    }

    #[test]
    fn request_pointer_lock_is_a_promise_the_settling_note_answers() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                native_dom::install(rt);
                install(rt);
            });
            with_state(|s| *s = State::default());
            let el = tree::with_tree(|t| {
                let root = t.create_root();
                let tag = t.intern("canvas");
                let el = t.create_element(tag);
                t.insert(el, root, 0);
                el
            });
            js(&format!(
                "globalThis.canvas = Object.create(HTMLElement.prototype); canvas.__skId = {el}; \
                 __sk_set_node_lookup((id) => (id === {el} ? canvas : undefined)); globalThis.seen = []; \
                 canvas.requestPointerLock({{ unadjustedMovement: true }}).then(() => seen.push('locked')); \
                 canvas.requestPointerLock().catch((e) => seen.push(`${{e instanceof DOMException}} ${{e.name}}: ${{e.message}}`)); \
                 Object.create(HTMLElement.prototype).requestPointerLock(); ''"
            ));
            let requests: Vec<(u32, u32, bool)> = with_state(|s| {
                s.ops
                    .iter()
                    .filter_map(|op| match *op {
                        Op::Request {
                            id,
                            element,
                            unadjusted,
                            ..
                        } => Some((id, element, unadjusted)),
                        Op::Exit => None,
                    })
                    .collect()
            });
            let [(first, a, true), (second, b, false), (_, 0, false)] = requests[..] else {
                panic!("{requests:?}");
            };
            assert_eq!((a, b), (el, el));
            assert_eq!(js("String(document.pointerLockElement)"), "null");

            with_state(|s| {
                s.held = Some(Held {
                    element: el,
                    window: W,
                    unadjusted: true,
                });
            });
            JsRuntime::with(|rt| unsafe {
                settle(
                    rt.raw(),
                    &Note::Error {
                        request: second,
                        name: "WrongDocumentError",
                        message: "Not here.",
                    },
                );
                settle(rt.raw(), &Note::Change { request: first });
                settle(rt.raw(), &Note::Change { request: first });
            });
            assert_eq!(js("String(document.pointerLockElement === canvas)"), "true");
            js("''");
            assert_eq!(
                js("seen.join('|')"),
                "true WrongDocumentError: Not here.|locked"
            );

            js("document.exitPointerLock()");
            assert!(with_state(|s| s.ops.contains(&Op::Exit)));
            navigated();
            assert!(
                PENDING.with(|p| p.borrow().is_empty()),
                "a reload forgets every request"
            );
        });
    }

    #[test]
    fn a_reload_drops_the_lock_silently_and_its_window_unlocks() {
        let (mut s, now) = gestured();
        let mut host = Fake::new();
        s.request(1, false, now);
        s.tick(&mut host);
        s.request(2, false, now);
        s.navigated();
        assert_eq!(s.tick(&mut host), vec![]);
        assert!(!host.locked);
        assert_eq!(host.calls, vec!["lock false", "unlock"]);
    }
}
