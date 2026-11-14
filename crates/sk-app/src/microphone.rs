//! The microphone behind `getUserMedia({ audio })`: redjs's capture gate
//! (redjs `webapi/mediastream.rs`, "The capture gate") and the system's device
//! notifications (`devices`).
//!
//! Capture is enabled: `getUserMedia({ audio })` opens the device once the user allowed this app in
//! its prompt (crates/sk-permissions `consent`). `SK_MEDIA_CAPTURE=deny` latches capture shut (agent
//! and test launches, so they never open the machine's microphone); `fake` captures a synthetic
//! 440 Hz tone instead of any device, behind the same prompt.
//!
//! Echo cancellation, noise suppression and gain control are the embedder's
//! (`mediastream::set_capture_processing`): the engine has none of its own,
//! and `getSettings()` reports what runs.

mod devices;

use sk_runtime::redjs::webapi::mediastream;

/// Opens the gate as `SK_MEDIA_CAPTURE` says and starts the device notifications; once, at boot.
pub fn install() {
    let raw = std::env::var("SK_MEDIA_CAPTURE").ok();
    match capture_mode(raw.as_deref()) {
        CaptureMode::Deny => {
            mediastream::forbid_capture();
            eprintln!("[sk] SK_MEDIA_CAPTURE=deny: microphone capture is off for this process");
        }
        CaptureMode::Fake => {
            mediastream::set_fake_capture();
            mediastream::set_capture_allowed(true);
            eprintln!("[sk] SK_MEDIA_CAPTURE=fake: getUserMedia captures a synthetic 440 Hz tone; no device is opened");
        }
        CaptureMode::Device => {
            if let Some(v) = raw.as_deref().filter(|v| !v.trim().is_empty()) {
                eprintln!("[sk] SK_MEDIA_CAPTURE={v:?} is neither `deny` nor `fake`: ignored");
            }
            mediastream::set_capture_allowed(true);
        }
    }
    // devicechange from the OS's own notifications, and the default followed
    // when the user changes it.
    devices::install();
    if let Some(hint) = system_denied_hint() {
        mediastream::set_system_denied_hint(hint);
    }
}

fn system_denied_hint() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some(
            "On Windows: Settings > Privacy & security > Microphone (ms-settings:privacy-microphone); both \
             \"Microphone access\" and \"Let desktop apps access your microphone\" must be on. \
             appPermissions.request(\"microphone\") opens that page.",
        )
    } else if cfg!(target_os = "macos") {
        Some(
            "On macOS: System Settings › Privacy & Security › Microphone \
             (x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone); this app must be \
             on. appPermissions.request(\"microphone\") opens that page.",
        )
    } else {
        None
    }
}

/// What `SK_MEDIA_CAPTURE` asks for.
#[derive(Debug, PartialEq, Eq)]
enum CaptureMode {
    /// The machine's devices, after consent (unset, or anything else).
    Device,
    Deny,
    Fake,
}

/// `SK_MEDIA_CAPTURE` (any case, surrounding space ignored).
fn capture_mode(value: Option<&str>) -> CaptureMode {
    match value.map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("deny") => CaptureMode::Deny,
        Some(v) if v.eq_ignore_ascii_case("fake") => CaptureMode::Fake,
        _ => CaptureMode::Device,
    }
}

#[cfg(test)]
mod tests {
    use super::{capture_mode, system_denied_hint, CaptureMode};

    #[test]
    fn only_deny_latches_the_gate_and_only_fake_swaps_the_device() {
        assert_eq!(capture_mode(Some("deny")), CaptureMode::Deny);
        assert_eq!(capture_mode(Some(" DENY ")), CaptureMode::Deny);
        assert_eq!(capture_mode(Some("fake")), CaptureMode::Fake);
        assert_eq!(capture_mode(Some(" Fake")), CaptureMode::Fake);
        assert_eq!(capture_mode(Some("allow")), CaptureMode::Device);
        assert_eq!(capture_mode(Some("")), CaptureMode::Device);
        assert_eq!(capture_mode(None), CaptureMode::Device);
    }

    #[test]
    fn a_system_denial_names_this_platforms_settings_page() {
        let hint = system_denied_hint();
        if cfg!(target_os = "windows") {
            assert!(hint.unwrap().contains("ms-settings:privacy-microphone"));
        } else if cfg!(target_os = "macos") {
            let hint = hint.unwrap();
            assert!(
                hint.contains("Privacy_Microphone") && !hint.contains("Windows"),
                "{hint}"
            );
        } else {
            assert_eq!(hint, None, "no page to name");
        }
    }
}
