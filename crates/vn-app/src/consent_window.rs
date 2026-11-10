//! The permission prompt on Windows: a window of the app's own, drawn by gpui,
//! in place of vn-desktop's TaskDialog (docs/PERMISSIONS.md §5.1).
//!
//! ```text
//!   consent.rs ── ask_permission (frame thread) ── vn_desktop::dialog::ask_with
//!                                                    │ owner, queue, close, scripted seam
//!                                                    ▼
//!                                  vn-ask thread: present() ── Msg::Open ──▶ gpui main thread
//!                                    waits for the reply,                    open(): a hidden window,
//!                                    Msg::Close when the owner closes        sized to its content,
//!                                                    ◀── reply (once) ───── centred, owned, shown
//! ```
//!
//! # What it looks like, and why
//!
//! A Windows 11 ContentDialog, as the system's own consent prompts are: a
//! 440 DIP card on DWM's transient acrylic (`DWMSBT_TRANSIENTWINDOW`) with
//! rounded corners, the app's icon and name in caption type, the capability's
//! Segoe Fluent Icons glyph in an accent-tinted circle, the question in
//! Subtitle type over a sentence of body text, and a footer band holding the
//! buttons, equal width. It follows the system's light or dark app theme (live)
//! and the system accent colour. Before 22H2 there is no system acrylic, and
//! the card is drawn in the theme's solid colours instead. It has no entrance
//! animation of its own: it is shown already drawn at its final size, and the
//! open transition is DWM's.
//!
//! The first button (Allow) is the accent-filled one, whatever the default.
//! Initial focus, and so Enter, is on [`Ask::default`]: consent.rs makes
//! that Block, so a stray Enter grants nothing.
//!
//! # Input
//!
//! - Enter or Space presses the focused button; Esc and Alt+F4 dismiss.
//! - Tab, Shift+Tab and the arrow keys move between the buttons, with the
//!   Windows 11 focus ring once the keyboard is in use.
//! - Alt+letter presses the button whose `&` marks that letter (underlined
//!   while Alt is down, as Windows shows access keys).
//! - **Input protection**: for [`INPUT_DELAY`] after the prompt becomes the
//!   active window (shown, or activated again), clicks, Enter, Space and
//!   access keys are ignored, so a click or a key aimed at the app the moment
//!   the prompt appears cannot land on Allow (Chromium's permission bubble
//!   does the same). Esc and Tab are never ignored.
//!
//! # Lifecycle
//!
//! vn-desktop owns the rules (dialog/ask.rs): one question per owner at a
//! time (a second capability waits its turn), none for an owner that is
//! closing, and the owner is destroyed only once [`present`] has returned.
//! While the prompt is up its owner is disabled, as a TaskDialog's is. The
//! reply is sent exactly once: by a button, by a dismissal, by the owner
//! closing (`Msg::Close`), or by the window going away any other way (the
//! view's `Drop`).
//!
//! Accessibility: gpui reports the window to UI Automation through AccessKit,
//! so the card is a `Dialog` named by the question and each button a `Button`
//! with its access key.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::{
    canvas, div, img, prelude::*, px, size, App, AsyncApp, Bounds, ClickEvent, Context, DisplayId, FocusHandle,
    FontWeight, HighlightStyle, Hsla, Image, ImageFormat, KeyDownEvent, MouseButton, Rgba, Role, StyledText,
    Subscription, TitlebarOptions, UnderlineStyle, Window, WindowAppearance, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use vn_desktop::dialog::{Ask, Cancellation};
use vn_desktop::Fail;
use vn_runtime::redjs::webapi::permissions::Capability;

use crate::popout_win;

/// How long input is ignored once the prompt is the active window.
pub const INPUT_DELAY: Duration = Duration::from_millis(500);

/// The card's width, DIP. The height follows the text.
const WIDTH: f32 = 440.0;

/// The button styled as the accent (filled) one: Allow, by `Question`'s
/// contract (button 0 is Allow, button 1 Block).
const ACCENT: usize = 0;

// ---------------------------------------------------------------------------
// the pure parts
// ---------------------------------------------------------------------------

/// A button's label as drawn, and its access key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub text: String,
    /// The byte range of the access key's character in `text`, and that
    /// character, lower-cased.
    pub key: Option<(Range<usize>, char)>,
}

/// `&Allow` is "Allow" with access key `a`; `&&` is a literal ampersand
/// (the TaskDialog and Win32 menu convention, vn-desktop `label_of`).
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

/// Does a keystroke name access key `key`? `key_char` is the character the
/// layout types (`р` on a Russian layout), `name` gpui's key name (the
/// layout's unshifted character, often its Latin letter), and `vk_char` the
/// Latin letter of the key the layout puts `key` on (`VkKeyScanW`), which is
/// how a Cyrillic access key matches whichever of the two gpui reports.
pub fn access_key_matches(key: char, name: &str, key_char: Option<&str>, vk_char: Option<char>) -> bool {
    let same = |s: &str| {
        let mut it = s.chars().flat_map(char::to_lowercase);
        it.next() == Some(key) && it.next().is_none()
    };
    same(name)
        || key_char.is_some_and(same)
        || vk_char.is_some_and(|v| name.chars().count() == 1 && name.eq_ignore_ascii_case(v.encode_utf8(&mut [0; 4])))
}

/// What the input protection allows. Unarmed until the prompt is active,
/// then closed for [`INPUT_DELAY`]. A press counts from its start: a key
/// down, or a mouse button down on the button (a release after the delay
/// of a press begun before it is still refused).
#[derive(Debug, Default, Clone, Copy)]
pub struct Guard {
    open_at: Option<Instant>,
}

impl Guard {
    /// The prompt became the active window at `now`. Once: gpui hears of
    /// an activation a little after the show that caused it, and that must
    /// not restart the delay.
    pub fn arm(&mut self, now: Instant) {
        if self.open_at.is_none() {
            self.open_at = Some(now + INPUT_DELAY);
        }
    }

    /// It stopped being the active window.
    pub fn disarm(&mut self) {
        self.open_at = None;
    }

    pub fn allows(&self, now: Instant) -> bool {
        self.open_at.is_some_and(|t| now >= t)
    }
}

/// An sRGB colour, 0–255.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// WCAG relative luminance.
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }

    fn alpha(self, a: f32) -> Hsla {
        Rgba { r: self.0 as f32 / 255.0, g: self.1 as f32 / 255.0, b: self.2 as f32 / 255.0, a }.into()
    }
}

/// WCAG contrast ratio of two colours.
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Black or white, whichever reads better on `fill`: the text of an accent
/// button. Windows' own choice for its default blues (white on the light
/// theme's dark accent, black on the dark theme's light one) falls out of
/// it, and so does a readable answer for a user's own accent.
pub fn text_on(fill: Rgb) -> Rgb {
    let (black, white) = (Rgb(0, 0, 0), Rgb(255, 255, 255));
    if contrast(fill, black) > contrast(fill, white) { black } else { white }
}

/// The accent fills Windows 11 uses: `AccentDark1` on the light theme,
/// `AccentLight2` on the dark one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accent {
    pub light_theme: Rgb,
    pub dark_theme: Rgb,
}

impl Accent {
    /// Windows' default blue.
    pub const DEFAULT: Accent = Accent { light_theme: Rgb(0x00, 0x5F, 0xB8), dark_theme: Rgb(0x60, 0xCD, 0xFF) };

    pub fn fill(self, dark: bool) -> Rgb {
        if dark { self.dark_theme } else { self.light_theme }
    }
}

/// Is the accent distinct enough from the card to colour the capability's
/// glyph (3:1, WCAG's for graphics)? A grey or near-black accent on the dark
/// theme is not, and the glyph takes the text colour instead.
pub fn glyph_in_accent(fill: Rgb, dark: bool) -> bool {
    let card = if dark { Rgb(0x2B, 0x2B, 0x2B) } else { Rgb(0xFF, 0xFF, 0xFF) };
    contrast(fill, card) >= 3.0
}

/// Physical px, `[left, top, right, bottom]`.
pub type Rect = [i32; 4];

/// Where a `w`×`h` client area goes: centred on `anchor`, then moved into
/// `work` (a monitor's work area), keeping its top-left corner on it when it
/// is larger.
pub fn center_in(anchor: Rect, work: Rect, w: i32, h: i32) -> (i32, i32) {
    let x = anchor[0] + (anchor[2] - anchor[0] - w) / 2;
    let y = anchor[1] + (anchor[3] - anchor[1] - h) / 2;
    (x.min(work[2] - w).max(work[0]), y.min(work[3] - h).max(work[1]))
}

/// The capability's Segoe Fluent Icons / Segoe MDL2 Assets glyph.
pub fn glyph(cap: Capability) -> char {
    match cap {
        Capability::Microphone => '\u{E720}',
        Capability::Camera => '\u{E722}',
        Capability::DisplayCapture => '\u{E7F4}',
        Capability::Notifications => '\u{EA8F}',
        Capability::ClipboardRead | Capability::ClipboardWrite => '\u{E77F}',
        Capability::Geolocation => '\u{E81D}',
    }
}

// ---------------------------------------------------------------------------
// the vn-ask thread's side
// ---------------------------------------------------------------------------

/// What the prompt shows.
#[derive(Debug, Clone)]
pub struct Spec {
    pub capability: Capability,
    /// The app's name, in the caption row and the window title.
    pub app: String,
    pub instruction: String,
    pub content: String,
    pub buttons: Vec<String>,
    /// Focused at first, so the button Enter presses.
    pub default: usize,
    /// The asking window's HWND, 0 for none.
    pub owner: isize,
    /// The theme, when it is not the system's (the preview).
    pub dark: Option<bool>,
}

type Answer = Result<Option<usize>, String>;

enum Msg {
    Open { id: u64, spec: Spec, reply: mpsc::Sender<Answer> },
    Close(u64),
}

static TX: OnceLock<UnboundedSender<Msg>> = OnceLock::new();

/// Starts the main thread's side: a task that opens and closes prompts for
/// [`present`]. Call once, on the gpui main thread.
pub fn install(cx: &mut App) {
    let (tx, mut rx) = unbounded::<Msg>();
    if TX.set(tx).is_err() {
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(msg) = rx.next().await {
            cx.update(|cx| receive(msg, cx));
        }
    })
    .detach();
}

/// vn-desktop's presenter (`dialog::ask_with`), on the `vn-ask` thread: shows
/// the prompt on the main thread and waits for its reply. When the owner
/// starts closing it closes the prompt, and returns once that is done.
pub fn present(capability: Capability, q: &Ask, owner: isize, cancel: &Cancellation) -> Result<Option<usize>, Fail> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let fail = |m: &str| Fail::new("NotSupportedError", m.to_string());
    let Some(tx) = TX.get() else { return Err(fail("the prompt window is not installed")) };
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (reply, answer) = mpsc::channel();
    let spec = Spec {
        capability,
        app: q.title.clone(),
        instruction: q.instruction.clone(),
        content: q.content.clone(),
        buttons: q.buttons.clone(),
        default: q.default,
        owner,
        dark: None,
    };
    tx.unbounded_send(Msg::Open { id, spec, reply }).map_err(|_| fail("the main thread has stopped"))?;
    let mut closing: Option<Instant> = None;
    loop {
        match answer.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(a)) => return Ok(a),
            Ok(Err(e)) => return Err(fail(&e)),
            // Every holder of the reply is gone without sending: the window
            // went away (the view's `Drop` replies first, so this is a main
            // loop that has stopped).
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        match closing {
            None if cancel.is_requested() => {
                let _ = tx.unbounded_send(Msg::Close(id));
                closing = Some(Instant::now());
            }
            // A main thread that does not answer a close in seconds is not
            // running; the owner must not wait on it forever.
            Some(t) if t.elapsed() > Duration::from_secs(5) => return Ok(None),
            _ => {}
        }
    }
}

thread_local! {
    /// The prompts on screen, by `present`'s id.
    static OPEN: RefCell<Vec<(u64, WindowHandle<Prompt>)>> = const { RefCell::new(Vec::new()) };
}

fn receive(msg: Msg, cx: &mut App) {
    match msg {
        Msg::Open { id, spec, reply } => {
            let sent = reply.clone();
            let done: Done = Box::new(move |a| {
                let _ = sent.send(Ok(a));
            });
            match open(id, spec, done, cx) {
                Ok(handle) => OPEN.with(|o| o.borrow_mut().push((id, handle))),
                Err(e) => {
                    let _ = reply.send(Err(format!("{e:#}")));
                }
            }
        }
        Msg::Close(id) => {
            let handle = OPEN.with(|o| o.borrow().iter().find(|(i, _)| *i == id).map(|(_, h)| *h));
            if let Some(handle) = handle {
                let _ = handle.update(cx, |p, window, _| p.finish(None, window));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// the window
// ---------------------------------------------------------------------------

type Done = Box<dyn FnOnce(Option<usize>)>;

struct Fonts {
    display: &'static str,
    text: &'static str,
    small: &'static str,
    icons: &'static str,
}

/// Segoe UI Variable's optical sizes and Segoe Fluent Icons on Windows 11;
/// Segoe UI and Segoe MDL2 Assets before it.
fn fonts(cx: &App) -> &'static Fonts {
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
        }
    })
}

/// The colours of one theme: Windows 11's ContentDialog tokens.
struct Theme {
    text: Hsla,
    text2: Hsla,
    /// The card above the footer, and the footer band.
    body: Hsla,
    footer: Hsla,
    stroke: Hsla,
    button: [Hsla; 3],
    button_text_pressed: Hsla,
    button_border: Hsla,
    accent: [Hsla; 3],
    on_accent: Hsla,
    on_accent_pressed: Hsla,
    accent_border: Hsla,
    tint: Hsla,
    glyph: Hsla,
    ring: (Hsla, Hsla),
}

impl Theme {
    fn new(dark: bool, acrylic: bool, accent: Accent) -> Theme {
        let fill = accent.fill(dark);
        let on = text_on(fill);
        let black = Rgb(0, 0, 0);
        let white = Rgb(255, 255, 255);
        let vivid = glyph_in_accent(fill, dark);
        if dark {
            Theme {
                text: white.alpha(1.0),
                text2: white.alpha(0.786),
                // ContentDialog's two surfaces, #2B2B2B over #202020, laid
                // over the acrylic thinly enough that it shows through.
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
                ring: (white.alpha(1.0), black.alpha(0.7)),
            }
        } else {
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
                ring: (black.alpha(0.896), white.alpha(0.7)),
            }
        }
    }
}

struct Prompt {
    id: u64,
    spec: Spec,
    labels: Vec<Label>,
    focus: Vec<FocusHandle>,
    guard: Guard,
    accent: Accent,
    acrylic: bool,
    icon: Option<Arc<Image>>,
    done: Option<Done>,
    hwnd: isize,
    /// The owner was disabled by this prompt and must be enabled again.
    owner_disabled: bool,
    /// The card's laid-out height, DIP, from its last frame.
    height: Rc<Cell<f32>>,
    /// The button a mouse press began on, once the guard allowed it.
    pressing: Option<usize>,
    _subs: Vec<Subscription>,
}

impl Prompt {
    fn dark(&self, window: &Window) -> bool {
        self.spec.dark.unwrap_or(matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark))
    }

    /// The reply, once; then the window goes.
    fn finish(&mut self, answer: Option<usize>, window: &mut Window) {
        let Some(done) = self.done.take() else { return };
        // The owner first: an owned window that is destroyed hands the
        // activation to its owner only when the owner can take it.
        self.release_owner();
        done(answer);
        window.remove_window();
    }

    fn release_owner(&mut self) {
        if std::mem::take(&mut self.owner_disabled) {
            win::enable(self.spec.owner, true);
            if popout_win::is_foreground(self.hwnd) {
                win::activate(self.spec.owner);
            }
        }
    }

    fn press(&mut self, i: usize, window: &mut Window) {
        if i < self.labels.len() && self.guard.allows(Instant::now()) {
            self.finish(Some(i), window);
        }
    }

    fn focused(&self, window: &Window) -> Option<usize> {
        self.focus.iter().position(|f| f.is_focused(window))
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let m = k.modifiers;
        let plain = !m.alt && !m.control && !m.platform;
        let n = self.focus.len();
        let step = |p: &Self, window: &mut Window, cx: &mut Context<Self>, back: bool| {
            let at = p.focused(window).unwrap_or(p.spec.default);
            let next = if back { (at + n - 1) % n } else { (at + 1) % n };
            window.focus(&p.focus[next], cx);
        };
        match k.key.as_str() {
            "escape" if plain => self.finish(None, window),
            "tab" if plain => step(self, window, cx, m.shift),
            "left" | "up" if plain => step(self, window, cx, true),
            "right" | "down" if plain => step(self, window, cx, false),
            "enter" | "space" if plain && !m.shift && !e.is_held => {
                let i = self.focused(window).unwrap_or(self.spec.default);
                self.press(i, window);
            }
            _ if m.alt && !m.control && !m.platform && !e.is_held => {
                let vk = |c: char| win::latin_key_of(c);
                let hit = self.labels.iter().position(|l| {
                    l.key.as_ref().is_some_and(|(_, c)| access_key_matches(*c, &k.key, k.key_char.as_deref(), vk(*c)))
                });
                match hit {
                    Some(i) => {
                        window.focus(&self.focus[i], cx);
                        self.press(i, window);
                    }
                    None => return,
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn button(&self, i: usize, t: &Theme, alt: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = &self.labels[i];
        let accent = i == ACCENT;
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
        let ring = window.last_input_was_keyboard() && self.focus[i].is_focused(window);
        let shortcut = label.key.as_ref().map(|(_, c)| format!("Alt+{}", c.to_uppercase()));
        div()
            .flex_1()
            .relative()
            .child(
                div()
                    .id(("vn-consent-button", i))
                    .track_focus(&self.focus[i])
                    .role(Role::Button)
                    .aria_label(label.text.clone())
                    .when_some(shortcut, |d, s| d.aria_keyshortcuts(s))
                    .h(px(32.0))
                    .w_full()
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(border)
                    .bg(fill[0])
                    .text_color(fg)
                    .hover(move |s| s.bg(fill[1]))
                    .active(move |s| s.bg(fill[2]).text_color(fg_pressed))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, _| {
                            this.pressing = this.guard.allows(Instant::now()).then_some(i);
                        }),
                    )
                    // A mouse click only: the keyboard's Enter and Space are
                    // `key_down`'s, on the key's down stroke and not repeated.
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, _| {
                        if matches!(e, ClickEvent::Mouse(_)) && this.pressing.take() == Some(i) {
                            this.press(i, window);
                        }
                    }))
                    .child(text),
            )
            .when(ring, |d| {
                // Windows 11's focus visual: 2 px outside, 1 px inside it,
                // 3 px out from the control.
                d.child(
                    div()
                        .absolute()
                        .top(px(-3.0))
                        .left(px(-3.0))
                        .right(px(-3.0))
                        .bottom(px(-3.0))
                        .rounded(px(7.0))
                        .border_2()
                        .border_color(t.ring.0)
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(t.ring.1),
                        ),
                )
            })
    }
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark(window);
        let t = Theme::new(dark, self.acrylic, self.accent);
        let f = fonts(cx);
        let alt = window.modifiers().alt;
        let height = self.height.clone();
        let buttons: Vec<_> = (0..self.labels.len()).map(|i| self.button(i, &t, alt, window, cx)).collect();
        let icon = match &self.icon {
            Some(image) => img(image.clone()).size(px(16.0)).into_any_element(),
            // Segoe Fluent Icons' AppIconDefault.
            None => div()
                .font_family(f.icons)
                .text_size(px(16.0))
                .line_height(px(16.0))
                .text_color(t.text2)
                .child("\u{ECAA}")
                .into_any_element(),
        };
        div()
            .id("vn-consent")
            .role(Role::Dialog)
            .aria_label(self.spec.instruction.clone())
            .aria_description(self.spec.content.clone())
            .key_context("VnConsent")
            .on_key_down(cx.listener(Self::key_down))
            .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
            .size_full()
            .flex()
            .flex_col()
            .font_family(f.text)
            .text_color(t.text)
            // The window, while it is taller than the card (the first frame).
            .bg(t.footer)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w_full()
                    .flex()
                    .flex_col()
                    // What the window is sized to: the card's own height.
                    .child(
                        canvas(move |b, _, _| height.set(f32::from(b.size.height)), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    )
                    .child(self.body(&t, f, icon))
                    .child(self.footer(&t, buttons)),
            )
    }
}

impl Prompt {
    fn body(&self, t: &Theme, f: &Fonts, icon: gpui::AnyElement) -> impl IntoElement {
        div()
                    .id("vn-consent-body")
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
                                    .child(self.spec.app.clone()),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(20.0))
                            .size(px(40.0))
                            .rounded_full()
                            .bg(t.tint)
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_family(f.icons)
                            .text_size(px(20.0))
                            .line_height(px(20.0))
                            .text_color(t.glyph)
                            .child(glyph(self.spec.capability).to_string()),
                    )
                    .child(
                        div()
                            .mt(px(16.0))
                            .font_family(f.display)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(20.0))
                            .line_height(px(28.0))
                            .child(self.spec.instruction.clone()),
                    )
                    .when(!self.spec.content.is_empty(), |d| {
                        d.child(
                            div()
                                .mt(px(8.0))
                                .text_size(px(14.0))
                                .line_height(px(20.0))
                                .text_color(t.text2)
                                .child(self.spec.content.clone()),
                        )
                    })
    }

    fn footer(&self, t: &Theme, buttons: Vec<impl IntoElement>) -> impl IntoElement {
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
}

impl Drop for Prompt {
    fn drop(&mut self) {
        self.release_owner();
        if let Some(done) = self.done.take() {
            done(None);
        }
        let id = self.id;
        let _ = OPEN.try_with(|o| o.borrow_mut().retain(|(i, _)| *i != id));
    }
}

fn hwnd_of(window: &Window) -> Option<isize> {
    let handle = <Window as HasWindowHandle>::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }
}

/// Opens the prompt, hidden, and reveals it once it has its size and place.
fn open(id: u64, spec: Spec, done: Done, cx: &mut App) -> anyhow::Result<WindowHandle<Prompt>> {
    let owner = win::usable(spec.owner).then_some(spec.owner).unwrap_or(0);
    let spec = Spec { owner, ..spec };
    if spec.buttons.is_empty() {
        anyhow::bail!("a question without buttons");
    }
    let monitor = win::monitor_of(owner);
    let display = (monitor != 0).then(|| DisplayId::new(monitor as u64)).filter(|d| cx.find_display(*d).is_some());
    let acrylic = crate::window::has_system_backdrop() && !PREVIEW_SOLID.load(Ordering::Relaxed);
    let accent = PREVIEW_ACCENT.get().copied().or_else(win::accent).unwrap_or(Accent::DEFAULT);
    let height = Rc::new(Cell::new(0.0));
    let title = spec.app.clone();
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display, size(px(WIDTH), px(320.0)), cx))),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                appears_transparent: true,
                ..Default::default()
            }),
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
        },
        {
            let height = height.clone();
            move |window, cx| {
                cx.new(|cx| {
                    let labels: Vec<Label> = spec.buttons.iter().map(|b| parse_label(b)).collect();
                    let focus: Vec<FocusHandle> =
                        (0..labels.len()).map(|i| cx.focus_handle().tab_index(i as isize).tab_stop(true)).collect();
                    let default = spec.default.min(labels.len() - 1);
                    window.focus(&focus[default], cx);
                    let hwnd = hwnd_of(window).unwrap_or(0);
                    let icon = win::icon_png(owner, hwnd, (16.0 * window.scale_factor()).round() as i32)
                        .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                    let subs = vec![
                        cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
                        cx.observe_window_activation(window, |p: &mut Prompt, window, cx| {
                            if window.is_window_active() {
                                p.guard.arm(Instant::now());
                            } else {
                                p.guard.disarm();
                            }
                            cx.notify();
                        }),
                    ];
                    window.on_window_should_close(cx, |window, cx| {
                        // Alt+F4, or the system menu's Close: a dismissal.
                        if let Some(view) = window.root::<Prompt>().flatten() {
                            view.update(cx, |p, _| {
                                p.release_owner();
                                if let Some(done) = p.done.take() {
                                    done(None);
                                }
                            });
                        }
                        true
                    });
                    Prompt {
                        id,
                        spec: Spec { default, ..spec },
                        labels,
                        focus,
                        guard: Guard::default(),
                        accent,
                        acrylic,
                        icon,
                        done: Some(done),
                        hwnd,
                        owner_disabled: false,
                        pressing: None,
                        height,
                        _subs: subs,
                    }
                })
            }
        },
    )?;
    let hwnd = handle.update(cx, |p, _, _| p.hwnd)?;
    if hwnd == 0 {
        let _ = handle.update(cx, |p, window, _| p.finish(None, window));
        anyhow::bail!("the prompt window has no HWND");
    }
    win::style(hwnd, owner);
    if let Some(dark) = handle.update(cx, |p, _, _| p.spec.dark)? {
        win::set_dark(hwnd, dark);
    }
    cx.spawn(async move |cx: &mut AsyncApp| reveal(handle, hwnd, owner, height, cx).await).detach();
    Ok(handle)
}

/// Out of any `App` update, as every resize must be (popout.rs
/// `warm_small`): sizes the hidden window to its card, centres it on its
/// owner, draws it at that size, and shows it.
async fn reveal(handle: WindowHandle<Prompt>, hwnd: isize, owner: isize, height: Rc<Cell<f32>>, cx: &mut AsyncApp) {
    for _ in 0..4 {
        let Ok(scale) = handle.update(cx, |_, window, _| window.scale_factor()) else { return };
        let (w, h) = ((WIDTH * scale).round() as i32, (height.get().max(1.0) * scale).round() as i32);
        let Some(want) = win::placement(hwnd, owner, w, h) else { break };
        if popout_win::window_rect(hwnd) == Some(want) {
            break;
        }
        popout_win::set_rect(hwnd, want);
        popout_win::paint_now(hwnd);
    }
    let Ok(live) = handle.update(cx, |p, _, _| p.done.is_some()) else { return };
    if !live {
        return;
    }
    if owner != 0 {
        win::enable(owner, false);
        let _ = handle.update(cx, |p, _, _| p.owner_disabled = true);
    }
    let activate = !crate::quiet::never_focus();
    popout_win::show(hwnd, activate);
    let _ = handle.update(cx, |p, window, cx| {
        if window.is_window_active() || popout_win::is_foreground(hwnd) {
            p.guard.arm(Instant::now());
        }
        cx.notify();
    });
}

// ---------------------------------------------------------------------------
// Win32
// ---------------------------------------------------------------------------

mod win {
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
        SetForegroundWindow, DI_NORMAL, GCLP_HICON, GCLP_HICONSM, HICON, IMAGE_ICON, LR_DEFAULTCOLOR,
    };

    fn h(raw: isize) -> HWND {
        HWND(raw as *mut _)
    }

    fn rect(r: RECT) -> Rect {
        [r.left, r.top, r.right, r.bottom]
    }

    /// A window that can own the prompt: one that exists.
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

    /// The owner's monitor, or the primary one.
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
    /// prompt is centred on its monitor (the primary one without an owner).
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

    /// The window rect that puts a `w`×`h` client area centred on the owner.
    pub fn placement(raw: isize, owner: isize, w: i32, h: i32) -> Option<Rect> {
        let outer = super::popout_win::window_rect(raw)?;
        let inner = super::popout_win::client_rect_on_screen(raw)?;
        let border = [inner[0] - outer[0], inner[1] - outer[1], outer[2] - inner[2], outer[3] - inner[3]];
        let anchor = anchor(owner)?;
        let work = work_area(anchor).unwrap_or(anchor);
        let (x, y) = super::center_in(anchor, work, w, h);
        Some([x - border[0], y - border[1], x + w + border[2], y + h + border[3]])
    }

    /// While the window is still hidden: owned (no taskbar button, above
    /// its owner), round corners.
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

    /// The preview's theme, in place of the system's: what tints the acrylic.
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

    /// The system's accent fills (`UISettings`, what WinUI reads).
    pub fn accent() -> Option<Accent> {
        use windows::UI::ViewManagement::{UIColorType, UISettings};
        let s = UISettings::new().ok()?;
        let get = |t| s.GetColorValue(t).ok().map(|c| Rgb(c.R, c.G, c.B));
        Some(Accent { light_theme: get(UIColorType::AccentDark1)?, dark_theme: get(UIColorType::AccentLight2)? })
    }

    /// The Latin letter of the key that types `c` on the current layout.
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

    /// The app's icon at `px`×`px`, as a PNG: the exe's own (resource 1, the
    /// one gpui gives its windows), else the owner's class icon.
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

    /// The preview's: another exe's first icon.
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

// ---------------------------------------------------------------------------
// the preview
// ---------------------------------------------------------------------------

/// What the preview shows (examples/consent_prompt.rs).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct Preview {
    pub capability: Capability,
    pub russian: bool,
    pub dark: Option<bool>,
    pub app: String,
    /// Ask over a window of its own, rather than with no owner.
    pub owner: bool,
    pub accent: Option<Accent>,
    /// An exe whose icon stands in for the app's.
    pub icon_from: Option<String>,
    /// Ask through vn-desktop's question (`dialog::ask_with`, the vn-ask
    /// thread, [`present`]) as the app does, and, with a delay, close the
    /// owner under the prompt after it.
    pub through_desktop: bool,
    pub close_owner_after: Option<Duration>,
    /// The card without acrylic, as Windows 10 draws it.
    pub solid: bool,
}

/// The preview's owner, for vn-desktop's owner resolver.
static PREVIEW_OWNER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// The preview owner's vn-desktop key.
const PREVIEW_KEY: u64 = 1;

/// `cargo run -p vn-app --example consent_prompt`: the prompt on its own,
/// for a look at it. Prints the answer and exits.
#[doc(hidden)]
pub fn preview(p: Preview) {
    if let Some(a) = p.accent {
        let _ = PREVIEW_ACCENT.set(a);
    }
    PREVIEW_SOLID.store(p.solid, Ordering::Relaxed);
    if let Some(exe) = p.icon_from.clone() {
        let _ = PREVIEW_ICON.set(exe);
    }
    gpui_platform::application().run(move |cx: &mut App| {
        let q = vn_permissions::consent::question(p.capability, &p.app, p.russian);
        let mut owner = 0;
        if p.owner {
            let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
            if let Ok(h) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some(p.app.clone().into()), ..Default::default() }),
                    window_background: WindowBackgroundAppearance::MicaBackdrop,
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Blank),
            ) {
                owner = h.update(cx, |_, window, _| hwnd_of(window).unwrap_or(0)).unwrap_or(0);
            }
        }
        if p.through_desktop {
            install(cx);
            PREVIEW_OWNER.store(owner, Ordering::Relaxed);
            vn_desktop::dialog::set_owner_resolver(|| {
                let hwnd = PREVIEW_OWNER.load(Ordering::Relaxed);
                (hwnd != 0).then_some(vn_desktop::dialog::Owner { key: PREVIEW_KEY, hwnd })
            });
            let capability = p.capability;
            let waits_for_close = p.close_owner_after.is_some();
            vn_desktop::dialog::ask_with(
                Ask {
                    what: "permission",
                    title: q.title,
                    instruction: q.instruction,
                    content: q.content,
                    buttons: q.buttons,
                    default: q.default,
                },
                Box::new(move |q, owner, cancel| present(capability, q, owner, cancel)),
                Box::new(move |a| {
                    println!("answer: {a:?}");
                    if !waits_for_close {
                        std::process::exit(0);
                    }
                }),
            );
            if let Some(after) = p.close_owner_after {
                cx.spawn(async move |cx: &mut AsyncApp| {
                    cx.background_executor().timer(after).await;
                    let waits = vn_desktop::dialog::owner_closing(PREVIEW_KEY);
                    let started = Instant::now();
                    while !vn_desktop::dialog::is_idle(PREVIEW_KEY) {
                        cx.background_executor().timer(Duration::from_millis(16)).await;
                    }
                    println!("owner closing: waited={waits} idle after {} ms", started.elapsed().as_millis());
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
            return;
        }
        let spec = Spec {
            capability: p.capability,
            app: q.title,
            instruction: q.instruction,
            content: q.content,
            buttons: q.buttons,
            default: q.default,
            owner,
            dark: p.dark,
        };
        let done: Done = Box::new(|a| println!("answer: {a:?}"));
        // The prompt's reply ends the preview once its window has gone.
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        if let Err(e) = open(0, spec, done, cx) {
            eprintln!("the prompt could not open: {e:#}");
            cx.quit();
        }
    });
}

/// The preview draws Windows 10's solid card on Windows 11.
static PREVIEW_SOLID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The preview's accent, in place of the system's.
static PREVIEW_ACCENT: OnceLock<Accent> = OnceLock::new();
/// The preview's icon: an exe's, in place of this one's (which has none).
static PREVIEW_ICON: OnceLock<String> = OnceLock::new();

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_loses_its_marker_and_keeps_its_key() {
        let l = parse_label("&Allow");
        assert_eq!(l.text, "Allow");
        assert_eq!(l.key, Some((0..1, 'a')));
        let l = parse_label("&Блокировать");
        assert_eq!(l.text, "Блокировать");
        assert_eq!(l.key, Some((0..2, 'б')), "a byte range, for the underline");
        let l = parse_label("Save && Q&uit");
        assert_eq!(l.text, "Save & Quit");
        assert_eq!(l.key, Some((8..9, 'u')));
        assert_eq!(parse_label("Plain").key, None);
        assert_eq!(parse_label("Trailing&").text, "Trailing");
        // The first marker wins, as Windows' does.
        assert_eq!(parse_label("&One &Two").key, Some((0..1, 'o')));
    }

    #[test]
    fn an_access_key_matches_the_letter_or_the_key_it_is_on() {
        assert!(access_key_matches('a', "a", Some("a"), Some('a')));
        assert!(access_key_matches('a', "A", None, None));
        assert!(!access_key_matches('a', "b", Some("b"), Some('a')), "the key must be the access key's");
        // A Russian layout: `р` is on the H key. gpui names it either way.
        assert!(access_key_matches('р', "h", None, Some('h')));
        assert!(access_key_matches('р', "р", None, Some('h')));
        assert!(access_key_matches('р', "h", Some("Р"), None));
        assert!(!access_key_matches('р', "enter", None, Some('h')));
    }

    #[test]
    fn input_is_ignored_until_the_prompt_has_been_active_for_the_delay() {
        let t0 = Instant::now();
        let mut g = Guard::default();
        assert!(!g.allows(t0), "not shown yet");
        g.arm(t0);
        assert!(!g.allows(t0));
        assert!(!g.allows(t0 + INPUT_DELAY - Duration::from_millis(1)));
        assert!(g.allows(t0 + INPUT_DELAY));
        // Deactivated, then activated again: the delay starts over.
        // gpui hearing of the same activation later does not restart it.
        g.arm(t0 + INPUT_DELAY / 2);
        assert!(g.allows(t0 + INPUT_DELAY));
        g.disarm();
        assert!(!g.allows(t0 + INPUT_DELAY * 4));
        g.arm(t0 + INPUT_DELAY * 4);
        assert!(!g.allows(t0 + INPUT_DELAY * 5 - Duration::from_millis(1)));
        assert!(g.allows(t0 + INPUT_DELAY * 5));
    }

    #[test]
    fn accent_text_is_whichever_of_black_and_white_reads_better() {
        // Windows' defaults: white on the light theme's blue, black on the
        // dark theme's.
        assert_eq!(text_on(Accent::DEFAULT.light_theme), Rgb(255, 255, 255));
        assert_eq!(text_on(Accent::DEFAULT.dark_theme), Rgb(0, 0, 0));
        // A user's grey accent (AccentLight2 #595959) and a pale yellow one.
        assert_eq!(text_on(Rgb(0x59, 0x59, 0x59)), Rgb(255, 255, 255));
        assert_eq!(text_on(Rgb(0xFF, 0xE0, 0x80)), Rgb(0, 0, 0));
        assert!(contrast(Rgb(0, 0, 0), Rgb(255, 255, 255)) > 20.9);
        // The glyph is coloured by the default blues, not by a grey accent
        // on the dark card.
        assert!(glyph_in_accent(Accent::DEFAULT.light_theme, false));
        assert!(glyph_in_accent(Accent::DEFAULT.dark_theme, true));
        assert!(!glyph_in_accent(Rgb(0x59, 0x59, 0x59), true));
        assert!((contrast(Rgb(10, 20, 30), Rgb(10, 20, 30)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_prompt_is_centred_on_its_owner_and_kept_on_its_monitor() {
        let work = [0, 0, 1920, 1040];
        assert_eq!(center_in([100, 100, 1100, 700], work, 440, 300), (380, 250));
        // An owner hanging off the right edge: the prompt stays on screen.
        assert_eq!(center_in([1700, 100, 2700, 700], work, 440, 300), (1480, 250));
        // A monitor to the left of the primary one.
        assert_eq!(center_in([-1500, 0, -500, 600], [-1920, 0, 0, 1040], 440, 300), (-1220, 150));
        // Larger than the work area: its top-left corner stays on it.
        assert_eq!(center_in([0, 0, 100, 100], [0, 0, 400, 200], 440, 300), (0, 0));
    }

    #[test]
    fn every_capability_has_a_glyph() {
        assert_eq!(glyph(Capability::Microphone), '\u{E720}');
        assert_eq!(glyph(Capability::Camera), '\u{E722}');
        for &c in Capability::ALL {
            assert!(('\u{E000}'..='\u{F8FF}').contains(&glyph(c)), "a private-use icon glyph");
        }
    }
}
