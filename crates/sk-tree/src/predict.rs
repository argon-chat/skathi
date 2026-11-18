//! `PointerEvent.getPredictedEvents()`: Chromium's default pointer predictor, ported from
//! `ui/base/prediction/kalman_filter.cc`, `ui/base/prediction/kalman_predictor.cc` and the
//! predicted-event loop of `third_party/blink/renderer/platform/widget/input/input_event_prediction.cc`.
//! Times are milliseconds, positions CSS px.
//!
//! Copyright 2018 The Chromium Authors
//!
//! Redistribution and use in source and binary forms, with or without modification, are permitted
//! provided that the following conditions are met:
//!
//!    * Redistributions of source code must retain the above copyright notice, this list of
//! conditions and the following disclaimer.
//!    * Redistributions in binary form must reproduce the above copyright notice, this list of
//! conditions and the following disclaimer in the documentation and/or other materials provided
//! with the distribution.
//!    * Neither the name of Google LLC nor the names of its contributors may be used to endorse or
//! promote products derived from this software without specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
//! IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
//! FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
//! DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
//! DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//! WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY
//! WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

type Vec3 = [f64; 3];
type Mat3 = [[f64; 3]; 3];

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn mul_vec(a: &Mat3, v: &Vec3) -> Vec3 {
    [0, 1, 2].map(|i| (0..3).map(|k| a[i][k] * v[k]).sum::<f64>())
}

fn transpose(a: &Mat3) -> Mat3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| a[j][i]))
}

fn outer(a: &Vec3, b: &Vec3) -> Mat3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| a[i] * b[j]))
}

fn add(a: &Mat3, b: &Mat3) -> Mat3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| a[i][j] + b[i][j]))
}

const STABLE_ITER_NUM: u32 = 4;
const SIGMA_PROCESS: f64 = 0.01;
const SIGMA_MEASUREMENT: f64 = 1.0;

/// One axis: position, velocity and acceleration under a constant-acceleration model.
#[derive(Clone, Debug)]
struct KalmanFilter {
    state: Vec3,
    error_covariance: Mat3,
    iterations: u32,
}

impl Default for KalmanFilter {
    fn default() -> Self {
        Self {
            state: [0.0; 3],
            error_covariance: IDENTITY,
            iterations: 0,
        }
    }
}

impl KalmanFilter {
    fn stable(&self) -> bool {
        self.iterations >= STABLE_ITER_NUM
    }

    fn update(&mut self, observation: f64, dt: f64) {
        self.iterations += 1;
        if self.iterations == 1 {
            self.state = [observation, 0.0, 0.0];
            return;
        }
        self.predict(dt);
        // The measurement vector H is (1, 0, 0).
        let y = observation - self.state[0];
        let p = &self.error_covariance;
        let s = p[0][0] + SIGMA_MEASUREMENT;
        let gain = [p[0][0] / s, p[1][0] / s, p[2][0] / s];
        for (x, k) in self.state.iter_mut().zip(gain) {
            *x += k * y;
        }
        let i_kh = add(&IDENTITY, &outer(&gain.map(|k| -k), &[1.0, 0.0, 0.0]));
        self.error_covariance = add(
            &mul(&mul(&i_kh, &self.error_covariance), &transpose(&i_kh)),
            &outer(&gain.map(|k| k * SIGMA_MEASUREMENT), &gain),
        );
    }

    fn predict(&mut self, dt: f64) {
        let transition = [[1.0, dt, 0.5 * dt * dt], [0.0, 1.0, dt], [0.0, 0.0, 1.0]];
        self.state = mul_vec(&transition, &self.state);
        let noise = [0.5 * dt * dt, dt, 1.0];
        self.error_covariance = add(
            &mul(
                &mul(&transition, &self.error_covariance),
                &transpose(&transition),
            ),
            &outer(&noise, &noise.map(|n| n * SIGMA_PROCESS)),
        );
    }
}

/// A gap this long starts a new stroke.
const MAX_TIME_DELTA: f64 = 20.0;
const DEFAULT_TIME_INTERVAL: f64 = 8.0;
const MIN_TIME_INTERVAL: f64 = 2.5;
/// How far past the latest reading predictions reach.
pub const MAX_PREDICTION_TIME: f64 = 25.0;
const ACCELERATION_INFLUENCE: f64 = 0.5;
const VELOCITY_INFLUENCE: f64 = 1.0;

/// One pointer's predictor; reset when its stroke ends.
#[derive(Clone, Debug, Default)]
pub struct KalmanPredictor {
    x: KalmanFilter,
    y: KalmanFilter,
    /// Smooths the time between readings.
    interval: KalmanFilter,
    /// The latest reading: x, y, time.
    last: Option<(f64, f64, f64)>,
}

impl KalmanPredictor {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, x: f64, y: f64, time: f64) {
        if let Some((.., last)) = self.last {
            let dt = time - last;
            if dt > MAX_TIME_DELTA {
                self.reset();
            } else {
                self.interval.update(dt, 0.0);
            }
        }
        let dt = self.interval.state[0];
        self.last = Some((x, y, time));
        self.x.update(x, dt);
        self.y.update(y, dt);
    }

    pub fn has_prediction(&self) -> bool {
        self.x.stable() && self.y.stable()
    }

    /// Where the pointer is expected at `time`.
    pub fn predict(&self, time: f64) -> Option<(f64, f64)> {
        if !self.has_prediction() {
            return None;
        }
        let (x, y, last) = self.last?;
        let dt = time - last;
        let at = |f: &KalmanFilter, p: f64| {
            p + f.state[1] * VELOCITY_INFLUENCE * dt + f.state[2] * ACCELERATION_INFLUENCE * dt * dt
        };
        Some((at(&self.x, x), at(&self.y, y)))
    }

    /// The smoothed time between readings.
    pub fn time_interval(&self) -> f64 {
        let t = self.interval.state[0];
        if t != 0.0 {
            t.max(MIN_TIME_INTERVAL)
        } else {
            DEFAULT_TIME_INTERVAL
        }
    }

    /// The predicted readings after the latest one, a reading interval apart up to
    /// [`MAX_PREDICTION_TIME`] past it: x, y, time.
    pub fn predictions(&self) -> Vec<(f64, f64, f64)> {
        let Some((.., last)) = self.last else {
            return Vec::new();
        };
        let step = self.time_interval();
        let mut out = Vec::new();
        let mut time = last + step;
        while time <= last + MAX_PREDICTION_TIME {
            let Some((x, y)) = self.predict(time) else {
                break;
            };
            out.push((x, y, time));
            time += step;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_stroke_is_extrapolated_along_its_line() {
        let mut p = KalmanPredictor::default();
        for i in 0..30 {
            let t = 1000.0 + f64::from(i) * 4.0;
            p.update(10.0 + 0.5 * (t - 1000.0), 40.0 - 0.25 * (t - 1000.0), t);
        }
        let last = 1000.0 + 29.0 * 4.0;
        let predicted = p.predictions();
        assert_eq!(
            predicted
                .iter()
                .map(|&(.., t)| t - last)
                .collect::<Vec<_>>(),
            [4.0, 8.0, 12.0, 16.0, 20.0, 24.0]
        );
        for &(x, y, t) in &predicted {
            let (ex, ey) = (10.0 + 0.5 * (t - 1000.0), 40.0 - 0.25 * (t - 1000.0));
            assert!(
                (x - ex).abs() < 0.5 && (y - ey).abs() < 0.5,
                "{x},{y} at {t}"
            );
        }
    }

    #[test]
    fn a_pause_starts_over_and_needs_readings_again() {
        let mut p = KalmanPredictor::default();
        for i in 0..6 {
            p.update(f64::from(i) * 2.0, 0.0, f64::from(i) * 8.0);
        }
        assert!(!p.predictions().is_empty());
        p.update(100.0, 0.0, 200.0);
        assert!(p.predictions().is_empty());
    }
}
