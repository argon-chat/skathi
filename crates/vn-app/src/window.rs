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
    /// OS-drawn chrome: a real `WS_CAPTION`, the OS's own min/max/close, the
    /// OS's double-click-to-zoom and system menu. Needs fork item F-03
    /// (`dwstyle |= WS_CAPTION` when `!hide_title_bar`), which landed — so this
    /// variant is now reachable and `titleBarStyle: "default"` no longer falls
    /// back.
    ///
    /// In this mode `window-drag` / `--window-control` are INERT (A-25): the OS
    /// owns the caption, `windowControlsOverlay.visible` is `false` and
    /// `display-mode` is `standalone`, which is exactly what a PWA in a
    /// standalone window reports.
    Default,
    /// Custom chrome: the app draws the titlebar, marks it `window-drag: move`
    /// and marks its buttons `--window-control: …`. The v1 default.
    Hidden,
}

/// Native AppKit controls remain visible in transparent custom titlebars.
/// Center their 14px button row in the app's declared strip; AppKit restores
/// the system position during fullscreen transitions.
pub(crate) fn traffic_light_position(custom_titlebar: bool, height: f32) -> Option<gpui::Point<gpui::Pixels>> {
    (cfg!(target_os = "macos") && custom_titlebar).then(|| {
        gpui::point(gpui::px(12.0), gpui::px(((height - 14.0) / 2.0).max(0.0)))
    })
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
            "acrylic" => (Backdrop::Acrylic, None),
            // Windows has ONE blurred material, so `blur` and `acrylic` are the
            // same pixels. Say so rather than let an author believe they picked
            // something different (A-20).
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
            // macOS has one system material, the behind-window blur of an
            // `NSVisualEffectView`, which is gpui's `Blurred` there. Mica and
            // acrylic would otherwise reach gpui's macOS backend as plain
            // transparency.
            #[cfg(target_os = "macos")]
            Backdrop::Mica | Backdrop::MicaAlt | Backdrop::Acrylic => WindowBackgroundAppearance::Blurred,
            #[cfg(not(target_os = "macos"))]
            Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
            #[cfg(not(target_os = "macos"))]
            Backdrop::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
            // DWM's own acrylic, `DWMSBT_TRANSIENTWINDOW` — Electron's
            // `backgroundMaterial: 'acrylic'`. The fork draws the accent
            // policy's blur instead before 22621 ([`Backdrop::resolve_for_os`]
            // says so).
            #[cfg(not(target_os = "macos"))]
            Backdrop::Acrylic => WindowBackgroundAppearance::AcrylicBackdrop,
            Backdrop::Transparent => WindowBackgroundAppearance::Transparent,
        }
    }

    /// One backdrop name from a `<VnWindow backdrop>` prop, resolved for this
    /// OS and with every advisory the config path would have printed.
    ///
    /// The config path ([`load_config`]) collects its advisories into the
    /// config's own list; a `<VnWindow>` has no config to collect into, so it
    /// prints them through [`advise`] — the same text, the same
    /// print-once ledger.
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

    /// DWM draws the system backdrops — Mica, Mica Alt and acrylic — from
    /// Windows 11 22H2 (build 22621). Before that, Mica must advise and fall
    /// back to acrylic, never silently no-op (Tauri's precedent), and acrylic
    /// is the older accent-policy blur, which says what it costs.
    fn resolve_for_os(self) -> (Backdrop, Option<&'static str>) {
        resolve_backdrop(self, has_system_backdrop())
    }
}

/// [`Backdrop::resolve_for_os`] with the OS's answer passed in, so both
/// answers are testable on one machine.
fn resolve_backdrop(b: Backdrop, system_backdrop: bool) -> (Backdrop, Option<&'static str>) {
    match b {
        Backdrop::Mica | Backdrop::MicaAlt if !system_backdrop => (Backdrop::Acrylic, Some(A20B)),
        Backdrop::Acrylic if !system_backdrop => (Backdrop::Acrylic, Some(A20C)),
        other => (other, None),
    }
}

/// The backdrop the primary window draws right now: `appWindow.backdrop`,
/// unless gpui says it is standing in for the material
/// (`Window::is_background_appearance_drawn`).
///
/// A fullscreen macOS window lives in a Space of its own, with nothing behind
/// it for the `NSVisualEffectView` every system backdrop becomes there to
/// blend. gpui's macOS backend draws the screen's desktop picture inside the
/// window under the blur instead, so the glass stays glass and this answers
/// `backdrop`. Only when that picture cannot be shown (none, one in a
/// privacy-protected folder, a format ImageIO cannot read) does the window
/// fall back to the flat system background: `appWindow.effectiveBackdrop`
/// then reads `none`, and `backdropchange` tells the app to paint its own
/// background. `backdrop` keeps the app's choice. DWM draws Mica and acrylic
/// behind a fullscreen window, and Windows always answers `drawn`.
pub(crate) fn effective_backdrop(b: Backdrop, drawn: bool) -> Backdrop {
    match b {
        Backdrop::Mica | Backdrop::MicaAlt | Backdrop::Acrylic if !drawn => Backdrop::None,
        other => other,
    }
}

/// `appWindow.theme`: which colour scheme the app's windows take, in place of
/// the OS's. Electron's `nativeTheme.themeSource`, with its spellings.
///
/// Two things follow it, as they follow `themeSource` in Electron:
/// - the window's own dark mode (`DWMWA_USE_IMMERSIVE_DARK_MODE`), which is
///   what tints Mica, acrylic and the OS caption;
/// - `prefers-color-scheme`, i.e. every `@media` and `matchMedia` here.
///
/// `System` is the initial value, and it returns both to the OS's setting.
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

    /// The dark-or-light answer for a window whose OS appearance is `os_dark`.
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

/// `titleBarOverlay` — the HOST-drawn caption-button cluster for custom-chrome
/// apps (§2.3, §4.3, D-decisions on the overlay).
///
/// Electron's shape, and Electron's honesty about what it is: the buttons are
/// drawn by the app framework, not by the OS. On Windows a real OS caption
/// cluster requires a real `WS_CAPTION`, which is `titleBarStyle: "default"` —
/// a different mode entirely. What makes these buttons OS-*behaved* rather than
/// hand-rolled is that each carries a `WindowControlArea`, so `WM_NCHITTEST`
/// answers `HTMINBUTTON`/`HTMAXBUTTON`/`HTCLOSE`: the OS performs the action,
/// paints its own hover highlight where it wants to, and — the part no app can
/// fake — `HTMAXBUTTON` is what arms the **Windows 11 snap-layouts flyout**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    /// Background of the cluster strip. `None` = transparent, i.e. whatever the
    /// app painted underneath shows through.
    pub color: Option<u32>,
    /// Glyph colour. `None` = follow the window appearance.
    pub symbol_color: Option<u32>,
}

/// The Win11 caption-button metrics: 46 × 32 epx each, three of them.
/// Hardcoded because they ARE the OS's numbers — an app that wants a different
/// cluster width wants its own buttons, which is mode 1 with no overlay.
pub const OVERLAY_BUTTON_WIDTH: f32 = 46.0;
pub const OVERLAY_BUTTONS: f32 = 3.0;

impl Default for Overlay {
    fn default() -> Self {
        Overlay { color: None, symbol_color: None }
    }
}

impl Overlay {
    /// Total width of the cluster, in CSS px — the amount
    /// `getTitlebarAreaRect()` and `env(titlebar-area-width)` must subtract.
    pub fn width() -> f32 {
        OVERLAY_BUTTON_WIDTH * OVERLAY_BUTTONS
    }
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` → `0xAARRGGBB`, the shape `gpui::rgba`
/// wants. Electron accepts CSS colour strings here; hex is the only form worth
/// carrying without a colour parser, so anything else says so rather than
/// silently painting black.
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
    /// The host-drawn caption-button cluster. `None` = the app draws its own
    /// (`--window-control`), which is the v1 default.
    pub title_bar_overlay: Option<Overlay>,
    pub backdrop: Backdrop,
    pub resizable: bool,
    pub minimizable: bool,
    pub maximizable: bool,
    pub always_on_top: bool,
    /// `window.focus()` policy: `false` = `request_attention` (the default —
    /// this host does not steal focus), `true` = `activate_window` (D-07).
    pub focus_raise: bool,
    /// Do this app's devtools start ON? `vn.config.json`'s TOP-LEVEL
    /// `"devtools": false` turns them off; the builders copy that flag into the
    /// window block, which is the JSON blob the host already reads before the
    /// window exists.
    ///
    /// Since m22 this is the STATIC SPELLING OF `appDevtools.disable()`, not a
    /// key gate: `false` means the process starts with no listener, no
    /// handshake file and no F12 — because the port carries `eval`, and an app
    /// whose author turned the devtools off must not still be holding a
    /// localhost socket that runs arbitrary JS in it. `VN_DEVTOOLS=1` in the
    /// environment overrides it (`devtools::resolve_initial`), which is what
    /// keeps an app from being able to make itself undebuggable.
    ///
    /// Before m22 it gated only the key and the port stayed open. The inversion
    /// and its reasoning are docs/ARCHITECTURE.md §11 decision 12.
    pub devtools: bool,
    /// May F12 open the inspector, given that the devtools are on at all?
    ///
    /// `VN_DEVTOOLS_KEY=off` and nothing else — there is no `vn.config.json`
    /// spelling, because an app that wants the key gone wants
    /// `"devtools": false`. It exists for a harness driving an app over the
    /// wire on a machine somebody is using, where a stray F12 opening a window
    /// is noise; the protocol is deliberately untouched.
    pub devtools_key: bool,
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

/// Does `vn.config.json` want the devtools on? Read once by
/// `devtools::boot`, which owns the runtime switch from then on.
#[cfg(vn_devtools)]
pub fn devtools_config_enabled() -> bool {
    CONFIG.with(|c| c.borrow().devtools)
}

/// May F12 open the inspector, `VN_DEVTOOLS_KEY` having had its say? Read from
/// the loaded config, so it answers from the keystroke interceptor without
/// re-parsing anything.
#[cfg(vn_devtools)]
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

/// Reads the `window` block, applies the environment overrides on top
/// (`VN_WINDOW`, `VN_WINDOW_FOCUS` — the perf and screenshot harnesses depend
/// on winning, D-21), and stashes the result for the rest of the process.
///
/// Called BEFORE `cx.open_window`: this is the only tier that can set
/// creation-time-only gpui options without a visible flash (D-20).
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
            // Fork item F-03 landed (`dwstyle |= WS_CAPTION` when
            // `!hide_title_bar`), so this is a real mode now: the window is
            // NOT frameless, the OS draws the caption and its buttons, and the
            // host's own fallback strip must never appear underneath it.
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
                "`titleBarOverlay` expects `false` or an object `{ height?, color?, symbolColor? }` \
                 — see docs/WINDOWING.md §4.1."
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
    // Same precedence as every other override here: the environment wins, so a
    // harness can turn F12 off (or back on) without editing the app under test.
    //
    // The KEY only, and that is the whole point of it having its own field
    // since m22: `VN_DEVTOOLS_KEY=off` keeps the protocol, so a spec can drive
    // an app over the wire on a machine somebody is using without a stray F12
    // opening a window. Turning the devtools THEMSELVES off is `VN_DEVTOOLS=0`
    // or `"devtools": false`, and both take the port with them.
    match std::env::var("VN_DEVTOOLS_KEY").as_deref() {
        Ok("off" | "none" | "0") => cfg.devtools_key = false,
        Ok("f12" | "on" | "1") => cfg.devtools_key = true,
        _ => {}
    }

    // OS-drawn chrome owns the top strip, so the host's fallback titlebar must
    // never be stacked underneath it — not even when the config asks for it.
    // Said out loud rather than silently ignored (A-26).
    if cfg.title_bar_style == TitleBarStyle::Default {
        if asked_for_fallback {
            advisories.push(A26.to_string());
        }
        cfg.title_bar_fallback = false;
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

const A18: &str = "`frame: false` is treated as `titleBarStyle: \"hidden\"` in v1. A true frameless \
                   window would drop WS_THICKFRAME, which also removes the drop shadow, window \
                   animations and edge resizing.";
const A26: &str = "`titleBarFallback: true` is ignored under `titleBarStyle: \"default\"` — the OS \
                   draws the caption, so a second host-drawn strip underneath it would just eat \
                   32 px of the app. Remove one of the two.";

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

    /// The HWND's shown/hidden state, independently of minimization or focus.
    /// A not-yet-captured HWND keeps the ordinary visible-start default.
    pub fn is_visible() -> bool {
        hwnd().is_none_or(|h| unsafe { IsWindowVisible(h) }.as_bool())
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
            set_always_on_top_hwnd(h, on);
        }
    }

    /// [`set_always_on_top`] against a named window rather than the primary
    /// one — a `<VnWindow alwaysOnTop>` pop-out.
    pub fn set_always_on_top_for(raw: isize, on: bool) {
        set_always_on_top_hwnd(HWND(raw as *mut _), on);
    }

    /// `DWMWA_USE_IMMERSIVE_DARK_MODE` on one window: the dark or light Mica
    /// tint, and the OS caption's colours. What gpui itself writes when the OS
    /// theme changes (`configure_dwm_dark_mode`), here for `appWindow.theme`.
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
        // `SWP_NOACTIVATE`: a palette that jumps in front must not also steal
        // the caret from whatever the user is typing into.
        let _ = unsafe {
            SetWindowPos(h, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
        };
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
    /// not the current one. Treated as minimised: `document.hidden`, and no
    /// frames ([`super::cloaked`]).
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

    /// Reads `DWMWA_SYSTEMBACKDROP_TYPE` back out of DWM — what the compositor
    /// currently believes, not what we last asked for.
    ///
    /// Worth having as a first-class read: `set_background_appearance` drives
    /// TWO independent mechanisms (the DWM system backdrop and the older accent
    /// policy), and the `acrylic → mica` bug was exactly one of them being left
    /// armed. It is also the only machine-checkable answer to "does Mica
    /// survive a real `WS_CAPTION`" — `DWMSBT_MAINWINDOW` reading back on a
    /// captioned window means DWM accepted it.
    ///
    /// `0 = DWMSBT_AUTO`, `1 = NONE`, `2 = MAINWINDOW` (mica),
    /// `3 = TRANSIENTWINDOW` (acrylic), `4 = TABBEDWINDOW` (mica-alt).
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

    /// Reads `DWMWA_USE_IMMERSIVE_DARK_MODE` back out of DWM, for
    /// `appWindow.theme`'s state: what the compositor tints Mica with.
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

    /// The window's monitor and that monitor's rect + work area, PHYSICAL px:
    /// `MonitorFromWindow` + `GetMonitorInfoW`, no enumeration. The display
    /// cache's key (`display_part`).
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
            [a.left, a.top, a.right, a.bottom, w.left, w.top, w.right, w.bottom],
        ))
    }
}

#[cfg(not(target_os = "windows"))]
mod win {
    pub fn window_rect() -> Option<(f32, f32, f32, f32)> { None }
    pub fn is_zoomed() -> bool { false }
    pub fn is_iconic() -> bool { false }
    pub fn is_visible() -> bool { true }
    pub fn restore() {}
    pub fn set_position(_x: f32, _y: f32) {}
    pub fn set_outer_size(_w: f32, _h: f32) {}
    pub fn set_always_on_top(_on: bool) {}
    pub fn set_always_on_top_for(_raw: isize, _on: bool) {}
    pub fn set_dark_mode_for(_raw: isize, _dark: bool) {}
    pub fn start_dragging() {}
    pub fn is_cloaked() -> bool { false }
    pub fn system_backdrop() -> Option<u32> { None }
    pub fn dark_mode() -> Option<bool> { None }
    pub fn cursor_pos() -> Option<(f32, f32)> { None }
    pub fn monitor_key() -> Option<(isize, [i32; 8])> { None }
}

thread_local! {
    /// Each window's HWND, captured once. It never changes for the lifetime of
    /// a window, and caching it is what lets the Win32 helpers work from
    /// `open_window`'s builder closure (where nothing is published yet) as well
    /// as from a host fn.
    ///
    /// **Keyed by window** (docs/MULTIWINDOW.md §5.1): one slot held whichever
    /// HWND won the race, and every Win32 helper — `set_always_on_top`,
    /// `start_dragging`, `is_iconic`, the backdrop calls — would then drive the
    /// wrong OS window. `appWindow.*` still means the PRIMARY one (A-M04);
    /// what the map buys is that a pop-out can be driven deliberately
    /// ([`hwnd_of`]) instead of by accident.
    static HWND_CACHE: RefCell<std::collections::HashMap<vn_tree::WinKey, isize>> =
        RefCell::new(std::collections::HashMap::new());
    /// The window `appWindow.*` and every Win32 helper below address. Set by
    /// [`init`] from inside the primary window's own builder closure.
    static PRIMARY_WINDOW: Cell<vn_tree::WinKey> = const { Cell::new(0) };
}

fn hwnd_raw() -> Option<isize> {
    let primary = PRIMARY_WINDOW.with(|c| c.get());
    if let Some(cached) = hwnd_of(primary) {
        return Some(cached);
    }
    with_commit(|window, _| capture_hwnd(window)).flatten()
}

/// `HWND_TOPMOST` for ONE window, addressed by the HWND rather than by "the
/// window" — which is what `<VnWindow alwaysOnTop>` needs and what the single
/// `HWND_CACHE` slot could not express (docs/MULTIWINDOW.md §5.1).
pub fn set_always_on_top_hwnd(hwnd: isize, on: bool) {
    win::set_always_on_top_for(hwnd, on);
}

/// One window's HWND, if it has already been captured.
pub fn hwnd_of(key: vn_tree::WinKey) -> Option<isize> {
    HWND_CACHE.with(|m| m.borrow().get(&key).copied()).filter(|&h| h != 0)
}

pub fn capture_hwnd(window: &Window) -> Option<isize> {
    // gpui implements `raw_window_handle::HasWindowHandle` on `Window`, but the
    // inherent `Window::window_handle()` shadows the trait method — hence UFCS.
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    let raw = match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }?;
    let key = vn_tree::windows::key(window);
    HWND_CACHE.with(|m| m.borrow_mut().insert(key, raw));
    Some(raw)
}

/// The primary window's HWND, once captured.
pub fn primary_hwnd() -> Option<isize> {
    hwnd_of(PRIMARY_WINDOW.with(|c| c.get()))
}

/// The primary window's device pixels per CSS px, from the latest snapshot
/// (1 before the first one).
pub fn primary_scale() -> f32 {
    let s = SNAPSHOT.with(|s| s.borrow().scale);
    if s > 0.0 { s } else { 1.0 }
}

/// Forgets a window that has closed, so its HWND cannot be handed to a Win32
/// call against a window that no longer exists.
pub fn forget_window(key: vn_tree::WinKey) {
    HWND_CACHE.with(|m| m.borrow_mut().remove(&key));
    #[cfg(target_os = "macos")]
    NS_WINDOW_CACHE.with(|m| m.borrow_mut().remove(&key));
}

#[cfg(target_os = "macos")]
thread_local! {
    /// Each window's `NSWindow`, captured as a dialog owner. Kept apart from
    /// `HWND_CACHE`: every Win32 helper reads that one, and none of them may
    /// be handed an `NSWindow`. vn-desktop only uses a pointer from here after
    /// finding it in `[NSApp windows]`, so a stale one is harmless.
    static NS_WINDOW_CACHE: RefCell<std::collections::HashMap<vn_tree::WinKey, isize>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The native window a file dialog is owned by (vn-desktop `dialog::Owner`):
/// the HWND on Windows; on macOS the `NSWindow` of gpui's `NSView` (its raw
/// handle), which the dialog's sheet attaches to.
pub fn dialog_parent(window: &Window) -> Option<isize> {
    #[cfg(target_os = "macos")]
    return capture_ns_window(window);
    #[cfg(not(target_os = "macos"))]
    return capture_hwnd(window);
}

#[cfg(target_os = "macos")]
fn capture_ns_window(window: &Window) -> Option<isize> {
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    let ns_window = vn_desktop::dialog::ns_window_of_view(h.ns_view.as_ptr())?;
    let key = vn_tree::windows::key(window);
    NS_WINDOW_CACHE.with(|m| m.borrow_mut().insert(key, ns_window));
    Some(ns_window)
}

/// [`dialog_parent`] of a window by key, once captured.
pub fn dialog_parent_of(key: vn_tree::WinKey) -> Option<isize> {
    #[cfg(target_os = "macos")]
    return NS_WINDOW_CACHE.with(|m| m.borrow().get(&key).copied()).filter(|&w| w != 0);
    #[cfg(not(target_os = "macos"))]
    return hwnd_of(key);
}

/// Windows 11 22H2 = build 22621 or later: where `DWMWA_SYSTEMBACKDROP_TYPE`
/// exists, so where DWM draws Mica, Mica Alt and acrylic. The fork's own
/// check (`gpui_windows`' `has_system_backdrop`) is the same number. 21H2
/// (22000) had Mica only through an undocumented attribute the fork never set,
/// so it counts as before.
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
        // `RtlGetVersion` is the only version query Windows does not lie about
        // for an unmanifested process (`GetVersionEx` reports 6.2 forever).
        unsafe { RtlGetVersion(&mut vi) }.is_ok() && vi.dwBuildNumber >= 22621
    })
}

/// macOS always has its system material (`NSVisualEffectView`), which every
/// system backdrop maps to ([`Backdrop::to_gpui`]).
#[cfg(not(target_os = "windows"))]
pub(crate) fn has_system_backdrop() -> bool {
    cfg!(target_os = "macos")
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
    /// Whether the OS window is shown; minimizing and cloaking remain separate.
    pub visible: bool,
    pub fullscreen: bool,
    pub focused: bool,
    pub cloaked: bool,
    pub screen: (f32, f32, f32, f32, f32, f32),
    pub displays: usize,
    pub title: String,
    pub backdrop: Backdrop,
    /// What the window draws: `backdrop`, or `None` while a macOS fullscreen
    /// window cannot show the desktop picture under its glass
    /// ([`effective_backdrop`]).
    pub effective_backdrop: Backdrop,
    pub theme: Theme,
    pub always_on_top: bool,
    pub title_bar_height: f32,
    pub fallback_titlebar: bool,
    pub overlay: Option<Overlay>,
    /// The window CLIENT area, CSS px. `inner_width`/`inner_height` are the app
    /// ROOT's box inside it, which is what a browser calls the viewport.
    pub client: (f32, f32),
    /// `(left, top, right, bottom)` — how far the app root is inset inside the
    /// client area by host-drawn chrome. All zero in custom-chrome mode.
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
    /// The snapshot every getter reads. Published once per frame, so a getter
    /// never needs a phase and never lies about which frame it is describing.
    static SNAPSHOT: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    /// DOM visibility is published at window initialization, before any app JS.
    /// Keep its edge separate from PREV, whose first funnel still emits boot.
    static PAGE_HIDDEN: Cell<Option<bool>> = const { Cell::new(None) };
    // The HWND stays hidden while its first styled frame is prepared. This is
    // internal presentation state, not a page becoming document.hidden.
    static STARTUP_REVEAL_PENDING: Cell<bool> = const { Cell::new(false) };
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
    /// The backdrop gpui was last given for the primary window, so the frame
    /// funnel re-applies one only when `appWindow.backdrop` moves.
    static APPLIED_BACKDROP: Cell<Option<Backdrop>> = const { Cell::new(None) };
}

#[derive(Clone, Debug)]
struct Live {
    title: String,
    backdrop: Backdrop,
    /// `appWindow.theme`. App-wide, as Electron's `nativeTheme` is: pop-outs
    /// follow it too.
    theme: Theme,
    always_on_top: bool,
    title_bar_height: f32,
    overlay: Option<Overlay>,
    /// `false` once the app has painted a `window-drag: move` region of its own
    /// (§4.2 mode 2) — or from the start if the config said so.
    fallback_titlebar: bool,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            title: "vue-native".into(),
            backdrop: Backdrop::Mica,
            theme: Theme::System,
            always_on_top: false,
            title_bar_height: 32.0,
            overlay: None,
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
            theme: Theme::System,
            always_on_top: cfg.always_on_top,
            title_bar_height: cfg.title_bar_height,
            // OS chrome draws the real thing; a host cluster on top of it would
            // be a second set of buttons.
            overlay: (cfg.title_bar_style == TitleBarStyle::Hidden)
                .then_some(cfg.title_bar_overlay)
                .flatten(),
            fallback_titlebar: cfg.title_bar_fallback,
        }
    });
    // `open_window` created the window with this one (`window_background`).
    APPLIED_BACKDROP.with(|a| a.set(Some(cfg.backdrop)));
    // Declared BEFORE anything per-window is published: `set_os_chrome` and
    // `set_env` below both address "the primary window", and until this runs
    // there is no such key (docs/MULTIWINDOW.md §5.6). `build_root` asserts the
    // same thing every frame, from the other side.
    let key = vn_tree::windows::key(window);
    PRIMARY_WINDOW.with(|c| c.set(key));
    vn_tree::windows::set_primary(key);
    capture_hwnd(window);
    // The primary's NSWindow, for a dialog asked outside any window's frame.
    #[cfg(target_os = "macos")]
    dialog_parent(window);
    // In OS-chrome mode the caption belongs to the OS, so `window-drag` and
    // `--window-control` must stop emitting hitboxes entirely — otherwise a
    // stray `move` region in the content would hand the OS `HTCAPTION` for a
    // piece of the client area and swallow its clicks. Inert, and said out loud
    // the first time one is painted (A-25).
    vn_tree::set_os_chrome(cfg.title_bar_style == TitleBarStyle::Default);
    if cfg.always_on_top {
        win::set_always_on_top(true);
    }
    // Before the first snapshot, so nothing can be read through a stale cache.
    window.observe_window_display(|_, _| invalidate_display()).detach();
    cx.observe_displays(|_, _| invalidate_display()).detach();
    let s = snapshot(window, cx);
    SNAPSHOT.with(|c| *c.borrow_mut() = s);
    // Seed the cascade env too, so the very first restyle already knows the
    // real viewport, DPI and chrome mode.
    let dark = effective_dark(window);
    let s = SNAPSHOT.with(|c| c.borrow().clone());
    vn_tree::set_env(css_env(&s, dark, cx.reduce_motion()));
    // A hidden initial window may never receive a platform frame. Publish now
    // so module-scope document.hidden already describes the real HWND.
    publish_visibility(&s);
    // `visibilitychange` on the platform's edge rather than the frame's: a
    // minimised window paints no frames, so the per-frame diff below only ever
    // saw `minimized` flip at the moment it was restored — and then compared
    // two visible snapshots. `hidden` was never reported (measured before this
    // landed: minimise → restore delivered zero `visibilitychange` events).
    window
        .observe_window_visibility(|_, window, cx| visibility_changed(window, cx))
        .detach();
}

/// gpui's visibility observer — on Windows `IsWindowVisible && !IsIconic`,
/// reported from a foreground task after `WM_SIZE` / `WM_SHOWWINDOW`, so never
/// from inside the update that minimised the window.
///
/// Runs the same funnel the frame does, out of band: the snapshot reads
/// shown/hidden and minimized state from the OS, then publishes DOM visibility
/// independently of the `minimize` / `restore` events. The next frame's
/// diff then sees nothing new, so nothing fires twice.
fn visibility_changed(window: &mut Window, cx: &mut gpui::App) {
    if vn_runtime::bridge::poisoned()
        // Nested inside a published phase (a modal loop entered from a host
        // fn): the frame's own diff picks the change up afterwards.
        || vn_runtime::with_app(|_| ()).is_some()
        // Before the first frame: that frame's funnel owns `boot`.
        || PREV.with(|p| p.borrow().is_none())
    {
        return;
    }
    vn_runtime::bridge::with_published(window, cx, vn_runtime::Phase::Commit, || {
        with_commit(|window, cx| pump(window, cx));
        JsRuntime::with(|rt| rt.ctx().drain_microtasks());
    });
}

/// Is the window on another virtual desktop right now? Asked by the frame pump
/// every frame, of DWM directly rather than of the snapshot: while cloaked the
/// snapshot is only refreshed by the background tick.
///
/// A cloaked window is a hidden page, as a minimised one is: `document.hidden`,
/// no `requestAnimationFrame`, timers and sockets on the background tick. A
/// browser does the same (Chromium's Windows occlusion tracker counts a cloaked
/// window as hidden). The difference from minimised is only in how the frames
/// stop: gpui parks a minimised window's frame callback, and a cloaked one keeps
/// being asked to paint every vsync, so the pump declines instead
/// (`crate::pump_frame`).
pub fn cloaked() -> bool {
    win::is_cloaked()
}

/// A background-but-shown window remains visible; hidden, minimized and
/// cloaked windows are hidden pages, independently of one another.
fn hidden(s: &Snapshot) -> bool {
    !s.visible || s.minimized || s.cloaked
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
    }) else { return; };
    JsRuntime::with(|rt| unsafe {
        vn_runtime::redjs::ffi::redjs_dom_set_visibility(rt.raw(), hidden as i32)
    });
    // Paint-only presents must follow the same visibility as the DOM.
    vn_tree::set_page_hidden(hidden);
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

/// The display half of a [`Snapshot`]: `screen.*` and `isExtended`.
#[derive(Clone)]
struct DisplayPart {
    /// What the cached value was read against: the monitor, its rect and work
    /// area (physical px), and the window's scale factor.
    key: Option<(isize, [i32; 8], u32)>,
    screen: (f32, f32, f32, f32, f32, f32),
    displays: usize,
}

thread_local! {
    /// The primary window's [`DisplayPart`]. `None` = read it again.
    static DISPLAY: RefCell<Option<DisplayPart>> = const { RefCell::new(None) };
    /// The primary window's display refresh interval in ms, as of the last read.
    static REFRESH_MS: Cell<Option<f32>> = const { Cell::new(None) };
}

/// Dropped by gpui's display observers (a display added, removed or changed;
/// the window moved to another one), so the next snapshot re-reads.
fn invalidate_display() {
    DISPLAY.with(|d| *d.borrow_mut() = None);
}

/// The display half, cached.
///
/// `Window::display(cx)` is `cx.platform.displays()` plus a find —
/// `EnumDisplayMonitors`, then `GetMonitorInfoW` + `GetDpiForMonitor` per
/// monitor — and `cx.displays().len()` was a second enumeration: both on every
/// snapshot, i.e. every frame. Now they run when gpui's observers say a display
/// changed, or when the cheap per-frame key moves: `MonitorFromWindow` +
/// `GetMonitorInfoW`, two calls. The key is what keeps `screen.avail*` honest —
/// a taskbar resize moves the work area without any display event.
fn display_part(window: &Window, cx: &gpui::App) -> DisplayPart {
    let primary = vn_tree::windows::key(window) == PRIMARY_WINDOW.with(|c| c.get());
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
    let part = DisplayPart { key, screen, displays };
    if primary {
        DISPLAY.with(|d| *d.borrow_mut() = key.is_some().then(|| part.clone()));
        REFRESH_MS.with(|c| c.set(window.refresh_interval(cx).map(|r| r.as_secs_f32() * 1000.0)));
    }
    part
}

/// The primary window's display refresh interval in ms — one frame's budget.
/// `None` before the first snapshot or when the platform does not say.
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

/// Reads the window once. Everything derived — every event, every getter —
/// comes from here, so nothing can observe two different frames at once.
pub fn snapshot(window: &Window, cx: &gpui::App) -> Snapshot {
    let viewport = window.viewport_size();
    let bounds = window.bounds();
    let live = LIVE.with(|l| l.borrow().clone());
    let DisplayPart { screen, displays, .. } = display_part(window, cx);
    let scale = window.scale_factor();
    let fullscreen = window.is_fullscreen();
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
    // `innerWidth`/`innerHeight` are the APP's viewport, which is the app root's
    // painted box — not the window's client area.
    //
    // In custom-chrome mode the two are the same rectangle and nothing changes.
    // In mode 2 the host draws a titlebar, 24 px of padding and a status panel
    // around the root, and all of that is UA CHROME: a browser's `innerHeight`
    // excludes the browser's own toolbars by definition, so reporting the client
    // area here made `window.innerHeight` disagree with
    // `document.documentElement.clientHeight` and with every
    // `getBoundingClientRect()` (which are root-relative) by the height of the
    // chrome — 167 px on `apps/shadcn`. Anything that sizes itself against the
    // viewport (reka's `Select` computes `window.innerHeight - 2*margin`) was
    // that much too optimistic.
    //
    // Before the first painted frame the root has no box yet, so the client area
    // is the honest answer — and it is also the right one, since no chrome has
    // been drawn either.
    let client = (f32::from(viewport.width), f32::from(viewport.height));
    let (inner, inset) = match vn_tree::root_bounds() {
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
        screen,
        displays,
        title: live.title,
        backdrop: live.backdrop,
        effective_backdrop: effective_backdrop(live.backdrop, window.is_background_appearance_drawn()),
        theme: live.theme,
        always_on_top: live.always_on_top,
        title_bar_height: live.title_bar_height,
        fallback_titlebar: live.fallback_titlebar,
        overlay: live.overlay,
    }
}

/// The Window Controls Overlay rect (§2.3) — the area of the titlebar strip
/// available to APP CONTENT, and the same numbers `env(titlebar-area-*)`
/// resolves to.
///
/// Zero, with `visible: false`, whenever the strip does not belong to content:
/// OS chrome, fullscreen, minimised — and while the host's own fallback
/// titlebar is still up, which is the honest answer the merge report's known
/// issue was about. A host-drawn button cluster takes its width off the right.
fn titlebar_area(s: &Snapshot) -> (bool, (f32, f32, f32, f32)) {
    let visible = display_mode(s.fullscreen) == vn_css::DisplayMode::WindowControlsOverlay
        && !s.fullscreen
        && !s.minimized
        && !s.fallback_titlebar;
    if !visible {
        return (false, (0.0, 0.0, 0.0, 0.0));
    }
    let cluster = if s.overlay.is_some() { Overlay::width() } else { 0.0 };
    // Windows puts the caption buttons on the RIGHT, so the content area keeps
    // `x = 0` and loses width. macOS would keep the width and move `x`, which
    // is why the app-facing rule is "read the rect, never assume a side".
    (true, (0.0, 0.0, (s.inner_width - cluster).max(0.0), s.title_bar_height))
}

/// The `@media` + `env()` environment, derived from one snapshot.
fn css_env(s: &Snapshot, dark: bool, reduce_motion: bool) -> vn_css::Env {
    let (_, t) = titlebar_area(s);
    vn_css::Env {
        viewport_width: s.inner_width,
        viewport_height: s.inner_height,
        dark,
        reduce_motion,
        scale_factor: s.scale,
        display_mode: display_mode(s.fullscreen),
        titlebar_area: [t.0, t.1, t.2, t.3],
        coarse_pointer: coarse_pointer(),
        // Desktop Windows reports none. `PlatformWindow::insets()` is the hook
        // if a platform ever does.
        safe_area_insets: [0.0; 4],
    }
}

fn os_dark(window: &Window) -> bool {
    matches!(
        window.appearance(),
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    )
}

/// Whether `window` is dark: `appWindow.theme`, or the OS's setting under
/// `system`. What `prefers-color-scheme` answers.
pub fn effective_dark(window: &Window) -> bool {
    LIVE.with(|l| l.borrow().theme).is_dark(os_dark(window))
}

thread_local! {
    /// The OS appearance each window was last seen with, for [`keep_theme`].
    static OS_DARK_SEEN: RefCell<std::collections::HashMap<vn_tree::WinKey, bool>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Keeps a window's DWM dark mode on `appWindow.theme`. Called every frame, for
/// the primary window and for each pop-out.
///
/// gpui writes the OS's appearance into the window whenever the OS theme
/// changes (`configure_dwm_dark_mode`, after its appearance callback has run),
/// which would quietly undo a forced theme. So the first frame a window shows
/// after an OS change writes the forced one back. A window's first frame counts
/// as a change, which is how a pop-out opened under a forced theme takes it.
/// Nothing is written while the theme is `system`: there gpui's value is right.
fn keep_theme(window: &Window) {
    let key = vn_tree::windows::key(window);
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

/// `appWindow.theme = …`: every open window's dark mode follows at once, and
/// `prefers-color-scheme` on the next frame's env.
fn apply_theme(theme: Theme, window: &Window) {
    LIVE.with(|l| l.borrow_mut().theme = theme);
    let dark = theme.is_dark(os_dark(window));
    let hwnds: Vec<isize> = HWND_CACHE.with(|m| m.borrow().values().copied().filter(|&h| h != 0).collect());
    for h in hwnds {
        win::set_dark_mode_for(h, dark);
    }
}

/// The `@media` + `env()` environment of a SECONDARY window — a `<VnWindow>`
/// pop-out (docs/MULTIWINDOW.md §6).
///
/// Built from that window directly rather than from a `Snapshot`, because a
/// `Snapshot` is 20 fields of `appWindow` state and a pop-out has none of it:
/// it has no observer, no `beforeunload`, no `document.title`. What it has is a
/// viewport, a DPI and a chrome mode, which is exactly what the cascade asks
/// for.
///
/// `viewport` is the box that window's PAINTED ROOT was laid out at — the same
/// definition the primary window uses (`inner_width` is the app root's box, not
/// the client area), so `@media (max-width: 400px)` inside a 380 px pop-out
/// answers against 380 rather than against the main window's 620.
pub fn popout_env(
    window: &Window,
    cx: &gpui::App,
    viewport: (f32, f32),
    os_chrome: bool,
    title_bar_height: f32,
) -> vn_css::Env {
    keep_theme(window);
    let dark = effective_dark(window);
    let display_mode = if os_chrome {
        vn_css::DisplayMode::Standalone
    } else {
        // Same rule as the primary window's (D-11): custom chrome means the
        // titlebar strip belongs to content, which is what a reused PWA
        // component branches on.
        vn_css::DisplayMode::WindowControlsOverlay
    };
    let titlebar_area = if os_chrome {
        [0.0; 4]
    } else {
        [0.0, 0.0, viewport.0, title_bar_height]
    };
    vn_css::Env {
        viewport_width: viewport.0,
        viewport_height: viewport.1,
        dark,
        // Shared by construction: it is an OS preference, and pushing a second
        // value would stomp the one animator (docs/MULTIWINDOW.md §5.3).
        reduce_motion: cx.reduce_motion(),
        scale_factor: window.scale_factor(),
        display_mode,
        titlebar_area,
        coarse_pointer: coarse_pointer(),
        safe_area_insets: [0.0; 4],
    }
}

/// Is a touch screen or a pen digitizer attached? — `any-pointer: coarse`.
///
/// `SM_DIGITIZER` is Windows' own answer and the one Chromium reads for the
/// same media feature. It cannot change without a device being plugged in, and
/// it is asked once per frame otherwise, so the answer is cached.
fn coarse_pointer() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            // NID_INTEGRATED_TOUCH | NID_EXTERNAL_TOUCH | NID_INTEGRATED_PEN |
            // NID_EXTERNAL_PEN — "some digitizer is present", which is exactly
            // what `any-pointer: coarse` asks.
            const SM_DIGITIZER: i32 = 94;
            const ANY_DIGITIZER: i32 = 0x01 | 0x02 | 0x04 | 0x08;
            // SAFETY: a pure read of a system metric; no pointers involved.
            let value = unsafe { windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX(SM_DIGITIZER),
            ) };
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

/// The snapshot as the JSON the JS side mirrors into `screen`, `outerWidth`,
/// `navigator.windowControlsOverlay`, `appWindow.*` and friends.
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
        ("title", Json::s(s.title.clone())),
        ("backdrop", Json::s(s.backdrop.as_str())),
        ("effectiveBackdrop", Json::s(s.effective_backdrop.as_str())),
        ("theme", Json::s(s.theme.as_str())),
        // The window's dark mode as DWM has it, read back like `systemBackdrop`.
        (
            "systemDarkMode",
            match win::dark_mode() {
                Some(v) => Json::Bool(v),
                None => Json::Null,
            },
        ),
        // What DWM believes, read back rather than remembered — the two
        // mechanisms `set_background_appearance` drives can disagree.
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
        // The window CLIENT area, and how far the app root sits inside it.
        // `innerWidth`/`innerHeight` are the ROOT's box (see `snapshot`), so
        // these two are what an app needs to reason about the host's own chrome
        // — and `client - inset` is `inner` by construction.
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

// ---------------------------------------------------------------------------
// the frame funnel
// ---------------------------------------------------------------------------

thread_local! {
    static PREV: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    static BOOTED: Cell<bool> = const { Cell::new(false) };
}

/// Runs inside `pump_frame`'s `Phase::Commit`, BEFORE `rt.pump` — and from
/// `visibility_changed`, since a minimised window has no frames.
///
/// Order is the contract:
/// 1. drain anything a module-scope mutator queued;
/// 2. snapshot;
/// 3. `redjs_dom_resize` when the viewport or the DPI moved — this is what
///    updates `innerWidth`/`innerHeight`/`devicePixelRatio` and fires the
///    trusted `resize`;
/// 4. `redjs_dom_set_visibility` when shown/minimised/cloaked hidden-ness moved;
/// 5. the JS observer, with the state delta.
///
/// Step 3 before step 5 is what makes `fullscreenchange` fire AFTER the
/// viewport has finished resizing, which the Fullscreen spec requires.
pub fn pump(window: &mut Window, cx: &mut gpui::App) {
    drain_pending(window, cx);
    // Before the snapshot, so a backdrop queued since the last frame is
    // applied and the snapshot's `effectiveBackdrop` describes it.
    sync_backdrop(window, false);

    let s = snapshot(window, cx);
    SNAPSHOT.with(|c| *c.borrow_mut() = s.clone());
    let prev = PREV.with(|p| p.borrow().clone());

    let json = snapshot_json(&s);
    let ctx = JsRuntime::with(|rt| rt.raw());

    // Publish the snapshot to JS BEFORE anything fires. `redjs_dom_resize`
    // dispatches the trusted `resize` SYNCHRONOUSLY, so a handler that reads
    // `outerWidth` or `screenX` from it would otherwise see LAST frame's
    // numbers while `innerWidth` already held this frame's — the kind of
    // half-updated view a browser never shows.
    let dirty = prev.as_ref().is_none_or(|p| *p != s);
    if dirty {
        notify("sync", &json);
    }

    // The viewport is a PUSHED value: `innerWidth` / `innerHeight` /
    // `devicePixelRatio` are whatever `redjs_dom_resize` last wrote, and JS has
    // no way to pull them. So every snapshot that moves them pushes — the FIRST
    // one included, which it was not before.
    //
    // Nothing noticed, because the first two snapshots always disagreed: frame
    // 1's had no painted app root, so `snapshot` fell back to the window's
    // client area, and frame 2's had the root's real box. The second frame's
    // "resize" therefore corrected a value the boot frame had never published.
    // The moment a layout could run BEFORE the first frame
    // (`vn_tree::sync_layout`), the two agreed — and JS kept redjs's built-in
    // default viewport for the life of the process.
    //
    // The push is not the `resize` EVENT, which still fires only on a change: a
    // browser does not fire `resize` at load either.
    let viewport_moved = prev.as_ref().is_none_or(|p| {
        p.inner_width != s.inner_width || p.inner_height != s.inner_height || p.scale != s.scale
    });
    if viewport_moved {
        unsafe {
            vn_runtime::redjs::ffi::redjs_dom_resize(
                ctx,
                s.inner_width as i32,
                s.inner_height as i32,
                s.scale as f64,
            )
        };
    }

    // This also seeds visibility if no window-init publication was made.
    // The separate cache prevents an extra visibilitychange on the first pump.
    publish_visibility(&s);

    let mut kinds: Vec<&str> = Vec::new();
    match &prev {
        None => kinds.push("boot"),
        Some(p) => {
            if viewport_moved {
                kinds.push("resize");
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
            if backdrop_moved(p, &s) {
                kinds.push("backdropchange");
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
                // Retiring the fallback titlebar hands the app the top strip:
                // the titlebar area is genuinely new geometry.
                || p.fallback_titlebar != s.fallback_titlebar
                // Showing/hiding the host cluster changes the rect's WIDTH,
                // which is the whole reason `geometrychange` exists.
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

    // The env every `@media` condition and every live `MediaQueryList` is
    // evaluated against. Published here rather than after the pump so a
    // `matchMedia(...).matches` read from a `resize` handler already sees the
    // new viewport.
    keep_theme(window);
    let dark = effective_dark(window);
    let env_changed = vn_tree::set_env(css_env(&s, dark, cx.reduce_motion()));
    if env_changed && BOOTED.with(|b| b.get()) {
        IN_DISPATCH.with(|f| f.set(true));
        notify("mediachange", &json);
        IN_DISPATCH.with(|f| f.set(false));
    }
}

/// `backdropchange`: the app's backdrop, or what the window can draw of it,
/// moved since the last frame.
fn backdrop_moved(p: &Snapshot, s: &Snapshot) -> bool {
    p.backdrop != s.backdrop || p.effective_backdrop != s.effective_backdrop
}

/// Hands gpui `appWindow.backdrop` when it is not the one gpui last got.
/// `force` re-applies it regardless, as an explicit `appWindow.backdrop = …`
/// always did.
///
/// Fullscreen does not change what gpui is given: its macOS backend keeps the
/// blur in native fullscreen by drawing the desktop picture under it, and says
/// when it cannot ([`effective_backdrop`]). Swapping the material out for
/// `none` there, as this once did, left the window opaque black under a page
/// that stays transparent.
fn sync_backdrop(window: &mut Window, force: bool) {
    let want = LIVE.with(|l| l.borrow().backdrop);
    if !force && APPLIED_BACKDROP.with(|a| a.get()) == Some(want) {
        return;
    }
    APPLIED_BACKDROP.with(|a| a.set(Some(want)));
    window.set_background_appearance(want.to_gpui());
    // The renderer reads the appearance when it draws: `none` clears to
    // opaque and every material to transparent, and text drops subpixel AA
    // over a material. Until a frame is drawn, the window keeps the last one,
    // so `material → none` left it see-through.
    window.refresh();
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
    // `close-requested` comes from gpui's should-close callback, outside any
    // publication: an owed checkpoint runs first (vn_runtime::bridge::owe_drain).
    vn_runtime::bridge::settle_owed_drain();
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
    SetTheme(Theme),
    StartDragging,
}

/// Re-reads the window into gpui's own caches after a SYNCHRONOUS `SetWindowPos`.
///
/// This is the fix for the "`setBounds({width, height})` resizes without a
/// relayout" bug. `SetWindowPos` **sends** `WM_SIZE`/`WM_MOVE` rather than
/// posting them, so the fork's `handle_size_msg` runs re-entrantly, inside this
/// frame's `Phase::Commit` — i.e. while `App` is already mutably borrowed.
/// gpui's `on_resize` callback is
/// `handle.update(&mut cx, |_, window, cx| window.bounds_changed(cx)).log_err()`:
/// that `update` cannot take the borrow, fails, and **`log_err` swallows it**.
/// The platform layer's `logical_size` is updated (so `Window::bounds()` and
/// `GetWindowRect` both look right), but `Window::viewport_size` — what
/// `innerWidth`, the cascade env and taffy's root box all read — keeps last
/// frame's number forever.
///
/// `maximize()` is unaffected because `Window::zoom_window` goes through
/// `ShowWindowAsync`, and `setContentBounds` because `PlatformWindow::resize`
/// spawns its `SetWindowPos` on the executor: in both cases `WM_SIZE` arrives
/// later, on the message loop, with nothing borrowed.
///
/// So we do by hand exactly what the swallowed callback would have done. The
/// platform state is already correct by the time `SetWindowPos` returns, which
/// is what makes this a re-read rather than a guess.
fn resync_bounds(window: &mut Window, cx: &mut gpui::App) {
    window.bounds_changed(cx);
}

fn apply(op: Op, window: &mut Window, cx: &mut gpui::App) {
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
        // `appWindow` is the primary window, so this closes the app: the
        // windows go now and the teardown runs behind them (src/shutdown.rs).
        // Its pop-outs go with it, as on a WM_CLOSE (docs/MULTIWINDOW.md
        // §7.8): left open, they would keep the app running with no window
        // on screen. Removed after a file dialog this window owns has closed
        // (dialogs.rs).
        Op::Close => {
            crate::shutdown::begin("close()");
            if crate::popout::count() > 0 {
                crate::popout::close_all(cx);
            }
            if !vn_desktop::dialog::owner_closing(vn_tree::windows::key(window)) {
                window.remove_window();
            } else {
                // gpui's own handle, not raw-window-handle's (in scope here).
                let handle = gpui::Window::window_handle(window);
                cx.defer(move |cx| crate::dialogs::remove_window(handle, cx));
            }
        }
        // `toggle_fullscreen` is a TOGGLE, so guard every call with the live
        // state or a double `setFullscreen(true)` leaves fullscreen.
        Op::Fullscreen(on) => {
            if window.is_fullscreen() != on {
                window.toggle_fullscreen();
            }
        }
        // A test host neither raises nor flashes its taskbar button
        // (src/quiet.rs).
        Op::Activate(_) if crate::quiet::never_focus() => {}
        Op::Activate(raise) => {
            if raise {
                window.activate_window();
            } else {
                window.request_attention();
            }
        }
        Op::Bounds { x, y, w, h, content } => apply_bounds(window, cx, x, y, w, h, content),
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
        // Kept even while fullscreen suspends it: leaving fullscreen draws the
        // backdrop the app asked for last.
        Op::SetBackdrop(b) => {
            LIVE.with(|l| l.borrow_mut().backdrop = b);
            sync_backdrop(window, true);
        }
        Op::SetTheme(t) => {
            apply_theme(t, window);
            // The env (`prefers-color-scheme`) is re-read on the next frame;
            // ask for one, since nothing else may be painting.
            window.refresh();
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
            // `Window::resize` takes a CONTENT size and expands it through
            // `calculate_window_rect` before `SetWindowPos` — which is exactly
            // `resizeTo`'s "resize the viewport" wording, so no divergence.
            window.resize(gpui::size(gpui::px(nw), gpui::px(nh)));
        } else {
            win::set_outer_size(nw * s.scale, nh * s.scale);
            moved_synchronously = true;
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
        moved_synchronously = true;
    }
    // `Window::resize` (the `content` branch above) spawns its `SetWindowPos`
    // on the executor, so its `WM_SIZE` lands outside this borrow and gpui
    // refreshes itself; the two Win32 escape hatches send theirs re-entrantly
    // and need the re-read done by hand. See [`resync_bounds`].
    if moved_synchronously {
        resync_bounds(window, cx);
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

/// Element fullscreen explicitly addresses the primary window, even when
/// JavaScript was entered from a pop-out's input listener. Its operation is
/// consumed by the primary pump rather than applied to the published window.
pub(crate) fn element_fullscreen(on: Option<bool>) -> bool {
    let current = SNAPSHOT.with(|snapshot| snapshot.borrow().fullscreen);
    if let Some(on) = on {
        PENDING.with(|pending| pending.borrow_mut().push_back(Op::Fullscreen(on)));
    }
    current
}

/// `appWindow.close()`, asked from Rust: the embedder's `Handle::close`
/// (src/embed.rs). Main thread.
pub(crate) fn request_close() {
    submit(Op::Close);
}

/// The embedder shows the main window through gpui's activation path, which
/// applies an initially hidden window's stored placement before showing it.
pub(crate) fn request_activate() {
    submit(Op::Activate(true));
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
        Op::SetTheme(t) => Op::SetTheme(*t),
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
    // The exact-read escape hatch AND the boot seed.
    //
    // Re-reads the window when a Commit phase is live, which is what makes
    // `appWindow.isMaximized()` exact IMMEDIATELY after `maximize()` — the
    // state verbs go through live Win32 queries (`IsZoomed`/`IsIconic`), so
    // there is no reason to make a caller wait a frame for them. Geometry
    // still lags by one frame, because gpui's bounds cache is refreshed from
    // `WM_MOVE`/`WM_SIZE` (docs/WINDOWING.md §3.1) — and the CSSOM View spec
    // tells authors to expect exactly that from `moveTo`.
    //
    // Outside a Commit phase (module scope, a timer that outran the pump) it
    // falls back to the published snapshot rather than throwing: nothing on
    // this surface may throw the phase error.
    if let Some(s) = with_commit(|window, cx| snapshot(window, cx)) {
        SNAPSHOT.with(|c| *c.borrow_mut() = s);
    }
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

host_fn!(fn vn_window_set_theme(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    match Theme::from_str(&name) {
        Some(t) => submit(Op::SetTheme(t)),
        None => advise(&format!("unknown `theme`: {name:?} — expected system | light | dark.")),
    }
    UNDEF
});

host_fn!(fn vn_window_set_overlay(g, args) {
    // The host half of `setTitleBarOverlay({ height, color, symbolColor })`.
    //
    // `height` alone (arg 1 absent) keeps whatever cluster state is live, which
    // is what `defineWindow({ titleBarHeight })` means. Passing an explicit
    // `false` retires the cluster; passing an object installs or updates it.
    // Colours arrive as `#rrggbb` strings, so the parser is shared with the
    // config path rather than duplicated in JS.
    if let Some(h) = arg_f32(args.arg(0)) {
        LIVE.with(|l| l.borrow_mut().title_bar_height = h.max(0.0));
    }
    let mode = args.arg(1).as_number().unwrap_or(-1.0);
    let read = |i: u32| unsafe { read_string(g, args.arg(i).0) }.unwrap_or_default();
    match mode {
        // 0 = `titleBarOverlay: false`.
        m if m == 0.0 => LIVE.with(|l| l.borrow_mut().overlay = None),
        // 1 = an object; colours are empty strings when not given.
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

const A27: &str = "`setTitleBarOverlay()` does nothing under `titleBarStyle: \"default\"` — the OS \
                   draws the caption buttons and owns their colours. The overlay is for \
                   custom-chrome windows (docs/WINDOWING.md §4.2 mode 1).";

/// The host-drawn cluster the Shell paints, or `None`. Read once per frame by
/// `Shell::render_body` — never drawn in fullscreen (drag regions and caption
/// buttons are both inert there) or while the window is minimised.
pub fn overlay() -> Option<(f32, Overlay)> {
    let s = SNAPSHOT.with(|s| s.borrow().clone());
    if s.fullscreen || s.minimized {
        return None;
    }
    s.overlay.map(|o| (s.title_bar_height, o))
}

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

// `__vn_window_hitboxes(windowId?)` — what the last frame of ONE window handed
// gpui. Absent = the primary window, which is what it always meant.
//
// The window argument is phase 1's (docs/MULTIWINDOW.md §7.6): the hitbox list
// describes the chrome of the window that emitted it, and a `<VnWindow
// chrome="hidden">` draws its own caption. With one global list a pop-out's
// build CLEARED the primary window's, which is both a wrong answer here and a
// live bug in the fallback-titlebar retire rule.
host_fn!(fn vn_window_hitboxes(_g, args) {
    // The eval-reachable debug hook the acceptance transcript needs: a
    // synthesised `ctl.ts click` cannot exercise `WM_NCHITTEST`, so the only
    // way to prove a drag region was emitted is to read back what the last
    // painted frame handed gpui (docs/WINDOWING.md §13).
    let key = match args.arg(0).as_number() {
        Some(n) if n > 0.0 => n as u64,
        _ => vn_tree::windows::primary(),
    };
    let items: Vec<Json> = vn_tree::control_hitboxes_in(key)
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
        ("__vn_window_set_theme", 1, vn_window_set_theme as *const _),
        ("__vn_window_set_overlay", 4, vn_window_set_overlay as *const _),
        ("__vn_window_match_media", 1, vn_window_match_media as *const _),
        ("__vn_window_hitboxes", 1, vn_window_hitboxes as *const _),
        ("__vn_window_cursor", 0, vn_window_cursor as *const _),
        ("__vn_window_advise", 1, vn_window_advise as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `appWindow.theme` takes `nativeTheme.themeSource`'s three spellings and
    /// nothing else; `system` is the OS's answer, the other two override it.
    #[test]
    fn the_theme_takes_electrons_spellings_and_forces_only_when_asked() {
        for t in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_str(t.as_str()), Some(t));
        }
        assert_eq!(Theme::from_str("Dark"), None, "the client's config spelling is mapped before this");
        assert_eq!(Theme::from_str("auto"), None);
        assert!(Theme::System.is_dark(true) && !Theme::System.is_dark(false));
        assert!(!Theme::Light.is_dark(true) && Theme::Dark.is_dark(false));
    }

    /// `acrylic` is DWM's own acrylic wherever DWM draws system backdrops, as
    /// Electron's `backgroundMaterial: 'acrylic'` is, and says nothing there.
    /// Before 22621 it is the accent-policy blur, and that is advised; Mica
    /// falls back to it, advised too. `blur` is the same material, advised
    /// wherever it is asked for.
    #[test]
    fn acrylic_is_the_system_material_and_only_its_fallback_is_advised() {
        assert_eq!(Backdrop::from_str("acrylic"), Some((Backdrop::Acrylic, None)));
        assert_eq!(Backdrop::from_str("blur"), Some((Backdrop::Acrylic, Some(A20))));
        assert_eq!(Backdrop::Acrylic.to_gpui(), WindowBackgroundAppearance::AcrylicBackdrop);

        for b in [Backdrop::None, Backdrop::Mica, Backdrop::MicaAlt, Backdrop::Acrylic, Backdrop::Transparent] {
            assert_eq!(resolve_backdrop(b, true), (b, None), "{b:?} on 22621+");
        }
        assert_eq!(resolve_backdrop(Backdrop::Acrylic, false), (Backdrop::Acrylic, Some(A20C)));
        assert_eq!(resolve_backdrop(Backdrop::Mica, false), (Backdrop::Acrylic, Some(A20B)));
        assert_eq!(resolve_backdrop(Backdrop::MicaAlt, false), (Backdrop::Acrylic, Some(A20B)));
        assert_eq!(resolve_backdrop(Backdrop::None, false), (Backdrop::None, None));

        // The config path says nothing about `acrylic` on this machine's
        // Windows when DWM draws it, and the value survives the load.
        let cfg = load_config(Some(r#"{"backdrop":"acrylic"}"#.to_string()));
        assert_eq!(cfg.backdrop, Backdrop::Acrylic);
        let advised = ADVISED.with(|s| s.borrow().contains(A20C));
        assert_eq!(advised, !has_system_backdrop());
    }

    /// `titleBarOverlay.color` takes CSS hex, which is what Electron's option
    /// takes. `0xAARRGGBB` is what `gpui::rgba` reads.
    #[test]
    fn hex_colours_parse_in_all_three_lengths() {
        assert_eq!(parse_color("#fff"), Some(0xffff_ffff));
        assert_eq!(parse_color("#202020"), Some(0xff20_2020));
        assert_eq!(parse_color("#20202080"), Some(0x8020_2020));
        assert_eq!(parse_color("  #c42b1c  "), Some(0xffc4_2b1c));
        // Not hex, not a colour: refused rather than silently painted black.
        assert_eq!(parse_color("rgb(1,2,3)"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#zzz"), None);
    }

    /// The WCO rect is zero — and `visible` false — in every mode where the
    /// titlebar strip does NOT belong to app content. The last of those cases
    /// is the one the merge report filed: while the host's own fallback
    /// titlebar is still up, the strip is the host's.
    #[test]
    fn the_titlebar_rect_is_only_non_zero_when_the_strip_belongs_to_content() {
        let base = Snapshot {
            inner_width: 1000.0,
            title_bar_height: 32.0,
            fallback_titlebar: false,
            ..Snapshot::default()
        };
        assert_eq!(titlebar_area(&base), (true, (0.0, 0.0, 1000.0, 32.0)));

        let hosted = Snapshot { fallback_titlebar: true, ..base.clone() };
        assert_eq!(titlebar_area(&hosted), (false, (0.0, 0.0, 0.0, 0.0)));

        let full = Snapshot { fullscreen: true, ..base.clone() };
        assert_eq!(titlebar_area(&full), (false, (0.0, 0.0, 0.0, 0.0)));

        let min = Snapshot { minimized: true, ..base.clone() };
        assert_eq!(titlebar_area(&min), (false, (0.0, 0.0, 0.0, 0.0)));
        let cloaked = Snapshot { cloaked: true, ..base.clone() };
        let unfocused = Snapshot { focused: false, ..base.clone() };
        let not_shown = Snapshot { visible: false, ..base.clone() };
        assert!(!not_shown.minimized, "a hidden window need not be minimized");
        assert_eq!((hidden(&base), hidden(&min), hidden(&cloaked), hidden(&unfocused), hidden(&not_shown)), (false, true, true, false, true));

        // A host-drawn cluster takes its width off the RIGHT on Windows, so the
        // content area keeps x = 0 and loses width (3 x 46 px).
        let overlaid = Snapshot { overlay: Some(Overlay::default()), ..base.clone() };
        assert_eq!(titlebar_area(&overlaid), (true, (0.0, 0.0, 862.0, 32.0)));
    }

    /// A system material reads as drawn unless gpui stands in for it (a macOS
    /// fullscreen window without a desktop picture to put under the glass);
    /// fullscreen alone no longer suspends it, on either OS.
    #[test]
    fn a_system_backdrop_is_suspended_only_while_gpui_stands_in_for_it() {
        for b in [Backdrop::Mica, Backdrop::MicaAlt, Backdrop::Acrylic] {
            assert_eq!(effective_backdrop(b, true), b, "{b:?} drawn, windowed or fullscreen");
            assert_eq!(effective_backdrop(b, false), Backdrop::None, "{b:?} with no desktop picture");
        }
        // Not a system material: what the app asked for is what it gets.
        for b in [Backdrop::None, Backdrop::Transparent] {
            assert_eq!(effective_backdrop(b, true), b);
            assert_eq!(effective_backdrop(b, false), b);
        }
    }

    /// Losing and regaining the desktop picture, and an explicit backdrop
    /// change, each fire `backdropchange` once; a frame with neither fires
    /// nothing — entering fullscreen with the picture shown included.
    #[test]
    fn backdropchange_follows_the_backdrop_and_what_the_window_draws() {
        let windowed = Snapshot { backdrop: Backdrop::Acrylic, effective_backdrop: Backdrop::Acrylic, ..Snapshot::default() };
        let full_glass = Snapshot { fullscreen: true, ..windowed.clone() };
        assert!(!backdrop_moved(&windowed, &full_glass), "the glass stays in fullscreen");
        let full = Snapshot {
            fullscreen: true,
            effective_backdrop: effective_backdrop(Backdrop::Acrylic, false),
            ..windowed.clone()
        };
        let solid = Snapshot { backdrop: Backdrop::None, effective_backdrop: Backdrop::None, ..windowed.clone() };
        assert!(backdrop_moved(&windowed, &full));
        assert!(backdrop_moved(&full, &windowed));
        assert!(backdrop_moved(&windowed, &solid));
        assert!(!backdrop_moved(&full, &full.clone()));
        // Asking for a material while fullscreen changes `backdrop` only, and
        // still fires: the app's choice moved even though the window did not.
        let full_solid = Snapshot { backdrop: Backdrop::None, ..full.clone() };
        assert!(backdrop_moved(&full, &full_solid));
    }

    #[test]
    fn startup_prewarm_does_not_publish_a_hidden_page_or_mask_later_hides() {
        cancel_startup_reveal();
        assert!(!startup_visibility(false), "visible(false) embedding remains hidden");
        begin_startup_reveal();
        assert!(startup_visibility(false), "the hidden HWND is only an internal first-frame warm-up");
        assert!(startup_visibility(true), "the native show completes the warm-up");
        assert!(!startup_visibility(false), "later explicit hides must be observable");
        begin_startup_reveal();
        cancel_startup_reveal();
        assert!(!startup_visibility(false), "cancelled startup must not retain the override");
    }

    #[test]
    fn visibility_is_seeded_at_boot_and_only_actual_hidden_edges_publish() {
        let shown = Snapshot::default();
        let hidden = Snapshot { visible: false, ..shown.clone() };
        assert_eq!(visibility_delta(None, &shown), Some(false));
        assert_eq!(visibility_delta(None, &hidden), Some(true));
        assert_eq!(visibility_delta(Some(true), &hidden), None, "the first pump must not repeat the boot publication");
        assert_eq!(visibility_delta(Some(true), &shown), Some(false), "showing restores DOM visibility");
        assert_eq!(visibility_delta(Some(false), &hidden), Some(true), "hiding does not require minimization");
        let minimized = Snapshot { minimized: true, ..shown.clone() };
        let cloaked = Snapshot { cloaked: true, ..shown.clone() };
        assert_eq!(visibility_delta(Some(true), &minimized), None, "hidden-to-minimized is still a hidden page");
        assert_eq!(visibility_delta(Some(true), &cloaked), None);
        let unfocused = Snapshot { focused: false, ..shown };
        assert_eq!(visibility_delta(Some(false), &unfocused), None, "focus is independent of visibility");
    }

    /// The rect the CSS side resolves `env(titlebar-area-*)` from is the SAME
    /// one the JS getter reports — one function, two consumers, so they cannot
    /// drift apart.
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
        assert_eq!(env.titlebar_area, [0.0, 0.0, 800.0, 36.0]);
        assert_eq!(env.viewport_width, 800.0);
        assert_eq!(env.viewport_height, 600.0);
        assert_eq!(env.safe_area_insets, [0.0; 4]);
    }

    /// `titleBarStyle: "default"` no longer advises and falls back (fork item
    /// F-03 landed), and it forces the host's own fallback strip off — two
    /// titlebars stacked is not a mode.
    #[test]
    fn os_chrome_is_a_real_mode_and_refuses_a_second_titlebar() {
        let cfg = load_config(Some(
            r#"{"titleBarStyle":"default","titleBarFallback":true}"#.to_string(),
        ));
        assert_eq!(cfg.title_bar_style, TitleBarStyle::Default);
        assert!(!cfg.title_bar_fallback);
    }

    /// `"titleBarFallback": false` is the one-line answer to windowing-2 §8.11:
    /// an app that declares its own titlebar and sizes it purely from
    /// `env(titlebar-area-height)` sees 0 for the ONE frame before the host's
    /// fallback strip retires, because the strip legitimately belongs to the
    /// host until then. Turning the fallback off means it never belongs to the
    /// host, so the rect is real from the first frame.
    ///
    /// This walks the whole chain — JSON → `Config` → the boot `Snapshot` →
    /// `env(titlebar-area-*)` — because every link in it already existed and
    /// what was never checked is that they agree.
    #[test]
    fn title_bar_fallback_false_gives_a_real_env_rect_on_the_first_frame() {
        let cfg = load_config(Some(
            r#"{"titleBarStyle":"hidden","titleBarHeight":36,"titleBarFallback":false}"#
                .to_string(),
        ));
        assert!(!cfg.title_bar_fallback);

        // The boot snapshot, as `init` builds it: `fallback_titlebar` is the
        // config value verbatim, and nothing has painted yet.
        let boot = Snapshot {
            inner_width: 980.0,
            inner_height: 720.0,
            title_bar_height: cfg.title_bar_height,
            fallback_titlebar: cfg.title_bar_fallback,
            ..Snapshot::default()
        };
        assert_eq!(titlebar_area(&boot), (true, (0.0, 0.0, 980.0, 36.0)));
        assert_eq!(css_env(&boot, false, false).titlebar_area, [0.0, 0.0, 980.0, 36.0]);

        // …and the default (fallback ON) is the zero rect until it retires,
        // which is the behaviour the report described and is still correct:
        // the strip is the host's, and saying otherwise would be the lie.
        let default_cfg = load_config(Some(
            r#"{"titleBarStyle":"hidden","titleBarHeight":36}"#.to_string(),
        ));
        assert!(default_cfg.title_bar_fallback);
        let first_frame = Snapshot { fallback_titlebar: true, ..boot.clone() };
        assert_eq!(titlebar_area(&first_frame), (false, (0.0, 0.0, 0.0, 0.0)));
        assert_eq!(css_env(&first_frame, false, false).titlebar_area, [0.0; 4]);
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
            Some(Overlay { color: Some(0xff20_2020), symbol_color: Some(0xffff_ffff) })
        );

        // `false` (and absent) means the app draws its own — the v1 default.
        let cfg = load_config(Some(r#"{"titleBarOverlay":false}"#.to_string()));
        assert_eq!(cfg.title_bar_overlay, None);
        let cfg = load_config(Some(r#"{}"#.to_string()));
        assert_eq!(cfg.title_bar_overlay, None);
    }
}
