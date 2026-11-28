//! The crash dialog: what the user sees once a native panic has poisoned the runtime, drawn by the
//! host with no JavaScript.
//!
//! A poisoned runtime runs no JS again (the frame pump and background tick return before their
//! wave), so the buttons of an app's own error screen, being JS, do nothing. The only way out is a
//! new process, so the dialog offers that without needing the runtime. It is a Windows 11
//! ContentDialog like the permission prompt (src/content_dialog.rs): a collapsed Details expander
//! (panic message and location, app version, OS build) and the buttons Restart, Copy report and
//! Close, in the language `sk-locale` chooses.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::App;

pub fn ui_lang() -> String {
    sk_locale::language()
}

#[derive(Debug)]
pub struct Strings {
    pub title: &'static str,
    pub body: &'static str,
    pub body_no_restart: &'static str,
    pub details: &'static str,
    pub restart: &'static str,
    pub copy: &'static str,
    pub copied: &'static str,
    pub close: &'static str,
    pub version: &'static str,
}

/// Leaked: the dialog opens once, as the process ends.
pub fn strings(lang: &str) -> &'static Strings {
    use sk_locale::keys;
    let t = |key: &str| -> &'static str {
        Box::leak(
            sk_locale::text_in(lang, key)
                .unwrap_or_else(|| key.to_string())
                .into_boxed_str(),
        )
    };
    Box::leak(Box::new(Strings {
        title: t(keys::CRASH_TITLE),
        body: t(keys::CRASH_BODY),
        body_no_restart: t(keys::CRASH_BODY_NO_RESTART),
        details: t(keys::CRASH_DETAILS),
        restart: t(keys::CRASH_RESTART),
        copy: t(keys::CRASH_COPY),
        copied: t(keys::CRASH_COPIED),
        close: t(keys::CRASH_CLOSE),
        version: t(keys::CRASH_VERSION),
    }))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Restart,
    Copy,
    Close,
}

/// The footer, in order. Embedded there is no Restart: the process is the
/// embedder's to start again.
pub fn actions(embedded: bool) -> &'static [Action] {
    if embedded {
        &[Action::Copy, Action::Close]
    } else {
        &[Action::Restart, Action::Copy, Action::Close]
    }
}

pub fn primary(actions: &[Action]) -> usize {
    actions
        .iter()
        .position(|a| *a == Action::Restart)
        .or_else(|| actions.iter().position(|a| *a == Action::Close))
        .unwrap_or(0)
}

pub fn body(s: &Strings, actions: &[Action]) -> &'static str {
    if actions.contains(&Action::Restart) {
        s.body
    } else {
        s.body_no_restart
    }
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

#[derive(Debug, Clone)]
pub struct Report {
    pub app: String,
    pub version: Option<String>,
    pub os: String,
    pub time: SystemTime,
    pub what: &'static str,
    pub reason: String,
}

impl Report {
    pub fn title(&self, s: &Strings) -> String {
        sk_locale::format(s.title, &[("app", &self.app)])
    }

    pub fn about(&self, s: &Strings) -> String {
        match &self.version {
            Some(v) => format!(
                "{} · {}",
                sk_locale::format(s.version, &[("version", v)]),
                self.os
            ),
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
             Host: Skathi {}\r\n\
             OS: {}\r\n\
             Time: {}\r\n\
             Error: {}: {}\r\n",
            self.app,
            env!("CARGO_PKG_VERSION"),
            self.os,
            utc(self.time),
            self.what,
            self.reason,
        )
    }
}

pub fn utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
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
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
        use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
        let mut vi = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        // `RtlGetVersion`: `GetVersionEx` reports 6.2 to an unmanifested process.
        if unsafe { RtlGetVersion(&mut vi) } >= 0 {
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

pub fn app_version() -> Option<String> {
    #[cfg(feature = "prod")]
    {
        crate::payload::get()
            .map(|l| l.meta.version.clone())
            .filter(|v| !v.is_empty())
    }
    #[cfg(not(feature = "prod"))]
    {
        None
    }
}

/// `text` with a line break in every run of more than `columns` characters with no space: gpui wraps
/// at spaces and punctuation but never inside a word, so a long identifier or path would overflow
/// the details' well.
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

pub fn relaunch_args(argv: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    argv.into_iter().skip(1).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Nothing,
    /// The `[crash]` line only: a test host (src/quiet.rs) is driven by a runner that reads the log,
    /// and a closing app is already going.
    Log,
    Dialog,
}

pub fn decide(first: bool, test_host: bool, closing: bool) -> Decision {
    match (first, test_host || closing) {
        (false, _) => Decision::Nothing,
        (true, true) => Decision::Log,
        (true, false) => Decision::Dialog,
    }
}

static APP_NAME: Mutex<Option<String>> = Mutex::new(None);
static DECIDED: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_app_name(name: &str) {
    let name = name.trim();
    *APP_NAME.lock().unwrap_or_else(|p| p.into_inner()) =
        (!name.is_empty()).then(|| name.to_string());
}

fn app_name() -> String {
    APP_NAME
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(|| "Skathi".to_string())
}

static ENTRY_ERROR: Mutex<Option<String>> = Mutex::new(None);
static ENTRY_FAILED: AtomicBool = AtomicBool::new(false);
static ENTRY_THREW: Mutex<Option<String>> = Mutex::new(None);

/// The app could not start (its entry threw, or its runtime does not fit this host): stderr, the
/// log ring and devtools `ping`.
pub(crate) fn entry_threw(error: &str) {
    let line = format!("[entry] the app could not start: {error}");
    eprintln!("{line}");
    sk_tree::log::record("error", line);
    *ENTRY_THREW.lock().unwrap_or_else(|p| p.into_inner()) = Some(error.to_string());
}

/// A dev boot or cold restart evaluates the entry again.
#[cfg(not(feature = "prod"))]
pub(crate) fn entry_booting() {
    *ENTRY_THREW.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Why the app could not start, if it could not.
#[cfg(sk_devtools)]
pub(crate) fn entry_error() -> Option<String> {
    ENTRY_THREW
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// The app's entry threw before it put anything in the window: the user is looking at an empty one.
#[cfg(feature = "prod")]
pub(crate) fn entry_failed(error: String) {
    *ENTRY_ERROR.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
    ENTRY_FAILED.store(true, Ordering::SeqCst);
}

/// The app cannot start: the process ends with exit code 1 (an embedder hears an unclean exit), after
/// the dialog when one is shown, at once when nobody is there to read it.
pub(crate) fn on_entry_failed(cx: &mut App) {
    if !ENTRY_FAILED.swap(false, Ordering::SeqCst) {
        return;
    }
    let Some(reason) = ENTRY_ERROR.lock().unwrap_or_else(|p| p.into_inner()).take() else {
        return;
    };
    let first = !DECIDED.swap(true, Ordering::SeqCst);
    let decision = decide(
        first,
        crate::quiet::never_focus(),
        crate::shutdown::is_closing(),
    );
    sk_tree::log::record(
        "crash",
        format!("[crash] the app's entry threw and mounted nothing: {reason}"),
    );
    crate::shutdown::failed(format!("the app could not start: {reason}"));
    if decision == Decision::Log {
        cx.defer(|cx| quit(Action::Close, cx));
    }
    if decision != Decision::Dialog {
        return;
    }
    let report = Report {
        app: app_name(),
        version: app_version(),
        os: os_version(),
        time: SystemTime::now(),
        what: "the app's entry threw",
        reason,
    };
    cx.spawn(async move |cx| {
        #[cfg(target_os = "windows")]
        view::restore_primary(cx.update(view::primary_hwnd));
        cx.update(|cx| show(report, cx))
    })
    .detach();
}

/// Called where JS would have run; only the first call logs and opens the dialog.
pub(crate) fn on_poisoned(cx: &mut App) {
    let first = !DECIDED.swap(true, Ordering::SeqCst);
    let decision = decide(
        first,
        crate::quiet::never_focus(),
        crate::shutdown::is_closing(),
    );
    if decision == Decision::Nothing {
        return;
    }
    let report = Report {
        app: app_name(),
        version: app_version(),
        os: os_version(),
        time: SystemTime::now(),
        what: "native panic",
        reason: sk_runtime::bridge::poison_reason().unwrap_or_else(|| "(no message)".to_string()),
    };
    let line = format!(
        "[crash] a native panic stopped the app's JavaScript (I9): {}",
        report.reason
    );
    eprintln!("{line}");
    sk_tree::log::record("crash", line);
    if decision == Decision::Dialog {
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

pub(crate) fn quit(action: Action, cx: &mut App) {
    let relaunch = action == Action::Restart && !crate::embed::active();
    crate::shutdown::begin(if relaunch {
        "restart after a crash"
    } else {
        "close after a crash"
    });
    // After `begin`: a single-instance app's new process is told this one is
    // closing, and waits for it to end before it opens its window.
    if relaunch {
        match self::relaunch(relaunch_args(std::env::args_os())) {
            Ok(pid) => sk_tree::log::record(
                "crash",
                format!("[crash] restarting: started process {pid}"),
            ),
            Err(e) => {
                let line = format!("[crash] the app could not be started again: {e}");
                eprintln!("{line}");
                sk_tree::log::record("crash", line);
            }
        }
    }
    let main = crate::MAIN_WINDOW.with(|w| w.get());
    let closed = main.is_some_and(|h| {
        h.update(cx, |_, window, cx| {
            crate::window::close_primary("close after a crash", window, cx)
        })
        .is_ok()
    });
    if !closed {
        cx.quit();
    }
}

/// Starts this program again with `args`, detached; the pid of the new process.
pub(crate) fn relaunch(args: Vec<OsString>) -> Result<u32, String> {
    // An AppImage's own executable lives in a mount that goes away with this process.
    let appimage = cfg!(target_os = "linux")
        .then(|| std::env::var_os("APPIMAGE"))
        .flatten()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let exe: PathBuf = match appimage {
        Some(path) => path,
        None => std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?,
    };
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(args)
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

#[cfg(not(target_os = "windows"))]
fn show(report: Report, cx: &mut App) {
    let s = strings(&ui_lang());
    let acts = actions(crate::embed::active())
        .iter()
        .copied()
        .filter(|a| *a != Action::Copy)
        .collect::<Vec<_>>();
    if let Some(h) = crate::MAIN_WINDOW.with(|w| w.get()) {
        let _ = h.update(cx, |_, window, _| window.activate_window());
    }
    let ask = sk_desktop::dialog::Ask {
        what: "crash",
        title: report.app.clone(),
        instruction: report.title(s),
        content: format!(
            "{}\n\n{}\n{}",
            body(s, &acts),
            report.reason,
            report.about(s)
        ),
        buttons: acts.iter().map(|a| a.label(s).to_string()).collect(),
        default: primary(&acts),
    };
    let (tx, rx) = futures::channel::oneshot::channel::<Option<usize>>();
    sk_desktop::dialog::ask(
        ask,
        Box::new(move |a| {
            let _ = tx.send(a);
        }),
    );
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

#[cfg(target_os = "windows")]
pub use view::{preview, Preview};

#[cfg(target_os = "windows")]
mod view {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui::{
        div, prelude::*, px, App, AsyncApp, ClickEvent, ClipboardItem, Context, FocusHandle,
        FontWeight, Image, ImageFormat, KeyDownEvent, MouseButton, Role, Subscription, Window,
        WindowAppearance, WindowHandle,
    };

    use super::{actions, primary, strings, ui_lang, Action, Report, Strings};
    use crate::content_dialog::{
        self as card, fonts, hwnd_of, parse_label, win, Accent, Fonts, Guard, Label, Theme,
    };

    const WIDTH: f32 = 500.0;

    const COPIED_FOR: Duration = Duration::from_secs(2);

    const GLYPH: char = '\u{E7BA}';

    const DETAILS_LINES: f32 = 6.0;

    const DETAILS_COLUMNS: usize = 60;

    type Outcome = Box<dyn FnOnce(Action, &mut App)>;

    struct Spec {
        report: Report,
        lang: String,
        actions: &'static [Action],
        owner: isize,
        dark: Option<bool>,
        expanded: bool,
    }

    struct Crash {
        spec: Spec,
        s: &'static Strings,
        labels: Vec<Label>,
        focus: Vec<FocusHandle>,
        expander: FocusHandle,
        expanded: bool,
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
            self.spec.dark.unwrap_or(matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ))
        }

        fn release_owner(&mut self) {
            if std::mem::take(&mut self.owner_disabled) {
                card::release_owner(self.spec.owner, self.hwnd);
            }
        }

        /// The answer, once: the owner back, the window gone, then the app
        /// told (after this update, so its close runs outside this window's).
        fn finish(&mut self, action: Action, window: &mut Window, cx: &mut App) {
            let Some(outcome) = self.outcome.take() else {
                return;
            };
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
            cx.spawn(async move |_, cx: &mut AsyncApp| {
                card::refit(handle, hwnd, WIDTH, &height, cx).await
            })
            .detach();
        }

        fn ring_order(&self) -> Vec<FocusHandle> {
            std::iter::once(self.expander.clone())
                .chain(self.focus.iter().cloned())
                .collect()
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
                    let at = order
                        .iter()
                        .position(|f| f.is_focused(window))
                        .unwrap_or(default + 1);
                    let n = order.len();
                    let next = if m.shift {
                        (at + n - 1) % n
                    } else {
                        (at + 1) % n
                    };
                    window.focus(&order[next], cx);
                }
                "left" | "right" | "up" | "down" if plain => {
                    let n = self.focus.len();
                    let at = button.unwrap_or(default);
                    let back = matches!(k.key.as_str(), "left" | "up");
                    window.focus(
                        &self.focus[if back { (at + n - 1) % n } else { (at + 1) % n }],
                        cx,
                    );
                }
                "enter" | "space" if plain && !m.shift && !e.is_held => {
                    if self.expander.is_focused(window) {
                        self.toggle(window, cx);
                    } else {
                        self.press(button.unwrap_or(default), window, cx);
                    }
                }
                _ if m.alt && !m.control && !m.platform && !e.is_held => {
                    match card::access_key_hit(&self.labels, k) {
                        Some(i) => {
                            window.focus(&self.focus[i], cx);
                            self.press(i, window, cx);
                        }
                        None => return,
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
        }

        fn button(
            &self,
            i: usize,
            t: &Theme,
            alt: bool,
            window: &Window,
            cx: &mut Context<Self>,
        ) -> impl IntoElement + use<> {
            let action = self.spec.actions[i];
            let copied = action == Action::Copy && self.copied.is_some();
            let label = if copied {
                parse_label(self.s.copied)
            } else {
                self.labels[i].clone()
            };
            let accent = i == primary(self.spec.actions);
            card::button(
                ("sk-crash-button", i),
                &label,
                accent,
                t,
                alt,
                &self.focus[i],
                window,
                |b| {
                    b.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, _| {
                            this.pressing = this.guard.allows(Instant::now()).then_some(i);
                        }),
                    )
                    .on_click(cx.listener(
                        move |this, e: &ClickEvent, window, cx| {
                            if matches!(e, ClickEvent::Mouse(_)) && this.pressing.take() == Some(i)
                            {
                                this.press(i, window, cx);
                            }
                        },
                    ))
                },
            )
        }

        fn details(
            &self,
            t: &Theme,
            f: &Fonts,
            window: &Window,
            cx: &mut Context<Self>,
        ) -> impl IntoElement + use<> {
            let ring = window.last_input_was_keyboard() && self.expander.is_focused(window);
            let chevron = if self.expanded {
                '\u{E70E}'
            } else {
                '\u{E70D}'
            };
            let hover = t.subtle;
            let toggle = div()
                .id("sk-crash-details")
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
                .child(div().flex().ml(px(-8.0)).child(card::with_ring(
                    div().relative().child(toggle),
                    ring,
                    t,
                    4.0,
                )))
                .when(self.expanded, |d| {
                    d.child(
                        div()
                            .mt(px(8.0))
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .id("sk-crash-reason")
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
                                    .child(super::breakable(
                                        &self.spec.report.reason,
                                        DETAILS_COLUMNS,
                                    )),
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
            let buttons: Vec<_> = (0..self.labels.len())
                .map(|i| self.button(i, &t, alt, window, cx))
                .collect();
            let title = self.spec.report.title(self.s);
            let sentence = super::body(self.s, self.spec.actions);
            let body = card::body(
                "sk-crash-body",
                &self.spec.report.app,
                card::app_icon(&self.icon, &t, f),
                &t,
                f,
            )
            .child(card::glyph_badge(GLYPH, t.critical_tint, t.critical, f))
            .child(card::title(title.clone(), f))
            .child(card::content(sentence.to_string(), &t))
            .child(self.details(&t, f, window, cx));
            div()
                .id("sk-crash")
                .role(Role::Dialog)
                .aria_label(title)
                .aria_description(sentence)
                .key_context("SkCrash")
                .on_key_down(cx.listener(Self::key_down))
                .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
                .size_full()
                .flex()
                .flex_col()
                .font_family(f.text)
                .font_weight(FontWeight::NORMAL)
                .text_color(t.text)
                .bg(t.footer)
                .child(card::card(
                    self.height.clone(),
                    body,
                    card::footer(&t, buttons),
                ))
        }
    }

    impl Drop for Crash {
        fn drop(&mut self) {
            // Gone without an answer (the app quitting some other way): the
            // primary window is usable again, and its own close works.
            self.release_owner();
        }
    }

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
            sk_tree::log::record("crash", line);
        }
    }

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
                    let s = strings(&spec.lang);
                    let labels: Vec<Label> = spec
                        .actions
                        .iter()
                        .map(|a| parse_label(a.label(s)))
                        .collect();
                    let expander = cx.focus_handle().tab_index(0).tab_stop(true);
                    let focus: Vec<FocusHandle> = (0..labels.len())
                        .map(|i| cx.focus_handle().tab_index(i as isize + 1).tab_stop(true))
                        .collect();
                    window.focus(&focus[primary(spec.actions)], cx);
                    let hwnd = hwnd_of(window).unwrap_or(0);
                    let icon =
                        win::icon_png(owner, hwnd, (16.0 * window.scale_factor()).round() as i32)
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
        cx.spawn(async move |cx: &mut AsyncApp| reveal(handle, hwnd, owner, height, cx).await)
            .detach();
        Ok(handle)
    }

    async fn reveal(
        handle: WindowHandle<Crash>,
        hwnd: isize,
        owner: isize,
        height: Rc<Cell<f32>>,
        cx: &mut AsyncApp,
    ) {
        card::fit(handle.into(), hwnd, owner, WIDTH, &height, cx).await;
        let Ok(live) = handle.update(cx, |p, _, _| p.outcome.is_some()) else {
            return;
        };
        if !live {
            return;
        }
        if card::disable_owner(owner) {
            let _ = handle.update(cx, |p, _, _| p.owner_disabled = true);
        }
        let activate = !crate::quiet::never_focus();
        crate::popout_win::show(hwnd, activate);
        let _ = handle.update(cx, |p, window, cx| {
            if window.is_window_active() || crate::popout_win::is_foreground(hwnd) {
                p.guard.arm(Instant::now());
            }
            cx.notify();
        });
    }

    #[doc(hidden)]
    #[derive(Debug, Clone)]
    pub struct Preview {
        pub lang: String,
        pub dark: Option<bool>,
        pub app: String,
        pub version: Option<String>,
        pub reason: String,
        pub owner: bool,
        pub embedded: bool,
        pub expanded: bool,
        pub accent: Option<Accent>,
        pub icon_from: Option<String>,
        pub solid: bool,
    }

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
            let owner = if p.owner {
                card::preview_owner(&p.app, cx)
            } else {
                0
            };
            let spec = Spec {
                report: Report {
                    app: p.app.clone(),
                    version: p.version.clone(),
                    os: super::os_version(),
                    time: std::time::SystemTime::now(),
                    what: "native panic",
                    reason: p.reason.clone(),
                },
                lang: if p.lang.is_empty() {
                    super::ui_lang()
                } else {
                    p.lang.clone()
                },
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

    #[test]
    fn utc_is_the_civil_date() {
        assert_eq!(utc(UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            utc(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29 00:00:00 UTC",
            "a leap day"
        );
        assert_eq!(
            utc(UNIX_EPOCH + Duration::from_secs(1_798_761_599)),
            "2026-12-31 23:59:59 UTC"
        );
    }

    #[test]
    fn a_long_unbroken_run_is_broken_and_ordinary_text_is_not() {
        let ordinary = "already borrowed: BorrowMutError at crates/sk-tree/src/lib.rs:120:9";
        assert_eq!(breakable(ordinary, 60), ordinary);
        assert_eq!(breakable("abcdefgh", 3), "abc\ndef\ngh");
        assert_eq!(
            breakable("abc def", 3),
            "abc def",
            "a run of exactly `columns` stays whole"
        );
        assert_eq!(
            breakable("ab\ncdef", 3),
            "ab\ncde\nf",
            "a line break ends a run"
        );
        assert_eq!(
            breakable("ёжикжук", 3),
            "ёжи\nкжу\nк",
            "by characters, not bytes"
        );
    }
}
