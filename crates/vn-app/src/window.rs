//! Windowing & chrome — the host half of `docs/WINDOWING.md`.
//!
//! Three tiers of surface land here, and which tier a capability belongs to is
//! not a matter of taste (the compat-first law, §0 of that document):
//!
//! 1. **browser-standard** — `document.title`, `innerWidth`/`outerWidth`,
//!    `screen`, `resizeTo`/`moveTo`, `matchMedia`, the Fullscreen API,
//!    `visibilitychange`. These are wired to real window state so JS can never
//!    disagree with what `@media` already sees;
//! 2. **standards-track custom chrome** — CSS `window-drag` (handled in
//!    `vn-css`/`vn-tree`) and `navigator.windowControlsOverlay`;
//! 3. **namespaced extension** — `appWindow`, for the things the web has no
//!    words for (minimize, maximize, backdrop material).
//!
//! ## Everything funnels through one per-frame snapshot
//!
//! [`snapshot`] reads the window ONCE per frame, [`pump`] diffs it against the
//! previous frame and produces every derived event in a fixed order — resize
//! first, then visibility, then the JS observer. That ordering is not
//! cosmetic: the Fullscreen spec requires `fullscreenchange` to fire *after*
//! the viewport has finished resizing, and doing it in one place is what makes
//! that true by construction rather than by luck.
//!
//! Getters never touch a phase at all — they read the published snapshot — and
//! mutators QUEUE instead of throwing when no `Phase::Commit` is live, which is
//! what makes module-scope `document.title = "…"` and `defineWindow({…})` work
//! (D-29). Nothing in this surface can throw "called outside a published
//! phase".

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use gpui::{Window, WindowBackgroundAppearance};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use vn_runtime::{host_fn, with_commit, JsRuntime};
use vn_runtime::redjs::api::Value;
use vn_runtime::redjs::webapi::util::read_string;

use crate::json::{self, Json};

const UNDEF: u64 = 0xa;

// ---------------------------------------------------------------------------
// configuration (docs/WINDOWING.md §4.1)
// ---------------------------------------------------------------------------

/// `titleBarStyle`. An ENUM, never a boolean `frameless`: macOS needs `hidden`
/// vs `hiddenInset`, so the axis has to exist from day one (D-22).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBarStyle {
    /// OS-drawn chrome. Needs fork item F-03 (`WS_CAPTION` for
    /// `!hide_title_bar`); until that lands, asking for it advises (A-17) and
    /// falls back to `Hidden` — never silently, which is why nothing
    /// constructs this variant yet.
    #[allow(dead_code)]
    Default,
    /// Custom chrome: the app draws the titlebar, marks it `window-drag: move`
    /// and marks its buttons `--window-control: …`. The v1 default.
    Hidden,
}

/// The unified backdrop value space: Tauri's single list with Electron's
/// lowercase spellings (D-26).
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
            "acrylic" => (Backdrop::Acrylic, Some(A20C)),
            // gpui exposes ONE blurred material on Windows, so `blur` and
            // `acrylic` are the same pixels. Say so rather than let an author
            // believe they picked something different (A-20).
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
            Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
            Backdrop::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
            Backdrop::Acrylic => WindowBackgroundAppearance::Blurred,
            Backdrop::Transparent => WindowBackgroundAppearance::Transparent,
        }
    }

    /// Mica is a Windows 11 material. Asking for it on Windows 10 must advise
    /// and fall back to acrylic, never silently no-op (Tauri's precedent).
    fn resolve_for_os(self) -> (Backdrop, Option<&'static str>) {
        match self {
            Backdrop::Mica | Backdrop::MicaAlt if !is_windows_11() => (Backdrop::Acrylic, Some(A20B)),
            other => (other, None),
        }
    }
}

const A20: &str = "`backdrop: \"blur\"` resolves to `acrylic` here — gpui exposes one blurred \
                   material on Windows. Use `acrylic` to say so explicitly.";
const A20B: &str = "Mica is a Windows 11 material; falling back to `acrylic`.";
const A20C: &str = "`acrylic` has known compositor cost while resizing and dragging on Windows 10 \
                    1903+ and Windows 11 22000. `mica` is cheaper if you do not need the \
                    per-window tint.";

/// Everything `apps/<app>/vn.config.json`'s `window` block can say.
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
    /// The built-in Shell titlebar (§4.2 mode 2). `true` = keep it until the
    /// app paints a drag region of its own.
    pub title_bar_fallback: bool,
    pub backdrop: Backdrop,
    pub resizable: bool,
    pub minimizable: bool,
    pub maximizable: bool,
    pub always_on_top: bool,
    /// `window.focus()` policy: `false` = `request_attention` (the default —
    /// this host does not steal focus), `true` = `activate_window` (D-07).
    pub focus_raise: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            title: "vue-native".into(),
            width: 640.0,
            height: 420.0,
            min_width: None,
            min_height: None,
            frame: true,
            title_bar_style: TitleBarStyle::Hidden,
            // The Win11 standard caption height. macOS would default to 28pt,
            // which is why this is one configurable number and not a constant
            // sprinkled through the layout (§7.2).
            title_bar_height: 32.0,
            title_bar_fallback: true,
            backdrop: Backdrop::Mica,
            resizable: true,
            minimizable: true,
            maximizable: true,
            always_on_top: false,
            focus_raise: false,
        }
    }
}

thread_local! {
    static CONFIG: RefCell<Config> = RefCell::new(Config::default());
}

pub fn config() -> Config {
    CONFIG.with(|c| c.borrow().clone())
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

/// Reads the `window` block, applies the environment overrides on top
/// (`VN_WINDOW`, `VN_WINDOW_FOCUS` — the perf and screenshot harnesses depend
/// on winning, D-21), and stashes the result for the rest of the process.
///
/// Called BEFORE `cx.open_window`: this is the only tier that can set
/// creation-time-only gpui options without a visible flash (D-20).
pub fn load_config(raw: Option<String>) -> Config {
    let mut cfg = Config::default();
    let mut advisories: Vec<String> = Vec::new();

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
            Some("default") => {
                // Fork item F-03 is not in this build: gpui never sets
                // `WS_CAPTION` for `WindowKind::Normal`, and
                // `appears_transparent: false` gives a thick resize frame with
                // NO titlebar at all — which is worse than either mode.
                advisories.push(A17.to_string());
                cfg.title_bar_style = TitleBarStyle::Hidden;
            }
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
    }

    // Environment wins: a perf or screenshot harness has to be able to choose
    // a viewport without editing the app (D-21).
    if let Some((w, h)) = std::env::var("VN_WINDOW").ok().and_then(|v| {
        let (w, h) = v.split_once(['x', 'X'])?;
        Some((w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?))
    }) {
        cfg.width = w;
        cfg.height = h;
    }
    if std::env::var("VN_WINDOW_FOCUS").is_ok_and(|v| v == "raise") {
        cfg.focus_raise = true;
    }

    let (backdrop, note) = cfg.backdrop.resolve_for_os();
    cfg.backdrop = backdrop;
    advisories.extend(note.map(str::to_string));

    // Windows 11 snap layouts reject zones above 500 epx even though the
    // flyout still appears — an invisible failure, so it is worth a line
    // (A-21). Our default is no minimum at all, so this only bites opt-ins.
    if cfg.min_width.is_some_and(|w| w > 500.0) {
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

const A17: &str = "`titleBarStyle: \"default\"` (OS-drawn chrome) needs the `WS_CAPTION` patch in \
                   vendor/zed (fork item F-03, docs/WINDOWING.md §9); falling back to \
                   `titleBarStyle: \"hidden\"` (custom chrome).";
const A18: &str = "`frame: false` is treated as `titleBarStyle: \"hidden\"` in v1. A true frameless \
                   window would drop WS_THICKFRAME, which also removes the drop shadow, window \
                   animations and edge resizing.";

// ---------------------------------------------------------------------------
// advisories
// ---------------------------------------------------------------------------

thread_local! {
    static ADVISED: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
}

/// One per message, ever — the house style shared with `vn-css`'s `once` and
/// `vn-tree`'s `warn_compat`.
pub fn advise(msg: &str) {
    ADVISED.with(|s| {
        if s.borrow_mut().insert(msg.to_string()) {
            eprintln!("[vue-native compat] {msg}");
            vn_tree::log::record("advisory", msg);
        }
    });
}

host_fn!(fn vn_window_advise(g, args) {
    let msg = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    advise(&msg);
    UNDEF
});

// ---------------------------------------------------------------------------
// Win32 escape hatches
// ---------------------------------------------------------------------------
//
// gpui has no `PlatformWindow::set_position`, no runtime min/max size and no
// non-toggling restore, so four of the verbs below reach the HWND directly.
// That is safe here precisely because gpui's bounds cache is refreshed from the
// `WM_MOVE`/`WM_SIZE` those calls generate — nothing desyncs, reads simply lag
// by one frame (which is also what the CSSOM View spec tells authors to expect
// from `moveTo`). Fork items F-04..F-06 replace them upstream later.

#[cfg(target_os = "windows")]
mod win {
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn hwnd() -> Option<HWND> {
        super::hwnd_raw().map(|h| HWND(h as *mut _))
    }

    /// `GetWindowRect` — the WINDOW rect, frame included.
    ///
    /// NOT `Window::bounds()`, which is produced by `calculate_client_rect` and
    /// is therefore the CLIENT rect: it is `getContentBounds()`, not
    /// `getBounds()` (D-06, correcting the research report).
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

    pub fn restore() {
        if let Some(h) = hwnd() {
            let _ = unsafe { ShowWindowAsync(h, SW_RESTORE) };
        }
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
            let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = unsafe {
                SetWindowPos(h, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
            };
        }
    }

    /// `appWindow.startDragging()` — the manual equivalent of a
    /// `window-drag: move` region, for a titlebar that wants to start the drag
    /// from its own `mousedown` handler.
    pub fn start_dragging() {
        if let Some(h) = hwnd() {
            unsafe {
                let _ = ReleaseCapture();
                let _ = SendMessageW(h, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
            }
        }
    }

    /// Whether the window is CLOAKED — hidden because its virtual desktop is
    /// not the current one. Deliberately NOT folded into `document.hidden` in
    /// v1 (see the note on the visibility funnel); reported so the omission is
    /// documented rather than silent.
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

    pub fn cursor_pos() -> Option<(f32, f32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok()?;
        Some((p.x as f32, p.y as f32))
    }
}

#[cfg(not(target_os = "windows"))]
mod win {
    pub fn window_rect() -> Option<(f32, f32, f32, f32)> { None }
    pub fn is_zoomed() -> bool { false }
    pub fn is_iconic() -> bool { false }
    pub fn restore() {}
    pub fn set_position(_x: f32, _y: f32) {}
    pub fn set_outer_size(_w: f32, _h: f32) {}
    pub fn set_always_on_top(_on: bool) {}
    pub fn start_dragging() {}
    pub fn is_cloaked() -> bool { false }
    pub fn cursor_pos() -> Option<(f32, f32)> { None }
}

thread_local! {
    /// The window's HWND, captured once. It never changes for the lifetime of
    /// the window, and caching it is what lets the Win32 helpers work from
    /// `open_window`'s builder closure (where nothing is published yet) as well
    /// as from a host fn.
    static HWND_CACHE: Cell<isize> = const { Cell::new(0) };
}

fn hwnd_raw() -> Option<isize> {
    let cached = HWND_CACHE.with(|c| c.get());
    if cached != 0 {
        return Some(cached);
    }
    let found = with_commit(|window, _| capture_hwnd(window)).flatten();
    found
}

fn capture_hwnd(window: &Window) -> Option<isize> {
    // gpui implements `raw_window_handle::HasWindowHandle` on `Window`, but the
    // inherent `Window::window_handle()` shadows the trait method — hence UFCS.
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    let raw = match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }?;
    HWND_CACHE.with(|c| c.set(raw));
    Some(raw)
}

/// Windows 11 = build 22000 or later. Mica exists only there.
#[cfg(target_os = "windows")]
fn is_windows_11() -> bool {
    use std::sync::OnceLock;
    static IS_11: OnceLock<bool> = OnceLock::new();
    *IS_11.get_or_init(|| {
        use windows::Wdk::System::SystemServices::RtlGetVersion;
        use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
        let mut vi = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        // `RtlGetVersion` is the only version query Windows does not lie about
        // for an unmanifested process (`GetVersionEx` reports 6.2 forever).
        unsafe { RtlGetVersion(&mut vi) }.is_ok() && vi.dwBuildNumber >= 22000
    })
}

#[cfg(not(target_os = "windows"))]
fn is_windows_11() -> bool {
    false
}

// ---------------------------------------------------------------------------
// the per-frame state snapshot (docs/WINDOWING.md §5.3)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub inner_width: f32,
    pub inner_height: f32,
    pub outer: (f32, f32, f32, f32),
    pub content: (f32, f32, f32, f32),
    pub scale: f32,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub focused: bool,
    pub cloaked: bool,
    pub screen: (f32, f32, f32, f32, f32, f32),
    pub displays: usize,
    pub title: String,
    pub backdrop: Backdrop,
    pub always_on_top: bool,
    pub title_bar_height: f32,
    pub fallback_titlebar: bool,
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
            fullscreen: false,
            focused: false,
            cloaked: false,
            screen: (0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            displays: 1,
            title: String::new(),
            backdrop: Backdrop::Mica,
            always_on_top: false,
            title_bar_height: 32.0,
            fallback_titlebar: true,
        }
    }
}

thread_local! {
    /// The snapshot every getter reads. Published once per frame, so a getter
    /// never needs a phase and never lies about which frame it is describing.
    static SNAPSHOT: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    /// The registered JS observer (`__vn_window_observe`), protected against GC.
    static OBSERVER: Cell<u64> = const { Cell::new(0) };
    /// Mutators that arrived outside `Phase::Commit` (module scope, or from
    /// inside the dispatch below). Drained at the top of the next frame.
    static PENDING: RefCell<VecDeque<Op>> = const { RefCell::new(VecDeque::new()) };
    /// Set across the whole dispatch step. `resizeTo` from a `resize` handler
    /// is a classic infinite loop; browsers do not guard it and neither do we,
    /// but our frame-driven funnel could RE-ENTER within one frame, which is a
    /// different bug. While this is set, mutators queue instead of applying.
    static IN_DISPATCH: Cell<bool> = const { Cell::new(false) };
    /// The mutable half of the chrome state: `backdrop`, `alwaysOnTop`,
    /// `titleBarHeight` and the fallback titlebar all change at runtime.
    static LIVE: RefCell<Live> = RefCell::new(Live::default());
}

#[derive(Clone, Debug)]
struct Live {
    title: String,
    backdrop: Backdrop,
    always_on_top: bool,
    title_bar_height: f32,
    /// `false` once the app has painted a `window-drag: move` region of its own
    /// (§4.2 mode 2) — or from the start if the config said so.
    fallback_titlebar: bool,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            title: "vue-native".into(),
            backdrop: Backdrop::Mica,
            always_on_top: false,
            title_bar_height: 32.0,
            fallback_titlebar: true,
        }
    }
}

/// Seeds the mutable half from the config and publishes the first snapshot,
/// from inside `open_window`'s builder closure.
///
/// The first snapshot has to exist BEFORE the JS prelude evaluates: the prelude
/// seeds `screen`, `outerWidth`, the WCO rect and `appWindow`'s getters from
/// `__vn_window_state()`, and a window full of zeros at boot would be a lie
/// that survives until the first frame.
pub fn init(cfg: &Config, window: &mut Window, cx: &mut gpui::App) {
    LIVE.with(|l| {
        *l.borrow_mut() = Live {
            title: cfg.title.clone(),
            backdrop: cfg.backdrop,
            always_on_top: cfg.always_on_top,
            title_bar_height: cfg.title_bar_height,
            fallback_titlebar: cfg.title_bar_fallback,
        }
    });
    capture_hwnd(window);
    if cfg.always_on_top {
        win::set_always_on_top(true);
    }
    let s = snapshot(window, cx);
    SNAPSHOT.with(|c| *c.borrow_mut() = s);
    // Seed the cascade env too, so the very first restyle already knows the
    // real viewport, DPI and chrome mode.
    let dark = matches!(
        window.appearance(),
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    );
    let s = SNAPSHOT.with(|c| c.borrow().clone());
    vn_tree::set_env(vn_css::Env {
        viewport_width: s.inner_width,
        viewport_height: s.inner_height,
        dark,
        reduce_motion: cx.reduce_motion(),
        scale_factor: s.scale,
        display_mode: display_mode(s.fullscreen),
    });
}

/// Whether the host still draws its own titlebar strip.
///
/// The hardcoded strip in `Shell::render` must not simply be deleted: an app
/// that declares no drag region would become unmovable. So it retires on the
/// first painted frame in which any node's effective `window-drag` is `move`,
/// with one `[window]` log line — never silently removing a behaviour an app
/// was relying on (D-23).
pub fn fallback_titlebar() -> bool {
    LIVE.with(|l| l.borrow().fallback_titlebar)
}

fn display_mode(fullscreen: bool) -> vn_css::DisplayMode {
    if fullscreen {
        return vn_css::DisplayMode::Fullscreen;
    }
    match config().title_bar_style {
        TitleBarStyle::Default => vn_css::DisplayMode::Standalone,
        // WCO means "the titlebar strip belongs to content", which is precisely
        // true in custom-chrome mode whether or not a host cluster is drawn. It
        // is the branch a reused PWA component must take, so reporting anything
        // else would make correct components lay out wrongly (D-11).
        TitleBarStyle::Hidden => vn_css::DisplayMode::WindowControlsOverlay,
    }
}

/// Reads the window once. Everything derived — every event, every getter —
/// comes from here, so nothing can observe two different frames at once.
pub fn snapshot(window: &Window, cx: &gpui::App) -> Snapshot {
    let viewport = window.viewport_size();
    let bounds = window.bounds();
    let live = LIVE.with(|l| l.borrow().clone());
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
    let scale = window.scale_factor();
    // `GetWindowRect` is in PHYSICAL pixels; everything else on this surface is
    // CSS px, which is what gpui's `Pixels` already are.
    let outer = win::window_rect()
        .map(|(x, y, w, h)| (x / scale, y / scale, w / scale, h / scale))
        .unwrap_or((
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ));
    Snapshot {
        inner_width: f32::from(viewport.width),
        inner_height: f32::from(viewport.height),
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
        fullscreen: window.is_fullscreen(),
        focused: window.is_window_active(),
        cloaked: win::is_cloaked(),
        screen,
        displays,
        title: live.title,
        backdrop: live.backdrop,
        always_on_top: live.always_on_top,
        title_bar_height: live.title_bar_height,
        fallback_titlebar: live.fallback_titlebar,
    }
}

fn rect(t: (f32, f32, f32, f32)) -> Json {
    Json::obj(vec![
        ("x", Json::n(t.0)),
        ("y", Json::n(t.1)),
        ("width", Json::n(t.2)),
        ("height", Json::n(t.3)),
    ])
}

/// The snapshot as the JSON the JS side mirrors into `screen`, `outerWidth`,
/// `navigator.windowControlsOverlay`, `appWindow.*` and friends.
fn snapshot_json(s: &Snapshot) -> String {
    let mode = display_mode(s.fullscreen);
    // The WCO rect describes the area available to APP CONTENT. In default
    // chrome (the OS owns the caption) and in fullscreen (where drag regions
    // are inert anyway) it is the zero rect and `visible` is false — exactly
    // what a PWA in a tab reports, so a component written for a real PWA takes
    // the right branch unmodified.
    let wco_visible =
        mode == vn_css::DisplayMode::WindowControlsOverlay && !s.fullscreen && !s.minimized;
    let titlebar = if wco_visible {
        (0.0, 0.0, s.inner_width, s.title_bar_height)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let cfg = config();
    Json::obj(vec![
        ("innerWidth", Json::n(s.inner_width)),
        ("innerHeight", Json::n(s.inner_height)),
        ("outer", rect(s.outer)),
        ("content", rect(s.content)),
        ("devicePixelRatio", Json::n(s.scale)),
        ("maximized", Json::Bool(s.maximized)),
        ("minimized", Json::Bool(s.minimized)),
        ("fullscreen", Json::Bool(s.fullscreen)),
        ("focused", Json::Bool(s.focused)),
        ("cloaked", Json::Bool(s.cloaked)),
        ("title", Json::s(s.title.clone())),
        ("backdrop", Json::s(s.backdrop.as_str())),
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
        ("focusPolicy", Json::s(if cfg.focus_raise { "raise" } else { "attention" })),
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

// ---------------------------------------------------------------------------
// the frame funnel
// ---------------------------------------------------------------------------

thread_local! {
    static PREV: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    static BOOTED: Cell<bool> = const { Cell::new(false) };
}

/// Runs inside `pump_frame`'s `Phase::Commit`, BEFORE `rt.pump`.
///
/// Order is the contract:
/// 1. drain anything a module-scope mutator queued;
/// 2. snapshot;
/// 3. `redjs_dom_resize` when the viewport or the DPI moved — this is what
///    updates `innerWidth`/`innerHeight`/`devicePixelRatio` and fires the
///    trusted `resize`;
/// 4. `redjs_dom_set_visibility` when minimised-ness moved;
/// 5. the JS observer, with the state delta.
///
/// Step 3 before step 5 is what makes `fullscreenchange` fire AFTER the
/// viewport has finished resizing, which the Fullscreen spec requires.
pub fn pump(window: &mut Window, cx: &mut gpui::App) {
    drain_pending(window, cx);

    let s = snapshot(window, cx);
    SNAPSHOT.with(|c| *c.borrow_mut() = s.clone());
    let prev = PREV.with(|p| p.borrow().clone());

    let json = snapshot_json(&s);
    let ctx = JsRuntime::with(|rt| rt.raw());

    let mut kinds: Vec<&str> = Vec::new();
    match &prev {
        None => kinds.push("boot"),
        Some(p) => {
            if p.inner_width != s.inner_width
                || p.inner_height != s.inner_height
                || p.scale != s.scale
            {
                unsafe {
                    vn_runtime::redjs::ffi::redjs_dom_resize(
                        ctx,
                        s.inner_width as i32,
                        s.inner_height as i32,
                        s.scale as f64,
                    )
                };
                kinds.push("resize");
            }
            // "Hidden" means MINIMISED, not unfocused: a browser does not
            // report a background-but-visible window as hidden, and ours keeps
            // rendering when inactive by design. Virtual-desktop cloaking is
            // read (`snapshot.cloaked`) but deliberately NOT folded in here —
            // documented rather than silently reported as `visible`.
            if p.minimized != s.minimized {
                unsafe {
                    vn_runtime::redjs::ffi::redjs_dom_set_visibility(ctx, s.minimized as i32)
                };
            }
            if p.maximized != s.maximized {
                kinds.push(if s.maximized { "maximize" } else { "unmaximize" });
            }
            if p.minimized != s.minimized {
                kinds.push(if s.minimized { "minimize" } else { "restore" });
            }
            if p.fullscreen != s.fullscreen {
                kinds.push("fullscreenchange");
            }
            if p.focused != s.focused {
                kinds.push(if s.focused { "focus" } else { "blur" });
            }
            if p.outer.0 != s.outer.0 || p.outer.1 != s.outer.1 {
                kinds.push("move");
            }
            // WICG's own note is that `geometrychange` fires "as frequently as
            // a scroll listener"; the official advice is to debounce, and our
            // docs repeat it.
            if p.inner_width != s.inner_width
                || p.scale != s.scale
                || p.maximized != s.maximized
                || p.fullscreen != s.fullscreen
                || p.title_bar_height != s.title_bar_height
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

    // The env every `@media` condition and every live `MediaQueryList` is
    // evaluated against. Published here rather than after the pump so a
    // `matchMedia(...).matches` read from a `resize` handler already sees the
    // new viewport.
    let dark = matches!(
        window.appearance(),
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    );
    let env_changed = vn_tree::set_env(vn_css::Env {
        viewport_width: s.inner_width,
        viewport_height: s.inner_height,
        dark,
        reduce_motion: cx.reduce_motion(),
        scale_factor: s.scale,
        display_mode: display_mode(s.fullscreen),
    });
    if env_changed && BOOTED.with(|b| b.get()) {
        IN_DISPATCH.with(|f| f.set(true));
        notify("mediachange", &json);
        IN_DISPATCH.with(|f| f.set(false));
    }
}

/// Retires the fallback titlebar the first frame the app paints a drag region
/// of its own. Called after `build_root` has run, i.e. from the frame pump.
pub fn note_painted_frame() {
    if !vn_tree::painted_drag_region() {
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
                   fallback titlebar (docs/WINDOWING.md §4.2 mode 2)";
        eprintln!("{msg}");
        vn_tree::log::record("window", msg);
    }
}

/// Fires `DOMContentLoaded` + `load` once, after the app bundle has evaluated.
/// Several reused browser components wait on these; redjs has exported the hook
/// since day one and nothing ever called it.
pub fn content_loaded() {
    if BOOTED.with(|b| b.replace(true)) {
        return;
    }
    JsRuntime::with(|rt| unsafe { vn_runtime::redjs::ffi::redjs_dom_content_loaded(rt.raw()) });
}

/// Calls the registered observer. Returns its boolean — which is the
/// `beforeunload` veto when `kind` is `close-requested`.
fn notify(kind: &str, state_json: &str) -> bool {
    let f = OBSERVER.with(|o| o.get());
    if f == 0 {
        return true;
    }
    JsRuntime::with(|rt| {
        let args = [rt.ctx().string(kind), rt.ctx().string(state_json)];
        let out = match rt.ctx().call(Value(f), Value::undefined(), &args) {
            Ok(v) => v.as_bool().unwrap_or(true),
            Err(vn_runtime::redjs::api::Error::Exception(e)) => {
                let err = rt.describe(e);
                eprintln!("[window observer] {}\n{}", err.header, err.stack);
                vn_tree::log::record("error", format!("[window observer] {}", err.header));
                true
            }
            Err(_) => true,
        };
        // Vue schedules its patch flush on microtasks; drain now so a handler's
        // DOM mutations land inside this frame's wave, exactly as an input
        // event's do.
        rt.ctx().drain_microtasks();
        out
    })
}

/// `beforeunload` — registered on the `open_window` builder.
///
/// `Window::on_window_should_close` is a SYNCHRONOUS boolean, which is
/// precisely `beforeunload`'s shape (the web event is synchronous too). v1
/// shows no UA confirmation dialog: `Window::prompt` returns a
/// `oneshot::Receiver`, so bridging it into a sync bool means either a visible
/// "refused to close, then closed" flicker or a modal Win32 `MessageBox` off
/// the gpui path (D-27). We veto and hand the app `close-requested` instead.
pub fn should_close() -> bool {
    let json = SNAPSHOT.with(|s| snapshot_json(&s.borrow()));
    notify("close-requested", &json)
}

// ---------------------------------------------------------------------------
// mutators — queued rather than thrown when no Commit phase is live
// ---------------------------------------------------------------------------

enum Op {
    Title(String),
    Minimize,
    Maximize,
    Unmaximize,
    ToggleMaximize,
    Close,
    Fullscreen(bool),
    Activate(bool),
    Bounds { x: Option<f32>, y: Option<f32>, w: Option<f32>, h: Option<f32>, content: bool },
    Center,
    AlwaysOnTop(bool),
    SetBackdrop(Backdrop),
    StartDragging,
}

fn apply(op: Op, window: &mut Window, _cx: &mut gpui::App) {
    match op {
        Op::Title(t) => {
            window.set_window_title(&t);
            LIVE.with(|l| l.borrow_mut().title = t);
        }
        Op::Minimize => window.minimize_window(),
        // `Window::zoom_window` maps to `ShowWindowAsync(SW_MAXIMIZE)` and does
        // NOT toggle, so the two verbs stay distinct here (fork item F-07 makes
        // it a toggle upstream, matching macOS).
        Op::Maximize => window.zoom_window(),
        Op::Unmaximize => win::restore(),
        Op::ToggleMaximize => {
            if win::is_zoomed() || window.is_maximized() {
                win::restore();
            } else {
                window.zoom_window();
            }
        }
        Op::Close => window.remove_window(),
        // `toggle_fullscreen` is a TOGGLE, so guard every call with the live
        // state or a double `setFullscreen(true)` leaves fullscreen.
        Op::Fullscreen(on) => {
            if window.is_fullscreen() != on {
                window.toggle_fullscreen();
            }
        }
        Op::Activate(raise) => {
            if raise {
                window.activate_window();
            } else {
                window.request_attention();
            }
        }
        Op::Bounds { x, y, w, h, content } => apply_bounds(window, x, y, w, h, content),
        Op::Center => {
            let s = SNAPSHOT.with(|s| s.borrow().clone());
            let (_, _, ow, oh) = s.outer;
            let (_, _, aw, ah, al, at) = s.screen;
            win::set_position(al + (aw - ow) / 2.0, at + (ah - oh) / 2.0);
        }
        Op::AlwaysOnTop(on) => {
            win::set_always_on_top(on);
            LIVE.with(|l| l.borrow_mut().always_on_top = on);
        }
        Op::SetBackdrop(b) => {
            window.set_background_appearance(b.to_gpui());
            LIVE.with(|l| l.borrow_mut().backdrop = b);
        }
        Op::StartDragging => win::start_dragging(),
    }
}

/// `resizeTo`/`moveTo`/`setBounds` — best-effort, non-throwing, clamped to the
/// display's `visible_bounds()`.
///
/// The clamp is the spec's own "optionally, restrain" latitude, and it stops an
/// app parking itself behind the taskbar (D-05). A refused call is a silent
/// no-op, as in every UA.
fn apply_bounds(
    window: &mut Window,
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    h: Option<f32>,
    content: bool,
) {
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
            // `Window::resize` takes a CONTENT size and expands it through
            // `calculate_window_rect` before `SetWindowPos` — which is exactly
            // `resizeTo`'s "resize the viewport" wording, so no divergence.
            window.resize(gpui::size(gpui::px(nw), gpui::px(nh)));
        } else {
            win::set_outer_size(nw * s.scale, nh * s.scale);
        }
    }
    if x.is_some() || y.is_some() {
        let (ox, oy, ow, oh) = s.outer;
        // A partial `setBounds({x})` moves the OUTER origin either way: the
        // asymmetry (`resizeTo` sizes the viewport, `moveTo` places the window)
        // is the CSSOM View spec's, not ours.
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
    }
}

/// Applies now if a `Phase::Commit` is live and we are not inside the dispatch
/// step; otherwise queues. Never throws (D-29).
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

/// `submit` needs the op twice (apply it, or queue it) and `Op` is not `Clone`
/// because `Op::Title` owns a `String`. Rather than clone every op on the hot
/// path, take it by reference and rebuild — the enum is tiny.
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
        Op::Bounds { x, y, w, h, content } => Op::Bounds {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            content: *content,
        },
        Op::Center => Op::Center,
        Op::AlwaysOnTop(b) => Op::AlwaysOnTop(*b),
        Op::SetBackdrop(b) => Op::SetBackdrop(*b),
        Op::StartDragging => Op::StartDragging,
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

// ---------------------------------------------------------------------------
// host functions (docs/WINDOWING.md §5.4)
// ---------------------------------------------------------------------------

fn arg_f32(v: Value) -> Option<f32> {
    let n = v.as_number()?;
    n.is_finite().then_some(n as f32)
}

fn arg_bool(v: Value) -> bool {
    v.as_bool().unwrap_or_else(|| v.as_number().is_some_and(|n| n != 0.0))
}

host_fn!(fn vn_window_state(_g, _args) {
    // The exact-read escape hatch AND the boot seed. Reads the published
    // snapshot, so it costs no Win32 calls and never needs a phase.
    let json = SNAPSHOT.with(|s| snapshot_json(&s.borrow()));
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

host_fn!(fn vn_window_observe(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    OBSERVER.with(|o| o.set(f.0));
    UNDEF
});

host_fn!(fn vn_window_set_title(g, args) {
    let t = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    submit(Op::Title(t));
    UNDEF
});

host_fn!(fn vn_window_verb(g, args) {
    // One host fn for the arity-0 verbs: crossing the boundary costs ~3 ns, but
    // a global per verb costs a name in every app's scope forever.
    let verb = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    match verb.as_str() {
        "minimize" => submit(Op::Minimize),
        "maximize" => submit(Op::Maximize),
        "unmaximize" => submit(Op::Unmaximize),
        "toggleMaximize" => submit(Op::ToggleMaximize),
        "close" => submit(Op::Close),
        "center" => submit(Op::Center),
        "startDragging" => submit(Op::StartDragging),
        other => advise(&format!("unknown appWindow verb {other:?} (host protocol mismatch)")),
    }
    UNDEF
});

host_fn!(fn vn_window_set_fullscreen(_g, args) {
    submit(Op::Fullscreen(arg_bool(args.arg(0))));
    UNDEF
});

host_fn!(fn vn_window_activate(_g, args) {
    // 0 = request attention (the default: this host does not steal focus),
    // 1 = raise, only under `window.focus: "raise"`.
    submit(Op::Activate(arg_bool(args.arg(0))));
    UNDEF
});

host_fn!(fn vn_window_set_bounds(_g, args) {
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

host_fn!(fn vn_window_set_always_on_top(_g, args) {
    submit(Op::AlwaysOnTop(arg_bool(args.arg(0))));
    UNDEF
});

host_fn!(fn vn_window_set_backdrop(g, args) {
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

host_fn!(fn vn_window_set_titlebar_height(_g, args) {
    // The JS half of `setTitleBarOverlay({ height })`. `color`/`symbolColor`
    // are accepted and advised about on the JS side: v1 draws no button
    // cluster, so there is nothing to tint.
    if let Some(h) = arg_f32(args.arg(0)) {
        LIVE.with(|l| l.borrow_mut().title_bar_height = h.max(0.0));
    }
    UNDEF
});

host_fn!(fn vn_window_match_media(g, args) {
    // The SAME evaluator the cascade runs, against the SAME env — which is the
    // whole point: before this, `@media (max-width: 600px)` worked while
    // `matchMedia("(max-width: 600px)")` answered false forever (D-09).
    let query = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let json = match vn_tree::match_media(&query) {
        Ok(m) => Json::obj(vec![("matches", Json::Bool(m))]),
        Err(why) => Json::obj(vec![
            ("matches", Json::Bool(false)),
            ("error", Json::s(format!("`{query}` {why}"))),
        ]),
    }
    .to_json();
    JsRuntime::with(|rt| rt.ctx().string(&json)).0
});

host_fn!(fn vn_window_hitboxes(_g, _args) {
    // The eval-reachable debug hook the acceptance transcript needs: a
    // synthesised `ctl.ts click` cannot exercise `WM_NCHITTEST`, so the only
    // way to prove a drag region was emitted is to read back what the last
    // painted frame handed gpui (docs/WINDOWING.md §13).
    let items: Vec<Json> = vn_tree::control_hitboxes()
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

host_fn!(fn vn_window_cursor(_g, _args) {
    // `screenX`/`screenY` on a synthesised mouse event have no source in the
    // content dispatch path; this is the one place the real cursor position is
    // readable, and it is what `appWindow.startDragging()` docs point at.
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

/// Registers the whole `__vn_window_*` surface.
pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const std::ffi::c_void)] = &[
        ("__vn_window_state", 0, vn_window_state as *const _),
        ("__vn_window_observe", 1, vn_window_observe as *const _),
        ("__vn_window_set_title", 1, vn_window_set_title as *const _),
        ("__vn_window_verb", 1, vn_window_verb as *const _),
        ("__vn_window_set_fullscreen", 1, vn_window_set_fullscreen as *const _),
        ("__vn_window_activate", 1, vn_window_activate as *const _),
        ("__vn_window_set_bounds", 5, vn_window_set_bounds as *const _),
        ("__vn_window_set_always_on_top", 1, vn_window_set_always_on_top as *const _),
        ("__vn_window_set_backdrop", 1, vn_window_set_backdrop as *const _),
        ("__vn_window_set_titlebar_height", 1, vn_window_set_titlebar_height as *const _),
        ("__vn_window_match_media", 1, vn_window_match_media as *const _),
        ("__vn_window_hitboxes", 0, vn_window_hitboxes as *const _),
        ("__vn_window_cursor", 0, vn_window_cursor as *const _),
        ("__vn_window_advise", 1, vn_window_advise as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}
