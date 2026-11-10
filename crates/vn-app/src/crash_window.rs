//! The crash dialog: what the user sees once a native panic has poisoned the
//! runtime (invariant I9, docs/ARCHITECTURE.md §1), drawn by the host with no
//! JavaScript at all.
//!
//! ```text
//!   host fn panics ── host_fn!'s catch ── bridge::panic_to_js: poison_with("<msg> at <loc>"),
//!                                          throw `native panic at …` into JS
//!   next frame (pump_frame) or background tick ── poisoned() ── on_poisoned (once)
//!        │ log one `[crash]` line; a test host stops here
//!        ▼
//!   the main window restored if minimised ── the dialog, owned by it, disables it
//!        │
//!   Restart ── shutdown::begin ── the exe again, same argv ── close_primary ── gpui quit
//!   Close / Esc / Alt+F4 ── shutdown::begin ── close_primary ── gpui quit
//! ```
//!
//! # Why the host draws it
//!
//! A poisoned runtime runs no JS ever again: the frame pump returns before
//! its wave, the background tick likewise. Whatever error screen the app's
//! own code put up from the thrown `Error` (the Argon client's Vue
//! ErrorBoundary: "Something went wrong / Reload app") is painted, but its
//! buttons are JS, and `location.reload()` runs inside the very wave that no
//! longer runs (src/reload.rs). The user saw a "Reload app" that did nothing.
//! The only way out of a poisoned process is a new process, so the dialog
//! offers exactly that, and it must not need the runtime to do it.
//!
//! # What it looks like
//!
//! The permission prompt's Windows 11 ContentDialog (src/content_dialog.rs):
//! transient acrylic, round corners, the system's theme (live) and accent, the
//! app's icon and name in the caption row. In place of the capability's glyph,
//! Segoe Fluent Icons' Warning triangle in a circle tinted with Windows'
//! critical red (`SystemFillColorCritical`, not the accent); the title "*App*
//! ran into a problem"; a sentence saying what happened and what to do; a
//! collapsed **Details** expander with the panic's message and location in a
//! monospace well (Consolas; scrolling past six lines, the window growing
//! about its centre to fit it) and the app's version and the OS build; and
//! the buttons **Restart** (accent,
//! focused, so Enter), **Copy report** (a plain-text report to the
//! clipboard; the label reads "Copied" for two seconds) and **Close**. English,
//! or Russian when that is the Windows display language. Wider than the
//! prompt (three buttons, the longest «Скопировать отчёт»).
//!
//! Input is the prompt's: Tab and the arrow keys, Enter and Space, the access
//! keys (Alt+R / C / L, Alt+П / С / З), the focus ring, and its input
//! protection, so a key or a click aimed at the app the moment it broke does
//! not restart it.
//!
//! # What the buttons do
//!
//! **Restart** relaunches the process: the close a window's X starts
//! ([`crate::shutdown::begin`]: the windows hidden, a second launch of a
//! single-instance app told "closing" so it waits for this process to end, a
//! deadline armed), this executable started again with this command line,
//! detached, and then the primary window closed as `appWindow.close()` closes
//! it ([`crate::window::close_primary`]), so gpui quits through its quit hooks
//! (IndexedDB's checkpoint, the cookie jar, the voice rooms' leave). Embedded
//! (src/embed.rs) the process is the embedder's: there is no Restart, and
//! Close closes the app as an embedder sees any close.
//!
//! **Close**, Esc and Alt+F4 are that same close without the relaunch.
//!
//! On other platforms the dialog is the platform's alert (vn-desktop's
//! question, an `NSAlert` sheet on macOS) with Restart and Close, and the
//! panic's text in its message.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::App;

// ---------------------------------------------------------------------------
// the pure parts
// ---------------------------------------------------------------------------

/// The dialog's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

/// The language for a Windows language id (`GetUserDefaultUILanguage`):
/// Russian for LANG_RUSSIAN (the primary language, its low 10 bits),
/// English for everything else.
pub fn lang_of(langid: u16) -> Lang {
    if langid & 0x3ff == 0x19 { Lang::Ru } else { Lang::En }
}

/// The user's Windows display language; English elsewhere.
pub fn ui_lang() -> Lang {
    #[cfg(target_os = "windows")]
    {
        lang_of(unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() })
    }
    #[cfg(not(target_os = "windows"))]
    {
        Lang::En
    }
}

/// Everything the dialog says. `{app}` is the app's name, `{v}` its version;
/// `&` marks a button's access key.
#[derive(Debug)]
pub struct Strings {
    pub title: &'static str,
    pub body: &'static str,
    /// The body where there is no Restart (embedded).
    pub body_no_restart: &'static str,
    pub details: &'static str,
    pub restart: &'static str,
    pub copy: &'static str,
    pub copied: &'static str,
    pub close: &'static str,
    pub version: &'static str,
}

const EN: Strings = Strings {
    title: "{app} ran into a problem",
    body: "An internal error stopped the app. Restart it to keep going.",
    body_no_restart: "An internal error stopped the app. Close it and open it again to keep going.",
    details: "Details",
    restart: "&Restart",
    copy: "&Copy report",
    copied: "Copied",
    close: "C&lose",
    version: "Version {v}",
};

const RU: Strings = Strings {
    title: "{app} столкнулся с ошибкой",
    body: "Внутренняя ошибка остановила приложение. Перезапустите его, чтобы продолжить работу.",
    body_no_restart: "Внутренняя ошибка остановила приложение. Закройте его и откройте снова, чтобы продолжить работу.",
    details: "Подробности",
    restart: "&Перезапустить",
    copy: "&Скопировать отчёт",
    copied: "Скопировано",
    close: "&Закрыть",
    version: "Версия {v}",
};

pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::En => &EN,
        Lang::Ru => &RU,
    }
}

/// A footer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Restart,
    Copy,
    Close,
}

/// The footer, in order. Embedded there is no Restart: the process is the
/// embedder's to start again.
pub fn actions(embedded: bool) -> &'static [Action] {
    if embedded { &[Action::Copy, Action::Close] } else { &[Action::Restart, Action::Copy, Action::Close] }
}

/// The accent-filled button, focused at first (so the one Enter presses):
/// Restart, or Close where there is none.
pub fn primary(actions: &[Action]) -> usize {
    actions.iter().position(|a| *a == Action::Restart).or_else(|| actions.iter().position(|a| *a == Action::Close)).unwrap_or(0)
}

/// The sentence under the title: it names the way out the footer offers.
pub fn body(s: &Strings, actions: &[Action]) -> &'static str {
    if actions.contains(&Action::Restart) { s.body } else { s.body_no_restart }
}

impl Action {
    pub fn label(self, s: &Strings) -> &'static str {
        match self {
            Action::Restart => s.restart,
            Action::Copy => s.copy,
            Action::Close => s.close,
        }
    }
}

/// What the dialog and the copied report tell.
#[derive(Debug, Clone)]
pub struct Report {
    pub app: String,
    pub version: Option<String>,
    /// `Windows 10.0.26100 (x86_64)`.
    pub os: String,
    pub time: SystemTime,
    /// The panic: `"<message> at <file:line:col>"`.
    pub reason: String,
}

impl Report {
    pub fn title(&self, s: &Strings) -> String {
        s.title.replace("{app}", &self.app)
    }

    /// The line under the panic in Details: the version, then the OS.
    pub fn about(&self, s: &Strings) -> String {
        match &self.version {
            Some(v) => format!("{} · {}", s.version.replace("{v}", v), self.os),
            None => self.os.clone(),
        }
    }

    /// The plain-text report Copy puts on the clipboard. In English whatever
    /// the dialog's language: it is read by whoever fixes the bug.
    pub fn text(&self) -> String {
        let app = match &self.version {
            Some(v) => format!("{} {v}", self.app),
            None => self.app.clone(),
        };
        format!(
            "{} ran into a problem\r\n\
             App: {app}\r\n\
             Host: vue-native {}\r\n\
             OS: {}\r\n\
             Time: {}\r\n\
             Error: native panic: {}\r\n",
            self.app,
            env!("CARGO_PKG_VERSION"),
            self.os,
            utc(self.time),
            self.reason,
        )
    }
}

/// `2026-11-10 14:03:12 UTC`. UTC, so a report read in another time zone
/// says the same moment; no time-zone database needed.
pub fn utc(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The OS and its build, as the report names it.
pub fn os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        use windows::Wdk::System::SystemServices::RtlGetVersion;
        use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
        let mut vi =
            OSVERSIONINFOW { dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
        // `RtlGetVersion`: `GetVersionEx` reports 6.2 to an unmanifested process.
        if unsafe { RtlGetVersion(&mut vi) }.is_ok() {
            return format!(
                "Windows {}.{}.{} ({})",
                vi.dwMajorVersion,
                vi.dwMinorVersion,
                vi.dwBuildNumber,
                std::env::consts::ARCH
            );
        }
        format!("Windows ({})", std::env::consts::ARCH)
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
    }
}

/// The app's version: a packaged app's (`version` in the payload `vn build`
/// stamped); a dev host runs whatever is in `dist/`, which has none.
pub fn app_version() -> Option<String> {
    #[cfg(feature = "prod")]
    {
        crate::payload::get().map(|l| l.meta.version.clone()).filter(|v| !v.is_empty())
    }
    #[cfg(not(feature = "prod"))]
    {
        None
    }
}

/// `text` with a line break in every run of more than `columns` characters
/// that has no space in it. gpui wraps a line at its spaces and punctuation
/// but never inside a word, and a panic's text can hold a long identifier
/// or path that would otherwise run out of the details' well.
pub fn breakable(text: &str, columns: usize) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / columns.max(1));
    let mut run = 0;
    for c in text.chars() {
        if c.is_whitespace() {
            run = 0;
        } else {
            if run == columns {
                out.push('\n');
                run = 0;
            }
            run += 1;
        }
        out.push(c);
    }
    out
}

/// The relaunch's arguments: this process's, without the program name.
pub fn relaunch_args(argv: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    argv.into_iter().skip(1).collect()
}

/// What a poisoned runtime gets, decided once per process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Already decided: nothing more.
    Nothing,
    /// The `[crash]` line only: a test host (src/quiet.rs) is driven by a
    /// runner that reads the log, and a dialog nobody answers would only
    /// keep it alive; a closing app is already going.
    Log,
    /// The line and the dialog.
    Dialog,
}

pub fn decide(first: bool, test_host: bool, closing: bool) -> Decision {
    match (first, test_host || closing) {
        (false, _) => Decision::Nothing,
        (true, true) => Decision::Log,
        (true, false) => Decision::Dialog,
    }
}

// ---------------------------------------------------------------------------
// the host's side
// ---------------------------------------------------------------------------

static APP_NAME: Mutex<Option<String>> = Mutex::new(None);
static DECIDED: AtomicBool = AtomicBool::new(false);

/// The app's display name, for the title: the window title from its config,
/// as the permission prompt has it.
pub(crate) fn set_app_name(name: &str) {
    let name = name.trim();
    *APP_NAME.lock().unwrap_or_else(|p| p.into_inner()) = (!name.is_empty()).then(|| name.to_string());
}

fn app_name() -> String {
    APP_NAME.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_else(|| "vue-native".to_string())
}

/// The runtime is poisoned: called where JS would have run (the frame pump,
/// the background tick), on every such call. The first logs the reason and
/// opens the dialog; the rest do nothing.
pub(crate) fn on_poisoned(cx: &mut App) {
    let first = !DECIDED.swap(true, Ordering::SeqCst);
    let decision = decide(first, crate::quiet::never_focus(), crate::shutdown::is_closing());
    if decision == Decision::Nothing {
        return;
    }
    let report = Report {
        app: app_name(),
        version: app_version(),
        os: os_version(),
        time: SystemTime::now(),
        reason: vn_runtime::bridge::poison_reason().unwrap_or_else(|| "(no message)".to_string()),
    };
    let line = format!("[crash] a native panic stopped the app's JavaScript (I9): {}", report.reason);
    eprintln!("{line}");
    vn_tree::log::record("crash", line);
    if decision == Decision::Dialog {
        // A task, so the window opens outside the pump's update of the
        // primary window.
        cx.spawn(async move |cx| {
            // Restored out of any `App` update: the restore's `WM_SIZE` is
            // sent, and gpui must be free to take the window for it.
            #[cfg(target_os = "windows")]
            view::restore_primary(cx.update(view::primary_hwnd));
            cx.update(|cx| show(report, cx))
        })
        .detach();
    }
}

/// The dialog's answer: the app closes, and with Restart starts again.
pub(crate) fn quit(action: Action, cx: &mut App) {
    let relaunch = action == Action::Restart && !crate::embed::active();
    crate::shutdown::begin(if relaunch { "restart after a crash" } else { "close after a crash" });
    // After `begin`: a single-instance app's new process is told this one is
    // closing, and waits for it to end before it opens its window.
    if relaunch {
        match relaunch_now() {
            Ok(pid) => vn_tree::log::record("crash", format!("[crash] restarting: started process {pid}")),
            Err(e) => {
                let line = format!("[crash] the app could not be started again: {e}");
                eprintln!("{line}");
                vn_tree::log::record("crash", line);
            }
        }
    }
    let main = crate::MAIN_WINDOW.with(|w| w.get());
    let closed = main.is_some_and(|h| {
        h.update(cx, |_, window, cx| crate::window::close_primary("close after a crash", window, cx)).is_ok()
    });
    if !closed {
        cx.quit();
    }
}

/// Starts this executable again with this process's arguments, detached:
/// no console, its own process group, no inherited stdio.
fn relaunch_now() -> Result<u32, String> {
    let exe: PathBuf = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(relaunch_args(std::env::args_os()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let child = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
    let pid = child.id();
    // The new process may take the foreground when its window opens: the user
    // asked for it, through us.
    #[cfg(target_os = "windows")]
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
    Ok(pid)
}

#[cfg(target_os = "windows")]
fn show(report: Report, cx: &mut App) {
    view::show(report, cx);
}

/// Elsewhere: the platform's alert, asked through vn-desktop's question
/// (an `NSAlert` sheet on the primary window on macOS).
#[cfg(not(target_os = "windows"))]
fn show(report: Report, cx: &mut App) {
    let s = strings(ui_lang());
    let acts = actions(crate::embed::active()).iter().copied().filter(|a| *a != Action::Copy).collect::<Vec<_>>();
    if let Some(h) = crate::MAIN_WINDOW.with(|w| w.get()) {
        let _ = h.update(cx, |_, window, _| window.activate_window());
    }
    let ask = vn_desktop::dialog::Ask {
        what: "crash",
        title: report.app.clone(),
        instruction: report.title(s),
        content: format!("{}\n\n{}\n{}", body(s, &acts), report.reason, report.about(s)),
        buttons: acts.iter().map(|a| a.label(s).to_string()).collect(),
        default: primary(&acts),
    };
    let (tx, rx) = futures::channel::oneshot::channel::<Option<usize>>();
    vn_desktop::dialog::ask(ask, Box::new(move |a| {
        let _ = tx.send(a);
    }));
    cx.spawn(async move |cx| {
        // A dismissal leaves the app as it is: its window's own close works
        // (`window::should_close` runs no JS once poisoned).
        if let Ok(Some(i)) = rx.await {
            if let Some(&action) = acts.get(i) {
                cx.update(|cx| quit(action, cx));
            }
        }
    })
    .detach();
}

// ---------------------------------------------------------------------------
// the window (Windows)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub use view::{preview, Preview};

#[cfg(target_os = "windows")]
mod view {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui::{
        div, prelude::*, px, App, AsyncApp, ClickEvent, ClipboardItem, Context, FocusHandle, FontWeight, Image,
        ImageFormat, KeyDownEvent, MouseButton, Role, Subscription, Window, WindowAppearance, WindowHandle,
    };

    use super::{actions, primary, strings, ui_lang, Action, Lang, Report, Strings};
    use crate::content_dialog::{self as card, fonts, hwnd_of, parse_label, win, Accent, Fonts, Guard, Label, Theme};

    /// The card's width, DIP: wider than the prompt's 440 for three buttons,
    /// within ContentDialog's 548.
    const WIDTH: f32 = 500.0;

    /// How long Copy reads "Copied".
    const COPIED_FOR: Duration = Duration::from_secs(2);

    /// Segoe Fluent Icons' Warning: a triangle reads as a fault at a glance,
    /// where ErrorBadge's own circle sat in the tinted circle twice.
    const GLYPH: char = '\u{E7BA}';

    /// The details' well: six lines of 16 DIP, then it scrolls.
    const DETAILS_LINES: f32 = 6.0;

    /// Consolas' 12 px columns that fit the well (426 DIP at 6.6 DIP a
    /// column is 64), less a margin: a longer unbroken run is broken here.
    const DETAILS_COLUMNS: usize = 60;

    /// What the app does with the answer: [`super::quit`], or the preview's.
    type Outcome = Box<dyn FnOnce(Action, &mut App)>;

    struct Spec {
        report: Report,
        lang: Lang,
        actions: &'static [Action],
        /// The primary window's HWND, 0 for none.
        owner: isize,
        /// The theme, when it is not the system's (the preview).
        dark: Option<bool>,
        expanded: bool,
    }

    struct Crash {
        spec: Spec,
        s: &'static Strings,
        labels: Vec<Label>,
        /// The buttons', then the expander's.
        focus: Vec<FocusHandle>,
        expander: FocusHandle,
        expanded: bool,
        /// Copy's label reads "Copied" while this is the latest copy.
        copied: Option<u64>,
        copies: u64,
        guard: Guard,
        accent: Accent,
        acrylic: bool,
        icon: Option<Arc<Image>>,
        outcome: Option<Outcome>,
        hwnd: isize,
        owner_disabled: bool,
        height: Rc<Cell<f32>>,
        pressing: Option<usize>,
        _subs: Vec<Subscription>,
    }

    impl Crash {
        fn dark(&self, window: &Window) -> bool {
            self.spec.dark.unwrap_or(matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark))
        }

        fn release_owner(&mut self) {
            if std::mem::take(&mut self.owner_disabled) {
                card::release_owner(self.spec.owner, self.hwnd);
            }
        }

        /// The answer, once: the owner back, the window gone, then the app
        /// told (after this update, so its close runs outside this window's).
        fn finish(&mut self, action: Action, window: &mut Window, cx: &mut App) {
            let Some(outcome) = self.outcome.take() else { return };
            self.release_owner();
            window.remove_window();
            cx.defer(move |cx| outcome(action, cx));
        }

        fn press(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
            if i >= self.labels.len() || !self.guard.allows(Instant::now()) {
                return;
            }
            match self.spec.actions[i] {
                Action::Copy => self.copy(cx),
                a => self.finish(a, window, cx),
            }
        }

        fn copy(&mut self, cx: &mut Context<Self>) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.spec.report.text()));
            self.copies += 1;
            let n = self.copies;
            self.copied = Some(n);
            cx.notify();
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                cx.background_executor().timer(COPIED_FOR).await;
                let _ = this.update(cx, |p, cx| {
                    if p.copied == Some(n) {
                        p.copied = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }

        fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            self.expanded = !self.expanded;
            cx.notify();
            let (handle, hwnd, height) = (window.window_handle(), self.hwnd, self.height.clone());
            cx.spawn(async move |_, cx: &mut AsyncApp| card::refit(handle, hwnd, WIDTH, &height, cx).await).detach();
        }

        /// The focusables in Tab order: the expander, then the buttons.
        fn ring_order(&self) -> Vec<FocusHandle> {
            std::iter::once(self.expander.clone()).chain(self.focus.iter().cloned()).collect()
        }

        fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
            let k = &e.keystroke;
            let m = k.modifiers;
            let plain = !m.alt && !m.control && !m.platform;
            let default = primary(self.spec.actions);
            let button = self.focus.iter().position(|f| f.is_focused(window));
            match k.key.as_str() {
                "escape" if plain => self.finish(Action::Close, window, cx),
                "tab" if plain => {
                    let order = self.ring_order();
                    let at = order.iter().position(|f| f.is_focused(window)).unwrap_or(default + 1);
                    let n = order.len();
                    let next = if m.shift { (at + n - 1) % n } else { (at + 1) % n };
                    window.focus(&order[next], cx);
                }
                "left" | "right" | "up" | "down" if plain => {
                    let n = self.focus.len();
                    let at = button.unwrap_or(default);
                    let back = matches!(k.key.as_str(), "left" | "up");
                    window.focus(&self.focus[if back { (at + n - 1) % n } else { (at + 1) % n }], cx);
                }
                "enter" | "space" if plain && !m.shift && !e.is_held => {
                    if self.expander.is_focused(window) {
                        self.toggle(window, cx);
                    } else {
                        self.press(button.unwrap_or(default), window, cx);
                    }
                }
                _ if m.alt && !m.control && !m.platform && !e.is_held => match card::access_key_hit(&self.labels, k) {
                    Some(i) => {
                        window.focus(&self.focus[i], cx);
                        self.press(i, window, cx);
                    }
                    None => return,
                },
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
        }

        fn button(&self, i: usize, t: &Theme, alt: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
            let action = self.spec.actions[i];
            let copied = action == Action::Copy && self.copied.is_some();
            let label = if copied { parse_label(self.s.copied) } else { self.labels[i].clone() };
            let accent = i == primary(self.spec.actions);
            card::button(("vn-crash-button", i), &label, accent, t, alt, &self.focus[i], window, |b| {
                b.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, _| {
                        this.pressing = this.guard.allows(Instant::now()).then_some(i);
                    }),
                )
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if matches!(e, ClickEvent::Mouse(_)) && this.pressing.take() == Some(i) {
                        this.press(i, window, cx);
                    }
                }))
            })
        }

        /// The Details expander: a disclosure row, and under it, open, the
        /// panic in a monospace well and the version and OS.
        fn details(&self, t: &Theme, f: &Fonts, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
            let ring = window.last_input_was_keyboard() && self.expander.is_focused(window);
            // ChevronDown / ChevronUp.
            let chevron = if self.expanded { '\u{E70E}' } else { '\u{E70D}' };
            let hover = t.subtle;
            let toggle = div()
                .id("vn-crash-details")
                .track_focus(&self.expander)
                .role(Role::Button)
                .aria_label(self.s.details)
                .aria_expanded(self.expanded)
                .h(px(28.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded(px(4.0))
                .text_size(px(14.0))
                .line_height(px(20.0))
                .hover(move |s| s.bg(hover))
                .child(self.s.details)
                .child(
                    div()
                        .font_family(f.icons)
                        .text_size(px(12.0))
                        .line_height(px(12.0))
                        .text_color(t.text2)
                        .child(chevron.to_string()),
                )
                .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                    if matches!(e, ClickEvent::Mouse(_)) {
                        this.toggle(window, cx);
                    }
                }));
            div()
                .mt(px(12.0))
                .flex()
                .flex_col()
                // A row, so the toggle (its hover, its focus ring) is as wide
                // as its words, and they line up with the body's: its hover
                // runs past them.
                .child(div().flex().ml(px(-8.0)).child(card::with_ring(div().relative().child(toggle), ring, t, 4.0)))
                .when(self.expanded, |d| {
                    d.child(
                        div()
                            .mt(px(8.0))
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .id("vn-crash-reason")
                                    .role(Role::Document)
                                    .aria_label(self.spec.report.reason.clone())
                                    .max_h(px(16.0 * DETAILS_LINES + 16.0))
                                    .overflow_y_scroll()
                                    .px(px(12.0))
                                    .py(px(8.0))
                                    .rounded(px(4.0))
                                    .border_1()
                                    .border_color(t.button_border)
                                    .bg(t.well)
                                    .font_family(f.mono)
                                    .text_size(px(12.0))
                                    .line_height(px(16.0))
                                    .child(super::breakable(&self.spec.report.reason, DETAILS_COLUMNS)),
                            )
                            .child(
                                div()
                                    .font_family(f.small)
                                    .text_size(px(12.0))
                                    .line_height(px(16.0))
                                    .text_color(t.text2)
                                    .child(self.spec.report.about(self.s)),
                            ),
                    )
                })
        }
    }

    impl Render for Crash {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let dark = self.dark(window);
            let t = Theme::new(dark, self.acrylic, self.accent);
            let f = fonts(cx);
            let alt = window.modifiers().alt;
            let buttons: Vec<_> = (0..self.labels.len()).map(|i| self.button(i, &t, alt, window, cx)).collect();
            let title = self.spec.report.title(self.s);
            let sentence = super::body(self.s, self.spec.actions);
            let body = card::body("vn-crash-body", &self.spec.report.app, card::app_icon(&self.icon, &t, f), &t, f)
                .child(card::glyph_badge(GLYPH, t.critical_tint, t.critical, f))
                .child(card::title(title.clone(), f))
                .child(card::content(sentence.to_string(), &t))
                .child(self.details(&t, f, window, cx));
            div()
                .id("vn-crash")
                .role(Role::Dialog)
                .aria_label(title)
                .aria_description(sentence)
                .key_context("VnCrash")
                .on_key_down(cx.listener(Self::key_down))
                .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
                .size_full()
                .flex()
                .flex_col()
                .font_family(f.text)
                .font_weight(FontWeight::NORMAL)
                .text_color(t.text)
                .bg(t.footer)
                .child(card::card(self.height.clone(), body, card::footer(&t, buttons)))
        }
    }

    impl Drop for Crash {
        fn drop(&mut self) {
            // Gone without an answer (the app quitting some other way): the
            // primary window is usable again, and its own close works.
            self.release_owner();
        }
    }

    /// The primary window's HWND, 0 for none.
    pub(super) fn primary_hwnd(cx: &mut App) -> isize {
        crate::MAIN_WINDOW
            .with(|w| w.get())
            .and_then(|h| h.update(cx, |_, window, _| hwnd_of(window)).ok().flatten())
            .unwrap_or(0)
    }

    /// Brings a minimised or hidden primary window back before the dialog
    /// opens over it: an owned window shows only with its owner. Out of any
    /// `App` update.
    pub(super) fn restore_primary(hwnd: isize) {
        win::restore(hwnd);
    }

    /// The dialog over the primary window, which it disables while it is up.
    pub(super) fn show(report: Report, cx: &mut App) {
        let owner = primary_hwnd(cx);
        let spec = Spec {
            report,
            lang: ui_lang(),
            actions: actions(crate::embed::active()),
            owner,
            dark: None,
            expanded: false,
        };
        if let Err(e) = open(spec, Box::new(super::quit), cx) {
            let line = format!("[crash] the crash dialog could not open: {e:#}");
            eprintln!("{line}");
            vn_tree::log::record("crash", line);
        }
    }

    /// Opens the dialog, hidden, and reveals it once it has its size and place.
    fn open(spec: Spec, outcome: Outcome, cx: &mut App) -> anyhow::Result<WindowHandle<Crash>> {
        let owner = win::usable(spec.owner).then_some(spec.owner).unwrap_or(0);
        let spec = Spec { owner, ..spec };
        let display = card::display_of(owner, cx);
        let acrylic = card::acrylic();
        let accent = card::accent();
        let height = Rc::new(Cell::new(0.0));
        let options = card::window_options(spec.report.app.clone(), WIDTH, display, acrylic, cx);
        let handle = cx.open_window(options, {
            let height = height.clone();
            move |window, cx| {
                cx.new(|cx| {
                    let s = strings(spec.lang);
                    let labels: Vec<Label> = spec.actions.iter().map(|a| parse_label(a.label(s))).collect();
                    // Tab order: the expander, then the buttons (`key_down`).
                    let expander = cx.focus_handle().tab_index(0).tab_stop(true);
                    let focus: Vec<FocusHandle> = (0..labels.len())
                        .map(|i| cx.focus_handle().tab_index(i as isize + 1).tab_stop(true))
                        .collect();
                    window.focus(&focus[primary(spec.actions)], cx);
                    let hwnd = hwnd_of(window).unwrap_or(0);
                    let icon = win::icon_png(owner, hwnd, (16.0 * window.scale_factor()).round() as i32)
                        .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                    let subs = vec![
                        cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
                        cx.observe_window_activation(window, |p: &mut Crash, window, cx| {
                            if window.is_window_active() {
                                p.guard.arm(Instant::now());
                            } else {
                                p.guard.disarm();
                            }
                            cx.notify();
                        }),
                    ];
                    window.on_window_should_close(cx, |window, cx| {
                        // Alt+F4, or the system menu's Close: Close.
                        if let Some(view) = window.root::<Crash>().flatten() {
                            let outcome = view.update(cx, |p, _| {
                                p.release_owner();
                                p.outcome.take()
                            });
                            if let Some(outcome) = outcome {
                                cx.defer(move |cx| outcome(Action::Close, cx));
                            }
                        }
                        true
                    });
                    Crash {
                        expanded: spec.expanded,
                        spec,
                        s,
                        labels,
                        focus,
                        expander,
                        copied: None,
                        copies: 0,
                        guard: Guard::default(),
                        accent,
                        acrylic,
                        icon,
                        outcome: Some(outcome),
                        hwnd,
                        owner_disabled: false,
                        height,
                        pressing: None,
                        _subs: subs,
                    }
                })
            }
        })?;
        let hwnd = handle.update(cx, |p, _, _| p.hwnd)?;
        if hwnd == 0 {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            anyhow::bail!("the crash dialog has no HWND");
        }
        card::prepare(hwnd, owner, handle.update(cx, |p, _, _| p.spec.dark)?);
        cx.spawn(async move |cx: &mut AsyncApp| reveal(handle, hwnd, owner, height, cx).await).detach();
        Ok(handle)
    }

    async fn reveal(handle: WindowHandle<Crash>, hwnd: isize, owner: isize, height: Rc<Cell<f32>>, cx: &mut AsyncApp) {
        card::fit(handle.into(), hwnd, owner, WIDTH, &height, cx).await;
        let Ok(live) = handle.update(cx, |p, _, _| p.outcome.is_some()) else { return };
        if !live {
            return;
        }
        if card::disable_owner(owner) {
            let _ = handle.update(cx, |p, _, _| p.owner_disabled = true);
        }
        // A test host never gets here (`decide`); the preview may ask not to
        // take the foreground the same way.
        let activate = !crate::quiet::never_focus();
        crate::popout_win::show(hwnd, activate);
        let _ = handle.update(cx, |p, window, cx| {
            if window.is_window_active() || crate::popout_win::is_foreground(hwnd) {
                p.guard.arm(Instant::now());
            }
            cx.notify();
        });
    }

    // -----------------------------------------------------------------------
    // the preview
    // -----------------------------------------------------------------------

    /// What the preview shows (examples/crash_dialog.rs).
    #[doc(hidden)]
    #[derive(Debug, Clone)]
    pub struct Preview {
        pub russian: bool,
        pub dark: Option<bool>,
        pub app: String,
        pub version: Option<String>,
        pub reason: String,
        /// Over a window of its own, rather than with no owner.
        pub owner: bool,
        /// As an embedded vue-native shows it: no Restart.
        pub embedded: bool,
        /// Details open from the start.
        pub expanded: bool,
        pub accent: Option<Accent>,
        /// An exe whose icon stands in for the app's.
        pub icon_from: Option<String>,
        /// The card without acrylic, as Windows 10 draws it.
        pub solid: bool,
    }

    /// `cargo run -p vn-app --example crash_dialog`: the dialog on its own,
    /// for a look at it. Nothing has crashed; the buttons print what they
    /// would do, Copy really copies, and the preview ends with the dialog.
    #[doc(hidden)]
    pub fn preview(p: Preview) {
        if let Some(a) = p.accent {
            let _ = card::PREVIEW_ACCENT.set(a);
        }
        card::PREVIEW_SOLID.store(p.solid, std::sync::atomic::Ordering::Relaxed);
        if let Some(exe) = p.icon_from.clone() {
            let _ = card::PREVIEW_ICON.set(exe);
        }
        gpui_platform::application().run(move |cx: &mut App| {
            let owner = if p.owner { card::preview_owner(&p.app, cx) } else { 0 };
            let spec = Spec {
                report: Report {
                    app: p.app.clone(),
                    version: p.version.clone(),
                    os: super::os_version(),
                    time: std::time::SystemTime::now(),
                    reason: p.reason.clone(),
                },
                lang: if p.russian { Lang::Ru } else { Lang::En },
                actions: actions(p.embedded),
                owner,
                dark: p.dark,
                expanded: p.expanded,
            };
            let outcome: Outcome = Box::new(|a, cx| {
                println!("answer: {a:?}");
                cx.quit();
            });
            if let Err(e) = open(spec, outcome, cx) {
                eprintln!("the crash dialog could not open: {e:#}");
                cx.quit();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn report(version: Option<&str>) -> Report {
        Report {
            app: "Argon".into(),
            version: version.map(String::from),
            os: "Windows 10.0.26100 (x86_64)".into(),
            time: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            reason: "already borrowed: BorrowMutError at crates/vn-tree/src/lib.rs:120:9".into(),
        }
    }

    #[test]
    fn russian_is_chosen_by_the_primary_language_only() {
        assert_eq!(lang_of(0x0419), Lang::Ru, "ru-RU");
        assert_eq!(lang_of(0x0819), Lang::Ru, "ru-MD: another sublanguage, still Russian");
        assert_eq!(lang_of(0x0409), Lang::En, "en-US");
        assert_eq!(lang_of(0x0422), Lang::En, "uk-UA is not Russian");
        assert_eq!(lang_of(0x0000), Lang::En);
        assert_eq!(strings(Lang::Ru).details, "Подробности");
        assert_eq!(strings(Lang::En).details, "Details");
    }

    #[test]
    fn the_title_names_the_app_in_either_language() {
        let r = report(None);
        assert_eq!(r.title(strings(Lang::En)), "Argon ran into a problem");
        assert_eq!(r.title(strings(Lang::Ru)), "Argon столкнулся с ошибкой");
    }

    #[test]
    fn every_button_has_its_own_access_key() {
        for lang in [Lang::En, Lang::Ru] {
            let s = strings(lang);
            // The character after the first lone `&`, as `parse_label` reads it.
            let key_of = |l: &str| {
                let at = l.find('&')?;
                l[at + 1..].chars().next().and_then(|c| c.to_lowercase().next())
            };
            let keys: Vec<char> = actions(false).iter().map(|a| key_of(a.label(s)).expect("an access key")).collect();
            let mut unique = keys.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), keys.len(), "{lang:?}: {keys:?}");
        }
    }

    #[test]
    fn embedded_there_is_no_restart_and_close_is_the_default() {
        assert_eq!(body(strings(Lang::En), actions(true)), strings(Lang::En).body_no_restart, "it does not say Restart");
        assert_eq!(body(strings(Lang::Ru), actions(false)), strings(Lang::Ru).body);
        assert_eq!(actions(false), &[Action::Restart, Action::Copy, Action::Close]);
        assert_eq!(primary(actions(false)), 0);
        assert_eq!(actions(true), &[Action::Copy, Action::Close]);
        assert_eq!(primary(actions(true)), 1, "Close, not Copy, is what Enter does embedded");
    }

    #[test]
    fn the_report_says_what_whoever_fixes_it_needs() {
        let text = report(Some("1.4.2")).text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Argon ran into a problem");
        assert_eq!(lines[1], "App: Argon 1.4.2");
        assert_eq!(lines[2], concat!("Host: vue-native ", env!("CARGO_PKG_VERSION")));
        assert_eq!(lines[3], "OS: Windows 10.0.26100 (x86_64)");
        assert_eq!(lines[4], "Time: 2023-11-14 22:13:20 UTC");
        assert_eq!(lines[5], "Error: native panic: already borrowed: BorrowMutError at crates/vn-tree/src/lib.rs:120:9");
        assert!(text.ends_with("\r\n"), "Windows line ends, for Notepad and the clipboard");
        assert_eq!(report(None).text().lines().nth(1), Some("App: Argon"), "no version, no dangling space");
    }

    #[test]
    fn details_name_the_version_and_the_os() {
        assert_eq!(report(Some("1.4.2")).about(strings(Lang::En)), "Version 1.4.2 · Windows 10.0.26100 (x86_64)");
        assert_eq!(report(Some("1.4.2")).about(strings(Lang::Ru)), "Версия 1.4.2 · Windows 10.0.26100 (x86_64)");
        assert_eq!(report(None).about(strings(Lang::En)), "Windows 10.0.26100 (x86_64)");
    }

    #[test]
    fn utc_is_the_civil_date() {
        assert_eq!(utc(UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00:00 UTC", "a leap day");
        assert_eq!(utc(UNIX_EPOCH + Duration::from_secs(1_798_761_599)), "2026-12-31 23:59:59 UTC");
    }

    #[test]
    fn the_dialog_opens_once_and_never_in_a_test_host() {
        assert_eq!(decide(true, false, false), Decision::Dialog);
        assert_eq!(decide(false, false, false), Decision::Nothing, "the pump and the tick both ask; one dialog");
        assert_eq!(decide(true, true, false), Decision::Log, "a test host logs and carries on");
        assert_eq!(decide(true, false, true), Decision::Log, "a closing app is already going");
        assert_eq!(decide(false, true, false), Decision::Nothing);
    }

    #[test]
    fn a_long_unbroken_run_is_broken_and_ordinary_text_is_not() {
        let ordinary = "already borrowed: BorrowMutError at crates/vn-tree/src/lib.rs:120:9";
        assert_eq!(breakable(ordinary, 60), ordinary);
        assert_eq!(breakable("abcdefgh", 3), "abc\ndef\ngh");
        assert_eq!(breakable("abc def", 3), "abc def", "a run of exactly `columns` stays whole");
        assert_eq!(breakable("ab\ncdef", 3), "ab\ncde\nf", "a line break ends a run");
        assert_eq!(breakable("ёжикжук", 3), "ёжи\nкжу\nк", "by characters, not bytes");
    }

    #[test]
    fn a_relaunch_has_the_arguments_without_the_program() {
        let argv = ["C:\\Apps\\Argon.exe", "--profile", "work space", "argon://invite/x"].map(OsString::from);
        assert_eq!(relaunch_args(argv), ["--profile", "work space", "argon://invite/x"].map(OsString::from));
        assert!(relaunch_args([OsString::from("argon")]).is_empty());
        assert!(relaunch_args(std::iter::empty()).is_empty());
    }
}
