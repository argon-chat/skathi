//! Native macOS microphone consent through AVFoundation and TCC.
//!
//! AVFoundation owns this app's permission prompt. Its completion runs on an
//! arbitrary queue and only writes to a Rust queue; the normal permission pump
//! resolves JS promises and emits change events on the VM thread. Other macOS
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

#[derive(Default)]
struct Requests {
    next: u32,
    asking: bool,
    waiting: Vec<u32>,
    settled: Vec<(u32, PermissionState)>,
    changes: Vec<Capability>,
}

impl Requests {
    fn begin(&mut self) -> (u32, bool) {
        self.next = self.next.wrapping_add(1).max(1);
        self.waiting.push(self.next);
        let show = !self.asking;
        self.asking = true;
        (self.next, show)
    }

    fn complete(&mut self, state: PermissionState) {
        self.asking = false;
        self.settled
            .extend(self.waiting.drain(..).map(|token| (token, state)));
        if !self.changes.contains(&Capability::Microphone) {
            self.changes.push(Capability::Microphone);
        }
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
        cap == Capability::Microphone
    }

    fn status(&self, cap: Capability) -> PermissionState {
        match cap {
            Capability::Microphone => microphone_state(),
            _ => PermissionState::Unsupported,
        }
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        if cap != Capability::Microphone {
            return RequestOutcome::Settled(self.status(cap));
        }
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
                let _ = std::process::Command::new("/usr/bin/open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
            return RequestOutcome::Settled(state);
        }
        let (token, show) = self
            .requests()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .begin();
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
                    .complete(state);
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
        let (first, show) = requests.begin();
        assert!(show);
        let (second, show) = requests.begin();
        assert!(!show);
        assert_ne!(first, second);
        requests.complete(PermissionState::Granted);
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
            requests.begin().1,
            "a completed request does not leave a prompt in flight"
        );
    }
}
