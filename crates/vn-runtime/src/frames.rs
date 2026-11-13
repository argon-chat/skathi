//! The frame clock's wake seam: a producer that wakes the redjs pump also calls [`wake`] to get a frame.

use std::sync::OnceLock;

static WAKER: OnceLock<fn()> = OnceLock::new();

pub fn set_waker(waker: fn()) {
    let _ = WAKER.set(waker);
}

/// Asks for a frame at the next refresh. Callable from any thread; a no-op until a host installed its waker.
pub fn wake() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}
