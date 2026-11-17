//! The frame clock's wake seam: a producer that wakes the redjs pump also calls [`wake`] to get a frame.
//!
//! An idle host stops pumping until something asks: [`wake`] from any thread when work was queued
//! for the pump, [`wake_at`] from the UI thread when a frame must look again at a known time.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

static WAKER: OnceLock<fn()> = OnceLock::new();

pub fn set_waker(waker: fn()) {
    let _ = WAKER.set(waker);
    redjs::web::set_wake_hook(wake);
}

/// Asks for a frame at the next refresh. Callable from any thread; a no-op until a host installed its waker.
pub fn wake() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}

thread_local! {
    static WAKE_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// From the UI thread: a frame must run by `at` even if nothing else asks for one.
pub fn wake_at(at: Instant) {
    WAKE_AT.with(|w| w.set(Some(w.get().map_or(at, |t| t.min(at)))));
}

/// The earliest [`wake_at`] since the last call, for the host's idle decision.
pub fn take_wake_at() -> Option<Instant> {
    WAKE_AT.with(Cell::take)
}
