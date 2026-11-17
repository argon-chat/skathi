//! Native checkboxes and radio buttons: checkedness and radio groups held in the tree, the click's
//! activation steps the runtime brackets its dispatch with, the `checked`, `defaultChecked` and
//! `indeterminate` accessors, arrow keys inside a radio group, and the default skin.
use std::ffi::c_void;

use gpui::prelude::*;
use gpui::{div, px, relative, AnyElement, App, Hsla, KeyDownEvent, Window};
use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

use crate::native_dom::{accessor, node_id, proto};
use crate::tree::{self, Kind, Tree};
use crate::windows;

const CHECKED: &str = ":checked";
const INDETERMINATE: &str = ":indeterminate";
const DIRTY: &str = ":checked-dirty";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    Checkbox,
    Radio,
}

pub(crate) fn kind(t: &Tree, id: u32) -> Option<Toggle> {
    if t.node(id).is_none_or(|n| t.name(n.tag) != "input") {
        return None;
    }
    let ty = t.prop(id, "type")?.trim();
    if ty.eq_ignore_ascii_case("checkbox") {
        Some(Toggle::Checkbox)
    } else if ty.eq_ignore_ascii_case("radio") {
        Some(Toggle::Radio)
    } else {
        None
    }
}

fn is_input(t: &Tree, id: u32) -> bool {
    t.node(id).is_some_and(|n| t.name(n.tag) == "input")
}

fn flag(t: &Tree, id: u32, name: &str) -> bool {
    t.prop(id, name).is_some()
}

fn set_flag(t: &mut Tree, id: u32, name: &str, on: bool) {
    if flag(t, id, name) != on {
        let key = t.intern(name);
        t.set_prop(id, key, on.then(String::new));
    }
}

pub(crate) fn checked(t: &Tree, id: u32) -> bool {
    flag(t, id, CHECKED)
}

pub(crate) fn indeterminate(t: &Tree, id: u32) -> bool {
    flag(t, id, INDETERMINATE)
}

/// The other radios of `id`'s group: the same non-empty `name`, the same form owner, the same tree.
fn group(t: &Tree, id: u32) -> Vec<u32> {
    let Some(name) = t.prop(id, "name").filter(|n| !n.is_empty()) else {
        return Vec::new();
    };
    let mut scope = id;
    let mut cur = t.node(id).map_or(0, |n| n.parent);
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        scope = cur;
        if t.name(n.tag) == "form" {
            break;
        }
        cur = n.parent;
    }
    let mut out = Vec::new();
    let mut stack: Vec<u32> = t
        .node(scope)
        .map(|n| n.children.iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(c) = stack.pop() {
        let Some(n) = t.node(c) else { continue };
        if n.kind != Kind::Element {
            stack.extend(n.children.iter().rev());
            continue;
        }
        match t.name(n.tag) {
            "form" => continue,
            "input" => {
                if c == id || (kind(t, c) == Some(Toggle::Radio) && t.prop(c, "name") == Some(name))
                {
                    out.push(c);
                }
            }
            _ => stack.extend(n.children.iter().rev()),
        }
    }
    out
}

fn set_checked(t: &mut Tree, id: u32, on: bool) {
    set_flag(t, id, CHECKED, on);
    if on && kind(t, id) == Some(Toggle::Radio) {
        for other in group(t, id) {
            if other != id {
                set_flag(t, other, CHECKED, false);
            }
        }
    }
}

/// The `checked` attribute changed: checkedness follows it until a script or the user sets it.
pub(crate) fn attribute_changed(t: &mut Tree, id: u32) {
    if is_input(t, id) && !flag(t, id, DIRTY) {
        let on = t.prop(id, "checked").is_some_and(|v| v != "false");
        set_checked(t, id, on);
    }
}

const NONE: f64 = -1.0;
const RADIO_ALREADY: f64 = -2.0;

/// HTML's legacy-pre-activation behaviour, before a click's listeners run: toggles the control and
/// answers what [`end`] restores if the click is cancelled, or [`NONE`].
fn begin(t: &mut Tree, id: u32) -> f64 {
    let Some(k) = kind(t, id) else { return NONE };
    if crate::render::actually_disabled(t, id) {
        return NONE;
    }
    set_flag(t, id, DIRTY, true);
    match k {
        Toggle::Checkbox => {
            let was = checked(t, id);
            let token = f64::from(u8::from(was) | (u8::from(indeterminate(t, id)) << 1));
            set_flag(t, id, INDETERMINATE, false);
            set_checked(t, id, !was);
            token
        }
        Toggle::Radio if checked(t, id) => RADIO_ALREADY,
        Toggle::Radio => {
            let previous = group(t, id)
                .into_iter()
                .find(|&r| r != id && checked(t, r))
                .unwrap_or(0);
            set_checked(t, id, true);
            f64::from(previous)
        }
    }
}

/// After the listeners: puts a cancelled click's state back. Answers whether `input` and `change`
/// fire.
fn end(t: &mut Tree, id: u32, token: f64, cancelled: bool) -> bool {
    if token == NONE || token == RADIO_ALREADY {
        return false;
    }
    let Some(k) = kind(t, id) else { return false };
    if !cancelled {
        return true;
    }
    match k {
        Toggle::Checkbox => {
            let token = token as u8;
            set_checked(t, id, token & 1 != 0);
            set_flag(t, id, INDETERMINATE, token & 2 != 0);
        }
        Toggle::Radio => {
            let previous = token as u32;
            if previous != 0 && group(t, id).contains(&previous) {
                set_checked(t, previous, true);
            } else {
                set_flag(t, id, CHECKED, false);
            }
        }
    }
    false
}

host_fn!(fn toggle_begin(_g, args) {
    let id = args.arg(0).as_number().unwrap_or(0.0) as u32;
    enc_f64(tree::with_tree(|t| begin(t, id)))
});

host_fn!(fn toggle_end(_g, args) {
    let id = args.arg(0).as_number().unwrap_or(0.0) as u32;
    let token = args.arg(1).as_number().unwrap_or(NONE);
    let cancelled = args.arg(2).as_bool().unwrap_or(false);
    enc_bool(tree::with_tree(|t| end(t, id, token, cancelled)))
});

/// The input `this` mirrors, or `None` for any other receiver.
unsafe fn input_of(g: *mut c_void, this: u64) -> Option<u32> {
    let id = node_id(g, this)?;
    tree::read_tree(|t| is_input(t, id)).then_some(id)
}

/// On anything but an input the property is a plain expando, as in a browser.
unsafe fn expando(g: *mut c_void, this: u64, name: &str, v: u64) -> u64 {
    if is_cell(this) {
        define(g, this, name, v, 0);
    }
    UNDEF
}

host_fn!(fn checked_get(g, args) {
    match unsafe { input_of(g, args.this().0) } {
        Some(id) => enc_bool(tree::read_tree(|t| checked(t, id))),
        None => UNDEF,
    }
});

host_fn!(fn checked_set(g, args) {
    unsafe {
        let (this, v) = (args.this().0, args.arg(0).0);
        let Some(id) = input_of(g, this) else { return expando(g, this, "checked", v) };
        let on = to_bool(g, v);
        tree::with_tree(|t| {
            set_flag(t, id, DIRTY, true);
            set_checked(t, id, on);
        });
    }
    UNDEF
});

host_fn!(fn indeterminate_get(g, args) {
    match unsafe { input_of(g, args.this().0) } {
        Some(id) => enc_bool(tree::read_tree(|t| indeterminate(t, id))),
        None => UNDEF,
    }
});

host_fn!(fn indeterminate_set(g, args) {
    unsafe {
        let (this, v) = (args.this().0, args.arg(0).0);
        let Some(id) = input_of(g, this) else { return expando(g, this, "indeterminate", v) };
        let on = to_bool(g, v);
        tree::with_tree(|t| set_flag(t, id, INDETERMINATE, on));
    }
    UNDEF
});

host_fn!(fn default_checked_get(g, args) {
    match unsafe { input_of(g, args.this().0) } {
        Some(id) => enc_bool(tree::read_tree(|t| t.prop(id, "checked").is_some_and(|v| v != "false"))),
        None => UNDEF,
    }
});

host_fn!(fn default_checked_set(g, args) {
    unsafe {
        let (this, v) = (args.this().0, args.arg(0).0);
        if input_of(g, this).is_none() {
            return expando(g, this, "defaultChecked", v);
        }
        // Through the element's own attribute methods, so the mirror and mutation observers see it.
        let call_on = |method: &str, args: &[u64]| {
            let f = get(g, this, method);
            if is_cell(f) {
                let _ = call(g, f, this, args);
            }
        };
        let name = js_string(g, "checked");
        if to_bool(g, v) {
            call_on("setAttribute", &[name, js_string(g, "")]);
        } else {
            call_on("removeAttribute", &[name]);
        }
    }
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__sk_toggle_begin", 1, toggle_begin as *const c_void);
    rt.register("__sk_toggle_end", 3, toggle_end as *const c_void);
    unsafe {
        let g = rt.raw();
        let el = proto(g, "HTMLElement");
        if !is_cell(el) {
            return;
        }
        accessor(g, el, "checked", checked_get, Some(checked_set));
        accessor(
            g,
            el,
            "indeterminate",
            indeterminate_get,
            Some(indeterminate_set),
        );
        accessor(
            g,
            el,
            "defaultChecked",
            default_checked_get,
            Some(default_checked_set),
        );
    }
}

/// Arrow keys inside a radio group move the focus to the next enabled radio and click it, as
/// Chromium does: the click checks it and fires `input` and `change`.
pub(crate) fn bind_root<E: InteractiveElement>(e: E) -> E {
    e.on_key_down(|ev: &KeyDownEvent, window, cx| {
        if windows::here(window, |w| w.key_prevented) {
            return;
        }
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.shift {
            return;
        }
        let forward = match ev.keystroke.key.as_str() {
            "right" | "down" => true,
            "left" | "up" => false,
            _ => return,
        };
        let id = windows::focused_node(windows::key(window));
        let Some(next) = tree::read_tree(|t| next_radio(t, id, forward)) else {
            return;
        };
        window.prevent_default();
        step_to(next, window, cx);
    })
}

fn next_radio(t: &Tree, id: u32, forward: bool) -> Option<u32> {
    if kind(t, id) != Some(Toggle::Radio) {
        return None;
    }
    let members: Vec<u32> = group(t, id)
        .into_iter()
        .filter(|&r| r == id || !crate::render::actually_disabled(t, r))
        .collect();
    let at = members.iter().position(|&r| r == id)?;
    let n = members.len();
    let next = members[if forward {
        (at + 1) % n
    } else {
        (at + n - 1) % n
    }];
    (next != id).then_some(next)
}

fn step_to(id: u32, window: &mut Window, cx: &mut App) {
    let _ = crate::render::focus_node(id, window, cx);
    crate::render::sync_focus_events(window, cx);
    crate::render::js_dispatch_app(
        id,
        tree::EV_CLICK,
        &crate::EventPayload::default(),
        window,
        cx,
    );
    window.refresh();
}

/// The nearest `accent-color` up the tree (it inherits); `auto` or none is Chromium's blue, lighter
/// under a dark `color-scheme`.
fn accent(t: &Tree, id: u32, dark: bool) -> Hsla {
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        if let Some(set) = n.computed.as_ref().and_then(|cs| cs.extras.accent_color) {
            if let Some(c) = set {
                return c;
            }
            break;
        }
        cur = n.parent;
    }
    gpui::rgb(if dark { 0x99c8ff } else { 0x0075ff }).into()
}

/// The UA defaults Chromium gives the box: 13px square, its margins, only where the author set none.
pub(crate) fn defaults(style: &mut gpui::StyleRefinement, k: Toggle, skin: bool) {
    if skin {
        style.size.width.get_or_insert(px(13.).into());
        style.size.height.get_or_insert(px(13.).into());
    }
    let (top, right, bottom, left) = match k {
        Toggle::Checkbox => (3., 3., 3., 4.),
        Toggle::Radio => (3., 3., 0., 5.),
    };
    style.margin.top.get_or_insert(px(top).into());
    style.margin.right.get_or_insert(px(right).into());
    style.margin.bottom.get_or_insert(px(bottom).into());
    style.margin.left.get_or_insert(px(left).into());
}

pub(crate) fn skin(t: &Tree, id: u32, k: Toggle, window: &Window) -> AnyElement {
    let on = checked(t, id);
    let mixed = k == Toggle::Checkbox && indeterminate(t, id);
    let disabled = crate::render::actually_disabled(t, id);
    let pressed = windows::here(window, |w| w.active == Some(id));
    let hovered = tree::is_hovered(id) && !disabled;
    let dark = crate::scheme::declared(t, id, window) == Some(true);
    let accent = accent(t, id, dark);
    let grey = |v: u32| -> Hsla { gpui::rgb(v).into() };
    // Chromium's palettes; the dark one is what `color-scheme: dark` gives a checkbox.
    let (face, face_pressed, rim, rim_hovered) = if dark {
        (0x3b3b3b, 0x545454, 0x858585, 0xacacac)
    } else {
        (0xffffff, 0xe5e5e5, 0x767676, 0x4f4f4f)
    };
    let filled = (on || mixed) && k == Toggle::Checkbox;
    let (mut fill, mut edge) = if filled {
        (accent, accent)
    } else if on {
        (grey(face), accent)
    } else {
        (
            grey(if pressed { face_pressed } else { face }),
            grey(if hovered { rim_hovered } else { rim }),
        )
    };
    let mut mark = grey(face);
    if disabled {
        fill = if filled {
            grey(0x767676).opacity(0.3)
        } else {
            fill.opacity(0.3)
        };
        edge = edge.opacity(0.3);
        if k == Toggle::Radio {
            mark = grey(0x767676).opacity(0.3);
        }
    } else if hovered && filled {
        fill = fill.blend(gpui::black().opacity(0.15));
        edge = fill;
    }
    let mut frame = div()
        .absolute()
        .size_full()
        .border_1()
        .border_color(edge)
        .bg(fill);
    frame = match k {
        Toggle::Checkbox => frame.rounded(px(2.)),
        Toggle::Radio => frame.rounded_full(),
    };
    let inner: Option<AnyElement> = match k {
        Toggle::Radio if on => Some(
            div()
                .absolute()
                .top(relative(0.25))
                .left(relative(0.25))
                .size(relative(0.5))
                .rounded_full()
                .bg(if disabled { mark } else { edge })
                .into_any_element(),
        ),
        Toggle::Checkbox if mixed => Some(
            div()
                .absolute()
                .left(relative(0.2))
                .right(relative(0.2))
                .top(relative(0.5))
                .mt(px(-1.))
                .h(px(2.))
                .bg(mark)
                .into_any_element(),
        ),
        Toggle::Checkbox if on => Some(check_mark(mark)),
        _ => None,
    };
    let mut wrap = div().absolute().size_full().child(frame);
    if let Some(inner) = inner {
        wrap = wrap.child(inner);
    }
    wrap.into_any_element()
}

fn check_mark(color: Hsla) -> AnyElement {
    let mut c = gpui::canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let at = |fx: f32, fy: f32| gpui::point(px(x + fx * w), px(y + fy * h));
            let mut b = gpui::PathBuilder::stroke(px((w / 7.).clamp(1.5, 3.)));
            b.move_to(at(0.22, 0.52));
            b.line_to(at(0.42, 0.72));
            b.line_to(at(0.78, 0.3));
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        },
    );
    let style = c.style();
    style.position = Some(gpui::Position::Absolute);
    let zero = || Some(gpui::Length::Definite(px(0.).into()));
    style.inset.top = zero();
    style.inset.right = zero();
    style.inset.bottom = zero();
    style.inset.left = zero();
    c.into_any_element()
}

/// `aria-checked` for a native checkbox or radio.
pub(crate) fn aria_state(t: &Tree, id: u32, k: Toggle) -> gpui::Toggled {
    if k == Toggle::Checkbox && indeterminate(t, id) {
        gpui::Toggled::Mixed
    } else if checked(t, id) {
        gpui::Toggled::True
    } else {
        gpui::Toggled::False
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(t: &mut Tree, parent: u32, ty: &str, name: &str) -> u32 {
        let tag = t.intern("input");
        let id = t.create_element(tag);
        let (ty_key, name_key) = (t.intern("type"), t.intern("name"));
        t.set_prop(id, ty_key, Some(ty.into()));
        if !name.is_empty() {
            t.set_prop(id, name_key, Some(name.into()));
        }
        t.insert(id, parent, 0);
        id
    }

    fn element(t: &mut Tree, parent: u32, tag: &str) -> u32 {
        let tag = t.intern(tag);
        let id = t.create_element(tag);
        t.insert(id, parent, 0);
        id
    }

    #[test]
    fn a_click_toggles_a_checkbox_and_a_cancelled_one_puts_it_back() {
        tree::with_tree(|t| {
            let root = t.create_root();
            let cb = input(t, root, "checkbox", "");
            set_flag(t, cb, INDETERMINATE, true);
            let token = begin(t, cb);
            assert!(checked(t, cb) && !indeterminate(t, cb));
            assert!(!end(t, cb, token, true));
            assert!(!checked(t, cb) && indeterminate(t, cb));
            let token = begin(t, cb);
            assert!(end(t, cb, token, false));
            assert!(checked(t, cb));
            let key = t.intern("disabled");
            t.set_prop(cb, key, Some(String::new()));
            assert_eq!(begin(t, cb), NONE);
            assert!(checked(t, cb));
        });
    }

    #[test]
    fn radios_group_by_name_within_their_form_and_a_cancelled_click_restores_the_old_pick() {
        tree::with_tree(|t| {
            let root = t.create_root();
            let a = input(t, root, "radio", "size");
            let b = input(t, root, "radio", "size");
            let other = input(t, root, "radio", "color");
            let form = element(t, root, "form");
            let inside = input(t, form, "radio", "size");
            set_checked(t, a, true);
            set_checked(t, other, true);
            set_checked(t, inside, true);
            assert!(checked(t, a) && checked(t, other) && checked(t, inside));

            let token = begin(t, b);
            assert!(checked(t, b) && !checked(t, a) && checked(t, other) && checked(t, inside));
            assert!(!end(t, b, token, true));
            assert!(checked(t, a) && !checked(t, b));

            let token = begin(t, b);
            assert!(end(t, b, token, false));
            assert!(checked(t, b) && !checked(t, a));
            let again = begin(t, b);
            assert!(
                !end(t, b, again, false),
                "a checked radio fires no input or change"
            );
            assert!(checked(t, b));
            assert_eq!(next_radio(t, b, true), Some(a));
        });
    }

    #[test]
    fn the_checked_attribute_sets_checkedness_until_it_is_dirty() {
        tree::with_tree(|t| {
            let root = t.create_root();
            let cb = input(t, root, "checkbox", "");
            let key = t.intern("checked");
            t.set_prop(cb, key, Some(String::new()));
            attribute_changed(t, cb);
            assert!(checked(t, cb));
            t.set_prop(cb, key, None);
            attribute_changed(t, cb);
            assert!(!checked(t, cb));
            let token = begin(t, cb);
            end(t, cb, token, false);
            t.set_prop(cb, key, None);
            attribute_changed(t, cb);
            assert!(checked(t, cb));
        });
    }
}
