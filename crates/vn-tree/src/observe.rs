//! ResizeObserver and IntersectionObserver, run where a browser runs them:
//! after the frame's layout and before its paint.
//!
//! HTML "update the rendering" (§8.1.7.3), for a document whose rendering is
//! being updated, runs the animation frame callbacks, then:
//!
//! 1. recalculates styles and updates layout;
//! 2. ResizeObserver (Resize Observer §3.4.4): depth = 0; gathers the active
//!    observations deeper than `depth`; while there are any, broadcasts them
//!    (each observer's callback, with its entries), sets `depth` to the
//!    shallowest target the broadcast reported, recalculates styles and
//!    updates layout again, and gathers again at `depth`. An observation it
//!    had to skip (at or above `depth`) raises the "loop completed with
//!    undelivered notifications" error;
//! 3. runs the update intersection observations steps, which queue entries
//!    and a task to deliver them;
//!
//! and then paints. So an observer's callback sees the size the frame is about
//! to paint, and what it changes in response is painted in the SAME frame: a
//! panel sized from its content's height never shows the frame in between.
//!
//! vue-native used to poll both observers from `requestAnimationFrame`, which
//! runs BEFORE the frame's layout: a callback saw the previous frame's sizes,
//! its response painted a frame later, and a newly mounted target with no box
//! yet forced a layout of its own from inside the rAF (m26 fixes-lag-1).
//!
//! [`update_layout_observers`] is that step, once per frame, called by the
//! host from the frame pump after the JS wave and its flush. gpui would draw
//! the window right after the pump (its `on_next_frame` callbacks run first);
//! when this frame is going to draw ([`gpui::Window::vn_is_dirty`]), the step
//! draws it ITSELF first, which is step 1, and gpui then finds the window
//! clean and presents that draw. The runtime's `layoutStep`
//! (packages/vue-native/src/runtime.ts) does the gathering and the callbacks,
//! one call per observer, and the microtask checkpoint a browser runs after
//! each callback runs between those calls. Between rounds the step lays the
//! window out again when the callbacks changed anything.
//!
//! Nothing runs when nothing is observed, and a frame that lays nothing out
//! (an idle frame, a present-only frame: decision 28's `<video>` and canvas
//! frames written into the tile the last scene samples) runs no step unless
//! an `observe()` is waiting for its first report.

use std::cell::Cell;

use gpui::{App, Window};
use vn_runtime::redjs::api::Value;
use vn_runtime::{host_fn, JsRuntime};

use crate::{log, perf, render};

thread_local! {
    /// The runtime's `layoutStep`, a protected JS function; 0 = none.
    static HOOK: Cell<u64> = const { Cell::new(0) };
    /// The runtime has observations (`__vn_layout_watch(true)`, and the last
    /// step's answer).
    static OBSERVING: Cell<bool> = const { Cell::new(false) };
    /// An `observe()` since the last step: it reports the target's first size
    /// at the next frame whether or not anything is laid out.
    static PENDING: Cell<bool> = const { Cell::new(false) };
    /// `render::layout_count()` when the last step ended.
    static SEEN: Cell<u64> = const { Cell::new(0) };
}

/// `layoutStep(op)`, the runtime's half (the numbers are the protocol).
mod op {
    /// Gather at depth 0. Answers how many observers have entries.
    pub const START: f64 = 0.0;
    /// Call the next observer that has entries. Answers how many are left.
    pub const NEXT: f64 = 1.0;
    /// Gather at the depth the last broadcast reached. Answers as `START`.
    pub const AGAIN: f64 = 2.0;
    /// The loop error if an observation was skipped, then the intersection
    /// update. Answers bit 0: the loop error was raised; bit 1: something is
    /// still observed.
    pub const FINISH: f64 = 3.0;
}

/// Each round reports only targets deeper than the last round's shallowest,
/// so the loop ends within the tree's depth. These only stop a runtime that
/// never says it is done.
const MAX_ROUNDS: u32 = 4096;
const MAX_CALLS: u32 = 1 << 16;

// `__vn_set_layout_hook(layoutStep)`: the runtime installs its step function,
// once per evaluation; a reload's runtime replaces its predecessor's.
host_fn!(fn vn_set_layout_hook(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    let old = HOOK.with(|c| c.replace(f.0));
    if old != 0 {
        unsafe { vn_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    Value::undefined().0
});

// `__vn_layout_watch(observing)`: `true` from `observe()` (a step is owed at
// the next frame), `false` when a runtime lets every observation go (its
// unload).
host_fn!(fn vn_layout_watch(_g, args) {
    let on = args.arg(0).as_bool().unwrap_or(true);
    OBSERVING.with(|c| c.set(on));
    PENDING.with(|c| c.set(on));
    Value::undefined().0
});

/// Registers the observer step's half of the `__vn_*` surface.
pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__vn_set_layout_hook", 1, vn_set_layout_hook as *const std::ffi::c_void);
    rt.register("__vn_layout_watch", 1, vn_layout_watch as *const std::ffi::c_void);
}

/// The frame's ResizeObserver and IntersectionObserver step (module docs).
/// `window` is the primary window, from the frame pump, after the JS wave and
/// before gpui's own draw decision.
///
/// A popped-out window draws in its own frame callback, after this one, so a
/// target it paints is reported from that window's previous frame: one frame
/// later than a target in the primary window (docs/MULTIWINDOW.md).
pub fn update_layout_observers(window: &mut Window, cx: &mut App) {
    let f = HOOK.with(Cell::get);
    if f == 0 || !OBSERVING.with(Cell::get) {
        return;
    }
    // The pump's, not an input listener's: a callback's layout read may draw.
    crate::off_input(|| step(f, window, cx));
}

fn step(f: u64, window: &mut Window, cx: &mut App) {
    // "Recalculate styles and update layout": the frame's own draw, if this
    // frame is going to draw.
    if window.vn_is_dirty() {
        window.draw(cx).clear(cx);
    }
    let pending = PENDING.with(|c| c.replace(false));
    if !pending && render::layout_count() == SEEN.with(Cell::get) {
        return;
    }
    let span = perf::Span::start();
    let mut relayout_ms = 0.0f32;
    let (mut rounds, mut relayouts, mut calls) = (0u32, 0u32, 0u32);
    let mut due = call(f, op::START, window, cx);
    while due > 0 && rounds < MAX_ROUNDS {
        rounds += 1;
        while due > 0 && calls < MAX_CALLS {
            calls += 1;
            due = call(f, op::NEXT, window, cx);
        }
        // "Recalculate styles and update layout", again: what the callbacks
        // changed is flushed by now (each call ends with the wave's flush).
        if window.vn_is_dirty() || crate::layout_owed() {
            let t = perf::Span::start();
            window.draw(cx).clear(cx);
            relayouts += 1;
            relayout_ms += t.ms();
        }
        due = call(f, op::AGAIN, window, cx);
    }
    let state = call(f, op::FINISH, window, cx);
    OBSERVING.with(|c| c.set(state & 2 != 0));
    // The skipped observations are still active, and a browser delivers them
    // in the next rendering update (Chromium schedules one), so the next
    // frame steps whether or not it lays anything out. (An `observe()` made
    // during the step has already asked for one.)
    if state & 1 != 0 {
        PENDING.with(|c| c.set(true));
    }
    SEEN.with(|c| c.set(render::layout_count()));
    perf::note_observers(rounds, relayouts, relayout_ms, span.ms() - relayout_ms, state & 1 != 0);
}

/// One `layoutStep(op)` as its own JS wave: the call, then the microtask
/// checkpoint (Vue's scheduler flushes the patch a callback asked for), then
/// the tree flush. Answers the runtime's number, 0 on an exception.
fn call(f: u64, which: f64, window: &mut Window, cx: &mut App) -> u32 {
    let mut out = 0u32;
    render::js_wave_app(window, cx, || {
        JsRuntime::with(|rt| {
            match rt.ctx().call(Value(f), Value::undefined(), &[Value::number(which)]) {
                Ok(v) => out = v.as_number().filter(|n| n.is_finite() && *n > 0.0).map_or(0, |n| n as u32),
                Err(vn_runtime::redjs::api::Error::Exception(e)) => {
                    let err = rt.describe(e);
                    log::record("js", format!("[layout step] {}\n{}", err.header, err.stack));
                }
                Err(_) => {}
            }
            rt.ctx().drain_microtasks();
        });
        true
    });
    out
}
