//! libvpx 1.17's VP9 decoder, vendored and compiled with `cc` (VENDOR.md).
//!
//! This is the raw C surface: the declarations the safe wrapper is built on.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_long, c_uint, c_void};

/// The C declarations, as `vpx/vpx_decoder.h`, `vpx/vpx_codec.h` and
/// `vpx/vpx_image.h` have them (the `layout` test checks them against the C
/// compiler), and the vn-vpx-sys additions from `csrc/`.
#[allow(non_camel_case_types)]
pub mod ffi {
    use super::*;

    pub type vpx_codec_err_t = c_int;
    pub const VPX_CODEC_OK: vpx_codec_err_t = 0;
    pub const VPX_CODEC_ERROR: vpx_codec_err_t = 1;
    pub const VPX_CODEC_MEM_ERROR: vpx_codec_err_t = 2;
    pub const VPX_CODEC_ABI_MISMATCH: vpx_codec_err_t = 3;
    pub const VPX_CODEC_INCAPABLE: vpx_codec_err_t = 4;
    pub const VPX_CODEC_UNSUP_BITSTREAM: vpx_codec_err_t = 5;
    pub const VPX_CODEC_UNSUP_FEATURE: vpx_codec_err_t = 6;
    pub const VPX_CODEC_CORRUPT_FRAME: vpx_codec_err_t = 7;
    pub const VPX_CODEC_INVALID_PARAM: vpx_codec_err_t = 8;

    /// `VPX_DECODER_ABI_VERSION`: 3 + `VPX_CODEC_ABI_VERSION` (4 +
    /// `VPX_IMAGE_ABI_VERSION` (5)).
    pub const VPX_DECODER_ABI_VERSION: c_int = 12;

    pub const VPX_IMG_FMT_PLANAR: c_int = 0x100;
    pub const VPX_IMG_FMT_HIGHBITDEPTH: c_int = 0x800;
    pub const VPX_IMG_FMT_I420: c_int = VPX_IMG_FMT_PLANAR | 2;
    pub const VPX_IMG_FMT_I422: c_int = VPX_IMG_FMT_PLANAR | 5;
    pub const VPX_IMG_FMT_I444: c_int = VPX_IMG_FMT_PLANAR | 6;
    pub const VPX_IMG_FMT_I440: c_int = VPX_IMG_FMT_PLANAR | 7;

    /// Opaque: `vpx_codec_iface_t`.
    #[repr(C)]
    pub struct vpx_codec_iface_t {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct vpx_codec_ctx_t {
        pub name: *const c_char,
        pub iface: *const vpx_codec_iface_t,
        pub err: vpx_codec_err_t,
        pub err_detail: *const c_char,
        pub init_flags: c_long,
        pub config: *const c_void,
        pub priv_: *mut c_void,
    }

    #[repr(C)]
    pub struct vpx_codec_dec_cfg_t {
        pub threads: c_uint,
        pub w: c_uint,
        pub h: c_uint,
    }

    #[repr(C)]
    pub struct vpx_codec_stream_info_t {
        pub sz: c_uint,
        pub w: c_uint,
        pub h: c_uint,
        pub is_kf: c_uint,
    }

    #[repr(C)]
    pub struct vpx_image_t {
        pub fmt: c_int,
        pub cs: c_int,
        pub range: c_int,
        pub w: c_uint,
        pub h: c_uint,
        pub bit_depth: c_uint,
        pub d_w: c_uint,
        pub d_h: c_uint,
        pub r_w: c_uint,
        pub r_h: c_uint,
        pub x_chroma_shift: c_uint,
        pub y_chroma_shift: c_uint,
        pub planes: [*mut u8; 4],
        pub stride: [c_int; 4],
        pub bps: c_int,
        pub user_priv: *mut c_void,
        pub img_data: *mut u8,
        pub img_data_owner: c_int,
        pub self_allocd: c_int,
        pub fb_priv: *mut c_void,
    }

    pub type vpx_codec_iter_t = *const c_void;

    /// `vn_vpx_account` in csrc/vn_vpx_mem.c.
    #[repr(C)]
    #[derive(Default)]
    pub struct vn_vpx_account {
        pub live: usize,
        pub peak: usize,
        pub limit: usize,
    }

    extern "C" {
        pub fn vpx_codec_vp9_dx() -> *const vpx_codec_iface_t;
        pub fn vpx_codec_dec_init_ver(
            ctx: *mut vpx_codec_ctx_t,
            iface: *const vpx_codec_iface_t,
            cfg: *const vpx_codec_dec_cfg_t,
            flags: c_long,
            ver: c_int,
        ) -> vpx_codec_err_t;
        pub fn vpx_codec_decode(
            ctx: *mut vpx_codec_ctx_t,
            data: *const u8,
            data_sz: c_uint,
            user_priv: *mut c_void,
            deadline: c_long,
        ) -> vpx_codec_err_t;
        pub fn vpx_codec_get_frame(ctx: *mut vpx_codec_ctx_t, iter: *mut vpx_codec_iter_t) -> *mut vpx_image_t;
        pub fn vpx_codec_destroy(ctx: *mut vpx_codec_ctx_t) -> vpx_codec_err_t;
        pub fn vpx_codec_err_to_string(err: vpx_codec_err_t) -> *const c_char;
        pub fn vpx_codec_error_detail(ctx: *const vpx_codec_ctx_t) -> *const c_char;
        pub fn vpx_codec_peek_stream_info(
            iface: *const vpx_codec_iface_t,
            data: *const u8,
            data_sz: c_uint,
            si: *mut vpx_codec_stream_info_t,
        ) -> vpx_codec_err_t;
        pub fn vpx_codec_version_str() -> *const c_char;
        pub fn vp9_rtcd();
        pub fn vpx_dsp_rtcd();
        pub fn vpx_scale_rtcd();
        pub fn vp9_init_intra_predictors();

        pub fn vn_vpx_account_enter(account: *mut vn_vpx_account);
        pub fn vn_vpx_account_leave();
        pub fn vn_vpx_layout(which: c_int) -> usize;
        #[cfg(feature = "shim-stats")]
        pub fn vn_vpx_shim_stats(out: *mut i64);
    }
}

/// libvpx's version string, `"v1.17.0"`.
pub fn version() -> &'static str {
    // SAFETY: a static NUL-terminated string from libvpx.
    unsafe { CStr::from_ptr(ffi::vpx_codec_version_str()) }.to_str().unwrap_or("")
}
