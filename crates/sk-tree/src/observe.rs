//! ResizeObserver and IntersectionObserver, run where a browser runs them: after the frame's layout
//! and before its paint, so a callback sees the size the frame is about to paint and its response
//! lands in the SAME frame (polling from rAF runs before layout and lags a frame).
//!
//! [`update_layout_observers`] runs once per frame from the pump. When the frame will draw it draws
//! itself first (the layout step), then gpui finds the window clean and presents that draw. The
//! runtime's `layoutStep` gathers and calls back, with a microtask checkpoint between observers, and
//! the window is laid out again between rounds. Nothing runs when nothing is observed or the frame
//! laid nothing out.

use std::cell::Cell;

use gpui::{App, Window};
use sk_runtime::redjs::api::Value;
use sk_runtime::{host_fn, JsRuntime};

use crate::{log, perf, render};

thread_local! {
    static HOOK: Cell<u64> = const { Cell::new(0) };
    static OBSERVING: Cell<bool> = const { Cell::new(false) };
    static PENDING: Cell<bool> = const { Cell::new(false) };
    static SEEN: Cell<u64> = const { Cell::new(0) };
}

mod op {
    /// Gather at depth 0.
    pub const START: f64 = 0.0;
    /// Call the next observer that has entries.
    pub const NEXT: f64 = 1.0;
    /// Gather at the depth the last broadcast reached.
    pub const AGAIN: f64 = 2.0;
    pub const FINISH: f64 = 3.0;
}

const MAX_ROUNDS: u32 = 4096;
const MAX_CALLS: u32 = 1 << 16;

host_fn!(fn sk_set_layout_hook(g, args) {
    let f = args.arg(0);
    unsafe { sk_runtime::redjs::ffi::redjs_protect(g, f.0) };
    let old = HOOK.with(|c| c.replace(f.0));
    if old != 0 {
        unsafe { sk_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    Value::undefined().0
});

host_fn!(fn sk_layout_watch(_g, args) {
    let on = args.arg(0).as_bool().unwrap_or(true);
    OBSERVING.with(|c| c.set(on));
    PENDING.with(|c| c.set(on));
    Value::undefined().0
});

pub(crate) fn install(rt: &JsRuntime) {
    rt.register(
        "__sk_set_layout_hook",
        1,
        sk_set_layout_hook as *const std::ffi::c_void,
    );
    rt.register(
        "__sk_layout_watch",
        1,
        sk_layout_watch as *const std::ffi::c_void,
    );
}

pub fn update_layout_observers(window: &mut Window, cx: &mut App) {
    let f = HOOK.with(Cell::get);
    if f == 0 || !OBSERVING.with(Cell::get) {
        return;
    }
    // The pump's, not an input listener's: a callback's layout read may draw.
    crate::off_input(|| step(f, window, cx));
}

fn step(f: u64, window: &mut Window, cx: &mut App) {
    let _profile_window =
        sk_runtime::redjs::profiling::WindowScope::enter(crate::windows::key(window));
    let _profile = sk_runtime::redjs::profiling::span("layout", "Resize / intersection observers");
    if window.vn_is_dirty() {
        window.draw(cx).clear(cx);
        crate::containers::settle(|| {
            window.draw(cx).clear(cx);
        });
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
        if window.vn_is_dirty() || crate::layout_owed() {
            let t = perf::Span::start();
            window.draw(cx).clear(cx);
            crate::containers::settle(|| {
                window.draw(cx).clear(cx);
            });
            relayouts += 1;
            relayout_ms += t.ms();
        }
        due = call(f, op::AGAIN, window, cx);
    }
    let state = call(f, op::FINISH, window, cx);
    OBSERVING.with(|c| c.set(state & 2 != 0));
    // The skipped observations are still active, and a browser delivers them in the next rendering
    // update (Chromium schedules one), so the next frame steps whether or not it lays anything out.
    if state & 1 != 0 {
        PENDING.with(|c| c.set(true));
    }
    // What the callbacks changed is flushed by the next wave, and an idle frame clock would not
    // run one on its own.
    if calls > 0 || state & 1 != 0 {
        sk_runtime::frames::wake();
    }
    SEEN.with(|c| c.set(render::layout_count()));
    perf::note_observers(
        rounds,
        relayouts,
        relayout_ms,
        span.ms() - relayout_ms,
        state & 1 != 0,
    );
}

fn call(f: u64, which: f64, window: &mut Window, cx: &mut App) -> u32 {
    let mut out = 0u32;
    render::js_wave_app(window, cx, || {
        JsRuntime::with(|rt| {
            match rt
                .ctx()
                .call(Value(f), Value::undefined(), &[Value::number(which)])
            {
                Ok(v) => {
                    out = v
                        .as_number()
                        .filter(|n| n.is_finite() && *n > 0.0)
                        .map_or(0, |n| n as u32)
                }
                Err(sk_runtime::redjs::api::Error::Exception(e)) => {
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
