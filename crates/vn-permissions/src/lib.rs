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
//!   packages/vue-native/src/runtime.js ──__vn_permissions_*────┤  (request, 4 states)
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

pub mod stub;

#[cfg(target_os = "windows")]
pub mod win;

#[cfg(target_os = "macos")]
pub mod mac;

pub use vn_runtime::redjs::webapi::permissions::{Capability as Cap, PermissionState as State};

/// How many settled requests one frame may deliver. A user cannot answer more
/// than a handful of dialogs per frame; the cap only exists so that a
/// misbehaving backend cannot lengthen a frame without bound.
const DRAIN_CAP: usize = 16;

thread_local! {
    /// The one protected JS callback, exactly as `vn-voice`'s dispatcher works.
    static DISPATCHER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The backend for this build.
///
/// A `&'static` because [`api::set_host`] stores one for the life of the
/// process; there is exactly one OS under one process and no reason to make it
/// swappable at runtime.
fn backend() -> &'static dyn PermissionHost {
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
host_fn!(fn vn_permissions_list(_g, _args) {
    let mut s = String::from("[");
    for (i, cap) in Capability::ALL.iter().copied().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"name\":\"{}\",\"state\":\"{}\",\"canRequest\":{}}}",
            cap.web_name(),
            state_of(cap).as_str(),
            can_request(cap),
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
    let json = match Capability::from_web_name(&name) {
        None => format!("{{\"error\":\"{name} is not a permission name\"}}"),
        Some(cap) => match api::host() {
            None => format!("{{\"state\":\"{}\"}}", PermissionState::Unsupported.as_str()),
            Some(h) => match h.request(cap) {
                RequestOutcome::Settled(st) => format!("{{\"state\":\"{}\"}}", st.as_str()),
                RequestOutcome::Pending(token) => format!("{{\"token\":{token}}}"),
            },
        },
    };
    JsRuntime::with(|rt| rt.ctx().string(&json).0)
});

host_fn!(fn vn_permissions_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    DISPATCHER.with(|d| d.set(f.0));
    Value::undefined().0
});

fn state_of(cap: Capability) -> PermissionState {
    api::host().map(|h| h.status(cap)).unwrap_or(PermissionState::Unsupported)
}

/// Would `request()` do anything here? Drives whether the demo page's button is
/// enabled — a button that is guaranteed to be a no-op should not be offered.
fn can_request(cap: Capability) -> bool {
    can_request_state(state_of(cap))
}

/// Split out from [`can_request`] so the rule is testable without a host being
/// installed process-wide.
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
    api::set_host(backend());
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_permissions_platform", 0, vn_permissions_platform as *const _),
        ("__vn_permissions_query", 1, vn_permissions_query as *const _),
        ("__vn_permissions_list", 0, vn_permissions_list as *const _),
        ("__vn_permissions_request", 1, vn_permissions_request as *const _),
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
    let Some(host) = api::host() else { return };
    let mut settled = host.drain_settled();
    if settled.is_empty() {
        return;
    }
    settled.truncate(DRAIN_CAP);

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
