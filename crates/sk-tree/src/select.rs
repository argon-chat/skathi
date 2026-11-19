//! `<select>`: the closed box Chromium draws on Windows, and the popup list it opens.
//!
//! The box is the element's own div (author border, padding, background) around the selected option's
//! label and an 8×4 chevron, with Chromium's internal padding and a `normal` line height; the width is
//! the widest option's. `appearance: none` removes the arrow and padding. The selection itself is the
//! runtime's: it pushes the selected index as the reserved `:selected` prop, and a pick goes back as
//! the select's `input` host event with the option's index in `a`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, px, relative, AnyElement, App, Bounds, Hsla, KeyDownEvent, KeyUpEvent, MouseButton,
    Pixels, Point, ScrollHandle, VnHitKind, Window,
};

use crate::popup::Slot;
use crate::tree::{self, Kind, Tree};
use crate::windows::{self, WinKey};
use crate::EventPayload;

/// Chromium's menulist padding (`LayoutThemeDefault::PopupInternalPadding*`).
const PAD_START: f32 = 4.0;
const PAD_BLOCK: f32 = 1.0;
/// The arrow box: the scroll bar button's width (15px under Windows 11's Fluent scroll bars), at
/// the end of the padding box.
const ARROW_BOX: f32 = 15.0;
/// 1px, then the arrow box.
const PAD_END: f32 = 1.0 + ARROW_BOX;
/// The chevron (`ThemePainterDefault::SetupMenuListArrow`: 8px wide, half as tall).
const ARROW_W: f32 = 8.0;
/// The popup's rows before it scrolls (Chromium's list picker).
const MAX_ROWS: usize = 20;
/// A pause this long starts a new type-ahead search.
const TYPEAHEAD_RESET: Duration = Duration::from_millis(1000);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Row {
    Group {
        label: String,
    },
    Opt {
        index: usize,
        label: String,
        disabled: bool,
        hidden: bool,
        grouped: bool,
    },
}

impl Row {
    fn pickable(&self) -> Option<usize> {
        match self {
            Row::Opt {
                index,
                disabled: false,
                hidden: false,
                ..
            } => Some(*index),
            _ => None,
        }
    }
}

fn flag(t: &Tree, id: u32, name: &str) -> bool {
    t.prop(id, name).is_some_and(|v| v != "false")
}

fn element_named(t: &Tree, id: u32, tag: &str) -> bool {
    t.node(id)
        .is_some_and(|n| n.kind == Kind::Element && t.name(n.tag) == tag)
}

pub(crate) fn is_select(t: &Tree, id: u32) -> bool {
    element_named(t, id, "select")
}

/// HTML's option label: a non-empty `label` attribute, else the option's text with ASCII white
/// space stripped and collapsed.
pub(crate) fn option_label(t: &Tree, id: u32) -> String {
    if let Some(l) = t.prop(id, "label").filter(|l| !l.is_empty()) {
        return l.to_string();
    }
    let mut raw = String::new();
    collect_text(t, id, &mut raw, 0);
    raw.split([' ', '\t', '\n', '\x0c', '\r'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
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
            out.push(Row::Group {
                label: t.prop(c, "label").unwrap_or("").to_string(),
            });
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

pub(crate) fn shown(t: &Tree, select: u32) -> Option<usize> {
    let ix = t.prop(select, ":selected")?.trim().parse::<i64>().ok()?;
    usize::try_from(ix).ok()
}

fn usable(t: &Tree, select: u32) -> bool {
    is_select(t, select) && !crate::render::actually_disabled(t, select)
}

fn choices(rows: &[Row]) -> Vec<usize> {
    rows.iter().filter_map(Row::pickable).collect()
}

fn step(rows: &[Row], from: Option<usize>, delta: isize) -> Option<usize> {
    let c = choices(rows);
    let (first, last) = (*c.first()?, *c.last()?);
    let Some(from) = from else { return Some(first) };
    let at = if delta > 0 {
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
    Some(if at < 0 {
        first
    } else {
        c.get(at as usize).copied().unwrap_or(last)
    })
}

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

#[derive(Default)]
struct Typeahead {
    text: String,
    at: Option<Instant>,
}

impl Typeahead {
    fn active(&self, now: Instant) -> bool {
        !self.text.is_empty()
            && self
                .at
                .is_some_and(|t| now.duration_since(t) < TYPEAHEAD_RESET)
    }

    fn find(&mut self, rows: &[Row], from: Option<usize>, ch: char, now: Instant) -> Option<usize> {
        if !self.active(now) {
            self.text.clear();
        }
        self.at = Some(now);
        self.text.extend(ch.to_lowercase());
        let first = self.text.chars().next()?;
        let cycle = self.text.chars().all(|c| c == first);
        let needle: String = if cycle {
            first.to_string()
        } else {
            self.text.clone()
        };
        let c = choices(rows);
        if c.is_empty() {
            return None;
        }
        let start = match from {
            Some(f) => c
                .iter()
                .position(|&i| if cycle { i > f } else { i >= f })
                .unwrap_or(0),
            None => 0,
        };
        let label = |ix: usize| {
            rows.iter().find_map(|r| match r {
                Row::Opt { index, label, .. } if *index == ix => {
                    Some(label.trim_start().to_lowercase())
                }
                _ => None,
            })
        };
        (0..c.len())
            .map(|k| c[(start + k) % c.len()])
            .find(|&ix| label(ix).is_some_and(|l| l.starts_with(&needle)))
    }
}

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

struct Open {
    select: u32,
    window: WinKey,
    highlight: Option<usize>,
    scroll: ScrollHandle,
    reveal: bool,
    bounds: Option<Bounds<Pixels>>,
    typeahead: Typeahead,
}

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    static PRESSED: Cell<Option<(WinKey, u32)>> = const { Cell::new(None) };
    /// The window whose next release is swallowed: its press closed a popup.
    static SWALLOW_UP: Cell<Option<WinKey>> = const { Cell::new(None) };
    static TYPED: RefCell<Typeahead> = RefCell::new(Typeahead::default());
}

fn open_in(k: WinKey) -> Option<u32> {
    OPEN.with(|o| {
        o.borrow()
            .as_ref()
            .filter(|o| o.window == k)
            .map(|o| o.select)
    })
}

/// The open select if it still paints; otherwise it closes, without a repaint (the caller is mid-event, or mid-build).
fn still_open(k: WinKey, cx: &mut App) -> Option<u32> {
    let select = open_in(k)?;
    let painted = windows::peek(k, |w| {
        w.bounds.contains_key(&select) || w.prev_bounds.contains_key(&select)
    })
    .unwrap_or(false);
    let ok =
        painted && windows::focused_node(k) == select && tree::read_tree(|t| usable(t, select));
    if ok {
        return Some(select);
    }
    forget(cx);
    None
}

/// Closes without a repaint.
fn forget(cx: &mut App) {
    if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
        crate::popup::close(Slot::Select, o.select, cx);
    }
}

fn changed(select: u32, window: &mut Window, cx: &mut App) {
    tree::with_tree(|t| t.touch(select));
    crate::render::request_repaint(window, cx);
}

fn open(select: u32, window: &mut Window, cx: &mut App) {
    let Some(highlight) = tree::read_tree(|t| usable(t, select).then(|| shown(t, select))) else {
        return;
    };
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
        crate::popup::close(Slot::Select, o.select, cx);
        changed(o.select, window, cx);
    }
}

fn pick(select: u32, index: usize, window: &mut Window, cx: &mut App) {
    close(window, cx);
    commit(select, index, window, cx);
}

fn commit(select: u32, index: usize, window: &mut Window, cx: &mut App) {
    let payload = EventPayload {
        a: index as f64,
        ..Default::default()
    };
    crate::render::js_dispatch_app(select, tree::EV_INPUT, &payload, window, cx);
}

fn highlight(index: Option<usize>, reveal: bool, window: &mut Window, cx: &mut App) {
    let select = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let o = o.as_mut()?;
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

pub(crate) fn bind_press<E: InteractiveElement>(e: E, id: u32) -> E {
    e.on_mouse_down(MouseButton::Left, move |_, window, _| {
        PRESSED.with(|p| p.set(Some((windows::key(window), id))));
    })
}

pub(crate) fn press_settled(cancelled: bool, window: &mut Window, cx: &mut App) {
    let Some((k, select)) = PRESSED.with(|p| p.take()) else {
        return;
    };
    if cancelled || k != windows::key(window) {
        return;
    }
    open(select, window, cx);
}

pub(crate) fn refine(kind: VnHitKind, p: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
    let k = windows::key(window);
    if kind == VnHitKind::Up && SWALLOW_UP.with(|s| s.get()) == Some(k) {
        SWALLOW_UP.with(|s| s.set(None));
        hit_nothing(kind, window);
        return true;
    }
    if still_open(k, cx).is_none() {
        return false;
    }
    let inside = OPEN.with(|o| {
        o.borrow()
            .as_ref()
            .and_then(|o| o.bounds)
            .is_some_and(|b| b.contains(&p))
    });
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
        VnHitKind::Up => false,
    }
}

pub(crate) fn hit_nothing(kind: VnHitKind, window: &mut Window) {
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

pub(crate) fn covers(window: &Window, p: Point<Pixels>) -> bool {
    let k = windows::key(window);
    OPEN.with(|o| {
        o.borrow()
            .as_ref()
            .filter(|o| o.window == k)
            .and_then(|o| o.bounds)
            .is_some_and(|b| b.contains(&p))
    })
}

/// The build root's key listeners: the popup's keys in the capture phase; a closed select's in the
/// bubble phase after every `keydown` handler, so a cancelled one does nothing.
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

fn popup_key(ev: &KeyDownEvent, window: &mut Window, cx: &mut App) -> bool {
    let k = windows::key(window);
    let Some(select) = still_open(k, cx) else {
        return false;
    };
    let rows = tree::read_tree(|t| rows(t, select));
    let (current, searching) = OPEN
        .with(|o| {
            o.borrow()
                .as_ref()
                .map(|o| (o.highlight, o.typeahead.active(Instant::now())))
        })
        .unwrap_or_default();
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    let accept = |window: &mut Window, cx: &mut App| match current
        .filter(|&i| rows.iter().any(|r| r.pickable() == Some(i)))
    {
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
                    o.borrow_mut()
                        .as_mut()
                        .and_then(|o| o.typeahead.find(&rows, current, ch, Instant::now()))
                });
                if found.is_some() {
                    highlight(found, true, window, cx);
                }
            }
        }
    }
    true
}

fn closed_key(ev: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    if open_in(k).is_some() || windows::here(window, |w| w.key_prevented) {
        return;
    }
    let select = windows::focused_node(k);
    let Some((rows, current)) =
        tree::read_tree(|t| usable(t, select).then(|| (rows(t, select), shown(t, select))))
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

pub(crate) fn closed_children(
    t: &Tree,
    id: u32,
    skin: bool,
    case: Option<sk_css::TextTransform>,
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

#[derive(Clone, Copy)]
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

pub(crate) fn popup_child(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    case: Option<sk_css::TextTransform>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let k = windows::key(window);
    if open_in(k) != Some(id) {
        return None;
    }
    if !window.is_window_active()
        || windows::focused_node(k) != id
        || !usable(t, id)
        || crate::popup::owner_moved(Slot::Select, window)
    {
        forget(cx);
        return None;
    }
    let (current, scroll, reveal) = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let o = o.as_mut()?;
        Some((o.highlight, o.scroll.clone(), std::mem::take(&mut o.reveal)))
    })?;
    let rows = rows(t, id);
    let dark = crate::scheme::declared(t, id, window)
        .unwrap_or_else(|| windows::env_of(k).unwrap_or_else(crate::env).dark);
    let pal = palette(dark);
    let own_background = node
        .computed
        .as_ref()
        .and_then(|cs| cs.base.background.as_ref())
        .and_then(gpui::Fill::color)
        .and_then(|b| b.as_solid())
        .filter(|c| c.a > 0.0);
    let rect = windows::peek(k, |w| {
        w.bounds
            .get(&id)
            .or_else(|| w.prev_bounds.get(&id))
            .copied()
    })
    .flatten();
    let (width, room) = match rect {
        Some(r) if crate::popup::native() => (r.2, crate::popup::room(window, r, 0.0)),
        Some((_, y, w, h)) => {
            let viewport = f32::from(window.viewport_size().height);
            (w, Some((viewport - (y + h)).max(y).max(0.0)))
        }
        None => (0.0, None),
    };
    // Built before layout, so the window's text style is not the select's yet.
    let font = node
        .computed
        .as_ref()
        .and_then(|cs| cs.base.text.font_size)
        .unwrap_or(window.text_style().font_size)
        .to_pixels(window.rem_size());
    let row_height = f32::from(font) * 1.2 + 2.0 * PAD_BLOCK;
    let max_height = (MAX_ROWS as f32 * row_height + 2.0).min(room.unwrap_or(f32::INFINITY));
    let reveal = Rc::new(Cell::new(reveal));
    let build = move || {
        list(
            &rows,
            Shown {
                select: id,
                owner: k,
                current,
                scroll: &scroll,
                reveal: reveal.take(),
                max_height,
                row_height,
                case,
                pal,
            },
        )
        .min_w(px(width))
        .bg(own_background.unwrap_or(pal.background))
        .into_any_element()
    };
    Some(
        crate::popup::Popup {
            slot: Slot::Select,
            anchor: id,
            fallback: Default::default(),
            gap: 0.0,
            margin: 0.0,
            inherit_text: true,
            build: Rc::new(build),
            placed: note_placed,
            dismiss: close,
        }
        .into_element(),
    )
}

struct Shown<'a> {
    select: u32,
    owner: WinKey,
    current: Option<usize>,
    scroll: &'a ScrollHandle,
    /// Scroll the highlighted row into view.
    reveal: bool,
    max_height: f32,
    row_height: f32,
    case: Option<sk_css::TextTransform>,
    pal: Palette,
}

fn list(rows: &[Row], s: Shown) -> gpui::Stateful<gpui::Div> {
    let Shown {
        select: id,
        owner,
        current,
        scroll,
        reveal,
        max_height,
        row_height,
        case,
        pal,
    } = s;
    let mut list = div()
        .id(("sk-select-list", id as usize))
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
                        .child(if label.is_empty() {
                            "\u{200b}".into()
                        } else {
                            crate::render::transformed_text(label, case)
                        }),
                );
            }
            Row::Opt { hidden: true, .. } => {}
            Row::Opt {
                index,
                label,
                disabled,
                grouped,
                ..
            } => {
                let index = *index;
                let lit = current == Some(index);
                if lit {
                    reveal_row = Some(row_ix);
                }
                let text = if label.is_empty() {
                    "\u{200b}".into()
                } else {
                    crate::render::transformed_text(label, case)
                };
                let mut row = div()
                    .id(("sk-select-option", index))
                    .pl(px(if *grouped {
                        PAD_START + 12.0
                    } else {
                        PAD_START
                    }))
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
                        .on_mouse_move(move |_, window, cx| {
                            crate::popup::in_window(owner, window, cx, move |window, cx| {
                                highlight(Some(index), false, window, cx)
                            })
                        })
                        .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                            crate::popup::in_window(owner, window, cx, move |window, cx| {
                                pick(id, index, window, cx)
                            })
                        });
                }
                list = list.child(row);
            }
        }
    }
    if reveal {
        if let Some(r) = reveal_row {
            let shown_before = rows[..r]
                .iter()
                .filter(|r| !matches!(r, Row::Opt { hidden: true, .. }))
                .count();
            scroll.scroll_to_item(shown_before);
        }
    }
    div()
        .id(("sk-select-popup", id as usize))
        .occlude()
        .flex_none()
        .flex()
        .flex_col()
        .border_1()
        .border_color(pal.border)
        .line_height(relative(1.2))
        .child(list)
}

fn note_placed(k: WinKey, select: u32, placed: Bounds<Pixels>) {
    OPEN.with(|o| {
        if let Some(o) = o
            .borrow_mut()
            .as_mut()
            .filter(|o| o.select == select && o.window == k)
        {
            o.bounds = Some(placed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(index: usize, label: &str) -> Row {
        Row::Opt {
            index,
            label: label.into(),
            disabled: false,
            hidden: false,
            grouped: false,
        }
    }

    fn off(index: usize, label: &str) -> Row {
        Row::Opt {
            index,
            label: label.into(),
            disabled: true,
            hidden: false,
            grouped: false,
        }
    }

    /// select > (option "  Every\n body ", optgroup[label=G] > (option[label=Lbl] "x",
    /// option[disabled]), option)
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
                Row::Opt {
                    index: 1,
                    label: "Lbl".into(),
                    disabled: false,
                    hidden: false,
                    grouped: true
                },
                Row::Opt {
                    index: 2,
                    label: "C".into(),
                    disabled: true,
                    hidden: false,
                    grouped: true
                },
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
        let rows = vec![
            off(0, "a"),
            opt(1, "b"),
            Row::Group { label: "g".into() },
            off(2, "c"),
            opt(3, "d"),
            opt(4, "e"),
        ];
        assert_eq!(
            moved(&rows, Some(1), "down"),
            Some(Some(3)),
            "past the group and the disabled one"
        );
        assert_eq!(moved(&rows, Some(3), "up"), Some(Some(1)));
        assert_eq!(
            moved(&rows, Some(1), "up"),
            Some(Some(1)),
            "the first stays"
        );
        assert_eq!(
            moved(&rows, Some(4), "right"),
            Some(Some(4)),
            "the last stays"
        );
        assert_eq!(
            moved(&rows, Some(2), "down"),
            Some(Some(3)),
            "on from a disabled selected one"
        );
        assert_eq!(moved(&rows, Some(2), "up"), Some(Some(1)));
        assert_eq!(
            moved(&rows, None, "down"),
            Some(Some(1)),
            "nothing selected: the first"
        );
        assert_eq!(moved(&rows, Some(1), "end"), Some(Some(4)));
        assert_eq!(moved(&rows, Some(4), "home"), Some(Some(1)));
        assert_eq!(moved(&rows, Some(1), "pagedown"), Some(Some(4)));
        assert_eq!(moved(&rows, Some(4), "pageup"), Some(Some(1)));
        assert_eq!(moved(&rows, Some(1), "a"), None);
        assert_eq!(
            moved(&[off(0, "x")], Some(0), "down"),
            Some(None),
            "nothing to move to"
        );
    }

    #[test]
    fn typing_finds_by_prefix_and_a_repeated_letter_cycles() {
        let rows = vec![
            opt(0, "Apple"),
            opt(1, "  banana"),
            opt(2, "Blueberry"),
            off(3, "Bad"),
            opt(4, "cherry"),
        ];
        let t0 = Instant::now();
        let mut ta = Typeahead::default();
        assert_eq!(
            ta.find(&rows, Some(0), 'b', t0),
            Some(1),
            "leading white space and case do not count"
        );
        assert_eq!(
            ta.find(&rows, Some(1), 'b', t0),
            Some(2),
            "the same letter again: the next one"
        );
        assert_eq!(
            ta.find(&rows, Some(2), 'b', t0),
            Some(1),
            "…cycling, past the disabled one"
        );
        let mut ta = Typeahead::default();
        assert_eq!(ta.find(&rows, Some(0), 'b', t0), Some(1));
        assert_eq!(
            ta.find(&rows, Some(1), 'l', t0),
            Some(2),
            "a prefix, from the current one"
        );
        assert_eq!(ta.find(&rows, Some(2), 'x', t0), None);
        let later = t0 + TYPEAHEAD_RESET + Duration::from_millis(1);
        assert_eq!(ta.find(&rows, Some(2), 'c', later), Some(4));
        assert!(ta.active(later));
        assert!(!ta.active(later + TYPEAHEAD_RESET));
    }
}
