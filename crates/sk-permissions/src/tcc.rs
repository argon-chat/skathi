//! The macOS backend's bookkeeping, free of the frameworks so it runs as unit tests everywhere;
//! `mac.rs` makes the calls.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use sk_runtime::redjs::webapi::permissions::{Capability, PermissionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Media {
    pub cap: Capability,
    pub usage_key: &'static str,
    pub page: &'static str,
    pub anchor: &'static str,
}

pub(crate) fn media(cap: Capability) -> Option<Media> {
    match cap {
        Capability::Microphone => Some(Media {
            cap,
            usage_key: "NSMicrophoneUsageDescription",
            page: "Microphone",
            anchor: "Privacy_Microphone",
        }),
        Capability::Camera => Some(Media {
            cap,
            usage_key: "NSCameraUsageDescription",
            page: "Camera",
            anchor: "Privacy_Camera",
        }),
        _ => None,
    }
}

pub(crate) fn privacy_pane_url(anchor: &str) -> String {
    format!("x-apple.systempreferences:com.apple.preference.security?{anchor}")
}

pub(crate) fn authorization_state(value: isize) -> PermissionState {
    // AVAuthorizationStatus: notDetermined=0, restricted=1, denied=2,
    // authorized=3. A future unknown value cannot be treated as a grant.
    match value {
        0 => PermissionState::Prompt,
        3 => PermissionState::Granted,
        _ => PermissionState::Denied,
    }
}

/// Screen recording's state from the preflight and whether this run's request was refused.
pub(crate) fn screen_state(granted: bool, refused: bool) -> PermissionState {
    if granted {
        PermissionState::Granted
    } else if refused {
        PermissionState::Denied
    } else {
        PermissionState::Prompt
    }
}

#[derive(Default)]
pub(crate) struct Requests {
    next: u32,
    asking: Vec<Capability>,
    pub(crate) waiting: Vec<(u32, Capability)>,
    pub(crate) settled: Vec<(u32, PermissionState)>,
    pub(crate) changes: Vec<Capability>,
    screen_refused: bool,
    seen: Vec<(Capability, PermissionState)>,
}

impl Requests {
    pub(crate) fn begin(&mut self, cap: Capability) -> (u32, bool) {
        self.next = self.next.wrapping_add(1).max(1);
        self.waiting.push((self.next, cap));
        let show = !self.asking.contains(&cap);
        if show {
            self.asking.push(cap);
        }
        (self.next, show)
    }

    pub(crate) fn complete(&mut self, cap: Capability, state: PermissionState) {
        self.asking.retain(|c| *c != cap);
        if cap == Capability::DisplayCapture {
            self.screen_refused = state != PermissionState::Granted;
        }
        self.remember(cap, state);
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

    fn remember(&mut self, cap: Capability, state: PermissionState) {
        match self.seen.iter_mut().find(|(c, _)| *c == cap) {
            Some(entry) => entry.1 = state,
            None => self.seen.push((cap, state)),
        }
    }

    pub(crate) fn read(&mut self, cap: Capability, state: PermissionState) -> PermissionState {
        let before = self.seen.iter().find(|(c, _)| *c == cap).map(|(_, s)| *s);
        if before.is_some_and(|seen| seen != state) {
            self.changed(cap);
        }
        self.remember(cap, state);
        state
    }

    pub(crate) fn screen(&mut self, granted: bool) -> PermissionState {
        let state = screen_state(granted, self.screen_refused);
        self.read(Capability::DisplayCapture, state)
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
    fn the_camera_is_the_microphones_model_with_its_own_key_and_pane() {
        let mic = media(Capability::Microphone).unwrap();
        let camera = media(Capability::Camera).unwrap();
        assert_eq!(
            (mic.usage_key, mic.page, mic.anchor),
            (
                "NSMicrophoneUsageDescription",
                "Microphone",
                "Privacy_Microphone"
            )
        );
        assert_eq!(
            (camera.usage_key, camera.page, camera.anchor),
            ("NSCameraUsageDescription", "Camera", "Privacy_Camera")
        );
        assert_eq!(
            privacy_pane_url(camera.anchor),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera"
        );
        assert_eq!(
            media(Capability::DisplayCapture),
            None,
            "screen recording is CoreGraphics', not AVFoundation's"
        );
        assert_eq!(media(Capability::Geolocation), None);
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
    fn the_camera_and_microphone_prompts_are_separate() {
        let mut requests = Requests::default();
        let (mic, show_mic) = requests.begin(Capability::Microphone);
        let (camera, show_camera) = requests.begin(Capability::Camera);
        assert!(show_mic && show_camera, "each has its own TCC prompt");
        requests.complete(Capability::Camera, PermissionState::Denied);
        assert_eq!(
            std::mem::take(&mut requests.settled),
            vec![(camera, PermissionState::Denied)]
        );
        assert_eq!(
            requests.waiting,
            vec![(mic, Capability::Microphone)],
            "the microphone waiter stays"
        );
        assert_eq!(
            std::mem::take(&mut requests.changes),
            vec![Capability::Camera]
        );
        assert_eq!(
            requests.read(Capability::Camera, PermissionState::Denied),
            PermissionState::Denied
        );
        assert!(requests.changes.is_empty());
    }

    #[test]
    fn a_toggle_in_system_settings_is_a_change_on_the_next_read() {
        let mut requests = Requests::default();
        assert_eq!(
            requests.read(Capability::Camera, PermissionState::Prompt),
            PermissionState::Prompt
        );
        assert!(requests.changes.is_empty(), "a first read is not a change");
        requests.read(Capability::Camera, PermissionState::Prompt);
        assert!(
            requests.changes.is_empty(),
            "a read that agrees is not a change"
        );
        requests.read(Capability::Camera, PermissionState::Denied);
        assert_eq!(
            std::mem::take(&mut requests.changes),
            vec![Capability::Camera]
        );
        requests.read(Capability::Camera, PermissionState::Granted);
        requests.read(Capability::Microphone, PermissionState::Granted);
        assert_eq!(
            std::mem::take(&mut requests.changes),
            vec![Capability::Camera]
        );
    }

    #[test]
    fn screen_recording_is_prompt_until_a_request_in_this_run_is_refused() {
        assert_eq!(screen_state(true, false), PermissionState::Granted);
        assert_eq!(
            screen_state(true, true),
            PermissionState::Granted,
            "a grant wins over an earlier refusal"
        );
        assert_eq!(screen_state(false, false), PermissionState::Prompt);
        assert_eq!(screen_state(false, true), PermissionState::Denied);

        let mut requests = Requests::default();
        assert_eq!(requests.screen(false), PermissionState::Prompt);
        let (mic, _) = requests.begin(Capability::Microphone);
        let (screen, show) = requests.begin(Capability::DisplayCapture);
        assert!(show, "the screen prompt is not the microphone's");
        requests.complete(Capability::DisplayCapture, PermissionState::Denied);
        assert_eq!(
            std::mem::take(&mut requests.settled),
            vec![(screen, PermissionState::Denied)]
        );
        assert_eq!(
            requests.waiting,
            vec![(mic, Capability::Microphone)],
            "the microphone waiter stays"
        );
        assert_eq!(
            std::mem::take(&mut requests.changes),
            vec![Capability::DisplayCapture]
        );
        assert_eq!(requests.screen(false), PermissionState::Denied);
        assert!(
            requests.changes.is_empty(),
            "a read that agrees is not a change"
        );
        assert_eq!(requests.screen(true), PermissionState::Granted);
        assert_eq!(
            std::mem::take(&mut requests.changes),
            vec![Capability::DisplayCapture]
        );
    }
}
