//! `<iframe>`: a document in a platform webview, drawn into the element's box.
//!
//! This module knows the tree (which `<iframe>` nodes exist, where their boxes are, how document
//! events become JS events), not the webview: a [`Backend`] does, registered with [`set_backend`].
//! With none (Linux, no WebView2 runtime, tests) every `src` is answered with `error`.
//!
//! A document exists while the element is CONNECTED and has a `src`/`srcdoc`. Attribute writes and
//! insertion/removal [`note`] the node; [`tick`] reconciles each (create, navigate, drop). Moving
//! the element keeps the frame: the node id is its identity. Backend events arrive on the UI thread
//! outside any gpui frame, so the [`Sink`] queues and [`tick`] dispatches `load`/`error`/`message`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use gpui::{App, Bounds, Pixels, Window};

use crate::tree::{self, Tree};
use crate::EventPayload;

/// What the element's attributes ask the document to be.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameAttrs {
    /// The document's URL, resolved against the app's origin by the backend.
    pub src: Option<String>,
    /// An inline document; wins over `src` when present, as in HTML.
    pub srcdoc: Option<String>,
    /// The `sandbox` attribute, verbatim. `None`: not sandboxed.
    pub sandbox: Option<String>,
    /// The `allow` attribute (Permissions Policy), verbatim.
    pub allow: Option<String>,
    pub referrer_policy: Option<String>,
    pub name: Option<String>,
    pub allow_fullscreen: bool,
}

impl FrameAttrs {
    /// Is there a document to load at all?
    fn has_document(&self) -> bool {
        self.srcdoc.is_some() || self.src.is_some()
    }

    /// The text `load` / `error` name the navigation by.
    fn key(&self) -> &str {
        self.src.as_deref().unwrap_or("about:srcdoc")
    }
}

/// Where the document is on screen this frame, in device pixels of its window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The element's content box.
    pub bounds: [f32; 4],
    /// The part of it that is visible: the box cut by every clipping ancestor.
    pub clip: [f32; 4],
    /// Corner radii of the box, top-left clockwise.
    pub radii: [f32; 4],
    /// The window's scale factor.
    pub scale: f32,
    /// The element's own `opacity` times its ancestors'.
    pub opacity: f32,
}

/// What a document tells its element.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameEvent {
    /// The navigation finished and the document's own `load` fired.
    Loaded,
    /// The navigation failed, or the browser process went away.
    Failed(String),
    /// `window.parent.postMessage(..)` in the document: the sender's serialized origin (`"null"`
    /// for an opaque one) and the payload's wire text.
    Message { origin: String, wire: String },
    /// The pointer the document wants over its box changed.
    Cursor(gpui::CursorStyle),
}

/// Callable from the UI thread at any time; never re-enters JS — the event is queued for the next
/// [`tick`].
#[derive(Clone)]
pub struct Sink {
    node: u32,
    wake: Rc<dyn Fn()>,
}

impl Sink {
    pub fn send(&self, event: FrameEvent) {
        EVENTS.with(|q| q.borrow_mut().push_back((self.node, event)));
        (self.wake)();
    }
}

/// A pointer event over the visible part of the document's box (R6.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerInput {
    pub kind: PointerKind,
    /// Device pixels from the box's top-left corner.
    pub x: f32,
    pub y: f32,
    /// Buttons held: 1 left, 2 right, 4 middle.
    pub buttons: u8,
    pub shift: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerKind {
    Move,
    /// 0 left, 1 middle, 2 right — the DOM's `button`.
    Down(u8),
    Up(u8),
    /// Device pixels to scroll by, x then y, positive down and right.
    Wheel(f32, f32),
    /// The pointer left the box.
    Leave,
}

/// One live document.
pub trait Frame {
    /// Load what `attrs` describe, replacing the current document.
    fn navigate(&mut self, attrs: &FrameAttrs);
    /// Deliver a message to the document's `window` (R2.1).
    fn post(&mut self, wire: &str, target_origin: &str);
    /// Show the document at `placement`, or hide it (`None`: not painted this frame — `display:
    /// none`, culled, scrolled out, R5.5).
    fn place(&mut self, placement: Option<Placement>);
    /// The document goes with it and is not reloaded (R1.8).
    fn rehome(&mut self, window: &mut Window);
    /// Give the document the keyboard focus, or take it back (R1.6).
    fn focus(&mut self, on: bool);
    /// A pointer event the app's own hit test resolved to this box.
    fn pointer(&mut self, input: PointerInput);
}

/// The platform's webview.
pub trait Backend {
    /// Cheap: asked by `vnFeatures.iframe` on every read.
    fn supported(&self) -> bool;
    /// A new, empty frame in `window`.
    fn create(&self, node: u32, sink: Sink, window: &mut Window, cx: &mut App) -> Option<Box<dyn Frame>>;
}

struct Live {
    frame: Box<dyn Frame>,
    attrs: FrameAttrs,
    win: crate::windows::WinKey,
    /// Has the current navigation been answered?
    answered: bool,
    placed: Option<Placement>,
    /// The pointer the document asks for over its box.
    cursor: gpui::CursorStyle,
}

/// How many documents may be open at once in the process (R7.4).
pub const MAX_FRAMES: usize = 8;

thread_local! {
    static BACKEND: RefCell<Option<Rc<dyn Backend>>> = const { RefCell::new(None) };
    /// Wakes the UI loop from a backend callback (set with the backend).
    static WAKE: RefCell<Rc<dyn Fn()>> = RefCell::new(Rc::new(|| {}));
    /// Nodes to reconcile on the next tick.
    static NOTED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static WAITING: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static FRAMES: RefCell<HashMap<u32, Live>> = RefCell::new(HashMap::new());
    static EVENTS: RefCell<VecDeque<(u32, FrameEvent)>> = const { RefCell::new(VecDeque::new()) };
    /// The iframes the paint walk reached since the last tick.
    static PAINTED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// The windows that built an element tree since the last tick.
    static DREW: RefCell<Vec<crate::windows::WinKey>> = const { RefCell::new(Vec::new()) };
    static REFUSED: RefCell<HashMap<u32, FrameAttrs>> = RefCell::new(HashMap::new());
}

/// Installs the platform's webview.
pub fn set_backend(backend: Rc<dyn Backend>, wake: Rc<dyn Fn()>) {
    BACKEND.with(|b| *b.borrow_mut() = Some(backend));
    WAKE.with(|w| *w.borrow_mut() = wake);
}

/// `vnFeatures.iframe`.
pub fn supported() -> bool {
    BACKEND.with(|b| b.borrow().as_ref().is_some_and(|b| b.supported()))
}

/// Is `name` an attribute whose change the document has to hear about?
pub(crate) fn is_frame_attr(name: &str) -> bool {
    matches!(name, "src" | "srcdoc" | "sandbox" | "allow" | "referrerpolicy" | "name" | "allowfullscreen")
}

/// An `<iframe>` changed in a way [`tick`] has to look at.
pub(crate) fn note(id: u32) {
    NOTED.with(|n| n.borrow_mut().insert(id));
}

pub(crate) fn note_structure() {
    let waiting: Vec<u32> = WAITING.with(|w| w.borrow().iter().copied().collect());
    let live: Vec<u32> = FRAMES.with(|f| f.borrow().keys().copied().collect());
    if waiting.is_empty() && live.is_empty() {
        return;
    }
    NOTED.with(|n| n.borrow_mut().extend(waiting.into_iter().chain(live)));
}

fn attrs_of(t: &Tree, id: u32) -> FrameAttrs {
    let get = |name: &str| t.prop(id, name).map(str::to_string);
    FrameAttrs {
        src: get("src"),
        srcdoc: get("srcdoc"),
        sandbox: get("sandbox"),
        allow: get("allow"),
        referrer_policy: get("referrerpolicy"),
        name: get("name"),
        allow_fullscreen: t.prop(id, "allowfullscreen").is_some(),
    }
}

/// Is the node in a document: does its parent chain end at a root?
fn connected(t: &Tree, id: u32) -> bool {
    let mut at = id;
    for _ in 0..4096 {
        let Some(node) = t.node(at) else { return false };
        if node.parent == 0 {
            return t.name(node.tag) == "root";
        }
        at = node.parent;
    }
    false
}

/// What [`tick`] should do about one noted node.
#[derive(Debug, PartialEq)]
enum Step {
    /// The node is gone, disconnected, or has nothing to load.
    Drop,
    /// Not connected yet, but has a document to load once it is.
    Wait,
    /// Connected, with a document.
    Show(FrameAttrs),
}

fn step(t: &Tree, id: u32) -> Step {
    let Some(node) = t.node(id) else { return Step::Drop };
    if t.name(node.tag) != "iframe" {
        return Step::Drop;
    }
    let attrs = attrs_of(t, id);
    if !attrs.has_document() {
        return Step::Drop;
    }
    if connected(t, id) { Step::Show(attrs) } else { Step::Wait }
}

/// Called once per frame from [`crate::frame_tick`].
pub(crate) fn tick(window: &mut Window, cx: &mut App) {
    let noted: Vec<u32> = NOTED.with(|n| n.borrow_mut().drain().collect());
    let win = crate::windows::key(window);
    for id in noted {
        match tree::read_tree(|t| step(t, id)) {
            Step::Drop => {
                WAITING.with(|w| w.borrow_mut().remove(&id));
                REFUSED.with(|r| r.borrow_mut().remove(&id));
                // Dropped here, on the UI thread, with the window at hand.
                drop(FRAMES.with(|f| f.borrow_mut().remove(&id)));
            }
            Step::Wait => {
                WAITING.with(|w| w.borrow_mut().insert(id));
                drop(FRAMES.with(|f| f.borrow_mut().remove(&id)));
            }
            Step::Show(attrs) => {
                WAITING.with(|w| w.borrow_mut().remove(&id));
                show(id, attrs, win, window, cx);
            }
        }
    }
    place_all(win);
    sync_focus(win, window);
    deliver(window, cx);
}

fn show(id: u32, attrs: FrameAttrs, win: crate::windows::WinKey, window: &mut Window, cx: &mut App) {
    let backend = BACKEND.with(|b| b.borrow().clone()).filter(|b| b.supported());
    let Some(backend) = backend else {
        let again = REFUSED.with(|r| r.borrow().get(&id) == Some(&attrs));
        if !again {
            EVENTS.with(|q| {
                q.borrow_mut().push_back((id, FrameEvent::Failed("no webview backend on this system".into())))
            });
            REFUSED.with(|r| r.borrow_mut().insert(id, attrs));
        }
        return;
    };
    let existing = FRAMES.with(|f| f.borrow().get(&id).map(|l| (l.attrs == attrs, l.win == win)));
    match existing {
        Some((true, _)) => {}
        Some((false, _)) => FRAMES.with(|f| {
            if let Some(live) = f.borrow_mut().get_mut(&id) {
                live.frame.navigate(&attrs);
                live.attrs = attrs;
                live.answered = false;
            }
        }),
        None if FRAMES.with(|f| f.borrow().len()) >= MAX_FRAMES => {
            // One more would cost the ones that are running (R7.4).
            let again = REFUSED.with(|r| r.borrow().get(&id) == Some(&attrs));
            if !again {
                EVENTS.with(|q| {
                    q.borrow_mut().push_back((
                        id,
                        FrameEvent::Failed(format!("{MAX_FRAMES} documents are already open; this one was not loaded")),
                    ))
                });
                REFUSED.with(|r| r.borrow_mut().insert(id, attrs));
            }
        }
        None => {
            REFUSED.with(|r| r.borrow_mut().remove(&id));
            let sink = Sink { node: id, wake: WAKE.with(|w| w.borrow().clone()) };
            match backend.create(id, sink, window, cx) {
                Some(mut frame) => {
                    frame.navigate(&attrs);
                    FRAMES.with(|f| {
                        f.borrow_mut().insert(id, Live {
                            frame,
                            attrs,
                            win,
                            answered: false,
                            placed: None,
                            cursor: gpui::CursorStyle::Arrow,
                        })
                    });
                }
                None => EVENTS.with(|q| {
                    q.borrow_mut().push_back((id, FrameEvent::Failed("the webview could not be created".into())))
                }),
            }
        }
    }
}

/// Hides each frame of this window that the last paint did not reach (`display: none`, culled,
/// scrolled out — R5.5).
fn place_all(win: crate::windows::WinKey) {
    let drew = DREW.with(|d| {
        let mut d = d.borrow_mut();
        let before = d.len();
        d.retain(|w| *w != win);
        d.len() != before
    });
    if !drew {
        return;
    }
    let seen: HashSet<u32> = PAINTED.with(|p| std::mem::take(&mut *p.borrow_mut()));
    FRAMES.with(|f| {
        for (id, live) in f.borrow_mut().iter_mut() {
            if live.win == win && !seen.contains(id) && live.placed.is_some() {
                live.frame.place(None);
                live.placed = None;
            }
        }
    });
    PAINTED.with(|p| {
        let mut p = p.borrow_mut();
        for id in seen {
            if FRAMES.with(|f| f.borrow().get(&id).is_some_and(|l| l.win != win)) {
                p.insert(id);
            }
        }
    });
}

/// `win` is building the element tree of a draw: the paint that follows reaches every iframe that
/// is on screen in it.
pub(crate) fn drawing(win: crate::windows::WinKey) {
    DREW.with(|d| {
        let mut d = d.borrow_mut();
        if !d.contains(&win) {
            d.push(win);
        }
    });
}

/// Its document follows it there.
pub(crate) fn painted_in(id: u32, window: &mut Window) {
    let win = crate::windows::key(window);
    FRAMES.with(|f| {
        if let Some(live) = f.borrow_mut().get_mut(&id) {
            live.frame.rehome(window);
            if live.win != win {
                live.win = win;
                live.placed = None;
            }
        }
    });
}

/// The pointer the document wants over its box.
pub(crate) fn cursor(id: u32) -> Option<gpui::CursorStyle> {
    FRAMES.with(|f| f.borrow().get(&id).map(|l| l.cursor))
}

/// Told to the frame at once, so the document moves in the frame its box does (R5.4).
pub(crate) fn placed(id: u32, placement: Placement) {
    PAINTED.with(|p| p.borrow_mut().insert(id));
    FRAMES.with(|f| {
        if let Some(live) = f.borrow_mut().get_mut(&id) {
            if live.placed != Some(placement) {
                live.frame.place(Some(placement));
                live.placed = Some(placement);
            }
        }
    });
}

/// Dispatches the queued document events at their nodes.
fn deliver(window: &mut Window, cx: &mut App) {
    loop {
        let Some((id, event)) = EVENTS.with(|q| q.borrow_mut().pop_front()) else { break };
        let key = FRAMES.with(|f| f.borrow().get(&id).map(|l| l.attrs.key().to_string()));
        let key = key.or_else(|| REFUSED.with(|r| r.borrow().get(&id).map(|a| a.key().to_string())));
        let Some(key) = key else { continue };
        match event {
            FrameEvent::Loaded | FrameEvent::Failed(_) => {
                let first = FRAMES.with(|f| match f.borrow_mut().get_mut(&id) {
                    Some(live) => !std::mem::replace(&mut live.answered, true),
                    None => true,
                });
                if !first {
                    continue;
                }
                let code = if matches!(event, FrameEvent::Loaded) { tree::EV_LOAD } else { tree::EV_ERROR };
                if let FrameEvent::Failed(why) = &event {
                    crate::log::record("warn", format!("<iframe> {key}: {why}"));
                }
                let payload = EventPayload { key: Some(&key), ..Default::default() };
                crate::render::js_dispatch_app(id, code, &payload, window, cx);
            }
            FrameEvent::Cursor(style) => {
                let changed = FRAMES.with(|f| match f.borrow_mut().get_mut(&id) {
                    Some(live) => std::mem::replace(&mut live.cursor, style) != style,
                    None => false,
                });
                if changed {
                    window.refresh();
                }
            }
            FrameEvent::Message { origin, wire } => {
                let text = message_key(&origin, &wire);
                let payload = EventPayload { key: Some(&text), ..Default::default() };
                crate::render::js_dispatch_app(id, tree::EV_MESSAGE, &payload, window, cx);
            }
        }
    }
}

/// `{"o": origin, "d": wire}` — `wire` is already JSON text.
fn message_key(origin: &str, wire: &str) -> String {
    let mut out = String::with_capacity(wire.len() + origin.len() + 16);
    out.push_str("{\"o\":\"");
    for c in origin.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str("\",\"d\":");
    out.push_str(wire);
    out.push('}');
    out
}

/// `contentWindow.postMessage`.
pub(crate) fn post(id: u32, wire: &str, target_origin: &str) {
    FRAMES.with(|f| {
        if let Some(live) = f.borrow_mut().get_mut(&id) {
            live.frame.post(wire, target_origin);
        }
    });
}

/// The element took or lost the keyboard focus.
pub(crate) fn focus(id: u32, on: bool) {
    FRAMES.with(|f| {
        if let Some(live) = f.borrow_mut().get_mut(&id) {
            live.frame.focus(on);
        }
    });
}

/// A pointer event at the element, for its document.
pub(crate) fn pointer(id: u32, kind: PointerKind, at: (f32, f32), buttons: u8, shift: bool, control: bool) {
    FRAMES.with(|f| {
        if let Some(live) = f.borrow_mut().get_mut(&id) {
            // Not painted: there is no box to be relative to.
            let Some(place) = live.placed else { return };
            let input = PointerInput { kind, x: at.0 - place.bounds[0], y: at.1 - place.bounds[1], buttons, shift, control };
            live.frame.pointer(input);
        }
    });
}

thread_local! {
    /// Per window: the iframe whose document has the keyboard.
    static FOCUSED: RefCell<HashMap<crate::windows::WinKey, u32>> = RefCell::new(HashMap::new());
}

/// The keyboard follows the app's focus (R6.3): an `<iframe>` is a focusable element, and while it
/// is the window's focused node — by a press, `Tab` or `el.focus()` — its document has the keys;
/// when the focus moves on, the window has them back.
fn sync_focus(win: crate::windows::WinKey, window: &Window) {
    let had = FOCUSED.with(|f| f.borrow().get(&win).copied().unwrap_or(0));
    if !window.is_window_active() {
        if had != 0 {
            FOCUSED.with(|f| f.borrow_mut().remove(&win));
        }
        return;
    }
    let node = crate::windows::focused_node(win);
    let want = if has_document(node) { node } else { 0 };
    if want == had {
        return;
    }
    if want != 0 {
        focus(want, true);
    } else {
        focus(had, false);
    }
    FOCUSED.with(|f| {
        let mut f = f.borrow_mut();
        if want == 0 { f.remove(&win) } else { f.insert(win, want) };
    });
}

/// Is an element of the app fullscreen? Then Escape is the app's even while a document has the
/// keyboard: it leaves fullscreen and the document does not hear it, as in a browser.
pub fn fullscreen_active() -> bool {
    tree::read_tree(|t| crate::fullscreen::current(t).is_some())
}

/// Does this node have a live document?
pub(crate) fn has_document(id: u32) -> bool {
    FRAMES.with(|f| f.borrow().contains_key(&id))
}

pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    drop(FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        let gone: Vec<u32> = f.keys().copied().filter(|&id| !live(id)).collect();
        gone.into_iter().filter_map(|id| f.remove(&id)).collect::<Vec<_>>()
    }));
    WAITING.with(|w| w.borrow_mut().retain(|&id| live(id)));
    REFUSED.with(|r| r.borrow_mut().retain(|&id, _| live(id)));
    PAINTED.with(|p| p.borrow_mut().retain(|&id| live(id)));
}

/// Their elements, if they are painted again elsewhere, load anew.
pub(crate) fn forget_window(win: crate::windows::WinKey) {
    let gone: Vec<Live> = FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        let ids: Vec<u32> = f.iter().filter(|(_, l)| l.win == win).map(|(id, _)| *id).collect();
        ids.into_iter().filter_map(|id| f.remove(&id)).collect()
    });
    drop(gone);
    FOCUSED.with(|f| f.borrow_mut().remove(&win));
    DREW.with(|d| d.borrow_mut().retain(|w| *w != win));
}

/// Every document goes: the app is quitting.
pub fn shutdown() {
    drop(FRAMES.with(|f| std::mem::take(&mut *f.borrow_mut())));
    EVENTS.with(|q| q.borrow_mut().clear());
}

/// The placement of a box painted at `bounds` under `mask`, for [`placed`].
pub(crate) fn placement(
    bounds: Bounds<Pixels>,
    mask: Bounds<Pixels>,
    radii: [f32; 4],
    scale: f32,
    opacity: f32,
) -> Placement {
    let rect = |b: Bounds<Pixels>| {
        [
            f32::from(b.origin.x) * scale,
            f32::from(b.origin.y) * scale,
            f32::from(b.size.width) * scale,
            f32::from(b.size.height) * scale,
        ]
    };
    Placement { bounds: rect(bounds), clip: rect(bounds.intersect(&mask)), radii: radii.map(|r| r * scale), scale, opacity }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{read_tree, with_tree};
    use std::cell::Cell;

    #[derive(Default)]
    struct Fake {
        on: Cell<bool>,
        log: Rc<RefCell<Vec<String>>>,
        sinks: Rc<RefCell<Vec<Sink>>>,
    }

    struct FakeFrame(u32, Rc<RefCell<Vec<String>>>);

    impl Frame for FakeFrame {
        fn navigate(&mut self, attrs: &FrameAttrs) {
            self.1.borrow_mut().push(format!("navigate {} {}", self.0, attrs.key()));
        }
        fn post(&mut self, wire: &str, target: &str) {
            self.1.borrow_mut().push(format!("post {} {wire} {target}", self.0));
        }
        fn place(&mut self, placement: Option<Placement>) {
            self.1.borrow_mut().push(format!("place {} {}", self.0, placement.is_some()));
        }
        fn rehome(&mut self, _: &mut Window) {}
        fn focus(&mut self, on: bool) {
            self.1.borrow_mut().push(format!("focus {} {on}", self.0));
        }
        fn pointer(&mut self, input: PointerInput) {
            self.1.borrow_mut().push(format!("pointer {} {:?}", self.0, input.kind));
        }
    }

    impl Drop for FakeFrame {
        fn drop(&mut self) {
            self.1.borrow_mut().push(format!("drop {}", self.0));
        }
    }

    fn iframe(src: Option<&str>, attach: bool) -> u32 {
        with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("iframe");
            let el = t.create_element(tag);
            if let Some(src) = src {
                let key = t.intern("src");
                t.set_prop(el, key, Some(src.into()));
            }
            if attach {
                t.insert(el, root, 0);
            }
            el
        })
    }

    #[test]
    fn a_document_needs_a_connected_element_with_a_src() {
        let loose = iframe(Some("/game.html"), false);
        assert_eq!(read_tree(|t| step(t, loose)), Step::Wait);
        let empty = iframe(None, true);
        assert_eq!(read_tree(|t| step(t, empty)), Step::Drop);
        let ready = iframe(Some("/game.html"), true);
        assert!(matches!(read_tree(|t| step(t, ready)), Step::Show(a) if a.src.as_deref() == Some("/game.html")));
        assert_eq!(read_tree(|t| step(t, 999_999)), Step::Drop);
    }

    #[test]
    fn srcdoc_is_a_document_without_a_src() {
        let el = iframe(None, true);
        with_tree(|t| {
            let key = t.intern("srcdoc");
            t.set_prop(el, key, Some("<p>hi</p>".into()));
        });
        let Step::Show(attrs) = read_tree(|t| step(t, el)) else { panic!("srcdoc alone is a document") };
        assert_eq!(attrs.key(), "about:srcdoc");
    }

    #[test]
    fn a_message_key_is_json_with_the_wire_embedded() {
        assert_eq!(message_key("null", "[\"o\",[]]"), "{\"o\":\"null\",\"d\":[\"o\",[]]}");
        assert_eq!(message_key("a\"b\\c", "1"), "{\"o\":\"a\\\"b\\\\c\",\"d\":1}");
    }

    #[test]
    fn nothing_is_supported_without_a_backend_and_a_backend_answers_for_itself() {
        BACKEND.with(|b| *b.borrow_mut() = None);
        assert!(!supported());
        let fake = Rc::new(Fake::default());
        set_backend(fake.clone(), Rc::new(|| {}));
        assert!(!supported(), "a backend whose runtime is missing");
        fake.on.set(true);
        assert!(supported());
        BACKEND.with(|b| *b.borrow_mut() = None);
    }

    impl Backend for Fake {
        fn supported(&self) -> bool {
            self.on.get()
        }
        fn create(&self, node: u32, sink: Sink, _: &mut Window, _: &mut App) -> Option<Box<dyn Frame>> {
            self.sinks.borrow_mut().push(sink);
            Some(Box::new(FakeFrame(node, self.log.clone())))
        }
    }

    #[test]
    fn a_frame_is_placed_once_per_change_and_hidden_when_not_painted() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let id = 77_001;
        FRAMES.with(|f| {
            f.borrow_mut().insert(id, Live {
                frame: Box::new(FakeFrame(id, log.clone())),
                attrs: FrameAttrs::default(),
                win: 5,
                answered: false,
                placed: None,
                cursor: gpui::CursorStyle::Arrow,
            })
        });
        let at = Placement { bounds: [0.0, 0.0, 10.0, 10.0], clip: [0.0, 0.0, 10.0, 10.0], radii: [0.0; 4], scale: 1.0, opacity: 1.0 };
        drawing(5);
        placed(id, at);
        placed(id, at);
        place_all(5);
        drawing(5);
        placed(id, at);
        place_all(5);
        place_all(5);
        // A draw that did not reach it hides it.
        drawing(5);
        place_all(5);
        post(id, "1", "*");
        gc(|_| false);
        assert_eq!(
            *log.borrow(),
            ["place 77001 true", "place 77001 false", "post 77001 1 *", "drop 77001"],
            "same place: no call; not painted: hidden; gone: dropped"
        );
    }

    #[test]
    fn a_placement_is_in_device_pixels_and_clipped() {
        use gpui::{point, px, size};
        let bounds = Bounds { origin: point(px(10.0), px(20.0)), size: size(px(100.0), px(50.0)) };
        let mask = Bounds { origin: point(px(0.0), px(0.0)), size: size(px(60.0), px(40.0)) };
        let p = placement(bounds, mask, [4.0; 4], 2.0, 0.5);
        assert_eq!(p.bounds, [20.0, 40.0, 200.0, 100.0]);
        assert_eq!(p.clip, [20.0, 40.0, 100.0, 40.0]);
        assert_eq!(p.radii, [8.0; 4]);
    }
}
