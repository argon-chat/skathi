//! The ThorVG backend — the vendored C++ engine behind `vn-thorvg-sys`.
//!
//! ThorVG is what LottieFiles' own player ships, and it is the only engine in
//! reach that implements Lottie *expressions*. Unlike `rasterlottie` it has no
//! support-profile check: it renders what it understands and quietly skips
//! what it does not, so `load` here fails only when the document is not a
//! Lottie at all. That difference is a capability difference, not an API one —
//! `LottieState` looks identical from the outside either way.
//!
//! # Safety
//!
//! ## The engine is process-global and not concurrency-safe
//!
//! Every `tvg_*` call goes through [`ENGINE`], a plain global mutex. That is
//! not defensive programming, it is a measured requirement: ThorVG 1.1.0's
//! software engine keeps unsynchronised process-global state (the loader
//! registry, the renderer refcount, the composition pools), and two threads
//! each driving their *own* animation and their *own* canvas is enough to
//! corrupt it. Measured on this crate's own test suite:
//!
//! | | crashes |
//! | --- | --- |
//! | `--test-threads=8`, no lock | **12 / 40** runs, `STATUS_ACCESS_VIOLATION` |
//! | `--test-threads=1`, no lock | 0 / 40 |
//! | `--test-threads=8`, this lock | 0 / 40 |
//!
//! The lock is uncontended in the only configuration that matters — gpui
//! paints on one thread — and an uncontended `Mutex` is tens of nanoseconds
//! against a rasterisation measured in tens of *micro*seconds, so it does not
//! show up in the numbers in `research/reports/m11/thorvg.md` §5. Serialising
//! is also the honest wrapper: a safe Rust API must not hand out a way to
//! corrupt memory by doing something as ordinary as loading two animations
//! from a thread pool.
//!
//! ## Object lifetimes
//!
//! Everything else is one object graph owned by one [`ThorVg`], never shared
//! (the raw pointers make the struct `!Send` automatically, which is a useful
//! second line of defence — a `LottieState` cannot travel between threads even
//! though the lock would now make that survivable):
//!
//! * `animation` owns `picture` (`Animation::Impl` calls `picture->ref()`),
//!   and `canvas` takes a second reference in `tvg_canvas_add`. Both are
//!   released in [`Drop`], so the picture outlives every use of it.
//! * The pixel target is `self.buffer`, whose heap allocation is only ever
//!   replaced *after* `tvg_swcanvas_set_target` has been pointed at the
//!   replacement, so ThorVG never holds a dangling target.
//! * `tvg_picture_load_data` is called with `copy = true`, so ThorVG owns its
//!   own copy of the JSON and the `String` here can die at the end of `load`.
//!   It also disables ThorVG's content-hash loader cache, which matters:
//!   without `copy` two `<lottie>` nodes showing the same file would *share*
//!   one loader and fight over its playhead.

use std::ffi::c_char;
use std::ptr;

use serde_json::Value;
use vn_thorvg_sys as tvg;

use super::Raster;
use crate::state::{LottieError, LottieMeta};

/// ThorVG's mimetype for Bodymovin JSON. `"lottie"` is *not* accepted —
/// `_convert()` in `tvgLoaderMgr.cpp` matches `"lot"` and `"lottie+json"`
/// only, and an unrecognised type silently falls through to trying every
/// loader in turn.
const MIME_LOTTIE: &[u8] = b"lottie+json\0";

/// The one lock every `tvg_*` call in this module is made under. See the
/// module docs for the measurement that makes it mandatory.
///
/// It guards a `Result` rather than `()` so engine start-up happens exactly
/// once, inside the same critical section: `tvg_engine_init` mutates the same
/// globals everything else touches.
static ENGINE: std::sync::Mutex<Option<Result<(), i32>>> = std::sync::Mutex::new(None);

/// Takes the engine lock, bringing ThorVG up on first use.
///
/// `threads = 0` keeps ThorVG's TaskScheduler on the calling thread. That is
/// deliberate: frames are rasterised inline from gpui's paint pass, a sticker
/// at 256px is a fraction of a millisecond, and a background pool would buy
/// nothing while making the timing non-deterministic.
///
/// The engine is never terminated. `tvg_engine_term` is refcounted against
/// `init`, and there is no point in the process where the last `LottieState`
/// is provably gone; leaking one engine for the process lifetime is the
/// cheaper correctness argument.
fn engine() -> Result<std::sync::MutexGuard<'static, Option<Result<(), i32>>>, LottieError> {
    // A poisoned lock means a previous caller panicked mid-render. ThorVG's
    // own state is still consistent (it is C++ with no unwinding through it),
    // so recovering is right — refusing every subsequent frame would not be.
    let mut guard = ENGINE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let started = *guard.get_or_insert_with(|| unsafe {
        tvg::tvg_engine_init(0).ok().map_err(|error| error as i32)
    });
    match started {
        Ok(()) => Ok(guard),
        Err(code) => Err(LottieError::Parse(format!(
            "ThorVG engine initialisation failed (Tvg_Result {code})"
        ))),
    }
}

/// `Some(())` on success — the `?`-friendly spelling of a `Tvg_Result`, for
/// the render path where a failed call means "no frame this time".
fn ok(result: tvg::Tvg_Result) -> Option<()> {
    result.ok().ok()
}

pub(crate) struct ThorVg {
    animation: tvg::Tvg_Animation,
    /// Owned by `animation`; borrowed here so every call does not have to
    /// re-fetch it.
    picture: tvg::Tvg_Paint,
    canvas: tvg::Tvg_Canvas,
    /// ThorVG's render target: `target.0 * target.1` premultiplied ARGB8888
    /// words. Kept across frames so a steady-state animation allocates once.
    buffer: Vec<u32>,
    target: (u32, u32),
    /// The animation's authored canvas size, for the `Picture::size` scale.
    intrinsic: (u32, u32),
    /// `Picture::size` is a paint mutation, so it is only re-applied when the
    /// scale actually moves — otherwise every frame would dirty the whole
    /// scene twice over.
    applied_scale: f32,
    /// ThorVG's frame domain is `0 ..= op - ip`; ours is absolute source
    /// frames. This is the offset between them.
    in_point: f32,
}

impl ThorVg {
    pub fn load(document: Value, meta: &LottieMeta) -> Result<Self, LottieError> {
        // Held for the whole of `load`: `tvg_picture_load_data` registers the
        // loader in ThorVG's process-global `LoaderMgr`.
        let _engine = engine()?;

        // ThorVG's loader parses text, so the document goes back to JSON here.
        // That is also the point at which `crate::normalize`'s rewrites become
        // visible to it — the pass runs for both backends so that a file
        // renders the same way whichever one is drawing it.
        let json = serde_json::to_string(&document).map_err(LottieError::Json)?;

        unsafe {
            let animation = tvg::tvg_animation_new();
            if animation.is_null() {
                return Err(LottieError::Parse("tvg_animation_new returned null".into()));
            }
            let picture = tvg::tvg_animation_get_picture(animation);
            if picture.is_null() {
                tvg::tvg_animation_del(animation);
                return Err(LottieError::Parse(
                    "tvg_animation_get_picture returned null".into(),
                ));
            }

            let loaded = tvg::tvg_picture_load_data(
                picture,
                json.as_ptr().cast::<c_char>(),
                json.len() as u32,
                MIME_LOTTIE.as_ptr().cast::<c_char>(),
                ptr::null(),
                true,
            );
            if let Err(error) = loaded.ok() {
                tvg::tvg_animation_del(animation);
                return Err(LottieError::Parse(format!(
                    "ThorVG rejected the document: tvg_picture_load_data -> {}",
                    error.name()
                )));
            }

            let canvas = tvg::tvg_swcanvas_create(tvg::Tvg_Engine_Option::TVG_ENGINE_OPTION_DEFAULT);
            if canvas.is_null() {
                tvg::tvg_animation_del(animation);
                return Err(LottieError::Parse("tvg_swcanvas_create returned null".into()));
            }
            if let Err(error) = tvg::tvg_canvas_add(canvas, picture).ok() {
                tvg::tvg_canvas_destroy(canvas);
                tvg::tvg_animation_del(animation);
                return Err(LottieError::Parse(format!(
                    "tvg_canvas_add -> {}",
                    error.name()
                )));
            }

            Ok(Self {
                animation,
                picture,
                canvas,
                buffer: Vec::new(),
                target: (0, 0),
                intrinsic: (meta.width.max(1), meta.height.max(1)),
                applied_scale: f32::NAN,
                in_point: meta.in_point,
            })
        }
    }

    pub fn render(&mut self, frame: f32, scale: f32, width: u32, height: u32) -> Option<Raster> {
        if width == 0 || height == 0 {
            return None;
        }
        // Held across the whole update/draw/sync sequence, which is the unit
        // ThorVG's internals assume nothing else is running alongside.
        let _engine = engine().ok()?;
        unsafe {
            if self.target != (width, height) {
                // Allocate first, retarget second, drop the old buffer third:
                // the canvas must never point at freed memory, not even
                // transiently.
                let mut buffer = vec![0u32; (width as usize) * (height as usize)];
                ok(tvg::tvg_swcanvas_set_target(
                    self.canvas,
                    buffer.as_mut_ptr(),
                    width, // stride, in words — the raster is tightly packed
                    width,
                    height,
                    tvg::Tvg_Colorspace::TVG_COLORSPACE_ARGB8888,
                ))?;
                self.buffer = buffer;
                self.target = (width, height);
            }

            if self.applied_scale != scale {
                // Uniform scale from the *authored* size, matching what
                // `rasterlottie` does with `RenderConfig::scale`. The canvas
                // is the ceil of that, so a fractional scale letterboxes by
                // at most one pixel instead of stretching.
                ok(tvg::tvg_picture_set_size(
                    self.picture,
                    self.intrinsic.0 as f32 * scale,
                    self.intrinsic.1 as f32 * scale,
                ))?;
                self.applied_scale = scale;
            }

            // `Animation::frame` reports InsufficientCondition when the
            // playhead is already there (`LottieLoader::frame` short-circuits
            // on a diff below 0.0009). That is a no-op, not a failure — and
            // it is the common case when only the raster size changed.
            let result = tvg::tvg_animation_set_frame(self.animation, frame - self.in_point);
            if !matches!(
                result,
                tvg::Tvg_Result::TVG_RESULT_SUCCESS
                    | tvg::Tvg_Result::TVG_RESULT_INSUFFICIENT_CONDITION
            ) {
                return None;
            }

            ok(tvg::tvg_canvas_update(self.canvas))?;
            ok(tvg::tvg_canvas_draw(self.canvas, true))?;
            ok(tvg::tvg_canvas_sync(self.canvas))?;
        }

        // ARGB8888 is `0xAARRGGBB`, so its little-endian bytes are exactly the
        // B, G, R, A gpui's atlas wants — premultiplied on both sides. No
        // channel swap, unlike the rasterlottie path.
        let mut bgra = Vec::with_capacity(self.buffer.len() * 4);
        for word in &self.buffer {
            bgra.extend_from_slice(&word.to_le_bytes());
        }
        Some(Raster {
            width,
            height,
            bgra,
        })
    }
}

impl Drop for ThorVg {
    fn drop(&mut self) {
        // Teardown unregisters the loader from the same global registry that
        // `load` touched, so it takes the same lock. If the engine never came
        // up there is nothing to destroy.
        let Ok(_engine) = engine() else { return };
        // Canvas first: it releases its reference to the picture, then the
        // animation releases the last one and the picture is freed. Doing it
        // the other way round works too (both are refcounts) but this order
        // matches ThorVG's own examples.
        unsafe {
            tvg::tvg_canvas_destroy(self.canvas);
            tvg::tvg_animation_del(self.animation);
        }
    }
}
