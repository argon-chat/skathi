//! `<input type=color>`'s picker: a popover drawn in the input's window, under the input. The
//! runtime opens it (`__sk_color_picker`) once the input's `showpicker` event was not cancelled.
//! While it is open it takes the keyboard and the pointer over it. Each edit writes `value` and
//! fires `input`; Enter, a press outside or the window going inactive commits with `change`;
//! Escape puts the old value back.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use gpui::prelude::*;
use gpui::{
    canvas, div, linear_color_stop, linear_gradient, linear_gradient_stops, px, AnyElement, App,
    Bounds, BoxShadow, ClipboardItem, Hsla, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, VnHitKind, Window,
};

use crate::color;
use crate::tree::{self, Tree};
use crate::windows::{self, WinKey};
use crate::EventPayload;

const CONTENT: f32 = 216.0;
const PAD: f32 = 12.0;
const GAP: f32 = 12.0;
const SQUARE_H: f32 = 144.0;
const TRACK: f32 = 12.0;
const THUMB: f32 = 18.0;
const FIELD_H: f32 = 32.0;
const CHIP: f32 = 20.0;
const RECENT_MAX: usize = 8;
const RADIUS: f32 = 8.0;
const ANCHOR_GAP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsva {
    h: f32,
    s: f32,
    v: f32,
    a: f32,
}

impl Hsva {
    /// `hue` stands in where the color has none (grays).
    fn from_rgba([r, g, b, a]: [f32; 4], hue: f32) -> Self {
        let max = r.max(g).max(b);
        let d = max - r.min(g).min(b);
        let h = if d <= 0.0 {
            hue
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        Hsva {
            h,
            s: if max > 0.0 { d / max } else { 0.0 },
            v: max,
            a,
        }
    }

    fn rgba(self) -> [f32; 4] {
        let h = self.h.rem_euclid(1.0) * 6.0;
        let c = self.v * self.s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        [r + m, g + m, b + m, self.a]
    }

    fn clamped(self) -> Self {
        Hsva {
            h: self.h.clamp(0.0, 1.0),
            s: self.s.clamp(0.0, 1.0),
            v: self.v.clamp(0.0, 1.0),
            a: self.a.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Square,
    Hue,
    Alpha,
    Hex,
    Recent,
}

#[derive(Clone)]
struct HexEdit {
    text: String,
    /// Selected whole: the next digit replaces it.
    all: bool,
}

struct Open {
    input: u32,
    window: WinKey,
    color: Hsva,
    original: String,
    /// The value this picker last wrote; any other value came from the app.
    written: String,
    part: Part,
    hex: Option<HexEdit>,
    recent: usize,
    drag: Option<Part>,
    bounds: Option<Bounds<Pixels>>,
    fallback: windows::Rect,
    keyboard: bool,
}

static SYSTEM: AtomicBool = AtomicBool::new(false);
static RECENTS_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    static REQUEST: Cell<Option<u32>> = const { Cell::new(None) };
    static SWALLOW_UP: Cell<Option<WinKey>> = const { Cell::new(None) };
    static RECENTS: RefCell<Option<Vec<[f32; 4]>>> = const { RefCell::new(None) };
}

/// `sk.config.json`'s `"colorPicker": "system"`: color inputs open the OS dialog instead.
pub fn use_system_dialog(on: bool) {
    SYSTEM.store(on, Ordering::Relaxed);
}

/// Where the recent colors persist; `None` keeps them for the run.
pub fn set_recents_file(path: Option<PathBuf>) {
    *RECENTS_FILE.lock().unwrap_or_else(|e| e.into_inner()) = path;
    RECENTS.with(|r| r.borrow_mut().take());
}

fn usable(t: &Tree, id: u32) -> bool {
    color::is_color_input(t, id) && !crate::render::actually_disabled(t, id)
}

/// Opens the picker for `id` at its window's next build. `false`: the app asked for the OS dialog.
pub(crate) fn request(id: u32) -> bool {
    if SYSTEM.load(Ordering::Relaxed) {
        return false;
    }
    if OPEN.with(|o| o.borrow().as_ref().is_some_and(|o| o.input == id)) {
        return true;
    }
    let ok = tree::with_tree(|t| {
        if !usable(t, id) {
            return false;
        }
        let key = t.intern(":picker");
        t.set_prop(id, key, Some("open".into()));
        true
    });
    if ok {
        REQUEST.with(|r| r.set(Some(id)));
    }
    true
}

fn open_in(k: WinKey) -> bool {
    OPEN.with(|o| o.borrow().as_ref().is_some_and(|o| o.window == k))
}

fn with_open<R>(f: impl FnOnce(&mut Open) -> R) -> Option<R> {
    OPEN.with(|o| o.borrow_mut().as_mut().map(f))
}

fn repaint(window: &mut Window, cx: &mut App) {
    let root = windows::root_of(windows::key(window));
    tree::with_tree(|t| t.touch(root));
    crate::render::request_repaint(window, cx);
}

fn write(input: u32, text: &str, commit: bool, window: &mut Window, cx: &mut App) {
    tree::with_tree(|t| {
        let key = t.intern("value");
        t.set_prop(input, key, Some(text.to_string()));
    });
    crate::render::js_dispatch_app(
        input,
        tree::EV_INPUT,
        &EventPayload {
            key: Some(text),
            b: if commit { 1.0 } else { 0.0 },
            ..Default::default()
        },
        window,
        cx,
    );
}

/// Applies an edit to the color, firing `input` when the value it serializes to moved.
fn apply(edit: impl FnOnce(&mut Hsva), window: &mut Window, cx: &mut App) {
    let wrote = with_open(|o| {
        edit(&mut o.color);
        o.color = o.color.clamped();
        let alpha = tree::read_tree(|t| color::has_alpha(t, o.input));
        let text = color::serialize(o.color.rgba(), alpha);
        (text != o.written).then(|| {
            o.written = text.clone();
            (o.input, text)
        })
    })
    .flatten();
    if let Some((input, text)) = wrote {
        write(input, &text, false, window, cx);
    }
    repaint(window, cx);
}

fn finish(commit: bool, window: &mut Window, cx: &mut App) {
    if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
        settle(o, commit, window, cx);
    }
}

fn settle(o: Open, commit: bool, window: &mut Window, cx: &mut App) {
    let alive = tree::with_tree(|t| {
        if t.node(o.input).is_some() {
            let key = t.intern(":picker");
            t.set_prop(o.input, key, None);
        }
        usable(t, o.input).then(|| t.prop(o.input, "value").unwrap_or("").to_string())
    });
    if let Some(current) = alive.filter(|c| *c != o.original) {
        if commit {
            if let Some(c) = color::parse(&current) {
                remember(c);
            }
            write(o.input, &current, true, window, cx);
        } else {
            write(o.input, &o.original, false, window, cx);
        }
    }
    repaint(window, cx);
}

/// The window's popover layer, from its root's build.
pub(crate) fn layer(t: &Tree, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let k = windows::key(window);
    if let Some(id) = REQUEST
        .with(Cell::get)
        .filter(|&id| windows::window_of_node_in(t, id) == k)
    {
        REQUEST.with(|r| r.set(None));
        if usable(t, id) {
            let value = t.prop(id, "value").unwrap_or("").to_string();
            let rgba = color::parse(&value).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let mouse = window.mouse_position();
            let fresh = Open {
                input: id,
                window: k,
                color: Hsva::from_rgba(rgba, 0.0),
                original: value.clone(),
                written: value,
                part: Part::Square,
                hex: None,
                recent: 0,
                drag: None,
                bounds: None,
                fallback: (f32::from(mouse.x), f32::from(mouse.y), 0.0, 0.0),
                keyboard: window.last_input_was_keyboard(),
            };
            if let Some(old) = OPEN.with(|o| o.borrow_mut().replace(fresh)) {
                window.defer(cx, move |window, cx| settle(old, true, window, cx));
            }
        }
    }
    let input = OPEN.with(|o| {
        o.borrow()
            .as_ref()
            .filter(|o| o.window == k)
            .map(|o| o.input)
    })?;
    if !usable(t, input) || !window.is_window_active() {
        if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
            window.defer(cx, move |window, cx| settle(o, true, window, cx));
        }
        return None;
    }
    let alpha = color::has_alpha(t, input);
    let value = t.prop(input, "value").unwrap_or("");
    let recents = recents();
    let state = with_open(|o| {
        if o.written != value {
            o.written = value.to_string();
            if let Some(c) = color::parse(value) {
                o.color = Hsva::from_rgba(c, o.color.h);
                o.hex = None;
            }
        }
        if !alpha {
            o.color.a = 1.0;
        }
        if (o.part == Part::Alpha && !alpha) || (o.part == Part::Recent && recents.is_empty()) {
            o.part = Part::Square;
        }
        View {
            color: o.color,
            part: o.part,
            hex: o.hex.clone(),
            recent: o.recent.min(recents.len().saturating_sub(1)),
            keyboard: o.keyboard,
            fallback: o.fallback,
        }
    })?;
    let dark = crate::scheme::declared(t, input, window)
        .unwrap_or_else(|| windows::env_of(k).unwrap_or_else(crate::env).dark);
    let accent = t
        .node(input)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.extras.accent_color.flatten());
    let pal = Palette::new(dark, accent);
    let popover = view(&state, alpha, &recents, pal);
    Some(
        crate::popup::Placed {
            anchor: input,
            fallback: state.fallback,
            gap: ANCHOR_GAP,
            child: popover,
            placed: note_placed,
        }
        .deferred(),
    )
}

fn note_placed(k: WinKey, input: u32, placed: Bounds<Pixels>) {
    with_open(|o| {
        if o.input == input && o.window == k {
            o.bounds = Some(placed);
        }
    });
}

pub(crate) fn refine(kind: VnHitKind, p: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
    let k = windows::key(window);
    if kind == VnHitKind::Up && SWALLOW_UP.with(Cell::get) == Some(k) {
        SWALLOW_UP.with(|s| s.set(None));
        crate::select::hit_nothing(kind, window);
        return true;
    }
    let Some((inside, dragging)) = OPEN.with(|o| {
        o.borrow()
            .as_ref()
            .filter(|o| o.window == k)
            .map(|o| (o.bounds.is_some_and(|b| b.contains(&p)), o.drag.is_some()))
    }) else {
        return false;
    };
    if inside || dragging {
        crate::select::hit_nothing(kind, window);
        return true;
    }
    match kind {
        VnHitKind::Down => {
            close_later(window, cx);
            SWALLOW_UP.with(|s| s.set(Some(k)));
            crate::select::hit_nothing(kind, window);
            true
        }
        VnHitKind::Scroll => {
            close_later(window, cx);
            false
        }
        VnHitKind::Up => false,
    }
}

/// Commits after the press that closed it, outside its hit test.
fn close_later(window: &mut Window, cx: &mut App) {
    if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
        window.defer(cx, move |window, cx| settle(o, true, window, cx));
    }
}

pub(crate) fn covers(window: &Window, p: Point<Pixels>) -> bool {
    let k = windows::key(window);
    OPEN.with(|o| {
        o.borrow().as_ref().is_some_and(|o| {
            o.window == k && (o.drag.is_some() || o.bounds.is_some_and(|b| b.contains(&p)))
        })
    })
}

/// The build root's capture-phase key listeners: an open picker takes every key.
pub(crate) fn bind_root<E: InteractiveElement>(e: E) -> E {
    e.capture_key_down(|ev: &KeyDownEvent, window, cx| {
        if open_in(windows::key(window)) {
            on_key(ev, window, cx);
            cx.stop_propagation();
        }
    })
    .capture_key_up(|_: &KeyUpEvent, window, cx| {
        if open_in(windows::key(window)) {
            cx.stop_propagation();
        }
    })
    // Tab is an action (`sk_elements::tab_nav`), which runs before key listeners.
    .capture_action(|_: &sk_elements::FocusNext, window, cx| {
        if open_in(windows::key(window)) {
            cycle(true, window, cx);
            cx.stop_propagation();
        }
    })
    .capture_action(|_: &sk_elements::FocusPrev, window, cx| {
        if open_in(windows::key(window)) {
            cycle(false, window, cx);
            cx.stop_propagation();
        }
    })
}

fn cycle(forward: bool, window: &mut Window, cx: &mut App) {
    let Some((part, input)) = with_open(|o| {
        o.keyboard = true;
        (o.part, o.input)
    }) else {
        return;
    };
    let alpha = tree::read_tree(|t| color::has_alpha(t, input));
    let order = parts(alpha, !recents().is_empty());
    let at = order.iter().position(|&p| p == part).unwrap_or(0) as isize;
    let next = (at + if forward { 1 } else { -1 }).rem_euclid(order.len() as isize);
    focus(order[next as usize], window, cx);
}

fn parts(alpha: bool, recents: bool) -> Vec<Part> {
    let mut v = vec![Part::Square, Part::Hue];
    if alpha {
        v.push(Part::Alpha);
    }
    v.push(Part::Hex);
    if recents {
        v.push(Part::Recent);
    }
    v
}

fn hex_digits(c: [f32; 4], alpha: bool) -> String {
    let [r, g, b, a] = c.map(color::byte);
    if alpha && a < 255 {
        format!("{r:02X}{g:02X}{b:02X}{a:02X}")
    } else {
        format!("{r:02X}{g:02X}{b:02X}")
    }
}

fn focus(part: Part, window: &mut Window, cx: &mut App) {
    let recents = recents();
    with_open(|o| {
        o.part = part;
        o.hex = (part == Part::Hex).then(|| {
            let alpha = tree::read_tree(|t| color::has_alpha(t, o.input));
            HexEdit {
                text: hex_digits(o.color.rgba(), alpha),
                all: true,
            }
        });
        if part == Part::Recent {
            let now = o.color.rgba().map(color::byte);
            o.recent = recents
                .iter()
                .position(|c| c.map(color::byte) == now)
                .unwrap_or(0);
        }
    });
    repaint(window, cx);
}

fn pick_recent(ix: usize, window: &mut Window, cx: &mut App) {
    let Some(&c) = recents().get(ix) else { return };
    with_open(|o| {
        o.part = Part::Recent;
        o.recent = ix;
        o.hex = None;
    });
    apply(|h| *h = Hsva::from_rgba(c, h.h), window, cx);
}

fn on_key(ev: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let Some((part, input, recent)) = with_open(|o| {
        o.keyboard = true;
        (o.part, o.input, o.recent)
    }) else {
        return;
    };
    let alpha = tree::read_tree(|t| color::has_alpha(t, input));
    let count = recents().len();
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    match key {
        "escape" => return finish(false, window, cx),
        "enter" => return finish(true, window, cx),
        "tab" => return cycle(!m.shift, window, cx),
        _ => {}
    }
    if part == Part::Hex {
        return hex_key(ev, alpha, window, cx);
    }
    if m.control || m.alt || m.platform {
        return repaint(window, cx);
    }
    let step = if m.shift { 0.1 } else { 0.01 };
    let degrees = if m.shift { 10.0 / 360.0 } else { 1.0 / 360.0 };
    match part {
        Part::Square => match key {
            "left" => apply(|c| c.s -= step, window, cx),
            "right" => apply(|c| c.s += step, window, cx),
            "up" => apply(|c| c.v += step, window, cx),
            "down" => apply(|c| c.v -= step, window, cx),
            "pageup" => apply(|c| c.v += 0.1, window, cx),
            "pagedown" => apply(|c| c.v -= 0.1, window, cx),
            _ => repaint(window, cx),
        },
        Part::Hue => match key {
            "left" | "down" => apply(|c| c.h -= degrees, window, cx),
            "right" | "up" => apply(|c| c.h += degrees, window, cx),
            "pagedown" => apply(|c| c.h -= 10.0 / 360.0, window, cx),
            "pageup" => apply(|c| c.h += 10.0 / 360.0, window, cx),
            "home" => apply(|c| c.h = 0.0, window, cx),
            "end" => apply(|c| c.h = 1.0, window, cx),
            _ => repaint(window, cx),
        },
        Part::Alpha => match key {
            "left" | "down" => apply(|c| c.a -= step, window, cx),
            "right" | "up" => apply(|c| c.a += step, window, cx),
            "pagedown" => apply(|c| c.a -= 0.1, window, cx),
            "pageup" => apply(|c| c.a += 0.1, window, cx),
            "home" => apply(|c| c.a = 0.0, window, cx),
            "end" => apply(|c| c.a = 1.0, window, cx),
            _ => repaint(window, cx),
        },
        Part::Recent if count > 0 => {
            let to = match key {
                "left" | "up" => recent.saturating_sub(1),
                "right" | "down" => (recent + 1).min(count - 1),
                "home" => 0,
                "end" => count - 1,
                "space" => recent,
                _ => return repaint(window, cx),
            };
            pick_recent(to, window, cx);
        }
        _ => repaint(window, cx),
    }
}

fn hex_key(ev: &KeyDownEvent, alpha: bool, window: &mut Window, cx: &mut App) {
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    let current = with_open(|o| hex_digits(o.color.rgba(), alpha)).unwrap_or_default();
    let mut paste = None;
    if m.control || m.platform {
        match key {
            "a" => {
                with_open(|o| {
                    o.hex
                        .get_or_insert(HexEdit {
                            text: current,
                            all: true,
                        })
                        .all = true
                });
            }
            "c" | "x" => cx.write_to_clipboard(ClipboardItem::new_string(format!(
                "#{}",
                current.to_ascii_lowercase()
            ))),
            "v" => paste = cx.read_from_clipboard().and_then(|c| c.text()),
            _ => {}
        }
        if let Some(text) = paste {
            let text = text.trim();
            let parsed = color::parse(text).or_else(|| color::parse(&format!("#{text}")));
            if let Some(c) = parsed {
                apply(|h| *h = Hsva::from_rgba(c, h.h), window, cx);
                let digits = with_open(|o| hex_digits(o.color.rgba(), alpha)).unwrap_or_default();
                with_open(|o| {
                    o.hex = Some(HexEdit {
                        text: digits,
                        all: false,
                    })
                });
            }
        }
        return repaint(window, cx);
    }
    let max = if alpha { 8 } else { 6 };
    let typed = ev
        .keystroke
        .key_char
        .as_deref()
        .and_then(|s| {
            let mut chars = s.chars();
            chars.next().filter(|_| chars.next().is_none())
        })
        .filter(char::is_ascii_hexdigit);
    let complete = with_open(|o| {
        let edit = o.hex.get_or_insert(HexEdit {
            text: current.clone(),
            all: true,
        });
        match key {
            "backspace" | "delete" if edit.all => {
                edit.text.clear();
                edit.all = false;
            }
            "backspace" => {
                edit.text.pop();
            }
            "left" | "right" | "home" | "end" => edit.all = false,
            _ => {
                if let Some(ch) = typed {
                    if edit.all {
                        edit.text.clear();
                        edit.all = false;
                    }
                    if edit.text.len() < max {
                        edit.text.push(ch.to_ascii_uppercase());
                        let n = edit.text.len();
                        if n == 3 || n == 6 || (alpha && (n == 4 || n == 8)) {
                            return color::parse(&format!("#{}", edit.text));
                        }
                    }
                }
            }
        }
        None
    })
    .flatten();
    match complete {
        Some(c) => apply(|h| *h = Hsva::from_rgba(c, h.h), window, cx),
        None => repaint(window, cx),
    }
}

fn begin(part: Part, window: &mut Window, cx: &mut App) {
    with_open(|o| {
        o.part = part;
        o.hex = None;
        o.drag = Some(part);
        o.keyboard = false;
    });
    repaint(window, cx);
}

fn dragging() -> Option<Part> {
    OPEN.with(|o| o.borrow().as_ref().and_then(|o| o.drag))
}

fn drag_to(part: Part, b: Bounds<Pixels>, p: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let (x, y) = (f32::from(p.x - b.origin.x), f32::from(p.y - b.origin.y));
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let along = ((x - TRACK / 2.0) / (w - TRACK).max(1.0)).clamp(0.0, 1.0);
    match part {
        Part::Square => apply(
            |c| {
                c.s = (x / w).clamp(0.0, 1.0);
                c.v = 1.0 - (y / h).clamp(0.0, 1.0);
            },
            window,
            cx,
        ),
        Part::Hue => apply(|c| c.h = along, window, cx),
        Part::Alpha => apply(|c| c.a = along, window, cx),
        Part::Hex | Part::Recent => {}
    }
}

/// A part the pointer drags: a press in `b` starts, moves anywhere follow, the release ends.
fn track(part: Part, b: Bounds<Pixels>, window: &mut Window) {
    window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
        if phase.bubble() && ev.button == MouseButton::Left && b.contains(&ev.position) {
            begin(part, window, cx);
            drag_to(part, b, ev.position, window, cx);
        }
    });
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
        if phase.bubble() && dragging() == Some(part) {
            drag_to(part, b, ev.position, window, cx);
        }
    });
    window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
        if phase.bubble() && ev.button == MouseButton::Left && dragging() == Some(part) {
            drag_to(part, b, ev.position, window, cx);
            with_open(|o| o.drag = None);
        }
    });
}

#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    surface: Hsla,
    stroke: Hsla,
    shadow: Hsla,
    text: Hsla,
    text2: Hsla,
    field: Hsla,
    field_focus: Hsla,
    field_stroke: Hsla,
    underline: Hsla,
    hairline: Hsla,
    accent: Hsla,
    on_accent: Hsla,
    ring: (Hsla, Hsla),
}

impl Palette {
    fn new(dark: bool, accent: Option<Hsla>) -> Self {
        let white = |a: f32| gpui::hsla(0.0, 0.0, 1.0, a);
        let black = |a: f32| gpui::hsla(0.0, 0.0, 0.0, a);
        if dark {
            let accent = accent.unwrap_or_else(|| gpui::rgb(0x60cdff).into());
            Palette {
                dark,
                surface: gpui::rgb(0x2c2c2c).into(),
                stroke: white(0.09),
                shadow: black(0.36),
                text: white(1.0),
                text2: white(0.786),
                field: white(0.061),
                field_focus: gpui::rgb(0x1f1f1f).into(),
                field_stroke: white(0.07),
                underline: white(0.54),
                hairline: white(0.14),
                accent,
                on_accent: on(accent),
                ring: (white(1.0), black(0.7)),
            }
        } else {
            let accent = accent.unwrap_or_else(|| gpui::rgb(0x005fb8).into());
            Palette {
                dark,
                surface: gpui::rgb(0xf9f9f9).into(),
                stroke: black(0.1),
                shadow: black(0.14),
                text: black(0.896),
                text2: black(0.606),
                field: white(0.7),
                field_focus: white(1.0),
                field_stroke: black(0.0578),
                underline: black(0.45),
                hairline: black(0.1),
                accent,
                on_accent: on(accent),
                ring: (black(0.896), white(0.7)),
            }
        }
    }
}

fn on(fill: Hsla) -> Hsla {
    if fill.l > 0.6 {
        gpui::black()
    } else {
        gpui::white()
    }
}

struct View {
    color: Hsva,
    part: Part,
    hex: Option<HexEdit>,
    recent: usize,
    keyboard: bool,
    fallback: windows::Rect,
}

fn ring(b: Bounds<Pixels>, radius: f32, pal: Palette, window: &mut Window) {
    let (outer, inner) = pal.ring;
    for (grow, width, color) in [(3.0, 2.0, outer), (1.0, 1.0, inner)] {
        window.paint_quad(gpui::quad(
            b.dilate(px(grow)),
            px(radius + grow),
            gpui::transparent_black(),
            px(width),
            color,
            gpui::BorderStyle::Solid,
        ));
    }
}

fn thumb(center: Point<Pixels>, fill: [f32; 4], window: &mut Window) {
    let r = THUMB / 2.0;
    let b = Bounds::new(
        gpui::point(center.x - px(r), center.y - px(r)),
        gpui::size(px(THUMB), px(THUMB)),
    );
    window.paint_quad(gpui::quad(
        b.dilate(px(1.0)),
        px(r + 1.0),
        gpui::transparent_black(),
        px(1.0),
        gpui::hsla(0.0, 0.0, 0.0, 0.3),
        gpui::BorderStyle::Solid,
    ));
    window.paint_quad(gpui::quad(
        b,
        px(r),
        gpui::white(),
        px(0.0),
        gpui::transparent_black(),
        gpui::BorderStyle::Solid,
    ));
    window.paint_quad(gpui::quad(
        b.dilate(px(-3.0)),
        px(r - 3.0),
        color::hsla(fill),
        px(0.0),
        gpui::transparent_black(),
        gpui::BorderStyle::Solid,
    ));
}

fn plain(b: Bounds<Pixels>, radius: f32, fill: impl Into<gpui::Background>) -> gpui::PaintQuad {
    gpui::quad(
        b,
        px(radius),
        fill,
        px(0.0),
        gpui::transparent_black(),
        gpui::BorderStyle::Solid,
    )
}

fn hairline(b: Bounds<Pixels>, radius: f32, color: Hsla) -> gpui::PaintQuad {
    gpui::quad(
        b,
        px(radius),
        gpui::transparent_black(),
        px(1.0),
        color,
        gpui::BorderStyle::Solid,
    )
}

fn square(c: Hsva, focused: bool, pal: Palette) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            let r = 6.0;
            let hue = Hsva {
                s: 1.0,
                v: 1.0,
                a: 1.0,
                ..c
            };
            window.paint_quad(plain(b, r, color::hsla(hue.rgba())));
            window.paint_quad(plain(
                b,
                r,
                linear_gradient(
                    90.0,
                    linear_color_stop(gpui::white(), 0.0),
                    linear_color_stop(gpui::hsla(0.0, 0.0, 1.0, 0.0), 1.0),
                ),
            ));
            window.paint_quad(plain(
                b,
                r,
                linear_gradient(
                    180.0,
                    linear_color_stop(gpui::hsla(0.0, 0.0, 0.0, 0.0), 0.0),
                    linear_color_stop(gpui::black(), 1.0),
                ),
            ));
            window.paint_quad(hairline(b, r, pal.hairline));
            let at = gpui::point(
                b.origin.x + b.size.width * c.s,
                b.origin.y + b.size.height * (1.0 - c.v),
            );
            thumb(at, Hsva { a: 1.0, ..c }.rgba(), window);
            if focused {
                ring(b, r, pal, window);
            }
            track(Part::Square, b, window);
        },
    )
    .w(px(CONTENT))
    .h(px(SQUARE_H))
    .into_any_element()
}

fn slider(part: Part, c: Hsva, focused: bool, pal: Palette) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            let r = TRACK / 2.0;
            let t = Bounds::new(
                gpui::point(b.origin.x, b.origin.y + (b.size.height - px(TRACK)) / 2.0),
                gpui::size(b.size.width, px(TRACK)),
            );
            let (at, fill) = if part == Part::Hue {
                let stops = [
                    0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000,
                ]
                .iter()
                .enumerate()
                .map(|(i, &hex)| linear_color_stop(gpui::rgb(hex), i as f32 / 6.0))
                .collect::<Vec<_>>();
                window.paint_quad(plain(t, r, linear_gradient_stops(90.0, &stops)));
                let hue = Hsva {
                    s: 1.0,
                    v: 1.0,
                    a: 1.0,
                    ..c
                };
                (c.h, hue.rgba())
            } else {
                color::paint_checker(t, r, pal.dark, window);
                color::mask_corners(t, r, pal.surface, window);
                let [cr, cg, cb, _] = c.rgba();
                window.paint_quad(plain(
                    t,
                    r,
                    linear_gradient(
                        90.0,
                        linear_color_stop(color::hsla([cr, cg, cb, 0.0]), 0.0),
                        linear_color_stop(color::hsla([cr, cg, cb, 1.0]), 1.0),
                    ),
                ));
                (c.a, c.rgba())
            };
            window.paint_quad(hairline(t, r, pal.hairline));
            let x = t.origin.x + px(r) + (t.size.width - px(TRACK)) * at;
            thumb(gpui::point(x, t.origin.y + px(r)), fill, window);
            if focused {
                ring(t, r, pal, window);
            }
            track(part, b, window);
        },
    )
    .w(px(CONTENT))
    .h(px(THUMB))
    .into_any_element()
}

/// A color sample: a checkerboard shows through a translucent one.
fn chip(c: [f32; 4], size: f32, radius: f32, focused: bool, pal: Palette) -> gpui::Canvas<()> {
    let mut chip = canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            if c[3] < 1.0 {
                color::paint_checker(b, radius.max(4.0), pal.dark, window);
                color::mask_corners(b, radius, pal.surface, window);
            }
            window.paint_quad(plain(b, radius, color::hsla(c)));
            window.paint_quad(hairline(b, radius, pal.hairline));
            if focused {
                ring(b, radius, pal, window);
            }
        },
    );
    chip.style().size.width = Some(px(size).into());
    chip.style().size.height = Some(px(size).into());
    chip.style().flex_shrink = Some(0.0);
    chip
}

fn hex_field(v: &View, alpha: bool, pal: Palette) -> AnyElement {
    let focused = v.part == Part::Hex;
    let (text, all) = match &v.hex {
        Some(e) if focused => (e.text.clone(), e.all),
        _ => (hex_digits(v.color.rgba(), alpha), false),
    };
    let digits = if all && !text.is_empty() {
        div()
            .bg(pal.accent)
            .text_color(pal.on_accent)
            .rounded(px(2.0))
            .child(text)
    } else {
        div().child(text)
    };
    let mut line = div()
        .flex()
        .items_center()
        .child(div().text_color(pal.text2).child("#"))
        .child(digits);
    if focused && !all {
        line = line.child(div().w(px(1.0)).h(px(16.0)).bg(pal.text));
    }
    div()
        .id("sk-color-picker-hex")
        .relative()
        .flex_1()
        .h(px(FIELD_H))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(pal.field_stroke)
        .bg(if focused { pal.field_focus } else { pal.field })
        .font_family(if cfg!(windows) { "Consolas" } else { "Menlo" })
        .child(line)
        .child(
            div()
                .absolute()
                .bottom(px(0.0))
                .left(px(3.0))
                .right(px(3.0))
                .h(px(if focused { 2.0 } else { 1.0 }))
                .bg(if focused { pal.accent } else { pal.underline }),
        )
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            with_open(|o| o.keyboard = false);
            focus(Part::Hex, window, cx);
        })
        .into_any_element()
}

fn view(v: &View, alpha: bool, recents: &[[f32; 4]], pal: Palette) -> AnyElement {
    let ring_on = |p: Part| v.keyboard && v.part == p;
    let shown = if alpha {
        v.color.rgba()
    } else {
        Hsva { a: 1.0, ..v.color }.rgba()
    };
    let mut body = div()
        .id("sk-color-picker")
        .occlude()
        .flex()
        .flex_col()
        .gap(px(GAP))
        .p(px(PAD))
        .w(px(CONTENT + 2.0 * PAD + 2.0))
        .bg(pal.surface)
        .border_1()
        .border_color(pal.stroke)
        .rounded(px(RADIUS))
        .font_family(".SystemUIFont")
        .text_size(px(13.0))
        .line_height(px(18.0))
        .text_color(pal.text)
        .child(square(v.color, ring_on(Part::Square), pal))
        .child(slider(Part::Hue, v.color, ring_on(Part::Hue), pal));
    body.style().box_shadow = Some(vec![
        BoxShadow::new(px(0.0), px(8.0), pal.shadow).blur_radius(px(16.0)),
        BoxShadow::new(px(0.0), px(1.0), pal.shadow.opacity(0.6)).blur_radius(px(3.0)),
    ]);
    if alpha {
        body = body.child(slider(Part::Alpha, v.color, ring_on(Part::Alpha), pal));
    }
    body = body.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(chip(shown, FIELD_H, 4.0, false, pal))
            .child(hex_field(v, alpha, pal)),
    );
    if !recents.is_empty() {
        let gap = (CONTENT - RECENT_MAX as f32 * CHIP) / (RECENT_MAX - 1) as f32;
        body = body.child(
            div()
                .flex()
                .gap(px(gap))
                .children(recents.iter().enumerate().map(|(i, &c)| {
                    let c = if alpha { c } else { [c[0], c[1], c[2], 1.0] };
                    div()
                        .id(("sk-color-picker-recent", i))
                        .child(chip(
                            c,
                            CHIP,
                            4.0,
                            ring_on(Part::Recent) && v.recent == i,
                            pal,
                        ))
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            with_open(|o| o.keyboard = false);
                            pick_recent(i, window, cx);
                        })
                })),
        );
    }
    body.into_any_element()
}

fn recents() -> Vec<[f32; 4]> {
    RECENTS.with(|r| {
        r.borrow_mut()
            .get_or_insert_with(|| {
                let file = RECENTS_FILE.lock().ok().and_then(|f| f.clone());
                file.and_then(|f| std::fs::read_to_string(f).ok())
                    .map(|s| parse_recents(&s))
                    .unwrap_or_default()
            })
            .clone()
    })
}

fn parse_recents(text: &str) -> Vec<[f32; 4]> {
    text.lines()
        .filter_map(|l| color::parse(l.trim()))
        .take(RECENT_MAX)
        .collect()
}

fn format_recents(list: &[[f32; 4]]) -> String {
    list.iter()
        .map(|c| {
            let [r, g, b, a] = c.map(color::byte);
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}\n")
        })
        .collect()
}

fn remember(c: [f32; 4]) {
    let mut list = recents();
    let bytes = c.map(color::byte);
    list.retain(|x| x.map(color::byte) != bytes);
    list.insert(0, c);
    list.truncate(RECENT_MAX);
    RECENTS.with(|r| *r.borrow_mut() = Some(list.clone()));
    let file = RECENTS_FILE.lock().ok().and_then(|f| f.clone());
    if let Some(file) = file {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&file, format_recents(&list)) {
            log::warn!(
                "color picker: could not save recent colors to {}: {e}",
                file.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips_and_a_gray_keeps_the_hue_it_had() {
        for rgba in [
            [1.0, 0.0, 0.0, 1.0],
            [0.2, 0.4, 0.6, 0.5],
            [0.9, 0.9, 0.1, 1.0],
            [0.5, 0.0, 1.0, 1.0],
        ] {
            let back = Hsva::from_rgba(rgba, 0.0).rgba();
            for (a, b) in rgba.iter().zip(back) {
                assert!((a - b).abs() < 1e-5, "{rgba:?} -> {back:?}");
            }
        }
        assert_eq!(Hsva::from_rgba([0.5, 0.5, 0.5, 1.0], 0.25).h, 0.25);
        assert_eq!(Hsva::from_rgba([0.0, 0.0, 0.0, 1.0], 0.7).h, 0.7);
    }

    #[test]
    fn recents_are_written_one_per_line_and_read_back_capped() {
        let list = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.501_960_8, 1.0, 0.501_960_8]];
        let text = format_recents(&list);
        assert_eq!(text, "#ff0000ff\n#0080ff80\n");
        let bytes = |l: &[[f32; 4]]| l.iter().map(|c| c.map(color::byte)).collect::<Vec<_>>();
        assert_eq!(bytes(&parse_recents(&text)), bytes(&list));
        let many = "#000\n".repeat(20) + "junk\n";
        assert_eq!(parse_recents(&many).len(), RECENT_MAX);
    }
}
