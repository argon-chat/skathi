//! The JS event loop while the window draws no frames.
//!
//! The frame pump ([`crate::pump_frame`]) is an `on_next_frame` callback, so it
//! runs only when gpui draws, and a minimised window draws nothing: gpui on
//! Windows parks the window's frame callback on `SIZE_MINIMIZED` and gives it
//! back on restore. Before this module, everything the pump drives stopped
//! with it: timers, WebSocket and fetch delivery, IndexedDB results, the voice
//! and permission queues, and the devtools socket. The Argon client's realtime
//! socket closed with 1006 after 92 s minimised, and on restore its sleep
//! watcher (a 10 s timer arriving more than 30 s late) reloaded the app
//! (research/reports/m26/client-sweep-1.md, finding 4).
//!
//! A browser throttles a hidden page but never stops it. So a gpui timer runs
//! here every [`TICK`], and whenever no frame has been pumped for [`STALL`] it
//! runs the same JS wave the frame does ([`crate::js_wave`] with
//! [`crate::Wave::Background`]) and the same flush after it
//! ([`crate::after_wave`]), minus the animation frames:
//!
//! - timers, native tasks (fetch, image decodes), WebSocket events, IndexedDB
//!   results, Web Audio upkeep, permission changes and microtasks run;
//! - the host queues drain: voice events, `appPermissions` / `appShell`
//!   settlements, the localStorage write, a requested `location.reload()`,
//!   one devtools command per tick (so `eval` answers while minimised);
//! - `requestAnimationFrame` callbacks (and redjs's idle callbacks, which run
//!   after them) wait for the next drawn frame, as in a hidden browser page;
//! - tree changes are restyled and the window is marked dirty; it is drawn
//!   when it is shown again.
//!
//! The trigger is "no frame for `STALL`", not "minimised": it covers every way
//! a window stops getting frames (minimised, `SW_HIDE`, a stalled swap chain)
//! with one rule, and while frames arrive the tick costs one `Instant`
//! comparison. `document.hidden` and `visibilitychange` are not this module's:
//! they follow the minimise state through gpui's visibility observer
//! (src/window.rs `visibility_changed`).
//!
//! **Re-entrancy.** The tick is a gpui foreground task, so it runs from the
//! message loop, never inside the frame pump (one thread). It stands down when
//! it finds itself inside a published phase (a nested message loop entered
//! from a host fn), when the window is mid-update (`update` fails), and after
//! a caught native panic, exactly like `window::visibility_changed`.
//!
//! **Divergence (doc only).** Chrome aligns a hidden page's timers to 1 s
//! wake-ups (and chained timers to 1 min after five minutes hidden). Here they
//! keep a [`TICK`] resolution: a timer is never later than a browser's, and a
//! socket message waits at most one tick. `requestIdleCallback` waits for the
//! next drawn frame, where a browser still runs it (late) in a hidden page.

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
    /// When the frame pump last ran (its start and its end both stamp it).
    static LAST_FRAME: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Background waves run so far — read by devtools-free diagnostics and the
    /// first-takeover log line.
    static WAVES: Cell<u64> = const { Cell::new(0) };
    /// Whether the tick is currently standing in for the frame clock, so the
    /// log says so once per hidden period rather than once per tick.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Called by the frame pump: frames are flowing, the tick stands down.
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

/// The decision, pure: is the frame clock stalled? `None` (the pump has never
/// run) is not a stall: the first frame owns the boot wave.
pub fn due(last_frame: Option<Instant>, now: Instant) -> bool {
    match last_frame {
        Some(t) => now.saturating_duration_since(t) >= STALL,
        None => false,
    }
}

/// Starts the tick. Called once, after the primary window exists.
pub fn start(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(TICK).await;
        if !due(LAST_FRAME.with(|c| c.get()), Instant::now()) {
            continue;
        }
        let Some(handle) = crate::MAIN_WINDOW.with(|w| w.get()) else { continue };
        // Fails while the window is mid-update (a nested message loop) or
        // closed; either way there is nothing to do on this tick.
        let _ = handle.update(cx, |_, window, cx| wave(window, cx));
    })
    .detach();
}

/// One background wave: the frame's JS wave without the animation frames,
/// then the frame's flush.
fn wave(window: &mut gpui::Window, cx: &mut App) {
    if vn_runtime::bridge::poisoned()
        // Nested inside a published phase: the outer wave owns the JS.
        || vn_runtime::with_app(|_| ()).is_some()
    {
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
