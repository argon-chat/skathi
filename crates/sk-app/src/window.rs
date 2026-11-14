//! Window state and chrome: browser-standard getters wired to real window state, `window-drag`,
//! `navigator.windowControlsOverlay` and the `appWindow` extension.
//!
//! [`snapshot`] reads the window once per frame and [`pump`] diffs it into events in a fixed order
//! (resize, visibility, JS observer). Getters read the published snapshot; mutators queue when no
//! `Phase::Commit` is live.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use gpui::{Window, WindowBackgroundAppearance};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use sk_runtime::redjs::api::Value;
use sk_runtime::redjs::webapi::util::read_string;
use sk_runtime::{host_fn, with_commit, JsRuntime};

use crate::json::{self, Json};

const UNDEF: u64 = 0xa;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBarStyle {
    /// OS-drawn chrome (`WS_CAPTION`); `window-drag` / `--window-control` are inert.
    Default,
    /// The app draws the titlebar. The default.
    Hidden,
}

/// AppKit's 14px button row is centred in the app's strip (AppKit resets it on fullscreen transitions).
pub(crate) fn traffic_light_position(
    custom_titlebar: bool,
    height: f32,
) -> Option<gpui::Point<gpui::Pixels>> {
    (cfg!(target_os = "macos") && custom_titlebar)
        .then(|| gpui::point(gpui::px(12.0), gpui::px(((height - 14.0) / 2.0).max(0.0))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    Mica,
    MicaAlt,
    Acrylic,
    Transparent,
}

impl Backdrop {
    fn from_str(s: &str) -> Option<(Backdrop, Option<&'static str>)> {
        Some(match s {
            "none" | "opaque" => (Backdrop::None, None),
            "mica" => (Backdrop::Mica, None),
            "mica-alt" | "tabbed" => (Backdrop::MicaAlt, None),
            "acrylic" => (Backdrop::Acrylic, None),
            // Windows has one blurred material: `blur` and `acrylic` are the same pixels.
            "blur" => (Backdrop::Acrylic, Some(A20)),
            "transparent" => (Backdrop::Transparent, None),
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Backdrop::None => "none",
            Backdrop::Mica => "mica",
            Backdrop::MicaAlt => "mica-alt",
            Backdrop::Acrylic => "acrylic",
            Backdrop::Transparent => "transparent",
        }
    }

    pub fn to_gpui(self) -> WindowBackgroundAppearance {
        match self {
            Backdrop::None => WindowBackgroundAppearance::Opaque,
            // macOS has one system material (behind-window blur).
            #[cfg(target_os = "macos")]
            Backdrop::Mica | Backdrop::MicaAlt | Backdrop::Acrylic => {
                WindowBackgroundAppearance::Blurred
            }
            // gpui's `Blurred` is KDE-only: opaque, reads as `none` ([`effective_backdrop`]).
            #[cfg(target_os = "linux")]
            Backdrop::Mica | Backdrop::MicaAlt | Backdrop::Acrylic => {
                WindowBackgroundAppearance::Opaque
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            Backdrop::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            Backdrop::Acrylic => WindowBackgroundAppearance::AcrylicBackdrop,
            Backdrop::Transparent => WindowBackgroundAppearance::Transparent,
        }
    }

    /// A `<SkWindow backdrop>` name resolved for this OS; advisories print through [`advise`].
    pub fn parse(name: &str) -> Option<Backdrop> {
        let (want, note) = Backdrop::from_str(name)?;
        if let Some(n) = note {
            advise(n);
        }
        let (resolved, os_note) = want.resolve_for_os();
        if let Some(n) = os_note {
            advise(n);
        }
        Some(resolved)
    }

    fn resolve_for_os(self) -> (Backdrop, Option<&'static str>) {
        let (backdrop, note) = resolve_backdrop(self, has_system_backdrop());
        // The notes name Windows builds.
        (backdrop, note.filter(|_| cfg!(target_os = "windows")))
    }
}

/// DWM draws system backdrops from build 22621; before that Mica advises and falls back to acrylic.
fn resolve_backdrop(b: Backdrop, system_backdrop: bool) -> (Backdrop, Option<&'static str>) {
    match b {
        Backdrop::Mica | Backdrop::MicaAlt if !system_backdrop => (Backdrop::Acrylic, Some(A20B)),
        Backdrop::Acrylic if !system_backdrop => (Backdrop::Acrylic, Some(A20C)),
        other => (other, None),
    }
}

/// `appWindow.backdrop`, unless gpui stands in for the material (e.g. macOS fullscreen has no desktop picture to blur).
pub(crate) fn effective_backdrop(b: Backdrop, drawn: bool) -> Backdrop {
    match b {
        Backdrop::Mica | Backdrop::MicaAlt | Backdrop::Acrylic if !drawn => Backdrop::None,
        other => other,
    }
}

/// `appWindow.theme`: drives DWM dark mode and `prefers-color-scheme`; `System` follows the OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    fn from_str(s: &str) -> Option<Theme> {
        Some(match s {
            "system" => Theme::System,
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    fn is_dark(self, os_dark: bool) -> bool {
        match self {
            Theme::System => os_dark,
            Theme::Light => false,
            Theme::Dark => true,
        }
    }
}

const A20: &str = "`backdrop: \"blur\"` resolves to `acrylic` here — Windows has one blurred \
                   material. Use `acrylic` to say so explicitly.";
const A20B: &str = "Mica needs Windows 11 22H2 (build 22621); falling back to `acrylic`, which \
                    is the older accent-policy blur on this Windows.";
const A20C: &str = "`acrylic` before Windows 11 22H2 (build 22621) is the older accent-policy \
                    blur: it has no light or dark tint of its own, and it has known compositor \
                    cost while resizing and dragging. 22H2 and later draw DWM's own acrylic.";

/// `titleBarOverlay`: host-drawn caption buttons for custom chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    /// `None` = transparent.
    pub color: Option<u32>,
    /// `None` follows the window appearance.
    pub symbol_color: Option<u32>,
}

/// Win11 caption-button metrics (46 epx wide, three buttons), hardcoded.
pub const OVERLAY_BUTTON_WIDTH: f32 = 46.0;
pub const OVERLAY_BUTTONS: f32 = 3.0;

impl Default for Overlay {
    fn default() -> Self {
        Overlay {
            color: None,
            symbol_color: None,
        }
    }
}

impl Overlay {
    /// Cluster width in CSS px, subtracted by `getTitlebarAreaRect()` and `env(titlebar-area-width)`.
    pub fn width() -> f32 {
        OVERLAY_BUTTON_WIDTH * OVERLAY_BUTTONS
    }
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` to `0xAARRGGBB`; other forms are refused, not painted black.
fn parse_color(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#')?;
    let n = |i: usize, len: usize| u32::from_str_radix(&h[i..i + len], 16).ok();
    let dup = |v: u32| v * 17; // `f` -> `ff`
    Some(match h.len() {
        3 => 0xff00_0000 | (dup(n(0, 1)?) << 16) | (dup(n(1, 1)?) << 8) | dup(n(2, 1)?),
        6 => 0xff00_0000 | n(0, 6)?,
        8 => (n(6, 2)? << 24) | n(0, 6)?,
        _ => return None,
    })
}

fn color_of(v: &Json, key: &str, advisories: &mut Vec<String>) -> Option<u32> {
    let s = v.get(key).and_then(Json::as_str)?;
    match parse_color(s) {
        Some(c) => Some(c),
        None => {
            advisories.push(format!(
                "`titleBarOverlay.{key}: {s:?}` is not a hex colour — expected `#rgb`, `#rrggbb` \
                 or `#rrggbbaa`. Ignored."
            ));
            None
        }
    }
}

/// The `window` block of `sk.config.json`.
#[derive(Clone, Debug)]
pub struct Config {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub frame: bool,
    pub title_bar_style: TitleBarStyle,
    pub title_bar_height: f32,
    /// `true` keeps the built-in titlebar until the app paints a drag region.
    pub title_bar_fallback: bool,
    pub title_bar_overlay: Option<Overlay>,
    pub backdrop: Backdrop,
    pub resizable: bool,
    pub minimizable: bool,
    pub maximizable: bool,
    pub always_on_top: bool,
    /// `window.focus()`: `false` = `request_attention` (never steals focus), `true` = `activate_window`.
    pub focus_raise: bool,
    /// `false` means no listener, handshake file or F12, since the port carries `eval`. `SK_DEVTOOLS=1` overrides it.
    pub devtools: bool,
    /// Whether F12 opens the inspector; only `SK_DEVTOOLS_KEY=off` clears it.
    pub devtools_key: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            title: "Skathi".into(),
            width: 640.0,
            height: 420.0,
            min_width: None,
            min_height: None,
            frame: true,
            title_bar_style: TitleBarStyle::Hidden,
            title_bar_height: 32.0,
            title_bar_fallback: true,
            title_bar_overlay: None,
            backdrop: Backdrop::Mica,
            resizable: true,
            minimizable: true,
            maximizable: true,
            always_on_top: false,
            focus_raise: false,
            devtools: true,
            devtools_key: true,
        }
    }
}

thread_local! {
    static CONFIG: RefCell<Config> = RefCell::new(Config::default());
}

pub fn config() -> Config {
    CONFIG.with(|c| c.borrow().clone())
}

#[cfg(sk_devtools)]
pub fn devtools_config_enabled() -> bool {
    CONFIG.with(|c| c.borrow().devtools)
}

#[cfg(sk_devtools)]
pub fn devtools_key_enabled() -> bool {
    CONFIG.with(|c| c.borrow().devtools_key)
}

fn f32_of(v: &Json, key: &str) -> Option<f32> {
    v.get(key).and_then(Json::as_f64).map(|n| n as f32)
}

fn bool_of(v: &Json, key: &str) -> Option<bool> {
    match v.get(key) {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// Reads the `window` block, applies the `SK_WINDOW*` overrides and stashes the result. Called
/// before `cx.open_window`: creation-time gpui options cannot change without a visible flash.
pub fn load_config(raw: Option<String>) -> Config {
    let mut cfg = Config::default();
    let mut advisories: Vec<String> = Vec::new();
    let mut asked_for_fallback = false;

    if let Some(v) = raw.as_deref().and_then(json::parse) {
        if let Some(t) = v.get("title").and_then(Json::as_str) {
            cfg.title = t.to_string();
        }
        if let Some(n) = f32_of(&v, "width") {
            cfg.width = n;
        }
        if let Some(n) = f32_of(&v, "height") {
            cfg.height = n;
        }
        cfg.min_width = f32_of(&v, "minWidth");
        cfg.min_height = f32_of(&v, "minHeight");
        if let Some(b) = bool_of(&v, "frame") {
            cfg.frame = b;
            if !b {
                advisories.push(A18.to_string());
            }
        }
        match v.get("titleBarStyle").and_then(Json::as_str) {
            Some("default") => cfg.title_bar_style = TitleBarStyle::Default,
            Some("hidden") | None => cfg.title_bar_style = TitleBarStyle::Hidden,
            Some("hiddenInset") => {
                advisories.push(
                    "`titleBarStyle: \"hiddenInset\"` is macOS-only; treated as `\"hidden\"` on \
                     Windows."
                        .into(),
                );
                cfg.title_bar_style = TitleBarStyle::Hidden;
            }
            Some(other) => advisories.push(format!(
                "unknown `titleBarStyle`: {other:?} — expected \"default\" | \"hidden\" | \
                 \"hiddenInset\"; using \"hidden\"."
            )),
        }
        if let Some(n) = f32_of(&v, "titleBarHeight") {
            cfg.title_bar_height = n.max(0.0);
        }
        if let Some(b) = bool_of(&v, "titleBarFallback") {
            cfg.title_bar_fallback = b;
            asked_for_fallback = b;
        }
        match v.get("titleBarOverlay") {
            None | Some(Json::Bool(false)) | Some(Json::Null) => {}
            Some(Json::Bool(true)) => cfg.title_bar_overlay = Some(Overlay::default()),
            Some(o @ Json::Obj(_)) => {
                if let Some(n) = f32_of(o, "height") {
                    cfg.title_bar_height = n.max(0.0);
                }
                cfg.title_bar_overlay = Some(Overlay {
                    color: color_of(o, "color", &mut advisories),
                    symbol_color: color_of(o, "symbolColor", &mut advisories),
                });
            }
            Some(_) => advisories.push(
                "`titleBarOverlay` expects `false` or an object `{ height?, color?, symbolColor? }`."
                    .into(),
            ),
        }
        if let Some(s) = v.get("backdrop").and_then(Json::as_str) {
            match Backdrop::from_str(s) {
                Some((b, note)) => {
                    cfg.backdrop = b;
                    advisories.extend(note.map(str::to_string));
                }
                None => advisories.push(format!(
                    "unknown `backdrop`: {s:?} — expected mica | mica-alt | tabbed | acrylic | \
                     blur | transparent | none."
                )),
            }
        }
        for (key, slot) in [
            ("resizable", &mut cfg.resizable),
            ("minimizable", &mut cfg.minimizable),
            ("maximizable", &mut cfg.maximizable),
            ("alwaysOnTop", &mut cfg.always_on_top),
        ] {
            if let Some(b) = bool_of(&v, key) {
                *slot = b;
            }
        }
        if let Some("raise") = v.get("focus").and_then(Json::as_str) {
            cfg.focus_raise = true;
        }
        if let Some(b) = bool_of(&v, "devtools") {
            cfg.devtools = b;
        }
    }

    // Environment wins over the config.
    if let Some((w, h)) = std::env::var("SK_WINDOW").ok().and_then(|v| {
        let (w, h) = v.split_once(['x', 'X'])?;
        Some((w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?))
    }) {
        cfg.width = w;
        cfg.height = h;
    }
    if std::env::var("SK_WINDOW_FOCUS").is_ok_and(|v| v == "raise") {
        cfg.focus_raise = true;
    }
    // `SK_DEVTOOLS_KEY=off` drops only F12; `SK_DEVTOOLS=0` / `"devtools": false` also drop the port.
    match std::env::var("SK_DEVTOOLS_KEY").as_deref() {
        Ok("off" | "none" | "0") => cfg.devtools_key = false,
        Ok("f12" | "on" | "1") => cfg.devtools_key = true,
        _ => {}
    }

    // OS-drawn chrome owns the top strip, so the fallback titlebar is forced off.
    if cfg.title_bar_style == TitleBarStyle::Default {
        if asked_for_fallback {
            advisories.push(A26.to_string());
        }
        cfg.title_bar_fallback = false;
    }

    let (backdrop, note) = cfg.backdrop.resolve_for_os();
    cfg.backdrop = backdrop;
    advisories.extend(note.map(str::to_string));

    if cfg!(target_os = "windows") && cfg.min_width.is_some_and(|w| w > 500.0) {
        advisories.push(format!(
            "`minWidth: {}` exceeds the 500 epx ceiling Windows 11 snap layouts require: the \
             flyout will still appear but the zones will reject. Microsoft recommends <= 330.",
            cfg.min_width.unwrap_or_default()
        ));
    }

    for a in advisories {
        advise(&a);
    }
    CONFIG.with(|c| *c.borrow_mut() = cfg.clone());
    cfg
}

const A18: &str =
    "`frame: false` is treated as `titleBarStyle: \"hidden\"` in v1. A true frameless \
                   window would drop WS_THICKFRAME, which also removes the drop shadow, window \
                   animations and edge resizing.";
const A26: &str =
    "`titleBarFallback: true` is ignored under `titleBarStyle: \"default\"` — the OS \
                   draws the caption, so a second host-drawn strip underneath it would just eat \
                   32 px of the app. Remove one of the two.";

thread_local! {
    static ADVISED: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
}

/// One per message, ever.
pub fn advise(msg: &str) {
    ADVISED.with(|s| {
        if s.borrow_mut().insert(msg.to_string()) {
            eprintln!("[skathi compat] {msg}");
            sk_tree::log::record("advisory", msg);
        }
    });
}

host_fn!(fn sk_window_advise(g, args) {
    let msg = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    advise(&msg);
    UNDEF
});

// gpui has no `set_position`, runtime min/max size or non-toggling restore, so these verbs reach the
// HWND directly. gpui's bounds cache refreshes from the resulting `WM_MOVE`/`WM_SIZE`: reads lag a frame.

#[cfg(target_os = "windows")]
mod win {
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn hwnd() -> Option<HWND> {
        super::hwnd_raw().map(|h| HWND(h as *mut _))
    }

    /// Frame included; `Window::bounds()` is the client rect.
    pub fn window_rect() -> Option<(f32, f32, f32, f32)> {
        let h = hwnd()?;
        let mut r = RECT::default();
        unsafe { GetWindowRect(h, &mut r) }.ok()?;
        Some((
            r.left as f32,
            r.top as f32,
            (r.right - r.left) as f32,
            (r.bottom - r.top) as f32,
        ))
    }

    pub fn is_zoomed() -> bool {
        hwnd().is_some_and(|h| unsafe { IsZoomed(h) }.as_bool())
    }

    pub fn is_iconic() -> bool {
        hwnd().is_some_and(|h| unsafe { IsIconic(h) }.as_bool())
    }

    /// Independent of minimization or focus; visible before capture.
    pub fn is_visible() -> bool {
        hwnd().is_none_or(|h| unsafe { IsWindowVisible(h) }.as_bool())
    }

    pub fn restore() {
        if let Some(h) = hwnd() {
            let _ = unsafe { ShowWindowAsync(h, SW_RESTORE) };
        }
    }

    pub fn hide() -> bool {
        let Some(h) = hwnd() else { return false };
        let _ = unsafe { ShowWindowAsync(h, SW_HIDE) };
        true
    }

    pub fn set_position(x: f32, y: f32) {
        if let Some(h) = hwnd() {
            let _ = unsafe {
                SetWindowPos(
                    h,
                    None,
                    x.round() as i32,
                    y.round() as i32,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
    }

    pub fn set_outer_size(w: f32, h_: f32) {
        if let Some(h) = hwnd() {
            let _ = unsafe {
                SetWindowPos(
                    h,
                    None,
                    0,
                    0,
                    w.round().max(1.0) as i32,
                    h_.round().max(1.0) as i32,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
    }

    pub fn set_always_on_top(on: bool) {
        if let Some(h) = hwnd() {
            set_always_on_top_hwnd(h, on);
        }
    }

    pub fn set_always_on_top_for(raw: isize, on: bool) {
        set_always_on_top_hwnd(HWND(raw as *mut _), on);
    }

    /// `DWMWA_USE_IMMERSIVE_DARK_MODE` (Mica tint and OS caption colours), as gpui writes on OS theme change.
    pub fn set_dark_mode_for(raw: isize, dark: bool) {
        use windows::core::BOOL;
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
        let on: BOOL = dark.into();
        let _ = unsafe {
            DwmSetWindowAttribute(
                HWND(raw as *mut _),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &on as *const BOOL as *const _,
                std::mem::size_of::<BOOL>() as u32,
            )
        };
    }

    fn set_always_on_top_hwnd(h: HWND, on: bool) {
        let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        // `SWP_NOACTIVATE`: a palette that jumps in front must not steal the caret.
        let _ = unsafe {
            SetWindowPos(
                h,
                Some(after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }

    pub fn start_dragging() {
        if let Some(h) = hwnd() {
            unsafe {
                let _ = ReleaseCapture();
                let _ = SendMessageW(
                    h,
                    WM_NCLBUTTONDOWN,
                    Some(WPARAM(HTCAPTION as usize)),
                    Some(LPARAM(0)),
                );
            }
        }
    }

    /// Its virtual desktop is not current; treated as minimised ([`super::cloaked`]).
    pub fn is_cloaked() -> bool {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        let Some(h) = hwnd() else { return false };
        let mut cloaked: u32 = 0;
        let ok = unsafe {
            DwmGetWindowAttribute(
                h,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
        };
        ok.is_ok() && cloaked != 0
    }

    /// `DWMWA_SYSTEMBACKDROP_TYPE` read back from DWM: what the compositor believes, since system
    /// backdrop and accent policy can disagree. 0 auto, 1 none, 2 mica, 3 acrylic, 4 mica-alt.
    pub fn system_backdrop() -> Option<u32> {
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE};
        let h = hwnd()?;
        let mut v: u32 = 0;
        unsafe {
            DwmGetWindowAttribute(
                h,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &mut v as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
        }
        .ok()?;
        Some(v)
    }

    pub fn dark_mode() -> Option<bool> {
        use windows::core::BOOL;
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
        let h = hwnd()?;
        let mut v = BOOL(0);
        unsafe {
            DwmGetWindowAttribute(
                h,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &mut v as *mut BOOL as *mut _,
                std::mem::size_of::<BOOL>() as u32,
            )
        }
        .ok()?;
        Some(v.as_bool())
    }

    pub fn cursor_pos() -> Option<(f32, f32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok()?;
        Some((p.x as f32, p.y as f32))
    }

    /// The window's monitor plus its rect and work area, in physical px; the display cache key.
    pub fn monitor_key() -> Option<(isize, [i32; 8])> {
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        let h = hwnd()?;
        let m = unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) };
        if m.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            return None;
        }
        let (a, w) = (info.rcMonitor, info.rcWork);
        Some((
            m.0 as isize,
            [
                a.left, a.top, a.right, a.bottom, w.left, w.top, w.right, w.bottom,
            ],
        ))
    }
}

#[cfg(not(target_os = "windows"))]
mod win {
    pub fn window_rect() -> Option<(f32, f32, f32, f32)> {
        None
    }
    pub fn is_zoomed() -> bool {
        false
    }
    pub fn is_iconic() -> bool {
        false
    }
    pub fn is_visible() -> bool {
        true
    }
    pub fn restore() {}
    pub fn hide() -> bool {
        false
    }
    pub fn set_position(_x: f32, _y: f32) {}
    pub fn set_outer_size(_w: f32, _h: f32) {}
    pub fn set_always_on_top(_on: bool) {}
    pub fn set_always_on_top_for(_raw: isize, _on: bool) {}
    pub fn set_dark_mode_for(_raw: isize, _dark: bool) {}
    pub fn start_dragging() {}
    pub fn is_cloaked() -> bool {
        false
    }
    pub fn system_backdrop() -> Option<u32> {
        None
    }
    pub fn dark_mode() -> Option<bool> {
        None
    }
    pub fn cursor_pos() -> Option<(f32, f32)> {
        None
    }
    pub fn monitor_key() -> Option<(isize, [i32; 8])> {
        None
    }
}

thread_local! {
    /// Each window's HWND, captured once; `appWindow.*` means the primary one, a pop-out is driven via [`hwnd_of`].
    static HWND_CACHE: RefCell<std::collections::HashMap<sk_tree::WinKey, isize>> =
        RefCell::new(std::collections::HashMap::new());
    /// Set by [`init`].
    static PRIMARY_WINDOW: Cell<sk_tree::WinKey> = const { Cell::new(0) };
}

fn hwnd_raw() -> Option<isize> {
    let primary = PRIMARY_WINDOW.with(|c| c.get());
    if let Some(cached) = hwnd_of(primary) {
        return Some(cached);
    }
    with_commit(|window, _| capture_hwnd(window)).flatten()
}

pub fn set_always_on_top_hwnd(hwnd: isize, on: bool) {
    win::set_always_on_top_for(hwnd, on);
}

pub fn hwnd_of(key: sk_tree::WinKey) -> Option<isize> {
    HWND_CACHE
        .with(|m| m.borrow().get(&key).copied())
        .filter(|&h| h != 0)
}

pub fn capture_hwnd(window: &Window) -> Option<isize> {
    // gpui's inherent `Window::window_handle()` shadows the trait method, hence UFCS.
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    let raw = match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }?;
    let key = sk_tree::windows::key(window);
    HWND_CACHE.with(|m| m.borrow_mut().insert(key, raw));
    Some(raw)
}

pub fn primary_hwnd() -> Option<isize> {
    hwnd_of(PRIMARY_WINDOW.with(|c| c.get()))
}

/// Device pixels per CSS px (1 before the first snapshot).
pub fn primary_scale() -> f32 {
    let s = SNAPSHOT.with(|s| s.borrow().scale);
    if s > 0.0 {
        s
    } else {
        1.0
    }
}

pub fn forget_window(key: sk_tree::WinKey) {
    HWND_CACHE.with(|m| m.borrow_mut().remove(&key));
    #[cfg(target_os = "macos")]
    NS_WINDOW_CACHE.with(|m| m.borrow_mut().remove(&key));
}

#[cfg(target_os = "macos")]
thread_local! {
    /// A stale pointer is harmless: sk-desktop only uses one found in `[NSApp windows]`.
    static NS_WINDOW_CACHE: RefCell<std::collections::HashMap<sk_tree::WinKey, isize>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The native window a file dialog is owned by: the HWND, or on macOS the `NSWindow`.
pub fn dialog_parent(window: &Window) -> Option<isize> {
    #[cfg(target_os = "macos")]
    return capture_ns_window(window);
    #[cfg(not(target_os = "macos"))]
    return capture_hwnd(window);
}

#[cfg(target_os = "macos")]
fn capture_ns_window(window: &Window) -> Option<isize> {
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    let ns_window = sk_desktop::dialog::ns_window_of_view(h.ns_view.as_ptr())?;
    let key = sk_tree::windows::key(window);
    NS_WINDOW_CACHE.with(|m| m.borrow_mut().insert(key, ns_window));
    Some(ns_window)
}

pub fn dialog_parent_of(key: sk_tree::WinKey) -> Option<isize> {
    #[cfg(target_os = "macos")]
    return NS_WINDOW_CACHE
        .with(|m| m.borrow().get(&key).copied())
        .filter(|&w| w != 0);
    #[cfg(not(target_os = "macos"))]
    return hwnd_of(key);
}

/// Windows 11 22H2 (build 22621)+, where `DWMWA_SYSTEMBACKDROP_TYPE` exists.
#[cfg(target_os = "windows")]
pub(crate) fn has_system_backdrop() -> bool {
    use std::sync::OnceLock;
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        use windows::Wdk::System::SystemServices::RtlGetVersion;
        use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
        let mut vi = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        // `RtlGetVersion` is the only version query that does not lie for an unmanifested process.
        unsafe { RtlGetVersion(&mut vi) }.is_ok() && vi.dwBuildNumber >= 22621
    })
}

/// macOS always has its system material, which every system backdrop maps to.
#[cfg(not(target_os = "windows"))]
pub(crate) fn has_system_backdrop() -> bool {
    cfg!(target_os = "macos")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub inner_width: f32,
    pub inner_height: f32,
    pub outer: (f32, f32, f32, f32),
    pub content: (f32, f32, f32, f32),
    pub scale: f32,
    pub maximized: bool,
    pub minimized: bool,
    pub visible: bool,
    pub fullscreen: bool,
    pub focused: bool,
    pub cloaked: bool,
    pub occluded: bool,
    pub screen: (f32, f32, f32, f32, f32, f32),
    pub displays: usize,
    pub title: String,
    pub backdrop: Backdrop,
    /// What the window draws ([`effective_backdrop`]).
    pub effective_backdrop: Backdrop,
    pub theme: Theme,
    pub always_on_top: bool,
    pub title_bar_height: f32,
    pub fallback_titlebar: bool,
    pub overlay: Option<Overlay>,
    /// Client area in CSS px; `inner_*` is the app root's box inside it (the viewport).
    pub client: (f32, f32),
    /// `(left, top, right, bottom)` host-chrome inset of the app root; zero in custom-chrome mode.
    pub content_inset: (f32, f32, f32, f32),
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            inner_width: 0.0,
            inner_height: 0.0,
            outer: (0.0, 0.0, 0.0, 0.0),
            content: (0.0, 0.0, 0.0, 0.0),
            scale: 1.0,
            maximized: false,
            minimized: false,
            visible: true,
            fullscreen: false,
            focused: false,
            cloaked: false,
            occluded: false,
            screen: (0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            displays: 1,
            title: String::new(),
            backdrop: Backdrop::Mica,
            effective_backdrop: Backdrop::Mica,
            theme: Theme::System,
            always_on_top: false,
            title_bar_height: 32.0,
            fallback_titlebar: true,
            overlay: None,
            client: (0.0, 0.0),
            content_inset: (0.0, 0.0, 0.0, 0.0),
        }
    }
}

thread_local! {
    /// What every getter reads, published once per frame.
    static SNAPSHOT: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    /// DOM visibility, published at window init before app JS. Separate from PREV, whose first funnel still emits boot.
    static PAGE_HIDDEN: Cell<Option<bool>> = const { Cell::new(None) };
    // The HWND stays hidden while its first styled frame is prepared; not a page becoming document.hidden.
    static STARTUP_REVEAL_PENDING: Cell<bool> = const { Cell::new(false) };
    /// The `__sk_window_observe` callback, protected against GC.
    static OBSERVER: Cell<u64> = const { Cell::new(0) };
    /// Mutators that arrived outside `Phase::Commit`; drained at the top of the next frame.
    static PENDING: RefCell<VecDeque<Op>> = const { RefCell::new(VecDeque::new()) };
    /// Set across the dispatch step: mutators queue instead of applying, since the frame funnel could re-enter.
    static IN_DISPATCH: Cell<bool> = const { Cell::new(false) };
    static LIVE: RefCell<Live> = RefCell::new(Live::default());
    /// So the frame funnel re-applies a backdrop only when `appWindow.backdrop` moves.
    static APPLIED_BACKDROP: Cell<Option<Backdrop>> = const { Cell::new(None) };
}

#[derive(Clone, Debug)]
struct Live {
    title: String,
    backdrop: Backdrop,
    /// App-wide; pop-outs follow.
    theme: Theme,
    always_on_top: bool,
    title_bar_height: f32,
    overlay: Option<Overlay>,
    /// `false` once the app paints its own `window-drag: move` region.
    fallback_titlebar: bool,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            title: "Skathi".into(),
            backdrop: Backdrop::Mica,
            theme: Theme::System,
            always_on_top: false,
            title_bar_height: 32.0,
            overlay: None,
            fallback_titlebar: true,
        }
    }
}

/// Seeds the mutable half from the config and publishes the first snapshot (from `open_window`'s
/// builder closure); it must exist before the JS prelude evaluates.
pub fn init(cfg: &Config, window: &mut Window, cx: &mut gpui::App) {
    LIVE.with(|l| {
        *l.borrow_mut() = Live {
            title: cfg.title.clone(),
            backdrop: cfg.backdrop,
            theme: Theme::System,
            always_on_top: cfg.always_on_top,
            title_bar_height: cfg.title_bar_height,
            // OS chrome draws the real buttons.
            overlay: (cfg.title_bar_style == TitleBarStyle::Hidden)
                .then_some(cfg.title_bar_overlay)
                .flatten(),
            fallback_titlebar: cfg.title_bar_fallback,
        }
    });
    APPLIED_BACKDROP.with(|a| a.set(Some(cfg.backdrop)));
    // Before anything per-window is published: `set_os_chrome` and `set_env` address "the primary window".
    let key = sk_tree::windows::key(window);
    PRIMARY_WINDOW.with(|c| c.set(key));
    sk_tree::windows::set_primary(key);
    capture_hwnd(window);
    #[cfg(target_os = "macos")]
    dialog_parent(window);
    // OS chrome: a stray `move` region would hand the OS `HTCAPTION` over client area and swallow clicks.
    sk_tree::set_os_chrome(cfg.title_bar_style == TitleBarStyle::Default);
    if cfg.always_on_top {
        win::set_always_on_top(true);
    }
    // Before the first snapshot, so nothing can be read through a stale cache.
    window
        .observe_window_display(|_, _| invalidate_display())
        .detach();
    cx.observe_displays(|_, _| invalidate_display()).detach();
    let s = snapshot(window, cx);
    SNAPSHOT.with(|c| *c.borrow_mut() = s);
    // So the first restyle knows the real viewport, DPI and chrome mode.
    let dark = effective_dark(window);
    let s = SNAPSHOT.with(|c| c.borrow().clone());
    sk_tree::set_env(css_env(&s, dark, cx.reduce_motion()));
    // A hidden initial window may never get a platform frame; publish now so module-scope document.hidden is right.
    publish_visibility(&s);
    // Visibility and activation on the platform edge: a minimised or covered window paints no frames, so the per-frame diff would miss them.
    window
        .observe_window_visibility(|_, window, cx| platform_changed(window, cx))
        .detach();
    window
        .observe_window_activation(|window, cx| platform_changed(window, cx))
        .detach();
}

/// Runs the frame funnel out of band; the next frame's diff then sees nothing new, so nothing fires twice.
fn platform_changed(window: &mut Window, cx: &mut gpui::App) {
    if sk_runtime::bridge::poisoned()
        // Nested inside a published phase: the frame's own diff picks the change up.
        || sk_runtime::with_app(|_| ()).is_some()
        // Before the first frame: that frame's funnel owns `boot`.
        || PREV.with(|p| p.borrow().is_none())
    {
        return;
    }
    sk_runtime::bridge::with_published(window, cx, sk_runtime::Phase::Commit, || {
        with_commit(|window, cx| pump(window, cx));
        JsRuntime::with(|rt| rt.ctx().drain_microtasks());
    });
}

/// On another virtual desktop. A hidden page like a minimised one, but gpui keeps asking it to
/// paint every vsync, so the pump declines (`crate::pump_frame`).
pub fn cloaked() -> bool {
    win::is_cloaked()
}

/// Every window of the app is out of sight behind other windows: a hidden page, as when cloaked.
/// Cached for [`crate::occlusion::RECHECK`]. Never in a test host or with `SK_OCCLUSION=0`.
pub fn occluded() -> bool {
    thread_local! {
        static LAST: Cell<Option<(std::time::Instant, bool)>> = const { Cell::new(None) };
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("SK_OCCLUSION").map_or(true, |v| v.trim() != "0"))
        || crate::quiet::never_focus()
    {
        return false;
    }
    let Some(main) = hwnd_raw() else { return false };
    let now = std::time::Instant::now();
    if let Some((at, answer)) = LAST.with(Cell::get) {
        // Brought to the front: drawing resumes at once, not on the next recheck.
        if now.saturating_duration_since(at) < crate::occlusion::RECHECK
            && !(answer && is_foreground(main))
        {
            return answer;
        }
    }
    let answer = crate::occlusion::hwnd_covered(main)
        && crate::popout::ids()
            .into_iter()
            .filter_map(hwnd_of)
            .all(crate::occlusion::hwnd_unseen);
    LAST.with(|l| l.set(Some((now, answer))));
    answer
}

/// A window of this process is the foreground window. Not gpui's `active_window` (`GetActiveWindow`), which keeps naming our window after another app took the foreground.
#[cfg(target_os = "windows")]
pub fn app_focused(_cx: &gpui::App) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    pid == std::process::id()
}

#[cfg(not(target_os = "windows"))]
pub fn app_focused(cx: &gpui::App) -> bool {
    cx.active_window().is_some()
}

#[cfg(target_os = "windows")]
fn is_foreground(raw: isize) -> bool {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }.0 as isize == raw
}

#[cfg(not(target_os = "windows"))]
fn is_foreground(_raw: isize) -> bool {
    false
}

pub fn page_hidden() -> bool {
    PAGE_HIDDEN.with(|h| h.get()).unwrap_or(false)
}

/// A background-but-shown window remains visible.
fn hidden(s: &Snapshot) -> bool {
    !s.visible || s.minimized || s.cloaked || s.occluded
}

fn visibility_delta(previous: Option<bool>, snapshot: &Snapshot) -> Option<bool> {
    let hidden = hidden(snapshot);
    (previous != Some(hidden)).then_some(hidden)
}

fn publish_visibility(snapshot: &Snapshot) {
    let Some(hidden) = PAGE_HIDDEN.with(|previous| {
        let hidden = visibility_delta(previous.get(), snapshot)?;
        previous.set(Some(hidden));
        Some(hidden)
    }) else {
        return;
    };
    JsRuntime::with(|rt| unsafe {
        sk_runtime::redjs::ffi::redjs_dom_set_visibility(rt.raw(), hidden as i32)
    });
    sk_tree::set_page_hidden(hidden);
}

/// Whether the host still draws its own titlebar strip; retires once the app paints a `window-drag: move` region.
pub fn fallback_titlebar() -> bool {
    LIVE.with(|l| l.borrow().fallback_titlebar)
}

fn display_mode(fullscreen: bool) -> sk_css::DisplayMode {
    if fullscreen {
        return sk_css::DisplayMode::Fullscreen;
    }
    match config().title_bar_style {
        TitleBarStyle::Default => sk_css::DisplayMode::Standalone,
        // The titlebar strip belongs to content, with or without a host cluster; reused PWA components rely on it.
        TitleBarStyle::Hidden => sk_css::DisplayMode::WindowControlsOverlay,
    }
}

#[derive(Clone)]
struct DisplayPart {
    /// Monitor, its rect and work area (physical px), and the scale factor.
    key: Option<(isize, [i32; 8], u32)>,
    screen: (f32, f32, f32, f32, f32, f32),
    displays: usize,
}

thread_local! {
    /// The primary window's [`DisplayPart`]; `None` = read again.
    static DISPLAY: RefCell<Option<DisplayPart>> = const { RefCell::new(None) };
    static REFRESH_MS: Cell<Option<f32>> = const { Cell::new(None) };
}

fn invalidate_display() {
    DISPLAY.with(|d| *d.borrow_mut() = None);
}

/// Cached; re-read when gpui's display observers fire or the per-frame key moves (a taskbar resize moves the work area without a display event).
fn display_part(window: &Window, cx: &gpui::App) -> DisplayPart {
    let primary = sk_tree::windows::key(window) == PRIMARY_WINDOW.with(|c| c.get());
    let key = if primary {
        win::monitor_key().map(|(m, r)| (m, r, window.scale_factor().to_bits()))
    } else {
        None
    };
    if key.is_some() {
        if let Some(hit) = DISPLAY.with(|d| d.borrow().clone().filter(|c| c.key == key)) {
            return hit;
        }
    }
    let display = window.display(cx);
    let (screen, displays) = match &display {
        Some(d) => {
            let b = d.bounds();
            let v = d.visible_bounds();
            (
                (
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                    f32::from(v.size.width),
                    f32::from(v.size.height),
                    f32::from(v.origin.x),
                    f32::from(v.origin.y),
                ),
                cx.displays().len(),
            )
        }
        None => ((0.0, 0.0, 0.0, 0.0, 0.0, 0.0), 1),
    };
    let part = DisplayPart {
        key,
        screen,
        displays,
    };
    if primary {
        DISPLAY.with(|d| *d.borrow_mut() = key.is_some().then(|| part.clone()));
        REFRESH_MS.with(|c| {
            c.set(
                window
                    .refresh_interval(cx)
                    .map(|r| r.as_secs_f32() * 1000.0),
            )
        });
    }
    part
}

/// The primary window's display refresh interval in ms, one frame's budget; `None` before the first snapshot.
pub fn frame_budget_ms() -> Option<f32> {
    REFRESH_MS.with(|c| c.get())
}

pub(crate) fn begin_startup_reveal() {
    STARTUP_REVEAL_PENDING.with(|pending| pending.set(true));
}

pub(crate) fn cancel_startup_reveal() {
    STARTUP_REVEAL_PENDING.with(|pending| pending.set(false));
}

fn startup_visibility(native_visible: bool) -> bool {
    STARTUP_REVEAL_PENDING.with(|pending| {
        if native_visible {
            pending.set(false);
            true
        } else {
            pending.get()
        }
    })
}

/// Reads the window once: every event and getter derives from this, so nothing observes two frames at once.
pub fn snapshot(window: &Window, cx: &gpui::App) -> Snapshot {
    let viewport = window.viewport_size();
    let bounds = window.bounds();
    let live = LIVE.with(|l| l.borrow().clone());
    let DisplayPart {
        screen, displays, ..
    } = display_part(window, cx);
    let scale = window.scale_factor();
    let fullscreen = window.is_fullscreen();
    // `GetWindowRect` is physical px; everything else here is CSS px.
    let outer = win::window_rect()
        .map(|(x, y, w, h)| (x / scale, y / scale, w / scale, h / scale))
        .unwrap_or((
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ));
    // `innerWidth`/`innerHeight` are the app root's painted box, not the client area: host chrome is UA chrome, and the client area would disagree with `getBoundingClientRect()`. Before the first painted frame the client area is used.
    let client = (f32::from(viewport.width), f32::from(viewport.height));
    let (inner, inset) = match sk_tree::root_bounds() {
        Some((x, y, w, h)) if w > 0.0 && h > 0.0 => {
            ((w, h), (x, y, client.0 - x - w, client.1 - y - h))
        }
        _ => (client, (0.0, 0.0, 0.0, 0.0)),
    };
    Snapshot {
        inner_width: inner.0,
        inner_height: inner.1,
        client,
        content_inset: inset,
        outer,
        content: (
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ),
        scale,
        maximized: win::is_zoomed() || window.is_maximized(),
        minimized: win::is_iconic(),
        visible: startup_visibility(win::is_visible()),
        fullscreen,
        focused: window.is_window_active(),
        cloaked: win::is_cloaked(),
        occluded: occluded(),
        screen,
        displays,
        title: live.title,
        backdrop: live.backdrop,
        effective_backdrop: effective_backdrop(
            live.backdrop,
            window.is_background_appearance_drawn() && !cfg!(target_os = "linux"),
        ),
        theme: live.theme,
        always_on_top: live.always_on_top,
        title_bar_height: live.title_bar_height,
        fallback_titlebar: live.fallback_titlebar,
        overlay: live.overlay,
    }
}

/// The Window Controls Overlay rect (`env(titlebar-area-*)`); zero with `visible: false` when the strip is not content's.
fn titlebar_area(s: &Snapshot) -> (bool, (f32, f32, f32, f32)) {
    let visible = display_mode(s.fullscreen) == sk_css::DisplayMode::WindowControlsOverlay
        && !s.fullscreen
        && !s.minimized
        && !s.fallback_titlebar;
    if !visible {
        return (false, (0.0, 0.0, 0.0, 0.0));
    }
    let cluster = if s.overlay.is_some() {
        Overlay::width()
    } else {
        0.0
    };
    // Caption buttons sit on the right: the area keeps `x = 0` and loses width.
    (
        true,
        (
            0.0,
            0.0,
            (s.inner_width - cluster).max(0.0),
            s.title_bar_height,
        ),
    )
}

fn css_env(s: &Snapshot, dark: bool, reduce_motion: bool) -> sk_css::Env {
    let (visible, t) = titlebar_area(s);
    sk_css::Env {
        viewport_width: s.inner_width,
        viewport_height: s.inner_height,
        dark,
        reduce_motion,
        scale_factor: s.scale,
        display_mode: display_mode(s.fullscreen),
        titlebar_area: visible.then_some([t.0, t.1, t.2, t.3]),
        coarse_pointer: coarse_pointer(),
        safe_area_insets: [0.0; 4],
    }
}

fn os_dark(window: &Window) -> bool {
    matches!(
        window.appearance(),
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    )
}

/// What `prefers-color-scheme` answers: `appWindow.theme`, or the OS's setting under `system`.
pub fn effective_dark(window: &Window) -> bool {
    LIVE.with(|l| l.borrow().theme).is_dark(os_dark(window))
}

thread_local! {
    static OS_DARK_SEEN: RefCell<std::collections::HashMap<sk_tree::WinKey, bool>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Keeps a window's DWM dark mode on `appWindow.theme`. gpui rewrites the OS appearance on an OS
/// theme change, which would undo a forced theme, so the first frame after a change (a window's
/// first included) writes the forced one back. Nothing is written under `system`.
fn keep_theme(window: &Window) {
    let key = sk_tree::windows::key(window);
    let Some(hwnd) = hwnd_of(key) else { return };
    let os = os_dark(window);
    if OS_DARK_SEEN.with(|m| m.borrow_mut().insert(key, os)) == Some(os) {
        return;
    }
    let theme = LIVE.with(|l| l.borrow().theme);
    if theme != Theme::System {
        win::set_dark_mode_for(hwnd, theme.is_dark(os));
    }
}

fn apply_theme(theme: Theme, window: &Window) {
    LIVE.with(|l| l.borrow_mut().theme = theme);
    let dark = theme.is_dark(os_dark(window));
    let hwnds: Vec<isize> =
        HWND_CACHE.with(|m| m.borrow().values().copied().filter(|&h| h != 0).collect());
    for h in hwnds {
        win::set_dark_mode_for(h, dark);
    }
}

/// The CSS environment of a `<SkWindow>` pop-out; `viewport` is the box its painted root was laid out at.
pub fn popout_env(
    window: &Window,
    cx: &gpui::App,
    viewport: (f32, f32),
    os_chrome: bool,
    title_bar_height: f32,
) -> sk_css::Env {
    keep_theme(window);
    let dark = effective_dark(window);
    let display_mode = if os_chrome {
        sk_css::DisplayMode::Standalone
    } else {
        sk_css::DisplayMode::WindowControlsOverlay
    };
    let titlebar_area = (!os_chrome).then_some([0.0, 0.0, viewport.0, title_bar_height]);
    sk_css::Env {
        viewport_width: viewport.0,
        viewport_height: viewport.1,
        dark,
        // An OS preference; a second pushed value would stomp the one animator.
        reduce_motion: cx.reduce_motion(),
        scale_factor: window.scale_factor(),
        display_mode,
        titlebar_area,
        coarse_pointer: coarse_pointer(),
        safe_area_insets: [0.0; 4],
    }
}

/// `any-pointer: coarse`: a touch screen or pen digitizer is attached (`SM_DIGITIZER`, as Chromium). Cached.
fn coarse_pointer() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            const SM_DIGITIZER: i32 = 94;
            const ANY_DIGITIZER: i32 = 0x01 | 0x02 | 0x04 | 0x08;
            // SAFETY: a pure read of a system metric; no pointers involved.
            let value = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX(SM_DIGITIZER),
                )
            };
            value & ANY_DIGITIZER != 0
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    })
}

fn rect(t: (f32, f32, f32, f32)) -> Json {
    Json::obj(vec![
        ("x", Json::n(t.0)),
        ("y", Json::n(t.1)),
        ("width", Json::n(t.2)),
        ("height", Json::n(t.3)),
    ])
}

fn snapshot_json(s: &Snapshot) -> String {
    let mode = display_mode(s.fullscreen);
    let (wco_visible, titlebar) = titlebar_area(s);
    let cfg = config();
    Json::obj(vec![
        ("innerWidth", Json::n(s.inner_width)),
        ("innerHeight", Json::n(s.inner_height)),
        ("outer", rect(s.outer)),
        ("content", rect(s.content)),
        ("devicePixelRatio", Json::n(s.scale)),
        ("maximized", Json::Bool(s.maximized)),
        ("minimized", Json::Bool(s.minimized)),
        ("visible", Json::Bool(s.visible)),
        ("fullscreen", Json::Bool(s.fullscreen)),
        ("focused", Json::Bool(s.focused)),
        ("cloaked", Json::Bool(s.cloaked)),
        ("occluded", Json::Bool(s.occluded)),
        ("title", Json::s(s.title.clone())),
        ("backdrop", Json::s(s.backdrop.as_str())),
        ("effectiveBackdrop", Json::s(s.effective_backdrop.as_str())),
        ("theme", Json::s(s.theme.as_str())),
        (
            "systemDarkMode",
            match win::dark_mode() {
                Some(v) => Json::Bool(v),
                None => Json::Null,
            },
        ),
        (
            "systemBackdrop",
            match win::system_backdrop() {
                Some(v) => Json::n(v as f64),
                None => Json::Null,
            },
        ),
        ("alwaysOnTop", Json::Bool(s.always_on_top)),
        ("displayMode", Json::s(mode.as_str())),
        (
            "titleBarStyle",
            Json::s(match cfg.title_bar_style {
                TitleBarStyle::Default => "default",
                TitleBarStyle::Hidden => "hidden",
            }),
        ),
        ("titleBarOverlayVisible", Json::Bool(wco_visible)),
        ("titleBarArea", rect(titlebar)),
        ("fallbackTitleBar", Json::Bool(s.fallback_titlebar)),
        ("hostControls", Json::Bool(s.overlay.is_some())),
        ("clientWidth", Json::n(s.client.0)),
        ("clientHeight", Json::n(s.client.1)),
        (
            "contentInset",
            Json::obj(vec![
                ("left", Json::n(s.content_inset.0)),
                ("top", Json::n(s.content_inset.1)),
                ("right", Json::n(s.content_inset.2)),
                ("bottom", Json::n(s.content_inset.3)),
            ]),
        ),
        (
            "focusPolicy",
            Json::s(if crate::quiet::never_focus() {
                "never"
            } else if cfg.focus_raise {
                "raise"
            } else {
                "attention"
            }),
        ),
        (
            "screen",
            Json::obj(vec![
                ("width", Json::n(s.screen.0)),
                ("height", Json::n(s.screen.1)),
                ("availWidth", Json::n(s.screen.2)),
                ("availHeight", Json::n(s.screen.3)),
                ("availLeft", Json::n(s.screen.4)),
                ("availTop", Json::n(s.screen.5)),
                ("isExtended", Json::Bool(s.displays > 1)),
            ]),
        ),
    ])
    .to_json()
}

thread_local! {
    static PREV: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    static BOOTED: Cell<bool> = const { Cell::new(false) };
}

/// Runs inside `pump_frame`'s `Phase::Commit` before `rt.pump`, and from `platform_changed`. Order
/// is the contract: drain queued mutators, snapshot, resize on a viewport/DPI change, visibility,
/// then the JS observer. Resize before the observer makes `fullscreenchange` fire after the
/// viewport has resized, as the Fullscreen spec requires.
pub fn pump(window: &mut Window, cx: &mut gpui::App) {
    drain_pending(window, cx);
    // Before the snapshot so `effectiveBackdrop` describes a queued backdrop.
    sync_backdrop(window, false);

    let s = snapshot(window, cx);
    SNAPSHOT.with(|c| *c.borrow_mut() = s.clone());
    let prev = PREV.with(|p| p.borrow().clone());

    let json = snapshot_json(&s);
    let ctx = JsRuntime::with(|rt| rt.raw());

    // Before anything fires: `redjs_dom_resize` dispatches `resize` synchronously, and its handlers must not see last frame's numbers.
    let dirty = prev.as_ref().is_none_or(|p| *p != s);
    if dirty {
        notify("sync", &json);
    }

    // `innerWidth` etc. are whatever `redjs_dom_resize` last wrote, so the first pump pushes too (else JS keeps redjs's default viewport).
    let viewport_moved = prev.as_ref().is_none_or(|p| {
        p.inner_width != s.inner_width || p.inner_height != s.inner_height || p.scale != s.scale
    });
    if viewport_moved {
        unsafe {
            sk_runtime::redjs::ffi::redjs_dom_resize(
                ctx,
                s.inner_width as i32,
                s.inner_height as i32,
                s.scale as f64,
            )
        };
    }

    // Also seeds visibility if window init made no publication.
    publish_visibility(&s);

    let mut kinds: Vec<&str> = Vec::new();
    match &prev {
        None => kinds.push("boot"),
        Some(p) => {
            if viewport_moved {
                kinds.push("resize");
            }
            if p.maximized != s.maximized {
                kinds.push(if s.maximized {
                    "maximize"
                } else {
                    "unmaximize"
                });
            }
            if p.minimized != s.minimized {
                kinds.push(if s.minimized { "minimize" } else { "restore" });
            }
            if p.fullscreen != s.fullscreen {
                kinds.push("fullscreenchange");
            }
            if backdrop_moved(p, &s) {
                kinds.push("backdropchange");
            }
            if p.focused != s.focused {
                kinds.push(if s.focused { "focus" } else { "blur" });
            }
            if p.outer.0 != s.outer.0 || p.outer.1 != s.outer.1 {
                kinds.push("move");
            }
            if p.inner_width != s.inner_width
                || p.scale != s.scale
                || p.maximized != s.maximized
                || p.fullscreen != s.fullscreen
                || p.title_bar_height != s.title_bar_height
                || p.fallback_titlebar != s.fallback_titlebar
                || p.overlay.is_some() != s.overlay.is_some()
            {
                kinds.push("geometrychange");
            }
        }
    }
    PREV.with(|p| *p.borrow_mut() = Some(s.clone()));

    if !kinds.is_empty() {
        let names = kinds.join(",");
        IN_DISPATCH.with(|f| f.set(true));
        notify(&names, &json);
        IN_DISPATCH.with(|f| f.set(false));
    }

    // Before the pump, so `matchMedia` in a `resize` handler sees the new viewport.
    keep_theme(window);
    let dark = effective_dark(window);
    let env_changed = sk_tree::set_env(css_env(&s, dark, cx.reduce_motion()));
    if env_changed && BOOTED.with(|b| b.get()) {
        IN_DISPATCH.with(|f| f.set(true));
        notify("mediachange", &json);
        IN_DISPATCH.with(|f| f.set(false));
    }
}

fn backdrop_moved(p: &Snapshot, s: &Snapshot) -> bool {
    p.backdrop != s.backdrop || p.effective_backdrop != s.effective_backdrop
}

/// Hands gpui `appWindow.backdrop` if it differs from the last one given; `force` re-applies regardless.
fn sync_backdrop(window: &mut Window, force: bool) {
    let want = LIVE.with(|l| l.borrow().backdrop);
    if !force && APPLIED_BACKDROP.with(|a| a.get()) == Some(want) {
        return;
    }
    APPLIED_BACKDROP.with(|a| a.set(Some(want)));
    window.set_background_appearance(want.to_gpui());
    // The renderer reads the appearance when it draws; without a frame `material -> none` would stay see-through.
    window.refresh();
}

/// Retires the fallback titlebar the first frame the app paints its own drag region.
pub fn note_painted_frame() {
    if !sk_tree::painted_drag_region() {
        return;
    }
    let retired = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        if !l.fallback_titlebar {
            return false;
        }
        l.fallback_titlebar = false;
        true
    });
    if retired {
        let msg = "[window] the app painted a `window-drag: move` region — retiring the built-in \
                   fallback titlebar";
        eprintln!("{msg}");
        sk_tree::log::record("window", msg);
    }
}

/// Fires `DOMContentLoaded` + `load` once, after the app bundle has evaluated.
pub fn content_loaded() {
    if BOOTED.with(|b| b.replace(true)) {
        return;
    }
    JsRuntime::with(|rt| unsafe { sk_runtime::redjs::ffi::redjs_dom_content_loaded(rt.raw()) });
}

/// Calls the registered observer; its boolean is the close veto when `kind` is `close-requested`.
fn notify(kind: &str, state_json: &str) -> bool {
    let f = OBSERVER.with(|o| o.get());
    // No JS after a caught native panic: nothing can veto a close, so the window's own X still works.
    if f == 0 || sk_runtime::bridge::poisoned() {
        return true;
    }
    // `close-requested` comes from gpui's should-close callback, outside any publication: run an owed checkpoint first.
    sk_runtime::bridge::settle_owed_drain();
    JsRuntime::with(|rt| {
        let args = [rt.ctx().string(kind), rt.ctx().string(state_json)];
        let out = match rt.ctx().call(Value(f), Value::undefined(), &args) {
            Ok(v) => v.as_bool().unwrap_or(true),
            Err(sk_runtime::redjs::api::Error::Exception(e)) => {
                let err = rt.describe(e);
                eprintln!("[window observer] {}\n{}", err.header, err.stack);
                sk_tree::log::record("error", format!("[window observer] {}", err.header));
                true
            }
            Err(_) => true,
        };
        rt.ctx().drain_microtasks();
        out
    })
}

/// A user close request: `close-requested`, then `beforeunload`; either may keep the window.
pub fn should_close() -> bool {
    let json = SNAPSHOT.with(|s| snapshot_json(&s.borrow()));
    notify("close-requested", &json)
}

enum Op {
    Title(String),
    Minimize,
    Maximize,
    Unmaximize,
    ToggleMaximize,
    Close,
    Fullscreen(bool),
    Activate(bool),
    Bounds {
        x: Option<f32>,
        y: Option<f32>,
        w: Option<f32>,
        h: Option<f32>,
        content: bool,
    },
    Center,
    AlwaysOnTop(bool),
    SetBackdrop(Backdrop),
    SetTheme(Theme),
    StartDragging,
    Hide,
}

/// `SetWindowPos` sends `WM_SIZE`/`WM_MOVE` re-entrantly inside `Phase::Commit`, where `App` is
/// already borrowed: gpui's `on_resize` update fails (swallowed) and `Window::viewport_size` would
/// stay stale. This does what the swallowed callback would have done.
fn resync_bounds(window: &mut Window, cx: &mut gpui::App) {
    window.bounds_changed(cx);
}

/// Closes the app as `appWindow.close()` does (`why` names it in the `[shutdown]` line); pop-outs
/// go with the primary, which is removed after any file dialog it owns closes.
pub(crate) fn close_primary(why: &'static str, window: &mut Window, cx: &mut gpui::App) {
    crate::shutdown::begin(why);
    if crate::popout::count() > 0 {
        crate::popout::close_all(cx);
    }
    if !sk_desktop::dialog::owner_closing(sk_tree::windows::key(window)) {
        window.remove_window();
    } else {
        // gpui's own handle, not raw-window-handle's.
        let handle = gpui::Window::window_handle(window);
        cx.defer(move |cx| crate::dialogs::remove_window(handle, cx));
    }
}

fn apply(op: Op, window: &mut Window, cx: &mut gpui::App) {
    match op {
        Op::Title(t) => {
            window.set_window_title(&t);
            LIVE.with(|l| l.borrow_mut().title = t);
        }
        Op::Minimize => window.minimize_window(),
        // `zoom_window` does not toggle, so the verbs stay distinct.
        Op::Maximize => window.zoom_window(),
        Op::Unmaximize => win::restore(),
        Op::ToggleMaximize => {
            if win::is_zoomed() || window.is_maximized() {
                win::restore();
            } else {
                window.zoom_window();
            }
        }
        Op::Close => close_primary("close()", window, cx),
        // `toggle_fullscreen` toggles, so guard with the live state.
        Op::Fullscreen(on) => {
            if window.is_fullscreen() != on {
                window.toggle_fullscreen();
            }
        }
        Op::Activate(raise)
            if raise && APP_HIDDEN.swap(false, std::sync::atomic::Ordering::AcqRel) =>
        {
            let focus = !crate::quiet::never_focus();
            window.show_window(focus, || true, |_| {});
            if focus {
                REVEALED.store(true, std::sync::atomic::Ordering::Release);
                window.activate_window();
            }
        }
        // A test host neither raises nor flashes the taskbar.
        Op::Activate(_) if crate::quiet::never_focus() => {}
        Op::Activate(raise) => {
            if raise {
                REVEALED.store(true, std::sync::atomic::Ordering::Release);
                window.activate_window();
            } else {
                window.request_attention();
            }
        }
        Op::Bounds {
            x,
            y,
            w,
            h,
            content,
        } => apply_bounds(window, cx, x, y, w, h, content),
        Op::Center => {
            let s = SNAPSHOT.with(|s| s.borrow().clone());
            let (_, _, ow, oh) = s.outer;
            let (_, _, aw, ah, al, at) = s.screen;
            win::set_position(al + (aw - ow) / 2.0, at + (ah - oh) / 2.0);
            resync_bounds(window, cx);
        }
        Op::AlwaysOnTop(on) => {
            win::set_always_on_top(on);
            LIVE.with(|l| l.borrow_mut().always_on_top = on);
        }
        // Kept while fullscreen suspends it: leaving draws the last backdrop asked for.
        Op::SetBackdrop(b) => {
            LIVE.with(|l| l.borrow_mut().backdrop = b);
            sync_backdrop(window, true);
        }
        Op::SetTheme(t) => {
            apply_theme(t, window);
            // The env is re-read next frame; nothing else may be painting.
            window.refresh();
        }
        Op::StartDragging => win::start_dragging(),
        Op::Hide => {
            if !win::hide() {
                window.minimize_window();
            }
            APP_HIDDEN.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

/// Best-effort, clamped to the display's `visible_bounds()` so an app cannot park behind the taskbar.
fn apply_bounds(
    window: &mut Window,
    cx: &mut gpui::App,
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    h: Option<f32>,
    content: bool,
) {
    let mut moved_synchronously = false;
    let s = SNAPSHOT.with(|s| s.borrow().clone());
    let (_, _, aw, ah, al, at) = s.screen;
    let cur = if content { s.content } else { s.outer };

    if w.is_some() || h.is_some() {
        let mut nw = w.unwrap_or(cur.2).max(1.0);
        let mut nh = h.unwrap_or(cur.3).max(1.0);
        if aw > 0.0 {
            nw = nw.min(aw);
            nh = nh.min(ah);
        }
        if content {
            window.resize(gpui::size(gpui::px(nw), gpui::px(nh)));
        } else {
            win::set_outer_size(nw * s.scale, nh * s.scale);
            moved_synchronously = true;
        }
    }
    if x.is_some() || y.is_some() {
        let (ox, oy, ow, oh) = s.outer;
        let mut nx = x.unwrap_or(ox);
        let mut ny = y.unwrap_or(oy);
        if content {
            nx -= s.content.0 - ox;
            ny -= s.content.1 - oy;
        }
        if aw > 0.0 {
            nx = nx.clamp(al, (al + aw - ow).max(al));
            ny = ny.clamp(at, (at + ah - oh).max(at));
        }
        win::set_position(nx * s.scale, ny * s.scale);
        moved_synchronously = true;
    }
    // `Window::resize` spawns its `SetWindowPos`; the Win32 calls send `WM_SIZE` re-entrantly and need [`resync_bounds`].
    if moved_synchronously {
        resync_bounds(window, cx);
    }
}

/// Applies now if a `Phase::Commit` is live and not inside dispatch; otherwise queues.
fn submit(op: Op) {
    if IN_DISPATCH.with(|f| f.get()) {
        PENDING.with(|p| p.borrow_mut().push_back(op));
        return;
    }
    let applied = with_commit(|window, cx| apply(op_take(&op), window, cx)).is_some();
    if !applied {
        PENDING.with(|p| p.borrow_mut().push_back(op));
    }
}

/// Addresses the primary window even when JS was entered from a pop-out.
pub(crate) fn element_fullscreen(on: Option<bool>) -> bool {
    let current = SNAPSHOT.with(|snapshot| snapshot.borrow().fullscreen);
    if let Some(on) = on {
        PENDING.with(|pending| pending.borrow_mut().push_back(Op::Fullscreen(on)));
    }
    current
}

/// The embedder's `Handle::close`. Main thread.
pub(crate) fn request_close() {
    submit(Op::Close);
}

/// Main thread.
#[cfg(target_os = "linux")]
pub(crate) fn request_minimize() {
    submit(Op::Minimize);
}

#[cfg(target_os = "linux")]
pub(crate) fn request_toggle_maximize() {
    submit(Op::ToggleMaximize);
}

/// Goes through gpui's activation path, which applies a hidden window's stored placement before showing it.
pub(crate) fn request_activate() {
    submit(Op::Activate(true));
}

/// Set once the primary window is first raised; after that a hidden start is no longer unreachable.
static REVEALED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `appWindow.hide()` ran; the next raise shows the window again (gpui's activation only shows a hidden start).
static APP_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn revealed() -> bool {
    REVEALED.load(std::sync::atomic::Ordering::Acquire)
}

fn op_take(op: &Op) -> Op {
    match op {
        Op::Title(t) => Op::Title(t.clone()),
        Op::Minimize => Op::Minimize,
        Op::Maximize => Op::Maximize,
        Op::Unmaximize => Op::Unmaximize,
        Op::ToggleMaximize => Op::ToggleMaximize,
        Op::Close => Op::Close,
        Op::Fullscreen(b) => Op::Fullscreen(*b),
        Op::Activate(b) => Op::Activate(*b),
        Op::Bounds {
            x,
            y,
            w,
            h,
            content,
        } => Op::Bounds {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            content: *content,
        },
        Op::Center => Op::Center,
        Op::AlwaysOnTop(b) => Op::AlwaysOnTop(*b),
        Op::SetBackdrop(b) => Op::SetBackdrop(*b),
        Op::SetTheme(t) => Op::SetTheme(*t),
        Op::StartDragging => Op::StartDragging,
        Op::Hide => Op::Hide,
    }
}

fn drain_pending(window: &mut Window, cx: &mut gpui::App) {
    loop {
        let Some(op) = PENDING.with(|p| p.borrow_mut().pop_front()) else {
            return;
        };
        apply(op, window, cx);
    }
}

fn arg_f32(v: Value) -> Option<f32> {
    let n = v.as_number()?;
    n.is_finite().then_some(n as f32)
}

fn arg_bool(v: Value) -> bool {
    v.as_bool()
        .unwrap_or_else(|| v.as_number().is_some_and(|n| n != 0.0))
}

host_fn!(fn sk_window_state(_g, _args) {
    // Re-reads the window under a live Commit so state is exact right after `maximize()` (geometry still lags a frame); outside a Commit the published snapshot is returned.
    if let Some(s) = with_commit(|window, cx| snapshot(window, cx)) {
        SNAPSHOT.with(|c| *c.borrow_mut() = s);
    }
    let json = SNAPSHOT.with(|s| snapshot_json(&s.borrow()));
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

host_fn!(fn sk_window_observe(g, args) {
    let f = args.arg(0);
    unsafe { sk_runtime::redjs::ffi::redjs_protect(g, f.0) };
    OBSERVER.with(|o| o.set(f.0));
    UNDEF
});

host_fn!(fn sk_window_set_title(g, args) {
    let t = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    submit(Op::Title(t));
    UNDEF
});

host_fn!(fn sk_window_verb(g, args) {
    // One host fn for all arity-0 verbs: a global per verb costs a name in every app's scope.
    let verb = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    match verb.as_str() {
        "minimize" => submit(Op::Minimize),
        "maximize" => submit(Op::Maximize),
        "unmaximize" => submit(Op::Unmaximize),
        "toggleMaximize" => submit(Op::ToggleMaximize),
        "close" => submit(Op::Close),
        "center" => submit(Op::Center),
        "startDragging" => submit(Op::StartDragging),
        "hide" => submit(Op::Hide),
        "show" => request_activate(),
        other => advise(&format!("unknown appWindow verb {other:?} (host protocol mismatch)")),
    }
    UNDEF
});

host_fn!(fn sk_window_set_fullscreen(_g, args) {
    submit(Op::Fullscreen(arg_bool(args.arg(0))));
    UNDEF
});

host_fn!(fn sk_window_activate(_g, args) {
    // 0 = request attention (the default; never steals focus), 1 = raise (`window.focus: "raise"`).
    submit(Op::Activate(arg_bool(args.arg(0))));
    UNDEF
});

host_fn!(fn sk_window_set_bounds(_g, args) {
    let mask = args.arg(4).as_number().unwrap_or(0.0) as u32;
    let has = |bit: u32| mask & (1 << bit) != 0;
    submit(Op::Bounds {
        x: has(0).then(|| arg_f32(args.arg(0))).flatten(),
        y: has(1).then(|| arg_f32(args.arg(1))).flatten(),
        w: has(2).then(|| arg_f32(args.arg(2))).flatten(),
        h: has(3).then(|| arg_f32(args.arg(3))).flatten(),
        content: has(4),
    });
    UNDEF
});

host_fn!(fn sk_window_set_always_on_top(_g, args) {
    submit(Op::AlwaysOnTop(arg_bool(args.arg(0))));
    UNDEF
});

host_fn!(fn sk_window_set_backdrop(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    match Backdrop::from_str(&name) {
        Some((b, note)) => {
            if let Some(n) = note {
                advise(n);
            }
            let (b, os_note) = b.resolve_for_os();
            if let Some(n) = os_note {
                advise(n);
            }
            submit(Op::SetBackdrop(b));
        }
        None => advise(&format!(
            "unknown `backdrop`: {name:?} — expected mica | mica-alt | tabbed | acrylic | blur | \
             transparent | none."
        )),
    }
    UNDEF
});

host_fn!(fn sk_window_set_theme(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    match Theme::from_str(&name) {
        Some(t) => submit(Op::SetTheme(t)),
        None => advise(&format!("unknown `theme`: {name:?} — expected system | light | dark.")),
    }
    UNDEF
});

host_fn!(fn sk_window_set_overlay(g, args) {
    // `height` alone (arg 1 absent) keeps the live cluster, `false` retires it, an object installs or updates it.
    if let Some(h) = arg_f32(args.arg(0)) {
        LIVE.with(|l| l.borrow_mut().title_bar_height = h.max(0.0));
    }
    let mode = args.arg(1).as_number().unwrap_or(-1.0);
    let read = |i: u32| unsafe { read_string(g, args.arg(i).0) }.unwrap_or_default();
    match mode {
        m if m == 0.0 => LIVE.with(|l| l.borrow_mut().overlay = None),
        // An object; colours are empty strings when not given.
        m if m == 1.0 => {
            if config().title_bar_style == TitleBarStyle::Default {
                advise(A27);
                return UNDEF;
            }
            let mut adv = Vec::new();
            let mut parse = |s: String, key: &str| -> Option<u32> {
                if s.is_empty() {
                    return None;
                }
                let c = parse_color(&s);
                if c.is_none() {
                    adv.push(format!(
                        "`setTitleBarOverlay({{ {key}: {s:?} }})` is not a hex colour — expected \
                         `#rgb`, `#rrggbb` or `#rrggbbaa`. Ignored."
                    ));
                }
                c
            };
            let color = parse(read(2), "color");
            let symbol_color = parse(read(3), "symbolColor");
            for a in adv {
                advise(&a);
            }
            LIVE.with(|l| l.borrow_mut().overlay = Some(Overlay { color, symbol_color }));
        }
        _ => {}
    }
    UNDEF
});

const A27: &str =
    "`setTitleBarOverlay()` does nothing under `titleBarStyle: \"default\"` — the OS \
                   draws the caption buttons and owns their colours. The overlay is for \
                   custom-chrome windows.";

/// `None` in fullscreen or while minimised.
pub fn overlay() -> Option<(f32, Overlay)> {
    let s = SNAPSHOT.with(|s| s.borrow().clone());
    if s.fullscreen || s.minimized {
        return None;
    }
    s.overlay.map(|o| (s.title_bar_height, o))
}

host_fn!(fn sk_window_match_media(g, args) {
    let query = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let json = match sk_tree::match_media(&query) {
        Ok(m) => Json::obj(vec![("matches", Json::Bool(m))]),
        Err(why) => Json::obj(vec![
            ("matches", Json::Bool(false)),
            ("error", Json::s(format!("`{query}` {why}"))),
        ]),
    }
    .to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

// Debug hook: a synthesised click cannot exercise `WM_NCHITTEST`, so read back the hitboxes the last
// frame of one window handed gpui (absent id = the primary).
host_fn!(fn sk_window_hitboxes(_g, args) {
    let key = match args.arg(0).as_number() {
        Some(n) if n > 0.0 => n as u64,
        _ => sk_tree::windows::primary(),
    };
    let items: Vec<Json> = sk_tree::control_hitboxes_in(key)
        .into_iter()
        .map(|(id, area)| {
            Json::obj(vec![
                ("node", Json::n(id as f64)),
                (
                    "area",
                    Json::s(match area {
                        gpui::WindowControlArea::Drag => "drag",
                        gpui::WindowControlArea::NoDrag => "no-drag",
                        gpui::WindowControlArea::Min => "minimize",
                        gpui::WindowControlArea::Max => "maximize",
                        gpui::WindowControlArea::Close => "close",
                    }),
                ),
            ])
        })
        .collect();
    let json = Json::arr(items).to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

host_fn!(fn sk_window_cursor(_g, _args) {
    // Synthesised mouse events carry no `screenX`/`screenY`.
    let json = match win::cursor_pos() {
        Some((x, y)) => {
            let scale = SNAPSHOT.with(|s| s.borrow().scale).max(0.01);
            Json::obj(vec![("x", Json::n(x / scale)), ("y", Json::n(y / scale))])
        }
        None => Json::Null,
    }
    .to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const std::ffi::c_void)] = &[
        ("__sk_window_state", 0, sk_window_state as *const _),
        ("__sk_window_observe", 1, sk_window_observe as *const _),
        ("__sk_window_set_title", 1, sk_window_set_title as *const _),
        ("__sk_window_verb", 1, sk_window_verb as *const _),
        (
            "__sk_window_set_fullscreen",
            1,
            sk_window_set_fullscreen as *const _,
        ),
        ("__sk_window_activate", 1, sk_window_activate as *const _),
        (
            "__sk_window_set_bounds",
            5,
            sk_window_set_bounds as *const _,
        ),
        (
            "__sk_window_set_always_on_top",
            1,
            sk_window_set_always_on_top as *const _,
        ),
        (
            "__sk_window_set_backdrop",
            1,
            sk_window_set_backdrop as *const _,
        ),
        ("__sk_window_set_theme", 1, sk_window_set_theme as *const _),
        (
            "__sk_window_set_overlay",
            4,
            sk_window_set_overlay as *const _,
        ),
        (
            "__sk_window_match_media",
            1,
            sk_window_match_media as *const _,
        ),
        ("__sk_window_hitboxes", 1, sk_window_hitboxes as *const _),
        ("__sk_window_cursor", 0, sk_window_cursor as *const _),
        ("__sk_window_advise", 1, sk_window_advise as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_takes_electrons_spellings_and_forces_only_when_asked() {
        for t in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_str(t.as_str()), Some(t));
        }
        assert_eq!(
            Theme::from_str("Dark"),
            None,
            "the client's config spelling is mapped before this"
        );
        assert_eq!(Theme::from_str("auto"), None);
        assert!(Theme::System.is_dark(true) && !Theme::System.is_dark(false));
        assert!(!Theme::Light.is_dark(true) && Theme::Dark.is_dark(false));
    }

    #[test]
    fn acrylic_is_the_system_material_and_only_its_fallback_is_advised() {
        assert_eq!(
            Backdrop::from_str("acrylic"),
            Some((Backdrop::Acrylic, None))
        );
        assert_eq!(
            Backdrop::from_str("blur"),
            Some((Backdrop::Acrylic, Some(A20)))
        );
        let material = if cfg!(target_os = "macos") {
            WindowBackgroundAppearance::Blurred
        } else if cfg!(target_os = "linux") {
            WindowBackgroundAppearance::Opaque
        } else {
            WindowBackgroundAppearance::AcrylicBackdrop
        };
        assert_eq!(Backdrop::Acrylic.to_gpui(), material);

        for b in [
            Backdrop::None,
            Backdrop::Mica,
            Backdrop::MicaAlt,
            Backdrop::Acrylic,
            Backdrop::Transparent,
        ] {
            assert_eq!(resolve_backdrop(b, true), (b, None), "{b:?} on 22621+");
        }
        assert_eq!(
            resolve_backdrop(Backdrop::Acrylic, false),
            (Backdrop::Acrylic, Some(A20C))
        );
        assert_eq!(
            resolve_backdrop(Backdrop::Mica, false),
            (Backdrop::Acrylic, Some(A20B))
        );
        assert_eq!(
            resolve_backdrop(Backdrop::MicaAlt, false),
            (Backdrop::Acrylic, Some(A20B))
        );
        assert_eq!(
            resolve_backdrop(Backdrop::None, false),
            (Backdrop::None, None)
        );

        // Silent about `acrylic` where DWM draws it.
        let cfg = load_config(Some(r#"{"backdrop":"acrylic"}"#.to_string()));
        assert_eq!(cfg.backdrop, Backdrop::Acrylic);
        let advised = ADVISED.with(|s| s.borrow().contains(A20C));
        assert_eq!(
            advised,
            cfg!(target_os = "windows") && !has_system_backdrop()
        );
    }

    #[test]
    fn hex_colours_parse_in_all_three_lengths() {
        assert_eq!(parse_color("#fff"), Some(0xffff_ffff));
        assert_eq!(parse_color("#202020"), Some(0xff20_2020));
        assert_eq!(parse_color("#20202080"), Some(0x8020_2020));
        assert_eq!(parse_color("  #c42b1c  "), Some(0xffc4_2b1c));
        // Not hex: refused rather than painted black.
        assert_eq!(parse_color("rgb(1,2,3)"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#zzz"), None);
    }

    #[test]
    fn the_titlebar_rect_is_only_non_zero_when_the_strip_belongs_to_content() {
        let base = Snapshot {
            inner_width: 1000.0,
            title_bar_height: 32.0,
            fallback_titlebar: false,
            ..Snapshot::default()
        };
        assert_eq!(titlebar_area(&base), (true, (0.0, 0.0, 1000.0, 32.0)));

        let hosted = Snapshot {
            fallback_titlebar: true,
            ..base.clone()
        };
        assert_eq!(titlebar_area(&hosted), (false, (0.0, 0.0, 0.0, 0.0)));

        let full = Snapshot {
            fullscreen: true,
            ..base.clone()
        };
        assert_eq!(titlebar_area(&full), (false, (0.0, 0.0, 0.0, 0.0)));
        assert_eq!(
            css_env(&full, false, false).titlebar_area,
            None,
            "`env(titlebar-area-height, 40px)` must not collapse a titlebar to zero in fullscreen"
        );

        let min = Snapshot {
            minimized: true,
            ..base.clone()
        };
        assert_eq!(titlebar_area(&min), (false, (0.0, 0.0, 0.0, 0.0)));
        let cloaked = Snapshot {
            cloaked: true,
            ..base.clone()
        };
        let unfocused = Snapshot {
            focused: false,
            ..base.clone()
        };
        let not_shown = Snapshot {
            visible: false,
            ..base.clone()
        };
        assert!(
            !not_shown.minimized,
            "a hidden window need not be minimized"
        );
        assert_eq!(
            (
                hidden(&base),
                hidden(&min),
                hidden(&cloaked),
                hidden(&unfocused),
                hidden(&not_shown)
            ),
            (false, true, true, false, true)
        );
        let covered = Snapshot {
            occluded: true,
            ..base.clone()
        };
        assert!(
            hidden(&covered),
            "a window other windows cover is a hidden page"
        );

        // A host cluster takes its width off the right (3 x 46 px).
        let overlaid = Snapshot {
            overlay: Some(Overlay::default()),
            ..base.clone()
        };
        assert_eq!(titlebar_area(&overlaid), (true, (0.0, 0.0, 862.0, 32.0)));
    }

    #[test]
    fn a_system_backdrop_is_suspended_only_while_gpui_stands_in_for_it() {
        for b in [Backdrop::Mica, Backdrop::MicaAlt, Backdrop::Acrylic] {
            assert_eq!(
                effective_backdrop(b, true),
                b,
                "{b:?} drawn, windowed or fullscreen"
            );
            assert_eq!(
                effective_backdrop(b, false),
                Backdrop::None,
                "{b:?} with no desktop picture"
            );
        }
        // Not a system material: what the app asked for is what it gets.
        for b in [Backdrop::None, Backdrop::Transparent] {
            assert_eq!(effective_backdrop(b, true), b);
            assert_eq!(effective_backdrop(b, false), b);
        }
    }

    #[test]
    fn backdropchange_follows_the_backdrop_and_what_the_window_draws() {
        let windowed = Snapshot {
            backdrop: Backdrop::Acrylic,
            effective_backdrop: Backdrop::Acrylic,
            ..Snapshot::default()
        };
        let full_glass = Snapshot {
            fullscreen: true,
            ..windowed.clone()
        };
        assert!(
            !backdrop_moved(&windowed, &full_glass),
            "the glass stays in fullscreen"
        );
        let full = Snapshot {
            fullscreen: true,
            effective_backdrop: effective_backdrop(Backdrop::Acrylic, false),
            ..windowed.clone()
        };
        let solid = Snapshot {
            backdrop: Backdrop::None,
            effective_backdrop: Backdrop::None,
            ..windowed.clone()
        };
        assert!(backdrop_moved(&windowed, &full));
        assert!(backdrop_moved(&full, &windowed));
        assert!(backdrop_moved(&windowed, &solid));
        assert!(!backdrop_moved(&full, &full.clone()));
        // A backdrop asked for while fullscreen still fires: the app's choice moved.
        let full_solid = Snapshot {
            backdrop: Backdrop::None,
            ..full.clone()
        };
        assert!(backdrop_moved(&full, &full_solid));
    }

    #[test]
    fn startup_prewarm_does_not_publish_a_hidden_page_or_mask_later_hides() {
        cancel_startup_reveal();
        assert!(
            !startup_visibility(false),
            "visible(false) embedding remains hidden"
        );
        begin_startup_reveal();
        assert!(
            startup_visibility(false),
            "the hidden HWND is only an internal first-frame warm-up"
        );
        assert!(
            startup_visibility(true),
            "the native show completes the warm-up"
        );
        assert!(
            !startup_visibility(false),
            "later explicit hides must be observable"
        );
        begin_startup_reveal();
        cancel_startup_reveal();
        assert!(
            !startup_visibility(false),
            "cancelled startup must not retain the override"
        );
    }

    #[test]
    fn visibility_is_seeded_at_boot_and_only_actual_hidden_edges_publish() {
        let shown = Snapshot::default();
        let hidden = Snapshot {
            visible: false,
            ..shown.clone()
        };
        assert_eq!(visibility_delta(None, &shown), Some(false));
        assert_eq!(visibility_delta(None, &hidden), Some(true));
        assert_eq!(
            visibility_delta(Some(true), &hidden),
            None,
            "the first pump must not repeat the boot publication"
        );
        assert_eq!(
            visibility_delta(Some(true), &shown),
            Some(false),
            "showing restores DOM visibility"
        );
        assert_eq!(
            visibility_delta(Some(false), &hidden),
            Some(true),
            "hiding does not require minimization"
        );
        let minimized = Snapshot {
            minimized: true,
            ..shown.clone()
        };
        let cloaked = Snapshot {
            cloaked: true,
            ..shown.clone()
        };
        assert_eq!(
            visibility_delta(Some(true), &minimized),
            None,
            "hidden-to-minimized is still a hidden page"
        );
        assert_eq!(visibility_delta(Some(true), &cloaked), None);
        let unfocused = Snapshot {
            focused: false,
            ..shown
        };
        assert_eq!(
            visibility_delta(Some(false), &unfocused),
            None,
            "focus is independent of visibility"
        );
    }

    #[test]
    fn the_css_env_carries_the_same_titlebar_rect() {
        let s = Snapshot {
            inner_width: 800.0,
            inner_height: 600.0,
            title_bar_height: 36.0,
            fallback_titlebar: false,
            ..Snapshot::default()
        };
        let env = css_env(&s, false, false);
        assert_eq!(env.titlebar_area, Some([0.0, 0.0, 800.0, 36.0]));
        assert_eq!(env.viewport_width, 800.0);
        assert_eq!(env.viewport_height, 600.0);
        assert_eq!(env.safe_area_insets, [0.0; 4]);
    }

    #[test]
    fn os_chrome_is_a_real_mode_and_refuses_a_second_titlebar() {
        let cfg = load_config(Some(
            r#"{"titleBarStyle":"default","titleBarFallback":true}"#.to_string(),
        ));
        assert_eq!(cfg.title_bar_style, TitleBarStyle::Default);
        assert!(!cfg.title_bar_fallback);
    }

    #[test]
    fn title_bar_fallback_false_gives_a_real_env_rect_on_the_first_frame() {
        let cfg = load_config(Some(
            r#"{"titleBarStyle":"hidden","titleBarHeight":36,"titleBarFallback":false}"#
                .to_string(),
        ));
        assert!(!cfg.title_bar_fallback);

        let boot = Snapshot {
            inner_width: 980.0,
            inner_height: 720.0,
            title_bar_height: cfg.title_bar_height,
            fallback_titlebar: cfg.title_bar_fallback,
            ..Snapshot::default()
        };
        assert_eq!(titlebar_area(&boot), (true, (0.0, 0.0, 980.0, 36.0)));
        assert_eq!(
            css_env(&boot, false, false).titlebar_area,
            Some([0.0, 0.0, 980.0, 36.0])
        );

        // The default (fallback ON) has no area until it retires.
        let default_cfg = load_config(Some(
            r#"{"titleBarStyle":"hidden","titleBarHeight":36}"#.to_string(),
        ));
        assert!(default_cfg.title_bar_fallback);
        let first_frame = Snapshot {
            fallback_titlebar: true,
            ..boot.clone()
        };
        assert_eq!(titlebar_area(&first_frame), (false, (0.0, 0.0, 0.0, 0.0)));
        assert_eq!(css_env(&first_frame, false, false).titlebar_area, None);
    }

    #[test]
    fn title_bar_overlay_parses_electrons_shape() {
        let cfg = load_config(Some(
            r##"{"titleBarOverlay":{"height":40,"color":"#202020","symbolColor":"#fff"}}"##
                .to_string(),
        ));
        assert_eq!(cfg.title_bar_height, 40.0);
        assert_eq!(
            cfg.title_bar_overlay,
            Some(Overlay {
                color: Some(0xff20_2020),
                symbol_color: Some(0xffff_ffff)
            })
        );

        let cfg = load_config(Some(r#"{"titleBarOverlay":false}"#.to_string()));
        assert_eq!(cfg.title_bar_overlay, None);
        let cfg = load_config(Some(r#"{}"#.to_string()));
        assert_eq!(cfg.title_bar_overlay, None);
    }
}
