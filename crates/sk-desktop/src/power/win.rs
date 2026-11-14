//! A display and system power request.

use std::sync::Mutex;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired,
    PowerSetRequest,
};
use windows::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
use windows::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

struct Request(HANDLE);
// SAFETY: a power request handle is a kernel object handle, usable from any thread.
unsafe impl Send for Request {}

static HELD: Mutex<Option<Request>> = Mutex::new(None);

pub fn hold_screen(on: bool) -> bool {
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    if on == held.is_some() {
        return true;
    }
    if let Some(Request(h)) = held.take() {
        // SAFETY: `h` is the request created below, set for both types and not yet closed.
        unsafe {
            let _ = PowerClearRequest(h, PowerRequestDisplayRequired);
            let _ = PowerClearRequest(h, PowerRequestSystemRequired);
            let _ = CloseHandle(h);
        }
        return true;
    }
    let mut reason: Vec<u16> = "Screen Wake Lock".encode_utf16().chain([0]).collect();
    let ctx = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: PWSTR(reason.as_mut_ptr()),
        },
    };
    // SAFETY: `ctx` and the string it points at outlive the call, which copies the reason.
    let Ok(h) = (unsafe { PowerCreateRequest(&ctx) }) else {
        return false;
    };
    // SAFETY: `h` is a fresh power request.
    let ok = unsafe {
        PowerSetRequest(h, PowerRequestDisplayRequired).is_ok()
            && PowerSetRequest(h, PowerRequestSystemRequired).is_ok()
    };
    if !ok {
        // SAFETY: as above; nothing else holds it.
        unsafe {
            let _ = PowerClearRequest(h, PowerRequestDisplayRequired);
            let _ = CloseHandle(h);
        }
        return false;
    }
    *held = Some(Request(h));
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_request_is_taken_once_and_dropped() {
        assert!(super::hold_screen(true));
        assert!(super::hold_screen(true));
        assert!(super::HELD.lock().unwrap().is_some());
        assert!(super::hold_screen(false));
        assert!(super::HELD.lock().unwrap().is_none());
    }
}
