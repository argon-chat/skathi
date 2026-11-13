//! Raw FFI declarations for the [ThorVG] C API, with no safe wrapper: the only consumer is
//! `vn-lottie`'s ThorVG backend, which owns the lifetimes.
//!
//! `src/bindings.rs` is checked in and `build.rs` never runs `bindgen`; regenerate it with
//! `tools/regen-bindings` after moving the `thorvg/` submodule (see `build.rs`).
//!
//! [ThorVG]: https://github.com/thorvg/thorvg

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

include!("bindings.rs");

/// The ThorVG release the bindings were generated from, as `(major, minor, micro)`.
pub const THORVG_VERSION: (u32, u32, u32) =
    (TVG_VERSION_MAJOR, TVG_VERSION_MINOR, TVG_VERSION_MICRO);

impl Tvg_Result {
    pub fn ok(self) -> Result<(), Self> {
        if matches!(self, Self::TVG_RESULT_SUCCESS) {
            Ok(())
        } else {
            Err(self)
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::TVG_RESULT_SUCCESS => "SUCCESS",
            Self::TVG_RESULT_INVALID_ARGUMENT => "INVALID_ARGUMENT",
            Self::TVG_RESULT_INSUFFICIENT_CONDITION => "INSUFFICIENT_CONDITION",
            Self::TVG_RESULT_FAILED_ALLOCATION => "FAILED_ALLOCATION",
            Self::TVG_RESULT_MEMORY_CORRUPTION => "MEMORY_CORRUPTION",
            Self::TVG_RESULT_NOT_SUPPORTED => "NOT_SUPPORTED",
            Self::TVG_RESULT_SYSTEM_ERROR => "SYSTEM_ERROR",
            Self::TVG_RESULT_UNKNOWN => "UNKNOWN",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stale `src/bindings.rs` after a version bump fails here rather than at a wrong struct offset.
    #[test]
    fn the_linked_engine_is_the_version_the_bindings_were_generated_from() {
        let (mut major, mut minor, mut micro) = (0, 0, 0);
        let mut version: *const ::core::ffi::c_char = core::ptr::null();
        unsafe { tvg_engine_version(&mut major, &mut minor, &mut micro, &mut version) }
            .ok()
            .expect("tvg_engine_version");
        assert_eq!((major, minor, micro), THORVG_VERSION);
    }

    #[test]
    fn a_software_canvas_initialises_and_destroys() {
        unsafe {
            tvg_engine_init(0).ok().expect("engine init");
            let canvas = tvg_swcanvas_create(Tvg_Engine_Option::TVG_ENGINE_OPTION_DEFAULT);
            assert!(!canvas.is_null());
            let mut buffer = vec![0u32; 16 * 16];
            tvg_swcanvas_set_target(
                canvas,
                buffer.as_mut_ptr(),
                16,
                16,
                16,
                Tvg_Colorspace::TVG_COLORSPACE_ARGB8888,
            )
            .ok()
            .expect("set target");
            tvg_canvas_destroy(canvas).ok().expect("destroy");
            tvg_engine_term().ok().expect("engine term");
        }
    }
}
