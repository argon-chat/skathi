//! An inspector which exits during capture must not leave instrumentation on.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
static DEADLINE: AtomicU64 = AtomicU64::new(0);
static DURATION: AtomicU64 = AtomicU64::new(0);
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(super) fn arm(milliseconds: u64) {
    let duration = milliseconds.clamp(1000, 30_000);
    DURATION.store(duration, Ordering::Relaxed);
    DEADLINE.store(now().saturating_add(duration), Ordering::Relaxed);
}
pub(super) fn refresh() {
    if DEADLINE.load(Ordering::Relaxed) != 0 {
        DEADLINE.store(
            now().saturating_add(DURATION.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
    }
}
pub(super) fn clear() {
    DEADLINE.store(0, Ordering::Relaxed);
}
pub(super) fn expired() -> bool {
    let deadline = DEADLINE.load(Ordering::Relaxed);
    deadline != 0
        && now() >= deadline
        && DEADLINE
            .compare_exchange(deadline, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}
