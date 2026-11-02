//! `<select>`: the closed box Chromium draws on Windows, and the popup list it
//! opens (docs/RUNTIME.md §5.28).
//!
//! **The box** is the element's own div: the author's border, padding and
//! background, around what Chromium's menulist puts inside them. That is the
//! selected option's label on one line, inside Chromium's internal padding
//! (`LayoutThemeDefault`: 4px at the start, 1px above and below, and at the end
//! 1px plus a 15px arrow box), and the arrow: an 8×4 chevron in the text
//! colour, centred in the last 15px of the padding box, where Chromium puts it
//! whatever the author's padding. The line height is `normal`, whatever the
//! author's (Chromium locks it on a menulist). The width is the widest
//! option's: every label is laid out, and all but the shown one take no
//! height. `appearance: none` takes the arrow and the internal padding away
//! and gives the line height back, as in Chromium. There is no UA stylesheet
//! here, so an unstyled select has no border or background of its own.
//!
//! **The selection** is the runtime's (runtime.ts, "`<select>` and
//! `<option>`"). It pushes the selected index as the reserved `:selected`
//! prop, and a pick made here goes back as the select's `input` host event
//! with the option's index in `a`; the runtime applies it and fires `input`
//! and `change`, or nothing for the option already selected.
//!
//! **The popup** lists the options under the box, or above it when there is
//! more room there, at least as wide as the box and at most 20 rows tall,
//! scrolling past that. It is deferred above everything else in the window,
//! and its own boxes carry no node tag, so no element of the page is under a
//! press on it. Its colours are the select's (`color`, and `background-color`
//! when it has an opaque one), else the colour scheme's, and the highlight is
//! approximately Chromium's. While it is open:
//!
//! - a press inside it reaches no element of the page ([`refine`]), and a
//!   release on an enabled option picks it; the pointer moves the highlight;
//! - a press outside closes it and is swallowed with its release, as
//!   Chromium's popup takes the press that dismisses it; a wheel outside closes
//!   it and scrolls as usual;
//! - the keyboard is the popup's, and the page hears no key event: ↑/↓,
//!   Home/End and PageUp/PageDown move the highlight over the enabled options,
//!   typing finds an option by its label, Enter, Space, Tab, Alt+↑/↓ and F4
//!   pick the highlighted one, and Escape closes without picking;
//! - it closes when the select loses focus, is disabled or removed, or the
//!   window is deactivated.
//!
//! **Closed**, a focused select opens on Alt+↓/↑, F4, Space and Enter, and
//! ↑/↓, ←/→, Home/End, PageUp/PageDown and typing change the selection at
//! once, each change with its `input` and `change`, as Chromium on Windows
//! does. All of it is the default action of the page's own event: a cancelled
//! `keydown` does nothing, and a cancelled `mousedown` opens nothing.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{div, px, relative, AnyElement, App, Bounds, Hsla, KeyDownEvent, KeyUpEvent, MouseButton, Pixels, Point,
    ScrollHandle, VnHitKind, Window};

use crate::tree::{self, Kind, Tree};
use crate::windows::{self, WinKey};
use crate::EventPayload;

/// Chromium's menulist padding (`LayoutThemeDefault::PopupInternalPadding*`).
const PAD_START: f32 = 4.0;
const PAD_BLOCK: f32 = 1.0;
/// The arrow box: the scroll bar button's width (15px under Windows 11's
/// Fluent scroll bars), at the end of the padding box.
const ARROW_BOX: f32 = 15.0;
/// 1px, then the arrow box.
const PAD_END: f32 = 1.0 + ARROW_BOX;
/// The chevron (`ThemePainterDefault::SetupMenuListArrow`: 8px wide, half as tall).
const ARROW_W: f32 = 8.0;
/// The popup's rows before it scrolls (Chromium's list picker).
const MAX_ROWS: usize = 20;
/// A pause this long starts a new type-ahead search.
const TYPEAHEAD_RESET: Duration = Duration::from_millis(1000);

// ---------------------------------------------------------------------------
// The options, as the host tree has them
// ---------------------------------------------------------------------------

/// One line of the popup: an `<optgroup>`'s label, or an option.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Row {
    Group { label: String },
    Opt { index: usize, label: String, disabled: bool, hidden: bool, grouped: bool },
}

impl Row {
    /// An option the user can pick: enabled, and shown.
    fn pickable(&self) -> Option<usize> {
        match self {
            Row::Opt { index, disabled: false, hidden: false, .. } => Some(*index),
            _ => None,
        }
    }
}

/// A present boolean attribute ("false" is Vue's spelling of absent).
fn flag(t: &Tree, id: u32, name: &str) -> bool {
    t.prop(id, name).is_some_and(|v| v != "false")
}

fn element_named(t: &Tree, id: u32, tag: &str) -> bool {
    t.node(id).is_some_and(|n| n.kind == Kind::Element && t.name(n.tag) == tag)
}

pub(crate) fn is_select(t: &Tree, id: u32) -> bool {
    element_named(t, id, "select")
}

/// HTML's option label: a non-empty `label` attribute, else the option's text
/// with ASCII white space stripped and collapsed.
pub(crate) fn option_label(t: &Tree, id: u32) -> String {
    if let Some(l) = t.prop(id, "label").filter(|l| !l.is_empty()) {
        return l.to_string();
    }
    let mut raw = String::new();
    collect_text(t, id, &mut raw, 0);
    raw.split([' ', '\t', '\n', '\x0c', '\r']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn collect_text(t: &Tree, id: u32, out: &mut String, depth: usize) {
    let Some(n) = t.node(id) else { return };
    if depth > 64 {
        return;
    }
    match n.kind {
        Kind::Text => out.push_str(&n.text),
        Kind::Element => {
            out.push_str(&n.text);
            for &c in &n.children {
                collect_text(t, c, out, depth + 1);
            }
        }
        Kind::Anchor => {}
    }
}

/// The select's list of options, in HTML's order (its option children, and the
/// option children of its optgroup children), with the optgroups' labels
/// between them. An option's index is its place in that list, which is what
/// `selectedIndex` and the `:selected` prop count.
pub(crate) fn rows(t: &Tree, select: u32) -> Vec<Row> {
    let mut out = Vec::new();
    let Some(n) = t.node(select) else { return out };
    let mut index = 0;
    let mut push = |out: &mut Vec<Row>, id: u32, group_disabled: bool, grouped: bool| {
        out.push(Row::Opt {
            index,
            label: option_label(t, id),
            disabled: group_disabled || flag(t, id, "disabled"),
            hidden: flag(t, id, "hidden"),
            grouped,
        });
        index += 1;
    };
    for &c in &n.children {
        if element_named(t, c, "option") {
            push(&mut out, c, false, false);
        } else if element_named(t, c, "optgroup") {
            out.push(Row::Group { label: t.prop(c, "label").unwrap_or("").to_string() });
            let disabled = flag(t, c, "disabled");
            for &g in &t.node(c).map(|g| g.children.clone()).unwrap_or_default() {
                if element_named(t, g, "option") {
                    push(&mut out, g, disabled, true);
                }
            }
        }
    }
    out
}

/// The index the runtime says is selected: `selectedIndex`, `None` for -1.
pub(crate) fn shown(t: &Tree, select: u32) -> Option<usize> {
    let ix = t.prop(select, ":selected")?.trim().parse::<i64>().ok()?;
    usize::try_from(ix).ok()
}

/// Can `select` open, or change by the keyboard: a select, in the tree, and
/// not actually disabled.
fn usable(t: &Tree, select: u32) -> bool {
    is_select(t, select) && !crate::render::actually_disabled(t, select)
}

// ---------------------------------------------------------------------------
// Moving through the options
// ---------------------------------------------------------------------------

/// The pickable options, in order.
fn choices(rows: &[Row]) -> Vec<usize> {
    rows.iter().filter_map(Row::pickable).collect()
}

/// Where a key moves from `from`: `delta` pickable options on (negative:
/// back), clamped to the first and the last. Nothing selected: the first.
fn step(rows: &[Row], from: Option<usize>, delta: isize) -> Option<usize> {
    let c = choices(rows);
    let (first, last) = (*c.first()?, *c.last()?);
    let Some(from) = from else { return Some(first) };
    let at = if delta > 0 {
        // The last pickable one at or before `from`: a disabled option can be
        // selected, and moving on from it is moving past it.
        match c.iter().rposition(|&i| i <= from) {
            Some(p) => p as isize + delta,
            None => delta - 1,
        }
    } else {
        match c.iter().position(|&i| i >= from) {
            Some(p) => p as isize + delta,
            None => c.len() as isize + delta,
        }
    };
    Some(if at < 0 { first } else { c.get(at as usize).copied().unwrap_or(last) })
}

/// What one key does to the selection or the highlight, if it moves it.
fn moved(rows: &[Row], from: Option<usize>, key: &str) -> Option<Option<usize>> {
    let page = MAX_ROWS as isize - 1;
    Some(match key {
        "down" | "right" => step(rows, from, 1),
        "up" | "left" => step(rows, from, -1),
        "pagedown" => step(rows, from, page),
        "pageup" => step(rows, from, -page),
        "home" => choices(rows).first().copied(),
        "end" => choices(rows).last().copied(),
        _ => return None,
    })
}

/// A type-ahead search in progress (Chromium's `TypeAhead`).
#[derive(Default)]
struct Typeahead {
    text: String,
    at: Option<Instant>,
}

impl Typeahead {
    fn active(&self, now: Instant) -> bool {
        !self.text.is_empty() && self.at.is_some_and(|t| now.duration_since(t) < TYPEAHEAD_RESET)
    }

    /// Types `ch` and finds the option it names, from `from`. One character
    /// repeated cycles through the options starting with it, from the one
    /// after `from`; anything else is a prefix, matched from `from` on. Labels
    /// are compared without case and without their leading white space.
    fn find(&mut self, rows: &[Row], from: Option<usize>, ch: char, now: Instant) -> Option<usize> {
        if !self.active(now) {
            self.text.clear();
        }
        self.at = Some(now);
        self.text.extend(ch.to_lowercase());
        let first = self.text.chars().next()?;
        let cycle = self.text.chars().all(|c| c == first);
        let needle: String = if cycle { first.to_string() } else { self.text.clone() };
        let c = choices(rows);
        if c.is_empty() {
            return None;
        }
        let start = match from {
            Some(f) => c.iter().position(|&i| if cycle { i > f } else { i >= f }).unwrap_or(0),
            None => 0,
        };
        let label = |ix: usize| {
            rows.iter().find_map(|r| match r {
                Row::Opt { index, label, .. } if *index == ix => Some(label.trim_start().to_lowercase()),
                _ => None,
            })
        };
        (0..c.len()).map(|k| c[(start + k) % c.len()]).find(|&ix| label(ix).is_some_and(|l| l.starts_with(&needle)))
    }
}

/// A printable character a key typed, with no Ctrl, Alt or Windows key held.
fn typed(ev: &KeyDownEvent) -> Option<char> {
    let m = &ev.keystroke.modifiers;
    if m.control || m.alt || m.platform {
        return None;
    }
    let s = ev.keystroke.key_char.as_deref()?;
    let mut chars = s.chars();
    let ch = chars.next()?;
    (chars.next().is_none() && !ch.is_control()).then_some(ch)
}

// ---------------------------------------------------------------------------
// The popup's state
// ---------------------------------------------------------------------------

struct Open {
    select: u32,
    window: WinKey,
    /// The option Enter would pick: the selected one at first, then wherever
    /// the keyboard or the pointer moved it.
    highlight: Option<usize>,
    scroll: ScrollHandle,
    /// Scroll the highlighted row into view at the next build.
    reveal: bool,
    /// Where the popup was placed, in window space, at its last prepaint.
    bounds: Option<Bounds<Pixels>>,
    typeahead: Typeahead,
}

thread_local! {
    /// The one open popup; a desktop has one at a time, as it has one pointer.
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    /// The select a press is on, until the press's default action runs
    /// ([`press_settled`]).
    static PRESSED: Cell<Option<(WinKey, u32)>> = const { Cell::new(None) };
    /// The window whose next release is swallowed: its press closed a popup.
    static SWALLOW_UP: Cell<Option<WinKey>> = const { Cell::new(None) };
    /// A closed select's type-ahead.
    static TYPED: RefCell<Typeahead> = RefCell::new(Typeahead::default());
}

/// The select whose popup is open in this window.
fn open_in(k: WinKey) -> Option<u32> {
    OPEN.with(|o| o.borrow().as_ref().filter(|o| o.window == k).map(|o| o.select))
}

/// Is the popup still where it was: its select usable, focused, and painted by
/// this window? Not: it closes, without a repaint (the caller is mid-event, or
/// mid-build).
fn still_open(k: WinKey) -> Option<u32> {
    let select = open_in(k)?;
    let painted = windows::peek(k, |w| w.bounds.contains_key(&select) || w.prev_bounds.contains_key(&select))
        .unwrap_or(false);
    let ok = painted && windows::focused_node(k) == select && tree::read_tree(|t| usable(t, select));
    if ok {
        return Some(select);
    }
    OPEN.with(|o| o.borrow_mut().take());
    None
}

/// The select's subtree builds again, and every window paints.
fn changed(select: u32, window: &mut Window, cx: &mut App) {
    tree::with_tree(|t| t.touch(select));
    crate::render::request_repaint(window, cx);
}

fn open(select: u32, window: &mut Window, cx: &mut App) {
    let Some(highlight) = tree::read_tree(|t| usable(t, select).then(|| shown(t, select))) else { return };
    let previous = OPEN.with(|o| {
        o.borrow_mut().replace(Open {
            select,
            window: windows::key(window),
            highlight,
            scroll: ScrollHandle::new(),
            reveal: true,
            bounds: None,
            typeahead: Typeahead::default(),
        })
    });
    if let Some(p) = previous.filter(|p| p.select != select) {
        tree::with_tree(|t| t.touch(p.select));
    }
    changed(select, window, cx);
}

fn close(window: &mut Window, cx: &mut App) {
    if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
        changed(o.select, window, cx);
    }
}

/// `index` was picked: the popup closes, and the runtime is told.
fn pick(select: u32, index: usize, window: &mut Window, cx: &mut App) {
    close(window, cx);
    commit(select, index, window, cx);
}

/// Selects option `index` of `select` the way the user does: the runtime
/// applies it and fires `input` and `change` (runtime.ts, `pickOption`).
fn commit(select: u32, index: usize, window: &mut Window, cx: &mut App) {
    let payload = EventPayload { a: index as f64, ..Default::default() };
    crate::render::js_dispatch_app(select, tree::EV_INPUT, &payload, window, cx);
}

/// The highlight moves to `index`.
fn highlight(index: Option<usize>, reveal: bool, window: &mut Window, cx: &mut App) {
    let select = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let o = o.as_mut()?;
        // A key that lands on the option already highlighted still brings it
        // back into view, if the wheel scrolled it out.
        if o.highlight == index && !reveal {
            return None;
        }
        o.highlight = index;
        o.reveal |= reveal;
        Some(o.select)
    });
    if let Some(select) = select {
        changed(select, window, cx);
    }
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

/// The select's press listener: the popup opens as the press's default action
/// ([`press_settled`]), after the `mousedown` and the focus it gives.
pub(crate) fn bind_press<E: InteractiveElement>(e: E, id: u32) -> E {
    e.on_mouse_down(MouseButton::Left, move |_, window, _| {
        PRESSED.with(|p| p.set(Some((windows::key(window), id))));
    })
}

/// A press's default action has run (`render::apply_press_focus`): one on a
/// select opens its popup, unless its `mousedown` was cancelled.
pub(crate) fn press_settled(cancelled: bool, window: &mut Window, cx: &mut App) {
    let Some((k, select)) = PRESSED.with(|p| p.take()) else { return };
    if cancelled || k != windows::key(window) {
        return;
    }
    open(select, window, cx);
}

/// The hit refiner's first question (`pointer::refine`): is the event the
/// popup's? `true`: it is, and no element of the page hears it.
pub(crate) fn refine(kind: VnHitKind, p: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
    let k = windows::key(window);
    if kind == VnHitKind::Up && SWALLOW_UP.with(|s| s.get()) == Some(k) {
        SWALLOW_UP.with(|s| s.set(None));
        hit_nothing(kind, window);
        return true;
    }
    if still_open(k).is_none() {
        return false;
    }
    let inside = OPEN.with(|o| o.borrow().as_ref().and_then(|o| o.bounds).is_some_and(|b| b.contains(&p)));
    if inside {
        hit_nothing(kind, window);
        return true;
    }
    match kind {
        VnHitKind::Down => {
            close(window, cx);
            SWALLOW_UP.with(|s| s.set(Some(k)));
            hit_nothing(kind, window);
            true
        }
        VnHitKind::Scroll => {
            close(window, cx);
            false
        }
        // The release of the press that opened it.
        VnHitKind::Up => false,
    }
}

/// The event reaches no element: only the popup's own boxes, which carry no
/// node tag, keep it.
fn hit_nothing(kind: VnHitKind, window: &mut Window) {
    windows::here(window, |w| {
        w.hit = None;
        match kind {
            VnHitKind::Down => {
                w.down_hit = None;
                w.active = None;
            }
            VnHitKind::Up => w.active = None,
            VnHitKind::Scroll => {}
        }
    });
    window.vn_restrict_mouse_hit(&|_| false, false);
}

/// Is the pointer over the popup? Then nothing of the page is hovered anew
/// (`pointer::resolve_hover`).
pub(crate) fn covers(window: &Window, p: Point<Pixels>) -> bool {
    let k = windows::key(window);
    OPEN.with(|o| o.borrow().as_ref().filter(|o| o.window == k).and_then(|o| o.bounds).is_some_and(|b| b.contains(&p)))
}

// ---------------------------------------------------------------------------
// The keyboard
// ---------------------------------------------------------------------------

/// The build root's key listeners: the popup's keys in the capture phase,
/// before anything of the page; a closed select's in the bubble phase, after
/// every `keydown` handler, so a cancelled one does nothing.
pub(crate) fn bind_root<E: InteractiveElement>(e: E) -> E {
    e.capture_key_down(|ev: &KeyDownEvent, window, cx| {
        if popup_key(ev, window, cx) {
            cx.stop_propagation();
        }
    })
    .capture_key_up(|_: &KeyUpEvent, window, cx| {
        if open_in(windows::key(window)).is_some() {
            cx.stop_propagation();
        }
    })
    .on_key_down(closed_key)
}

/// A key while the popup is open. `true`: it was the popup's (they all are).
fn popup_key(ev: &KeyDownEvent, window: &mut Window, cx: &mut App) -> bool {
    let k = windows::key(window);
    let Some(select) = still_open(k) else { return false };
    let rows = tree::read_tree(|t| rows(t, select));
    let (current, searching) =
        OPEN.with(|o| o.borrow().as_ref().map(|o| (o.highlight, o.typeahead.active(Instant::now())))).unwrap_or_default();
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    let accept = |window: &mut Window, cx: &mut App| match current.filter(|&i| rows.iter().any(|r| r.pickable() == Some(i))) {
        Some(i) => pick(select, i, window, cx),
        None => close(window, cx),
    };
    match key {
        "escape" => close(window, cx),
        "enter" | "tab" => accept(window, cx),
        "f4" if !m.alt => accept(window, cx),
        "up" | "down" if m.alt => accept(window, cx),
        "space" if !searching => accept(window, cx),
        _ => {
            if let Some(to) = moved(&rows, current, key) {
                highlight(to, true, window, cx);
            } else if let Some(ch) = typed(ev) {
                let found = OPEN.with(|o| {
                    o.borrow_mut().as_mut().and_then(|o| o.typeahead.find(&rows, current, ch, Instant::now()))
                });
                if found.is_some() {
                    highlight(found, true, window, cx);
                }
            }
        }
    }
    true
}

/// A key on a focused, closed select, once every `keydown` handler has run.
fn closed_key(ev: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    if open_in(k).is_some() || windows::here(window, |w| w.key_prevented) {
        return;
    }
    let select = windows::focused_node(k);
    let Some((rows, current)) = tree::read_tree(|t| usable(t, select).then(|| (rows(t, select), shown(t, select))))
    else {
        return;
    };
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    let plain = !m.control && !m.alt && !m.platform;
    let now = Instant::now();
    let searching = TYPED.with(|s| s.borrow().active(now));
    match key {
        "up" | "down" if m.alt && !m.control && !m.platform => open(select, window, cx),
        "f4" if !m.modified() => open(select, window, cx),
        "enter" if !m.modified() => open(select, window, cx),
        "space" if plain && !searching => open(select, window, cx),
        _ if plain => {
            let to = match moved(&rows, current, key) {
                Some(to) => to,
                None => match typed(ev) {
                    Some(ch) => TYPED.with(|s| s.borrow_mut().find(&rows, current, ch, now)),
                    None => return,
                },
            };
            if let Some(to) = to.filter(|&to| Some(to) != current) {
                commit(select, to, window, cx);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// What a select shows inside its own box: the shown option's label, every
/// other label at no height (they size the box), and Chromium's arrow and
/// internal padding unless `appearance` is `none`.
pub(crate) fn closed_children(
    t: &Tree,
    id: u32,
    skin: bool,
    case: Option<vn_css::TextTransform>,
) -> Vec<AnyElement> {
    let rows = rows(t, id);
    let shown_ix = shown(t, id);
    let mut labels = div().flex().flex_col().min_w(px(0.)).overflow_hidden();
    if skin {
        labels = labels
            .pl(px(PAD_START))
            .pr(px(PAD_END))
            .py(px(PAD_BLOCK))
            .line_height(relative(1.2));
    }
    // The shown one first and in flow; an empty label still takes a line.
    let shown_label = rows.iter().find_map(|r| match r {
        Row::Opt { index, label, .. } if Some(*index) == shown_ix => Some(label.as_str()),
        _ => None,
    });
    let line = match shown_label {
        Some(l) if !l.is_empty() => crate::render::transformed_text(l, case),
        _ => "\u{200b}".into(),
    };
    labels = labels.child(div().whitespace_nowrap().child(line));
    for r in &rows {
        if let Row::Opt { index, label, .. } = r {
            if Some(*index) == shown_ix || label.is_empty() {
                continue;
            }
            let mut ghost = div().h(px(0.)).overflow_hidden().whitespace_nowrap();
            ghost.style().visibility = Some(gpui::Visibility::Hidden);
            labels = labels.child(ghost.child(crate::render::transformed_text(label, case)));
        }
    }
    let mut kids = vec![labels.into_any_element()];
    if skin {
        kids.push(arrow());
    }
    kids
}

/// Chromium's menulist arrow: a chevron in the text colour, centred in the
/// arrow box at the end of the padding box, at the box's middle.
fn arrow() -> AnyElement {
    let mut c = gpui::canvas(
        |_, _, _| (),
        |bounds, (), window, _| {
            let color = window.text_style().color;
            let cx = f32::from(bounds.right()) - ARROW_BOX / 2.0;
            let cy = f32::from(bounds.top()) + f32::from(bounds.size.height) / 2.0;
            let (hw, hh) = (ARROW_W / 2.0, ARROW_W / 4.0);
            let mut b = gpui::PathBuilder::stroke(px(1.5));
            b.move_to(gpui::point(px(cx - hw), px(cy - hh)));
            b.line_to(gpui::point(px(cx), px(cy + hh)));
            b.line_to(gpui::point(px(cx + hw), px(cy - hh)));
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

/// The popup's colours (approximately Chromium's list picker's).
struct Palette {
    background: Hsla,
    border: Hsla,
    highlight: Hsla,
    highlight_text: Hsla,
    disabled: Hsla,
}

fn palette(dark: bool) -> Palette {
    let c = |hex: u32| Hsla::from(gpui::rgb(hex));
    if dark {
        Palette {
            background: c(0x3b3b3b),
            border: c(0x858585),
            highlight: c(0x99c8ff),
            highlight_text: c(0x000000),
            disabled: c(0x8f8f8f),
        }
    } else {
        Palette {
            background: c(0xffffff),
            border: c(0x767676),
            highlight: c(0x1e90ff),
            highlight_text: c(0xffffff),
            disabled: c(0x6d6d6d),
        }
    }
}

/// The popup, if `id`'s is open in this window: a deferred child of the
/// select, so it inherits the select's text style and paints over everything.
pub(crate) fn popup_child(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    case: Option<vn_css::TextTransform>,
    window: &mut Window,
) -> Option<AnyElement> {
    let k = windows::key(window);
    if open_in(k) != Some(id) {
        return None;
    }
    if !window.is_window_active() || windows::focused_node(k) != id || !usable(t, id) {
        OPEN.with(|o| o.borrow_mut().take());
        return None;
    }
    let (current, scroll, reveal) = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let o = o.as_mut()?;
        Some((o.highlight, o.scroll.clone(), std::mem::take(&mut o.reveal)))
    })?;
    let rows = rows(t, id);
    let dark = windows::env_of(k).unwrap_or_else(crate::env).dark;
    let pal = palette(dark);
    let own_background = node
        .computed
        .as_ref()
        .and_then(|cs| cs.base.background.as_ref())
        .and_then(gpui::Fill::color)
        .and_then(|b| b.as_solid())
        .filter(|c| c.a > 0.0);
    // The select's box, last frame: the popup is at least as wide, and fits
    // the room above or below it.
    let rect = windows::peek(k, |w| w.bounds.get(&id).or_else(|| w.prev_bounds.get(&id)).copied()).flatten();
    let (width, room) = match rect {
        Some((_, y, w, h)) => {
            let viewport = f32::from(window.viewport_size().height);
            (w, (viewport - (y + h)).max(y).max(0.0))
        }
        None => (0.0, f32::INFINITY),
    };
    let font = window.text_style().font_size.to_pixels(window.rem_size());
    let row_height = f32::from(font) * 1.2 + 2.0 * PAD_BLOCK;
    let max_height = (MAX_ROWS as f32 * row_height + 2.0).min(room);

    let mut list = div()
        .id(("vn-select-list", id as usize))
        .flex()
        .flex_col()
        .max_h(px((max_height - 2.0).max(row_height)))
        .overflow_y_scroll()
        .track_scroll(&scroll);
    let mut reveal_row = None;
    for (row_ix, r) in rows.iter().enumerate() {
        match r {
            Row::Group { label } => {
                list = list.child(
                    div()
                        .px(px(PAD_START))
                        .py(px(PAD_BLOCK))
                        .whitespace_nowrap()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(if label.is_empty() { "\u{200b}".into() } else { crate::render::transformed_text(label, case) }),
                );
            }
            Row::Opt { hidden: true, .. } => {}
            Row::Opt { index, label, disabled, grouped, .. } => {
                let index = *index;
                let lit = current == Some(index);
                if lit {
                    reveal_row = Some(row_ix);
                }
                let text = if label.is_empty() { "\u{200b}".into() } else { crate::render::transformed_text(label, case) };
                let mut row = div()
                    .id(("vn-select-option", index))
                    .pl(px(if *grouped { PAD_START + 12.0 } else { PAD_START }))
                    .pr(px(PAD_START))
                    .py(px(PAD_BLOCK))
                    .whitespace_nowrap()
                    .child(text);
                if *disabled {
                    row = row.text_color(pal.disabled);
                } else {
                    if lit {
                        row = row.bg(pal.highlight).text_color(pal.highlight_text);
                    }
                    row = row
                        .on_mouse_move(move |_, window, cx| highlight(Some(index), false, window, cx))
                        .on_mouse_up(MouseButton::Left, move |_, window, cx| pick(id, index, window, cx));
                }
                list = list.child(row);
            }
        }
    }
    if reveal {
        if let Some(r) = reveal_row {
            // The list's children are the rows that are shown, in order.
            let shown_before = rows[..r].iter().filter(|r| !matches!(r, Row::Opt { hidden: true, .. })).count();
            scroll.scroll_to_item(shown_before);
        }
    }
    let frame = div()
        .id(("vn-select-popup", id as usize))
        .occlude()
        .flex()
        .flex_col()
        .min_w(px(width))
        .bg(own_background.unwrap_or(pal.background))
        .border_1()
        .border_color(pal.border)
        .line_height(relative(1.2))
        .child(list);
    let placed = Placed { select: id, child: frame.into_any_element() };
    Some(gpui::deferred(placed).priority(1).with_stack_key(std::rc::Rc::from([u128::MAX])).into_any_element())
}

/// The popup's placement: under the select's box, or above it when it does
/// not fit there and fits better above, kept inside the window sideways.
/// gpui's `anchored` flips around ONE point; a popup flips around a box.
struct Placed {
    select: u32,
    child: AnyElement,
}

impl IntoElement for Placed {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Placed {
    type RequestLayoutState = gpui::LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, gpui::LayoutId) {
        let child = self.child.request_layout(window, cx);
        let style = gpui::Style { position: gpui::Position::Absolute, display: gpui::Display::Flex, ..Default::default() };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut gpui::LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid = window.layout_bounds(*child);
        let k = windows::key(window);
        let rect = windows::peek(k, |w| w.bounds.get(&self.select).or_else(|| w.prev_bounds.get(&self.select)).copied())
            .flatten()
            .unwrap_or_default();
        let viewport = window.viewport_size();
        let origin = place(rect, laid.size, viewport);
        let placed = Bounds { origin, size: laid.size };
        let offset = origin - laid.origin;
        window.with_element_offset(gpui::point(offset.x.round(), offset.y.round()), |window| {
            self.child.prepaint(window, cx)
        });
        OPEN.with(|o| {
            if let Some(o) = o.borrow_mut().as_mut().filter(|o| o.select == self.select && o.window == k) {
                o.bounds = Some(placed);
            }
        });
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _child: &mut gpui::LayoutId,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// Where a popup of `size` goes for a select at `rect` (`x, y, w, h`) in a
/// window of `viewport`.
fn place(rect: windows::Rect, size: gpui::Size<Pixels>, viewport: gpui::Size<Pixels>) -> Point<Pixels> {
    let (x, y, _, h) = rect;
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
    let (pw, ph) = (f32::from(size.width), f32::from(size.height));
    let below = vh - (y + h);
    let top = if ph <= below || below >= y { y + h } else { (y - ph).max(0.0) };
    let left = x.min(vw - pw).max(0.0);
    gpui::point(px(left), px(top))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(index: usize, label: &str) -> Row {
        Row::Opt { index, label: label.into(), disabled: false, hidden: false, grouped: false }
    }

    fn off(index: usize, label: &str) -> Row {
        Row::Opt { index, label: label.into(), disabled: true, hidden: false, grouped: false }
    }

    /// select > (option "  Every\n body ", optgroup[label=G] > (option[label=Lbl] "x", option[disabled]), option)
    #[test]
    fn rows_are_html_s_list_of_options_with_the_group_labels() {
        let (select, rows) = tree::with_tree(|t| {
            let (s, o, g) = (t.intern("select"), t.intern("option"), t.intern("optgroup"));
            let (label, disabled) = (t.intern("label"), t.intern("disabled"));
            let select = t.create_element(s);
            let a = t.create_element(o);
            t.set_element_text(a, "  Every\n body ".into());
            let group = t.create_element(g);
            t.set_prop(group, label, Some("G".into()));
            let b = t.create_element(o);
            t.set_prop(b, label, Some("Lbl".into()));
            t.set_element_text(b, "x".into());
            let c = t.create_element(o);
            t.set_prop(c, disabled, Some(String::new()));
            let text = t.create_text();
            t.set_text(text, " C ".into());
            t.insert(text, c, 0);
            let d = t.create_element(o);
            t.insert(a, select, 0);
            t.insert(group, select, 0);
            t.insert(b, group, 0);
            t.insert(c, group, 0);
            t.insert(d, select, 0);
            (select, rows(t, select))
        });
        assert_eq!(
            rows,
            vec![
                opt(0, "Every body"),
                Row::Group { label: "G".into() },
                Row::Opt { index: 1, label: "Lbl".into(), disabled: false, hidden: false, grouped: true },
                Row::Opt { index: 2, label: "C".into(), disabled: true, hidden: false, grouped: true },
                opt(3, ""),
            ]
        );
        let sel = tree::with_tree(|t| {
            let k = t.intern(":selected");
            t.set_prop(select, k, Some("2".into()));
            shown(t, select)
        });
        assert_eq!(sel, Some(2));
        tree::with_tree(|t| {
            let k = t.intern(":selected");
            t.set_prop(select, k, Some("-1".into()));
            assert_eq!(shown(t, select), None);
        });
    }

    #[test]
    fn keys_move_over_the_enabled_options_and_stop_at_the_ends() {
        let rows = vec![off(0, "a"), opt(1, "b"), Row::Group { label: "g".into() }, off(2, "c"), opt(3, "d"), opt(4, "e")];
        assert_eq!(moved(&rows, Some(1), "down"), Some(Some(3)), "past the group and the disabled one");
        assert_eq!(moved(&rows, Some(3), "up"), Some(Some(1)));
        assert_eq!(moved(&rows, Some(1), "up"), Some(Some(1)), "the first stays");
        assert_eq!(moved(&rows, Some(4), "right"), Some(Some(4)), "the last stays");
        assert_eq!(moved(&rows, Some(2), "down"), Some(Some(3)), "on from a disabled selected one");
        assert_eq!(moved(&rows, Some(2), "up"), Some(Some(1)));
        assert_eq!(moved(&rows, None, "down"), Some(Some(1)), "nothing selected: the first");
        assert_eq!(moved(&rows, Some(1), "end"), Some(Some(4)));
        assert_eq!(moved(&rows, Some(4), "home"), Some(Some(1)));
        assert_eq!(moved(&rows, Some(1), "pagedown"), Some(Some(4)));
        assert_eq!(moved(&rows, Some(4), "pageup"), Some(Some(1)));
        assert_eq!(moved(&rows, Some(1), "a"), None);
        assert_eq!(moved(&[off(0, "x")], Some(0), "down"), Some(None), "nothing to move to");
    }

    #[test]
    fn typing_finds_by_prefix_and_a_repeated_letter_cycles() {
        let rows = vec![opt(0, "Apple"), opt(1, "  banana"), opt(2, "Blueberry"), off(3, "Bad"), opt(4, "cherry")];
        let t0 = Instant::now();
        let mut ta = Typeahead::default();
        assert_eq!(ta.find(&rows, Some(0), 'b', t0), Some(1), "leading white space and case do not count");
        assert_eq!(ta.find(&rows, Some(1), 'b', t0), Some(2), "the same letter again: the next one");
        assert_eq!(ta.find(&rows, Some(2), 'b', t0), Some(1), "…cycling, past the disabled one");
        let mut ta = Typeahead::default();
        assert_eq!(ta.find(&rows, Some(0), 'b', t0), Some(1));
        assert_eq!(ta.find(&rows, Some(1), 'l', t0), Some(2), "a prefix, from the current one");
        assert_eq!(ta.find(&rows, Some(2), 'x', t0), None);
        // A pause starts over.
        let later = t0 + TYPEAHEAD_RESET + Duration::from_millis(1);
        assert_eq!(ta.find(&rows, Some(2), 'c', later), Some(4));
        assert!(ta.active(later));
        assert!(!ta.active(later + TYPEAHEAD_RESET));
    }

    #[test]
    fn the_popup_goes_under_the_box_and_above_it_when_only_that_fits() {
        let vp = gpui::size(px(800.), px(600.));
        let sz = |w: f32, h: f32| gpui::size(px(w), px(h));
        // Room below.
        assert_eq!(place((10., 100., 200., 30.), sz(200., 300.), vp), gpui::point(px(10.), px(130.)));
        // Not below, more room above: above.
        assert_eq!(place((10., 500., 200., 30.), sz(200., 300.), vp), gpui::point(px(10.), px(200.)));
        // Fits neither, more room below: below (its list scrolls).
        assert_eq!(place((10., 100., 200., 30.), sz(200., 550.), vp), gpui::point(px(10.), px(130.)));
        // Kept inside the window sideways.
        assert_eq!(place((700., 100., 80., 30.), sz(200., 100.), vp), gpui::point(px(600.), px(130.)));
    }
}
