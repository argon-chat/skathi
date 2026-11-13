//! Platform backends for `navigator.permissions` (the `PermissionHost` trait lives in redjs)
//! and the `__vn_permissions_*` host fns behind vue-native's `appPermissions`.
//!
//! `appPermissions.request(name)` is an extension because the web has no standalone request;
//! it also exposes a fourth state, `unsupported`.
//!
//! Windows: an unpackaged win32 process is gated by pre-answered privacy toggles in the consent
//! store, so [`win`] reads them, never returns `Prompt`, and `request()` opens the `ms-settings:`
//! page and waits for the store to change.

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

#[cfg(any(target_os = "macos", test))]
mod tcc;

pub use consent::{set_app_name, set_asker, set_camera_capture, Asker, Question};
pub use vn_runtime::redjs::webapi::permissions::{Capability as Cap, PermissionState as State};

/// Settled requests delivered per frame; the rest wait for a later one.
const DRAIN_CAP: usize = 16;

thread_local! {
    static DISPATCHER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The backend for this build: the platform's, with the app's own microphone prompt on top
/// ([`consent`]).
fn backend() -> &'static consent::AppConsent {
    static C: std::sync::OnceLock<consent::AppConsent> = std::sync::OnceLock::new();
    C.get_or_init(|| consent::AppConsent::new(platform(), consent::Hooks::process()))
}

/// `permissions.json` in the app's storage folder, where its answers to the prompt are kept.
/// `None`, or never called: for this run only.
pub fn set_decisions_file(path: Option<std::path::PathBuf>) {
    backend().set_decisions_file(path);
}

/// Forgets the stored answer for `name`, or for every capability with a prompt when `None`, so
/// the next `getUserMedia` asks again. Returns the names forgotten.
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

/// Every capability's four-state status, the stored answers and their file, as JSON.
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

/// Where the user re-enables a capability the platform refused, for the refusal message; `None`
/// where there is no such page. macOS screen recording also needs a relaunch (TCC prompts once).
pub fn refusal_hint(cap: Capability) -> Option<String> {
    #[cfg(target_os = "macos")]
    match cap {
        Capability::DisplayCapture => return Some(screen_recording_hint(&consent::app_name())),
        _ => {
            if let Some(media) = tcc::media(cap) {
                return Some(mac_privacy_hint(&consent::app_name(), media.page, media.anchor));
            }
        }
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
fn mac_privacy_hint(app: &str, page: &str, anchor: &str) -> String {
    format!(
        "Turn on {app} in System Settings › Privacy & Security › {page} \
         (x-apple.systempreferences:com.apple.preference.security?{anchor})."
    )
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_hint(page: &str, device: &str, uri: &str) -> String {
    format!(
        "Turn on \"{page} access\" and \"Let desktop apps access your {device}\" in \
         Settings › Privacy & security › {page} (ms-settings:{uri})."
    )
}

/// The DOMException `(name, message)` for a refused `display-capture`.
pub fn display_capture_refusal() -> (&'static str, String) {
    capture_refusal(Capability::DisplayCapture)
}

/// The DOMException `(name, message)` for a denied capture of `cap`: Chrome's "Permission denied
/// by system" plus the [`refusal_hint`] when the platform refused, "Permission denied" when the
/// user blocked it in this app's prompt.
pub fn capture_refusal(cap: Capability) -> (&'static str, String) {
    refusal_by(api::host(), cap)
}

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

pub fn status(cap: Capability) -> PermissionState {
    state_of(cap)
}

/// Blocks a worker thread (never the VM's) until `display-capture` is decided, asking if nobody
/// has. Without a host nothing is refused.
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

/// Whether a request about `cap` is still waiting for its answer; a `prompt` with nothing in
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

/// `cap`'s state once it is no longer `prompt`, requesting it first if it is; `prompt` when the
/// request was dismissed or `timeout` passes.
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

host_fn!(fn vn_permissions_platform(_g, _args) {
    let name = api::host().map(|h| h.platform()).unwrap_or("this platform");
    JsRuntime::with(|rt| rt.ctx().string(name).0)
});

// `__vn_permissions_query(name) -> string`: the four-state answer, or `""` for an unknown name
// (`navigator.permissions.query` already delivers the TypeError).
host_fn!(fn vn_permissions_query(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let state = Capability::from_web_name(&name).map(state_of).map(PermissionState::as_str).unwrap_or("");
    JsRuntime::with(|rt| rt.ctx().string(state).0)
});

// `__vn_permissions_list() -> json`: the whole table in one call. `deniedBySystem` tells a
// platform denial (settings is the way back) from the user's Block in this app's prompt.
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

// `__vn_permissions_request(name) -> json`: `{"state":..}` settled now, `{"token":n}` settled
// later through the dispatcher, `{"error":..}` for an unknown name.
host_fn!(fn vn_permissions_request(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let json = request_json(&name, api::host());
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

/// The name is the page's own string: escaped into the error, never spliced.
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

host_fn!(fn vn_permissions_hint(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let hint = Capability::from_web_name(&name).and_then(refusal_hint).unwrap_or_default();
    JsRuntime::with(|rt| rt.ctx().string(&hint).0)
});

host_fn!(fn vn_permissions_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    // A reload sets its own dispatcher; release the old one.
    if let Some(old) = replace_dispatcher(f.0) {
        unsafe { vn_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    Value::undefined().0
});

/// Installs `new`; returns the previous dispatcher, if another value, for unprotecting.
fn replace_dispatcher(new: u64) -> Option<u64> {
    let old = DISPATCHER.with(|d| d.replace(new));
    (old != 0 && old != new).then_some(old)
}

fn state_of(cap: Capability) -> PermissionState {
    api::host().map(|h| h.status(cap)).unwrap_or(PermissionState::Unsupported)
}

/// Whether `request()` would do anything: false when granted or unsupported.
fn can_request_state(state: PermissionState) -> bool {
    !matches!(state, PermissionState::Granted | PermissionState::Unsupported)
}

/// Installs the platform backend into redjs and registers the extension host fns.
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

/// Settles finished requests into JS, inside the frame's Commit publication.
///
/// `PermissionStatus` change events are dispatched by redjs's own pump; this drains a different
/// queue.
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
        // The awaiting `.then()` lands in this same Commit publication.
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
        // It must not claim a change subscription it lacks.
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
        assert_eq!(h.status(Capability::DisplayCapture).web_state(), "prompt");
        assert_eq!(h.status(Capability::Notifications).web_state(), "denied");
    }

    #[test]
    fn a_request_for_something_already_granted_settles_without_asking() {
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
        // Re-setting the same state is not an event, or Windows would emit one `change` per
        // microphone use by any app.
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
        // Already decided: read, not asked again (a refused request would reopen System Settings).
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Denied);
        assert!(h.drain_settled().is_empty());
        let h = StubHost::scripted("test", &[(cap, PermissionState::Granted)]);
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Granted);
        let h = StubHost::scripted("test", &[(cap, PermissionState::Prompt)]);
        assert_eq!(await_decision(&h, cap, Duration::from_secs(5), &pending), PermissionState::Prompt);

        struct Stuck;
        impl PermissionHost for Stuck {
            fn platform(&self) -> &'static str { "test" }
            fn status(&self, _: Capability) -> PermissionState { PermissionState::Prompt }
            fn request(&self, _: Capability) -> RequestOutcome { RequestOutcome::Pending(1) }
        }
        assert_eq!(await_decision(&Stuck, cap, Duration::from_millis(60), &pending), PermissionState::Prompt, "bounded");
        // Dismissed: `prompt` with nothing in flight answers now, not after the timeout.
        let t = std::time::Instant::now();
        assert_eq!(await_decision(&Stuck, cap, Duration::from_secs(60), &|_| false), PermissionState::Prompt);
        assert!(t.elapsed() < Duration::from_secs(5), "a dismissal does not wait out the timeout");
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
        let mic = tcc::media(Capability::Microphone).unwrap();
        assert_eq!(
            mac_privacy_hint("Argon", mic.page, mic.anchor),
            "Turn on Argon in System Settings › Privacy & Security › Microphone \
             (x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone)."
        );
        let camera = tcc::media(Capability::Camera).unwrap();
        assert_eq!(
            mac_privacy_hint("Argon", camera.page, camera.anchor),
            "Turn on Argon in System Settings › Privacy & Security › Camera \
             (x-apple.systempreferences:com.apple.preference.security?Privacy_Camera)."
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
        {
            assert!(refusal_hint(Capability::Microphone).unwrap().contains("Privacy_Microphone"));
            assert!(refusal_hint(Capability::Camera).unwrap().contains("Privacy_Camera"));
        }
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
        assert!(!can_request_state(PermissionState::Granted));
        assert!(!can_request_state(PermissionState::Unsupported));
        assert!(can_request_state(PermissionState::Denied));
        assert!(can_request_state(PermissionState::Prompt));
    }
}
