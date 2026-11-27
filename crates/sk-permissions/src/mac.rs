//! Native macOS consent through TCC: microphone and camera (AVFoundation, `tcc::media`) and
//! screen recording (CoreGraphics).
//!
//! Screen recording has no "not determined": a grant made in System Settings takes effect after a
//! relaunch, so a refused screen request reads `denied` for the rest of the run.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::{class, msg_send, runtime::Bool};
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
};
use sk_runtime::redjs::webapi::permissions::{
    Capability, PermissionHost, PermissionState, RequestOutcome,
};

use crate::tcc::{self, Media, Requests};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: *const NSString;
    static AVMediaTypeVideo: *const NSString;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

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

/// UserNotifications aborts when instantiated outside an application bundle.
pub fn notification_bundle_available() -> bool {
    static BUNDLED: OnceLock<bool> = OnceLock::new();
    *BUNDLED.get_or_init(|| {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
    })
}

struct NotificationSnapshot {
    generation: u64,
    completed_generation: u64,
    last_read: Option<Instant>,
    state: Option<PermissionState>,
    changed: bool,
}

static NOTIFICATION_SETTINGS: Mutex<NotificationSnapshot> = Mutex::new(NotificationSnapshot {
    generation: 0,
    completed_generation: 0,
    last_read: None,
    state: None,
    changed: false,
});

fn read_notification_settings(complete: impl Fn(PermissionState) + Send + Sync + 'static) {
    let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the center owns settings for the duration of this callback.
        let state = match unsafe { settings.as_ref() }.authorizationStatus() {
            UNAuthorizationStatus::NotDetermined => PermissionState::Prompt,
            UNAuthorizationStatus::Authorized
            | UNAuthorizationStatus::Provisional
            | UNAuthorizationStatus::Ephemeral => PermissionState::Granted,
            _ => PermissionState::Denied,
        };
        complete(state);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&completion);
}

fn record_notification_state(generation: u64, state: PermissionState) {
    let mut cache = NOTIFICATION_SETTINGS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if cache.completed_generation < generation {
        cache.completed_generation = generation;
        cache.changed |= cache.state.unwrap_or(PermissionState::Prompt) != state;
        cache.state = Some(state);
    }
}

fn begin_notification_read() -> u64 {
    let mut cache = NOTIFICATION_SETTINGS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    cache.generation = cache.generation.wrapping_add(1);
    cache.last_read = Some(Instant::now());
    cache.generation
}

/// Starts a nonprompting refresh at launch and when the user returns from System Settings.
pub fn refresh_notification_permission() {
    if notification_bundle_available() {
        let generation = begin_notification_read();
        read_notification_settings(move |state| record_notification_state(generation, state));
    }
}

/// Reads the current OS decision on the notification worker and warms the synchronous JS query.
pub fn fresh_notification_permission() -> Result<PermissionState, &'static str> {
    if !notification_bundle_available() {
        return Ok(PermissionState::Unsupported);
    }
    let generation = begin_notification_read();
    let (send, receive) = std::sync::mpsc::channel();
    read_notification_settings(move |state| {
        record_notification_state(generation, state);
        let _ = send.send(state);
    });
    receive
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Timed out reading macOS notification settings")
}

fn notification_status() -> PermissionState {
    if !notification_bundle_available() {
        return PermissionState::Unsupported;
    }
    let (generation, state) = {
        let mut cache = NOTIFICATION_SETTINGS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = cache.state.unwrap_or(PermissionState::Prompt);
        if cache
            .last_read
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return state;
        }
        cache.generation = cache.generation.wrapping_add(1);
        cache.last_read = Some(Instant::now());
        (cache.generation, state)
    };
    read_notification_settings(move |s| record_notification_state(generation, s));
    state
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
            let once = if media.cap == Capability::Camera {
                &CAMERA
            } else {
                &MICROPHONE
            };
            once.call_once(|| {
                eprintln!(
                    "[permissions] {} access requires a macOS .app with a nonempty {} in Info.plist — \
                     in a Skathi app, set sk.config.json \"macos.{}\"",
                    media.cap.web_name(),
                    media.usage_key,
                    if media.cap == Capability::Camera {
                        "cameraUsageDescription"
                    } else {
                        "microphoneUsageDescription"
                    }
                )
            });
            return RequestOutcome::Settled(PermissionState::Denied);
        }
        let state = self.media_state(media);
        if state != PermissionState::Prompt {
            if state == PermissionState::Denied {
                // TCC cannot re-prompt after a denial: open the recovery page.
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
            // SAFETY: the framework copies the escaping block, which captures only an Arc/Mutex
            // queue and may be called back on any queue.
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
                open_privacy_pane("Privacy_ScreenCapture");
                return RequestOutcome::Settled(PermissionState::Denied);
            }
            state => return RequestOutcome::Settled(state),
        }
        let (token, show) = self.lock().begin(Capability::DisplayCapture);
        if show {
            let requests = Arc::clone(self.requests());
            let ask = move || {
                // SAFETY: no arguments. Shows the system prompt the first time only.
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
            if std::thread::Builder::new()
                .name("sk-screen-consent".into())
                .spawn(ask)
                .is_err()
            {
                self.lock()
                    .complete(Capability::DisplayCapture, PermissionState::Denied);
            }
        }
        RequestOutcome::Pending(token)
    }

    fn request_notifications(&self) -> RequestOutcome {
        if !notification_bundle_available() {
            return RequestOutcome::Settled(PermissionState::Unsupported);
        }
        let (token, ask) = self.lock().begin(Capability::Notifications);
        if ask {
            let requests = Arc::clone(self.requests());
            let generation = begin_notification_read();
            read_notification_settings(move |state| {
                let finish = {
                    let requests = Arc::clone(&requests);
                    move |generation, state| {
                        record_notification_state(generation, state);
                        requests
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .complete(Capability::Notifications, state);
                    }
                };
                match state {
                    PermissionState::Prompt => {
                        let completion =
                            RcBlock::new(move |_granted: Bool, _error: *mut NSError| {
                                let generation = begin_notification_read();
                                let finish = finish.clone();
                                read_notification_settings(move |state| finish(generation, state));
                            });
                        UNUserNotificationCenter::currentNotificationCenter()
                            .requestAuthorizationWithOptions_completionHandler(
                                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                                &completion,
                            );
                    }
                    PermissionState::Denied => {
                        let _ = std::process::Command::new("/usr/bin/open")
                            .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension").spawn();
                        finish(generation, state);
                    }
                    _ => finish(generation, state),
                }
            });
        }
        RequestOutcome::Pending(token)
    }
}

fn open_privacy_pane(anchor: &str) {
    let _ = std::process::Command::new("/usr/bin/open")
        .arg(tcc::privacy_pane_url(anchor))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Whether Info.plist says why the app wants `cap`: TCC terminates an app that requests or
/// captures without it, so nothing may touch the device unless this is true.
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
        matches!(
            cap,
            Capability::Microphone | Capability::Camera | Capability::DisplayCapture
        )
    }

    fn status(&self, cap: Capability) -> PermissionState {
        match (cap, tcc::media(cap)) {
            (_, Some(media)) => self.media_state(media),
            (Capability::DisplayCapture, None) => self.screen_capture_state(),
            (Capability::Notifications, None) => notification_status(),
            _ => PermissionState::Unsupported,
        }
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        match (cap, tcc::media(cap)) {
            (_, Some(media)) => self.request_media(media),
            (Capability::DisplayCapture, None) => self.request_screen_capture(),
            (Capability::Notifications, None) => self.request_notifications(),
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
        let mut changes = self
            .requests
            .get()
            .map(|q| std::mem::take(&mut q.lock().unwrap_or_else(|p| p.into_inner()).changes))
            .unwrap_or_default();
        let mut cache = NOTIFICATION_SETTINGS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if std::mem::take(&mut cache.changed) && !changes.contains(&Capability::Notifications) {
            changes.push(Capability::Notifications);
        }
        Some(changes)
    }
}
