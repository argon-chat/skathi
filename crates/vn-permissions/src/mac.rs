//! Native macOS consent through TCC: the microphone and the camera
//! (AVFoundation) and screen recording (CoreGraphics).
//!
//! AVFoundation owns this app's microphone and camera prompts, one model for
//! both (`tcc::media`): `authorizationStatusForMediaType:` with
//! `AVMediaTypeAudio` or `AVMediaTypeVideo`. Its completion runs on an
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
use objc2_foundation::{NSBundle, NSString};
use vn_runtime::redjs::webapi::permissions::{
    Capability, PermissionHost, PermissionState, RequestOutcome,
};

use crate::tcc::{self, Media, Requests};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    // An AVMediaType is an NSString constant owned by AVFoundation.
    static AVMediaTypeAudio: *const NSString;
    static AVMediaTypeVideo: *const NSString;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    // macOS 10.15+, C `bool`. Preflight never prompts. Request prompts at most
    // once per app (TCC remembers a refusal) and reports whether access is
    // granted.
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// The AVMediaType of an AVFoundation capability.
fn media_type(media: Media) -> &'static NSString {
    // SAFETY: both constants are AVFoundation's, valid for the process.
    unsafe {
        match media.cap {
            Capability::Camera => &*AVMediaTypeVideo,
            _ => &*AVMediaTypeAudio,
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

    fn lock(&self) -> std::sync::MutexGuard<'_, Requests> {
        self.requests().lock().unwrap_or_else(|p| p.into_inner())
    }

    fn screen_capture_state(&self) -> PermissionState {
        // SAFETY: no arguments; CoreGraphics documents no thread requirement.
        let granted = unsafe { CGPreflightScreenCaptureAccess() };
        self.lock().screen(granted)
    }

    fn media_state(&self, media: Media) -> PermissionState {
        let state = media_state(media);
        self.lock().read(media.cap, state)
    }

    fn request_media(&self, media: Media) -> RequestOutcome {
        if !usage_description_present(media.cap) {
            static MICROPHONE: std::sync::Once = std::sync::Once::new();
            static CAMERA: std::sync::Once = std::sync::Once::new();
            let once = if media.cap == Capability::Camera { &CAMERA } else { &MICROPHONE };
            once.call_once(|| eprintln!(
                "[permissions] {} access requires a macOS .app with a nonempty {} in Info.plist",
                media.cap.web_name(),
                media.usage_key
            ));
            return RequestOutcome::Settled(PermissionState::Denied);
        }
        let state = self.media_state(media);
        if state != PermissionState::Prompt {
            if state == PermissionState::Denied {
                // TCC cannot re-prompt after a denial. A user's explicit
                // request opens its recovery page without blocking the VM.
                open_privacy_pane(media.anchor);
            }
            return RequestOutcome::Settled(state);
        }
        let (token, show) = self.lock().begin(media.cap);
        if show {
            let requests = Arc::clone(self.requests());
            let cap = media.cap;
            let completion = RcBlock::new(move |granted: Bool| {
                let state = if granted.as_bool() {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                };
                requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .complete(cap, state);
            });
            // SAFETY: the framework copies the escaping completion block.
            // It captures only an Arc/Mutex queue, never a JS value or UI
            // object. The method is asynchronous (it returns at once, and
            // TCC's prompt is the system's, not drawn on this thread) and
            // may call back on any queue.
            unsafe {
                let _: () = msg_send![class!(AVCaptureDevice),
                    requestAccessForMediaType: media_type(media),
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
        .arg(tcc::privacy_pane_url(anchor))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Whether the app's Info.plist says why it wants `cap` (the microphone or
/// the camera). TCC terminates an app that requests or captures without it,
/// so nothing may touch the device unless this is true; a camera backend
/// checks it too before it opens one.
pub fn usage_description_present(cap: Capability) -> bool {
    let Some(media) = tcc::media(cap) else {
        return false;
    };
    let key = NSString::from_str(media.usage_key);
    NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .is_some_and(|text| !text.to_string().trim().is_empty())
}

fn media_state(media: Media) -> PermissionState {
    if !usage_description_present(media.cap) {
        // TCC terminates an app that requests or captures without this key.
        // Refuse the capture gate before either API can be called, including
        // bare cargo-run hosts which do not have an application Info.plist.
        return PermissionState::Denied;
    }
    // SAFETY: AVCaptureDevice and the media type are supplied by the linked
    // framework. The SDK declares this class method with NSInteger result.
    let status: isize = unsafe {
        msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: media_type(media)]
    };
    tcc::authorization_state(status)
}

impl PermissionHost for MacHost {
    fn platform(&self) -> &'static str {
        "macOS"
    }

    fn has_native_prompt(&self, cap: Capability) -> bool {
        matches!(cap, Capability::Microphone | Capability::Camera | Capability::DisplayCapture)
    }

    fn status(&self, cap: Capability) -> PermissionState {
        match (cap, tcc::media(cap)) {
            (_, Some(media)) => self.media_state(media),
            (Capability::DisplayCapture, None) => self.screen_capture_state(),
            _ => PermissionState::Unsupported,
        }
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        match (cap, tcc::media(cap)) {
            (_, Some(media)) => self.request_media(media),
            (Capability::DisplayCapture, None) => self.request_screen_capture(),
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
