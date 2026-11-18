//! The pointer predictor: the Kalman predictor of Google's ink-stroke-modeler
//! (<https://github.com/google/ink-stroke-modeler>, commit f2388813b0), ported from
//! `ink_stroke_modeler/internal/prediction/kalman_predictor.{h,cc}`,
//! `ink_stroke_modeler/internal/prediction/kalman_filter/{kalman_filter,axis_predictor}.{h,cc}`,
//! `KalmanPredictorParams` in `ink_stroke_modeler/params.h` and `Normalize01`/`Interp` in
//! `ink_stroke_modeler/internal/utils.h`.
//!
//! Copyright 2022 Google LLC
//!
//! Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file
//! except in compliance with the License. You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software distributed under the
//! License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND,
//! either express or implied. See the License for the specific language governing permissions and
//! limitations under the License.
//!
//! Modified for Skathi: ported to Rust; fed a pointer's raw readings instead of the modeler's
//! wobble-smoothed inputs; the prediction continues from the latest reading at its time, where
//! upstream first joins the modeler's spring-smoothed tip to the estimate with a cubic connector;
//! readings are converted to cm and s so upstream's test parameters apply; the mean time between
//! readings is taken over the intervals rather than the readings; a gap of more than 20 ms
//! between readings starts over, and readings that share one time give no prediction.

use std::collections::VecDeque;

use super::{add, identity, mul, mul_vec, outer, transpose, Mat, Predictor};

type Vec2 = [f64; 2];
type Vec4 = [f64; 4];
type Mat4 = Mat<4>;

const CM_PER_PX: f64 = 2.54 / 96.0;
/// A gap this long (ms) starts a new stroke.
const MAX_TIME_DELTA: f64 = 20.0;

/// `KalmanPredictorParams` and `SamplingParams::min_output_rate`, in cm and s.
#[derive(Clone, Debug)]
pub struct Params {
    pub process_noise: f64,
    pub measurement_noise: f64,
    pub min_stable_iteration: u32,
    pub max_time_samples: usize,
    pub acceleration_weight: f64,
    pub jerk_weight: f64,
    /// The longest prediction.
    pub prediction_interval: f64,
    /// Predictions per second.
    pub min_output_rate: f64,
    pub desired_number_of_samples: u32,
    pub max_estimation_distance: f64,
    pub min_travel_speed: f64,
    pub max_travel_speed: f64,
    pub max_linear_deviation: f64,
    pub baseline_linearity_confidence: f64,
}

/// Upstream's tests' parameters (`kalman_predictor_test.cc`), whose measurement noise is one CSS
/// px in cm; the rest are `params.h` defaults.
pub const DEFAULT: Params = Params {
    process_noise: 0.00026458,
    measurement_noise: 0.026458,
    min_stable_iteration: 4,
    max_time_samples: 20,
    acceleration_weight: 0.5,
    jerk_weight: 0.1,
    prediction_interval: 1.0 / 60.0,
    min_output_rate: 180.0,
    desired_number_of_samples: 20,
    max_estimation_distance: 0.04,
    min_travel_speed: 3.0,
    max_travel_speed: 15.0,
    max_linear_deviation: 0.2,
    baseline_linearity_confidence: 0.4,
};

/// One axis under a constant-jerk model, one time unit per reading.
#[derive(Clone, Debug)]
struct AxisPredictor {
    state: Vec4,
    error_covariance: Mat4,
    iterations: u32,
}

impl Default for AxisPredictor {
    fn default() -> Self {
        Self {
            state: [0.0; 4],
            error_covariance: identity(),
            iterations: 0,
        }
    }
}

const TRANSITION: Mat4 = [
    [1.0, 1.0, 0.5, 1.0 / 6.0],
    [0.0, 1.0, 1.0, 0.5],
    [0.0, 0.0, 1.0, 1.0],
    [0.0, 0.0, 0.0, 1.0],
];
const PROCESS_NOISE_VECTOR: Vec4 = [1.0 / 6.0, 0.5, 1.0, 1.0];

impl AxisPredictor {
    fn update(&mut self, observation: f64, params: &Params) {
        self.iterations += 1;
        if self.iterations == 1 {
            self.state[0] = observation;
            return;
        }
        self.state = mul_vec(&TRANSITION, &self.state);
        self.error_covariance = add(
            &mul(
                &mul(&TRANSITION, &self.error_covariance),
                &transpose(&TRANSITION),
            ),
            &outer(
                &PROCESS_NOISE_VECTOR,
                &PROCESS_NOISE_VECTOR.map(|n| n * params.process_noise),
            ),
        );
        // The measurement vector H is (1, 0, 0, 0).
        let y = observation - self.state[0];
        let p = &self.error_covariance;
        let s = p[0][0] + params.measurement_noise;
        let gain: Vec4 = std::array::from_fn(|i| p[0][i] / s);
        for (x, k) in self.state.iter_mut().zip(gain) {
            *x += k * y;
        }
        let i_kh = add(
            &identity(),
            &outer(&gain.map(|k| -k), &[1.0, 0.0, 0.0, 0.0]),
        );
        self.error_covariance = add(
            &mul(&mul(&i_kh, &self.error_covariance), &transpose(&i_kh)),
            &outer(&gain, &gain.map(|k| k * params.measurement_noise)),
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct State {
    position: Vec2,
    velocity: Vec2,
    acceleration: Vec2,
    jerk: Vec2,
}

fn evaluate_cubic(s: &State, dt: f64) -> State {
    let (dt2, dt3) = (dt * dt, dt * dt * dt);
    State {
        position: [0, 1].map(|i| {
            s.position[i]
                + s.velocity[i] * dt
                + s.acceleration[i] * dt2 / 2.0
                + s.jerk[i] * dt3 / 6.0
        }),
        velocity: [0, 1].map(|i| s.velocity[i] + s.acceleration[i] * dt + s.jerk[i] * dt2 / 2.0),
        acceleration: [0, 1].map(|i| s.acceleration[i] + s.jerk[i] * dt),
        jerk: s.jerk,
    }
}

fn distance(a: Vec2, b: Vec2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn normalize01(start: f64, end: f64, value: f64) -> f64 {
    if start == end {
        return if value > start { 1.0 } else { 0.0 };
    }
    ((value - start) / (end - start)).clamp(0.0, 1.0)
}

fn interp(start: f64, end: f64, amount: f64) -> f64 {
    start + (end - start) * amount.clamp(0.0, 1.0)
}

/// One pointer's predictor; reset when its stroke ends.
#[derive(Clone, Debug)]
pub struct KalmanPredictor {
    params: Params,
    x: AxisPredictor,
    y: AxisPredictor,
    /// The latest readings' times, in s.
    sample_times: VecDeque<f64>,
    /// The latest reading: x, y (px), time (ms).
    last: Option<(f64, f64, f64)>,
}

impl Default for KalmanPredictor {
    fn default() -> Self {
        Self::with_params(DEFAULT)
    }
}

impl KalmanPredictor {
    pub fn with_params(params: Params) -> Self {
        Self {
            params,
            x: AxisPredictor::default(),
            y: AxisPredictor::default(),
            sample_times: VecDeque::new(),
            last: None,
        }
    }

    /// The filters' state at the latest reading, in cm and s.
    fn estimated_state(&self) -> Option<State> {
        let stable = self.params.min_stable_iteration;
        if self.x.iterations < stable || self.y.iterations < stable {
            return None;
        }
        let (Some(front), Some(back)) = (self.sample_times.front(), self.sample_times.back())
        else {
            return None;
        };
        // The mean interval, as upstream's README (Algorithm #6) has it; its code divides by the
        // sample count instead, overstating the velocity by up to a third.
        let dt = (back - front) / self.sample_times.len().saturating_sub(1).max(1) as f64;
        if dt <= 0.0 {
            return None;
        }
        let w = &self.params;
        let axis = |i: usize| [self.x.state[i], self.y.state[i]];
        let scaled = |i: usize, by: f64| axis(i).map(|v| v * by);
        Some(State {
            position: axis(0),
            velocity: scaled(1, 1.0 / dt),
            acceleration: scaled(2, w.acceleration_weight / (dt * dt)),
            jerk: scaled(3, w.jerk_weight / (dt * dt * dt)),
        })
    }

    /// How many predictions to make: the full interval's worth scaled by the confidence, the
    /// product of four heuristics in [0, 1].
    fn number_of_points(&self, estimated: &State, last: Vec2) -> usize {
        let p = &self.params;
        let target = p.prediction_interval * p.min_output_rate;
        // More readings average out each one's noise.
        let sample_ratio =
            (f64::from(self.x.iterations) / f64::from(p.desired_number_of_samples)).min(1.0);
        // The further the estimate from the latest reading, the less it is trusted.
        let error = 1.0
            - normalize01(
                0.0,
                p.max_estimation_distance,
                distance(last, estimated.position),
            );
        let end = evaluate_cubic(estimated, p.prediction_interval);
        // A slow prediction wobbles visibly as it changes direction.
        let speed = distance(estimated.position, end.position) / p.prediction_interval;
        let speed = normalize01(p.min_travel_speed, p.max_travel_speed, speed);
        // Acceleration and jerk overtaking the velocity fling the prediction off the stroke.
        let linear =
            [0, 1].map(|i| estimated.position[i] + p.prediction_interval * estimated.velocity[i]);
        let deviation = distance(end.position, linear);
        let linearity = interp(
            p.baseline_linearity_confidence,
            1.0,
            1.0 - normalize01(0.0, p.max_linear_deviation, deviation),
        );
        let confidence = sample_ratio * error * speed * linearity;
        (target * confidence).ceil().max(0.0) as usize
    }
}

impl Predictor for KalmanPredictor {
    fn reset(&mut self) {
        self.x = AxisPredictor::default();
        self.y = AxisPredictor::default();
        self.sample_times.clear();
        self.last = None;
    }

    fn update(&mut self, x: f64, y: f64, time: f64) {
        if self
            .last
            .is_some_and(|(.., last)| time - last > MAX_TIME_DELTA)
        {
            self.reset();
        }
        self.last = Some((x, y, time));
        self.sample_times.push_back(time / 1000.0);
        if self.sample_times.len() > self.params.max_time_samples {
            self.sample_times.pop_front();
        }
        self.x.update(x * CM_PER_PX, &self.params);
        self.y.update(y * CM_PER_PX, &self.params);
    }

    /// Up to `prediction_interval` past the latest reading, one prediction per
    /// `1 / min_output_rate`, as many as the confidence allows.
    fn predictions(&self) -> Vec<(f64, f64, f64)> {
        let Some((x, y, time)) = self.last else {
            return Vec::new();
        };
        let Some(estimated) = self.estimated_state() else {
            return Vec::new();
        };
        let last = [x * CM_PER_PX, y * CM_PER_PX];
        let n = self.number_of_points(&estimated, last);
        let offset = [0, 1].map(|i| last[i] - estimated.position[i]);
        let step = 1.0 / self.params.min_output_rate;
        let mut state = estimated;
        (1..=n)
            .map(|k| {
                state = evaluate_cubic(&state, step);
                (
                    (state.position[0] + offset[0]) / CM_PER_PX,
                    (state.position[1] + offset[1]) / CM_PER_PX,
                    time + k as f64 * step * 1000.0,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::chrome;
    use super::*;

    #[test]
    fn a_steady_stroke_is_extrapolated_along_its_line() {
        let mut p = KalmanPredictor::default();
        let at = |t: f64| (10.0 + 0.5 * (t - 1000.0), 40.0 - 0.25 * (t - 1000.0));
        for i in 0..40 {
            let t = 1000.0 + f64::from(i) * 4.0;
            let (x, y) = at(t);
            p.update(x, y, t);
        }
        let last = 1000.0 + 39.0 * 4.0;
        let predicted = p.predictions();
        assert_eq!(predicted.len(), 3);
        for (k, &(x, y, t)) in predicted.iter().enumerate() {
            assert!((t - last - (k + 1) as f64 * 1000.0 / 180.0).abs() < 1e-9);
            let (ex, ey) = at(t);
            assert!(
                (x - ex).abs() < 0.5 && (y - ey).abs() < 0.5,
                "{x},{y} at {t}"
            );
        }
    }

    /// The furthest predictions reach past the end of a hand-like stroke (300 px in 400 ms with
    /// a minimum-jerk profile, held still for 100 ms after), read at 240 Hz with ±0.25 px of
    /// noise and delivered at 60 Hz.
    fn hook_past_end(p: &mut dyn Predictor) -> f64 {
        let (length, duration) = (300.0, 400.0);
        let x = |t: f64| {
            let s = (t / duration).min(1.0);
            length * s * s * s * (10.0 - 15.0 * s + 6.0 * s * s)
        };
        let mut hook: f64 = 0.0;
        let mut frame = 1000.0 / 60.0;
        for i in 1..120 {
            let t = f64::from(i) * 1000.0 / 240.0;
            let noise = 0.25 * (f64::from(i) * 2.399).sin();
            p.update(x(t) + noise, 50.0 - noise, t);
            if t >= frame {
                frame += 1000.0 / 60.0;
                for &(px, ..) in &p.predictions() {
                    hook = hook.max(px - length);
                }
            }
        }
        hook
    }

    #[test]
    fn a_stroke_is_not_carried_past_where_it_ends() {
        let ink = hook_past_end(&mut KalmanPredictor::default());
        let chrome = hook_past_end(&mut chrome::KalmanPredictor::default());
        assert!(ink < 1.0, "ink {ink}, chrome {chrome}");
        assert!(chrome > 2.0, "ink {ink}, chrome {chrome}");
    }

    #[test]
    fn a_pause_starts_over_and_needs_readings_again() {
        let mut p = KalmanPredictor::default();
        for i in 0..30 {
            p.update(f64::from(i) * 4.0, 0.0, f64::from(i) * 8.0);
        }
        assert!(!p.predictions().is_empty());
        p.update(200.0, 0.0, 300.0);
        assert!(p.predictions().is_empty());
    }
}
