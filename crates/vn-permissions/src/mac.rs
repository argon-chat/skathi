//! Native macOS consent through TCC: the microphone (AVFoundation) and
//! screen recording (CoreGraphics).
//!
//! AVFoundation owns this app's microphone prompt. Its completion runs on an
//! arbitrary queue and only writes to a Rust queue; the normal permission pump
//! resolves JS promises and emits change events on the VM thread.
//!
//! Screen recording has no "not determined": `CGPreflightScreenCaptureAccess`
//! answers granted or not, and `CGRequestScreenCaptureAccess` shows the system
//! prompt only the first time an app ever asks. A grant made in System Settings
//! takes effect after the app is relaunched. So a screen request that ends
//! without access reads `denied` for the rest of the run, and recovery is
//! System Settings plus a relaunch (docs/PERMISSIONS.md §6.1). Other macOS
//! capabilities remain unsupported until their own platform backends exist.

use std::sync::{Arc, Mutex, OnceLock};

use block2::RcBlock;
use objc2::{class, msg_send, runtime::Bool};
use objc2_foundation::{ns_string, NSBundle, NSString};
use vn_runtime::redjs::webapi::permissions::{
    Capability, PermissionHost, PermissionState, RequestOutcome,
};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    // An AVMediaType is an NSString constant owned by AVFoundation.
    static AVMediaTypeAudio: *const NSString;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    // macOS 10.15+, C `bool`. Preflight never prompts. Request prompts at most
    // once per app (TCC remembers a refusal) and reports whether access is
    // granted.
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[derive(Default)]
struct Requests {
    next: u32,
    /// Capabilities whose request is in flight.
    asking: Vec<Capability>,
    waiting: Vec<(u32, Capability)>,
    settled: Vec<(u32, PermissionState)>,
    changes: Vec<Capability>,
    /// Screen recording was requested in this run and not granted: what
    /// tells a refusal from a capability nobody has asked about yet.
    screen_refused: bool,
    /// The screen-recording state last reported, so that a read which sees
    /// it move (a grant before relaunch, macOS 15's periodic re-ask) queues
    /// a `change`.
    screen_seen: Option<PermissionState>,
}

impl Requests {
    fn begin(&mut self, cap: Capability) -> (u32, bool) {
        self.next = self.next.wrapping_add(1).max(1);
        self.waiting.push((self.next, cap));
        let show = !self.asking.contains(&cap);
        if show {
            self.asking.push(cap);
        }
        (self.next, show)
    }

    fn complete(&mut self, cap: Capability, state: PermissionState) {
        self.asking.retain(|c| *c != cap);
        if cap == Capability::DisplayCapture {
            self.screen_refused = state != PermissionState::Granted;
            self.screen_seen = Some(state);
        }
        let mut done = Vec::new();
        self.waiting.retain(|&(token, c)| {
            if c == cap {
                done.push((token, state));
            }
            c != cap
        });
        self.settled.extend(done);
        self.changed(cap);
    }

    fn changed(&mut self, cap: Capability) {
        if !self.changes.contains(&cap) {
            self.changes.push(cap);
        }
    }

    /// One screen-recording read, `granted` being the preflight's answer.
    fn screen(&mut self, granted: bool) -> PermissionState {
        let state = screen_state(granted, self.screen_refused);
        if self.screen_seen.is_some_and(|seen| seen != state) {
            self.changed(Capability::DisplayCapture);
        }
        self.screen_seen = Some(state);
        state
    }
}

pub struct MacHost {
    requests: OnceLock<Arc<Mutex<Requests>>>,
}

impl MacHost {
    pub const fn new() -> MacHost {
        MacHost {
            requests: OnceLock::new(),
        }
    }

    fn requests(&self) -> &Arc<Mutex<Requests>> {
        self.requests
            .get_or_init(|| Arc::new(Mutex::new(Requests::default())))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Requests> {
        self.requests().lock().unwrap_or_else(|p| p.into_inner())
    }

    fn screen_capture_state(&self) -> PermissionState {
        // SAFETY: no arguments; CoreGraphics documents no thread requirement.
        let granted = unsafe { CGPreflightScreenCaptureAccess() };
        self.lock().screen(granted)
    }

    fn request_microphone(&self) -> RequestOutcome {
        if !microphone_usage_description_present() {
            static WARN: std::sync::Once = std::sync::Once::new();
            WARN.call_once(|| eprintln!(
                "[permissions] microphone access requires a macOS .app with a nonempty NSMicrophoneUsageDescription in Info.plist"
            ));
            return RequestOutcome::Settled(PermissionState::Denied);
        }
        let state = microphone_state();
        if state != PermissionState::Prompt {
            if state == PermissionState::Denied {
                // TCC cannot re-prompt after a denial. A user's explicit
                // request opens its recovery page without blocking the VM.
                open_privacy_pane("Privacy_Microphone");
            }
            return RequestOutcome::Settled(state);
        }
        let (token, show) = self.lock().begin(Capability::Microphone);
        if show {
            let requests = Arc::clone(self.requests());
            let completion = RcBlock::new(move |granted: Bool| {
                let state = if granted.as_bool() {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                };
                requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .complete(Capability::Microphone, state);
            });
            // SAFETY: the framework copies the escaping completion block.
            // It captures only an Arc/Mutex queue, never a JS value or UI
            // object. The method is asynchronous and may call back on any queue.
            unsafe {
                let _: () = msg_send![class!(AVCaptureDevice),
                    requestAccessForMediaType: &*AVMediaTypeAudio,
                    completionHandler: &*completion
                ];
            }
        }
        RequestOutcome::Pending(token)
    }

    fn request_screen_capture(&self) -> RequestOutcome {
        match self.screen_capture_state() {
            PermissionState::Prompt => {}
            PermissionState::Denied => {
                // Asked already in this run: TCC will not prompt again, and a
                // grant needs a relaunch, so open the pane and settle now.
                open_privacy_pane("Privacy_ScreenCapture");
                return RequestOutcome::Settled(PermissionState::Denied);
            }
            state => return RequestOutcome::Settled(state),
        }
        let (token, show) = self.lock().begin(Capability::DisplayCapture);
        if show {
            let requests = Arc::clone(self.requests());
            let ask = move || {
                // SAFETY: no arguments. Shows the system prompt the first
                // time this app asks. It can return before the user answers;
                // a grant made there only takes effect after a relaunch.
                let granted = unsafe { CGRequestScreenCaptureAccess() };
                let state = if granted {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                };
                requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .complete(Capability::DisplayCapture, state);
            };
            // Off the calling thread, which may be the VM's: the request goes
            // through tccd and the VM must not wait on it.
            if std::thread::Builder::new()
                .name("vn-screen-consent".into())
                .spawn(ask)
                .is_err()
            {
                self.lock()
                    .complete(Capability::DisplayCapture, PermissionState::Denied);
            }
        }
        RequestOutcome::Pending(token)
    }
}

/// Opens a Privacy & Security pane of System Settings without blocking.
fn open_privacy_pane(anchor: &str) {
    let _ = std::process::Command::new("/usr/bin/open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{anchor}"
        ))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn microphone_usage_description_present() -> bool {
    NSBundle::mainBundle()
        .objectForInfoDictionaryKey(ns_string!("NSMicrophoneUsageDescription"))
        .and_then(|value| value.downcast::<NSString>().ok())
        .is_some_and(|text| !text.to_string().trim().is_empty())
}

fn authorization_state(value: isize) -> PermissionState {
    // AVAuthorizationStatus: notDetermined=0, restricted=1, denied=2,
    // authorized=3. A future unknown value cannot be treated as a grant.
    match value {
        0 => PermissionState::Prompt,
        3 => PermissionState::Granted,
        _ => PermissionState::Denied,
    }
}

/// Screen recording's state from the preflight and whether this run's
/// request was refused. Without a refusal, a missing grant is `prompt`:
/// TCC cannot tell "never asked" from "said no" before the app asks.
fn screen_state(granted: bool, refused: bool) -> PermissionState {
    if granted {
        PermissionState::Granted
    } else if refused {
        PermissionState::Denied
    } else {
        PermissionState::Prompt
    }
}

fn microphone_state() -> PermissionState {
    if !microphone_usage_description_present() {
        // TCC terminates an app that requests or captures without this key.
        // Refuse the capture gate before either API can be called, including
        // bare cargo-run hosts which do not have an application Info.plist.
        return PermissionState::Denied;
    }
    // SAFETY: AVCaptureDevice and AVMediaTypeAudio are supplied by the linked
    // framework. The SDK declares this class method with NSInteger result.
    let status: isize = unsafe {
        msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: &*AVMediaTypeAudio]
    };
    authorization_state(status)
}

impl PermissionHost for MacHost {
    fn platform(&self) -> &'static str {
        "macOS"
    }

    fn has_native_prompt(&self, cap: Capability) -> bool {
        matches!(cap, Capability::Microphone | Capability::DisplayCapture)
    }

    fn status(&self, cap: Capability) -> PermissionState {
        match cap {
            Capability::Microphone => microphone_state(),
            Capability::DisplayCapture => self.screen_capture_state(),
            _ => PermissionState::Unsupported,
        }
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        match cap {
            Capability::Microphone => self.request_microphone(),
            Capability::DisplayCapture => self.request_screen_capture(),
            _ => RequestOutcome::Settled(self.status(cap)),
        }
    }

    fn drain_settled(&self) -> Vec<(u32, PermissionState)> {
        self.requests
            .get()
            .map(|q| std::mem::take(&mut q.lock().unwrap_or_else(|p| p.into_inner()).settled))
            .unwrap_or_default()
    }

    fn drain_changes(&self) -> Option<Vec<Capability>> {
        Some(
            self.requests
                .get()
                .map(|q| std::mem::take(&mut q.lock().unwrap_or_else(|p| p.into_inner()).changes))
                .unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avfoundation_statuses_do_not_promote_restrictions_or_unknowns() {
        assert_eq!(authorization_state(0), PermissionState::Prompt);
        assert_eq!(authorization_state(1), PermissionState::Denied);
        assert_eq!(authorization_state(2), PermissionState::Denied);
        assert_eq!(authorization_state(3), PermissionState::Granted);
        assert_eq!(authorization_state(99), PermissionState::Denied);
    }

    #[test]
    fn concurrent_requests_share_one_prompt_and_settle_every_token_once() {
        let mut requests = Requests::default();
        let (first, show) = requests.begin(Capability::Microphone);
        assert!(show);
        let (second, show) = requests.begin(Capability::Microphone);
        assert!(!show);
        assert_ne!(first, second);
        requests.complete(Capability::Microphone, PermissionState::Granted);
        assert_eq!(
            std::mem::take(&mut requests.settled),
            vec![
                (first, PermissionState::Granted),
                (second, PermissionState::Granted),
            ]
        );
        assert!(requests.settled.is_empty());
        assert_eq!(requests.changes, vec![Capability::Microphone]);
        assert!(
            requests.begin(Capability::Microphone).1,
            "a completed request does not leave a prompt in flight"
        );
    }

    #[test]
    fn screen_recording_is_prompt_until_a_request_in_this_run_is_refused() {
        assert_eq!(screen_state(true, false), PermissionState::Granted);
        assert_eq!(screen_state(true, true), PermissionState::Granted, "a grant wins over an earlier refusal");
        assert_eq!(screen_state(false, false), PermissionState::Prompt);
        assert_eq!(screen_state(false, true), PermissionState::Denied);

        let mut requests = Requests::default();
        assert_eq!(requests.screen(false), PermissionState::Prompt);
        let (mic, _) = requests.begin(Capability::Microphone);
        let (screen, show) = requests.begin(Capability::DisplayCapture);
        assert!(show, "the screen prompt is not the microphone's");
        requests.complete(Capability::DisplayCapture, PermissionState::Denied);
        assert_eq!(std::mem::take(&mut requests.settled), vec![(screen, PermissionState::Denied)]);
        assert_eq!(requests.waiting, vec![(mic, Capability::Microphone)], "the microphone waiter stays");
        assert_eq!(std::mem::take(&mut requests.changes), vec![Capability::DisplayCapture]);
        assert_eq!(requests.screen(false), PermissionState::Denied);
        assert!(requests.changes.is_empty(), "a read that agrees is not a change");
        // Granted in System Settings and visible to the preflight.
        assert_eq!(requests.screen(true), PermissionState::Granted);
        assert_eq!(std::mem::take(&mut requests.changes), vec![Capability::DisplayCapture]);
    }
}
