//! The JS event loop while the window draws no frames.
//!
//! The frame pump ([`crate::pump_frame`]) is an `on_next_frame` callback, so it runs only when gpui
//! draws, and a minimised window draws nothing (gpui on Windows parks its frame callback on
//! `SIZE_MINIMIZED`). Everything the pump drives would stop with it: timers, WebSocket and fetch
//! delivery, IndexedDB results, the voice and permission queues, the devtools socket.
//!
//! A browser throttles a hidden page but never stops it. So a gpui timer runs here every [`TICK`],
//! and whenever no frame has been pumped for [`STALL`] it runs the same JS wave the frame does
//! ([`crate::js_wave`] with [`crate::Wave::Background`]) and the same flush after it
//! ([`crate::after_wave`]), minus the animation frames.
//!
//! The trigger is "no frame for `STALL`", not "minimised": one rule covers every way a window stops
//! getting frames (minimised, `SW_HIDE`, a stalled swap chain, another virtual desktop).
//!
//! **Re-entrancy.** The tick is a gpui foreground task, so it runs from the message loop, never
//! inside the frame pump. It stands down when it finds itself inside a published phase (a nested
//! message loop entered from a host fn), when the window is mid-update, and after a caught native
//! panic, like `window::visibility_changed`; after the panic it opens the crash dialog in the
//! pump's place (src/crash_window.rs).
//!
//! **Divergence.** Chrome aligns a hidden page's timers to 1 s wake-ups. Here they keep a [`TICK`]
//! resolution, so a timer is never later than a browser's and a socket message waits at most one
//! tick. `requestIdleCallback` waits for the next drawn frame, where a browser still runs it (late).

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui::App;

/// How often the background tick looks. Also the worst-case delivery delay of
/// a socket message or a due timer while the window is hidden.
pub const TICK: Duration = Duration::from_millis(100);

/// How long without a pumped frame before the tick takes over. Well above any
/// display's frame interval and any ordinary slow frame, so the tick never
/// interleaves with a window that is drawing.
pub const STALL: Duration = Duration::from_millis(250);

thread_local! {
    static LAST_FRAME: Cell<Option<Instant>> = const { Cell::new(None) };
    static WAVES: Cell<u64> = const { Cell::new(0) };
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}

pub fn note_frame() {
    LAST_FRAME.with(|c| c.set(Some(Instant::now())));
    if ACTIVE.with(|a| a.replace(false)) {
        let n = WAVES.with(|w| w.get());
        vn_tree::log::record(
            "window",
            format!("[window] frames resumed — the background tick stands down ({n} background waves so far)"),
        );
    }
}

pub fn pumped_once() -> bool {
    LAST_FRAME.with(|c| c.get().is_some())
}

pub fn due(last_frame: Option<Instant>, now: Instant) -> bool {
    match last_frame {
        Some(t) => now.saturating_duration_since(t) >= STALL,
        None => false,
    }
}

pub fn start(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(TICK).await;
        if !due(LAST_FRAME.with(|c| c.get()), Instant::now()) {
            continue;
        }
        let Some(handle) = crate::MAIN_WINDOW.with(|w| w.get()) else { continue };
        let _ = handle.update(cx, |_, window, cx| wave(window, cx));
    })
    .detach();
}

fn wave(window: &mut gpui::Window, cx: &mut App) {
    if vn_runtime::with_app(|_| ()).is_some() {
        return;
    }
    // No JS after a caught native panic.
    // crash dialog; a minimised, hidden or cloaked window draws no frame, so
    // the tick does it here (src/crash_window.rs, once per process).
    if vn_runtime::bridge::poisoned() {
        crate::crash_window::on_poisoned(cx);
        return;
    }
    if !ACTIVE.with(|a| a.replace(true)) {
        vn_tree::log::record(
            "window",
            "[window] no frames (the window is minimised or hidden) — timers, sockets and devtools now run \
             on a background tick every 100 ms; requestAnimationFrame waits for the next drawn frame",
        );
    }
    WAVES.with(|w| w.set(w.get() + 1));
    crate::drain_log_tee();
    crate::js_wave(window, cx, crate::Wave::Background);
    crate::after_wave(window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pump_that_never_ran_is_not_a_stall() {
        assert!(!due(None, Instant::now()));
    }

    #[test]
    fn frames_at_any_display_rate_keep_the_tick_off() {
        let now = Instant::now();
        for interval_ms in [4u64, 7, 16, 33, 100, 200] {
            let last = now - Duration::from_millis(interval_ms);
            assert!(!due(Some(last), now), "{interval_ms} ms since the last frame is not a stall");
        }
    }

    #[test]
    fn a_parked_frame_clock_hands_over_to_the_tick() {
        let now = Instant::now();
        assert!(due(Some(now - STALL), now));
        assert!(due(Some(now - Duration::from_secs(92)), now));
    }

    #[test]
    fn a_clock_that_looks_backwards_is_not_a_stall() {
        // `Instant` is monotonic, but a stamp taken after `now` was read (the
        // pump ran between the two reads) must not underflow into a stall.
        let now = Instant::now();
        assert!(!due(Some(now + Duration::from_millis(5)), now));
    }

    #[test]
    fn the_tick_is_finer_than_the_stall() {
        // Otherwise a hidden window would wait two stalls for its first wave.
        assert!(TICK < STALL);
    }
}
