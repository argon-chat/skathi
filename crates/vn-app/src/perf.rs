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
//! [vn-perf]   restyle passes=12  nodes=6.1K  sel=340.2K total=48.3ms  avg=4.02  max=6.10  [inline 0.1 · cascade 44.0 · store 4.2]
//! [vn-perf]   build   passes=180 nodes=90.0K              total=210.4ms avg=1.17  max=3.90
//! ```
//!
//! The two indented lines are the phase breakdown ([`vn_tree::perf`]) for the
//! SAME second as the frame line above them: where the milliseconds went, how
//! many elements were re-cascaded and rebuilt, and how many selectors the
//! matcher actually ran. They are what makes "the mutation frame costs 90 ms"
//! an actionable statement instead of a complaint.
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

    // ---- the four-bucket frame accounting (see `pump_done`/`render_*`).
    /// The last stamp taken, whichever bucket it closed.
    cursor: Instant,
    pump_ms: f32,
    setup_ms: f32,
    render_ms: f32,
    /// The tail of a frame that DREW: gpui's layout, paint, present — plus
    /// whatever is left of the vsync interval, which on a slow frame is
    /// nothing. Split from the idle tail because mixing them hides the number
    /// that matters: what one drawn frame actually costs.
    tail_draw_ms: f32,
    /// The tail of a frame that drew nothing — pure wait for the next vsync.
    tail_idle_ms: f32,
    draws: u32,
    /// Did the frame currently being measured reach `render()`?
    drew: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Starts the clock. Called once from `main`, before the window opens.
pub fn boot() {
    // The rolling ring's `boot` is seeded FIRST and unconditionally: it is a
    // `thread_local` that would otherwise initialise itself lazily on the
    // first frame, which would make `boot -> first frame` read as zero — the
    // one number a startup regression shows up in.
    #[cfg(vn_devtools)]
    ROLL.with(|r| r.borrow_mut().boot = Instant::now());
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
            cursor: now,
            pump_ms: 0.0,
            setup_ms: 0.0,
            render_ms: 0.0,
            tail_draw_ms: 0.0,
            tail_idle_ms: 0.0,
            draws: 0,
            drew: false,
        });
    });
    eprintln!("[vn-perf] probe on (VN_PERF) — frame stats every {}s", HOW_OFTEN.as_secs());
}

// ---------------------------------------------------------------------------
// the rolling window the devtools Performance panel reads
// ---------------------------------------------------------------------------
//
// `VN_PERF` above is a REPORT: it prints a summary to stderr every second and
// costs a `Vec` push plus four `Instant` reads per frame. The panel needs
// something different — a window of the last few seconds, readable at any
// moment, from a build nobody remembered to set an environment variable on —
// so [`ROLL`] is a separate, unconditional, fixed-size ring.
//
// Its cost is one `Instant::now()` (which `frame()` was already going to take
// when the probe is on, and which is a `QueryPerformanceCounter` read
// otherwise: ~20 ns) and one store into a fixed array. No allocation, no
// branch on an environment variable, no growth. That is what makes it honest to
// leave wired in permanently: the profiler you have to restart the app to turn
// on is the profiler that never answers the question you actually had.
//
// It is `cfg(vn_devtools)` all the same, because a shipped `--features prod`
// build has nobody to read it.

/// How many frame intervals the panel can look back over. 600 ≈ 10 s at 60 Hz
/// and 3.6 s at 165 Hz — long enough to hold a whole interaction, short enough
/// that the array is 2.4 KB.
#[cfg(vn_devtools)]
pub const ROLL_LEN: usize = 600;

#[cfg(vn_devtools)]
struct Roll {
    /// Frame intervals in ms, newest at `head - 1`.
    ms: [f32; ROLL_LEN],
    head: usize,
    filled: usize,
    frames: u64,
    last: Option<Instant>,
    boot: Instant,
    first_frame: Option<Instant>,
}

#[cfg(vn_devtools)]
thread_local! {
    static ROLL: RefCell<Roll> = RefCell::new(Roll {
        ms: [0.0; ROLL_LEN],
        head: 0,
        filled: 0,
        frames: 0,
        last: None,
        boot: Instant::now(),
        first_frame: None,
    });
}

/// A window of frame intervals, oldest first, plus what the panel wants to
/// print beside the sparkline.
#[cfg(vn_devtools)]
pub struct Snapshot {
    pub samples: Vec<f32>,
    pub frames: u64,
    /// Milliseconds from `main` to the first frame — the number a startup
    /// regression shows up in first.
    pub boot_to_first_frame_ms: Option<f32>,
    pub uptime_ms: f32,
}

#[cfg(vn_devtools)]
impl Snapshot {
    /// `(avg, p50, p95, max, fps)` over the window. Empty windows report zeros
    /// rather than NaN — a panel should show "no frames yet", not "NaN".
    pub fn stats(&self) -> (f32, f32, f32, f32, f32) {
        if self.samples.is_empty() {
            return (0.0, 0.0, 0.0, 0.0, 0.0);
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let sum: f32 = sorted.iter().sum();
        let avg = sum / sorted.len() as f32;
        let at = |q: f32| sorted[((sorted.len() as f32 - 1.0) * q).round() as usize];
        (avg, at(0.5), at(0.95), sorted[sorted.len() - 1], if avg > 0.0 { 1000.0 / avg } else { 0.0 })
    }
}

/// The rolling window, oldest sample first.
#[cfg(vn_devtools)]
pub fn snapshot() -> Snapshot {
    ROLL.with(|r| {
        let r = r.borrow();
        let mut samples = Vec::with_capacity(r.filled);
        // `filled` entries ending at `head`, wrapped.
        let start = (r.head + ROLL_LEN - r.filled) % ROLL_LEN;
        for i in 0..r.filled {
            samples.push(r.ms[(start + i) % ROLL_LEN]);
        }
        Snapshot {
            samples,
            frames: r.frames,
            boot_to_first_frame_ms: r.first_frame.map(|f| ms_since(r.boot, f)),
            uptime_ms: ms_since(r.boot, Instant::now()),
        }
    })
}

/// The rolling ring's half of [`frame`]. Split out so the always-on cost is one
/// function with no environment lookup in it.
#[cfg(vn_devtools)]
#[inline]
fn roll_frame(now: Instant) {
    ROLL.with(|r| {
        let mut r = r.borrow_mut();
        if r.first_frame.is_none() {
            r.first_frame = Some(now);
        }
        if let Some(prev) = r.last {
            let head = r.head;
            r.ms[head] = (now - prev).as_secs_f32() * 1000.0;
            r.head = (head + 1) % ROLL_LEN;
            r.filled = (r.filled + 1).min(ROLL_LEN);
            r.frames += 1;
        }
        r.last = Some(now);
    });
}

/// One frame happened. Call from the per-frame pump.
#[inline]
pub fn frame() {
    #[cfg(vn_devtools)]
    roll_frame(Instant::now());
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
        // Closes the previous frame's tail (everything after the last stamp:
        // gpui's layout, paint, present, and the vsync wait).
        let tail = ms_since(st.cursor, now);
        if std::mem::take(&mut st.drew) {
            st.tail_draw_ms += tail;
        } else {
            st.tail_idle_ms += tail;
        }
        st.cursor = now;
        if now - st.window_start >= HOW_OFTEN {
            report(st, now);
            st.samples.clear();
            st.window_start = now;
        }
    });
}

fn ms_since(then: Instant, now: Instant) -> f32 {
    now.saturating_duration_since(then).as_secs_f32() * 1000.0
}

/// Closes a bucket and opens the next. The four calls below partition the wall
/// clock exactly — `pump + setup + render + tail` is the frame — which is what
/// makes "the mutation frame costs 90 ms" answerable rather than arguable.
fn stamp(f: impl FnOnce(&mut State, f32)) {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    STATE.with(|s| {
        let mut slot = s.borrow_mut();
        let Some(st) = slot.as_mut() else { return };
        let dt = ms_since(st.cursor, now);
        st.cursor = now;
        f(st, dt);
    });
}

/// The JS pump (host fns, HMR poll, timers/rAF/microtasks, the tree flush)
/// finished; gpui is about to decide whether to draw.
pub fn pump_done() {
    stamp(|st, dt| st.pump_ms += dt);
}

/// gpui reached the hosting view's `render()` — everything between here and
/// [`pump_done`] is gpui's own per-frame setup for a draw it decided to do.
pub fn render_begin() {
    stamp(|st, dt| st.setup_ms += dt);
}

/// The element tree for this frame is built; layout and paint follow.
pub fn render_done() {
    stamp(|st, dt| {
        st.render_ms += dt;
        st.draws += 1;
        st.drew = true;
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

/// `12345` -> `12.3K`, `1234567` -> `1.23M` — the counters run to millions and
/// a raw digit string makes the two columns that matter unreadable.
fn compact(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.1}K", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

/// The wall-clock partition of the window: every millisecond of the second is
/// in exactly one of the four buckets, so a slow second can be attributed
/// before anything is optimized.
fn report_frame_split(st: &mut State) {
    let total = st.pump_ms + st.setup_ms + st.render_ms + st.tail_draw_ms + st.tail_idle_ms;
    if total <= 0.0 {
        return;
    }
    // `gpui/frame` is what one DRAWN frame costs outside our own code: gpui's
    // pre-render setup, its layout/prepaint/paint, the present, and whatever is
    // left of the vsync interval (on a slow frame, nothing).
    let per_draw = if st.draws > 0 {
        (st.setup_ms + st.tail_draw_ms) / st.draws as f32
    } else {
        0.0
    };
    eprintln!(
        "[vn-perf]   frame   draws={:<4} pump={:.1}ms ({:.0}%)  render={:.1}ms ({:.0}%)  \
         gpui={:.1}ms ({:.0}%, {:.1}/draw)  idle-wait={:.1}ms",
        st.draws,
        st.pump_ms,
        100.0 * st.pump_ms / total,
        st.render_ms,
        100.0 * st.render_ms / total,
        st.setup_ms + st.tail_draw_ms,
        100.0 * (st.setup_ms + st.tail_draw_ms) / total,
        per_draw,
        st.tail_idle_ms,
    );
    st.pump_ms = 0.0;
    st.setup_ms = 0.0;
    st.render_ms = 0.0;
    st.tail_draw_ms = 0.0;
    st.tail_idle_ms = 0.0;
    st.draws = 0;
}

/// The phase breakdown, printed right under the frame line so both describe the
/// same second. Silent when nothing ran (an idle second still draws, so `build`
/// is always there; `restyle` only appears when something actually mutated).
fn report_phases() {
    let Some(p) = vn_tree::perf::take() else { return };
    if p.restyle_passes > 0 {
        eprintln!(
            "[vn-perf]   restyle passes={:<4} nodes={:<7} sel={:<7} total={:.1}ms  \
             avg={:.2}  max={:.2}   [inline {:.1} · cascade {:.1} (match {:.1} · vars {:.1} · \
             fold {:.1}) · store {:.1}]",
            p.restyle_passes,
            compact(p.nodes_cascaded),
            compact(p.rules_tested),
            p.restyle_ms,
            p.restyle_ms / p.restyle_passes as f32,
            p.restyle_max_ms,
            p.inline_ms,
            p.cascade_ms,
            p.match_ms,
            p.vars_ms,
            p.fold_ms,
            p.store_ms,
        );
    }
    if p.build_passes > 0 {
        eprintln!(
            "[vn-perf]   build   passes={:<4} nodes={:<7} {:<12} total={:.1}ms  \
             avg={:.2}  max={:.2}",
            p.build_passes,
            compact(p.elements_built),
            "",
            p.build_ms,
            p.build_ms / p.build_passes as f32,
            p.build_max_ms,
        );
    }
    if p.draw_passes > 0 {
        eprintln!(
            "[vn-perf]   gpui    draws={:<4} layout-req={:.1}ms  taffy={:.1}ms  \
             prepaint={:.1}ms  paint={:.1}ms   (per draw: {:.1} + {:.1} + {:.1} + {:.1})",
            p.draw_passes,
            p.layout_req_ms,
            p.taffy_ms,
            p.prepaint_ms,
            p.paint_ms,
            p.layout_req_ms / p.draw_passes as f32,
            p.taffy_ms / p.draw_passes as f32,
            p.prepaint_ms / p.draw_passes as f32,
            p.paint_ms / p.draw_passes as f32,
        );
    }
    if p.js_passes > 0 {
        eprintln!(
            "[vn-perf]   js      events={:<3} {:<28} total={:.1}ms  avg={:.2}  max={:.2}   \
             (inside pump/paint+wait, not extra)",
            p.js_passes,
            "",
            p.js_ms,
            p.js_ms / p.js_passes as f32,
            p.js_max_ms,
        );
    }
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
    report_frame_split(st);
    report_phases();
}
