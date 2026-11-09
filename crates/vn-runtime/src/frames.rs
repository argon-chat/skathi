//! The frame clock's wake seam.
//!
//! An idle window paces its frames (vn-app `src/idle.rs`), so a result that a
//! worker thread queues for JS could wait for the next paced frame. A producer
//! that already wakes the redjs pump calls [`wake`] beside it, and the host
//! delivers a frame at the next refresh instead.

use std::sync::OnceLock;

static WAKER: OnceLock<fn()> = OnceLock::new();

/// Installs the host's waker. Once per process; a second call is ignored.
pub fn set_waker(waker: fn()) {
    let _ = WAKER.set(waker);
}

/// Asks for a frame at the next refresh. Callable from any thread; a no-op
/// until a host installed its waker.
pub fn wake() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}
