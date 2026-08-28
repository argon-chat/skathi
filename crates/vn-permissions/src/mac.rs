//! The macOS backend — **design only**. Every capability reports
//! `Unsupported` until somebody writes the four framework calls below.
//!
//! This file exists so the trait is proven to fit macOS *before* the code is
//! written, which is the whole point of designing the layer now: if the shape
//! needed reworking for TCC, the time to find out is while there is one backend,
//! not three. The mapping is complete and each row names the exact API. Nothing
//! here is speculative about the shape — only about the FFI.
//!
//! # The mapping
//!
//! | Capability | Status read | Request | Info.plist key |
//! |---|---|---|---|
//! | `Microphone` | `AVCaptureDevice.authorizationStatus(for: .audio)` | `AVCaptureDevice.requestAccess(for: .audio)` | `NSMicrophoneUsageDescription` |
//! | `Camera` | `AVCaptureDevice.authorizationStatus(for: .video)` | `AVCaptureDevice.requestAccess(for: .video)` | `NSCameraUsageDescription` |
//! | `DisplayCapture` | `CGPreflightScreenCaptureAccess()` | `CGRequestScreenCaptureAccess()` | — (TCC only) |
//! | `Notifications` | `UNUserNotificationCenter.getNotificationSettings().authorizationStatus` | `requestAuthorization(options:)` | — |
//! | `ClipboardRead` | `NSPasteboard.general.accessBehavior` (macOS 26+) | implicit, on first read | — |
//! | `ClipboardWrite` | always granted | n/a | — |
//! | `Geolocation` | `CLLocationManager().authorizationStatus` | `requestWhenInUseAuthorization()` | `NSLocationWhenInUseUsageDescription` |
//!
//! ## The state mapping
//!
//! `AVAuthorizationStatus` and `CLAuthorizationStatus` both fit the four states
//! without loss, which is the result that mattered:
//!
//! | macOS | [`PermissionState`] |
//! |---|---|
//! | `.notDetermined` | `Prompt` |
//! | `.authorized` / `.authorizedAlways` / `.authorizedWhenInUse` | `Granted` |
//! | `.denied` | `Denied` |
//! | `.restricted` (MDM / parental controls) | `Denied` |
//!
//! `UNAuthorizationStatus` adds `.provisional` and `.ephemeral`; both deliver
//! notifications, so both are `Granted`. `NSPasteboard.AccessBehavior` maps
//! `.alwaysAllow → Granted`, `.alwaysDeny → Denied`, `.ask → Prompt`.
//!
//! ## Where macOS is the opposite of Windows
//!
//! * **`Prompt` is the common case.** Windows never returns it; macOS returns it
//!   for every capability the app has not yet exercised. An app written against
//!   Windows alone will treat `prompt` as "not granted" and disable a feature
//!   that one call would have unlocked. That asymmetry is the reason
//!   `PermissionState::Prompt` is in the trait at all rather than being folded
//!   into `Denied` for a Windows-only shipping target.
//! * **`request()` is real and native.** No `ms-settings:` detour:
//!   `requestAccess` raises the system dialog and calls back with a `Bool`. That
//!   is what [`RequestOutcome::Pending`] plus
//!   [`PermissionHost::drain_settled`] were shaped for — the completion handler
//!   pushes `(token, state)` onto a mutex-guarded queue and the next pump
//!   settles the JS promise. No polling, no timeout.
//!   The exception is `DisplayCapture`: `CGRequestScreenCaptureAccess()` returns
//!   immediately and the grant does **not** take effect until the app is
//!   relaunched, so that one settles with the pre-restart state and must carry
//!   an advisory saying so.
//! * **The prompt fires once, ever.** A `.denied` capability cannot be
//!   re-prompted; the only recovery is System Settings, which is exactly the
//!   Windows flow. The URLs are
//!   `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`
//!   (`…?Privacy_Camera`, `…?Privacy_ScreenCapture`, `…?Privacy_Location`) and
//!   `x-apple.systempreferences:com.apple.Notifications-Settings.extension`. So
//!   `request()` on macOS is `notDetermined → native dialog`, `denied → open
//!   Settings`, i.e. a superset of the Windows behaviour rather than a different
//!   shape.
//!
//! ## Two things that will bite whoever implements this
//!
//! * **A missing `Info.plist` usage string is a crash, not a denial.** Calling
//!   `AVCaptureDevice.requestAccess(for: .audio)` without
//!   `NSMicrophoneUsageDescription` terminates the process with
//!   `TCC: This app has crashed because it attempted to access privacy-sensitive
//!   data without a usage description`. So the macOS host must be shipped
//!   together with the plist keys, and a bare `cargo run` binary — no `.app`
//!   bundle — cannot use any of this. `UNUserNotificationCenter.current()`
//!   likewise throws for an unbundled executable.
//! * **TCC keys on code identity, not on path.** Grants live in
//!   `~/Library/Application Support/com.apple.TCC/TCC.db` (per user) and
//!   `/Library/…/TCC.db` (system, SIP-protected — unreadable and unwritable even
//!   as root, so there is no registry-style read to fall back on; the framework
//!   calls are the only way in). Rows are keyed by bundle id plus code-signing
//!   identity, which means an ad-hoc-signed dev build **loses every grant on
//!   each rebuild**. Expect the dev loop to re-prompt constantly and do not
//!   mistake it for a bug in this layer.
//! * **Screen capture re-consents periodically.** Since macOS 15 the system
//!   re-asks for screen recording on a schedule even for an approved app, so
//!   `DisplayCapture` can transition `Granted → Prompt` behind the app's back.
//!   That is what makes the `change` subscription worth wiring on macOS:
//!   `NSWorkspace.didActivateApplicationNotification` plus a re-read on
//!   foreground is the cheap version, and it is a real `Some(vec![])` rather
//!   than the `None` a platform with no signal returns.

use vn_runtime::redjs::webapi::permissions::{Capability, PermissionHost, PermissionState};

pub struct MacHost;

impl MacHost {
    pub const fn new() -> MacHost {
        MacHost
    }
}

impl PermissionHost for MacHost {
    fn platform(&self) -> &'static str {
        "macOS"
    }

    /// Every capability, until the framework calls above are written. The
    /// extension surface turns this into an advisory naming macOS, which is the
    /// compat-first answer: an app is told the feature is unavailable *here*,
    /// by name, rather than being handed a silent `denied`.
    fn status(&self, _cap: Capability) -> PermissionState {
        PermissionState::Unsupported
    }
}
