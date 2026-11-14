//! Phase counters for the `SK_PERF` probe: the restyle and build phases that run deep inside `sk-tree`,
//! accumulated into a one-second window that `sk-app` drains next to its own frame report. With the
//! probe off a phase costs one relaxed atomic load.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;

/// 0 = not looked at yet, 1 = off, 2 = on.
static ENABLED: AtomicU8 = AtomicU8::new(0);

static TRACING: AtomicBool = AtomicBool::new(false);

pub fn set_tracing(on: bool) {
    TRACING.store(on, Ordering::Relaxed);
}

pub fn env_enabled() -> bool {
    env_on()
}

#[inline]
pub fn enabled() -> bool {
    TRACING.load(Ordering::Relaxed) || env_on()
}

#[inline]
fn env_on() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("SK_PERF")
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

#[derive(Clone, Copy, Debug)]
pub struct Phases {
    /// How many times the restyle pass ran (every event dispatch flushes the tree, so a frame can run several).
    pub restyle_passes: u32,
    pub restyle_ms: f32,
    pub restyle_max_ms: f32,
    pub inline_ms: f32,
    pub cascade_ms: f32,
    pub match_ms: f32,
    pub vars_ms: f32,
    pub fold_ms: f32,
    pub store_ms: f32,
    pub nodes_cascaded: u64,
    pub rules_tested: u64,
    /// [`Self::rules_tested`] split by the index bucket that offered each one (`sk_css::perf::Split`).
    pub rules_split: sk_css::perf::Split,
    pub share_asked: u64,
    pub share_hits: u64,

    pub build_passes: u32,
    /// Cache boundaries that actually re-rendered; with the offers and stand-downs, whether the retained-render cache earns its keep.
    pub cached_renders: u32,
    pub cache_offers: u32,
    pub cache_standdowns: u32,
    pub build_ms: f32,
    pub build_max_ms: f32,
    pub elements_built: u64,

    pub layout_req_ms: f32,
    pub taffy_ms: f32,
    pub prepaint_ms: f32,
    pub paint_ms: f32,
    pub draw_passes: u32,

    /// Time inside `dispatch_event`: the JS half of an input event, including the microtask drain.
    pub js_passes: u32,
    pub js_ms: f32,
    pub js_max_ms: f32,

    pub restyle_full: u32,
    pub nodes_changed: u64,
    pub full_reasons: [u32; FullReason::COUNT],
    pub recompiles: u32,
    pub recompile_ms: f32,
    pub sync_layouts: u32,
    pub sync_layout_ms: f32,
    pub sync_layout_max_ms: f32,
    pub sync_layout_refused: u32,
    pub sync_by: [u32; SyncCaller::COUNT],
    /// JS waves whose changes a forced layout had already drawn (the host skips the redundant notification).
    pub sync_tail: u32,
    pub sync_tail_free: u32,
    /// The ResizeObserver / IntersectionObserver step after layout (`crate::observe`): steps, broadcast
    /// rounds, layouts redone and their time, the rest of the time, and "loop completed with
    /// undelivered notifications" errors.
    pub observer_steps: u32,
    pub observer_rounds: u32,
    pub observer_relayouts: u32,
    pub observer_relayout_ms: f32,
    pub observer_ms: f32,
    pub observer_loop_errors: u32,
    pub dom_ops: [u32; DomOp::COUNT],
    /// First and last DOM mutation in the window: a proxy for the span Vue spent rendering and patching.
    pub dom_first: Option<Instant>,
    pub dom_last: Option<Instant>,
    pub layout_reads: u32,
    pub scroll_reads: u32,
    pub style_reads: u32,
    pub queries: u32,
    pub style_flushes: u32,
    pub resample_sync: u32,
    pub resample_sync_ms: f32,
    pub resample_async: u32,
    pub fonts_registered: u32,
    pub font_bytes: u64,
    pub img_srcs: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum FullReason {
    Stylesheet = 0,
    Env = 1,
    StyleElement = 2,
    Fonts = 3,
    ViewportBasis = 4,
    RemBasis = 5,
}

impl FullReason {
    pub const COUNT: usize = 6;
    pub const NAMES: [&'static str; Self::COUNT] = [
        "stylesheet",
        "env",
        "style-element",
        "fonts",
        "viewport-basis",
        "rem-basis",
    ];
}

#[derive(Clone, Copy, Debug)]
pub enum SyncCaller {
    /// `getBoundingClientRect` / `offset*` / `client*` (`dom::bounds`).
    Bounds = 0,
    /// `scrollWidth` / `scrollHeight` / a clamped scroll write (`dom::layout_if_stale`).
    Scroll = 1,
    /// `el.focus()` on a node mounted this wave.
    Focus = 2,
}

impl SyncCaller {
    pub const COUNT: usize = 3;
    pub const NAMES: [&'static str; Self::COUNT] = ["bounds", "scroll", "focus"];
}

/// The `__sk_*` mutators, grouped.
#[derive(Clone, Copy, Debug)]
pub enum DomOp {
    /// `createElement` / `createText` / `createComment`.
    Create = 0,
    /// `insert` (also a move).
    Insert = 1,
    /// `remove` (a whole subtree).
    Remove = 2,
    /// `className`.
    Class = 3,
    /// One inline style property.
    Style = 4,
    /// An attribute / property.
    Prop = 5,
    /// `nodeValue` / `textContent`.
    Text = 6,
    /// Listener on/off, scope hashes.
    Other = 7,
}

impl DomOp {
    pub const COUNT: usize = 8;
    pub const NAMES: [&'static str; Self::COUNT] = [
        "create", "insert", "remove", "class", "style", "prop", "text", "other",
    ];
}

const ZERO: Phases = Phases {
    restyle_passes: 0,
    restyle_ms: 0.0,
    restyle_max_ms: 0.0,
    inline_ms: 0.0,
    cascade_ms: 0.0,
    match_ms: 0.0,
    vars_ms: 0.0,
    fold_ms: 0.0,
    store_ms: 0.0,
    nodes_cascaded: 0,
    rules_tested: 0,
    rules_split: sk_css::perf::Split {
        by_bucket: [0; 6],
        matched: 0,
    },
    share_asked: 0,
    share_hits: 0,
    build_passes: 0,
    cached_renders: 0,
    cache_offers: 0,
    cache_standdowns: 0,
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
    restyle_full: 0,
    nodes_changed: 0,
    full_reasons: [0; FullReason::COUNT],
    recompiles: 0,
    recompile_ms: 0.0,
    sync_layouts: 0,
    sync_layout_ms: 0.0,
    sync_layout_max_ms: 0.0,
    sync_layout_refused: 0,
    sync_by: [0; SyncCaller::COUNT],
    sync_tail: 0,
    sync_tail_free: 0,
    observer_steps: 0,
    observer_rounds: 0,
    observer_relayouts: 0,
    observer_relayout_ms: 0.0,
    observer_ms: 0.0,
    observer_loop_errors: 0,
    dom_ops: [0; DomOp::COUNT],
    dom_first: None,
    dom_last: None,
    layout_reads: 0,
    scroll_reads: 0,
    style_reads: 0,
    queries: 0,
    style_flushes: 0,
    resample_sync: 0,
    resample_sync_ms: 0.0,
    resample_async: 0,
    fonts_registered: 0,
    font_bytes: 0,
    img_srcs: 0,
};

impl Default for Phases {
    fn default() -> Self {
        ZERO
    }
}

thread_local! {
    static PHASES: RefCell<Phases> = const { RefCell::new(ZERO) };
    static LAYOUT_DONE: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

fn with(f: impl FnOnce(&mut Phases)) {
    PHASES.with(|p| f(&mut p.borrow_mut()));
}

pub fn take() -> Option<Phases> {
    if !enabled() {
        return None;
    }
    let taken = PHASES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    (taken.restyle_passes > 0 || taken.build_passes > 0 || taken.js_passes > 0).then_some(taken)
}

/// Drains the window whatever ran in it: the devtools trace closes one record per frame.
pub fn take_all() -> Phases {
    PHASES.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// A phase timer. `Span::start()` is a single atomic load when the probe is off, so callers need no
/// `if` of their own.
pub struct Span(Option<Instant>);

impl Span {
    #[inline]
    pub fn start() -> Span {
        Span(enabled().then(Instant::now))
    }

    #[inline]
    pub fn ms(&self) -> f32 {
        match self.0 {
            Some(t) => t.elapsed().as_secs_f32() * 1000.0,
            None => 0.0,
        }
    }
}

pub fn note_restyle(total_ms: f32, inline_ms: f32, cascade_ms: f32, store_ms: f32, nodes: u64) {
    if !enabled() {
        return;
    }
    sk_runtime::redjs::profiling::counter("nodesCascaded", nodes);
    let (rules, match_ms, vars_ms, fold_ms) = sk_css::perf::take();
    sk_runtime::redjs::profiling::counter("selectorsTested", rules);
    let split = sk_css::perf::take_split();
    let (share_asked, share_hits) = sk_css::perf::take_share();
    with(|p| {
        p.restyle_passes += 1;
        p.restyle_ms += total_ms;
        p.restyle_max_ms = p.restyle_max_ms.max(total_ms);
        p.inline_ms += inline_ms;
        p.cascade_ms += cascade_ms;
        p.match_ms += match_ms;
        p.vars_ms += vars_ms;
        p.fold_ms += fold_ms;
        p.store_ms += store_ms;
        p.nodes_cascaded += nodes;
        p.rules_tested += rules;
        for (total, n) in p.rules_split.by_bucket.iter_mut().zip(split.by_bucket) {
            *total += n;
        }
        p.rules_split.matched += split.matched;
        p.share_asked += share_asked;
        p.share_hits += share_hits;
    });
}

pub fn note_build(total_ms: f32, elements: u64) {
    if !enabled() {
        return;
    }
    sk_runtime::redjs::profiling::counter("elementsBuilt", elements);
    with(|p| {
        p.build_passes += 1;
        p.build_ms += total_ms;
        p.build_max_ms = p.build_max_ms.max(total_ms);
        p.elements_built += elements;
    });
}

#[inline]
pub fn note_cache_offer(stood_down: bool) {
    if !enabled() {
        return;
    }
    with(|p| {
        if stood_down {
            p.cache_standdowns += 1;
        } else {
            p.cache_offers += 1;
        }
    });
}

pub fn note_build_extra(total_ms: f32, elements: u64) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.build_ms += total_ms;
        p.elements_built += elements;
        p.cached_renders += 1;
        // The subtree's own prepaint+layout runs inside gpui's prepaint, which is where it belongs;
        // only the lowering is moved.
        p.prepaint_ms -= total_ms;
    });
}

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

pub fn note_taffy_gap() {
    if !enabled() {
        return;
    }
    let Some(t) = LAYOUT_DONE.with(|c| c.borrow_mut().take()) else {
        return;
    };
    let ended = Instant::now();
    let ms = ended.duration_since(t).as_secs_f32() * 1000.0;
    sk_runtime::redjs::profiling::interval("layout", "Taffy layout", t, ended);
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

pub fn note_restyle_shape(full: bool, changed: u64) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.restyle_full += u32::from(full);
        p.nodes_changed += changed;
    });
}

#[inline]
pub fn note_full(reason: FullReason) {
    if !enabled() {
        return;
    }
    with(|p| p.full_reasons[reason as usize] += 1);
}

// stylesheet compiles since boot (always on: one compile is ~100 ms, so a few relaxed adds beside
// it cost nothing)

static LIVE_BYTES_PROBE: std::sync::OnceLock<fn() -> Option<i64>> = std::sync::OnceLock::new();
static SHEET_COMPILES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SHEET_COMPILE_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SHEET_SOURCE_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SHEET_GROWTH: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static SHEET_GROWTH_SEEN: AtomicBool = AtomicBool::new(false);

pub fn set_live_bytes_probe(f: fn() -> Option<i64>) {
    let _ = LIVE_BYTES_PROBE.set(f);
}

pub fn live_bytes() -> Option<i64> {
    LIVE_BYTES_PROBE.get().and_then(|f| f())
}

/// One whole stylesheet compile and engine swap: `ms`, source length, and the Rust heap's net growth across it.
pub fn note_sheet_compile(ms: f32, source_len: usize, before: Option<i64>, after: Option<i64>) {
    use std::sync::atomic::Ordering::Relaxed;
    SHEET_COMPILES.fetch_add(1, Relaxed);
    SHEET_COMPILE_US.fetch_add((ms * 1000.0) as u64, Relaxed);
    SHEET_SOURCE_BYTES.fetch_add(source_len as u64, Relaxed);
    if let (Some(b), Some(a)) = (before, after) {
        SHEET_GROWTH.fetch_add(a - b, Relaxed);
        SHEET_GROWTH_SEEN.store(true, Relaxed);
    }
}

/// `(compiles, total ms, source bytes compiled, net Rust heap growth)` since boot; the growth is
/// `None` without a live-heap probe.
pub fn sheet_compiles() -> (u64, f64, u64, Option<i64>) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        SHEET_COMPILES.load(Relaxed),
        SHEET_COMPILE_US.load(Relaxed) as f64 / 1000.0,
        SHEET_SOURCE_BYTES.load(Relaxed),
        SHEET_GROWTH_SEEN
            .load(Relaxed)
            .then(|| SHEET_GROWTH.load(Relaxed)),
    )
}

pub fn note_recompile(ms: f32) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.recompiles += 1;
        p.recompile_ms += ms;
    });
}

/// A JS wave whose changes a forced layout had already drawn; `free` = nothing else needed the
/// frame to draw.
pub fn note_sync_tail(free: bool) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.sync_tail += 1;
        if free {
            p.sync_tail_free += 1;
        }
    });
}

pub fn note_observers(rounds: u32, relayouts: u32, relayout_ms: f32, ms: f32, loop_error: bool) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.observer_steps += 1;
        p.observer_rounds += rounds;
        p.observer_relayouts += relayouts;
        p.observer_relayout_ms += relayout_ms;
        p.observer_ms += ms.max(0.0);
        p.observer_loop_errors += u32::from(loop_error);
    });
}

pub fn note_sync_layout(caller: SyncCaller, drew: bool, ms: f32) {
    if !enabled() {
        return;
    }
    if drew {
        sk_runtime::redjs::profiling::counter("forcedLayouts", 1);
    }
    with(|p| {
        p.sync_by[caller as usize] += 1;
        if drew {
            p.sync_layouts += 1;
            p.sync_layout_ms += ms;
            p.sync_layout_max_ms = p.sync_layout_max_ms.max(ms);
        } else {
            p.sync_layout_refused += 1;
        }
    });
}

#[inline]
pub fn note_dom(op: DomOp) {
    if !enabled() {
        return;
    }
    if sk_runtime::redjs::profiling::enabled() {
        sk_runtime::redjs::profiling::counter("domWrites", 1);
        sk_runtime::redjs::profiling::counter(&format!("dom.{}", DomOp::NAMES[op as usize]), 1);
    }
    let now = Instant::now();
    with(|p| {
        p.dom_ops[op as usize] += 1;
        if p.dom_first.is_none() {
            p.dom_first = Some(now);
        }
        p.dom_last = Some(now);
    });
}

#[derive(Clone, Copy)]
pub enum Read {
    Layout,
    Scroll,
    Style,
    Query,
}

#[inline]
pub fn note_read(kind: Read) {
    if !enabled() {
        return;
    }
    sk_runtime::redjs::profiling::counter(
        match kind {
            Read::Layout => "layoutReads",
            Read::Scroll => "scrollReads",
            Read::Style => "styleReads",
            Read::Query => "selectorQueries",
        },
        1,
    );
    with(|p| match kind {
        Read::Layout => p.layout_reads += 1,
        Read::Scroll => p.scroll_reads += 1,
        Read::Style => p.style_reads += 1,
        Read::Query => p.queries += 1,
    });
}

pub fn note_style_flush() {
    if !enabled() {
        return;
    }
    with(|p| p.style_flushes += 1);
}

pub fn note_resample(sync_ms: Option<f32>) {
    if !enabled() {
        return;
    }
    with(|p| match sync_ms {
        Some(ms) => {
            p.resample_sync += 1;
            p.resample_sync_ms += ms;
        }
        None => p.resample_async += 1,
    });
}

pub fn note_fonts(count: u32, bytes: u64) {
    if !enabled() {
        return;
    }
    with(|p| {
        p.fonts_registered += count;
        p.font_bytes += bytes;
    });
}

pub fn note_img_src() {
    if !enabled() {
        return;
    }
    with(|p| p.img_srcs += 1);
}

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
