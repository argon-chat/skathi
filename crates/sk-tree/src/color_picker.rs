//! `<input type=color>`'s picker: a popover drawn in the input's window, under the input. The
//! runtime opens it (`__sk_color_picker`) once the input's `showpicker` event was not cancelled.
//! While it is open it takes the keyboard and the pointer over it. Each edit writes `value` and
//! fires `input`; Enter, a press outside or the window going inactive commits with `change`;
//! Escape puts the old value back.
//!
//! It edits a base color in the input's gamma-encoded space (sRGB, or Display P3 with
//! `colorspace`) and, with `hdr`, an exposure in stops; [`color::join`] makes the value.

use std::cell::{Cell, RefCell};
use std::f32::consts::{FRAC_PI_2, SQRT_2, TAU};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use gpui::prelude::*;
use gpui::{
    canvas, div, linear_color_stop, linear_gradient, linear_gradient_stops, px, AnyElement, App,
    Bounds, BoxShadow, ClipboardItem, Hsla, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, VnHitKind, Window,
};
use sk_css::RgbSpace;

use crate::color::{self, Mode, MAX_STOPS};
use crate::popup::Slot;
use crate::tree::{self, Tree};
use crate::windows::{self, WinKey};
use crate::EventPayload;

const CONTENT: f32 = 216.0;
const PAD: f32 = 12.0;
const GAP: f32 = 12.0;
const SQUARE_H: f32 = 144.0;
const DISC: f32 = 196.0;
const RING: f32 = 18.0;
const TRACK: f32 = 12.0;
const THUMB: f32 = 18.0;
const FIELD_H: f32 = 32.0;
const TABS_H: f32 = 28.0;
const SEG_H: f32 = 24.0;
const PREVIEW_H: f32 = 24.0;
const CHIP: f32 = 20.0;
const RECENT_MAX: usize = 8;
const RADIUS: f32 = 8.0;
const ANCHOR_GAP: f32 = 4.0;
/// Around the popup window's panel: its shadow's reach.
const SHADOW_ROOM: f32 = 24.0;
const STOP_PRESETS: [i32; 7] = [-10, -5, -2, 0, 2, 5, 10];
const HUES: [u32; 7] = [
    0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000,
];

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

    /// HSL's saturation and lightness.
    fn sl(self) -> (f32, f32) {
        let l = self.v * (1.0 - self.s / 2.0);
        let s = if l <= 0.0 || l >= 1.0 {
            0.0
        } else {
            (self.v - l) / l.min(1.0 - l)
        };
        (s, l)
    }

    fn with_sl(self, s: f32, l: f32) -> Self {
        let v = l + s * l.min(1.0 - l);
        Hsva {
            s: if v > 0.0 { 2.0 * (1.0 - l / v) } else { 0.0 },
            v,
            ..self
        }
    }
}

/// OKLCH from RGB in `space`: lightness 0..=1, chroma, hue in degrees.
fn to_oklch(rgb: [f32; 3], space: RgbSpace) -> [f32; 3] {
    let [r, g, b] = sk_css::convert_rgb(rgb, space, RgbSpace::SrgbLinear);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    [
        lightness,
        (a * a + b * b).sqrt(),
        b.atan2(a).to_degrees().rem_euclid(360.0),
    ]
}

fn from_oklch([lightness, chroma, hue]: [f32; 3], space: RgbSpace) -> [f32; 3] {
    let (a, b) = (
        chroma * hue.to_radians().cos(),
        chroma * hue.to_radians().sin(),
    );
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    sk_css::convert_rgb(
        [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ],
        RgbSpace::SrgbLinear,
        space,
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Shape {
    #[default]
    Square,
    Wheel,
    Triangle,
}

impl Shape {
    const ALL: [Shape; 3] = [Shape::Square, Shape::Wheel, Shape::Triangle];

    fn name(self) -> &'static str {
        match self {
            Shape::Square => "square",
            Shape::Wheel => "wheel",
            Shape::Triangle => "triangle",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Format {
    #[default]
    Hex,
    Rgb,
    Hsl,
    Hsv,
    Oklch,
}

impl Format {
    const ALL: [Format; 5] = [
        Format::Hex,
        Format::Rgb,
        Format::Hsl,
        Format::Hsv,
        Format::Oklch,
    ];

    fn name(self) -> &'static str {
        match self {
            Format::Hex => "hex",
            Format::Rgb => "rgb",
            Format::Hsl => "hsl",
            Format::Hsv => "hsv",
            Format::Oklch => "oklch",
        }
    }

    fn labels(self) -> [&'static str; 3] {
        match self {
            Format::Hex => ["#", "", ""],
            Format::Rgb => ["R", "G", "B"],
            Format::Hsl => ["H", "S", "L"],
            Format::Hsv => ["H", "S", "V"],
            Format::Oklch => ["L", "C", "H"],
        }
    }

    fn fields(self, alpha: bool) -> usize {
        if self == Format::Hex {
            1
        } else {
            3 + alpha as usize
        }
    }

    /// A field's range and its arrow-key step.
    fn range(self, field: usize) -> (f32, f32, f32) {
        match (self, field) {
            (_, 3) => (0.0, 100.0, 1.0),
            (Format::Rgb, _) => (0.0, 255.0, 1.0),
            (Format::Hsl | Format::Hsv, 0) | (Format::Oklch, 2) => (0.0, 360.0, 1.0),
            (Format::Oklch, 1) => (0.0, 0.4, 0.01),
            _ => (0.0, 100.0, 1.0),
        }
    }
}

fn values(c: Hsva, format: Format, mode: Mode) -> [f32; 4] {
    let [r, g, b, a] = c.rgba();
    let [x, y, z] = match format {
        Format::Hex | Format::Rgb => [r, g, b].map(|v| v * 255.0),
        Format::Hsl => {
            let (s, l) = c.sl();
            [c.h * 360.0, s * 100.0, l * 100.0]
        }
        Format::Hsv => [c.h * 360.0, c.s * 100.0, c.v * 100.0],
        Format::Oklch => {
            let [l, chroma, hue] = to_oklch([r, g, b], mode.working());
            [l * 100.0, chroma, hue]
        }
    };
    [x, y, z, a * 100.0]
}

fn value_text(format: Format, field: usize, v: f32) -> String {
    match (format, field) {
        (Format::Oklch, 0) => format!("{v:.1}"),
        (Format::Oklch, 1) => format!("{v:.3}"),
        _ => format!("{}", v.round() as i32),
    }
}

fn set_value(c: &mut Hsva, format: Format, field: usize, v: f32, mode: Mode) {
    let (lo, hi, _) = format.range(field);
    let v = v.clamp(lo, hi);
    if field == 3 {
        c.a = v / 100.0;
        return;
    }
    match format {
        Format::Hex => {}
        Format::Rgb => {
            let mut rgba = c.rgba();
            rgba[field] = v / 255.0;
            *c = Hsva::from_rgba(rgba, c.h);
        }
        Format::Hsl => {
            let (s, l) = c.sl();
            *c = match field {
                0 => Hsva { h: v / 360.0, ..*c },
                1 => c.with_sl(v / 100.0, l),
                _ => c.with_sl(s, v / 100.0),
            };
        }
        Format::Hsv => match field {
            0 => c.h = v / 360.0,
            1 => c.s = v / 100.0,
            _ => c.v = v / 100.0,
        },
        Format::Oklch => {
            let [r, g, b, a] = c.rgba();
            let mut lch = to_oklch([r, g, b], mode.working());
            lch[field] = if field == 0 { v / 100.0 } else { v };
            let [r, g, b] = from_oklch(lch, mode.working()).map(|x| x.clamp(0.0, 1.0));
            *c = Hsva::from_rgba([r, g, b, a], c.h);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Shape,
    Square,
    Hue,
    Alpha,
    Intensity,
    Format,
    Field(usize),
    Recent,
}

#[derive(Clone)]
struct Edit {
    field: usize,
    text: String,
    /// Selected whole: the next character replaces it.
    all: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Recent {
    /// The base color in the space it was picked in.
    css: String,
    stops: i32,
}

#[derive(Clone, Default)]
struct Prefs {
    shape: Shape,
    format: Format,
    recents: Vec<Recent>,
}

struct Open {
    input: u32,
    window: WinKey,
    color: Hsva,
    stops: i32,
    original: String,
    /// The value this picker last wrote; any other value came from the app.
    written: String,
    part: Part,
    edit: Option<Edit>,
    recent: usize,
    drag: Option<Part>,
    bounds: Option<Bounds<Pixels>>,
    fallback: windows::Rect,
    keyboard: bool,
}

static SYSTEM: AtomicBool = AtomicBool::new(false);
static PREFS_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    static REQUEST: Cell<Option<u32>> = const { Cell::new(None) };
    static SWALLOW_UP: Cell<Option<WinKey>> = const { Cell::new(None) };
    static PREFS: RefCell<Option<Prefs>> = const { RefCell::new(None) };
}

/// `sk.config.json`'s `"colorPicker": "system"`: color inputs open the OS dialog instead.
pub fn use_system_dialog(on: bool) {
    SYSTEM.store(on, Ordering::Relaxed);
}

/// Where the recent colors and the picker's shape and format persist; `None` keeps them for the run.
pub fn set_recents_file(path: Option<PathBuf>) {
    *PREFS_FILE.lock().unwrap_or_else(|e| e.into_inner()) = path;
    PREFS.with(|r| r.borrow_mut().take());
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

fn mode_of(input: u32) -> Mode {
    tree::read_tree(|t| Mode::of(t, input))
}

/// Runs `f` in the window the picker belongs to, also for its popup window's handlers.
fn in_owner(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let k = OPEN
        .with(|o| o.borrow().as_ref().map(|o| o.window))
        .unwrap_or_else(|| windows::key(window));
    crate::popup::in_window(k, window, cx, f);
}

fn repaint(window: &mut Window, cx: &mut App) {
    in_owner(window, cx, |window, cx| {
        let root = windows::root_of(windows::key(window));
        tree::with_tree(|t| t.touch(root));
        crate::render::request_repaint(window, cx);
    });
}

fn write(input: u32, text: &str, commit: bool, window: &mut Window, cx: &mut App) {
    tree::with_tree(|t| {
        let key = t.intern("value");
        t.set_prop(input, key, Some(text.to_string()));
    });
    let text = text.to_string();
    in_owner(window, cx, move |window, cx| {
        crate::render::js_dispatch_app(
            input,
            tree::EV_INPUT,
            &EventPayload {
                key: Some(&text),
                b: if commit { 1.0 } else { 0.0 },
                ..Default::default()
            },
            window,
            cx,
        );
    });
}

/// Applies an edit, firing `input` when the value it serializes to moved.
fn apply_open(edit: impl FnOnce(&mut Open), window: &mut Window, cx: &mut App) {
    let wrote = with_open(|o| {
        edit(o);
        o.color = o.color.clamped();
        let mode = mode_of(o.input);
        o.stops = if mode.hdr {
            o.stops.clamp(-MAX_STOPS, MAX_STOPS)
        } else {
            0
        };
        let text = color::serialize(color::join(o.color.rgba(), o.stops, mode), mode);
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

fn apply(edit: impl FnOnce(&mut Hsva), window: &mut Window, cx: &mut App) {
    apply_open(|o| edit(&mut o.color), window, cx);
}

fn finish(commit: bool, window: &mut Window, cx: &mut App) {
    if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
        settle(o, commit, window, cx);
    }
}

fn settle(o: Open, commit: bool, window: &mut Window, cx: &mut App) {
    crate::popup::close(Slot::Picker, o.input, cx);
    let alive = tree::with_tree(|t| {
        if t.node(o.input).is_some() {
            let key = t.intern(":picker");
            t.set_prop(o.input, key, None);
        }
        usable(t, o.input).then(|| {
            (
                t.prop(o.input, "value").unwrap_or("").to_string(),
                Mode::of(t, o.input),
            )
        })
    });
    if let Some((current, mode)) = alive.filter(|(c, _)| *c != o.original) {
        if commit {
            let picked = if current == o.written {
                Some((o.color.rgba(), o.stops))
            } else {
                color::read(&current, mode)
            };
            if let Some((base, stops)) = picked {
                remember(recent_of(base, stops, mode));
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
            let (base, stops) =
                color::read(&value, Mode::of(t, id)).unwrap_or(([0.0, 0.0, 0.0, 1.0], 0));
            let mouse = window.mouse_position();
            let fresh = Open {
                input: id,
                window: k,
                color: Hsva::from_rgba(base, 0.0),
                stops,
                original: value.clone(),
                written: value,
                part: Part::Square,
                edit: None,
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
    if !usable(t, input)
        || !window.is_window_active()
        || crate::popup::owner_moved(Slot::Picker, window)
    {
        if let Some(o) = OPEN.with(|o| o.borrow_mut().take()) {
            window.defer(cx, move |window, cx| settle(o, true, window, cx));
        }
        return None;
    }
    let mode = Mode::of(t, input);
    let value = t.prop(input, "value").unwrap_or("");
    let prefs = prefs();
    let state = with_open(|o| {
        if o.written != value {
            o.written = value.to_string();
            if let Some((base, stops)) = color::read(value, mode) {
                o.color = Hsva::from_rgba(base, o.color.h);
                o.stops = stops;
                o.edit = None;
            }
        }
        if !mode.alpha {
            o.color.a = 1.0;
        }
        if !mode.hdr {
            o.stops = 0;
        }
        if !parts(mode, !prefs.recents.is_empty(), prefs.format).contains(&o.part) {
            o.part = Part::Square;
            o.edit = None;
        }
        View {
            color: o.color,
            stops: o.stops,
            part: o.part,
            edit: o.edit.clone(),
            recent: o.recent.min(prefs.recents.len().saturating_sub(1)),
            keyboard: o.keyboard,
            fallback: o.fallback,
            mode,
            shape: prefs.shape,
            format: prefs.format,
        }
    })?;
    let dark = crate::scheme::declared(t, input, window)
        .unwrap_or_else(|| windows::env_of(k).unwrap_or_else(crate::env).dark);
    let accent = t
        .node(input)
        .and_then(|n| n.computed.as_ref())
        .and_then(|cs| cs.extras.accent_color.flatten());
    let pal = Palette::new(dark, accent);
    let fallback = state.fallback;
    let recents = prefs.recents;
    Some(
        crate::popup::Popup {
            slot: Slot::Picker,
            anchor: input,
            fallback,
            gap: ANCHOR_GAP,
            margin: SHADOW_ROOM,
            inherit_text: false,
            build: Rc::new(move || view(&state, &recents, pal)),
            placed: note_placed,
        }
        .into_element(),
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
    let p = prefs();
    let order = parts(mode_of(input), !p.recents.is_empty(), p.format);
    let at = order.iter().position(|&p| p == part).unwrap_or(0) as isize;
    let next = (at + if forward { 1 } else { -1 }).rem_euclid(order.len() as isize);
    focus(order[next as usize], window, cx);
}

fn parts(mode: Mode, recents: bool, format: Format) -> Vec<Part> {
    let mut v = vec![Part::Shape, Part::Square, Part::Hue];
    if mode.alpha {
        v.push(Part::Alpha);
    }
    if mode.hdr {
        v.push(Part::Intensity);
    }
    v.push(Part::Format);
    v.extend((0..format.fields(mode.alpha)).map(Part::Field));
    if recents {
        v.push(Part::Recent);
    }
    v
}

/// The hex digits of the color as sRGB, the space hex means.
fn hex_digits(c: Hsva, mode: Mode) -> String {
    let [r, g, b, a] = c.rgba();
    let [r, g, b] = color::display([r, g, b], mode.working()).map(color::byte);
    let a = color::byte(a);
    if mode.alpha && a < 255 {
        format!("{r:02X}{g:02X}{b:02X}{a:02X}")
    } else {
        format!("{r:02X}{g:02X}{b:02X}")
    }
}

fn field_text(c: Hsva, format: Format, field: usize, mode: Mode) -> String {
    if format == Format::Hex {
        return hex_digits(c, mode);
    }
    value_text(format, field, values(c, format, mode)[field])
}

fn recent_of(base: [f32; 4], stops: i32, mode: Mode) -> Recent {
    let base = if mode.alpha {
        base
    } else {
        [base[0], base[1], base[2], 1.0]
    };
    Recent {
        css: color::css_in(base, mode.working()),
        stops,
    }
}

fn focus(part: Part, window: &mut Window, cx: &mut App) {
    let p = prefs();
    with_open(|o| {
        let mode = mode_of(o.input);
        o.part = part;
        o.edit = match part {
            Part::Field(field) => Some(Edit {
                field,
                text: field_text(o.color, p.format, field, mode),
                all: true,
            }),
            _ => None,
        };
        if part == Part::Recent {
            let now = recent_of(o.color.rgba(), o.stops, mode);
            o.recent = p.recents.iter().position(|r| *r == now).unwrap_or(0);
        }
    });
    repaint(window, cx);
}

fn pick_recent(ix: usize, window: &mut Window, cx: &mut App) {
    let Some(recent) = prefs().recents.get(ix).cloned() else {
        return;
    };
    apply_open(
        |o| {
            let mode = mode_of(o.input);
            o.part = Part::Recent;
            o.recent = ix;
            o.edit = None;
            if let Some(c) = sk_css::parse_color_in(&recent.css, mode.working()) {
                o.color = Hsva::from_rgba(c.map(|v| v.clamp(0.0, 1.0)), o.color.h);
                o.stops = recent.stops;
            }
        },
        window,
        cx,
    );
}

fn set_shape(shape: Shape, window: &mut Window, cx: &mut App) {
    with_prefs(|p| p.shape = shape);
    save_prefs();
    with_open(|o| {
        o.part = Part::Shape;
        o.edit = None;
    });
    repaint(window, cx);
}

fn set_format(format: Format, window: &mut Window, cx: &mut App) {
    with_prefs(|p| p.format = format);
    save_prefs();
    with_open(|o| {
        o.part = Part::Format;
        o.edit = None;
    });
    repaint(window, cx);
}

fn set_stops(stops: i32, window: &mut Window, cx: &mut App) {
    apply_open(
        |o| {
            o.part = Part::Intensity;
            o.edit = None;
            o.stops = stops;
        },
        window,
        cx,
    );
}

fn step<T: Copy + PartialEq>(all: &[T], now: T, key: &str) -> Option<T> {
    let at = all.iter().position(|&x| x == now).unwrap_or(0);
    let to = match key {
        "left" | "up" => at.saturating_sub(1),
        "right" | "down" => (at + 1).min(all.len() - 1),
        "home" => 0,
        "end" => all.len() - 1,
        _ => return None,
    };
    Some(all[to])
}

fn on_key(ev: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let Some((part, input, recent, stops)) = with_open(|o| {
        o.keyboard = true;
        (o.part, o.input, o.recent, o.stops)
    }) else {
        return;
    };
    let mode = mode_of(input);
    let p = prefs();
    let count = p.recents.len();
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    match key {
        "escape" => return finish(false, window, cx),
        "enter" => return finish(true, window, cx),
        "tab" => return cycle(!m.shift, window, cx),
        _ => {}
    }
    if let Part::Field(field) = part {
        return field_key(ev, field, p.format, mode, window, cx);
    }
    if m.control || m.alt || m.platform {
        return repaint(window, cx);
    }
    let step_by = if m.shift { 0.1 } else { 0.01 };
    let degrees = if m.shift { 10.0 / 360.0 } else { 1.0 / 360.0 };
    match part {
        Part::Square => match key {
            "left" => apply(|c| c.s -= step_by, window, cx),
            "right" => apply(|c| c.s += step_by, window, cx),
            "up" => apply(|c| c.v += step_by, window, cx),
            "down" => apply(|c| c.v -= step_by, window, cx),
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
            "left" | "down" => apply(|c| c.a -= step_by, window, cx),
            "right" | "up" => apply(|c| c.a += step_by, window, cx),
            "pagedown" => apply(|c| c.a -= 0.1, window, cx),
            "pageup" => apply(|c| c.a += 0.1, window, cx),
            "home" => apply(|c| c.a = 0.0, window, cx),
            "end" => apply(|c| c.a = 1.0, window, cx),
            _ => repaint(window, cx),
        },
        Part::Intensity => {
            let to = match key {
                "left" | "down" => stops - 1,
                "right" | "up" => stops + 1,
                "pagedown" => stops - 5,
                "pageup" => stops + 5,
                "home" => -MAX_STOPS,
                "end" => MAX_STOPS,
                _ => return repaint(window, cx),
            };
            set_stops(to, window, cx);
        }
        Part::Shape => match step(&Shape::ALL, p.shape, key) {
            Some(s) => set_shape(s, window, cx),
            None => repaint(window, cx),
        },
        Part::Format => match step(&Format::ALL, p.format, key) {
            Some(f) => set_format(f, window, cx),
            None => repaint(window, cx),
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

fn typed(ev: &KeyDownEvent) -> Option<char> {
    ev.keystroke.key_char.as_deref().and_then(|s| {
        let mut chars = s.chars();
        chars.next().filter(|_| chars.next().is_none())
    })
}

/// A pasted CSS color (or bare hex digits) becomes the base; with `hdr`, a value past 1 brings
/// its stops too.
fn paste(text: &str, mode: Mode, window: &mut Window, cx: &mut App) -> bool {
    let text = text.trim();
    let Some((base, stops)) =
        color::read(text, mode).or_else(|| color::read(&format!("#{text}"), mode))
    else {
        return false;
    };
    apply_open(
        |o| {
            o.color = Hsva::from_rgba(base, o.color.h);
            if stops != 0 {
                o.stops = stops;
            }
        },
        window,
        cx,
    );
    true
}

fn field_key(
    ev: &KeyDownEvent,
    field: usize,
    format: Format,
    mode: Mode,
    window: &mut Window,
    cx: &mut App,
) {
    let m = &ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();
    let current = with_open(|o| field_text(o.color, format, field, mode)).unwrap_or_default();
    if m.control || m.platform {
        match key {
            "a" => {
                with_open(|o| {
                    o.edit = Some(Edit {
                        field,
                        text: current,
                        all: true,
                    })
                });
            }
            "c" | "x" => {
                let text = if format == Format::Hex {
                    with_open(|o| format!("#{}", hex_digits(o.color, mode).to_ascii_lowercase()))
                } else {
                    with_open(|o| o.written.clone())
                };
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            "v" => {
                let text = cx.read_from_clipboard().and_then(|c| c.text());
                if text.is_some_and(|t| paste(&t, mode, window, cx)) {
                    with_open(|o| {
                        o.edit = Some(Edit {
                            field,
                            text: field_text(o.color, format, field, mode),
                            all: false,
                        })
                    });
                }
            }
            _ => {}
        }
        return repaint(window, cx);
    }
    if format == Format::Hex {
        return hex_key(ev, current, mode, window, cx);
    }
    let (_, _, unit) = format.range(field);
    let step_by = if m.shift { unit * 10.0 } else { unit };
    let ch = typed(ev).filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-');
    let set = with_open(|o| {
        let edit = o.edit.get_or_insert(Edit {
            field,
            text: current.clone(),
            all: true,
        });
        if edit.field != field {
            *edit = Edit {
                field,
                text: current.clone(),
                all: true,
            };
        }
        match key {
            "up" | "pageup" | "down" | "pagedown" => {
                let sign = if key.ends_with("up") { 1.0 } else { -1.0 };
                let big = if key.starts_with("page") { 10.0 } else { 1.0 };
                o.edit = None;
                let now = values(o.color, format, mode)[field];
                return Some(now + sign * big * step_by);
            }
            "backspace" | "delete" if edit.all => {
                edit.text.clear();
                edit.all = false;
            }
            "backspace" => {
                edit.text.pop();
            }
            "left" | "right" | "home" | "end" => edit.all = false,
            _ => {
                let Some(ch) = ch else { return None };
                if edit.all {
                    edit.text.clear();
                    edit.all = false;
                }
                if edit.text.len() < 7 {
                    edit.text.push(ch);
                }
            }
        }
        edit.text.parse::<f32>().ok().filter(|v| v.is_finite())
    })
    .flatten();
    match set {
        Some(v) => apply(|c| set_value(c, format, field, v, mode), window, cx),
        None => repaint(window, cx),
    }
}

fn hex_key(ev: &KeyDownEvent, current: String, mode: Mode, window: &mut Window, cx: &mut App) {
    let key = ev.keystroke.key.as_str();
    let max = if mode.alpha { 8 } else { 6 };
    let ch = typed(ev).filter(char::is_ascii_hexdigit);
    let complete = with_open(|o| {
        let edit = o.edit.get_or_insert(Edit {
            field: 0,
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
                if let Some(ch) = ch {
                    if edit.all {
                        edit.text.clear();
                        edit.all = false;
                    }
                    if edit.text.len() < max {
                        edit.text.push(ch.to_ascii_uppercase());
                        let n = edit.text.len();
                        if n == 3 || n == 6 || (mode.alpha && (n == 4 || n == 8)) {
                            return sk_css::parse_color_in(
                                &format!("#{}", edit.text),
                                mode.working(),
                            );
                        }
                    }
                }
            }
        }
        None
    })
    .flatten();
    match complete {
        Some(c) => apply(
            |h| *h = Hsva::from_rgba(c.map(|v| v.clamp(0.0, 1.0)), h.h),
            window,
            cx,
        ),
        None => repaint(window, cx),
    }
}

fn begin(part: Part, window: &mut Window, cx: &mut App) {
    with_open(|o| {
        o.part = part;
        o.edit = None;
        o.drag = Some(part);
        o.keyboard = false;
    });
    repaint(window, cx);
}

fn dragging() -> Option<Part> {
    OPEN.with(|o| o.borrow().as_ref().and_then(|o| o.drag))
}

fn shape() -> Shape {
    with_prefs(|p| p.shape)
}

fn drag_to(part: Part, b: Bounds<Pixels>, p: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let (x, y) = (f32::from(p.x - b.origin.x), f32::from(p.y - b.origin.y));
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let along = ((x - TRACK / 2.0) / (w - TRACK).max(1.0)).clamp(0.0, 1.0);
    let disc = Disc::of(b);
    let at = (f32::from(p.x), f32::from(p.y));
    match (part, shape()) {
        (Part::Square, Shape::Square) => apply(
            |c| {
                c.s = (x / w).clamp(0.0, 1.0);
                c.v = 1.0 - (y / h).clamp(0.0, 1.0);
            },
            window,
            cx,
        ),
        (Part::Square, Shape::Wheel) => {
            let sq = disc.square();
            let (sx, sy) = (f32::from(sq.origin.x), f32::from(sq.origin.y));
            let side = f32::from(sq.size.width);
            apply(
                |c| {
                    c.s = ((at.0 - sx) / side).clamp(0.0, 1.0);
                    c.v = 1.0 - ((at.1 - sy) / side).clamp(0.0, 1.0);
                },
                window,
                cx,
            )
        }
        (Part::Square, Shape::Triangle) => apply(
            |c| {
                let (s, v) = sv_at(disc.triangle(c.h), at, c.s);
                c.s = s;
                c.v = v;
            },
            window,
            cx,
        ),
        (Part::Hue, Shape::Square) => apply(|c| c.h = along, window, cx),
        (Part::Hue, _) => apply(|c| c.h = disc.hue_at(at), window, cx),
        (Part::Alpha, _) => apply(|c| c.a = along, window, cx),
        (Part::Intensity, _) => apply_open(
            |o| o.stops = (along * 2.0 * MAX_STOPS as f32).round() as i32 - MAX_STOPS,
            window,
            cx,
        ),
        _ => {}
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
    follow(Some(part), b, window);
}

/// The ring takes the hue and what it surrounds the saturation and value.
fn track_disc(b: Bounds<Pixels>, window: &mut Window) {
    window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
        if !phase.bubble() || ev.button != MouseButton::Left || !b.contains(&ev.position) {
            return;
        }
        let d = Disc::of(b);
        let r = d.distance((f32::from(ev.position.x), f32::from(ev.position.y)));
        let part = if r >= d.inner - 1.0 && r <= d.outer + 1.0 {
            Part::Hue
        } else if r < d.inner {
            Part::Square
        } else {
            return;
        };
        begin(part, window, cx);
        drag_to(part, b, ev.position, window, cx);
    });
    follow(None, b, window);
}

/// Moves and the release for a drag of `part`, or of the square or hue when `None`.
fn follow(part: Option<Part>, b: Bounds<Pixels>, window: &mut Window) {
    let ours = move |p: Option<Part>| match part {
        Some(want) => p == Some(want),
        None => matches!(p, Some(Part::Square | Part::Hue)),
    };
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
        if let Some(p) = dragging().filter(|&p| phase.bubble() && ours(Some(p))) {
            drag_to(p, b, ev.position, window, cx);
        }
    });
    window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
        if phase.bubble() && ev.button == MouseButton::Left {
            if let Some(p) = dragging().filter(|&p| ours(Some(p))) {
                drag_to(p, b, ev.position, window, cx);
                with_open(|o| o.drag = None);
            }
        }
    });
}

/// The wheel and the triangle: a hue ring around the saturation and value.
#[derive(Clone, Copy)]
struct Disc {
    x: f32,
    y: f32,
    outer: f32,
    inner: f32,
}

impl Disc {
    fn of(b: Bounds<Pixels>) -> Disc {
        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
        let outer = w.min(h) / 2.0;
        Disc {
            x: f32::from(b.origin.x) + w / 2.0,
            y: f32::from(b.origin.y) + h / 2.0,
            outer,
            inner: outer - RING,
        }
    }

    fn distance(self, (x, y): (f32, f32)) -> f32 {
        (x - self.x).hypot(y - self.y)
    }

    /// Hue 0 at the top, increasing clockwise.
    fn at(self, hue: f32, r: f32) -> (f32, f32) {
        let a = hue * TAU - FRAC_PI_2;
        (self.x + r * a.cos(), self.y + r * a.sin())
    }

    fn hue_at(self, (x, y): (f32, f32)) -> f32 {
        (((y - self.y).atan2(x - self.x) + FRAC_PI_2) / TAU).rem_euclid(1.0)
    }

    fn square(self) -> Bounds<Pixels> {
        let half = (self.inner - 6.0) / SQRT_2;
        Bounds::new(
            pt(self.x - half, self.y - half),
            gpui::size(px(2.0 * half), px(2.0 * half)),
        )
    }

    /// The pure hue, white and black corners.
    fn triangle(self, hue: f32) -> [(f32, f32); 3] {
        let r = self.inner - 10.0;
        [
            self.at(hue, r),
            self.at(hue + 1.0 / 3.0, r),
            self.at(hue - 1.0 / 3.0, r),
        ]
    }

    fn circle(self, r: f32) -> Bounds<Pixels> {
        Bounds::new(
            pt(self.x - r, self.y - r),
            gpui::size(px(2.0 * r), px(2.0 * r)),
        )
    }
}

fn pt(x: f32, y: f32) -> Point<Pixels> {
    gpui::point(px(x), px(y))
}

/// The weights of `p` on the triangle's hue, white and black corners.
fn barycentric(t: [(f32, f32); 3], (x, y): (f32, f32)) -> [f32; 3] {
    let [(x1, y1), (x2, y2), (x3, y3)] = t;
    let det = (y2 - y3) * (x1 - x3) + (x3 - x2) * (y1 - y3);
    if det.abs() < f32::EPSILON {
        return [0.0, 0.0, 1.0];
    }
    let u = ((y2 - y3) * (x - x3) + (x3 - x2) * (y - y3)) / det;
    let w = ((y3 - y1) * (x - x3) + (x1 - x3) * (y - y3)) / det;
    [u, w, 1.0 - u - w]
}

fn triangle_point(t: [(f32, f32); 3], s: f32, v: f32) -> (f32, f32) {
    let k = [s * v, (1.0 - s) * v, 1.0 - v];
    (
        k[0] * t[0].0 + k[1] * t[1].0 + k[2] * t[2].0,
        k[0] * t[0].1 + k[1] * t[1].1 + k[2] * t[2].1,
    )
}

/// Saturation and value at `p`, pulled into the triangle; `s` stays on the black corner.
fn sv_at(t: [(f32, f32); 3], p: (f32, f32), s: f32) -> (f32, f32) {
    let k = barycentric(t, p).map(|k| k.max(0.0));
    let sum = (k[0] + k[1] + k[2]).max(f32::EPSILON);
    let (u, w) = (k[0] / sum, k[1] / sum);
    let v = (u + w).clamp(0.0, 1.0);
    (if v > 1e-4 { (u / v).clamp(0.0, 1.0) } else { s }, v)
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
    track: Hsla,
    pill: Hsla,
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
                track: black(0.25),
                pill: white(0.1),
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
                track: black(0.055),
                pill: white(1.0),
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
    stops: i32,
    part: Part,
    edit: Option<Edit>,
    recent: usize,
    keyboard: bool,
    fallback: windows::Rect,
    mode: Mode,
    shape: Shape,
    format: Format,
}

impl View {
    /// The base as the window paints it.
    fn shown(&self) -> [f32; 4] {
        let [r, g, b, a] = self.color.rgba();
        let [r, g, b] = color::display([r, g, b], self.mode.working());
        [r, g, b, if self.mode.alpha { a } else { 1.0 }]
    }

    /// The value as the window paints it, and whether it is brighter than the display's white.
    fn output(&self) -> ([f32; 3], bool) {
        let [r, g, b, _] = color::join(self.color.rgba(), self.stops, self.mode);
        let lin = sk_css::convert_rgb([r, g, b], self.mode.space(), RgbSpace::SrgbLinear);
        let over = self.mode.hdr && r.max(g).max(b) > 1.0;
        let rgb = sk_css::convert_rgb(lin, RgbSpace::SrgbLinear, RgbSpace::Srgb);
        (rgb.map(|c| c.clamp(0.0, 1.0)), over)
    }

    fn hue(&self) -> [f32; 4] {
        let [r, g, b, _] = Hsva {
            s: 1.0,
            v: 1.0,
            a: 1.0,
            ..self.color
        }
        .rgba();
        let [r, g, b] = color::display([r, g, b], self.mode.working());
        [r, g, b, 1.0]
    }
}

fn rgb1([r, g, b]: [f32; 3]) -> [f32; 4] {
    [r, g, b, 1.0]
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
    window.paint_quad(plain(b, r, gpui::white()));
    window.paint_quad(plain(b.dilate(px(-3.0)), r - 3.0, color::hsla(fill)));
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

fn hue_stops() -> Vec<gpui::LinearColorStop> {
    HUES.iter()
        .enumerate()
        .map(|(i, &hex)| linear_color_stop(gpui::rgb(hex), i as f32 / 6.0))
        .collect()
}

/// Saturation across, value down, under the current hue.
fn paint_sv(b: Bounds<Pixels>, v: &View, focused: bool, pal: Palette, window: &mut Window) {
    let r = 6.0;
    window.paint_quad(plain(b, r, color::hsla(v.hue())));
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
        b.origin.x + b.size.width * v.color.s,
        b.origin.y + b.size.height * (1.0 - v.color.v),
    );
    let [cr, cg, cb, _] = v.shown();
    thumb(at, [cr, cg, cb, 1.0], window);
    if focused {
        ring(b, r, pal, window);
    }
}

/// The triangle row by row: each row is a gradient, since the color is linear across it.
fn paint_triangle(t: [(f32, f32); 3], corners: [[f32; 3]; 3], window: &mut Window) {
    let top = t.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor();
    let bottom = t.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil();
    let at = |x: f32, y: f32| {
        let k = barycentric(t, (x, y)).map(|k| k.clamp(0.0, 1.0));
        let mix = |i: usize| k[0] * corners[0][i] + k[1] * corners[1][i] + k[2] * corners[2][i];
        color::hsla([mix(0), mix(1), mix(2), 1.0])
    };
    let mut y = top;
    while y < bottom {
        let mid = y + 0.5;
        let mut span: Option<(f32, f32)> = None;
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (t[a], t[b]);
            if p.1 != q.1 && (p.1 - mid) * (q.1 - mid) <= 0.0 {
                let x = p.0 + (mid - p.1) / (q.1 - p.1) * (q.0 - p.0);
                span = Some(span.map_or((x, x), |(l, r)| (l.min(x), r.max(x))));
            }
        }
        if let Some((l, r)) = span.filter(|(l, r)| r > l) {
            window.paint_quad(plain(
                Bounds::new(pt(l, y), gpui::size(px(r - l), px(1.6))),
                0.0,
                linear_gradient(
                    90.0,
                    linear_color_stop(at(l, mid), 0.0),
                    linear_color_stop(at(r, mid), 1.0),
                ),
            ));
        }
        y += 1.0;
    }
}

fn square(v: &View, pal: Palette) -> AnyElement {
    let v = View { edit: None, ..*v };
    let focused = v.keyboard && v.part == Part::Square;
    canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            paint_sv(b, &v, focused, pal, window);
            track(Part::Square, b, window);
        },
    )
    .w(px(CONTENT))
    .h(px(SQUARE_H))
    .into_any_element()
}

fn disc(v: &View, pal: Palette) -> AnyElement {
    let v = View { edit: None, ..*v };
    canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            let d = Disc::of(b);
            window.paint_quad(plain(
                d.circle(d.outer),
                d.outer,
                gpui::conic_gradient(0.0, &hue_stops()),
            ));
            window.paint_quad(hairline(d.circle(d.outer), d.outer, pal.hairline));
            window.paint_quad(plain(d.circle(d.inner), d.inner, pal.surface));
            window.paint_quad(hairline(d.circle(d.inner), d.inner, pal.hairline));
            let (hx, hy) = d.at(v.color.h, (d.outer + d.inner) / 2.0);
            thumb(pt(hx, hy), v.hue(), window);
            let focused = |p: Part| v.keyboard && v.part == p;
            if focused(Part::Hue) {
                ring(d.circle(d.outer), d.outer, pal, window);
            }
            if v.shape == Shape::Wheel {
                paint_sv(d.square(), &v, focused(Part::Square), pal, window);
            } else {
                let t = d.triangle(v.color.h);
                let [hr, hg, hb, _] = v.hue();
                paint_triangle(t, [[hr, hg, hb], [1.0; 3], [0.0; 3]], window);
                let mut edge = PathBuilder::stroke(px(1.0));
                edge.add_polygon(&t.map(|(x, y)| pt(x, y)), true);
                if let Ok(path) = edge.build() {
                    window.paint_path(path, pal.hairline);
                }
                let (sx, sy) = triangle_point(t, v.color.s, v.color.v);
                let [cr, cg, cb, _] = v.shown();
                thumb(pt(sx, sy), [cr, cg, cb, 1.0], window);
                if focused(Part::Square) {
                    let r = THUMB / 2.0 + 2.0;
                    ring(
                        Bounds::new(pt(sx - r, sy - r), gpui::size(px(2.0 * r), px(2.0 * r))),
                        r,
                        pal,
                        window,
                    );
                }
            }
            track_disc(b, window);
        },
    )
    .w(px(CONTENT))
    .h(px(DISC))
    .into_any_element()
}

fn slider(
    part: Part,
    width: f32,
    at: f32,
    fill: [f32; 4],
    focused: bool,
    pal: Palette,
    paint: impl Fn(Bounds<Pixels>, &mut Window) + 'static,
) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            let r = TRACK / 2.0;
            let t = Bounds::new(
                gpui::point(b.origin.x, b.origin.y + (b.size.height - px(TRACK)) / 2.0),
                gpui::size(b.size.width, px(TRACK)),
            );
            paint(t, window);
            window.paint_quad(hairline(t, r, pal.hairline));
            let x = t.origin.x + px(r) + (t.size.width - px(TRACK)) * at;
            thumb(gpui::point(x, t.origin.y + px(r)), fill, window);
            if focused {
                ring(t, r, pal, window);
            }
            track(part, b, window);
        },
    )
    .w(px(width))
    .h(px(THUMB))
    .into_any_element()
}

fn hue_slider(v: &View, pal: Palette) -> AnyElement {
    slider(
        Part::Hue,
        CONTENT,
        v.color.h,
        v.hue(),
        v.keyboard && v.part == Part::Hue,
        pal,
        move |t, window| {
            window.paint_quad(plain(
                t,
                TRACK / 2.0,
                linear_gradient_stops(90.0, &hue_stops()),
            ))
        },
    )
}

fn alpha_slider(v: &View, pal: Palette) -> AnyElement {
    let [r, g, b, a] = v.shown();
    slider(
        Part::Alpha,
        CONTENT,
        a,
        [r, g, b, a],
        v.keyboard && v.part == Part::Alpha,
        pal,
        move |t, window| {
            let radius = TRACK / 2.0;
            color::paint_checker(t, radius, pal.dark, window);
            color::mask_corners(t, radius, pal.surface, window);
            window.paint_quad(plain(
                t,
                radius,
                linear_gradient(
                    90.0,
                    linear_color_stop(color::hsla([r, g, b, 0.0]), 0.0),
                    linear_color_stop(color::hsla([r, g, b, 1.0]), 1.0),
                ),
            ));
        },
    )
}

fn signed(stops: i32) -> String {
    match stops {
        0 => "0".into(),
        s if s > 0 => format!("+{s}"),
        s => format!("\u{2212}{}", -s),
    }
}

fn multiplier(stops: i32) -> String {
    if stops >= 0 {
        format!("\u{d7}{}", 1u32 << stops)
    } else {
        format!("\u{d7}1/{}", 1u32 << -stops)
    }
}

/// Base against output, the exposure slider in stops and the preset stops.
fn intensity(v: &View, pal: Palette) -> AnyElement {
    let base = v.shown();
    let (out, over) = v.output();
    let preview = canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            let r = 4.0;
            let half = b.size.width / 2.0;
            let left = Bounds::new(b.origin, gpui::size(half, b.size.height));
            let right = Bounds::new(
                gpui::point(b.origin.x + half, b.origin.y),
                gpui::size(half, b.size.height),
            );
            if base[3] < 1.0 {
                color::paint_checker(b, 4.0, pal.dark, window);
                color::mask_corners(b, r, pal.surface, window);
            }
            window.paint_quad(gpui::quad(
                left,
                gpui::Corners {
                    top_left: px(r),
                    bottom_left: px(r),
                    ..Default::default()
                },
                color::hsla(base),
                px(0.0),
                gpui::transparent_black(),
                gpui::BorderStyle::Solid,
            ));
            let out = color::hsla([out[0], out[1], out[2], base[3]]);
            let corners = gpui::Corners {
                top_right: px(r),
                bottom_right: px(r),
                ..Default::default()
            };
            let fill = |bg: gpui::Background| {
                gpui::quad(
                    right,
                    corners,
                    bg,
                    px(0.0),
                    gpui::transparent_black(),
                    gpui::BorderStyle::Solid,
                )
            };
            window.paint_quad(fill(out.into()));
            if over {
                window.paint_quad(fill(gpui::radial_gradient(&[
                    linear_color_stop(gpui::white(), 0.0),
                    linear_color_stop(gpui::hsla(0.0, 0.0, 1.0, 0.85), 0.35),
                    linear_color_stop(gpui::hsla(0.0, 0.0, 1.0, 0.0), 1.0),
                ])));
                // A camera's zebra stripes: brighter than the display can show.
                window.paint_quad(fill(gpui::pattern_slash(
                    gpui::hsla(0.0, 0.0, 0.0, 0.16),
                    1.5,
                    5.0,
                )));
            }
            window.paint_quad(hairline(b, r, pal.hairline));
        },
    )
    .absolute()
    .size_full();
    let label = if over {
        gpui::black()
    } else {
        on(color::hsla(rgb1(out)))
    };
    let preview = div()
        .relative()
        .w(px(CONTENT))
        .h(px(PREVIEW_H))
        .child(preview)
        .child(
            div()
                .absolute()
                .right(px(8.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .flex()
                .items_center()
                .text_size(px(11.0))
                .text_color(label)
                .child(multiplier(v.stops)),
        );
    let along = (v.stops + MAX_STOPS) as f32 / (2 * MAX_STOPS) as f32;
    let exposure = slider(
        Part::Intensity,
        CONTENT - 44.0,
        along,
        rgb1(out),
        v.keyboard && v.part == Part::Intensity,
        pal,
        move |t, window| {
            window.paint_quad(plain(
                t,
                TRACK / 2.0,
                linear_gradient_stops(
                    90.0,
                    &[
                        linear_color_stop(gpui::black(), 0.0),
                        linear_color_stop(color::hsla(rgb1([base[0], base[1], base[2]])), 0.5),
                        linear_color_stop(gpui::white(), 1.0),
                    ],
                ),
            ))
        },
    );
    let chips = STOP_PRESETS.iter().enumerate().map(|(i, &s)| {
        let on_it = s == v.stops;
        div()
            .id(("sk-color-picker-stops", i))
            .flex_1()
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .text_size(px(12.0))
            .bg(if on_it { pal.accent } else { pal.track })
            .text_color(if on_it { pal.on_accent } else { pal.text })
            .child(signed(s))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                with_open(|o| o.keyboard = false);
                set_stops(s, window, cx);
            })
    });
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(preview)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(exposure)
                .child(
                    div()
                        .w(px(36.0))
                        .flex()
                        .justify_end()
                        .text_size(px(12.0))
                        .child(signed(v.stops)),
                ),
        )
        .child(div().flex().gap(px(4.0)).children(chips))
        .into_any_element()
}

/// A color sample: a checkerboard shows through a translucent one; `stops` other than 0 get a dot.
fn chip(
    c: [f32; 4],
    size: f32,
    radius: f32,
    focused: bool,
    stops: i32,
    pal: Palette,
) -> gpui::Canvas<()> {
    let mut chip = canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            if c[3] < 1.0 {
                color::paint_checker(b, radius.max(4.0), pal.dark, window);
                color::mask_corners(b, radius, pal.surface, window);
            }
            window.paint_quad(plain(b, radius, color::hsla(c)));
            window.paint_quad(hairline(b, radius, pal.hairline));
            if stops != 0 {
                let d = 7.0;
                let dot = Bounds::new(
                    gpui::point(
                        b.origin.x + b.size.width - px(d + 1.0),
                        b.origin.y + px(1.0),
                    ),
                    gpui::size(px(d), px(d)),
                );
                let (fill, edge) = if stops > 0 {
                    (gpui::white(), gpui::hsla(0.0, 0.0, 0.0, 0.6))
                } else {
                    (gpui::black(), gpui::hsla(0.0, 0.0, 1.0, 0.8))
                };
                window.paint_quad(gpui::quad(
                    dot,
                    px(d / 2.0),
                    fill,
                    px(1.0),
                    edge,
                    gpui::BorderStyle::Solid,
                ));
            }
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

fn focus_frame(radius: f32, pal: Palette) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |b, (), window, _| ring(b, radius, pal, window),
    )
    .absolute()
    .size_full()
    .into_any_element()
}

fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    items: &[T],
    picked: T,
    height: f32,
    focused: bool,
    pal: Palette,
    content: impl Fn(T, Hsla) -> AnyElement,
    pick: fn(T, &mut Window, &mut App),
) -> AnyElement {
    let row = div()
        .id(id)
        .relative()
        .flex()
        .h(px(height))
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(5.0))
        .bg(pal.track)
        .children(items.iter().enumerate().map(|(i, &item)| {
            let on_it = item == picked;
            let ink = if on_it { pal.text } else { pal.text2 };
            let mut seg = div()
                .id((id, i))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_color(ink)
                .child(content(item, ink))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    with_open(|o| o.keyboard = false);
                    pick(item, window, cx);
                });
            if on_it {
                seg = seg.bg(pal.pill).border_1().border_color(pal.field_stroke);
            }
            seg
        }));
    if focused {
        row.child(focus_frame(5.0, pal)).into_any_element()
    } else {
        row.into_any_element()
    }
}

fn shape_icon(shape: Shape, ink: Hsla) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |b, (), window, _| {
            let c = b.center();
            let (x, y) = (f32::from(c.x), f32::from(c.y));
            let r = 6.5;
            let mut p = PathBuilder::stroke(px(1.25));
            let circle = |p: &mut PathBuilder| {
                p.move_to(pt(x + r, y));
                p.arc_to(pt(r, r), px(0.0), false, true, pt(x - r, y));
                p.arc_to(pt(r, r), px(0.0), false, true, pt(x + r, y));
                p.close();
            };
            match shape {
                Shape::Square => p.add_polygon(
                    &[
                        pt(x - r, y - r),
                        pt(x + r, y - r),
                        pt(x + r, y + r),
                        pt(x - r, y + r),
                    ],
                    true,
                ),
                Shape::Wheel => {
                    circle(&mut p);
                    let s = 2.75;
                    p.add_polygon(
                        &[
                            pt(x - s, y - s),
                            pt(x + s, y - s),
                            pt(x + s, y + s),
                            pt(x - s, y + s),
                        ],
                        true,
                    );
                }
                Shape::Triangle => {
                    circle(&mut p);
                    let t = r - 2.25;
                    let corner = |deg: f32| {
                        let a = deg.to_radians();
                        pt(x + t * a.cos(), y + t * a.sin())
                    };
                    p.add_polygon(&[corner(-90.0), corner(30.0), corner(150.0)], true);
                }
            }
            if let Ok(path) = p.build() {
                window.paint_path(path, ink);
            }
        },
    )
    .w(px(16.0))
    .h(px(16.0))
    .into_any_element()
}

fn caret(pal: Palette) -> gpui::Div {
    div().w(px(1.0)).h(px(16.0)).bg(pal.text)
}

fn field_shell(
    id: (&'static str, usize),
    focused: bool,
    pal: Palette,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .relative()
        .flex_1()
        .min_w(px(0.0))
        .h(px(FIELD_H))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(pal.field_stroke)
        .bg(if focused { pal.field_focus } else { pal.field })
        .child(
            div()
                .absolute()
                .bottom(px(0.0))
                .left(px(3.0))
                .right(px(3.0))
                .h(px(if focused { 2.0 } else { 1.0 }))
                .bg(if focused { pal.accent } else { pal.underline }),
        )
}

fn edited(text: String, all: bool, focused: bool, pal: Palette) -> gpui::Div {
    let shown = if all && !text.is_empty() {
        div()
            .bg(pal.accent)
            .text_color(pal.on_accent)
            .rounded(px(2.0))
            .child(text)
    } else {
        div().child(text)
    };
    let line = div().flex().items_center().child(shown);
    if focused && !all {
        line.child(caret(pal))
    } else {
        line
    }
}

fn fields(v: &View, pal: Palette) -> AnyElement {
    let count = v.format.fields(v.mode.alpha);
    let values = values(v.color, v.format, v.mode);
    let labels = v.format.labels();
    let row = div().flex_1().min_w(px(0.0)).flex().gap(px(4.0));
    row.children((0..count).map(|i| {
        let focused = v.part == Part::Field(i);
        let (text, all) = match &v.edit {
            Some(e) if focused && e.field == i => (e.text.clone(), e.all),
            _ if v.format == Format::Hex => (hex_digits(v.color, v.mode), false),
            _ => (value_text(v.format, i, values[i]), false),
        };
        let label = if i == 3 { "A" } else { labels[i] };
        let mut shell = field_shell(("sk-color-picker-field", i), focused, pal)
            .px(px(if v.format == Format::Hex { 10.0 } else { 6.0 }))
            .gap(px(if v.format == Format::Hex { 0.0 } else { 4.0 }))
            .child(div().text_color(pal.text2).child(label))
            .child(edited(text, all, focused, pal))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                with_open(|o| o.keyboard = false);
                focus(Part::Field(i), window, cx);
            });
        if v.format == Format::Hex {
            shell = shell.font_family(if cfg!(windows) { "Consolas" } else { "Menlo" });
        } else {
            shell = shell.text_size(px(12.0));
        }
        shell
    }))
    .into_any_element()
}

/// The shape tabs, and a badge naming a space other than sRGB.
fn header(tabs: AnyElement, mode: Mode, pal: Palette) -> AnyElement {
    let name = match (mode.p3, mode.hdr) {
        (false, false) => return tabs,
        (true, false) => "P3",
        (false, true) => "HDR",
        (true, true) => "P3 HDR",
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(div().flex_1().child(tabs))
        .child(
            div()
                .h(px(TABS_H))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(pal.field_stroke)
                .text_size(px(11.0))
                .text_color(pal.text2)
                .child(name),
        )
        .into_any_element()
}

fn view(v: &View, recents: &[Recent], pal: Palette) -> AnyElement {
    let ring_on = |p: Part| v.keyboard && v.part == p;
    let tabs = segmented(
        "sk-color-picker-shape",
        &Shape::ALL,
        v.shape,
        TABS_H,
        ring_on(Part::Shape),
        pal,
        shape_icon,
        set_shape,
    );
    let mut body = div()
        .id("sk-color-picker")
        .occlude()
        .flex_none()
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
        .child(header(tabs, v.mode, pal));
    body.style().box_shadow = Some(vec![
        BoxShadow::new(px(0.0), px(8.0), pal.shadow).blur_radius(px(16.0)),
        BoxShadow::new(px(0.0), px(1.0), pal.shadow.opacity(0.6)).blur_radius(px(3.0)),
    ]);
    body = if v.shape == Shape::Square {
        body.child(square(v, pal)).child(hue_slider(v, pal))
    } else {
        body.child(disc(v, pal))
    };
    if v.mode.alpha {
        body = body.child(alpha_slider(v, pal));
    }
    if v.mode.hdr {
        body = body.child(intensity(v, pal));
    }
    let formats = segmented(
        "sk-color-picker-format",
        &Format::ALL,
        v.format,
        SEG_H,
        ring_on(Part::Format),
        pal,
        |f: Format, _| {
            div()
                .text_size(px(11.0))
                .child(f.name().to_ascii_uppercase())
                .into_any_element()
        },
        set_format,
    );
    body = body.child(
        div().flex().flex_col().gap(px(8.0)).child(formats).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(chip(v.shown(), FIELD_H, 4.0, false, 0, pal))
                .child(fields(v, pal)),
        ),
    );
    if !recents.is_empty() {
        let gap = (CONTENT - RECENT_MAX as f32 * CHIP) / (RECENT_MAX - 1) as f32;
        body = body.child(
            div()
                .flex()
                .gap(px(gap))
                .children(recents.iter().enumerate().map(|(i, r)| {
                    let c = color::parse(&r.css).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                    let c = if v.mode.alpha {
                        c
                    } else {
                        [c[0], c[1], c[2], 1.0]
                    };
                    div()
                        .id(("sk-color-picker-recent", i))
                        .child(chip(
                            c,
                            CHIP,
                            4.0,
                            ring_on(Part::Recent) && v.recent == i,
                            r.stops,
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

fn with_prefs<R>(f: impl FnOnce(&mut Prefs) -> R) -> R {
    PREFS.with(|p| {
        let mut p = p.borrow_mut();
        let prefs = p.get_or_insert_with(|| {
            let file = PREFS_FILE.lock().ok().and_then(|f| f.clone());
            file.and_then(|f| std::fs::read_to_string(f).ok())
                .map(|s| parse_prefs(&s))
                .unwrap_or_default()
        });
        f(prefs)
    })
}

fn prefs() -> Prefs {
    with_prefs(|p| p.clone())
}

/// One line per recent color, newest first: a CSS color, and a tab and its stops when not 0.
/// `shape=` and `format=` lines hold the picker's choices. Other lines are skipped.
fn parse_prefs(text: &str) -> Prefs {
    let mut p = Prefs::default();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("shape=") {
            p.shape = Shape::ALL
                .into_iter()
                .find(|s| s.name() == name)
                .unwrap_or_default();
            continue;
        }
        if let Some(name) = line.strip_prefix("format=") {
            p.format = Format::ALL
                .into_iter()
                .find(|f| f.name() == name)
                .unwrap_or_default();
            continue;
        }
        let (css, stops) = match line.split_once('\t') {
            Some((css, stops)) => (css.trim(), stops.trim().parse::<i32>().unwrap_or(0)),
            None => (line, 0),
        };
        if p.recents.len() < RECENT_MAX && color::parse(css).is_some() {
            p.recents.push(Recent {
                css: css.to_string(),
                stops: stops.clamp(-MAX_STOPS, MAX_STOPS),
            });
        }
    }
    p
}

fn format_prefs(p: &Prefs) -> String {
    let mut text = format!("shape={}\nformat={}\n", p.shape.name(), p.format.name());
    for r in &p.recents {
        text.push_str(&r.css);
        if r.stops != 0 {
            text.push_str(&format!("\t{}", r.stops));
        }
        text.push('\n');
    }
    text
}

fn save_prefs() {
    let text = format_prefs(&prefs());
    let file = PREFS_FILE.lock().ok().and_then(|f| f.clone());
    if let Some(file) = file {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&file, text) {
            log::warn!(
                "color picker: could not save recent colors to {}: {e}",
                file.display()
            );
        }
    }
}

fn remember(entry: Recent) {
    with_prefs(|p| {
        p.recents.retain(|r| *r != entry);
        p.recents.insert(0, entry);
        p.recents.truncate(RECENT_MAX);
    });
    save_prefs();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32], eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < eps)
    }

    #[test]
    fn hsv_round_trips_and_a_gray_keeps_the_hue_it_had() {
        for rgba in [
            [1.0, 0.0, 0.0, 1.0],
            [0.2, 0.4, 0.6, 0.5],
            [0.9, 0.9, 0.1, 1.0],
            [0.5, 0.0, 1.0, 1.0],
        ] {
            let back = Hsva::from_rgba(rgba, 0.0).rgba();
            assert!(close(&rgba, &back, 1e-5), "{rgba:?} -> {back:?}");
        }
        assert_eq!(Hsva::from_rgba([0.5, 0.5, 0.5, 1.0], 0.25).h, 0.25);
        assert_eq!(Hsva::from_rgba([0.0, 0.0, 0.0, 1.0], 0.7).h, 0.7);
    }

    #[test]
    fn hsl_and_oklch_fields_round_trip() {
        let c = Hsva::from_rgba([0.8, 0.3, 0.1, 1.0], 0.0);
        let (s, l) = c.sl();
        assert!(close(&c.with_sl(s, l).rgba(), &c.rgba(), 1e-5));
        let mut gray = Hsva::from_rgba([0.5, 0.5, 0.5, 1.0], 0.4);
        set_value(&mut gray, Format::Hsl, 1, 100.0, Mode::default());
        assert!(
            (gray.h - 0.4).abs() < 1e-6,
            "saturating a gray keeps its hue"
        );
        for space in [RgbSpace::Srgb, RgbSpace::DisplayP3] {
            let rgb = [0.8, 0.3, 0.1];
            let back = from_oklch(to_oklch(rgb, space), space);
            assert!(close(&rgb, &back, 1e-4), "{space:?}: {back:?}");
        }
        let white = to_oklch([1.0, 1.0, 1.0], RgbSpace::Srgb);
        assert!(
            (white[0] - 1.0).abs() < 1e-3 && white[1] < 1e-3,
            "{white:?}"
        );
        let mut c = Hsva::from_rgba([0.2, 0.4, 0.6, 1.0], 0.0);
        set_value(&mut c, Format::Rgb, 0, 255.0, Mode::default());
        assert!(close(&c.rgba(), &[1.0, 0.4, 0.6, 1.0], 1e-5));
        set_value(&mut c, Format::Rgb, 3, 50.0, Mode::default());
        assert_eq!(c.a, 0.5);
    }

    #[test]
    fn the_triangle_maps_saturation_and_value_both_ways() {
        let d = Disc {
            x: 100.0,
            y: 100.0,
            outer: 98.0,
            inner: 80.0,
        };
        let t = d.triangle(0.3);
        for (s, v) in [(1.0, 1.0), (0.0, 1.0), (0.5, 0.5), (0.25, 0.8)] {
            let (s2, v2) = sv_at(t, triangle_point(t, s, v), 0.0);
            assert!(close(&[s, v], &[s2, v2], 1e-4), "{s},{v} -> {s2},{v2}");
        }
        assert_eq!(
            sv_at(t, t[2], 0.6),
            (0.6, 0.0),
            "black keeps the saturation"
        );
        assert!((d.hue_at(d.at(0.3, 50.0)) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn prefs_keep_shape_format_and_hdr_recents_and_read_the_old_file() {
        let old = parse_prefs("#ff0000ff\n#0080ff80\njunk\n");
        assert_eq!(old.shape, Shape::Square);
        assert_eq!(old.recents.len(), 2);
        assert_eq!(old.recents[1].stops, 0);
        let p = Prefs {
            shape: Shape::Triangle,
            format: Format::Oklch,
            recents: vec![
                Recent {
                    css: "color(display-p3 1 0.5 0 / 0.5)".into(),
                    stops: 3,
                },
                Recent {
                    css: "color(srgb 0 0 1)".into(),
                    stops: 0,
                },
            ],
        };
        let back = parse_prefs(&format_prefs(&p));
        assert_eq!(back.shape, p.shape);
        assert_eq!(back.format, p.format);
        assert_eq!(back.recents, p.recents);
        let many = "#000\n".repeat(20);
        assert_eq!(parse_prefs(&many).recents.len(), RECENT_MAX);
    }
}
