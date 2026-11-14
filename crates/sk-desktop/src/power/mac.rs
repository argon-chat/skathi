//! A `PreventUserIdleDisplaySleep` assertion.

use std::ffi::c_void;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2_foundation::NSString;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        kind: *const c_void,
        level: u32,
        name: *const c_void,
        id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
}

const ASSERTION_LEVEL_ON: u32 = 255;

static HELD: Mutex<Option<u32>> = Mutex::new(None);

pub fn hold_screen(on: bool) -> bool {
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    if on == held.is_some() {
        return true;
    }
    if let Some(id) = held.take() {
        // SAFETY: `id` is the assertion created below, released once.
        unsafe { IOPMAssertionRelease(id) };
        return true;
    }
    // NSString is toll-free bridged with CFString.
    let kind = NSString::from_str("PreventUserIdleDisplaySleep");
    let name = NSString::from_str("Screen Wake Lock");
    let mut id = 0u32;
    // SAFETY: both strings outlive the call, which copies what it keeps.
    let r = unsafe {
        IOPMAssertionCreateWithName(
            Retained::as_ptr(&kind).cast(),
            ASSERTION_LEVEL_ON,
            Retained::as_ptr(&name).cast(),
            &mut id,
        )
    };
    if r != 0 {
        return false;
    }
    *held = Some(id);
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_assertion_is_taken_once_and_dropped() {
        assert!(super::hold_screen(true));
        assert!(super::hold_screen(true));
        assert!(super::HELD.lock().unwrap().is_some());
        assert!(super::hold_screen(false));
        assert!(super::HELD.lock().unwrap().is_none());
    }
}
