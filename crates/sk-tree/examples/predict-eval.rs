//! Replays pointer traces through the pointer predictors and compares them.
//!
//! ```text
//! cargo run --target x86_64-pc-windows-msvc -p sk-tree --example predict-eval -- [--latency MS] [--synthetic DIR] TRACE.json...
//! ```
//!
//! A trace is what `apps/pen-draw` records: `{ "strokes": [{ "pointerType", "down", "moves":
//! [{ "t", "readings": [{ "x", "y", "t", ... }] }], "up" }] }`, CSS px and ms. Each move's
//! readings are fed as one delivered `pointermove`, as the host does. `--synthetic DIR` writes
//! generated traces to DIR and evaluates them as well.
//!
//! The scores follow Chromium's `ui/base/prediction/prediction_metrics_handler.cc`: a predicted
//! point's error is its distance to the real position at its time (interpolated between
//! readings), counted as over-prediction when it lies ahead along the direction of motion and as
//! under-prediction otherwise. Frame scores take the tip a frame draws (the last prediction, or
//! the last reading without one) against the real position `--latency` ms after the last
//! reading, and jitter is how much that error changes from one frame to the next. Hooks are the
//! overshoot at sharp turns (the largest error of a prediction past the turn) and where the
//! pointer comes to rest or lifts (the furthest a prediction reaches past the rest point along the
//! incoming direction), from the frames of the 50 ms before it ("stopping") and after it
//! ("stopped", when the readings already show the pointer still).
//!
//! `ink 25ms` has upstream's alternate horizon, and `chrome 16.7ms` is Chromium's predictor cut to
//! the ink predictor's, to tell what the horizon does from what the confidence does.

use std::f64::consts::PI;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sk_tree::predict::{chrome, ink, Predictor};

#[derive(Clone, Copy, Debug)]
struct Reading {
    x: f64,
    y: f64,
    t: f64,
}

struct Stroke {
    down: Option<Reading>,
    moves: Vec<Vec<Reading>>,
    up: Option<Reading>,
}

fn reading(v: &Value) -> Option<Reading> {
    Some(Reading {
        x: v.get("x")?.as_f64()?,
        y: v.get("y")?.as_f64()?,
        t: v.get("t")?.as_f64()?,
    })
}

fn load(path: &Path) -> Result<Vec<Stroke>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let strokes = doc
        .get("strokes")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: no \"strokes\" array", path.display()))?;
    Ok(strokes
        .iter()
        .map(|s| Stroke {
            down: s.get("down").and_then(reading),
            moves: s
                .get("moves")
                .and_then(Value::as_array)
                .map(|moves| {
                    moves
                        .iter()
                        .map(|m| {
                            m.get("readings")
                                .and_then(Value::as_array)
                                .map(|r| r.iter().filter_map(reading).collect())
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .unwrap_or_default(),
            up: s.get("up").and_then(reading),
        })
        .collect())
}

/// The real path: every reading, times strictly increasing.
fn path(stroke: &Stroke) -> Vec<Reading> {
    let mut out: Vec<Reading> = Vec::new();
    let all = stroke
        .down
        .iter()
        .chain(stroke.moves.iter().flatten())
        .chain(stroke.up.iter());
    for &r in all {
        match out.last() {
            Some(last) if r.t <= last.t => {}
            _ => out.push(r),
        }
    }
    out
}

/// The real position at `t`, and the next reading after it.
fn real_at(path: &[Reading], t: f64) -> Option<((f64, f64), (f64, f64))> {
    let i = path.partition_point(|r| r.t <= t);
    if i == 0 || i == path.len() {
        return None;
    }
    let (a, b) = (path[i - 1], path[i]);
    let k = (t - a.t) / (b.t - a.t);
    Some(((a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k), (b.x, b.y)))
}

fn position_at(path: &[Reading], t: f64) -> (f64, f64) {
    match real_at(path, t) {
        Some((p, _)) => p,
        None if t < path[0].t => (path[0].x, path[0].y),
        None => {
            let last = path[path.len() - 1];
            (last.x, last.y)
        }
    }
}

/// Chromium's signed error: positive ahead of the real position along the motion.
fn signed_error(predicted: (f64, f64), real: (f64, f64), next: (f64, f64)) -> f64 {
    let e = (predicted.0 - real.0, predicted.1 - real.1);
    let d = (next.0 - real.0, next.1 - real.1);
    let len = e.0.hypot(e.1);
    if e.0 * d.0 + e.1 * d.1 >= 0.0 {
        len
    } else {
        -len
    }
}

fn unit(v: (f64, f64)) -> Option<(f64, f64)> {
    let len = v.0.hypot(v.1);
    (len > 1e-9).then(|| (v.0 / len, v.1 / len))
}

/// Where the pointer comes to rest (or lifts) after moving: time, position, incoming direction.
fn rests(path: &[Reading]) -> Vec<(f64, (f64, f64), (f64, f64))> {
    const STILL: f64 = 1.5;
    const HOLD: f64 = 40.0;
    const BEFORE: f64 = 20.0;
    const MOVING: f64 = 3.0;
    let mut out = Vec::new();
    let mut i = 0;
    while i < path.len() {
        let r = path[i];
        let still = path[i..]
            .iter()
            .take_while(|s| s.t <= r.t + HOLD)
            .all(|s| (s.x - r.x).hypot(s.y - r.y) <= STILL);
        let before = position_at(path, r.t - BEFORE);
        let incoming = (r.x - before.0, r.y - before.1);
        if still && r.t - BEFORE >= path[0].t && incoming.0.hypot(incoming.1) > MOVING {
            if let Some(dir) = unit(incoming) {
                out.push((r.t, (r.x, r.y), dir));
            }
            while i < path.len() && (path[i].x - r.x).hypot(path[i].y - r.y) <= STILL {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Sharp turns: the direction over 12 px of path before and after a reading differs by more
/// than 60°.
fn turns(path: &[Reading]) -> Vec<f64> {
    const SPAN: f64 = 12.0;
    const MIN_ANGLE: f64 = PI / 3.0;
    let mut length = vec![0.0];
    for w in path.windows(2) {
        length.push(length[length.len() - 1] + (w[1].x - w[0].x).hypot(w[1].y - w[0].y));
    }
    let total = length[length.len() - 1];
    let at_length = |s: f64| {
        let i = length.partition_point(|&l| l <= s).clamp(1, path.len() - 1);
        let (a, b) = (path[i - 1], path[i]);
        let k = ((s - length[i - 1]) / (length[i] - length[i - 1]).max(1e-9)).clamp(0.0, 1.0);
        (a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k)
    };
    let mut candidates: Vec<(f64, f64, f64)> = Vec::new();
    for (r, &s) in path.iter().zip(&length) {
        if s < SPAN || s + SPAN > total {
            continue;
        }
        let a = at_length(s - SPAN);
        let b = at_length(s + SPAN);
        let u = (r.x - a.0, r.y - a.1);
        let v = (b.0 - r.x, b.1 - r.y);
        let (lu, lv) = (u.0.hypot(u.1), v.0.hypot(v.1));
        if lu < SPAN / 2.0 || lv < SPAN / 2.0 {
            continue;
        }
        let angle = ((u.0 * v.0 + u.1 * v.1) / (lu * lv))
            .clamp(-1.0, 1.0)
            .acos();
        if angle > MIN_ANGLE {
            candidates.push((s, r.t, angle));
        }
    }
    let mut out: Vec<(f64, f64, f64)> = Vec::new();
    for (s, t, angle) in candidates {
        match out.last_mut() {
            Some(last) if s - last.0 < 2.0 * SPAN => {
                if angle > last.2 {
                    *last = (s, t, angle);
                }
            }
            _ => out.push((s, t, angle)),
        }
    }
    out.into_iter().map(|(_, t, _)| t).collect()
}

#[derive(Default)]
struct Score {
    over: Vec<f64>,
    under: Vec<f64>,
}

impl Score {
    fn add(&mut self, signed: f64) {
        if signed >= 0.0 {
            self.over.push(signed);
        } else {
            self.under.push(-signed);
        }
    }

    fn all(&self) -> impl Iterator<Item = f64> + '_ {
        self.over.iter().chain(&self.under).copied()
    }
}

#[derive(Default)]
struct Metrics {
    frames: usize,
    predicted_frames: usize,
    points: usize,
    horizon: Vec<f64>,
    point: Score,
    frame: Score,
    jitter: Vec<f64>,
    turn_hooks: Vec<f64>,
    /// Predictions made in the 50 ms before the pointer comes to rest.
    stopping_hooks: Vec<f64>,
    /// Predictions made in the 50 ms after.
    stopped_hooks: Vec<f64>,
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * p).round() as usize]
}

struct Frame {
    last: Reading,
    predicted: Vec<(f64, f64, f64)>,
}

fn replay(predictor: &mut dyn Predictor, stroke: &Stroke) -> Vec<Frame> {
    predictor.reset();
    let mut frames = Vec::new();
    for readings in &stroke.moves {
        let Some(&last) = readings.last() else {
            continue;
        };
        for r in readings {
            predictor.update(r.x, r.y, r.t);
        }
        frames.push(Frame {
            last,
            predicted: predictor.predictions(),
        });
    }
    frames
}

fn evaluate(m: &mut Metrics, stroke: &Stroke, frames: &[Frame], latency: f64) {
    let path = path(stroke);
    if path.len() < 2 {
        return;
    }
    let mut previous: Option<(f64, f64)> = None;
    for f in frames {
        m.frames += 1;
        if !f.predicted.is_empty() {
            m.predicted_frames += 1;
            m.horizon
                .push(f.predicted[f.predicted.len() - 1].2 - f.last.t);
        }
        for &(x, y, t) in &f.predicted {
            m.points += 1;
            if let Some((real, next)) = real_at(&path, t) {
                m.point.add(signed_error((x, y), real, next));
            }
        }
        let tip = f
            .predicted
            .last()
            .map_or((f.last.x, f.last.y), |&(x, y, _)| (x, y));
        match real_at(&path, f.last.t + latency) {
            Some((real, next)) => {
                m.frame.add(signed_error(tip, real, next));
                let error = (tip.0 - real.0, tip.1 - real.1);
                if let Some(p) = previous {
                    m.jitter.push((error.0 - p.0).hypot(error.1 - p.1));
                }
                previous = Some(error);
            }
            None => previous = None,
        }
    }
    for t in turns(&path) {
        let hook = frames
            .iter()
            .filter(|f| f.last.t >= t - 30.0 && f.last.t <= t + 5.0)
            .flat_map(|f| &f.predicted)
            .filter(|p| p.2 > t)
            .filter_map(|&(x, y, pt)| {
                real_at(&path, pt).map(|(real, _)| (x - real.0).hypot(y - real.1))
            })
            .fold(0.0, f64::max);
        m.turn_hooks.push(hook);
    }
    for (t, at, dir) in rests(&path) {
        let past = |from: f64, to: f64| {
            frames
                .iter()
                .filter(|f| f.last.t > from && f.last.t <= to)
                .flat_map(|f| &f.predicted)
                .filter(|p| p.2 > t)
                .map(|&(x, y, _)| ((x - at.0) * dir.0 + (y - at.1) * dir.1).max(0.0))
                .fold(0.0, f64::max)
        };
        m.stopping_hooks.push(past(t - 50.0, t));
        m.stopped_hooks.push(past(t, t + 50.0));
    }
}

fn hooks(v: &[f64]) -> String {
    format!(
        "{:.2} / {:.2} n={}",
        mean(v),
        v.iter().copied().fold(0.0, f64::max),
        v.len()
    )
}

/// A predictor's predictions up to `horizon` ms past the latest reading.
struct Capped<P> {
    inner: P,
    horizon: f64,
    last: f64,
}

impl<P: Predictor> Predictor for Capped<P> {
    fn reset(&mut self) {
        self.inner.reset();
    }

    fn update(&mut self, x: f64, y: f64, time: f64) {
        self.last = time;
        self.inner.update(x, y, time);
    }

    fn predictions(&self) -> Vec<(f64, f64, f64)> {
        let mut out = self.inner.predictions();
        out.retain(|p| p.2 - self.last <= self.horizon + 1e-9);
        out
    }
}

/// The pointer events' predictor; with the 25 ms horizon and 200 Hz output of upstream's
/// `AlternateParams` test; Chromium's; and Chromium's cut to the ink predictor's longest horizon.
fn predictors() -> Vec<(&'static str, Box<dyn Predictor>)> {
    vec![
        ("ink", Box::new(ink::KalmanPredictor::default())),
        (
            "ink 25ms",
            Box::new(ink::KalmanPredictor::with_params(ink::Params {
                prediction_interval: 0.025,
                min_output_rate: 200.0,
                ..ink::DEFAULT
            })),
        ),
        ("chrome", Box::new(chrome::KalmanPredictor::default())),
        (
            "chrome 16.7ms",
            Box::new(Capped {
                inner: chrome::KalmanPredictor::default(),
                horizon: ink::DEFAULT.prediction_interval * 1000.0,
                last: 0.0,
            }),
        ),
    ]
}

fn report(title: &str, results: &[(&str, Metrics)], latency: f64) -> String {
    type Row = (&'static str, fn(&Metrics) -> String);
    let rows: &[Row] = &[
        ("frames with a prediction", |m| {
            format!("{}/{}", m.predicted_frames, m.frames)
        }),
        ("mean horizon (ms)", |m| format!("{:.1}", mean(&m.horizon))),
        ("predicted points", |m| m.points.to_string()),
        ("OverPrediction mean (px)", |m| {
            format!("{:.2}", mean(&m.point.over))
        }),
        ("UnderPrediction mean (px)", |m| {
            format!("{:.2}", mean(&m.point.under))
        }),
        ("PredictionScore mean (px)", |m| {
            format!("{:.2}", mean(&m.point.all().collect::<Vec<_>>()))
        }),
        ("PredictionScore p95 (px)", |m| {
            format!(
                "{:.2}",
                percentile(&m.point.all().collect::<Vec<_>>(), 0.95)
            )
        }),
        ("FrameOverPrediction mean", |m| {
            format!("{:.2}", mean(&m.frame.over))
        }),
        ("FrameUnderPrediction mean", |m| {
            format!("{:.2}", mean(&m.frame.under))
        }),
        ("FramePredictionScore mean", |m| {
            format!("{:.2}", mean(&m.frame.all().collect::<Vec<_>>()))
        }),
        ("FramePredictionScore p95", |m| {
            format!(
                "{:.2}",
                percentile(&m.frame.all().collect::<Vec<_>>(), 0.95)
            )
        }),
        ("VisualJitter mean (px)", |m| {
            format!("{:.2}", mean(&m.jitter))
        }),
        ("turn hook mean / max (px)", |m| hooks(&m.turn_hooks)),
        ("stopping hook mean / max", |m| hooks(&m.stopping_hooks)),
        ("stopped hook mean / max", |m| hooks(&m.stopped_hooks)),
    ];
    let mut out = String::new();
    let _ = writeln!(out, "\n{title} (frame scores at {latency:.1} ms)");
    let _ = write!(out, "{:<28}", "");
    for (name, _) in results {
        let _ = write!(out, "{name:>20}");
    }
    out.push('\n');
    for &(label, value) in rows {
        let _ = write!(out, "{label:<28}");
        for (_, m) in results {
            let _ = write!(out, "{:>20}", value(m));
        }
        out.push('\n');
    }
    out
}

/// A small deterministic generator, so synthetic traces are the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }

    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.next().max(1e-12), self.next());
        (-2.0 * u.ln()).sqrt() * (2.0 * PI * v).cos()
    }
}

/// Minimum-jerk progress from 0 to 1, the profile of a human point-to-point movement.
fn min_jerk(s: f64) -> f64 {
    let s = s.clamp(0.0, 1.0);
    s * s * s * (10.0 - 15.0 * s + 6.0 * s * s)
}

/// Distance along a stroke at `t` ms: speed `v` (px/ms) after easing in over `ramp_in` ms, easing
/// out over the last `ramp_out` ms of `duration`.
fn eased_distance(t: f64, v: f64, ramp_in: f64, ramp_out: f64, duration: f64) -> f64 {
    // Speed eases with half a cosine; its integral is the distance.
    let ease = |x: f64, ramp: f64| {
        let x = x.clamp(0.0, ramp);
        v * (x / 2.0 - ramp / (2.0 * PI) * (PI * x / ramp).sin())
    };
    let cruise_end = duration - ramp_out;
    let full_in = ease(ramp_in, ramp_in);
    if t <= ramp_in {
        ease(t, ramp_in)
    } else if t <= cruise_end {
        full_in + v * (t - ramp_in)
    } else {
        let into = (t - cruise_end).min(ramp_out);
        full_in + v * (cruise_end - ramp_in) + v * into - ease(into, ramp_out)
    }
}

struct Shape {
    duration: f64,
    at: Box<dyn Fn(f64) -> (f64, f64)>,
}

fn shape(kind: &str, rng: &mut Rng) -> Shape {
    let (cx, cy) = (rng.range(250.0, 450.0), rng.range(200.0, 350.0));
    let rot = rng.range(0.0, 2.0 * PI);
    let rotate = move |(x, y): (f64, f64)| {
        (
            cx + x * rot.cos() - y * rot.sin(),
            cy + x * rot.sin() + y * rot.cos(),
        )
    };
    match kind {
        "line" => {
            let v = rng.range(0.5, 0.9);
            let duration = rng.range(450.0, 650.0);
            Shape {
                duration,
                at: Box::new(move |t| {
                    rotate((eased_distance(t, v, 60.0, 80.0, duration) - 250.0, 0.0))
                }),
            }
        }
        "circle" => {
            let r = rng.range(80.0, 150.0);
            let v = rng.range(0.4, 0.8);
            let duration = 1.25 * 2.0 * PI * r / v + 70.0;
            Shape {
                duration,
                at: Box::new(move |t| {
                    let a = eased_distance(t, v, 60.0, 80.0, duration) / r;
                    rotate((r * a.cos(), r * a.sin()))
                }),
            }
        }
        "zigzag" => {
            let mut corners = vec![(0.0, 0.0)];
            let mut times = vec![0.0];
            let n = 6;
            for i in 0..n {
                let len = rng.range(80.0, 140.0);
                let angle: f64 = if i % 2 == 0 { PI / 3.0 } else { -PI / 3.0 };
                let (x, y) = corners[corners.len() - 1];
                corners.push((x + len * angle.cos(), y + len * angle.sin()));
                times.push(times[times.len() - 1] + rng.range(120.0, 180.0));
            }
            let duration = times[times.len() - 1];
            Shape {
                duration,
                at: Box::new(move |t| {
                    let i = times.partition_point(|&s| s <= t).clamp(1, times.len() - 1);
                    let s = min_jerk((t - times[i - 1]) / (times[i] - times[i - 1]));
                    let (a, b) = (corners[i - 1], corners[i]);
                    rotate((a.0 - 300.0 + (b.0 - a.0) * s, a.1 + (b.1 - a.1) * s))
                }),
            }
        }
        "handwriting" => {
            let f = rng.range(3.0, 4.0) / 1000.0;
            let (a, b, c) = (
                rng.range(7.0, 10.0),
                rng.range(16.0, 22.0),
                rng.range(18.0, 26.0),
            );
            let duration = rng.range(1000.0, 1500.0);
            Shape {
                duration,
                at: Box::new(move |t| {
                    // A prolate cycloid: cursive loops.
                    let th = 2.0 * PI * f * eased_distance(t, 1.0, 60.0, 100.0, duration);
                    (cx - 200.0 + a * th - b * th.sin(), cy + c * th.cos())
                }),
            }
        }
        "stop" => {
            let v = rng.range(0.7, 1.1);
            let moving = rng.range(250.0, 400.0);
            let brake = rng.range(15.0, 25.0);
            let hold = 120.0;
            Shape {
                duration: moving + hold,
                at: Box::new(move |t| {
                    rotate((
                        eased_distance(t.min(moving), v, 50.0, brake, moving) - 200.0,
                        0.0,
                    ))
                }),
            }
        }
        _ => unreachable!(),
    }
}

fn synthetic(dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    for kind in ["line", "circle", "zigzag", "handwriting", "stop"] {
        for rate in [133.0, 240.0] {
            let period = 1000.0 / rate;
            let mut strokes = Vec::new();
            let mut t0 = 1000.0;
            for _ in 0..8 {
                let s = shape(kind, &mut rng);
                let noisy = |rng: &mut Rng, t: f64| {
                    let (x, y) = (s.at)(t - t0);
                    json!({
                        "x": x + 0.2 * rng.gauss(),
                        "y": y + 0.2 * rng.gauss(),
                        "pressure": 0.5,
                        "tiltX": 0,
                        "tiltY": 0,
                        "t": t,
                    })
                };
                let down = noisy(&mut rng, t0);
                let frame = 1000.0 / 60.0;
                let mut next_frame = t0 + rng.range(0.0, frame);
                let mut moves = Vec::new();
                let mut pending = Vec::new();
                let mut k = 1.0;
                loop {
                    let t = t0 + k * period + rng.range(-0.25, 0.25);
                    if t > t0 + s.duration {
                        break;
                    }
                    while t > next_frame {
                        if !pending.is_empty() {
                            moves.push(json!({ "t": next_frame, "readings": pending }));
                            pending = Vec::new();
                        }
                        next_frame += frame;
                    }
                    pending.push(noisy(&mut rng, t));
                    k += 1.0;
                }
                if !pending.is_empty() {
                    moves.push(json!({ "t": next_frame, "readings": pending }));
                }
                let up = noisy(&mut rng, t0 + s.duration);
                strokes
                    .push(json!({ "pointerType": "pen", "down": down, "moves": moves, "up": up }));
                t0 += s.duration + 500.0;
            }
            let file = dir.join(format!("{kind}-{rate}hz.json"));
            let doc = json!({ "format": "skathi-pointer-trace", "version": 1, "strokes": strokes });
            std::fs::write(&file, doc.to_string())
                .map_err(|e| format!("{}: {e}", file.display()))?;
            files.push(file);
        }
    }
    Ok(files)
}

fn main() -> Result<(), String> {
    let mut latency = 1000.0 / 60.0;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--latency" => {
                latency = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--latency takes a number of ms")?;
            }
            "--synthetic" => {
                let dir = args.next().ok_or("--synthetic takes a directory")?;
                files.extend(synthetic(Path::new(&dir))?);
            }
            _ => files.push(PathBuf::from(arg)),
        }
    }
    if files.is_empty() {
        return Err(
            "pass trace files (recorded by apps/pen-draw), or --synthetic DIR to generate some"
                .into(),
        );
    }
    let mut total: Vec<(&str, Metrics)> = predictors()
        .into_iter()
        .map(|(name, _)| (name, Metrics::default()))
        .collect();
    for file in &files {
        let strokes = load(file)?;
        let mut results: Vec<(&str, Metrics)> = Vec::new();
        for (i, (name, mut predictor)) in predictors().into_iter().enumerate() {
            let mut m = Metrics::default();
            for stroke in &strokes {
                let frames = replay(predictor.as_mut(), stroke);
                evaluate(&mut m, stroke, &frames, latency);
                evaluate(&mut total[i].1, stroke, &frames, latency);
            }
            results.push((name, m));
        }
        let title = format!(
            "{} ({} strokes)",
            file.file_name().map_or_else(
                || file.display().to_string(),
                |n| n.to_string_lossy().into()
            ),
            strokes.len()
        );
        print!("{}", report(&title, &results, latency));
    }
    if files.len() > 1 {
        print!("{}", report("all traces", &total, latency));
    }
    Ok(())
}
