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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    div, AnyElement, ClickEvent, Context, Empty, FocusHandle, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseUpEvent, Point, ScrollWheelEvent, SharedString, Window,
};

use crate::tree::{self, Kind, Tree};
use crate::EventPayload;
use vn_elements::{text_input, TextInputOptions, TextInputState};
use vn_runtime::{with_published, Phase};

thread_local! {
    /// Persistent per-node focus handles (minting a new handle each render
    /// would blur the node every frame). GC'd against tree liveness.
    static FOCUS_HANDLES: RefCell<HashMap<u32, FocusHandle>> = RefCell::new(HashMap::new());
    /// The node that owns keyboard focus (0 = none) — drives focus/blur events.
    static FOCUSED_NODE: Cell<u32> = const { Cell::new(0) };
    /// Persistent editor state per `<input>` node (content, selection, IME,
    /// undo, echo ring live across the per-frame element rebuild).
    static INPUT_STATES: RefCell<HashMap<u32, gpui::Entity<TextInputState>>> =
        RefCell::new(HashMap::new());
}

/// Drops per-node state for nodes that no longer exist. Called from
/// `flush_changes` after mutations.
pub(crate) fn gc_node_state() {
    FOCUS_HANDLES.with(|m| {
        m.borrow_mut()
            .retain(|&id, _| tree::read_tree(|t| t.node(id).is_some()));
    });
    INPUT_STATES.with(|m| {
        m.borrow_mut()
            .retain(|&id, _| tree::read_tree(|t| t.node(id).is_some()));
    });
    let focused = FOCUSED_NODE.with(|f| f.get());
    if focused != 0 && tree::read_tree(|t| t.node(focused).is_none()) {
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

/// Publish → dispatch → flush; the shape every listener shares.
fn js_dispatch<V: 'static>(
    id: u32,
    event: u32,
    payload: &EventPayload,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    with_published(window, &mut **cx, Phase::Commit, || {
        crate::dispatch_event(id, event, payload);
    });
    if crate::flush_changes() {
        cx.notify();
    }
}

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

/// Builds the current tree root (or nothing) for the hosting view `V`.
pub fn build_root<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            build_node(t, t.root, window, cx)
        }
    })
}

fn build_node<V: 'static>(t: &Tree, id: u32, window: &mut Window, cx: &mut Context<V>) -> AnyElement {
    let Some(node) = t.node(id) else {
        return Empty.into_any_element();
    };
    match node.kind {
        Kind::Text => SharedString::from(node.text.clone()).into_any_element(),
        Kind::Anchor => Empty.into_any_element(),
        Kind::Element => {
            // Every mirrored node gets a stable ElementId — hover/active/
            // scroll state in gpui is keyed by it.
            let mut d = div().id(id as usize);
            if let Some(cs) = &node.computed {
                // Base wholesale; state refinements go into gpui's own
                // interactivity ladder (the spike-proven installation trick —
                // the FnOnce builders run eagerly, only the result is stored).
                *d.style() = (*cs.base).clone();
                if let Some(h) = cs.hover.clone() {
                    d = d.hover(move |_| (*h).clone());
                }
                if let Some(a) = cs.active.clone() {
                    d = d.active(move |_| (*a).clone());
                }
            }

            let mask = node.listeners;
            let bit = move |ev: u32| mask & (1 << ev) != 0;
            let is_input = t.name(node.tag) == "input";

            // Keyboard/focus events need a persistent FocusHandle; mouse-down
            // moves focus (tab order comes later). `<input>` owns its own
            // focus/keyboard handling inside the native widget.
            let focusable = !is_input
                && (bit(tree::EV_KEYDOWN) || bit(tree::EV_KEYUP) || bit(tree::EV_FOCUS)
                    || bit(tree::EV_BLUR));
            let handle = focusable.then(|| {
                FOCUS_HANDLES.with(|m| {
                    m.borrow_mut()
                        .entry(id)
                        .or_insert_with(|| cx.focus_handle())
                        .clone()
                })
            });
            if let Some(h) = &handle {
                d = d.track_focus(h);
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
                        window.focus(h, cx);
                        focus_transition(id, window, cx);
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
            if bit(tree::EV_KEYDOWN) {
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
            if bit(tree::EV_KEYUP) {
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

            if is_input {
                let (input, focus_target) = build_input(t, id, node, window, cx);
                // Clicks on the wrapper's padding/border focus the editor —
                // the HTML behavior.
                d = d.on_any_mouse_down(cx.listener(move |_, _: &MouseDownEvent, window, cx| {
                    window.focus(&focus_target, cx);
                }));
                d.child(input).into_any_element()
            } else if !node.text.is_empty() {
                d.child(node.text.clone()).into_any_element()
            } else {
                d.children(node.children.iter().map(|&c| build_node(t, c, window, cx)))
                    .into_any_element()
            }
        }
    }
}

/// The native `<input>` widget: persistent `TextInputState` entity per node,
/// controlled through the DOM-compat prop channel (`value`, `placeholder`,
/// `disabled`), styled from the node's computed text style, reporting edits as
/// `input` events (with `e.target.value` populated on the JS side).
fn build_input<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (AnyElement, FocusHandle) {
    let state = INPUT_STATES.with(|m| {
        m.borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.new(|cx| TextInputState::new(window, cx)))
            .clone()
    });

    if let Some(value) = t.prop(id, "value") {
        let value = value.to_string();
        state.update(cx, |s, cx| s.sync_value(&value, cx));
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
    };

    let on_change: vn_elements::OnChange = std::rc::Rc::new(move |text, window, cx| {
        with_published(window, cx, Phase::Commit, || {
            crate::dispatch_event(
                id,
                tree::EV_INPUT,
                &EventPayload { key: Some(text), ..Default::default() },
            );
        });
        if crate::flush_changes() {
            window.refresh();
        }
    });

    let focus_target = state.read(cx).focus_handle();
    (text_input(&state, options, on_change, cx), focus_target)
}
