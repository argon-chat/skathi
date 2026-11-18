//! `PointerEvent.getPredictedEvents()`: per-pointer predictors fed a stroke's readings. Pointer
//! events use [`ink`]; [`chrome`] is kept to compare against (`examples/predict-eval.rs`). Times
//! are milliseconds, positions CSS px.

pub mod chrome;
pub mod ink;

pub trait Predictor {
    fn reset(&mut self);
    fn update(&mut self, x: f64, y: f64, time: f64);
    /// Where the pointer is expected after the latest reading: x, y, time.
    fn predictions(&self) -> Vec<(f64, f64, f64)>;
}

type Mat<const N: usize> = [[f64; N]; N];

const fn identity<const N: usize>() -> Mat<N> {
    let mut out = [[0.0; N]; N];
    let mut i = 0;
    while i < N {
        out[i][i] = 1.0;
        i += 1;
    }
    out
}

fn mul<const N: usize>(a: &Mat<N>, b: &Mat<N>) -> Mat<N> {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..N).map(|k| a[i][k] * b[k][j]).sum()))
}

fn mul_vec<const N: usize>(a: &Mat<N>, v: &[f64; N]) -> [f64; N] {
    std::array::from_fn(|i| (0..N).map(|k| a[i][k] * v[k]).sum())
}

fn transpose<const N: usize>(a: &Mat<N>) -> Mat<N> {
    std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]))
}

fn outer<const N: usize>(a: &[f64; N], b: &[f64; N]) -> Mat<N> {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i] * b[j]))
}

fn add<const N: usize>(a: &Mat<N>, b: &Mat<N>) -> Mat<N> {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] + b[i][j]))
}
