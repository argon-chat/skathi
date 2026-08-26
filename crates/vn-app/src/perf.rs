//! Frame-time profiler — `VN_PERF=1`.
//!
//! One `Instant` read per frame and one push into a `Vec<f32>`; every second
//! the window is summarised to stderr and cleared. With `VN_PERF` unset the
//! whole thing is a single relaxed atomic load and an early return, so it can
//! stay wired into the pump permanently.
//!
//! ```text
//! [vn-perf] boot->first-frame 214.7ms
//! [vn-perf] tag=grid-400-cached  t=+3.0s  frames=180  avg=16.7  p50=16.6  p95=17.9  max=31.2  fps=59.9
//! ```
//!
//! `__vn_perf_mark("<label>")` (registered in `main.rs`) labels the scenario
//! from the Vue app: it flushes the window in progress, prints a banner and
//! restarts the clock, which is what makes "idle vs scrolling vs churn"
//! separable in the log. It is a no-op when the probe is off, so an app may
//! call it unconditionally.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

const HOW_OFTEN: Duration = Duration::from_secs(1);

/// 0 = not looked at yet, 1 = off, 2 = on.
static ENABLED: AtomicU8 = AtomicU8::new(0);

#[inline]
pub fn enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("VN_PERF")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            ENABLED.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

struct State {
    /// When the process reached `main` — the denominator for "first paint".
    boot: Instant,
    first_frame: Option<Instant>,
    last_frame: Option<Instant>,
    window_start: Instant,
    /// Frame intervals in ms for the current one-second window.
    samples: Vec<f32>,
    tag: String,
    /// Frames since the current tag started.
    tagged_frames: u64,
    tag_start: Instant,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Starts the clock. Called once from `main`, before the window opens.
pub fn boot() {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            boot: now,
            first_frame: None,
            last_frame: None,
            window_start: now,
            samples: Vec::with_capacity(256),
            tag: "startup".into(),
            tagged_frames: 0,
            tag_start: now,
        });
    });
    eprintln!("[vn-perf] probe on (VN_PERF) — frame stats every {}s", HOW_OFTEN.as_secs());
}

/// One frame happened. Call from the per-frame pump.
#[inline]
pub fn frame() {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    STATE.with(|s| {
        let mut slot = s.borrow_mut();
        let Some(st) = slot.as_mut() else { return };
        if st.first_frame.is_none() {
            st.first_frame = Some(now);
            eprintln!(
                "[vn-perf] boot->first-frame {:.1}ms",
                (now - st.boot).as_secs_f32() * 1000.0
            );
        }
        if let Some(prev) = st.last_frame {
            st.samples.push((now - prev).as_secs_f32() * 1000.0);
            st.tagged_frames += 1;
        }
        st.last_frame = Some(now);
        if now - st.window_start >= HOW_OFTEN {
            report(st, now);
            st.samples.clear();
            st.window_start = now;
        }
    });
}

/// Labels the scenario the numbers belong to; flushes the window in progress.
pub fn mark(tag: &str) {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    STATE.with(|s| {
        let mut slot = s.borrow_mut();
        let Some(st) = slot.as_mut() else { return };
        if !st.samples.is_empty() {
            report(st, now);
        }
        eprintln!(
            "[vn-perf] ---- mark: {tag}  (previous tag {:?} ran {:.1}s / {} frames) ----",
            st.tag,
            (now - st.tag_start).as_secs_f32(),
            st.tagged_frames
        );
        st.samples.clear();
        st.window_start = now;
        st.tag = tag.to_string();
        st.tag_start = now;
        st.tagged_frames = 0;
    });
}

fn report(st: &mut State, now: Instant) {
    if st.samples.is_empty() {
        return;
    }
    // Sorting the window (a few hundred f32s) once a second is free next to a
    // frame, and gives exact percentiles rather than a running estimate.
    let mut sorted = st.samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let sum: f32 = sorted.iter().sum();
    let avg = sum / n as f32;
    let pct = |p: f32| sorted[((n as f32 * p).ceil() as usize).clamp(1, n) - 1];
    eprintln!(
        "[vn-perf] tag={:<22} t=+{:.1}s  frames={:<4} avg={:.2}  p50={:.2}  p95={:.2}  p99={:.2}  \
         max={:.2}  fps={:.1}  >33ms={}",
        st.tag,
        (now - st.boot).as_secs_f32(),
        n,
        avg,
        pct(0.50),
        pct(0.95),
        pct(0.99),
        sorted[n - 1],
        1000.0 / avg,
        sorted.iter().filter(|&&v| v > 33.0).count(),
    );
}
