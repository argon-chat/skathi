//! Platform backends for `navigator.permissions` and Skathi's `appPermissions`.
//! `appPermissions.request(name)` is an extension (the web has no standalone request) and adds a
//! fourth state, `unsupported`.

use sk_runtime::redjs::webapi::permissions::{
    self as api, Capability, PermissionHost, PermissionState, RequestOutcome,
};
use sk_runtime::JsRuntime;

pub mod consent;
pub mod decisions;
mod js;
pub mod stub;

pub use js::request as request_promise;

#[cfg(target_os = "windows")]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;

#[cfg(any(target_os = "macos", test))]
mod tcc;

pub use consent::{set_app_name, set_asker, set_camera_capture, Asker, Question};
pub use sk_runtime::redjs::webapi::permissions::{Capability as Cap, PermissionState as State};

const DRAIN_CAP: usize = 16;

/// The backend for this build: the platform's, with the app's own prompt on top ([`consent`]).
fn backend() -> &'static consent::AppConsent {
    static C: std::sync::OnceLock<consent::AppConsent> = std::sync::OnceLock::new();
    C.get_or_init(|| consent::AppConsent::new(platform(), consent::Hooks::process()))
}

/// `permissions.json` in the app's storage folder, where its answers are kept; `None`: this run only.
pub fn set_decisions_file(path: Option<std::path::PathBuf>) {
    backend().set_decisions_file(path);
}

/// Forgets the stored answer for `name` (or every prompting capability when `None`); returns the names forgotten.
pub fn reset(name: Option<&str>) -> Result<Vec<&'static str>, String> {
    let cap = match name {
        None => None,
        Some(n) => Some(
            Capability::from_web_name(n)
                .ok_or_else(|| format!("{n:?} is not a permission name"))?,
        ),
    };
    if let Some(c) = cap {
        if !consent::prompts(c) {
            return Err(format!(
                "{n} has no prompt of this app's to reset: its state is the platform's",
                n = c.web_name()
            ));
        }
    }
    Ok(backend()
        .reset(cap)
        .into_iter()
        .map(Capability::web_name)
        .collect())
}

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
        s.push_str(&format!(
            "{{\"name\":\"{}\",\"decision\":\"{}\",\"at\":{}}}",
            cap.web_name(),
            e.decision.as_str(),
            e.at
        ));
    }
    let file = b.decisions_file().map(|p| p.display().to_string());
    s.push_str(&format!(
        "],\"file\":{}}}",
        file.map(|f| json_string(&f))
            .unwrap_or_else(|| "null".into())
    ));
    s
}

/// Where the user re-enables a capability the platform refused; macOS screen recording also needs a relaunch.
pub fn refusal_hint(cap: Capability) -> Option<String> {
    #[cfg(target_os = "macos")]
    match cap {
        Capability::DisplayCapture => return Some(screen_recording_hint(&consent::app_name())),
        _ => {
            if let Some(media) = tcc::media(cap) {
                return Some(mac_privacy_hint(
                    &consent::app_name(),
                    media.page,
                    media.anchor,
                ));
            }
        }
    }
    #[cfg(target_os = "windows")]
    match cap {
        Capability::Microphone => {
            return Some(windows_hint(
                "Microphone",
                "microphone",
                "privacy-microphone",
            ))
        }
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

pub fn display_capture_refusal() -> (&'static str, String) {
    capture_refusal(Capability::DisplayCapture)
}

/// The DOMException `(name, message)` for a denied capture of `cap`.
pub fn capture_refusal(cap: Capability) -> (&'static str, String) {
    refusal_by(api::host(), cap)
}

pub fn refusal_by(host: Option<&dyn PermissionHost>, cap: Capability) -> (&'static str, String) {
    if (consent::Hooks::process().forbidden)() {
        return (
            "NotAllowedError",
            "Permission denied: media capture is switched off for this process.".into(),
        );
    }
    let system = host.is_none_or(|h| h.denied_by_system(cap));
    (
        "NotAllowedError",
        refusal_message(system, refusal_hint(cap)),
    )
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

/// Blocks a worker thread (never the VM's) until `display-capture` is decided, asking if nobody has.
pub fn require_display_capture() -> Result<(), (&'static str, String)> {
    let Some(host) = api::host() else {
        return Ok(());
    };
    match await_decision(
        host,
        Capability::DisplayCapture,
        std::time::Duration::from_secs(120),
        &in_flight,
    ) {
        PermissionState::Granted | PermissionState::Unsupported => Ok(()),
        PermissionState::Denied => Err(display_capture_refusal()),
        PermissionState::Prompt => Err(("NotAllowedError", "Permission dismissed".into())),
    }
}

/// Whether a request about `cap` is still waiting; a `prompt` with nothing in flight was dismissed.
pub fn in_flight(cap: Capability) -> bool {
    backend().in_flight(cap)
}

/// Blocks a worker thread until a request already made about `cap` is decided.
pub fn wait_for_decision(
    cap: Capability,
    timeout: std::time::Duration,
    cancelled: &dyn Fn() -> bool,
) -> PermissionState {
    let Some(host) = api::host() else {
        return PermissionState::Unsupported;
    };
    wait(
        host,
        cap,
        std::time::Instant::now() + timeout,
        &in_flight,
        cancelled,
    )
}

/// `cap`'s state once no longer `prompt`, requesting it first; `prompt` if dismissed or `timeout` passes.
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
        // Read in-flight first: an answer settles its waiters after it is stored.
        let waiting = in_flight(cap);
        match host.status(cap) {
            PermissionState::Prompt
                if waiting && !cancelled() && std::time::Instant::now() < deadline =>
            {
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

/// `(state, canRequest, deniedBySystem)` for `cap`.
pub fn describe(cap: Capability) -> (PermissionState, bool, bool) {
    let state = state_of(cap);
    let by_system =
        state == PermissionState::Denied && api::host().is_some_and(|h| h.denied_by_system(cap));
    (state, can_request_state(state), by_system)
}

fn state_of(cap: Capability) -> PermissionState {
    api::host()
        .map(|h| h.status(cap))
        .unwrap_or(PermissionState::Unsupported)
}

fn can_request_state(state: PermissionState) -> bool {
    !matches!(
        state,
        PermissionState::Granted | PermissionState::Unsupported
    )
}

pub fn install(rt: &JsRuntime) {
    let host: &'static dyn PermissionHost = backend();
    api::set_host(host);
    unsafe { js::install(rt.raw()) };
}

/// Settles finished requests into JS, inside the frame's Commit publication.
pub fn pump() {
    if !js::installed() || api::host().is_none() {
        return;
    }
    // Not the host's `drain_settled`: that queue also carries requests redjs settles itself.
    let settled = api::take_settled_up_to(DRAIN_CAP);
    if settled.is_empty() {
        return;
    }
    JsRuntime::with(|rt| {
        if unsafe { js::settle(rt.raw(), settled) } {
            rt.ctx().drain_microtasks();
        }
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
        let h = StubHost::scripted(
            "test",
            &[(Capability::Microphone, PermissionState::Granted)],
        )
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
        assert!(
            h.drain_settled().is_empty(),
            "a settlement is delivered once"
        );
    }

    #[test]
    fn a_state_change_queues_exactly_one_change_and_only_when_it_really_changed() {
        // Re-setting the same state is not an event, or Windows would emit one `change` per microphone use.
        let h = StubHost::scripted(
            "test",
            &[(Capability::Microphone, PermissionState::Granted)],
        );
        assert_eq!(h.drain_changes(), Some(vec![]));
        h.set(Capability::Microphone, PermissionState::Granted);
        assert_eq!(
            h.drain_changes(),
            Some(vec![]),
            "a no-op write is not a change"
        );
        h.set(Capability::Microphone, PermissionState::Denied);
        assert_eq!(h.drain_changes(), Some(vec![Capability::Microphone]));
        assert_eq!(
            h.drain_changes(),
            Some(vec![]),
            "a change is delivered once"
        );
    }

    #[test]
    fn a_worker_waits_for_a_decision_and_asks_only_from_prompt() {
        use std::time::Duration;
        let cap = Capability::DisplayCapture;
        let pending = |_: Capability| true;
        let h = StubHost::scripted("test", &[(cap, PermissionState::Prompt)])
            .granting(PermissionState::Denied);
        assert_eq!(
            await_decision(&h, cap, Duration::from_secs(5), &pending),
            PermissionState::Denied
        );
        assert_eq!(h.drain_settled().len(), 1, "asked exactly once");
        assert_eq!(
            await_decision(&h, cap, Duration::from_secs(5), &pending),
            PermissionState::Denied
        );
        assert!(h.drain_settled().is_empty());
        let h = StubHost::scripted("test", &[(cap, PermissionState::Granted)]);
        assert_eq!(
            await_decision(&h, cap, Duration::from_secs(5), &pending),
            PermissionState::Granted
        );
        let h = StubHost::scripted("test", &[(cap, PermissionState::Prompt)]);
        assert_eq!(
            await_decision(&h, cap, Duration::from_secs(5), &pending),
            PermissionState::Prompt
        );

        struct Stuck;
        impl PermissionHost for Stuck {
            fn platform(&self) -> &'static str {
                "test"
            }
            fn status(&self, _: Capability) -> PermissionState {
                PermissionState::Prompt
            }
            fn request(&self, _: Capability) -> RequestOutcome {
                RequestOutcome::Pending(1)
            }
        }
        assert_eq!(
            await_decision(&Stuck, cap, Duration::from_millis(60), &pending),
            PermissionState::Prompt,
            "bounded"
        );
        let t = std::time::Instant::now();
        assert_eq!(
            await_decision(&Stuck, cap, Duration::from_secs(60), &|_| false),
            PermissionState::Prompt
        );
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "a dismissal does not wait out the timeout"
        );
        let start = std::time::Instant::now();
        let deadline = start + Duration::from_secs(60);
        assert_eq!(
            wait(&Stuck, cap, deadline, &pending, &|| true),
            PermissionState::Prompt
        );
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
        assert_eq!(
            refusal_hint(Capability::DisplayCapture),
            None,
            "no other platform adds one"
        );
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
            assert!(refusal_hint(Capability::Microphone)
                .unwrap()
                .contains("ms-settings:privacy-microphone"));
            assert!(refusal_hint(Capability::Camera)
                .unwrap()
                .contains("ms-settings:privacy-webcam"));
        }
        #[cfg(target_os = "macos")]
        {
            assert!(refusal_hint(Capability::Microphone)
                .unwrap()
                .contains("Privacy_Microphone"));
            assert!(refusal_hint(Capability::Camera)
                .unwrap()
                .contains("Privacy_Camera"));
        }
        assert_eq!(refusal_hint(Capability::Geolocation), None);
    }

    #[test]
    fn a_refusal_is_the_systems_with_its_hint_or_the_users_without() {
        assert_eq!(
            refusal_message(true, Some("Turn it on.".into())),
            "Permission denied by system. Turn it on."
        );
        assert_eq!(refusal_message(true, None), "Permission denied by system");
        assert_eq!(
            refusal_message(false, Some("Turn it on.".into())),
            "Permission denied"
        );
    }

    #[test]
    fn can_request_is_false_for_granted_and_for_unsupported() {
        assert!(!can_request_state(PermissionState::Granted));
        assert!(!can_request_state(PermissionState::Unsupported));
        assert!(can_request_state(PermissionState::Denied));
        assert!(can_request_state(PermissionState::Prompt));
    }
}
