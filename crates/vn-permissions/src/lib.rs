//! vn-permissions — the platform half of vue-native's permission layer.
//!
//! # What is where, and why
//!
//! The browser-shaped surface (`navigator.permissions`, `PermissionStatus`, the
//! `change` event, the W3C name table) lives in **redjs**
//! (`redjs::webapi::permissions`), because it is browser semantics and redjs is
//! where browser semantics live. So does the [`PermissionHost`] trait it reads
//! through — a trait consumed by redjs has to be declared by redjs.
//!
//! This crate is the other half: the **backends** that implement that trait, and
//! the small `__vn_permissions_*` host-fn surface behind vue-native's own
//! `appPermissions` extension. That split is the same one `vn-voice` has with
//! `redjs::webapi::pcm`, and it is what keeps redjs shippable to a consumer
//! (PEWPEW) that has no consent store, no `ms-settings:` and no opinion about
//! Windows privacy toggles.
//!
//! ```text
//!   apps/hello  ──import { appPermissions } from "vue-native"──┐
//!                                                              │  tier 2: extension
//!   packages/vue-native/src/runtime.ts ──__vn_permissions_*────┤  (request, 4 states)
//!                                                              │
//!   apps/*      ──navigator.permissions.query(…)───────────────┤  tier 1: W3C
//!                                                              │
//!   redjs::webapi::permissions  ── dyn PermissionHost ─────────┘
//!                                        ▲
//!                          vn_permissions::{win,mac,stub}
//! ```
//!
//! # The two tiers, and the divergence between them
//!
//! Tier 1 is exactly the web: `navigator.permissions.query({name})` resolving a
//! `PermissionStatus` whose `state` is one of `granted | denied | prompt`.
//! Nothing about it is ours.
//!
//! Tier 2 exists because **the web has no standalone request**. A browser asks
//! for the microphone as a side effect of `getUserMedia`, for notifications
//! through `Notification.requestPermission()`, and for nothing at all through
//! `navigator.permissions`. A desktop app has a settings screen with a "grant
//! access" button next to a toggle, and there is no browser spelling for that
//! button. So `appPermissions.request(name)` is a documented vue-native
//! extension, named after the `appWindow` precedent, and it is where the fourth
//! state — `unsupported` — is visible too. See `docs/PERMISSIONS.md`.
//!
//! # Windows, in one paragraph
//!
//! Nothing to prompt for. An unpackaged win32 process is gated by pre-answered
//! privacy toggles in the consent store, so [`win`]'s `status()` reads them
//! honestly, never returns `Prompt`, and `request()` opens the relevant
//! `ms-settings:` page and waits for the store to change. `win.rs`'s module
//! header carries the probe evidence for every one of those decisions.

use std::ffi::c_void;

use vn_runtime::redjs::api::Value;
use vn_runtime::redjs::webapi::permissions::{
    self as api, Capability, PermissionHost, PermissionState, RequestOutcome,
};
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{host_fn, JsRuntime};

pub mod consent;
pub mod decisions;
pub mod stub;

#[cfg(target_os = "windows")]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;

pub use consent::{set_app_name, set_asker, Asker, Question};
pub use vn_runtime::redjs::webapi::permissions::{Capability as Cap, PermissionState as State};

/// How many settled requests one frame may deliver. One native prompt can
/// settle many waiting requests; the rest stay in the permission router for
/// later waves instead of extending this frame's callback work without bound.
const DRAIN_CAP: usize = 16;

thread_local! {
    /// The one protected JS callback, exactly as `vn-voice`'s dispatcher works.
    static DISPATCHER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The backend for this build: the platform's, with the app's own prompt on
/// top for the microphone ([`consent`]).
///
/// A `&'static` because [`api::set_host`] stores one for the life of the
/// process; there is exactly one OS under one process and no reason to make it
/// swappable at runtime.
fn backend() -> &'static consent::AppConsent {
    static C: std::sync::OnceLock<consent::AppConsent> = std::sync::OnceLock::new();
    C.get_or_init(|| consent::AppConsent::new(platform(), consent::Hooks::process()))
}

/// Where the app's answers to its prompt are kept: `permissions.json` in its
/// storage folder (vn-app passes it once storage is installed). `None`, or
/// never called: for this run only.
pub fn set_decisions_file(path: Option<std::path::PathBuf>) {
    backend().set_decisions_file(path);
}

/// Forgets the stored answer for `name` (a permission name), or for every
/// capability with a prompt when `None`, so the next `getUserMedia` asks
/// again. The devtools `permissions reset` (docs/DEVTOOLS.md). Returns the
/// names forgotten.
pub fn reset(name: Option<&str>) -> Result<Vec<&'static str>, String> {
    let cap = match name {
        None => None,
        Some(n) => Some(Capability::from_web_name(n).ok_or_else(|| format!("{n:?} is not a permission name"))?),
    };
    if let Some(c) = cap {
        if !consent::prompts(c) {
            return Err(format!("{n} has no prompt of this app's to reset: its state is the platform's", n = c.web_name()));
        }
    }
    Ok(backend().reset(cap).into_iter().map(Capability::web_name).collect())
}

/// The devtools `permissions` listing: every capability's four-state status,
/// the stored answers and where they are kept.
pub fn describe_json() -> String {
    let b = backend();
    let mut s = String::from("{\"states\":[");
    for (i, cap) in Capability::ALL.iter().copied().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"name\":\"{}\",\"state\":\"{}\",\"prompts\":{}}}",
            cap.web_name(),
            state_of(cap).as_str(),
            consent::prompts(cap)
        ));
    }
    s.push_str("],\"decisions\":[");
    for (i, (cap, e)) in b.decisions().into_iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{{\"name\":\"{}\",\"decision\":\"{}\",\"at\":{}}}", cap.web_name(), e.decision.as_str(), e.at));
    }
    let file = b.decisions_file().map(|p| p.display().to_string());
    s.push_str(&format!("],\"file\":{}}}", file.map(|f| json_string(&f)).unwrap_or_else(|| "null".into())));
    s
}

/// Where the user turns a capability back on once the platform has refused
/// it, for the message a refusal carries (`__vn_permissions_hint`,
/// `appCapture.permissionHint`); `None` where there is no such page. macOS
/// screen recording also needs a relaunch: TCC prompts once per app, ever.
pub fn refusal_hint(cap: Capability) -> Option<String> {
    #[cfg(target_os = "macos")]
    match cap {
        Capability::DisplayCapture => return Some(screen_recording_hint(&consent::app_name())),
        Capability::Microphone => return Some(mac_microphone_hint(&consent::app_name())),
        _ => {}
    }
    #[cfg(target_os = "windows")]
    match cap {
        Capability::Microphone => return Some(windows_hint("Microphone", "microphone", "privacy-microphone")),
        Capability::Camera => return Some(windows_hint("Camera", "camera", "privacy-webcam")),
        _ => {}
    }
    let _ = cap;
    None
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn screen_recording_hint(app: &str) -> String {
    format!(
        "Turn on {app} in System Settings › Privacy & Security › Screen & System Audio Recording \
         (Screen Recording before macOS 15), then quit and reopen {app}."
    )
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_microphone_hint(app: &str) -> String {
    format!(
        "Turn on {app} in System Settings › Privacy & Security › Microphone \
         (x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone)."
    )
}

/// The two consent-store toggles (`win.rs`), by the names Settings shows.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_hint(page: &str, device: &str, uri: &str) -> String {
    format!(
        "Turn on \"{page} access\" and \"Let desktop apps access your {device}\" in \
         Settings › Privacy & security › {page} (ms-settings:{uri})."
    )
}

/// The refusal of a `display-capture` the platform does not allow, as a
/// DOMException `(name, message)`: Chrome's "Permission denied by system",
/// plus the [`refusal_hint`].
pub fn display_capture_refusal() -> (&'static str, String) {
    capture_refusal(Capability::DisplayCapture)
}

/// The refusal of a capture of `cap` whose state is `denied`, as a
/// DOMException `(name, message)`, in Chrome's words: "Permission denied by
/// system" plus the [`refusal_hint`] when the platform's settings say no,
/// "Permission denied" when the user blocked it in this app's prompt.
pub fn capture_refusal(cap: Capability) -> (&'static str, String) {
    refusal_by(api::host(), cap)
}

/// [`capture_refusal`], as `host` tells whose denial it is.
pub fn refusal_by(host: Option<&dyn PermissionHost>, cap: Capability) -> (&'static str, String) {
    if (consent::Hooks::process().forbidden)() {
        return ("NotAllowedError", "Permission denied: media capture is switched off for this process.".into());
    }
    let system = host.is_none_or(|h| h.denied_by_system(cap));
    ("NotAllowedError", refusal_message(system, refusal_hint(cap)))
}

fn refusal_message(system: bool, hint: Option<String>) -> String {
    match (system, hint) {
        (true, Some(hint)) => format!("Permission denied by system. {hint}"),
        (true, None) => "Permission denied by system".into(),
        (false, _) => "Permission denied".into(),
    }
}

/// `cap`'s state as the installed host reads it (`Unsupported` with none).
pub fn status(cap: Capability) -> PermissionState {
    state_of(cap)
}

/// Blocks a worker thread, never the VM's, until `display-capture` is
/// decided, asking the platform first when nobody has. For native work that
/// reads the desktop outside `getDisplayMedia` (source thumbnails). Without a
/// host there is nobody to ask, and nothing is refused.
pub fn require_display_capture() -> Result<(), (&'static str, String)> {
    let Some(host) = api::host() else {
        return Ok(());
    };
    match await_decision(host, Capability::DisplayCapture, std::time::Duration::from_secs(120), &in_flight) {
        PermissionState::Granted | PermissionState::Unsupported => Ok(()),
        PermissionState::Denied => Err(display_capture_refusal()),
        PermissionState::Prompt => Err(("NotAllowedError", "Permission dismissed".into())),
    }
}

/// Whether a request about `cap` is still waiting for its answer (a question
/// on screen, a settings page being watched). A `prompt` with nothing in
/// flight was dismissed.
pub fn in_flight(cap: Capability) -> bool {
    backend().in_flight(cap)
}

/// Blocks a worker thread until a request already made about `cap` is
/// decided: its state once no longer `prompt`, or `prompt` when it was
/// dismissed, `timeout` passed or `cancelled` says so.
pub fn wait_for_decision(
    cap: Capability,
    timeout: std::time::Duration,
    cancelled: &dyn Fn() -> bool,
) -> PermissionState {
    let Some(host) = api::host() else {
        return PermissionState::Unsupported;
    };
    wait(host, cap, std::time::Instant::now() + timeout, &in_flight, cancelled)
}

/// `cap`'s state once it is no longer `prompt`, requesting it if it is, or
/// `prompt` when the request was dismissed or `timeout` passes first. The
/// request's token settles into the router like any other and is ignored.
fn await_decision(
    host: &dyn PermissionHost,
    cap: Capability,
    timeout: std::time::Duration,
    in_flight: &dyn Fn(Capability) -> bool,
) -> PermissionState {
    let deadline = std::time::Instant::now() + timeout;
    match host.status(cap) {
        PermissionState::Prompt => match host.request(cap) {
            RequestOutcome::Settled(state) => state,
            RequestOutcome::Pending(_) => wait(host, cap, deadline, in_flight, &|| false),
        },
        state => state,
    }
}

fn wait(
    host: &dyn PermissionHost,
    cap: Capability,
    deadline: std::time::Instant,
    in_flight: &dyn Fn(Capability) -> bool,
    cancelled: &dyn Fn() -> bool,
) -> PermissionState {
    loop {
        // In flight read first: an answer settles its waiters after it is
        // stored, so a request seen finished has its state readable.
        let waiting = in_flight(cap);
        match host.status(cap) {
            PermissionState::Prompt if waiting && !cancelled() && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            state => return state,
        }
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The platform's own backend.
fn platform() -> &'static dyn PermissionHost {
    #[cfg(target_os = "windows")]
    {
        static H: win::WindowsHost = win::WindowsHost::new();
        &H
    }
    #[cfg(target_os = "macos")]
    {
        static H: mac::MacHost = mac::MacHost::new();
        &H
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        use std::sync::OnceLock;
        static H: OnceLock<stub::StubHost> = OnceLock::new();
        H.get_or_init(|| stub::StubHost::unsupported("this platform"))
    }
}

// ---------------------------------------------------------------------------
// host functions
// ---------------------------------------------------------------------------

// `__vn_permissions_platform() -> string` — the name an advisory uses.
host_fn!(fn vn_permissions_platform(_g, _args) {
    let name = api::host().map(|h| h.platform()).unwrap_or("this platform");
    JsRuntime::with(|rt| rt.ctx().string(name).0)
});

// `__vn_permissions_query(name) -> string` — the FOUR-state answer, or `""`
// for a name that is not a capability.
//
// `""` rather than a throw: this is the extension tier's read, and the
// browser-shaped `TypeError` for an unknown name is already delivered by
// `navigator.permissions.query`. Two different errors for one typo is worse
// than one.
host_fn!(fn vn_permissions_query(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let state = Capability::from_web_name(&name).map(state_of).map(PermissionState::as_str).unwrap_or("");
    JsRuntime::with(|rt| rt.ctx().string(state).0)
});

// `__vn_permissions_list() -> json` — every capability, its four-state status,
// and whether `request()` on it would do anything on this platform. One call
// because a settings screen wants the whole table at once and seven host-fn
// round trips per repaint is silly.
//
// `deniedBySystem` tells a `denied` from the platform's own settings (a
// settings page is the way back) from the user's Block in this app's prompt
// (`request()` asks again); false for any other state.
host_fn!(fn vn_permissions_list(_g, _args) {
    let mut s = String::from("[");
    for (i, cap) in Capability::ALL.iter().copied().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let state = state_of(cap);
        let by_system = state == PermissionState::Denied && api::host().is_some_and(|h| h.denied_by_system(cap));
        s.push_str(&format!(
            "{{\"name\":\"{}\",\"state\":\"{}\",\"canRequest\":{},\"deniedBySystem\":{}}}",
            cap.web_name(),
            state.as_str(),
            can_request_state(state),
            by_system,
        ));
    }
    s.push(']');
    JsRuntime::with(|rt| rt.ctx().string(&s).0)
});

// `__vn_permissions_request(name) -> json`.
//
// `{"state":"granted"}` — settled synchronously, resolve now.
// `{"token":3}`         — a flow started; the dispatcher settles it later.
// `{"error":"..."}`     — not a capability name.
//
// Synchronous like every other host fn: the promise lives in JS, and the pump
// settles it. Same shape as `__vn_voice_call`'s reqId (`vn-voice/src/lib.rs`),
// for the same reason — there is no async `host_fn!`.
host_fn!(fn vn_permissions_request(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let json = request_json(&name, api::host());
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

/// `__vn_permissions_request`'s answer. The name is the page's own string,
/// so it goes into the error escaped, never spliced.
fn request_json(name: &str, host: Option<&dyn PermissionHost>) -> String {
    match Capability::from_web_name(name) {
        None => format!("{{\"error\":{}}}", json_string(&format!("{name} is not a permission name"))),
        Some(cap) => match host {
            None => format!("{{\"state\":\"{}\"}}", PermissionState::Unsupported.as_str()),
            Some(h) => match h.request(cap) {
                RequestOutcome::Settled(st) => format!("{{\"state\":\"{}\"}}", st.as_str()),
                RequestOutcome::Pending(token) => format!("{{\"token\":{token}}}"),
            },
        },
    }
}

// `__vn_permissions_hint(name) -> string` — the [`refusal_hint`], or `""`.
host_fn!(fn vn_permissions_hint(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let hint = Capability::from_web_name(&name).and_then(refusal_hint).unwrap_or_default();
    JsRuntime::with(|rt| rt.ctx().string(&hint).0)
});

host_fn!(fn vn_permissions_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    // A reload evaluates the runtime again, and it sets its own dispatcher:
    // the old one is let go rather than kept alive for the life of the
    // process (as `vn-voice`'s is).
    if let Some(old) = replace_dispatcher(f.0) {
        unsafe { vn_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    Value::undefined().0
});

/// Installs `new` as the dispatcher; the previous one, when it is another
/// value, is returned for unprotecting.
fn replace_dispatcher(new: u64) -> Option<u64> {
    let old = DISPATCHER.with(|d| d.replace(new));
    (old != 0 && old != new).then_some(old)
}

fn state_of(cap: Capability) -> PermissionState {
    api::host().map(|h| h.status(cap)).unwrap_or(PermissionState::Unsupported)
}

/// Would `request()` do anything here? Drives whether the demo page's button is
/// enabled — a button that is guaranteed to be a no-op should not be offered.
/// Testable without a host being installed process-wide.
fn can_request_state(state: PermissionState) -> bool {
    !matches!(state, PermissionState::Granted | PermissionState::Unsupported)
}

// ---------------------------------------------------------------------------
// install & pump
// ---------------------------------------------------------------------------

/// Installs the platform backend into redjs and registers the extension surface.
///
/// Called from `vn-app`'s boot beside `vn_voice::install`, unconditionally: the
/// backend is a few registry reads and one lazily-spawned watcher thread that
/// only starts once something actually subscribes.
pub fn install(rt: &JsRuntime) {
    let host: &'static dyn PermissionHost = backend();
    api::set_host(host);
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_permissions_platform", 0, vn_permissions_platform as *const _),
        ("__vn_permissions_query", 1, vn_permissions_query as *const _),
        ("__vn_permissions_list", 0, vn_permissions_list as *const _),
        ("__vn_permissions_request", 1, vn_permissions_request as *const _),
        ("__vn_permissions_hint", 1, vn_permissions_hint as *const _),
        ("__vn_permissions_set_dispatcher", 1, vn_permissions_set_dispatcher as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

/// Settles finished requests into JS.
///
/// Goes in `vn-app`'s `pump_frame` beside `vn_voice::pump()`, inside the same
/// `Phase::Commit` publication, and is **not** dev-gated.
///
/// The `change` half of the story is not here: `PermissionStatus` events are
/// dispatched by redjs's own `webapi::permissions::pump`, from inside
/// `rt.pump()`, because that is where the status objects live. Two drains of one
/// queue would race; `drain_changes` therefore has exactly one caller and this
/// function drains a different one.
pub fn pump() {
    let dispatcher = DISPATCHER.with(|d| d.get());
    if dispatcher == 0 {
        return;
    }
    if api::host().is_none() {
        return;
    }
    // Not the host's `drain_settled`: that queue also carries the requests
    // `getUserMedia` made, which redjs settles itself.
    let settled = api::take_settled_up_to(DRAIN_CAP);
    if settled.is_empty() {
        return;
    }

    JsRuntime::with(|rt| {
        for (token, state) in settled {
            let st = rt.ctx().string(state.as_str());
            let args = [Value::int32(token as i32), st];
            if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
                rt.ctx().call(Value(dispatcher), Value::undefined(), &args)
            {
                let err = rt.describe(e);
                eprintln!("[permissions dispatch] {}\n{}", err.header, err.stack);
            }
        }
        // The awaiting `.then()` and the Vue patch it triggers land inside this
        // same Commit publication — the rule `vn-voice::pump` documents.
        rt.ctx().drain_microtasks();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use stub::StubHost;

    #[test]
    fn a_host_with_no_backend_reports_unsupported_for_everything() {
        let h = StubHost::unsupported("nowhere");
        for cap in Capability::ALL.iter().copied() {
            assert_eq!(h.status(cap), PermissionState::Unsupported, "{cap:?}");
        }
        // And it must NOT claim a change subscription it does not have, or the
        // surface would promise `change` events that never arrive.
        assert!(h.drain_changes().is_none());
    }

    #[test]
    fn all_four_states_survive_the_round_trip_through_the_trait() {
        let h = StubHost::scripted(
            "test",
            &[
                (Capability::Microphone, PermissionState::Granted),
                (Capability::Camera, PermissionState::Denied),
                (Capability::DisplayCapture, PermissionState::Prompt),
                (Capability::Notifications, PermissionState::Unsupported),
            ],
        );
        assert_eq!(h.status(Capability::Microphone).as_str(), "granted");
        assert_eq!(h.status(Capability::Camera).as_str(), "denied");
        assert_eq!(h.status(Capability::DisplayCapture).as_str(), "prompt");
        assert_eq!(h.status(Capability::Notifications).as_str(), "unsupported");
        // And the browser projection of the same four.
        assert_eq!(h.status(Capability::DisplayCapture).web_state(), "prompt");
        assert_eq!(h.status(Capability::Notifications).web_state(), "denied");
    }

    #[test]
    fn a_request_for_something_already_granted_settles_without_asking() {
        // The behaviour that keeps a "grant access" button from throwing a
        // Settings window at a user who already granted it.
        let h = StubHost::scripted("test", &[(Capability::Microphone, PermissionState::Granted)])
            .granting(PermissionState::Granted);
        assert_eq!(
            h.request(Capability::Microphone),
            RequestOutcome::Settled(PermissionState::Granted)
        );
        assert!(h.drain_settled().is_empty(), "nothing was pending");
    }

    #[test]
    fn a_platform_that_cannot_ask_settles_with_the_truth_instead_of_hanging() {
        let h = StubHost::scripted("test", &[(Capability::Camera, PermissionState::Denied)]);
        assert_eq!(
            h.request(Capability::Camera),
            RequestOutcome::Settled(PermissionState::Denied)
        );
    }

    #[test]
    fn a_pending_request_settles_through_drain_settled_with_its_own_token() {
        let h = StubHost::scripted("test", &[(Capability::Camera, PermissionState::Denied)])
            .granting(PermissionState::Granted);
        let RequestOutcome::Pending(token) = h.request(Capability::Camera) else {
            panic!("expected a pending request");
        };
        assert_eq!(h.drain_settled(), vec![(token, PermissionState::Granted)]);
        assert!(h.drain_settled().is_empty(), "a settlement is delivered once");
    }

    #[test]
    fn a_state_change_queues_exactly_one_change_and_only_when_it_really_changed() {
        // The invariant that makes `change` trustworthy: re-setting the same
        // state is not an event. On Windows this is the difference between
        // silence and one `change` per microphone use by any app on the box.
        let h = StubHost::scripted("test", &[(Capability::Microphone, PermissionState::Granted)]);
        assert_eq!(h.drain_changes(), Some(vec![]));
        h.set(Capability::Microphone, PermissionState::Granted);
        assert_eq!(h.drain_changes(), Some(vec![]), "a no-op write is not a change");
        h.set(Capability::Microphone, PermissionState::Denied);
        assert_eq!(h.drain_changes(), Some(vec![Capability::Microphone]));
        assert_eq!(h.drain_changes(), Some(vec![]), "a change is delivered once");
    }

    #[test]
    fn a_worker_waits_for_a_decision_and_asks_only_from_prompt() {
        use std::time::Duration;
        let cap = Capability::DisplayCapture;
        let pending = |_: Capability| true;
        let h = StubHost::scripted("test", &[(cap, PermissionState::Prompt)]).granting(PermissionState::Denied);
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Denied);
        assert_eq!(h.drain_settled().len(), 1, "asked exactly once");
        // Already decided: read, not asked again (a refused request would
        // otherwise open System Settings on every thumbnail refresh).
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Denied);
        assert!(h.drain_settled().is_empty());
        let h = StubHost::scripted("test", &[(cap, PermissionState::Granted)]);
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Granted);
        // A platform that cannot ask settles with the truth at once.
        let h = StubHost::scripted("test", &[(cap, PermissionState::Prompt)]);
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Prompt);

        /// A request that never settles.
        struct Stuck;
        impl PermissionHost for Stuck {
            fn platform(&self) -> &'static str { "test" }
            fn status(&self, _: Capability) -> PermissionState { PermissionState::Prompt }
            fn request(&self, _: Capability) -> RequestOutcome { RequestOutcome::Pending(1) }
        }
        assert_eq!(await_decision(&Stuck, cap, Duration::from_millis(60), &pending), PermissionState::Prompt, "bounded");
        // Dismissed: `prompt` with nothing left in flight is the answer, now,
        // not after the timeout.
        let t = std::time::Instant::now();
        assert_eq!(await_decision(&Stuck, cap, Duration::from_secs(60), &|_| false), PermissionState::Prompt);
        assert!(t.elapsed() < Duration::from_secs(5), "a dismissal does not wait out the timeout");
        // And a waiter whose capture was stopped lets go too.
        let start = std::time::Instant::now();
        let deadline = start + Duration::from_secs(60);
        assert_eq!(wait(&Stuck, cap, deadline, &pending, &|| true), PermissionState::Prompt);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_screen_recording_hint_names_the_app_the_pane_and_the_relaunch() {
        let hint = screen_recording_hint("Argon");
        assert_eq!(
            hint,
            "Turn on Argon in System Settings › Privacy & Security › Screen & System Audio Recording \
             (Screen Recording before macOS 15), then quit and reopen Argon."
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(refusal_hint(Capability::DisplayCapture), None, "no other platform adds one");
    }

    #[test]
    fn a_refusal_hint_names_the_platforms_own_page() {
        assert_eq!(
            mac_microphone_hint("Argon"),
            "Turn on Argon in System Settings › Privacy & Security › Microphone \
             (x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone)."
        );
        assert_eq!(
            windows_hint("Camera", "camera", "privacy-webcam"),
            "Turn on \"Camera access\" and \"Let desktop apps access your camera\" in \
             Settings › Privacy & security › Camera (ms-settings:privacy-webcam)."
        );
        #[cfg(target_os = "windows")]
        {
            assert!(refusal_hint(Capability::Microphone).unwrap().contains("ms-settings:privacy-microphone"));
            assert!(refusal_hint(Capability::Camera).unwrap().contains("ms-settings:privacy-webcam"));
        }
        #[cfg(target_os = "macos")]
        assert!(refusal_hint(Capability::Microphone).unwrap().contains("Privacy_Microphone"));
        assert_eq!(refusal_hint(Capability::Geolocation), None);
    }

    #[test]
    fn a_refusal_is_the_systems_with_its_hint_or_the_users_without() {
        assert_eq!(refusal_message(true, Some("Turn it on.".into())), "Permission denied by system. Turn it on.");
        assert_eq!(refusal_message(true, None), "Permission denied by system");
        // The user's Block: Chrome's words, and no settings page to send them to.
        assert_eq!(refusal_message(false, Some("Turn it on.".into())), "Permission denied");
    }

    #[test]
    fn a_request_for_an_unknown_name_answers_an_escaped_error() {
        let json = request_json("mic\",\"state\":\"granted", None);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 1, "no field injected: {json}");
        assert_eq!(obj["error"], "mic\",\"state\":\"granted is not a permission name");
        assert_eq!(request_json("microphone", None), "{\"state\":\"unsupported\"}");
        let h = StubHost::scripted("test", &[(Capability::Camera, PermissionState::Granted)]);
        assert_eq!(request_json("camera", Some(&h)), "{\"state\":\"granted\"}");
    }

    #[test]
    fn a_new_dispatcher_lets_the_old_one_go() {
        // A reload sets a second dispatcher: the first must be unprotected,
        // and setting the same one again must not unprotect it.
        assert_eq!(replace_dispatcher(11), None);
        assert_eq!(replace_dispatcher(11), None, "the same function is kept");
        assert_eq!(replace_dispatcher(12), Some(11));
        assert_eq!(DISPATCHER.with(|d| d.get()), 12);
        DISPATCHER.with(|d| d.set(0));
    }

    #[test]
    fn can_request_is_false_for_granted_and_for_unsupported() {
        // The rule the demo page's button state is derived from: there is
        // nothing to ask for when you already have it, and nobody to ask when
        // the platform has no notion of it.
        assert!(!can_request_state(PermissionState::Granted));
        assert!(!can_request_state(PermissionState::Unsupported));
        assert!(can_request_state(PermissionState::Denied));
        assert!(can_request_state(PermissionState::Prompt));
    }
}
