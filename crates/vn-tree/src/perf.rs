//! Phase counters for the `VN_PERF` probe — "where did the frame go?".
//!
//! `vn-app`'s `perf` module owns the frame CLOCK (one `Instant` per frame, a
//! one-second window, percentiles). This module owns the two phases that clock
//! cannot see from the host side, because both run deep inside `vn-tree`:
//!
//! * **restyle** — [`crate::tree::Tree::restyle`]: re-parsing dirty inline
//!   styles, then the cascade (`vn_css::Engine::compute_node` per element),
//!   then storing the result and feeding the transition animator;
//! * **build** — [`crate::render::build_root`]: lowering the retained tree into
//!   this frame's gpui element tree.
//!
//! Both are accumulated into a one-second window that `vn-app` drains next to
//! its own frame report, so the two lines always describe the same second.
//!
//! Everything here is gated on the same `VN_PERF` env var: with the probe off a
//! phase costs one relaxed atomic load and an early return, so the hooks stay
//! wired in permanently rather than living behind a cargo feature nobody
//! remembers to turn on.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

/// 0 = not looked at yet, 1 = off, 2 = on. Mirrors `vn_app::perf::enabled`;
/// the crates cannot see each other's statics and the check is one load.
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

/// One second's worth of phase work. Times are milliseconds, summed over the
/// window; `*_max` is the single worst occurrence in it.
#[derive(Default, Clone, Copy, Debug)]
pub struct Phases {
    /// How many times the restyle pass ran (a frame can run several — every
    /// event dispatch flushes the tree before returning to the pump).
    pub restyle_passes: u32,
    pub restyle_ms: f32,
    pub restyle_max_ms: f32,
    /// Time inside the inline-style re-parse step of the pass.
    pub inline_ms: f32,
    /// Time inside `Engine::compute_node` — matching + folding + `var()`.
    pub cascade_ms: f32,
    /// Time storing the results and feeding the animator (the change gate).
    pub store_ms: f32,
    /// Elements handed to `compute_node` across the window.
    pub nodes_cascaded: u64,
    /// Selectors actually run through the matcher. With the rule index in
    /// place this is much smaller than `nodes_cascaded * rules_in_sheet`, and
    /// the ratio between the two is the index's whole reason for existing.
    pub rules_tested: u64,

    /// How many times `build_root` ran (once per gpui `render()`).
    pub build_passes: u32,
    pub build_ms: f32,
    pub build_max_ms: f32,
    /// Tree nodes lowered into gpui elements across the window.
    pub elements_built: u64,

    // ---- gpui's own element work, measured at the ROOT node's `BoundsProbe`.
    // The root's element methods bracket the whole recursion, so timing three
    // of them attributes every millisecond gpui spends between "here is your
    // element tree" and "the scene is ready":
    /// Building the layout request tree (recursive `request_layout`).
    pub layout_req_ms: f32,
    /// taffy's `compute_layout` — the gap between the root's `request_layout`
    /// returning and its `prepaint` being called.
    pub taffy_ms: f32,
    /// The prepaint recursion (hitboxes, scroll offsets, our bounds probes).
    pub prepaint_ms: f32,
    /// The paint recursion (quads, text, shadows into the scene).
    pub paint_ms: f32,
    pub draw_passes: u32,

    /// Time inside `dispatch_event` — the JS half of an input event: the
    /// runtime's capture/bubble walk over the mirror tree, the app's handlers,
    /// and the microtask drain that runs Vue's patch queue. It OVERLAPS the
    /// host's own phases (it is called from the pump and from gpui listeners
    /// alike), so it is reported on its own line rather than added to them.
    pub js_passes: u32,
    pub js_ms: f32,
    pub js_max_ms: f32,
}

thread_local! {
    static PHASES: RefCell<Phases> = const { RefCell::new(Phases {
        restyle_passes: 0,
        restyle_ms: 0.0,
        restyle_max_ms: 0.0,
        inline_ms: 0.0,
        cascade_ms: 0.0,
        store_ms: 0.0,
        nodes_cascaded: 0,
        rules_tested: 0,
        build_passes: 0,
        build_ms: 0.0,
        build_max_ms: 0.0,
        elements_built: 0,
        layout_req_ms: 0.0,
        taffy_ms: 0.0,
        prepaint_ms: 0.0,
        paint_ms: 0.0,
        draw_passes: 0,
        js_passes: 0,
        js_ms: 0.0,
        js_max_ms: 0.0,
    }) };
    /// When the root element's `request_layout` returned — taffy runs next.
    static LAYOUT_DONE: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

fn with(f: impl FnOnce(&mut Phases)) {
    PHASES.with(|p| f(&mut p.borrow_mut()));
}

/// Drains the window. Returns `None` when the probe is off or nothing ran.
pub fn take() -> Option<Phases> {
    if !enabled() {
        return None;
    }
    let taken = PHASES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    (taken.restyle_passes > 0 || taken.build_passes > 0 || taken.js_passes > 0).then_some(taken)
}

/// A phase timer. `Span::start()` is a single atomic load when the probe is
/// off, so callers need no `if` of their own.
pub struct Span(Option<Instant>);

impl Span {
    #[inline]
    pub fn start() -> Span {
        Span(enabled().then(Instant::now))
    }

    /// Elapsed milliseconds, or 0.0 with the probe off.
    #[inline]
    pub fn ms(&self) -> f32 {
        match self.0 {
            Some(t) => t.elapsed().as_secs_f32() * 1000.0,
            None => 0.0,
        }
    }
}

/// One completed restyle pass.
pub fn note_restyle(total_ms: f32, inline_ms: f32, cascade_ms: f32, store_ms: f32, nodes: u64) {
    if !enabled() {
        return;
    }
    let rules = vn_css::perf::take_rules_tested();
    with(|p| {
        p.restyle_passes += 1;
        p.restyle_ms += total_ms;
        p.restyle_max_ms = p.restyle_max_ms.max(total_ms);
        p.inline_ms += inline_ms;
        p.cascade_ms += cascade_ms;
        p.store_ms += store_ms;
        p.nodes_cascaded += nodes;
        p.rules_tested += rules;
    });
}

/// One completed `build_root`.
pub fn note_build(total_ms: f32, elements: u64) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.build_passes += 1;
        p.build_ms += total_ms;
        p.build_max_ms = p.build_max_ms.max(total_ms);
        p.elements_built += elements;
    });
}

/// The root element finished handing gpui its layout requests; taffy is next.
pub fn note_layout_req(ms: f32) {
    if !enabled() {
        return;
    }
    LAYOUT_DONE.with(|c| *c.borrow_mut() = Some(Instant::now()));
    with(|p| {
        p.layout_req_ms += ms;
        p.draw_passes += 1;
    });
}

/// The root element's prepaint is about to start — everything since
/// [`note_layout_req`] was taffy.
pub fn note_taffy_gap() {
    if !enabled() {
        return;
    }
    let Some(t) = LAYOUT_DONE.with(|c| c.borrow_mut().take()) else { return };
    let ms = t.elapsed().as_secs_f32() * 1000.0;
    with(|p| p.taffy_ms += ms);
}

pub fn note_prepaint(ms: f32) {
    if !enabled() {
        return;
    }
    with(|p| p.prepaint_ms += ms);
}

pub fn note_paint(ms: f32) {
    if !enabled() {
        return;
    }
    with(|p| p.paint_ms += ms);
}

/// One completed JS event dispatch.
pub fn note_js(total_ms: f32) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.js_passes += 1;
        p.js_ms += total_ms;
        p.js_max_ms = p.js_max_ms.max(total_ms);
    });
}
