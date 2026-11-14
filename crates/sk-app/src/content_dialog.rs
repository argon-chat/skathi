//! The Windows 11 ContentDialog the host draws itself with gpui: what the permission prompt
//! (src/consent_window.rs) and the crash dialog (src/crash_window.rs) are made of.
//!
//! It opens hidden, is sized and drawn out of any `App` update ([`fit`]) and only then shown, so the open transition is DWM's alone.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, img, prelude::*, px, size, AnyElement, AnyWindowHandle, App, AsyncApp, Bounds, Div, DisplayId,
    ElementId, FocusHandle, FontWeight, HighlightStyle, Hsla, Image, Rgba, Role, Stateful, StyledText,
    TitlebarOptions, UnderlineStyle, Window, WindowBackgroundAppearance, WindowBounds, WindowControlArea,
    WindowKind, WindowOptions,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::popout_win;

pub const INPUT_DELAY: Duration = Duration::from_millis(500);

pub const WIDTH: f32 = 440.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub text: String,
    pub key: Option<(Range<usize>, char)>,
}

pub fn parse_label(button: &str) -> Label {
    let mut text = String::with_capacity(button.len());
    let mut key = None;
    let mut chars = button.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('&') => text.push('&'),
            Some(k) => {
                if key.is_none() {
                    let at = text.len();
                    key = Some((at..at + k.len_utf8(), k.to_lowercase().next().unwrap_or(k)));
                }
                text.push(k);
            }
            None => {}
        }
    }
    Label { text, key }
}

/// Does a keystroke name access key `key`? `key_char` is the character the layout types, `name`
/// gpui's key name, `vk_char` the Latin letter of the key the layout puts `key` on (`VkKeyScanW`),
/// so a Cyrillic access key matches whichever of the two gpui reports.
pub fn access_key_matches(key: char, name: &str, key_char: Option<&str>, vk_char: Option<char>) -> bool {
    let same = |s: &str| {
        let mut it = s.chars().flat_map(char::to_lowercase);
        it.next() == Some(key) && it.next().is_none()
    };
    same(name)
        || key_char.is_some_and(same)
        || vk_char.is_some_and(|v| name.chars().count() == 1 && name.eq_ignore_ascii_case(v.encode_utf8(&mut [0; 4])))
}

pub(crate) fn access_key_hit(labels: &[Label], k: &gpui::Keystroke) -> Option<usize> {
    labels.iter().position(|l| {
        l.key.as_ref().is_some_and(|(_, c)| access_key_matches(*c, &k.key, k.key_char.as_deref(), win::latin_key_of(*c)))
    })
}

/// What the input protection allows. Unarmed until the dialog is active, then closed for
/// [`INPUT_DELAY`]. A press counts from its start, so a release after the delay of a press begun
/// before it is still refused.
#[derive(Debug, Default, Clone, Copy)]
pub struct Guard {
    open_at: Option<Instant>,
}

impl Guard {
    /// The dialog became the active window at `now`. Once: gpui reports an activation a little after
    /// the show, which must not restart the delay.
    pub fn arm(&mut self, now: Instant) {
        if self.open_at.is_none() {
            self.open_at = Some(now + INPUT_DELAY);
        }
    }

    pub fn disarm(&mut self) {
        self.open_at = None;
    }

    pub fn allows(&self, now: Instant) -> bool {
        self.open_at.is_some_and(|t| now >= t)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }

    pub(crate) fn alpha(self, a: f32) -> Hsla {
        Rgba { r: self.0 as f32 / 255.0, g: self.1 as f32 / 255.0, b: self.2 as f32 / 255.0, a }.into()
    }
}

pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub fn text_on(fill: Rgb) -> Rgb {
    let (black, white) = (Rgb(0, 0, 0), Rgb(255, 255, 255));
    if contrast(fill, black) > contrast(fill, white) { black } else { white }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accent {
    pub light_theme: Rgb,
    pub dark_theme: Rgb,
}

impl Accent {
    pub const DEFAULT: Accent = Accent { light_theme: Rgb(0x00, 0x5F, 0xB8), dark_theme: Rgb(0x60, 0xCD, 0xFF) };

    pub fn fill(self, dark: bool) -> Rgb {
        if dark { self.dark_theme } else { self.light_theme }
    }
}

/// Is the accent distinct enough from the card to colour a glyph (3:1,
/// WCAG's for graphics)? A grey or near-black accent on the dark theme is
/// not, and the glyph takes the text colour instead.
pub fn glyph_in_accent(fill: Rgb, dark: bool) -> bool {
    let card = if dark { Rgb(0x2B, 0x2B, 0x2B) } else { Rgb(0xFF, 0xFF, 0xFF) };
    contrast(fill, card) >= 3.0
}

pub type Rect = [i32; 4];

pub fn center_in(anchor: Rect, work: Rect, w: i32, h: i32) -> (i32, i32) {
    let x = anchor[0] + (anchor[2] - anchor[0] - w) / 2;
    let y = anchor[1] + (anchor[3] - anchor[1] - h) / 2;
    (x.min(work[2] - w).max(work[0]), y.min(work[3] - h).max(work[1]))
}

pub(crate) struct Fonts {
    pub display: &'static str,
    pub text: &'static str,
    pub small: &'static str,
    pub icons: &'static str,
    /// Code: Consolas. Not Windows 11's Cascadia Mono: gpui draws that variable font a weight
    /// heavier than the body text at 12 px.
    pub mono: &'static str,
}

pub(crate) fn fonts(cx: &App) -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let names = cx.text_system().all_font_names();
        let has = |n: &str| names.iter().any(|f| f == n);
        let variable = has("Segoe UI Variable Text");
        Fonts {
            display: if variable { "Segoe UI Variable Display" } else { "Segoe UI" },
            text: if variable { "Segoe UI Variable Text" } else { "Segoe UI" },
            small: if variable { "Segoe UI Variable Small" } else { "Segoe UI" },
            icons: if has("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" },
            mono: if has("Consolas") { "Consolas" } else { "Cascadia Mono" },
        }
    })
}

pub(crate) struct Theme {
    pub text: Hsla,
    pub text2: Hsla,
    pub body: Hsla,
    pub footer: Hsla,
    pub stroke: Hsla,
    pub button: [Hsla; 3],
    pub button_text_pressed: Hsla,
    pub button_border: Hsla,
    pub accent: [Hsla; 3],
    pub on_accent: Hsla,
    pub on_accent_pressed: Hsla,
    pub accent_border: Hsla,
    pub tint: Hsla,
    pub glyph: Hsla,
    pub critical_tint: Hsla,
    pub critical: Hsla,
    pub well: Hsla,
    pub subtle: Hsla,
    pub ring: (Hsla, Hsla),
}

impl Theme {
    pub fn new(dark: bool, acrylic: bool, accent: Accent) -> Theme {
        let fill = accent.fill(dark);
        let on = text_on(fill);
        let black = Rgb(0, 0, 0);
        let white = Rgb(255, 255, 255);
        let vivid = glyph_in_accent(fill, dark);
        if dark {
            let critical = Rgb(0xFF, 0x99, 0xA4);
            Theme {
                text: white.alpha(1.0),
                text2: white.alpha(0.786),
                body: Rgb(0x2B, 0x2B, 0x2B).alpha(if acrylic { 0.6 } else { 1.0 }),
                footer: Rgb(0x20, 0x20, 0x20).alpha(if acrylic { 0.75 } else { 1.0 }),
                stroke: black.alpha(0.32),
                button: [white.alpha(0.061), white.alpha(0.084), white.alpha(0.033)],
                button_text_pressed: white.alpha(0.786),
                button_border: white.alpha(0.07),
                accent: [fill.alpha(1.0), fill.alpha(0.9), fill.alpha(0.8)],
                on_accent: on.alpha(1.0),
                on_accent_pressed: on.alpha(0.5),
                accent_border: white.alpha(0.08),
                tint: if vivid { fill.alpha(0.16) } else { white.alpha(0.08) },
                glyph: if vivid { fill.alpha(1.0) } else { white.alpha(1.0) },
                critical_tint: critical.alpha(0.16),
                critical: critical.alpha(1.0),
                well: black.alpha(0.2),
                subtle: white.alpha(0.061),
                ring: (white.alpha(1.0), black.alpha(0.7)),
            }
        } else {
            let critical = Rgb(0xC4, 0x2B, 0x1C);
            Theme {
                text: black.alpha(0.896),
                text2: black.alpha(0.606),
                body: white.alpha(if acrylic { 0.8 } else { 1.0 }),
                footer: Rgb(0xF3, 0xF3, 0xF3).alpha(if acrylic { 0.8 } else { 1.0 }),
                stroke: black.alpha(0.0578),
                button: [white.alpha(0.7), Rgb(0xF9, 0xF9, 0xF9).alpha(0.5), Rgb(0xF9, 0xF9, 0xF9).alpha(0.3)],
                button_text_pressed: black.alpha(0.606),
                button_border: black.alpha(0.1),
                accent: [fill.alpha(1.0), fill.alpha(0.9), fill.alpha(0.8)],
                on_accent: on.alpha(1.0),
                on_accent_pressed: on.alpha(0.7),
                accent_border: black.alpha(0.1),
                tint: if vivid { fill.alpha(0.12) } else { black.alpha(0.06) },
                glyph: if vivid { fill.alpha(1.0) } else { black.alpha(0.896) },
                critical_tint: critical.alpha(0.12),
                critical: critical.alpha(1.0),
                well: black.alpha(0.037),
                subtle: black.alpha(0.037),
                ring: (black.alpha(0.896), white.alpha(0.7)),
            }
        }
    }
}

pub(crate) fn accent() -> Accent {
    PREVIEW_ACCENT.get().copied().or_else(win::accent).unwrap_or(Accent::DEFAULT)
}

pub(crate) fn acrylic() -> bool {
    crate::window::has_system_backdrop() && !PREVIEW_SOLID.load(Ordering::Relaxed)
}

pub(crate) fn with_ring(d: Div, ring: bool, t: &Theme, radius: f32) -> Div {
    d.when(ring, |d| {
        d.child(
            div()
                .absolute()
                .top(px(-3.0))
                .left(px(-3.0))
                .right(px(-3.0))
                .bottom(px(-3.0))
                .rounded(px(radius + 3.0))
                .border_2()
                .border_color(t.ring.0)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .rounded(px(radius + 1.0))
                        .border_1()
                        .border_color(t.ring.1),
                ),
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn button(
    id: impl Into<ElementId>,
    label: &Label,
    accent: bool,
    t: &Theme,
    alt: bool,
    focus: &FocusHandle,
    window: &Window,
    wire: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
) -> Div {
    let (fill, fg, fg_pressed, border) = if accent {
        (t.accent, t.on_accent, t.on_accent_pressed, t.accent_border)
    } else {
        (t.button, t.text, t.button_text_pressed, t.button_border)
    };
    let underline = alt.then(|| label.key.as_ref().map(|(r, _)| r.clone())).flatten();
    let text = StyledText::new(label.text.clone()).with_highlights(underline.map(|r| {
        (
            r,
            HighlightStyle {
                underline: Some(UnderlineStyle { thickness: px(1.0), ..Default::default() }),
                ..Default::default()
            },
        )
    }));
    let ring = window.last_input_was_keyboard() && focus.is_focused(window);
    let shortcut = label.key.as_ref().map(|(_, c)| format!("Alt+{}", c.to_uppercase()));
    let inner = div()
        .id(id)
        .track_focus(focus)
        .role(Role::Button)
        .aria_label(label.text.clone())
        .when_some(shortcut, |d, s| d.aria_keyshortcuts(s))
        .h(px(32.0))
        .w_full()
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .rounded(px(4.0))
        .border_1()
        .border_color(border)
        .bg(fill[0])
        .text_color(fg)
        .hover(move |s| s.bg(fill[1]))
        .active(move |s| s.bg(fill[2]).text_color(fg_pressed))
        .child(text);
    with_ring(div().flex_1().relative().child(wire(inner)), ring, t, 4.0)
}

pub(crate) fn app_icon(icon: &Option<Arc<Image>>, t: &Theme, f: &Fonts) -> AnyElement {
    match icon {
        Some(image) => img(image.clone()).size(px(16.0)).into_any_element(),
        None => div()
            .font_family(f.icons)
            .text_size(px(16.0))
            .line_height(px(16.0))
            .text_color(t.text2)
            .child("\u{ECAA}")
            .into_any_element(),
    }
}

pub(crate) fn body(id: &'static str, app: &str, icon: AnyElement, t: &Theme, f: &Fonts) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .flex()
        .flex_col()
        .pt(px(16.0))
        .px(px(24.0))
        .pb(px(24.0))
        .bg(t.body)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(16.0))
                .child(icon)
                .child(
                    div()
                        .font_family(f.small)
                        .text_size(px(12.0))
                        .line_height(px(16.0))
                        .text_color(t.text2)
                        .child(app.to_string()),
                ),
        )
}

pub(crate) fn glyph_badge(glyph: char, tint: Hsla, color: Hsla, f: &Fonts) -> Div {
    div()
        .mt(px(20.0))
        .size(px(40.0))
        .rounded_full()
        .bg(tint)
        .flex()
        .items_center()
        .justify_center()
        .font_family(f.icons)
        .text_size(px(20.0))
        .line_height(px(20.0))
        .text_color(color)
        .child(glyph.to_string())
}

pub(crate) fn title(text: String, f: &Fonts) -> Div {
    div()
        .mt(px(16.0))
        .font_family(f.display)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(20.0))
        .line_height(px(28.0))
        .child(text)
}

pub(crate) fn content(text: String, t: &Theme) -> Div {
    div().mt(px(8.0)).text_size(px(14.0)).line_height(px(20.0)).text_color(t.text2).child(text)
}

pub(crate) fn footer(t: &Theme, buttons: Vec<impl IntoElement>) -> Div {
    div()
        .flex()
        .gap(px(8.0))
        .p(px(24.0))
        .border_t_1()
        .border_color(t.stroke)
        .bg(t.footer)
        .text_size(px(14.0))
        .line_height(px(20.0))
        .children(buttons)
}

pub(crate) fn card(height: Rc<Cell<f32>>, body: impl IntoElement, footer: impl IntoElement) -> Div {
    div()
        .relative()
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .child(
            canvas(move |b, _, _| height.set(f32::from(b.size.height)), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .child(body)
        .child(footer)
}

pub(crate) fn hwnd_of(window: &Window) -> Option<isize> {
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }
}

pub(crate) fn display_of(owner: isize, cx: &App) -> Option<DisplayId> {
    let monitor = win::monitor_of(owner);
    (monitor != 0).then(|| DisplayId::new(monitor as u64)).filter(|d| cx.find_display(*d).is_some())
}

pub(crate) fn window_options(title: String, width: f32, display: Option<DisplayId>, acrylic: bool, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display, size(px(width), px(320.0)), cx))),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), appears_transparent: true, ..Default::default() }),
        focus: true,
        show: false,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        display_id: display,
        window_background: if acrylic {
            WindowBackgroundAppearance::AcrylicBackdrop
        } else {
            WindowBackgroundAppearance::Opaque
        },
        inactive_frame_interval: None,
        ..Default::default()
    }
}

pub(crate) fn prepare(hwnd: isize, owner: isize, dark: Option<bool>) {
    win::style(hwnd, owner);
    if let Some(dark) = dark {
        win::set_dark(hwnd, dark);
    }
}

/// Out of any `App` update, as every resize must be (popout.rs
/// `warm_small`): sizes the window to its card (`height`, from its last
/// frame), centres it on its owner, and draws it at that size.
pub(crate) async fn fit(handle: AnyWindowHandle, hwnd: isize, owner: isize, width: f32, height: &Cell<f32>, cx: &mut AsyncApp) {
    for _ in 0..4 {
        let Ok(scale) = handle.update(cx, |_, window, _| window.scale_factor()) else { return };
        let (w, h) = ((width * scale).round() as i32, (height.get().max(1.0) * scale).round() as i32);
        let Some(want) = win::placement(hwnd, owner, w, h) else { break };
        if popout_win::window_rect(hwnd) == Some(want) {
            break;
        }
        popout_win::set_rect(hwnd, want);
        popout_win::paint_now(hwnd);
    }
}

pub(crate) async fn refit(handle: AnyWindowHandle, hwnd: isize, width: f32, height: &Cell<f32>, cx: &mut AsyncApp) {
    // The frame with the new content first: `height` is the last frame's.
    popout_win::paint_now(hwnd);
    for _ in 0..4 {
        let Ok(scale) = handle.update(cx, |_, window, _| window.scale_factor()) else { return };
        let (w, h) = ((width * scale).round() as i32, (height.get().max(1.0) * scale).round() as i32);
        let Some(want) = win::regrown(hwnd, w, h) else { break };
        if popout_win::window_rect(hwnd) == Some(want) {
            break;
        }
        popout_win::set_rect(hwnd, want);
        popout_win::paint_now(hwnd);
    }
}

pub(crate) fn disable_owner(owner: isize) -> bool {
    if owner == 0 {
        return false;
    }
    win::enable(owner, false);
    true
}

/// Enables the owner again, and hands it the activation when the dialog had
/// it: an owned window that is destroyed hands the activation to its owner
/// only when the owner can take it.
pub(crate) fn release_owner(owner: isize, own: isize) {
    win::enable(owner, true);
    if popout_win::is_foreground(own) {
        win::activate(owner);
    }
}

pub(crate) static PREVIEW_SOLID: AtomicBool = AtomicBool::new(false);
pub(crate) static PREVIEW_ACCENT: OnceLock<Accent> = OnceLock::new();
pub(crate) static PREVIEW_ICON: OnceLock<String> = OnceLock::new();

pub(crate) struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

pub(crate) fn preview_owner(title: &str, cx: &mut App) -> isize {
    let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title.to_string().into()), ..Default::default() }),
            window_background: WindowBackgroundAppearance::MicaBackdrop,
            ..Default::default()
        },
        |_, cx| cx.new(|_| Blank),
    )
    .ok()
    .and_then(|h| h.update(cx, |_, window, _| hwnd_of(window)).ok().flatten())
    .unwrap_or(0)
}

pub(crate) mod win {
    use super::{Accent, Rect, Rgb};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetMonitorInfoW, MonitorFromPoint,
        MonitorFromRect, MonitorFromWindow, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VkKeyScanW};
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, DrawIconEx, GetClassLongPtrW, IsIconic, IsWindow, IsWindowVisible, LoadImageW,
        SetForegroundWindow, ShowWindow, DI_NORMAL, GCLP_HICON, GCLP_HICONSM, HICON, IMAGE_ICON, LR_DEFAULTCOLOR,
        SW_RESTORE, SW_SHOW,
    };

    fn h(raw: isize) -> HWND {
        HWND(raw as *mut _)
    }

    fn rect(r: RECT) -> Rect {
        [r.left, r.top, r.right, r.bottom]
    }

    pub fn usable(raw: isize) -> bool {
        raw != 0 && unsafe { IsWindow(Some(h(raw))) }.as_bool()
    }

    pub fn enable(raw: isize, on: bool) {
        if usable(raw) {
            let _ = unsafe { EnableWindow(h(raw), on) };
        }
    }

    pub fn activate(raw: isize) {
        if usable(raw) {
            let _ = unsafe { SetForegroundWindow(h(raw)) };
        }
    }

    /// Brings a minimised or hidden window back on screen: an owned dialog
    /// is hidden with its owner, so the owner must be up for it to be seen.
    pub fn restore(raw: isize) {
        if !usable(raw) {
            return;
        }
        unsafe {
            if IsIconic(h(raw)).as_bool() {
                let _ = ShowWindow(h(raw), SW_RESTORE);
            } else if !IsWindowVisible(h(raw)).as_bool() {
                let _ = ShowWindow(h(raw), SW_SHOW);
            }
        }
    }

    pub fn monitor_of(owner: isize) -> isize {
        let m = if owner != 0 {
            unsafe { MonitorFromWindow(h(owner), MONITOR_DEFAULTTONEAREST) }
        } else {
            unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) }
        };
        m.0 as isize
    }

    fn work_area(of: Rect) -> Option<Rect> {
        let r = RECT { left: of[0], top: of[1], right: of[2], bottom: of[3] };
        let m = unsafe { MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe { GetMonitorInfoW(m, &mut info) }.as_bool().then(|| rect(info.rcWork))
    }

    /// The owner as the user sees it: DWM's frame, without the invisible
    /// resize borders. A minimised or hidden owner is no anchor: then the
    /// dialog is centred on its monitor (the primary one without an owner).
    fn anchor(owner: isize) -> Option<Rect> {
        if owner != 0 && unsafe { IsWindowVisible(h(owner)).as_bool() && !IsIconic(h(owner)).as_bool() } {
            let mut r = RECT::default();
            let ok = unsafe {
                DwmGetWindowAttribute(
                    h(owner),
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut r as *mut _ as *mut _,
                    std::mem::size_of::<RECT>() as u32,
                )
            };
            if ok.is_ok() {
                return Some(rect(r));
            }
            return super::popout_win::window_rect(owner);
        }
        let m = monitor_of(owner);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe { GetMonitorInfoW(HMONITOR(m as *mut _), &mut info) }
            .as_bool()
            .then(|| rect(info.rcWork))
    }

    fn around(raw: isize, anchor: Rect, w: i32, h: i32) -> Option<Rect> {
        let outer = super::popout_win::window_rect(raw)?;
        let inner = super::popout_win::client_rect_on_screen(raw)?;
        let border = [inner[0] - outer[0], inner[1] - outer[1], outer[2] - inner[2], outer[3] - inner[3]];
        let work = work_area(anchor).unwrap_or(anchor);
        let (x, y) = super::center_in(anchor, work, w, h);
        Some([x - border[0], y - border[1], x + w + border[2], y + h + border[3]])
    }

    pub fn placement(raw: isize, owner: isize, w: i32, h: i32) -> Option<Rect> {
        around(raw, anchor(owner)?, w, h)
    }

    pub fn regrown(raw: isize, w: i32, h: i32) -> Option<Rect> {
        around(raw, super::popout_win::client_rect_on_screen(raw)?, w, h)
    }

    pub fn style(raw: isize, owner: isize) {
        if owner != 0 {
            super::popout_win::make_owned(raw, owner);
        }
        let corner = DWMWCP_ROUND;
        unsafe {
            let _ = DwmSetWindowAttribute(
                h(raw),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const _,
                std::mem::size_of_val(&corner) as u32,
            );
        }
    }

    pub fn set_dark(raw: isize, dark: bool) {
        let on = windows::core::BOOL::from(dark);
        unsafe {
            let _ = DwmSetWindowAttribute(
                h(raw),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &on as *const _ as *const _,
                std::mem::size_of_val(&on) as u32,
            );
        }
    }

    pub fn accent() -> Option<Accent> {
        use windows::UI::ViewManagement::{UIColorType, UISettings};
        let s = UISettings::new().ok()?;
        let get = |t| s.GetColorValue(t).ok().map(|c| Rgb(c.R, c.G, c.B));
        Some(Accent { light_theme: get(UIColorType::AccentDark1)?, dark_theme: get(UIColorType::AccentLight2)? })
    }

    pub fn latin_key_of(c: char) -> Option<char> {
        let mut buf = [0u16; 2];
        let [unit] = c.encode_utf16(&mut buf) else { return None };
        let scan = unsafe { VkKeyScanW(*unit) };
        if scan == -1 {
            return None;
        }
        let vk = (scan & 0xff) as u8;
        (vk.is_ascii_uppercase() || vk.is_ascii_digit()).then(|| (vk as char).to_ascii_lowercase())
    }

    pub fn icon_png(owner: isize, own: isize, px: i32) -> Option<Vec<u8>> {
        let px = px.clamp(16, 256);
        let module = unsafe { GetModuleHandleW(None) }.ok()?;
        let loaded = match super::PREVIEW_ICON.get() {
            Some(exe) => from_file(exe, px),
            None => unsafe { LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, px, px, LR_DEFAULTCOLOR) }
                .ok()
                .map(|h| HICON(h.0)),
        };
        let icon = loaded.or_else(|| {
            [owner, own].into_iter().filter(|&w| w != 0).find_map(|w| {
                let small = unsafe { GetClassLongPtrW(h(w), GCLP_HICONSM) };
                let big = unsafe { GetClassLongPtrW(h(w), GCLP_HICON) };
                let i = if small != 0 { small } else { big };
                (i != 0).then(|| HICON(i as *mut _))
            })
        })?;
        let png = draw(icon, px);
        if let Some(i) = loaded {
            let _ = unsafe { DestroyIcon(i) };
        }
        png
    }

    fn from_file(exe: &str, px: i32) -> Option<HICON> {
        use windows::Win32::UI::WindowsAndMessaging::PrivateExtractIconsW;
        let mut path = [0u16; 260];
        for (d, s) in path.iter_mut().zip(exe.encode_utf16().take(259)) {
            *d = s;
        }
        let mut icon = [HICON::default()];
        let n = unsafe { PrivateExtractIconsW(&path, 0, px, px, Some(&mut icon), None, 0) };
        (n == 1 && !icon[0].is_invalid()).then_some(icon[0])
    }

    fn draw(icon: HICON, px: i32) -> Option<Vec<u8>> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: px,
                    biHeight: -px,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, dib.into());
            let drawn = DrawIconEx(dc, 0, 0, icon, px, px, 0, None, DI_NORMAL).is_ok();
            let n = (px * px) as usize;
            let bgra = std::slice::from_raw_parts(bits as *const u8, n * 4).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(dc);
            if !drawn {
                return None;
            }
            // GDI leaves premultiplied BGRA; an icon without alpha leaves 0.
            let has_alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
            let mut rgba = Vec::with_capacity(n * 4);
            for p in bgra.chunks_exact(4) {
                let a = if has_alpha { p[3] } else { 255 };
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
                let (r, g, b) = if has_alpha { (un(p[2]), un(p[1]), un(p[0])) } else { (p[2], p[1], p[0]) };
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            let image = image::RgbaImage::from_raw(px as u32, px as u32, rgba)?;
            let mut png = Vec::new();
            image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            Some(png)
        }
    }
}
