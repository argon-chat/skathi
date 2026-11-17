//! Linux: the desktop has no per-app permission for the microphone or camera (access is the user's
//! device permissions, or a sandbox's static ones), so a capture this build can make reads `granted`.
//! Everything else is `unsupported`.

use sk_runtime::redjs::webapi::permissions::{Capability, PermissionHost, PermissionState};

use crate::consent;

pub struct LinuxHost;

impl PermissionHost for LinuxHost {
    fn platform(&self) -> &'static str {
        "Linux"
    }

    fn status(&self, cap: Capability) -> PermissionState {
        if !consent::prompts(cap) {
            PermissionState::Unsupported
        } else if (consent::Hooks::process().forbidden)() {
            PermissionState::Denied
        } else {
            PermissionState::Granted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consent::{AppConsent, Asker, Hooks};
    use sk_runtime::redjs::webapi::permissions::RequestOutcome;

    #[test]
    fn the_microphone_reads_granted_and_is_never_asked_about() {
        static OS: LinuxHost = LinuxHost;
        fn no() -> bool {
            false
        }
        fn never() -> Option<Asker> {
            panic!("nothing gates a capture on Linux: there is no question to ask")
        }
        let h = AppConsent::ungated(
            &OS,
            Hooks {
                forbidden: no,
                fake: no,
                asker: never,
            },
        );
        assert_eq!(h.status(Capability::Microphone), PermissionState::Granted);
        assert_eq!(
            h.request(Capability::Microphone),
            RequestOutcome::Settled(PermissionState::Granted)
        );
        assert_eq!(
            h.status(Capability::Notifications),
            PermissionState::Unsupported
        );
    }
}
