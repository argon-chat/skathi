//! A scriptable [`PermissionHost`] with no OS underneath it.
//!
//! Two jobs. It is the backend on any platform vue-native has not written one
//! for yet — where the honest answer for every capability is `Unsupported`, and
//! the vue-native extension surface turns that into an advisory naming the
//! platform. And it is the host the trait-level tests drive, because every state
//! and every transition can be scripted from a test without a real OS toggle
//! being flipped on somebody's machine.

use std::collections::HashMap;
use std::sync::Mutex;

use vn_runtime::redjs::webapi::permissions::{
    Capability, PermissionHost, PermissionState, RequestOutcome,
};

pub struct StubHost {
    platform: &'static str,
    states: Mutex<HashMap<Capability, PermissionState>>,
    /// What a `request()` turns a non-granted capability into. `None` = the
    /// platform cannot ask, so `request()` reports the current state unchanged.
    grants_on_request: Mutex<Option<PermissionState>>,
    subscribes: bool,
    changes: Mutex<Vec<Capability>>,
    settled: Mutex<Vec<(u32, PermissionState)>>,
    next_token: Mutex<u32>,
}

impl StubHost {
    /// Everything `Unsupported`, no subscription — the shape of a platform with
    /// no backend at all.
    pub fn unsupported(platform: &'static str) -> StubHost {
        StubHost {
            platform,
            states: Mutex::new(HashMap::new()),
            grants_on_request: Mutex::new(None),
            subscribes: false,
            changes: Mutex::new(Vec::new()),
            settled: Mutex::new(Vec::new()),
            next_token: Mutex::new(0),
        }
    }

    /// A host that answers from a table and reports a working change
    /// subscription.
    pub fn scripted(platform: &'static str, states: &[(Capability, PermissionState)]) -> StubHost {
        let h = StubHost::unsupported(platform);
        *h.states.lock().unwrap() = states.iter().copied().collect();
        StubHost { subscribes: true, ..h }
    }

    /// Make `request()` succeed, asynchronously, with this state.
    pub fn granting(self, state: PermissionState) -> StubHost {
        *self.grants_on_request.lock().unwrap() = Some(state);
        self
    }

    /// Move a capability as the OS would, queueing the `change` the surface is
    /// supposed to deliver.
    pub fn set(&self, cap: Capability, state: PermissionState) {
        let prev = self.states.lock().unwrap().insert(cap, state);
        if prev != Some(state) && self.subscribes {
            self.changes.lock().unwrap().push(cap);
        }
    }
}

impl PermissionHost for StubHost {
    fn platform(&self) -> &'static str {
        self.platform
    }

    fn status(&self, cap: Capability) -> PermissionState {
        self.states.lock().unwrap().get(&cap).copied().unwrap_or(PermissionState::Unsupported)
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        let now = self.status(cap);
        if now == PermissionState::Granted {
            return RequestOutcome::Settled(now);
        }
        let Some(target) = *self.grants_on_request.lock().unwrap() else {
            return RequestOutcome::Settled(now);
        };
        let mut n = self.next_token.lock().unwrap();
        *n += 1;
        let token = *n;
        drop(n);
        self.set(cap, target);
        self.settled.lock().unwrap().push((token, target));
        RequestOutcome::Pending(token)
    }

    fn drain_changes(&self) -> Option<Vec<Capability>> {
        if !self.subscribes {
            return None;
        }
        Some(std::mem::take(&mut *self.changes.lock().unwrap()))
    }

    fn drain_settled(&self) -> Vec<(u32, PermissionState)> {
        std::mem::take(&mut *self.settled.lock().unwrap())
    }
}
