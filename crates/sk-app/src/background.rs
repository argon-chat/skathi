//! The JS event loop while the window draws no frames.
//!
//! The frame pump ([`crate::pump_frame`]) is an `on_next_frame` callback, so it runs only when gpui
//! draws, and a minimised window draws nothing (gpui on Windows parks its frame callback on
//! `SIZE_MINIMIZED`). Everything the pump drives would stop with it: timers, WebSocket and fetch
//! delivery, IndexedDB results, the voice and permission queues, the devtools socket.
//!
//! A browser throttles a hidden page but never stops it. So while the page is hidden a gpui timer
//! runs here, every [`TICK`] at most and only while JS has work scheduled, and whenever no frame
//! has been pumped for [`STALL`] it runs the same JS wave the frame does ([`crate::js_wave`] with
//! [`crate::Wave::Background`]) and the same flush after it ([`crate::after_wave`]), minus the
//! animation frames.
//!
//! A visible idle window draws no frames either, by design (src/idle.rs), so the tick does not run
//! then: it waits for a [`poke`] (work queued for JS from another thread, or the page hiding) and
//! takes over only when no frame follows within `STALL`. That one rule covers every way a window
//! stops getting frames (minimised, `SW_HIDE`, a stalled swap chain, another virtual desktop).
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;
use std::time::{Duration, Instant};

use futures::task::AtomicWaker;
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
    static REPORTED: Cell<bool> = const { Cell::new(false) };
    /// Until the JS context or the host next has work, as of the last background wave.
    static NEXT_WORK: Cell<Option<Duration>> = const { Cell::new(None) };
}

pub fn note_frame() {
    LAST_FRAME.with(|c| c.set(Some(Instant::now())));
    let was_active = ACTIVE.with(|a| a.replace(false));
    if was_active && REPORTED.with(|r| r.replace(false)) {
        let n = WAVES.with(|w| w.get());
        sk_tree::log::record(
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

static POKED: AtomicBool = AtomicBool::new(false);
static POKE: AtomicWaker = AtomicWaker::new();

/// Something may need JS while no frame comes: look within [`STALL`]. Callable from any thread.
pub fn poke() {
    POKED.store(true, Ordering::Release);
    POKE.wake();
}

async fn poked() {
    std::future::poll_fn(|task| {
        if POKED.swap(false, Ordering::AcqRel) {
            return Poll::Ready(());
        }
        POKE.register(task.waker());
        if POKED.swap(false, Ordering::AcqRel) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await
}

pub fn start(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        // Only Windows paces an idle window to no frames (src/idle.rs).
        if !cfg!(target_os = "windows") {
            cx.background_executor().timer(TICK).await;
            if !due(LAST_FRAME.with(|c| c.get()), Instant::now()) {
                continue;
            }
        } else {
            // Once ticking, the tick waits for the work the last wave left, or for a poke.
            let wait = if ACTIVE.with(Cell::get) {
                NEXT_WORK.with(Cell::get).map(|d| d.max(TICK))
            } else if crate::window::page_hidden() {
                Some(TICK)
            } else {
                None
            };
            let poke = std::pin::pin!(poked());
            match wait {
                Some(wait) => {
                    let timer = std::pin::pin!(cx.background_executor().timer(wait));
                    futures::future::select(poke, timer).await;
                }
                None => poke.await,
            }
            if crate::window::page_hidden() {
                if !due(LAST_FRAME.with(|c| c.get()), Instant::now()) {
                    continue;
                }
            } else {
                // A visible idle window draws no frames until asked: give it `STALL` to answer.
                let asked = Instant::now();
                cx.background_executor().timer(STALL).await;
                if LAST_FRAME.with(Cell::get).is_some_and(|t| t >= asked) {
                    continue;
                }
            }
        }
        let Some(handle) = crate::MAIN_WINDOW.with(|w| w.get()) else {
            continue;
        };
        let _ = handle.update(cx, |_, window, cx| wave(window, cx));
    })
    .detach();
}

fn wave(window: &mut gpui::Window, cx: &mut App) {
    if sk_runtime::with_app(|_| ()).is_some() {
        return;
    }
    // No JS after a caught native panic.
    // crash dialog; a minimised, hidden or cloaked window draws no frame, so
    // the tick does it here (src/crash_window.rs, once per process).
    if sk_runtime::bridge::poisoned() {
        crate::crash_window::on_poisoned(cx);
        return;
    }
    ACTIVE.with(|a| a.set(true));
    // A visible idle window draws no frames by design; only a hidden page is worth a log line.
    if crate::window::page_hidden() && !REPORTED.with(|r| r.replace(true)) {
        sk_tree::log::record(
            "window",
            "[window] no frames (the window is minimised or hidden) — timers, sockets and devtools now run \
             on a background tick every 100 ms; requestAnimationFrame waits for the next drawn frame",
        );
    }
    WAVES.with(|w| w.set(w.get() + 1));
    crate::drain_log_tee();
    crate::js_wave(window, cx, crate::Wave::Background);
    crate::after_wave(window, cx);
    let now = Instant::now();
    let js = sk_runtime::JsRuntime::try_with(|rt| {
        #[cfg(feature = "audio")]
        if sk_runtime::redjs::webapi::audio::busy(rt.raw()) {
            return Some(now);
        }
        let at = sk_runtime::redjs::web::next_work_ms(rt.raw())?;
        let ms = at - sk_runtime::redjs::web::now_ms();
        Some(now + Duration::from_secs_f64(ms.max(0.0) / 1000.0))
    })
    .flatten();
    let host = sk_runtime::frames::take_wake_at();
    let next = js.into_iter().chain(host).min();
    NEXT_WORK.with(|n| n.set(next.map(|at| at.saturating_duration_since(now))));
}
