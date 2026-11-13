//! The permission prompt on Windows: a window of the app's own, drawn by gpui, in place of
//! vn-desktop's TaskDialog.
//!
//! consent.rs `ask_permission` (frame thread) calls `vn_desktop::dialog::ask_with`, whose presenter
//! [`present`] runs on the `vn-ask` thread: it sends `Msg::Open` to the gpui main thread (a hidden
//! window, sized to its content, centred, owned, then shown), waits for the reply, and sends
//! `Msg::Close` when the owner closes.
//!
//! The first button (Allow) is the accent-filled one; initial focus, and so Enter, is on
//! [`Ask::default`], which consent.rs makes Block so a stray Enter grants nothing.
//!
//! **Input protection**: for [`INPUT_DELAY`] after the prompt becomes the active window, clicks,
//!   Enter, Space and access keys are ignored, so input aimed at the app the moment the prompt
//!   appears cannot land on Allow. Esc and Tab are never ignored.
//!
//! vn-desktop owns the rules (dialog/ask.rs): one question per owner at a time, none for a closing
//! owner, and the owner is destroyed only once [`present`] has returned. The owner is disabled while
//! the prompt is up. The reply is sent exactly once: by a button, a dismissal, the owner closing
//! (`Msg::Close`), or the window going away any other way (the view's `Drop`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::{
    div, prelude::*, App, AsyncApp, ClickEvent, Context, FocusHandle, Image, ImageFormat, KeyDownEvent,
    MouseButton, Role, Subscription, Window, WindowAppearance, WindowHandle,
};
use vn_desktop::dialog::{Ask, Cancellation};
use vn_desktop::Fail;
use vn_runtime::redjs::webapi::permissions::Capability;

use crate::content_dialog::{self as card, fonts, hwnd_of, win, Fonts, Theme, WIDTH};
pub use crate::content_dialog::{
    access_key_matches, center_in, contrast, glyph_in_accent, parse_label, text_on, Accent, Guard, Label, Rect, Rgb,
    INPUT_DELAY,
};

const ACCENT: usize = 0;

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

#[derive(Debug, Clone)]
pub struct Spec {
    pub capability: Capability,
    pub app: String,
    pub instruction: String,
    pub content: String,
    pub buttons: Vec<String>,
    pub default: usize,
    pub owner: isize,
    pub dark: Option<bool>,
}

type Answer = Result<Option<usize>, String>;

enum Msg {
    Open { id: u64, spec: Spec, reply: mpsc::Sender<Answer> },
    Close(u64),
}

static TX: OnceLock<UnboundedSender<Msg>> = OnceLock::new();

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

type Done = Box<dyn FnOnce(Option<usize>)>;

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
    owner_disabled: bool,
    height: Rc<Cell<f32>>,
    pressing: Option<usize>,
    _subs: Vec<Subscription>,
}

impl Prompt {
    fn dark(&self, window: &Window) -> bool {
        self.spec.dark.unwrap_or(matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark))
    }

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
            card::release_owner(self.spec.owner, self.hwnd);
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
            _ if m.alt && !m.control && !m.platform && !e.is_held => match card::access_key_hit(&self.labels, k) {
                Some(i) => {
                    window.focus(&self.focus[i], cx);
                    self.press(i, window);
                }
                None => return,
            },
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn button(&self, i: usize, t: &Theme, alt: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        card::button(("vn-consent-button", i), &self.labels[i], i == ACCENT, t, alt, &self.focus[i], window, |b| {
            b.on_mouse_down(
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
        })
    }

    fn body(&self, t: &Theme, f: &Fonts) -> impl IntoElement {
        card::body("vn-consent-body", &self.spec.app, card::app_icon(&self.icon, t, f), t, f)
            .child(card::glyph_badge(glyph(self.spec.capability), t.tint, t.glyph, f))
            .child(card::title(self.spec.instruction.clone(), f))
            .when(!self.spec.content.is_empty(), |d| d.child(card::content(self.spec.content.clone(), t)))
    }
}

impl Render for Prompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark(window);
        let t = Theme::new(dark, self.acrylic, self.accent);
        let f = fonts(cx);
        let alt = window.modifiers().alt;
        let buttons: Vec<_> = (0..self.labels.len()).map(|i| self.button(i, &t, alt, window, cx)).collect();
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
            .bg(t.footer)
            .child(card::card(self.height.clone(), self.body(&t, f), card::footer(&t, buttons)))
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

fn open(id: u64, spec: Spec, done: Done, cx: &mut App) -> anyhow::Result<WindowHandle<Prompt>> {
    let owner = win::usable(spec.owner).then_some(spec.owner).unwrap_or(0);
    let spec = Spec { owner, ..spec };
    if spec.buttons.is_empty() {
        anyhow::bail!("a question without buttons");
    }
    let display = card::display_of(owner, cx);
    let acrylic = card::acrylic();
    let accent = card::accent();
    let height = Rc::new(Cell::new(0.0));
    let options = card::window_options(spec.app.clone(), WIDTH, display, acrylic, cx);
    let handle = cx.open_window(options, {
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
    })?;
    let hwnd = handle.update(cx, |p, _, _| p.hwnd)?;
    if hwnd == 0 {
        let _ = handle.update(cx, |p, window, _| p.finish(None, window));
        anyhow::bail!("the prompt window has no HWND");
    }
    card::prepare(hwnd, owner, handle.update(cx, |p, _, _| p.spec.dark)?);
    cx.spawn(async move |cx: &mut AsyncApp| reveal(handle, hwnd, owner, height, cx).await).detach();
    Ok(handle)
}

async fn reveal(handle: WindowHandle<Prompt>, hwnd: isize, owner: isize, height: Rc<Cell<f32>>, cx: &mut AsyncApp) {
    card::fit(handle.into(), hwnd, owner, WIDTH, &height, cx).await;
    let Ok(live) = handle.update(cx, |p, _, _| p.done.is_some()) else { return };
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
    pub capability: Capability,
    pub lang: String,
    pub dark: Option<bool>,
    pub app: String,
    pub owner: bool,
    pub accent: Option<Accent>,
    pub icon_from: Option<String>,
    pub through_desktop: bool,
    pub close_owner_after: Option<Duration>,
    pub solid: bool,
}

static PREVIEW_OWNER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
const PREVIEW_KEY: u64 = 1;

#[doc(hidden)]
pub fn preview(p: Preview) {
    if let Some(a) = p.accent {
        let _ = card::PREVIEW_ACCENT.set(a);
    }
    card::PREVIEW_SOLID.store(p.solid, Ordering::Relaxed);
    if let Some(exe) = p.icon_from.clone() {
        let _ = card::PREVIEW_ICON.set(exe);
    }
    gpui_platform::application().run(move |cx: &mut App| {
        let lang = if p.lang.is_empty() { vn_locale::language() } else { p.lang.clone() };
        let q = vn_permissions::consent::question(p.capability, &p.app, &lang);
        let owner = if p.owner { card::preview_owner(&p.app, cx) } else { 0 };
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
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        if let Err(e) = open(0, spec, done, cx) {
            eprintln!("the prompt could not open: {e:#}");
            cx.quit();
        }
    });
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
        assert_eq!(text_on(Accent::DEFAULT.light_theme), Rgb(255, 255, 255));
        assert_eq!(text_on(Accent::DEFAULT.dark_theme), Rgb(0, 0, 0));
        assert_eq!(text_on(Rgb(0x59, 0x59, 0x59)), Rgb(255, 255, 255));
        assert_eq!(text_on(Rgb(0xFF, 0xE0, 0x80)), Rgb(0, 0, 0));
        assert!(contrast(Rgb(0, 0, 0), Rgb(255, 255, 255)) > 20.9);
        assert!(glyph_in_accent(Accent::DEFAULT.light_theme, false));
        assert!(glyph_in_accent(Accent::DEFAULT.dark_theme, true));
        assert!(!glyph_in_accent(Rgb(0x59, 0x59, 0x59), true));
        assert!((contrast(Rgb(10, 20, 30), Rgb(10, 20, 30)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_prompt_is_centred_on_its_owner_and_kept_on_its_monitor() {
        let work = [0, 0, 1920, 1040];
        assert_eq!(center_in([100, 100, 1100, 700], work, 440, 300), (380, 250));
        assert_eq!(center_in([1700, 100, 2700, 700], work, 440, 300), (1480, 250));
        assert_eq!(center_in([-1500, 0, -500, 600], [-1920, 0, 0, 1040], 440, 300), (-1220, 150));
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
