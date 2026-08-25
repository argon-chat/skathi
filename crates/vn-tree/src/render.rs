//! Lowers the retained tree into gpui elements. Runs inside the hosting
//! view's `render()` each frame — gpui's element tree is rebuilt per frame by
//! design, so we simply re-emit the whole thing; per-component memoization
//! (`Entity::cached`) is a later optimization.
//!
//! Input events: gpui listeners are attached ONLY for events the JS side
//! registered (the listener bitmask — empty handlers cost a hitbox each).
//! Each listener publishes `Phase::Commit`, dispatches one compact payload
//! into JS (the runtime synthesizes the W3C event and runs capture/bubble
//! propagation over the mirror tree), then flushes tree changes so Vue's
//! reactive wave repaints in the same frame.
//!
//! Three things need state that outlives the per-frame rebuild, kept in side
//! maps keyed by node id and GC'd in [`gc_node_state`]:
//! * focus handles — a fresh one each frame would blur the node every frame,
//!   and `tabindex` is stamped onto the handle (gpui reads the tab order from
//!   there at paint time);
//! * `gpui::ScrollHandle`s — `overflow: scroll` in the computed style is what
//!   makes gpui lay out and clip a scroll box, but a handle we own is what lets
//!   us read the offset back for `e.target.scrollTop`;
//! * `Entity<TextInputState>` per `<input>`.
//!
//! `focus`/`blur` are not synthesized from mouse-down: they come from gpui
//! focus observers, so `Tab` and programmatic focus report identically.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, ClickEvent, Context, Empty, FocusHandle, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseUpEvent, Overflow, Point, ScrollHandle, ScrollWheelEvent,
    SharedString, Subscription, Window,
};

use crate::tree::{self, Kind, Tree};
use crate::EventPayload;
use vn_elements::{text_input, TextInputHandlers, TextInputOptions, TextInputState};
use vn_runtime::{with_published, Phase};

thread_local! {
    /// Persistent per-node focus handles (minting a new handle each render
    /// would blur the node every frame). GC'd against tree liveness.
    static FOCUS_HANDLES: RefCell<HashMap<u32, FocusHandle>> = RefCell::new(HashMap::new());
    /// The node that owns keyboard focus (0 = none) — drives focus/blur events.
    static FOCUSED_NODE: Cell<u32> = const { Cell::new(0) };
    /// gpui `on_focus` subscriptions, one per focusable node. Focus can move by
    /// mouse, by `Tab`, or programmatically; observing the handle catches all
    /// three with one code path.
    static FOCUS_SUBS: RefCell<HashMap<u32, Subscription>> = RefCell::new(HashMap::new());
    /// The single "nothing in the window is focused any more" observer.
    static FOCUS_LOST_SUB: RefCell<Option<Subscription>> = const { RefCell::new(None) };
    /// Persistent editor state per `<input>` node (content, selection, IME,
    /// undo, echo ring live across the per-frame element rebuild).
    static INPUT_STATES: RefCell<HashMap<u32, gpui::Entity<TextInputState>>> =
        RefCell::new(HashMap::new());
    /// Persistent scroll offset per scroll container. gpui keys anonymous
    /// scroll state by `ElementId` too, but a handle we own is what lets us
    /// *read* the offset back out for `e.target.scrollTop`.
    static SCROLL_HANDLES: RefCell<HashMap<u32, ScrollHandle>> = RefCell::new(HashMap::new());
    /// Last offset reported to JS per node, in CSS pixels — a `scroll` event
    /// only fires when the position actually moved.
    static SCROLL_REPORTED: RefCell<HashMap<u32, (f32, f32)>> = RefCell::new(HashMap::new());
    /// One-shot compat advisories already emitted (keyed by message).
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Browser-compat advisory: say what was dropped and what to do instead, once.
fn warn_compat(key: &str, msg: &str) {
    WARNED.with(|w| {
        if w.borrow_mut().insert(key.to_string()) {
            eprintln!("[vue-native compat] {msg}");
        }
    });
}

/// Drops per-node state for nodes that no longer exist. Called from
/// `flush_changes` after mutations.
pub(crate) fn gc_node_state() {
    let live = |id: u32| tree::read_tree(|t| t.node(id).is_some());
    FOCUS_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    FOCUS_SUBS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    INPUT_STATES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_HANDLES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    SCROLL_REPORTED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    let focused = FOCUSED_NODE.with(|f| f.get());
    if focused != 0 && !live(focused) {
        FOCUSED_NODE.with(|f| f.set(0));
    }
}

fn mods_bits(m: &gpui::Modifiers) -> f64 {
    let mut bits = 0u32;
    if m.control {
        bits |= 1;
    }
    if m.shift {
        bits |= 2;
    }
    if m.alt {
        bits |= 4;
    }
    if m.platform {
        bits |= 8;
    }
    bits as f64
}

fn button_code(b: MouseButton) -> f64 {
    match b {
        MouseButton::Left => 0.0,
        MouseButton::Middle => 1.0,
        MouseButton::Right => 2.0,
        MouseButton::Navigate(_) => 3.0,
    }
}

/// gpui keystroke names → DOM `KeyboardEvent.key` values.
fn dom_key(k: &str) -> String {
    match k {
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "tab" => "Tab".into(),
        "space" => " ".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "insert" => "Insert".into(),
        other => other.into(),
    }
}

fn mouse_payload(position: Point<gpui::Pixels>, button: f64, m: &gpui::Modifiers) -> EventPayload<'static> {
    EventPayload {
        a: f64::from(position.x),
        b: f64::from(position.y),
        c: button,
        d: mods_bits(m),
        key: None,
    }
}

/// Publish → dispatch → flush; the shape every listener shares. Returns
/// `false` when a JS handler called `preventDefault()`.
fn js_dispatch_app(id: u32, event: u32, payload: &EventPayload, window: &mut Window, cx: &mut App) -> bool {
    let proceed = with_published(window, cx, Phase::Commit, || {
        crate::dispatch_event(id, event, payload)
    });
    if crate::flush_changes() {
        window.refresh();
    }
    proceed
}

/// The `Context<V>` flavour: `cx.notify()` is the cheaper repaint request when
/// we already hold the hosting view.
fn js_dispatch<V: 'static>(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut Context<V>,
) -> bool {
    let proceed = with_published(window, &mut **cx, Phase::Commit, || {
        crate::dispatch_event(id, event, payload)
    });
    if crate::flush_changes() {
        cx.notify();
    }
    proceed
}

/// DOM order for a focus move: `blur` on the node that had it, then `focus` on
/// the node that got it. Neither event bubbles (the JS runtime knows).
fn focus_transition<V: 'static>(new_focus: u32, window: &mut Window, cx: &mut Context<V>) {
    let old = FOCUSED_NODE.with(|f| f.get());
    if old == new_focus {
        return;
    }
    FOCUSED_NODE.with(|f| f.set(new_focus));
    if old != 0 {
        js_dispatch(old, tree::EV_BLUR, &EventPayload::default(), window, cx);
    }
    if new_focus != 0 {
        js_dispatch(new_focus, tree::EV_FOCUS, &EventPayload::default(), window, cx);
    }
}

/// Subscribes `id` to its handle's focus changes, once. gpui fires these from
/// the draw that first shows the new focus state, i.e. outside any tree borrow.
fn watch_focus<V: 'static>(id: u32, handle: &FocusHandle, window: &mut Window, cx: &mut Context<V>) {
    if FOCUS_SUBS.with(|m| m.borrow().contains_key(&id)) {
        return;
    }
    let sub = cx.on_focus(handle, window, move |_, window, cx| {
        focus_transition(id, window, cx);
    });
    FOCUS_SUBS.with(|m| m.borrow_mut().insert(id, sub));
    // Focus leaving the window's tree entirely (the focused node was removed,
    // or something unfocusable was clicked) has to blur the last node.
    if FOCUS_LOST_SUB.with(|s| s.borrow().is_some()) {
        return;
    }
    let sub = cx.on_focus_lost(window, |_, window, cx| {
        focus_transition(0, window, cx);
    });
    FOCUS_LOST_SUB.with(|s| *s.borrow_mut() = Some(sub));
}

/// The DOM `tabindex` attribute: `>= 0` is a tab stop at that index, `< 0` is
/// focusable but skipped by `Tab`, absent is not focusable at all. gpui reads
/// both values off the focus handle, so they are stamped onto the persistent
/// one (same `FocusId` — focus is never disturbed by this).
fn tab_index_of(t: &Tree, id: u32) -> Option<isize> {
    let raw = t.prop(id, "tabindex")?;
    match raw.trim().parse::<isize>() {
        Ok(n) => Some(n),
        Err(_) => {
            warn_compat(
                "tabindex",
                &format!("tabindex=\"{raw}\" is not an integer; the attribute was ignored"),
            );
            None
        }
    }
}

fn focus_handle_for<V: 'static>(id: u32, tab_index: Option<isize>, cx: &mut Context<V>) -> FocusHandle {
    FOCUS_HANDLES.with(|m| {
        let mut map = m.borrow_mut();
        let handle = map.entry(id).or_insert_with(|| cx.focus_handle());
        let want_index = tab_index.unwrap_or(0);
        let want_stop = tab_index.is_some_and(|n| n >= 0);
        if handle.tab_index != want_index || handle.tab_stop != want_stop {
            *handle = handle.clone().tab_index(want_index).tab_stop(want_stop);
        }
        handle.clone()
    })
}

/// Builds the current tree root (or nothing) for the hosting view `V`.
pub fn build_root<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    // ONE clock read per frame: nodes animating together must sample the same
    // instant or they skew.
    let now = std::time::Instant::now();
    tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            build_node(t, t.root, window, cx, now)
        }
    })
}

fn build_node<V: 'static>(
    t: &Tree,
    id: u32,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
) -> AnyElement {
    let Some(node) = t.node(id) else {
        return Empty.into_any_element();
    };
    match node.kind {
        // An empty text node takes no space in a browser, but gpui lays a text
        // element out as a full line box — and Vue anchors every Fragment
        // (`v-for`, multi-root components) with `createText("")`, so leaving it
        // in adds a blank line per fragment.
        Kind::Text if node.text.is_empty() => Empty.into_any_element(),
        Kind::Text => SharedString::from(node.text.clone()).into_any_element(),
        Kind::Anchor => Empty.into_any_element(),
        Kind::Element => {
            // Every mirrored node gets a stable ElementId — hover/active/
            // scroll state in gpui is keyed by it.
            let mut d = div().id(id as usize);
            let mut scrolls = false;
            if let Some(cs) = &node.computed {
                // Base wholesale; state refinements go into gpui's own
                // interactivity ladder (the spike-proven installation trick —
                // the FnOnce builders run eagerly, only the result is stored).
                *d.style() = (*cs.base).clone();
                // Running CSS transitions: interpolated values refined over
                // the base, before the hover/active ladder installs.
                if let Some(overlay) = tree::animation_overlay(id, now) {
                    use gpui::Refineable as _;
                    d.style().refine(&overlay);
                }
                if let Some(h) = cs.hover.clone() {
                    d = d.hover(move |_| (*h).clone());
                }
                if let Some(a) = cs.active.clone() {
                    d = d.active(move |_| (*a).clone());
                }
                // `overflow: scroll|auto` on either axis. The style itself is
                // all gpui's layout/clipping needs (it came in through the
                // wholesale assignment above, so calling `.overflow_y_scroll()`
                // here would only re-set what is already set); what makes the
                // box actually *scroll* is a scroll offset to paint through —
                // `track_scroll` supplies one that outlives the frame, and its
                // presence is also what makes gpui insert the hitbox and the
                // wheel listener that move it.
                scrolls = cs.base.overflow.x == Some(Overflow::Scroll)
                    || cs.base.overflow.y == Some(Overflow::Scroll);
            }
            let scroll_handle = scrolls.then(|| {
                SCROLL_HANDLES.with(|m| m.borrow_mut().entry(id).or_default().clone())
            });
            if let Some(h) = &scroll_handle {
                d = d.track_scroll(h);
            }

            let mask = node.listeners;
            let bit = move |ev: u32| mask & (1 << ev) != 0;
            let is_input = t.name(node.tag) == "input";
            // `<input>` is tabbable by default in HTML; a plain <div> is not.
            let tab_index = tab_index_of(t, id).or(is_input.then_some(0));

            // Keyboard/focus events and tab navigation need a persistent
            // FocusHandle. `<input>` owns its own focus handle inside the
            // native widget — it is decorated through TextInputOptions instead.
            let focusable = !is_input
                && (tab_index.is_some()
                    || bit(tree::EV_KEYDOWN)
                    || bit(tree::EV_KEYUP)
                    || bit(tree::EV_FOCUS)
                    || bit(tree::EV_BLUR));
            let handle = focusable.then(|| focus_handle_for(id, tab_index, cx));
            if let Some(h) = &handle {
                d = d.track_focus(h);
                watch_focus(id, h, window, cx);
            }

            if bit(tree::EV_CLICK) {
                d = d.on_click(cx.listener(move |_, ev: &ClickEvent, window, cx| {
                    let payload = mouse_payload(ev.position(), 0.0, &ev.modifiers());
                    js_dispatch(id, tree::EV_CLICK, &payload, window, cx);
                }));
            }
            if bit(tree::EV_MOUSEDOWN) || focusable {
                let handle = handle.clone();
                d = d.on_any_mouse_down(cx.listener(move |_, ev: &MouseDownEvent, window, cx| {
                    if let Some(h) = &handle {
                        // Focus follows the press, as in a browser; the
                        // focus/blur events come out of the subscription.
                        window.focus(h, cx);
                    }
                    if bit(tree::EV_MOUSEDOWN) {
                        let payload = mouse_payload(ev.position, button_code(ev.button), &ev.modifiers);
                        js_dispatch(id, tree::EV_MOUSEDOWN, &payload, window, cx);
                    }
                }));
            }
            if bit(tree::EV_MOUSEUP) {
                // gpui's API is per-button here (no any-button variant on the
                // element traits) — attach one listener per button.
                for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
                    d = d.on_mouse_up(
                        button,
                        cx.listener(move |_, ev: &MouseUpEvent, window, cx| {
                            let payload =
                                mouse_payload(ev.position, button_code(ev.button), &ev.modifiers);
                            js_dispatch(id, tree::EV_MOUSEUP, &payload, window, cx);
                        }),
                    );
                }
            }
            if bit(tree::EV_WHEEL) {
                d = d.on_scroll_wheel(cx.listener(move |_, ev: &ScrollWheelEvent, window, cx| {
                    let delta = ev.delta.pixel_delta(gpui::px(16.0));
                    let payload = EventPayload {
                        a: f64::from(delta.x),
                        b: f64::from(delta.y),
                        c: 0.0,
                        d: mods_bits(&ev.modifiers),
                        key: None,
                    };
                    js_dispatch(id, tree::EV_WHEEL, &payload, window, cx);
                }));
            }
            // `<input>` forwards its own keystrokes from inside the widget (it
            // has to run before gpui resolves the editing key bindings), so the
            // wrapper must not also listen — the editor is its descendant and
            // would deliver every key twice.
            if bit(tree::EV_KEYDOWN) && !is_input {
                d = d.on_key_down(cx.listener(move |_, ev: &KeyDownEvent, window, cx| {
                    let key = dom_key(&ev.keystroke.key);
                    let payload = EventPayload {
                        d: mods_bits(&ev.keystroke.modifiers),
                        key: Some(&key),
                        ..Default::default()
                    };
                    js_dispatch(id, tree::EV_KEYDOWN, &payload, window, cx);
                }));
            }
            if bit(tree::EV_KEYUP) && !is_input {
                d = d.on_key_up(cx.listener(move |_, ev: &KeyUpEvent, window, cx| {
                    let key = dom_key(&ev.keystroke.key);
                    let payload = EventPayload {
                        d: mods_bits(&ev.keystroke.modifiers),
                        key: Some(&key),
                        ..Default::default()
                    };
                    js_dispatch(id, tree::EV_KEYUP, &payload, window, cx);
                }));
            }
            // `scroll` reports the position *after* the move. gpui installs its
            // own wheel listener that advances the offset during
            // `Interactivity::paint`, i.e. after ours; mouse listeners bubble
            // in reverse registration order, so gpui's runs first and the
            // handle already holds the new offset when this one reads it.
            if let Some(scroll) = scroll_handle.clone().filter(|_| bit(tree::EV_SCROLL)) {
                d = d.on_scroll_wheel(cx.listener(move |_, _: &ScrollWheelEvent, window, cx| {
                    // gpui offsets are <= 0 (content shifted up/left); CSS
                    // scrollTop/scrollLeft count down/right as positive.
                    let offset = scroll.offset();
                    let (left, top) = (-f32::from(offset.x), -f32::from(offset.y));
                    let moved = SCROLL_REPORTED.with(|m| {
                        let mut m = m.borrow_mut();
                        if m.get(&id) == Some(&(left, top)) {
                            return false;
                        }
                        m.insert(id, (left, top));
                        true
                    });
                    if moved {
                        let payload = EventPayload {
                            a: left as f64,
                            b: top as f64,
                            ..Default::default()
                        };
                        js_dispatch(id, tree::EV_SCROLL, &payload, window, cx);
                    }
                }));
            }

            if is_input {
                let (input, focus_target) = build_input(t, id, node, tab_index, window, cx);
                // The editor's own handle carries this node's focus identity
                // (mouse, Tab and programmatic focus all land on it), so the
                // focus/blur events hang off it.
                watch_focus(id, &focus_target, window, cx);
                // Clicks on the wrapper's padding/border focus the editor —
                // the HTML behavior.
                d = d.on_any_mouse_down(cx.listener(move |_, _: &MouseDownEvent, window, cx| {
                    window.focus(&focus_target, cx);
                }));
                d.child(input).into_any_element()
            } else if !node.text.is_empty() {
                d.child(node.text.clone()).into_any_element()
            } else {
                d.children(node.children.iter().map(|&c| build_node(t, c, window, cx, now)))
                    .into_any_element()
            }
        }
    }
}

/// The native `<input>` widget: persistent `TextInputState` entity per node,
/// controlled through the DOM-compat prop channel (`value`, `placeholder`,
/// `disabled`, `tabindex`), styled from the node's computed text style,
/// reporting edits as `input` events (with `e.target.value` populated on the JS
/// side) and keystrokes as cancellable `keydown`/`keyup`.
fn build_input<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    tab_index: Option<isize>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (AnyElement, FocusHandle) {
    let state = INPUT_STATES.with(|m| {
        m.borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.new(|cx| TextInputState::new(window, cx)))
            .clone()
    });

    // Controlled value: the write counter, not the string, is what tells the
    // widget an assignment happened (`v-model` re-forcing "" after the user
    // typed again writes the same string the previous clear did).
    if let Some((value, write)) = t.prop_write(id, "value") {
        let value = value.to_string();
        state.update(cx, |s, cx| s.sync_value(&value, write, cx));
    }

    let text_style = node.computed.as_ref().map(|cs| cs.base.text.clone());
    let text_color = text_style
        .as_ref()
        .and_then(|ts| ts.color)
        .unwrap_or_else(gpui::white);
    let font_size = text_style
        .as_ref()
        .and_then(|ts| ts.font_size)
        .map(|al| al.to_pixels(window.rem_size()))
        .unwrap_or(gpui::px(14.0));
    let placeholder_color = gpui::Hsla { a: text_color.a * 0.45, ..text_color };
    let options = TextInputOptions {
        placeholder: t.prop(id, "placeholder").unwrap_or("").to_string().into(),
        text_color,
        placeholder_color,
        selection_color: gpui::Hsla { h: 0.6, s: 0.8, l: 0.6, a: 0.35 },
        caret_color: text_color,
        font_size,
        disabled: t.prop(id, "disabled").is_some_and(|v| v != "false"),
        tab_index,
    };

    let mask = node.listeners;
    let bit = move |ev: u32| mask & (1 << ev) != 0;

    let on_change: vn_elements::OnChange = std::rc::Rc::new(move |text, window, cx| {
        js_dispatch_app(
            id,
            tree::EV_INPUT,
            &EventPayload { key: Some(text), ..Default::default() },
            window,
            cx,
        );
    });

    // Keystrokes are forwarded *before* the widget acts on them, so a JS
    // handler that calls `preventDefault()` cancels the default editing
    // (`dispatch_event` already reports `!defaultPrevented`).
    let on_key_down: Option<vn_elements::OnKeyDown> = bit(tree::EV_KEYDOWN).then(|| {
        std::rc::Rc::new(move |ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App| {
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            js_dispatch_app(id, tree::EV_KEYDOWN, &payload, window, cx)
        }) as vn_elements::OnKeyDown
    });
    let on_key_up: Option<vn_elements::OnKeyUp> = bit(tree::EV_KEYUP).then(|| {
        std::rc::Rc::new(move |ev: &KeyUpEvent, window: &mut Window, cx: &mut App| {
            let key = dom_key(&ev.keystroke.key);
            let payload = EventPayload {
                d: mods_bits(&ev.keystroke.modifiers),
                key: Some(&key),
                ..Default::default()
            };
            js_dispatch_app(id, tree::EV_KEYUP, &payload, window, cx);
        }) as vn_elements::OnKeyUp
    });

    let handlers = TextInputHandlers {
        on_change: Some(on_change),
        on_key_down,
        on_key_up,
    };

    let focus_target = state.read(cx).focus_handle();
    (text_input(&state, options, handlers, cx), focus_target)
}
