//! The app's own prompts for the microphone and camera, layered over the platform's consent store.
//!
//! Windows never asks an unpackaged process, but a browser asks per site and users expect the same of
//! an app: for the capabilities in [`prompts`], [`AppConsent`] puts the app's decision on top of the platform's:
//!
//! | platform (OS) | the app's decision | `status` |
//! |---|---|---|
//! | denied | any | `denied` (by the system) |
//! | granted / prompt / unsupported | none | `prompt` |
//! | granted / prompt / unsupported | Allow | `granted` |
//! | granted / prompt / unsupported | Block | `denied` (by the user) |
//!
//! Other capabilities are the platform's, except that the capture latches also apply to one the OS
//! itself prompts for (macOS screen recording).
//!
//! [`PermissionHost::request`] on a prompting capability: an OS denial opens the platform's settings
//! page and, once lifted, goes on to the question; a stored Allow grants at once; otherwise the question
//! is asked (the way back from a Block, since `getUserMedia` asks only from `prompt`). The question is
//! the embedder's dialog ([`set_asker`]): button 0 Allow, button 1 Block (default); a dismissal decides
//! nothing. Concurrent requests share one question.

use std::sync::{Arc, Mutex, OnceLock};

use vn_runtime::redjs::webapi::permissions::{Capability, PermissionHost, PermissionState, RequestOutcome};

use crate::decisions::{Decision, Entry, Store};

/// The capabilities this layer asks about itself: the microphone, and the camera where this build
/// can capture one. Where the OS prompts itself (macOS TCC), this layer defers to it.
pub fn prompts(cap: Capability) -> bool {
    prompts_with(cap, CAMERA_CAPTURE.load(std::sync::atomic::Ordering::Relaxed))
}

static CAMERA_CAPTURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_camera_capture(available: bool) {
    CAMERA_CAPTURE.store(available, std::sync::atomic::Ordering::Relaxed);
}

fn prompts_with(cap: Capability, camera_capture: bool) -> bool {
    match cap {
        Capability::Microphone => true,
        Capability::Camera => camera_capture,
        _ => false,
    }
}

/// What the embedder's dialog shows. Block is the default: `getUserMedia` needs no gesture, so a
/// stray Enter must not grant a capture for good.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub capability: Capability,
    pub title: String,
    pub instruction: String,
    pub content: String,
    pub buttons: Vec<String>,
    pub default: usize,
}

/// Shows a question and calls back once with the button pressed, or `None`. Must not block or call
/// back while the caller holds a lock it needs: the reply may come on any thread, or before it returns.
pub type Asker = fn(Question, Box<dyn FnOnce(Option<usize>) + Send>);

static ASKER: OnceLock<Asker> = OnceLock::new();

pub fn set_asker(f: Asker) {
    let _ = ASKER.set(f);
}

fn installed_asker() -> Option<Asker> {
    ASKER.get().copied()
}

static APP_NAME: Mutex<Option<String>> = Mutex::new(None);

pub fn set_app_name(name: &str) {
    let name = name.trim();
    *APP_NAME.lock().unwrap_or_else(|p| p.into_inner()) = (!name.is_empty()).then(|| name.to_string());
}

pub(crate) fn app_name() -> String {
    APP_NAME.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_else(|| "This app".to_string())
}

fn russian_ui() -> bool {
    #[cfg(target_os = "windows")]
    {
        let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        lang & 0x3ff == 0x19
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub fn question(cap: Capability, app: &str, russian: bool) -> Question {
    let camera = cap == Capability::Camera;
    let (instruction, content, allow, block) = if russian && camera {
        (
            format!("{app} запрашивает доступ к камере"),
            format!("Если разрешить, {app} сможет записывать видео с камеры — сейчас и в следующий раз. Выбор запоминается для этого приложения."),
            "&Разрешить",
            "&Блокировать",
        )
    } else if camera {
        (
            format!("{app} wants to use your camera"),
            format!("If you allow it, {app} can record video from your camera, now and next time. Your choice is remembered for this app."),
            "&Allow",
            "&Block",
        )
    } else if russian {
        (
            format!("{app} запрашивает доступ к микрофону"),
            format!(
                "Если разрешить, {app} сможет записывать звук с микрофона — сейчас и в следующий раз. \
                 Выбор запоминается для этого приложения."
            ),
            "&Разрешить",
            "&Блокировать",
        )
    } else {
        (
            format!("{app} wants to use your microphone"),
            format!(
                "If you allow it, {app} can record from your microphone, now and next time. \
                 Your choice is remembered for this app."
            ),
            "&Allow",
            "&Block",
        )
    };
    Question {
        capability: cap,
        title: app.to_string(),
        instruction,
        content,
        buttons: vec![allow.to_string(), block.to_string()],
        default: 1,
    }
}

#[derive(Clone, Copy)]
pub struct Hooks {
    pub forbidden: fn() -> bool,
    pub fake: fn() -> bool,
    pub asker: fn() -> Option<Asker>,
}

impl Hooks {
    #[cfg(feature = "audio")]
    pub fn process() -> Hooks {
        use vn_runtime::redjs::webapi::mediastream;
        Hooks { forbidden: mediastream::capture_forbidden, fake: mediastream::fake_capture, asker: installed_asker }
    }

    #[cfg(not(feature = "audio"))]
    pub fn process() -> Hooks {
        Hooks { forbidden: || false, fake: || false, asker: installed_asker }
    }
}

enum Waiter {
    Os { outer: u32, inner: u32, cap: Capability },
    Asked { outer: u32, cap: Capability },
}

#[derive(Default)]
struct Inner {
    next: u32,
    waiting: Vec<Waiter>,
    settled: Vec<(u32, PermissionState)>,
    changes: Vec<Capability>,
    asking: Vec<Capability>,
}

impl Inner {
    fn mint(&mut self) -> u32 {
        self.next = self.next.wrapping_add(1).max(1);
        self.next
    }

    fn changed(&mut self, cap: Capability) {
        if !self.changes.contains(&cap) {
            self.changes.push(cap);
        }
    }
}

struct Shared {
    os: &'static dyn PermissionHost,
    hooks: Hooks,
    store: Mutex<Store>,
    inner: Mutex<Inner>,
}

impl Shared {
    fn lock_store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn os_state(&self, cap: Capability) -> PermissionState {
        if (self.hooks.forbidden)() {
            return PermissionState::Denied;
        }
        if (self.hooks.fake)() {
            return PermissionState::Granted;
        }
        self.os.status(cap)
    }

    fn native_prompt(&self, cap: Capability) -> bool {
        self.os.has_native_prompt(cap) && !(self.hooks.fake)()
    }

    fn status(&self, cap: Capability) -> PermissionState {
        if !prompts(cap) {
            // The latches still govern one that only the OS asks about (macOS screen recording).
            return if self.os.has_native_prompt(cap) { self.os_state(cap) } else { self.os.status(cap) };
        }
        // TCC already asks for this app: a saved Allow must not turn its Prompt/Denied into a grant.
        if self.native_prompt(cap) {
            return self.os_state(cap);
        }
        if self.os_state(cap) == PermissionState::Denied {
            return PermissionState::Denied;
        }
        match self.lock_store().get(cap).map(|e| e.decision) {
            Some(Decision::Allow) => PermissionState::Granted,
            Some(Decision::Block) => PermissionState::Denied,
            None => PermissionState::Prompt,
        }
    }

    fn answered(&self, cap: Capability, reply: Option<usize>) {
        let decision = match reply {
            Some(0) => Some(Decision::Allow),
            Some(1) => Some(Decision::Block),
            _ => None,
        };
        let stored = decision.is_some_and(|d| self.lock_store().set(cap, Some(d)));
        let state = self.status(cap);
        let where_ = self.lock_store().path().map(|p| p.display().to_string());
        eprintln!(
            "[permissions] {}: {}{}",
            cap.web_name(),
            match decision {
                Some(Decision::Allow) => "allowed",
                Some(Decision::Block) => "blocked",
                None => "the prompt was dismissed, nothing is stored",
            },
            match (decision, where_) {
                (Some(_), Some(p)) => format!(" (stored in {p})"),
                (Some(_), None) => " (for this run: no storage folder)".to_string(),
                _ => String::new(),
            }
        );
        let mut i = self.lock_inner();
        if stored {
            i.changed(cap);
        }
        i.asking.retain(|c| *c != cap);
        let mut done = Vec::new();
        i.waiting.retain(|w| match w {
            Waiter::Asked { outer, cap: c } if *c == cap => {
                done.push(*outer);
                false
            }
            _ => true,
        });
        for outer in done {
            i.settled.push((outer, state));
        }
    }
}

pub struct AppConsent {
    shared: Arc<Shared>,
}

impl AppConsent {
    pub fn new(os: &'static dyn PermissionHost, hooks: Hooks) -> AppConsent {
        AppConsent {
            shared: Arc::new(Shared {
                os,
                hooks,
                store: Mutex::new(Store::new()),
                inner: Mutex::new(Inner::default()),
            }),
        }
    }

    pub fn set_decisions_file(&self, path: Option<std::path::PathBuf>) {
        self.shared.lock_store().set_path(path);
    }

    pub fn decisions(&self) -> Vec<(Capability, Entry)> {
        self.shared.lock_store().list()
    }

    pub fn decisions_file(&self) -> Option<std::path::PathBuf> {
        self.shared.lock_store().path().map(|p| p.to_path_buf())
    }

    pub fn reset(&self, cap: Option<Capability>) -> Vec<Capability> {
        let caps: Vec<Capability> = match cap {
            Some(c) => vec![c],
            None => Capability::ALL.iter().copied().filter(|c| prompts(*c)).collect(),
        };
        let mut forgotten = Vec::new();
        for c in caps {
            if self.shared.lock_store().set(c, None) {
                forgotten.push(c);
            }
        }
        let mut i = self.shared.lock_inner();
        for c in &forgotten {
            i.changed(*c);
        }
        forgotten
    }

    fn os_request(&self, cap: Capability) -> RequestOutcome {
        match self.shared.os.request(cap) {
            RequestOutcome::Settled(s) if !prompts(cap) => RequestOutcome::Settled(s),
            RequestOutcome::Settled(_) => RequestOutcome::Settled(self.shared.status(cap)),
            RequestOutcome::Pending(inner) => {
                let mut i = self.shared.lock_inner();
                let outer = i.mint();
                i.waiting.push(Waiter::Os { outer, inner, cap });
                RequestOutcome::Pending(outer)
            }
        }
    }

    pub fn in_flight(&self, cap: Capability) -> bool {
        self.shared.lock_inner().waiting.iter().any(|w| match w {
            Waiter::Os { cap: c, .. } | Waiter::Asked { cap: c, .. } => *c == cap,
        })
    }

    fn ask(&self, cap: Capability) -> RequestOutcome {
        let outer = self.shared.lock_inner().mint();
        self.ask_as(outer, cap);
        RequestOutcome::Pending(outer)
    }

    fn ask_as(&self, outer: u32, cap: Capability) {
        let show = {
            let mut i = self.shared.lock_inner();
            i.waiting.push(Waiter::Asked { outer, cap });
            let show = !i.asking.contains(&cap);
            if show {
                i.asking.push(cap);
            }
            show
        };
        if show {
            let shared = Arc::clone(&self.shared);
            match (self.shared.hooks.asker)() {
                Some(asker) => asker(
                    question(cap, &app_name(), russian_ui()),
                    Box::new(move |reply| shared.answered(cap, reply)),
                ),
                None => {
                    static ONCE: std::sync::Once = std::sync::Once::new();
                    ONCE.call_once(|| {
                        eprintln!("[permissions] this host has no dialog for the permission prompt: every request is dismissed");
                    });
                    shared.answered(cap, None);
                }
            }
        }
    }
}

impl PermissionHost for AppConsent {
    fn platform(&self) -> &'static str {
        self.shared.os.platform()
    }

    fn status(&self, cap: Capability) -> PermissionState {
        self.shared.status(cap)
    }

    fn denied_by_system(&self, cap: Capability) -> bool {
        !prompts(cap) || self.shared.os_state(cap) == PermissionState::Denied
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        if !prompts(cap) {
            let latched = (self.shared.hooks.forbidden)() || (self.shared.hooks.fake)();
            if latched && self.shared.os.has_native_prompt(cap) {
                return RequestOutcome::Settled(self.shared.os_state(cap));
            }
            return self.os_request(cap);
        }
        if (self.shared.hooks.forbidden)() {
            return RequestOutcome::Settled(PermissionState::Denied);
        }
        if self.shared.native_prompt(cap) {
            return self.os_request(cap);
        }
        if self.shared.os_state(cap) == PermissionState::Denied {
            return self.os_request(cap);
        }
        match self.shared.lock_store().get(cap).map(|e| e.decision) {
            Some(Decision::Allow) => return RequestOutcome::Settled(PermissionState::Granted),
            Some(Decision::Block) | None => {}
        }
        self.ask(cap)
    }

    fn drain_settled(&self) -> Vec<(u32, PermissionState)> {
        let from_os = self.shared.os.drain_settled();
        let mut chained = Vec::new();
        let mut i = self.shared.lock_inner();
        for (inner_token, os_state) in from_os {
            let pos = i.waiting.iter().position(|w| matches!(w, Waiter::Os { inner, .. } if *inner == inner_token));
            let Some(pos) = pos else { continue };
            let Waiter::Os { outer, cap, .. } = i.waiting.remove(pos) else { continue };
            let state = if prompts(cap) {
                drop(i);
                let s = self.shared.status(cap);
                // An OS denial now lifted, with no Allow yet: ask rather than settle `prompt`.
                let ask = s != PermissionState::Granted
                    && !self.shared.native_prompt(cap)
                    && self.shared.os_state(cap) != PermissionState::Denied;
                i = self.shared.lock_inner();
                if ask {
                    chained.push((outer, cap));
                    continue;
                }
                s
            } else {
                os_state
            };
            i.settled.push((outer, state));
        }
        drop(i);
        for (outer, cap) in chained {
            self.ask_as(outer, cap);
        }
        std::mem::take(&mut self.shared.lock_inner().settled)
    }

    fn drain_changes(&self) -> Option<Vec<Capability>> {
        let mut all = self.shared.os.drain_changes().unwrap_or_default();
        for c in std::mem::take(&mut self.shared.lock_inner().changes) {
            if !all.contains(&c) {
                all.push(c);
            }
        }
        Some(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::StubHost;
    use std::cell::RefCell;
    use PermissionState::*;

    type ReplyFn = Box<dyn FnOnce(Option<usize>) + Send>;

    thread_local! {
        static ASKED: RefCell<Vec<(Question, ReplyFn)>> = RefCell::new(Vec::new());
    }

    fn test_asker(q: Question, reply: ReplyFn) {
        ASKED.with(|a| a.borrow_mut().push((q, reply)));
    }
    fn asked() -> usize {
        ASKED.with(|a| a.borrow().len())
    }
    fn answer(button: Option<usize>) {
        let (_, reply) = ASKED.with(|a| a.borrow_mut().remove(0));
        reply(button);
    }

    fn no() -> bool {
        false
    }
    fn yes() -> bool {
        true
    }
    fn with_asker() -> Option<Asker> {
        Some(test_asker)
    }
    fn without_asker() -> Option<Asker> {
        None
    }

    fn consent_over(os: PermissionState, hooks: Hooks) -> AppConsent {
        let os: &'static StubHost = Box::leak(Box::new(
            StubHost::scripted("test", &[(Capability::Microphone, os), (Capability::Camera, Denied), (Capability::DisplayCapture, Denied)]).granting(Granted),
        ));
        AppConsent::new(os, hooks)
    }

    fn consent(os: PermissionState) -> AppConsent {
        consent_over(os, Hooks { forbidden: no, fake: no, asker: with_asker })
    }

    fn pending(o: RequestOutcome) -> u32 {
        match o {
            RequestOutcome::Pending(t) => t,
            other => panic!("expected a pending request, got {other:?}"),
        }
    }

    struct NativeConsent {
        state: Mutex<PermissionState>,
        settled: Mutex<Vec<(u32, PermissionState)>>,
        requested: std::sync::atomic::AtomicUsize,
    }

    impl PermissionHost for NativeConsent {
        fn platform(&self) -> &'static str { "native-test" }
        fn has_native_prompt(&self, cap: Capability) -> bool { cap == Capability::Microphone }
        fn status(&self, _cap: Capability) -> PermissionState { *self.state.lock().unwrap() }
        fn request(&self, _cap: Capability) -> RequestOutcome {
            self.requested.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            RequestOutcome::Pending(37)
        }
        fn drain_settled(&self) -> Vec<(u32, PermissionState)> {
            std::mem::take(&mut *self.settled.lock().unwrap())
        }
    }

    #[test]
    fn native_per_app_consent_bypasses_the_app_dialog_and_stored_answers() {
        let os = Box::leak(Box::new(NativeConsent {
            state: Mutex::new(Prompt), settled: Mutex::new(Vec::new()),
            requested: std::sync::atomic::AtomicUsize::new(0),
        }));
        let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
        h.shared.lock_store().set(Capability::Microphone, Some(Decision::Allow));
        assert_eq!(h.status(Capability::Microphone), Prompt, "a saved app Allow cannot grant TCC");
        let token = pending(h.request(Capability::Microphone));
        assert_eq!(asked(), 0, "the OS owns the only prompt");
        assert!(h.drain_settled().is_empty());
        *os.state.lock().unwrap() = Granted;
        os.settled.lock().unwrap().push((37, Granted));
        assert_eq!(h.drain_settled(), vec![(token, Granted)], "native token maps to the outer request exactly once");
        assert!(h.drain_settled().is_empty());
        assert_eq!(h.status(Capability::Microphone), Granted);
        *os.state.lock().unwrap() = Denied;
        assert_eq!(h.status(Capability::Microphone), Denied, "revoked native access overrides the saved app Allow");
        assert!(h.denied_by_system(Capability::Microphone));

        let fake = AppConsent::new(os, Hooks { forbidden: no, fake: yes, asker: with_asker });
        let fake_token = pending(fake.request(Capability::Microphone));
        assert_eq!(asked(), 1, "fake capture still uses this app's ordinary test consent");
        answer(Some(0));
        assert_eq!(fake.drain_settled(), vec![(fake_token, Granted)]);
        assert_eq!(os.requested.load(std::sync::atomic::Ordering::SeqCst), 1);

        let forbidden = AppConsent::new(os, Hooks { forbidden: yes, fake: no, asker: with_asker });
        assert_eq!(forbidden.request(Capability::Microphone), RequestOutcome::Settled(Denied));
        assert_eq!(os.requested.load(std::sync::atomic::Ordering::SeqCst), 1, "the deny latch never calls native APIs");
    }

    struct NativeScreen {
        state: Mutex<PermissionState>,
        requested: std::sync::atomic::AtomicUsize,
    }

    impl PermissionHost for NativeScreen {
        fn platform(&self) -> &'static str { "native-test" }
        fn has_native_prompt(&self, cap: Capability) -> bool { cap == Capability::DisplayCapture }
        fn status(&self, _cap: Capability) -> PermissionState { *self.state.lock().unwrap() }
        fn request(&self, _cap: Capability) -> RequestOutcome {
            self.requested.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.state.lock().unwrap() = Denied;
            RequestOutcome::Pending(51)
        }
        fn drain_settled(&self) -> Vec<(u32, PermissionState)> { vec![(51, Denied)] }
    }

    #[test]
    fn native_screen_recording_is_the_oss_except_under_the_capture_latches() {
        let os = Box::leak(Box::new(NativeScreen {
            state: Mutex::new(Prompt),
            requested: std::sync::atomic::AtomicUsize::new(0),
        }));
        let requested = || os.requested.load(std::sync::atomic::Ordering::SeqCst);

        let forbidden = AppConsent::new(os, Hooks { forbidden: yes, fake: no, asker: with_asker });
        assert_eq!(forbidden.status(Capability::DisplayCapture), Denied);
        assert_eq!(forbidden.request(Capability::DisplayCapture), RequestOutcome::Settled(Denied));
        let fake = AppConsent::new(os, Hooks { forbidden: no, fake: yes, asker: with_asker });
        assert_eq!(fake.status(Capability::DisplayCapture), Granted, "a fake desktop is not the OS's to refuse");
        assert_eq!(fake.request(Capability::DisplayCapture), RequestOutcome::Settled(Granted));
        assert_eq!(requested(), 0, "neither latch raises the system prompt");

        let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
        assert_eq!(h.status(Capability::DisplayCapture), Prompt);
        let token = pending(h.request(Capability::DisplayCapture));
        assert_eq!((requested(), asked()), (1, 0), "the OS prompts; the app's dialog does not");
        assert_eq!(h.drain_settled(), vec![(token, Denied)]);
        assert!(h.denied_by_system(Capability::DisplayCapture));
    }

    #[test]
    fn nothing_decided_is_prompt_and_one_question_answers_every_waiter() {
        let h = consent(Granted);
        assert_eq!(h.status(Capability::Microphone), Prompt, "the OS allows desktop apps; this app was never asked");
        let a = pending(h.request(Capability::Microphone));
        let b = pending(h.request(Capability::Microphone));
        assert_ne!(a, b);
        assert_eq!(asked(), 1, "a second request shares the question on screen");
        assert!(h.drain_settled().is_empty(), "nothing settles before the answer");
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(a, Granted), (b, Granted)]);
        assert_eq!(h.status(Capability::Microphone), Granted);
        assert_eq!(h.drain_changes(), Some(vec![Capability::Microphone]), "an answer is a `change`");
        assert_eq!(h.request(Capability::Microphone), RequestOutcome::Settled(Granted), "granted: nothing to ask");
        assert_eq!(asked(), 0);
        assert_eq!(h.decisions().len(), 1);
    }

    #[test]
    fn block_is_the_users_denial_and_a_request_asks_again() {
        let h = consent(Granted);
        let t = pending(h.request(Capability::Microphone));
        answer(Some(1));
        assert_eq!(h.drain_settled(), vec![(t, Denied)]);
        assert_eq!(h.status(Capability::Microphone), Denied);
        assert!(!h.denied_by_system(Capability::Microphone), "the user said no, not the OS");
        let t = pending(h.request(Capability::Microphone));
        assert_eq!(asked(), 1);
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(t, Granted)]);
    }

    #[test]
    fn a_dismissal_decides_nothing() {
        let h = consent(Granted);
        let t = pending(h.request(Capability::Microphone));
        answer(None);
        assert_eq!(h.drain_settled(), vec![(t, Prompt)], "Esc: still undecided");
        assert_eq!(h.drain_changes(), Some(vec![]), "and nothing changed");
        assert!(h.decisions().is_empty());
        let t = pending(h.request(Capability::Microphone));
        answer(Some(1));
        let _ = h.drain_settled();
        let t2 = pending(h.request(Capability::Microphone));
        answer(None);
        assert_ne!(t, t2);
        assert_eq!(h.drain_settled(), vec![(t2, Denied)]);
    }

    #[test]
    fn the_os_denial_wins_and_is_the_systems() {
        let h = consent(Denied);
        assert_eq!(h.status(Capability::Microphone), Denied);
        assert!(h.denied_by_system(Capability::Microphone));
        let t = pending(h.request(Capability::Microphone));
        assert_eq!(asked(), 0, "the OS is asked first, not the user");
        assert!(h.in_flight(Capability::Microphone));
        assert!(h.drain_settled().is_empty(), "the OS let it through: the question is next");
        assert_eq!(asked(), 1);
        assert!(h.in_flight(Capability::Microphone), "still waiting, now on the question");
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(t, Granted)]);
        assert!(!h.in_flight(Capability::Microphone));
    }

    #[test]
    fn an_os_request_that_ends_still_denied_settles_without_asking() {
        let os: &'static StubHost =
            Box::leak(Box::new(StubHost::scripted("test", &[(Capability::Microphone, Denied)]).granting(Denied)));
        let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
        let t = pending(h.request(Capability::Microphone));
        assert_eq!(h.drain_settled(), vec![(t, Denied)]);
        assert_eq!(asked(), 0, "nothing to ask while the OS refuses");
        assert!(h.denied_by_system(Capability::Microphone));
    }

    #[test]
    fn a_lifted_os_denial_asks_again_after_an_earlier_block() {
        let os: &'static StubHost =
            Box::leak(Box::new(StubHost::scripted("test", &[(Capability::Microphone, Denied)]).granting(Granted)));
        let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
        h.shared.lock_store().set(Capability::Microphone, Some(Decision::Block));
        let t = pending(h.request(Capability::Microphone));
        assert!(h.drain_settled().is_empty());
        assert_eq!(asked(), 1, "the explicit request is the way back from the Block too");
        answer(None);
        assert_eq!(h.drain_settled(), vec![(t, Denied)], "dismissed: the Block stands");
        assert!(!h.denied_by_system(Capability::Microphone));
    }

    #[test]
    fn in_flight_tells_a_dismissal_from_a_question_on_screen() {
        let h = consent(Granted);
        assert!(!h.in_flight(Capability::Microphone));
        let t = pending(h.request(Capability::Microphone));
        assert!(h.in_flight(Capability::Microphone));
        assert_eq!(h.status(Capability::Microphone), Prompt);
        answer(None);
        assert!(!h.in_flight(Capability::Microphone), "dismissed: prompt, and nothing left to wait for");
        assert_eq!(h.drain_settled(), vec![(t, Prompt)]);
    }

    #[test]
    fn the_camera_prompts_only_where_it_can_be_captured() {
        set_camera_capture(true);
        assert!(prompts_with(Capability::Microphone, false));
        assert!(prompts_with(Capability::Camera, true));
        assert!(!prompts_with(Capability::Camera, false), "no backend: the platform's own state");
        assert!(!prompts_with(Capability::DisplayCapture, true));
        assert_eq!(prompts(Capability::Camera), cfg!(any(target_os = "windows", target_os = "macos")));
        if !prompts(Capability::Camera) {
            let os: &'static StubHost = Box::leak(Box::new(StubHost::scripted("test", &[(Capability::Camera, Unsupported)])));
            let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
            assert_eq!(h.status(Capability::Camera), Unsupported);
            assert_eq!(h.request(Capability::Camera), RequestOutcome::Settled(Unsupported));
            assert_eq!(asked(), 0);
        }
    }

    #[test]
    fn deny_never_asks_and_fake_leaves_the_os_out() {
        let h = consent_over(Granted, Hooks { forbidden: yes, fake: no, asker: with_asker });
        assert_eq!(h.status(Capability::Microphone), Denied);
        assert!(h.denied_by_system(Capability::Microphone));
        assert_eq!(h.request(Capability::Microphone), RequestOutcome::Settled(Denied));
        assert_eq!(asked(), 0, "VN_MEDIA_CAPTURE=deny never shows a prompt");
        let h = consent_over(Denied, Hooks { forbidden: no, fake: yes, asker: with_asker });
        assert_eq!(h.status(Capability::Microphone), Prompt, "a fake microphone is not the OS's to deny");
        let t = pending(h.request(Capability::Microphone));
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(t, Granted)]);
    }

    #[test]
    fn without_a_dialog_every_question_is_dismissed() {
        let h = consent_over(Granted, Hooks { forbidden: no, fake: no, asker: without_asker });
        let t = pending(h.request(Capability::Microphone));
        assert_eq!(h.drain_settled(), vec![(t, Prompt)]);
    }

    #[test]
    fn other_capabilities_are_the_platforms_under_tokens_of_ours() {
        let h = consent(Granted);
        assert_eq!(h.status(Capability::DisplayCapture), Denied);
        assert!(h.denied_by_system(Capability::DisplayCapture));
        let mic = pending(h.request(Capability::Microphone));
        let cam = pending(h.request(Capability::DisplayCapture));
        assert_ne!(mic, cam, "one namespace for both kinds of request");
        assert_eq!(h.drain_settled(), vec![(cam, Granted)], "the platform's answer, as it gave it");
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(mic, Granted)]);
    }

    #[test]
    fn a_reset_forgets_the_answer_and_is_a_change() {
        let h = consent(Granted);
        let _ = pending(h.request(Capability::Microphone));
        answer(Some(1));
        let _ = h.drain_settled();
        let _ = h.drain_changes();
        assert_eq!(h.reset(None), vec![Capability::Microphone]);
        assert_eq!(h.status(Capability::Microphone), Prompt, "asked again next time");
        assert_eq!(h.drain_changes(), Some(vec![Capability::Microphone]));
        assert!(h.reset(Some(Capability::Microphone)).is_empty(), "nothing left to forget");
    }

    #[test]
    fn the_question_names_the_app_in_english_or_russian() {
        let q = question(Capability::Microphone, "Argon", false);
        assert_eq!(q.title, "Argon");
        assert_eq!(q.instruction, "Argon wants to use your microphone");
        assert_eq!(q.buttons, vec!["&Allow".to_string(), "&Block".to_string()]);
        assert_eq!(q.default, 1, "Enter blocks: a stray key press never grants");
        let q = question(Capability::Microphone, "Argon", true);
        assert_eq!(q.instruction, "Argon запрашивает доступ к микрофону");
        assert_eq!(q.buttons, vec!["&Разрешить".to_string(), "&Блокировать".to_string()]);
        let q = question(Capability::Camera, "Argon", true);
        assert_eq!(q.instruction, "Argon запрашивает доступ к камере");
        assert!(q.content.contains("записывать видео"));
        assert!(!q.content.contains("микрофон"));
        assert_eq!(question(Capability::Camera, "Argon", false).instruction, "Argon wants to use your camera");
    }

    #[test]
    fn native_camera_consent_is_the_microphones_model() {
        set_camera_capture(true);
        if !prompts(Capability::Camera) {
            return;
        }
        struct NativeAv {
            states: Mutex<Vec<(Capability, PermissionState)>>,
            settled: Mutex<Vec<(u32, PermissionState)>>,
            requested: Mutex<Vec<Capability>>,
        }
        impl NativeAv {
            fn set(&self, cap: Capability, state: PermissionState) {
                self.states.lock().unwrap().retain(|(c, _)| *c != cap);
                self.states.lock().unwrap().push((cap, state));
            }
        }
        impl PermissionHost for NativeAv {
            fn platform(&self) -> &'static str { "native-test" }
            fn has_native_prompt(&self, cap: Capability) -> bool {
                matches!(cap, Capability::Microphone | Capability::Camera)
            }
            fn status(&self, cap: Capability) -> PermissionState {
                self.states.lock().unwrap().iter().find(|(c, _)| *c == cap).map_or(Unsupported, |(_, s)| *s)
            }
            fn request(&self, cap: Capability) -> RequestOutcome {
                self.requested.lock().unwrap().push(cap);
                RequestOutcome::Pending(if cap == Capability::Camera { 38 } else { 37 })
            }
            fn drain_settled(&self) -> Vec<(u32, PermissionState)> {
                std::mem::take(&mut *self.settled.lock().unwrap())
            }
        }
        let os = Box::leak(Box::new(NativeAv {
            states: Mutex::new(vec![(Capability::Microphone, Granted), (Capability::Camera, Prompt)]),
            settled: Mutex::new(Vec::new()),
            requested: Mutex::new(Vec::new()),
        }));
        let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
        h.shared.lock_store().set(Capability::Camera, Some(Decision::Allow));
        assert_eq!(h.status(Capability::Camera), Prompt, "a saved app Allow cannot grant TCC");
        let token = pending(h.request(Capability::Camera));
        assert_eq!(asked(), 0, "TCC owns the only prompt");
        assert_eq!(*os.requested.lock().unwrap(), vec![Capability::Camera]);
        assert!(h.in_flight(Capability::Camera));
        os.set(Capability::Camera, Granted);
        os.settled.lock().unwrap().push((38, Granted));
        assert_eq!(h.drain_settled(), vec![(token, Granted)]);
        assert!(!h.in_flight(Capability::Camera));
        assert_eq!(h.status(Capability::Camera), Granted);
        assert_eq!(h.status(Capability::Microphone), Granted, "the microphone's state is its own");
        os.set(Capability::Camera, Denied);
        assert_eq!(h.status(Capability::Camera), Denied);
        assert!(h.denied_by_system(Capability::Camera));
        let _ = h.request(Capability::Camera);
        assert_eq!(os.requested.lock().unwrap().len(), 2);
        assert_eq!(asked(), 0);
        let forbidden = AppConsent::new(os, Hooks { forbidden: yes, fake: no, asker: with_asker });
        assert_eq!(forbidden.request(Capability::Camera), RequestOutcome::Settled(Denied));
        assert_eq!(os.requested.lock().unwrap().len(), 2);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn camera_consent_is_separate_from_microphone_consent() {
        set_camera_capture(true);
        let os = Box::leak(Box::new(StubHost::scripted("test", &[(Capability::Microphone, Granted), (Capability::Camera, Granted)])));
        let h = AppConsent::new(os, Hooks { forbidden: no, fake: no, asker: with_asker });
        let mic = pending(h.request(Capability::Microphone));
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(mic, Granted)]);
        assert_eq!(h.status(Capability::Camera), Prompt);
        let cam = pending(h.request(Capability::Camera));
        assert_eq!(ASKED.with(|q| q.borrow()[0].0.capability), Capability::Camera);
        answer(Some(1));
        assert_eq!(h.drain_settled(), vec![(cam, Denied)]);
        assert_eq!(h.status(Capability::Microphone), Granted);
        assert_eq!(h.status(Capability::Camera), Denied);
        assert!(!h.denied_by_system(Capability::Camera));
        let cam = pending(h.request(Capability::Camera));
        answer(Some(0));
        assert_eq!(h.drain_settled(), vec![(cam, Granted)]);
        assert_eq!(h.status(Capability::Camera), Granted);
    }
}
