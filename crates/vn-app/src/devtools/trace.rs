//! `trace` — a frame-by-frame record of where the host's time went, for an
//! interaction somebody is about to perform (m26 perf work: "opening a space
//! lags").
//!
//! `perf` answers "how long are frames" over a rolling window and `VN_PERF`
//! prints a per-second phase breakdown to stderr. Neither answers "what did the
//! frame that took 400 ms DO" — whether it was JS, a forced layout, a
//! whole-document restyle a font registration asked for, or taffy. This does:
//!
//! ```text
//!   trace start            arm: every vn_tree::perf counter, one record a frame
//!   (click a space)
//!   trace stop --frames    disarm; the summary, plus every non-idle frame
//! ```
//!
//! A frame is the span between two `pump_frame` entries: the JS wave (timers,
//! rAF, microtasks — fetch completions and Vue's scheduler run there), the
//! forced layouts JS caused inside it, the restyle at its end, gpui's draw of
//! the frame, and the wait for the next vsync. The vn-tree phase counters are
//! drained at the NEXT frame's start, so a frame's record includes its own
//! draw.
//!
//! Idle frames (no JS to speak of, no draw, no tree work) are not stored one by
//! one: the next active frame carries how many there were (`idleBefore`).
//!
//! It shares `vn_tree::perf`'s counters with `VN_PERF`, which drains them once
//! a second for its stderr report; the two would steal each other's numbers,
//! so a trace refuses to start while `VN_PERF` is on.

use std::cell::{Cell, RefCell};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use vn_tree::perf::{DomOp, FullReason, Phases, SyncCaller};

use crate::json::Json;

/// Active frames kept. ~10 minutes of continuous animation at 60 Hz, or any
/// number of interactions separated by idle time.
const DEFAULT_CAP: usize = 20_000;

#[derive(Clone)]
struct Frame {
    /// Since the trace started, at `pump_frame` entry.
    at_ms: f64,
    /// To the next frame's start.
    dur_ms: f32,
    /// `pump_frame` entry → `perf::pump_done`: the whole JS wave, the devtools
    /// command, the media tick, the restyle at its end.
    pump_ms: f32,
    /// `rt.pump(...)` alone: timers, rAF, microtasks.
    js_ms: f32,
    /// `Shell::render` — our lowering of the tree into gpui elements.
    render_ms: f32,
    drew: bool,
    phases: Phases,
    /// gpui line-layout lookups, and the misses that shaped text.
    lookups: u64,
    shapes: u64,
    shape_ms: f32,
    /// Idle frames since the previous stored frame.
    idle_before: u32,
    marks: Vec<String>,
}

struct Trace {
    start: Instant,
    start_epoch_ms: f64,
    cap: usize,
    frames: Vec<Frame>,
    dropped: u64,
    /// The frame in progress.
    cur: Option<(Instant, Frame)>,
    /// When this frame's pump closed; `render` stamps land after it.
    pump_open: Option<Instant>,
    render_open: Option<Instant>,
    idle: u32,
    idle_ms: f64,
    lines: (u64, u64, u64),
    total_frames: u64,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
    static NATIVE_PROFILE_WINDOW: Cell<usize> = const { Cell::new(0) };
}

fn epoch_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

fn active() -> bool {
    TRACE.with(|t| t.borrow().is_some())
}

/// Starts a trace (or restarts one), discarding what was recorded.
pub(crate) fn start(cap: Option<usize>) -> Result<Json, String> {
    if vn_tree::perf::env_enabled() {
        return Err("VN_PERF is on and owns the phase counters (it drains them every second); \
                    a trace would read zeros. Restart the host without VN_PERF."
            .into());
    }
    // Whatever the counters gathered before the trace is not the trace's.
    vn_tree::perf::set_tracing(true);
    let _ = vn_tree::perf::take_all();
    let now = Instant::now();
    let at = epoch_ms();
    TRACE.with(|t| {
        *t.borrow_mut() = Some(Trace {
            start: now,
            start_epoch_ms: at,
            cap: cap.unwrap_or(DEFAULT_CAP).max(16),
            frames: Vec::new(),
            dropped: 0,
            cur: None,
            pump_open: None,
            render_open: None,
            idle: 0,
            idle_ms: 0.0,
            lines: gpui::line_layout_counters(),
            total_frames: 0,
        })
    });
    Ok(Json::obj(vec![("started", Json::Bool(true)), ("startEpochMs", Json::n(at))]))
}

/// A label on the frame in progress (`trace mark`, `__vn_perf_mark`).
pub(crate) fn mark(label: &str) {
    vn_runtime::redjs::profiling::mark(label);
    TRACE.with(|t| {
        if let Some(tr) = t.borrow_mut().as_mut() {
            let at = tr.start.elapsed().as_secs_f64() * 1000.0;
            match tr.cur.as_mut() {
                Some((_, f)) => f.marks.push(label.to_string()),
                // Before the first frame: a frame of its own.
                None => tr.frames.push(Frame::empty(at, vec![label.to_string()])),
            }
        }
    });
}

impl Frame {
    fn empty(at_ms: f64, marks: Vec<String>) -> Frame {
        Frame {
            at_ms,
            dur_ms: 0.0,
            pump_ms: 0.0,
            js_ms: 0.0,
            render_ms: 0.0,
            drew: false,
            phases: Phases::default(),
            lookups: 0,
            shapes: 0,
            shape_ms: 0.0,
            idle_before: 0,
            marks,
        }
    }

    fn idle(&self) -> bool {
        let p = &self.phases;
        self.marks.is_empty()
            && !self.drew
            && self.shapes == 0
            && p.restyle_passes == 0
            && p.build_passes == 0
            && p.draw_passes == 0
            && p.js_passes == 0
            && p.sync_layouts + p.sync_layout_refused == 0
            && p.observer_steps == 0
            && p.dom_ops.iter().all(|&n| n == 0)
            && p.layout_reads + p.style_reads + p.queries + p.scroll_reads == 0
            && self.pump_ms < 2.0
            && self.js_ms < 1.0
    }
}

/// `pump_frame` entry: closes the previous frame and opens this one.
pub(crate) fn frame_start() {
    vn_runtime::redjs::profiling::frame_start();
    if !active() {
        return;
    }
    let now = Instant::now();
    let phases = vn_tree::perf::take_all();
    let lines = gpui::line_layout_counters();
    TRACE.with(|t| {
        let mut t = t.borrow_mut();
        let Some(tr) = t.as_mut() else { return };
        if let Some((opened, mut f)) = tr.cur.take() {
            f.dur_ms = (now - opened).as_secs_f32() * 1000.0;
            f.phases = phases;
            f.lookups = lines.0.saturating_sub(tr.lines.0);
            f.shapes = lines.1.saturating_sub(tr.lines.1);
            f.shape_ms = (lines.2.saturating_sub(tr.lines.2)) as f32 / 1e6;
            tr.total_frames += 1;
            if f.idle() {
                tr.idle += 1;
                tr.idle_ms += f64::from(f.dur_ms);
            } else {
                f.idle_before = std::mem::take(&mut tr.idle);
                if tr.frames.len() >= tr.cap {
                    tr.frames.remove(0);
                    tr.dropped += 1;
                }
                tr.frames.push(f);
            }
        }
        tr.lines = lines;
        let at = (now - tr.start).as_secs_f64() * 1000.0;
        tr.cur = Some((now, Frame::empty(at, Vec::new())));
        tr.pump_open = Some(now);
    });
}

/// `perf::pump_done`: the JS wave and everything after it in `pump_frame`.
pub(crate) fn pump_done() {
    with_cur(|tr, now| {
        if let (Some(open), Some((_, f))) = (tr.pump_open.take(), tr.cur.as_mut()) {
            f.pump_ms += (now - open).as_secs_f32() * 1000.0;
        }
    });
}

/// `rt.pump(...)` took `ms`.
pub(crate) fn note_js(ms: f32) {
    with_cur(|tr, _| {
        if let Some((_, f)) = tr.cur.as_mut() {
            f.js_ms += ms;
        }
    });
}

pub(crate) fn render_begin() {
    with_cur(|tr, now| tr.render_open = Some(now));
}

pub(crate) fn render_done() {
    with_cur(|tr, now| {
        if let (Some(open), Some((_, f))) = (tr.render_open.take(), tr.cur.as_mut()) {
            f.render_ms += (now - open).as_secs_f32() * 1000.0;
            f.drew = true;
        }
    });
}

fn with_cur(f: impl FnOnce(&mut Trace, Instant)) {
    TRACE.with(|t| {
        if let Some(tr) = t.borrow_mut().as_mut() {
            f(tr, Instant::now());
        }
    });
}

/// `trace stop` (`stop = true`) or `trace peek`: the summary, and every stored
/// frame when `frames` is asked for.
pub(crate) fn report(stop: bool, frames: bool) -> Result<Json, String> {
    let taken = TRACE.with(|t| {
        let mut t = t.borrow_mut();
        if stop {
            t.take()
        } else {
            t.as_ref().map(|tr| Trace {
                start: tr.start,
                start_epoch_ms: tr.start_epoch_ms,
                cap: tr.cap,
                frames: tr.frames.clone(),
                dropped: tr.dropped,
                cur: None,
                pump_open: None,
                render_open: None,
                idle: tr.idle,
                idle_ms: tr.idle_ms,
                lines: tr.lines,
                total_frames: tr.total_frames,
            })
        }
    });
    if stop {
        vn_tree::perf::set_tracing(false);
        let _ = vn_tree::perf::take_all();
    }
    let Some(tr) = taken else {
        return Err("no trace is running — `trace start` first".into());
    };
    let mut out = vec![
        ("startEpochMs", Json::n(tr.start_epoch_ms)),
        ("elapsedMs", Json::n(round1(tr.start.elapsed().as_secs_f64() * 1000.0))),
        ("framesTotal", Json::n(tr.total_frames as f64)),
        ("framesStored", Json::n(tr.frames.len() as f64)),
        ("framesIdle", Json::n((tr.total_frames as f64) - tr.frames.len() as f64)),
        ("dropped", Json::n(tr.dropped as f64)),
        ("summary", summary(&tr.frames)),
    ];
    if frames {
        out.push(("frames", Json::arr(tr.frames.iter().map(frame_json).collect())));
    }
    Ok(Json::obj(out))
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn r2(v: f32) -> Json {
    Json::n((f64::from(v) * 100.0).round() / 100.0)
}

fn dom_total(p: &Phases) -> u32 {
    p.dom_ops.iter().sum()
}

fn dom_span_ms(p: &Phases) -> f32 {
    match (p.dom_first, p.dom_last) {
        (Some(a), Some(b)) => (b - a).as_secs_f32() * 1000.0,
        _ => 0.0,
    }
}

/// One frame, with only the groups that saw anything.
fn frame_json(f: &Frame) -> Json {
    let p = &f.phases;
    let mut o: Vec<(&str, Json)> = vec![
        ("t", Json::n(round1(f.at_ms))),
        ("dur", r2(f.dur_ms)),
        ("pump", r2(f.pump_ms)),
        ("js", r2(f.js_ms)),
    ];
    if f.idle_before > 0 {
        o.push(("idleBefore", Json::n(f64::from(f.idle_before))));
    }
    if !f.marks.is_empty() {
        o.push(("marks", Json::arr(f.marks.iter().map(|m| Json::s(m.clone())).collect())));
    }
    if p.js_passes > 0 {
        o.push(("inputJs", Json::obj(vec![("n", Json::n(f64::from(p.js_passes))), ("ms", r2(p.js_ms))])));
    }
    if dom_total(p) > 0 {
        let mut d: Vec<(&str, Json)> = DomOp::NAMES
            .iter()
            .zip(p.dom_ops.iter())
            .filter(|(_, n)| **n > 0)
            .map(|(name, n)| (*name, Json::n(f64::from(*n))))
            .collect();
        d.push(("total", Json::n(f64::from(dom_total(p)))));
        d.push(("spanMs", r2(dom_span_ms(p))));
        o.push(("dom", Json::obj(d)));
    }
    if p.layout_reads + p.scroll_reads + p.style_reads + p.queries + p.style_flushes > 0 {
        o.push((
            "reads",
            Json::obj(vec![
                ("layout", Json::n(f64::from(p.layout_reads))),
                ("scroll", Json::n(f64::from(p.scroll_reads))),
                ("style", Json::n(f64::from(p.style_reads))),
                ("query", Json::n(f64::from(p.queries))),
                ("styleFlushes", Json::n(f64::from(p.style_flushes))),
            ]),
        ));
    }
    if p.sync_layouts + p.sync_layout_refused > 0 {
        let by: Vec<(&str, Json)> = SyncCaller::NAMES
            .iter()
            .zip(p.sync_by.iter())
            .filter(|(_, n)| **n > 0)
            .map(|(name, n)| (*name, Json::n(f64::from(*n))))
            .collect();
        o.push((
            "sync",
            Json::obj(vec![
                ("n", Json::n(f64::from(p.sync_layouts))),
                ("ms", r2(p.sync_layout_ms)),
                ("maxMs", r2(p.sync_layout_max_ms)),
                ("refused", Json::n(f64::from(p.sync_layout_refused))),
                // The wave ended with its changes already drawn by a forced
                // layout; `tailFree`: and nothing else asked for a draw.
                ("tail", Json::n(f64::from(p.sync_tail))),
                ("tailFree", Json::n(f64::from(p.sync_tail_free))),
                ("by", Json::obj(by)),
            ]),
        ));
    }
    if p.observer_steps > 0 {
        o.push(("observers", observers_json(p.observer_steps, p.observer_rounds, p.observer_relayouts, p.observer_relayout_ms, p.observer_ms, p.observer_loop_errors)));
    }
    if p.restyle_passes > 0 || p.recompiles > 0 || p.full_reasons.iter().any(|&n| n > 0) {
        let reasons: Vec<(&str, Json)> = FullReason::NAMES
            .iter()
            .zip(p.full_reasons.iter())
            .filter(|(_, n)| **n > 0)
            .map(|(name, n)| (*name, Json::n(f64::from(*n))))
            .collect();
        o.push((
            "restyle",
            Json::obj(vec![
                ("n", Json::n(f64::from(p.restyle_passes))),
                ("full", Json::n(f64::from(p.restyle_full))),
                ("ms", r2(p.restyle_ms)),
                ("maxMs", r2(p.restyle_max_ms)),
                ("nodes", Json::n(p.nodes_cascaded as f64)),
                ("changed", Json::n(p.nodes_changed as f64)),
                ("matchMs", r2(p.match_ms)),
                ("varsMs", r2(p.vars_ms)),
                ("foldMs", r2(p.fold_ms)),
                ("inlineMs", r2(p.inline_ms)),
                ("storeMs", r2(p.store_ms)),
                ("selectors", Json::n(p.rules_tested as f64)),
                ("recompiles", Json::n(f64::from(p.recompiles))),
                ("recompileMs", r2(p.recompile_ms)),
                ("why", Json::obj(reasons)),
            ]),
        ));
    }
    if p.build_passes > 0 || f.drew {
        o.push((
            "build",
            Json::obj(vec![
                ("n", Json::n(f64::from(p.build_passes))),
                ("ms", r2(p.build_ms)),
                ("elements", Json::n(p.elements_built as f64)),
                ("renderMs", r2(f.render_ms)),
                ("cache", Json::arr(vec![
                    Json::n(f64::from(p.cache_offers.saturating_sub(p.cached_renders))),
                    Json::n(f64::from(p.cached_renders)),
                    Json::n(f64::from(p.cache_standdowns)),
                ])),
            ]),
        ));
    }
    if p.draw_passes > 0 {
        o.push((
            "draw",
            Json::obj(vec![
                ("n", Json::n(f64::from(p.draw_passes))),
                ("layoutReq", r2(p.layout_req_ms)),
                ("taffy", r2(p.taffy_ms)),
                ("prepaint", r2(p.prepaint_ms)),
                ("paint", r2(p.paint_ms)),
            ]),
        ));
    }
    if f.shapes > 0 {
        o.push((
            "text",
            Json::obj(vec![
                ("lookups", Json::n(f.lookups as f64)),
                ("shapes", Json::n(f.shapes as f64)),
                ("shapeMs", r2(f.shape_ms)),
            ]),
        ));
    }
    if p.resample_sync + p.resample_async > 0 {
        o.push((
            "resample",
            Json::obj(vec![
                ("sync", Json::n(f64::from(p.resample_sync))),
                ("syncMs", r2(p.resample_sync_ms)),
                ("async", Json::n(f64::from(p.resample_async))),
            ]),
        ));
    }
    if p.fonts_registered > 0 {
        o.push((
            "fonts",
            Json::obj(vec![
                ("registered", Json::n(f64::from(p.fonts_registered))),
                ("bytes", Json::n(p.font_bytes as f64)),
            ]),
        ));
    }
    if p.img_srcs > 0 {
        o.push(("imgSrcs", Json::n(f64::from(p.img_srcs))));
    }
    Json::obj(o)
}

/// The ResizeObserver / IntersectionObserver step after layout, per frame and
/// in total (`vn_tree::perf::Phases::observer_steps` and the rest).
fn observers_json(steps: u32, rounds: u32, relayouts: u32, relayout_ms: f32, ms: f32, loop_errors: u32) -> Json {
    Json::obj(vec![
        ("n", Json::n(f64::from(steps))),
        ("rounds", Json::n(f64::from(rounds))),
        ("relayouts", Json::n(f64::from(relayouts))),
        ("relayoutMs", r2(relayout_ms)),
        ("ms", r2(ms)),
        ("loopErrors", Json::n(f64::from(loop_errors))),
    ])
}

/// Totals over the stored frames, and the ten longest.
fn summary(frames: &[Frame]) -> Json {
    let mut s = Totals::default();
    for f in frames {
        s.add(f);
    }
    let mut longest: Vec<&Frame> = frames.iter().collect();
    longest.sort_by(|a, b| b.dur_ms.partial_cmp(&a.dur_ms).unwrap_or(std::cmp::Ordering::Equal));
    longest.truncate(10);
    let over = |ms: f32| frames.iter().filter(|f| f.dur_ms > ms).count() as f64;
    let reasons: Vec<(&str, Json)> = FullReason::NAMES
        .iter()
        .zip(s.full_reasons.iter())
        .map(|(name, n)| (*name, Json::n(f64::from(*n))))
        .collect();
    let by: Vec<(&str, Json)> = SyncCaller::NAMES
        .iter()
        .zip(s.sync_by.iter())
        .map(|(name, n)| (*name, Json::n(f64::from(*n))))
        .collect();
    let dom: Vec<(&str, Json)> = DomOp::NAMES
        .iter()
        .zip(s.dom.iter())
        .map(|(name, n)| (*name, Json::n(*n as f64)))
        .collect();
    Json::obj(vec![
        ("frames", Json::n(frames.len() as f64)),
        ("over16ms", Json::n(over(16.7))),
        ("over33ms", Json::n(over(33.4))),
        ("over100ms", Json::n(over(100.0))),
        ("sumDurMs", r2(s.dur)),
        ("pumpMs", r2(s.pump)),
        ("jsMs", r2(s.js)),
        ("inputJsMs", r2(s.input_js)),
        ("renderMs", r2(s.render)),
        ("domOps", Json::obj(dom)),
        ("layoutReads", Json::n(s.layout_reads as f64)),
        ("styleReads", Json::n(s.style_reads as f64)),
        ("styleFlushes", Json::n(s.style_flushes as f64)),
        ("queries", Json::n(s.queries as f64)),
        (
            "sync",
            Json::obj(vec![
                ("n", Json::n(s.sync as f64)),
                ("ms", r2(s.sync_ms)),
                ("maxMs", r2(s.sync_max)),
                ("refused", Json::n(s.sync_refused as f64)),
                ("tail", Json::n(s.sync_tail as f64)),
                ("tailFree", Json::n(s.sync_tail_free as f64)),
                ("maxPerFrame", Json::n(s.sync_per_frame_max as f64)),
                ("by", Json::obj(by)),
            ]),
        ),
        (
            "observers",
            observers_json(s.observer_steps, s.observer_rounds, s.observer_relayouts, s.observer_relayout_ms, s.observer_ms, s.observer_loop_errors),
        ),
        (
            "restyle",
            Json::obj(vec![
                ("n", Json::n(s.restyles as f64)),
                ("full", Json::n(s.restyle_full as f64)),
                ("ms", r2(s.restyle_ms)),
                ("matchMs", r2(s.match_ms)),
                ("nodes", Json::n(s.nodes as f64)),
                ("changed", Json::n(s.changed as f64)),
                ("recompiles", Json::n(s.recompiles as f64)),
                ("recompileMs", r2(s.recompile_ms)),
                ("why", Json::obj(reasons)),
            ]),
        ),
        (
            "build",
            Json::obj(vec![
                ("n", Json::n(s.builds as f64)),
                ("ms", r2(s.build_ms)),
                ("elements", Json::n(s.elements as f64)),
            ]),
        ),
        (
            "draw",
            Json::obj(vec![
                ("n", Json::n(s.draws as f64)),
                ("layoutReq", r2(s.layout_req)),
                ("taffy", r2(s.taffy)),
                ("prepaint", r2(s.prepaint)),
                ("paint", r2(s.paint)),
            ]),
        ),
        (
            "text",
            Json::obj(vec![
                ("lookups", Json::n(s.lookups as f64)),
                ("shapes", Json::n(s.shapes as f64)),
                ("shapeMs", r2(s.shape_ms)),
            ]),
        ),
        (
            "resample",
            Json::obj(vec![
                ("sync", Json::n(s.resample_sync as f64)),
                ("syncMs", r2(s.resample_sync_ms)),
                ("async", Json::n(s.resample_async as f64)),
            ]),
        ),
        ("fontsRegistered", Json::n(s.fonts as f64)),
        ("imgSrcs", Json::n(s.img_srcs as f64)),
        (
            "longest",
            Json::arr(
                longest
                    .iter()
                    .map(|f| {
                        let p = &f.phases;
                        Json::obj(vec![
                            ("t", Json::n(round1(f.at_ms))),
                            ("dur", r2(f.dur_ms)),
                            ("pump", r2(f.pump_ms)),
                            ("js", r2(f.js_ms)),
                            ("inputJs", r2(p.js_ms)),
                            ("sync", Json::n(f64::from(p.sync_layouts))),
                            ("syncMs", r2(p.sync_layout_ms)),
                            ("restyleMs", r2(p.restyle_ms)),
                            ("restyleFull", Json::n(f64::from(p.restyle_full))),
                            ("recompileMs", r2(p.recompile_ms)),
                            ("buildMs", r2(p.build_ms)),
                            ("draws", Json::n(f64::from(p.draw_passes))),
                            ("taffy", r2(p.taffy_ms)),
                            ("prepaint", r2(p.prepaint_ms)),
                            ("paint", r2(p.paint_ms)),
                            ("shapeMs", r2(f.shape_ms)),
                            ("dom", Json::n(f64::from(dom_total(p)))),
                            ("marks", Json::arr(f.marks.iter().map(|m| Json::s(m.clone())).collect())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

#[derive(Default)]
struct Totals {
    dur: f32,
    pump: f32,
    js: f32,
    input_js: f32,
    render: f32,
    dom: [u64; DomOp::COUNT],
    layout_reads: u64,
    style_reads: u64,
    style_flushes: u64,
    queries: u64,
    sync: u64,
    sync_ms: f32,
    sync_max: f32,
    sync_refused: u64,
    sync_tail: u64,
    sync_tail_free: u64,
    sync_per_frame_max: u32,
    observer_steps: u32,
    observer_rounds: u32,
    observer_relayouts: u32,
    observer_relayout_ms: f32,
    observer_ms: f32,
    observer_loop_errors: u32,
    sync_by: [u32; SyncCaller::COUNT],
    restyles: u64,
    restyle_full: u64,
    restyle_ms: f32,
    match_ms: f32,
    nodes: u64,
    changed: u64,
    recompiles: u64,
    recompile_ms: f32,
    full_reasons: [u32; FullReason::COUNT],
    builds: u64,
    build_ms: f32,
    elements: u64,
    draws: u64,
    layout_req: f32,
    taffy: f32,
    prepaint: f32,
    paint: f32,
    lookups: u64,
    shapes: u64,
    shape_ms: f32,
    resample_sync: u64,
    resample_sync_ms: f32,
    resample_async: u64,
    fonts: u64,
    img_srcs: u64,
}

impl Totals {
    fn add(&mut self, f: &Frame) {
        let p = &f.phases;
        self.dur += f.dur_ms;
        self.pump += f.pump_ms;
        self.js += f.js_ms;
        self.input_js += p.js_ms;
        self.render += f.render_ms;
        for (a, b) in self.dom.iter_mut().zip(p.dom_ops.iter()) {
            *a += u64::from(*b);
        }
        self.layout_reads += u64::from(p.layout_reads);
        self.style_reads += u64::from(p.style_reads);
        self.style_flushes += u64::from(p.style_flushes);
        self.queries += u64::from(p.queries);
        self.sync += u64::from(p.sync_layouts);
        self.sync_ms += p.sync_layout_ms;
        self.sync_max = self.sync_max.max(p.sync_layout_max_ms);
        self.sync_refused += u64::from(p.sync_layout_refused);
        self.sync_tail += u64::from(p.sync_tail);
        self.sync_tail_free += u64::from(p.sync_tail_free);
        self.observer_steps += p.observer_steps;
        self.observer_rounds += p.observer_rounds;
        self.observer_relayouts += p.observer_relayouts;
        self.observer_relayout_ms += p.observer_relayout_ms;
        self.observer_ms += p.observer_ms;
        self.observer_loop_errors += p.observer_loop_errors;
        self.sync_per_frame_max = self.sync_per_frame_max.max(p.sync_layouts);
        for (a, b) in self.sync_by.iter_mut().zip(p.sync_by.iter()) {
            *a += *b;
        }
        self.restyles += u64::from(p.restyle_passes);
        self.restyle_full += u64::from(p.restyle_full);
        self.restyle_ms += p.restyle_ms;
        self.match_ms += p.match_ms;
        self.nodes += p.nodes_cascaded;
        self.changed += p.nodes_changed;
        self.recompiles += u64::from(p.recompiles);
        self.recompile_ms += p.recompile_ms;
        for (a, b) in self.full_reasons.iter_mut().zip(p.full_reasons.iter()) {
            *a += *b;
        }
        self.builds += u64::from(p.build_passes);
        self.build_ms += p.build_ms;
        self.elements += p.elements_built;
        self.draws += u64::from(p.draw_passes);
        self.layout_req += p.layout_req_ms;
        self.taffy += p.taffy_ms;
        self.prepaint += p.prepaint_ms;
        self.paint += p.paint_ms;
        self.lookups += f.lookups;
        self.shapes += f.shapes;
        self.shape_ms += f.shape_ms;
        self.resample_sync += u64::from(p.resample_sync);
        self.resample_sync_ms += p.resample_sync_ms;
        self.resample_async += u64::from(p.resample_async);
        self.fonts += u64::from(p.fonts_registered);
        self.img_srcs += u64::from(p.img_srcs);
    }
}


pub(crate) fn recording_start(cap: Option<usize>) -> Result<Json, String> {
    let limits = vn_runtime::redjs::profiling::Limits {
        frames: cap.unwrap_or(1200), ..Default::default()
    };
    let context = vn_runtime::JsRuntime::try_with(|runtime| runtime.ctx().raw() as usize);
    vn_tree::perf::set_tracing(true);
    let status = vn_runtime::redjs::profiling::start(limits, context);
    #[cfg(target_os = "windows")]
    gpui_windows::set_frame_profiler(Some(gpui_windows::FrameProfilerCallbacks { begin: native_profile_begin, end: vn_runtime::redjs::profiling::external_end }));
    Ok(recording_status_json(&status))
}

pub(crate) fn recording_stop() -> Result<Json, String> {
    #[cfg(target_os = "windows")]
    gpui_windows::set_frame_profiler(None);
    let status = vn_runtime::redjs::profiling::stop().ok_or_else(|| "no frame recording exists".to_string())?;
    if !active() { vn_tree::perf::set_tracing(false); }
    Ok(recording_status_json(&status))
}

pub(crate) fn recording_status() -> Result<Json, String> {
    let status = vn_runtime::redjs::profiling::status().unwrap_or(vn_runtime::redjs::profiling::Status {
        recording: false, session_id: 0, start_epoch_ms: 0., limits: Default::default(), frame_count: 0, dropped_frames: 0, dropped_spans: 0,
    });
    Ok(recording_status_json(&status))
}

pub(crate) fn cancel_recording() {
    #[cfg(target_os = "windows")]
    gpui_windows::set_frame_profiler(None);
    NATIVE_PROFILE_WINDOW.with(|target| target.set(0));
    vn_runtime::redjs::profiling::clear();
    TRACE.with(|trace| { trace.borrow_mut().take(); });
    vn_tree::perf::set_tracing(false);
}

pub(crate) fn frame_page(offset: usize, limit: usize) -> Result<Json, String> {
    let page = vn_runtime::redjs::profiling::frames(offset, limit).ok_or_else(|| "no frame recording exists".to_string())?;
    Ok(Json::obj(vec![
        ("sessionId", Json::n(page.status.session_id as f64)), ("recording", Json::Bool(page.status.recording)),
        ("total", Json::n(page.total as f64)), ("offset", Json::n(page.offset as f64)),
        ("nextOffset", offset_json(page.next_offset)),
        ("frames", Json::arr(page.frames.iter().map(recording_frame_json).collect())),
    ]))
}

pub(crate) fn frame_detail(frame_id: u64, offset: usize, limit: usize) -> Result<Json, String> {
    let page = vn_runtime::redjs::profiling::frame_detail(frame_id, offset, limit)
        .ok_or_else(|| "frame is unavailable (it may have been evicted by the recording limits)".to_string())?;
    Ok(Json::obj(vec![
        ("sessionId", Json::n(page.session_id as f64)), ("frame", recording_frame_json(&page.frame)),
        ("offset", Json::n(page.offset as f64)), ("total", Json::n(page.total as f64)),
        ("nextOffset", offset_json(page.next_offset)), ("truncated", Json::Bool(page.truncated)),
        ("spans", Json::arr(page.spans.iter().map(recording_span_json).collect())),
    ]))
}

fn offset_json(offset: Option<usize>) -> Json { offset.map_or(Json::Null, |offset| Json::n(offset as f64)) }
fn milliseconds(value: f64) -> Json { Json::n((value * 1000.).round() / 1000.) }
fn recording_status_json(status: &vn_runtime::redjs::profiling::Status) -> Json {
    Json::obj(vec![
        ("recording", Json::Bool(status.recording)), ("sessionId", Json::n(status.session_id as f64)),
        ("startEpochMs", Json::n(status.start_epoch_ms)), ("frameCount", Json::n(status.frame_count as f64)),
        ("droppedFrames", Json::n(status.dropped_frames as f64)), ("droppedSpans", Json::n(status.dropped_spans as f64)),
        ("limits", Json::obj(vec![("frames", Json::n(status.limits.frames as f64)), ("spansPerFrame", Json::n(status.limits.spans_per_frame as f64)), ("totalSpans", Json::n(status.limits.total_spans as f64))])),
        ("timing", Json::s("instrumented main-thread elapsed time; not GPU timestamps or sampled CPU cycles")),
        ("coverage", Json::obj(vec![("javascript", Json::s("native-invoked callbacks and their host calls")), ("microtasks", Json::s("checkpoint batches; individual Promise reactions are not sampled"))])),
    ])
}
fn recording_frame_json(frame: &vn_runtime::redjs::profiling::FrameSummary) -> Json {
    Json::obj(vec![
        ("id", Json::n(frame.id as f64)), ("startMs", milliseconds(frame.start_ms)),
        ("durationMs", milliseconds(frame.duration_ms)), ("cpuMs", milliseconds(frame.cpu_ms)),
        ("spanCount", Json::n(frame.span_count as f64)), ("droppedSpans", Json::n(frame.dropped_spans as f64)),
        ("categories", Json::arr(frame.categories.iter().map(|category| Json::obj(vec![
            ("category", Json::s(category.category)), ("selfMs", milliseconds(category.self_ms)),
            ("totalMs", milliseconds(category.total_ms)), ("count", Json::n(category.count as f64)),
        ])).collect())),
        ("hints", Json::arr(frame.hints.iter().map(|hint| {
            let mut values = vec![("code", Json::s(hint.code)), ("severity", Json::s(hint.severity)), ("message", Json::s(&hint.message)), ("count", Json::n(hint.count as f64))];
            if let Some(id) = hint.span_id { values.push(("spanId", Json::n(id as f64))); }
            Json::obj(values)
        }).collect())),
    ])
}
fn recording_span_json(span: &vn_runtime::redjs::profiling::EventSpan) -> Json {
    let mut values = vec![
        ("id", Json::n(span.id as f64)), ("parentId", span.parent_id.map_or(Json::Null, |id| Json::n(id as f64))),
        ("category", Json::s(span.category)), ("name", Json::s(&span.name)),
        ("startMs", milliseconds(span.start_ms)), ("durationMs", milliseconds(span.duration_ms)), ("selfMs", milliseconds(span.self_ms)),
        ("depth", Json::n(span.depth as f64)), ("count", Json::n(1.)),
    ];
    if let Some(site) = &span.callsite {
        let mut callsite = vec![("kind", Json::s(site.kind))];
        if !site.function.is_empty() { callsite.push(("function", Json::s(&site.function))); }
        if !site.url.is_empty() { callsite.push(("url", Json::s(&site.url))); }
        if site.line > 0 { callsite.push(("line", Json::n(f64::from(site.line)))); }
        if site.column > 0 { callsite.push(("column", Json::n(f64::from(site.column)))); }
        values.push(("callsite", Json::obj(callsite)));
    }
    if !span.counters.is_empty() {
        values.push(("counters", Json::Obj(span.counters.iter().map(|(name, value)| (name.clone(), Json::n(*value as f64))).collect())));
    }
    Json::obj(values)
}


pub(crate) fn set_recording_native_window(window: usize) { NATIVE_PROFILE_WINDOW.with(|target| target.set(window)); }
#[cfg(target_os = "windows")]
fn native_profile_begin(window: usize, category: &'static str, name: &'static str, file: &'static str, line: u32, column: u32) -> u64 {
    if !NATIVE_PROFILE_WINDOW.with(|target| target.get() != 0 && target.get() == window) { return 0; }
    vn_runtime::redjs::profiling::external_begin(category, name, file, line, column)
}

pub(crate) fn set_recording_window_id(window: u64) { vn_runtime::redjs::profiling::set_window_target(window); }
