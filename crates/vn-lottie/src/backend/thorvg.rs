//! The ThorVG backend: the vendored C++ engine behind `vn-thorvg-sys`. Unlike `rasterlottie` it has
//! no support-profile check: it renders what it understands and skips the rest, so `load` fails
//! only when the document is not a Lottie at all.
//!
//! # Safety
//!
//! The engine is process-global and not concurrency-safe: ThorVG 1.1.0's software engine keeps
//! unsynchronised global state (loader registry, renderer refcount, composition pools), and two
//! threads driving their own animations corrupt it. So every `tvg_*` call goes through [`ENGINE`],
//! a plain mutex, uncontended since gpui paints on one thread.
//!
//! [`ThorVg`] is `Send` (the sticker engine loads on one worker and may draw on another) and not
//! `Sync`: every call is made under [`ENGINE`], the object graph is owned by one [`ThorVg`], and
//! the expression engine's JerryScript context is looked up by thread id on every evaluation.
//!
//! Lifetimes: `animation` owns `picture` and `canvas` takes a second reference in `tvg_canvas_add`;
//! both are released in [`Drop`]. `self.buffer` is only replaced after `tvg_swcanvas_set_target`
//! points at the replacement. `tvg_picture_load_data` is called with `copy = true`, so ThorVG owns
//! its JSON and its content-hash loader cache is disabled (otherwise two `<lottie>` nodes showing
//! the same file would share a loader and its playhead).

use std::ffi::c_char;
use std::ptr;

use serde_json::Value;
use vn_thorvg_sys as tvg;

use super::Raster;
use crate::state::{LottieError, LottieMeta};

/// ThorVG's mimetype for Bodymovin JSON. `"lottie"` is not accepted (`_convert()` matches `"lot"`
/// and `"lottie+json"`); an unknown type falls through to trying every loader.
const MIME_LOTTIE: &[u8] = b"lottie+json\0";

/// The one lock every `tvg_*` call is made under; it guards a `Result` so engine start-up happens once.
static ENGINE: std::sync::Mutex<Option<Result<(), i32>>> = std::sync::Mutex::new(None);

/// Takes the engine lock, bringing ThorVG up on first use. `threads = 0` keeps ThorVG's
/// TaskScheduler on the calling thread. The engine is never terminated: `tvg_engine_term` is
/// refcounted and nothing proves the last `LottieState` gone.
fn engine() -> Result<std::sync::MutexGuard<'static, Option<Result<(), i32>>>, LottieError> {
    // A poisoned lock means a previous caller panicked mid-render; ThorVG's state is still consistent, so recover.
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

fn ok(result: tvg::Tvg_Result) -> Option<()> {
    result.ok().ok()
}

pub(crate) struct ThorVg {
    animation: tvg::Tvg_Animation,
    picture: tvg::Tvg_Paint,
    canvas: tvg::Tvg_Canvas,
    /// ThorVG's render target: `target.0 * target.1` ARGB8888 words, kept across frames.
    buffer: Vec<u32>,
    target: (u32, u32),
    straight: bool,
    intrinsic: (u32, u32),
    /// `Picture::size` dirties the scene, so it is only re-applied when the size moves.
    applied_size: (f32, f32),
    in_point: f32,
}

impl ThorVg {
    pub fn load(document: Value, meta: &LottieMeta) -> Result<Self, LottieError> {
        // Held for all of `load`: `tvg_picture_load_data` registers the loader in a process-global registry.
        let _engine = engine()?;

        // ThorVG's loader parses text, so the document goes back to JSON here, carrying `crate::normalize`'s rewrites.
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
                straight: false,
                intrinsic: (meta.width.max(1), meta.height.max(1)),
                applied_size: (f32::NAN, f32::NAN),
                in_point: meta.in_point,
            })
        }
    }

    /// `<lottie>`'s raster: uniform `scale` from the authored size into a `width × height` canvas, premultiplied BGRA.
    pub fn render(&mut self, frame: f32, scale: f32, width: u32, height: u32) -> Option<Raster> {
        // Uniform scale from the authored size, as `rasterlottie` does; the canvas is the ceil, so a
        // fractional scale letterboxes by at most a pixel.
        let size = (self.intrinsic.0 as f32 * scale, self.intrinsic.1 as f32 * scale);
        let words = self.draw(frame, size, (width, height), false)?;
        let mut bgra = Vec::with_capacity(words.len() * 4);
        for word in words {
            bgra.extend_from_slice(&word.to_le_bytes());
        }
        Some(Raster {
            width,
            height,
            bgra,
        })
    }

    /// The sticker engine's raster: the whole animation stretched to `width × height`, straight-alpha BGRA, borrowed until the next call.
    pub fn render_straight(&mut self, frame: f32, width: u32, height: u32) -> Option<&[u8]> {
        let words = self.draw(frame, (width as f32, height as f32), (width, height), true)?;
        #[cfg(target_endian = "big")]
        compile_error!("the ThorVG raster is read as little-endian BGRA bytes");
        // SAFETY: a `[u32]` is readable as 4× as many `u8`s; alignment only
        // goes down.
        Some(unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 4) })
    }

    fn draw(&mut self, frame: f32, size: (f32, f32), target: (u32, u32), straight: bool) -> Option<&[u32]> {
        let (width, height) = target;
        if width == 0 || height == 0 {
            return None;
        }
        let _engine = engine().ok()?;
        unsafe {
            if self.target != target || self.straight != straight {
                let mut buffer = vec![0u32; (width as usize) * (height as usize)];
                let colorspace = if straight {
                    tvg::Tvg_Colorspace::TVG_COLORSPACE_ARGB8888S
                } else {
                    tvg::Tvg_Colorspace::TVG_COLORSPACE_ARGB8888
                };
                ok(tvg::tvg_swcanvas_set_target(
                    self.canvas,
                    buffer.as_mut_ptr(),
                    width, // stride, in words — the raster is tightly packed
                    width,
                    height,
                    colorspace,
                ))?;
                self.buffer = buffer;
                self.target = target;
                self.straight = straight;
            }

            if self.applied_size != size {
                ok(tvg::tvg_picture_set_size(self.picture, size.0, size.1))?;
                self.applied_size = size;
            }

            // `Animation::frame` reports InsufficientCondition when the playhead is already there
            // (diff below 0.0009): a no-op, common when only the raster size changed.
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
        Some(&self.buffer)
    }

    pub fn target_bytes(&self) -> usize {
        self.buffer.capacity() * 4
    }
}

// SAFETY: every `tvg_*` call is made under `ENGINE`, the graph is owned exclusively, and ThorVG
// keys its only per-thread state (the expression context) by thread id.
unsafe impl Send for ThorVg {}

impl Drop for ThorVg {
    fn drop(&mut self) {
        // Teardown unregisters the loader from the global registry `load` touched, so it takes the
        // lock; if the engine never came up there is nothing to destroy.
        let Ok(_engine) = engine() else { return };
        unsafe {
            tvg::tvg_canvas_destroy(self.canvas);
            tvg::tvg_animation_del(self.animation);
        }
    }
}
