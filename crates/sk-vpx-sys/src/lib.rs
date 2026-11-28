//! libvpx 1.17's VP9 decoder, vendored and compiled with `cc` (VENDOR.md), and a small safe wrapper.
//!
//! A [`Decoder`] takes VP9 packets one at a time (a frame, or a superframe) and hands back the I420
//! frame each one shows. Nothing else of libvpx is exposed: no encoder, no VP8, no threads (every
//! decoder runs on its caller's thread, `threads = 1`).
//!
//! # Memory
//!
//! libvpx allocates through `csrc/sk_vpx_mem.c`, which charges every block to the decoder whose call
//! made it. [`Decoder::memory`] is that account, exact to the byte; [`Decoder::set_memory_limit`]
//! caps it: past the cap the decode returns [`ErrorCode::Mem`] and the decoder stays usable.
//!
//! # Errors
//!
//! A failed decode leaves the decoder usable: the next decode starts from a fresh libvpx context and
//! needs a keyframe ([`Decoder::decode`] says why).
//!
//! # Threads
//!
//! A [`Decoder`] is `Send` and not `Sync`. Decoders are independent, except that the first decode in
//! the process fills process-wide dispatch tables, guarded by a plain flag here
//! (`CONFIG_MULTITHREAD 0`). [`Decoder::new`] runs that set-up first, under a [`std::sync::Once`],
//! so decoders can be made and used on several threads at once.

use std::cell::UnsafeCell;
use std::ffi::CStr;
use std::fmt;
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int, c_long, c_uint, c_void};
use std::ptr;
use std::sync::Once;

/// The C declarations as the `vpx/` headers have them (the `layout` test checks them against the C
/// compiler), and the sk-vpx-sys additions from `csrc/`.
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

    pub const VPX_DECODER_ABI_VERSION: c_int = 12;

    pub const VPX_IMG_FMT_PLANAR: c_int = 0x100;
    pub const VPX_IMG_FMT_HIGHBITDEPTH: c_int = 0x800;
    pub const VPX_IMG_FMT_I420: c_int = VPX_IMG_FMT_PLANAR | 2;
    pub const VPX_IMG_FMT_I422: c_int = VPX_IMG_FMT_PLANAR | 5;
    pub const VPX_IMG_FMT_I444: c_int = VPX_IMG_FMT_PLANAR | 6;
    pub const VPX_IMG_FMT_I440: c_int = VPX_IMG_FMT_PLANAR | 7;

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

    /// `sk_vpx_account` in csrc/sk_vpx_mem.c.
    #[repr(C)]
    #[derive(Default)]
    pub struct sk_vpx_account {
        pub live: usize,
        pub peak: usize,
        pub limit: usize,
    }

    // libvpx's symbols are all `vnvpx_`-prefixed (config/prefix/, VENDOR.md
    // "Symbol prefix"): another libvpx, libwebrtc's, is in the same link.
    unsafe extern "C" {
        #[link_name = "vnvpx_vpx_codec_vp9_dx"]
        pub fn vpx_codec_vp9_dx() -> *const vpx_codec_iface_t;
        #[link_name = "vnvpx_vpx_codec_dec_init_ver"]
        pub fn vpx_codec_dec_init_ver(
            ctx: *mut vpx_codec_ctx_t,
            iface: *const vpx_codec_iface_t,
            cfg: *const vpx_codec_dec_cfg_t,
            flags: c_long,
            ver: c_int,
        ) -> vpx_codec_err_t;
        #[link_name = "vnvpx_vpx_codec_decode"]
        pub fn vpx_codec_decode(
            ctx: *mut vpx_codec_ctx_t,
            data: *const u8,
            data_sz: c_uint,
            user_priv: *mut c_void,
            deadline: c_long,
        ) -> vpx_codec_err_t;
        #[link_name = "vnvpx_vpx_codec_get_frame"]
        pub fn vpx_codec_get_frame(
            ctx: *mut vpx_codec_ctx_t,
            iter: *mut vpx_codec_iter_t,
        ) -> *mut vpx_image_t;
        #[link_name = "vnvpx_vpx_codec_destroy"]
        pub fn vpx_codec_destroy(ctx: *mut vpx_codec_ctx_t) -> vpx_codec_err_t;
        #[link_name = "vnvpx_vpx_codec_err_to_string"]
        pub fn vpx_codec_err_to_string(err: vpx_codec_err_t) -> *const c_char;
        #[link_name = "vnvpx_vpx_codec_error_detail"]
        pub fn vpx_codec_error_detail(ctx: *const vpx_codec_ctx_t) -> *const c_char;
        #[link_name = "vnvpx_vpx_codec_peek_stream_info"]
        pub fn vpx_codec_peek_stream_info(
            iface: *const vpx_codec_iface_t,
            data: *const u8,
            data_sz: c_uint,
            si: *mut vpx_codec_stream_info_t,
        ) -> vpx_codec_err_t;
        #[link_name = "vnvpx_vpx_codec_version_str"]
        pub fn vpx_codec_version_str() -> *const c_char;
        #[link_name = "vnvpx_vp9_rtcd"]
        pub fn vp9_rtcd();
        #[link_name = "vnvpx_vpx_dsp_rtcd"]
        pub fn vpx_dsp_rtcd();
        #[link_name = "vnvpx_vpx_scale_rtcd"]
        pub fn vpx_scale_rtcd();
        #[link_name = "vnvpx_vp9_init_intra_predictors"]
        pub fn vp9_init_intra_predictors();

        pub fn sk_vpx_account_enter(account: *mut sk_vpx_account);
        pub fn sk_vpx_account_leave();
        pub fn sk_vpx_layout(which: c_int) -> usize;
        #[cfg(feature = "shim-stats")]
        pub fn sk_vpx_shim_stats(out: *mut i64);
    }
}

pub fn version() -> &'static str {
    // SAFETY: a static NUL-terminated string from libvpx.
    unsafe { CStr::from_ptr(ffi::vpx_codec_version_str()) }
        .to_str()
        .unwrap_or("")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// `VPX_CODEC_MEM_ERROR`: an allocation failed, or passed [`Decoder::set_memory_limit`].
    Mem,
    /// `VPX_CODEC_UNSUP_BITSTREAM`, `VPX_CODEC_UNSUP_FEATURE`, `VPX_CODEC_INCAPABLE`: valid VP9 this
    /// build does not decode (a high bit depth profile, say).
    Unsupported,
    /// `VPX_CODEC_CORRUPT_FRAME`, `VPX_CODEC_ERROR`, `VPX_CODEC_INVALID_PARAM`: the packet is not a
    /// VP9 frame libvpx can decode, among them a frame larger than 4096×4096.
    Corrupt,
    /// Anything else (`VPX_CODEC_ABI_MISMATCH`: a bug here).
    Other(i32),
}

/// A libvpx failure: the code and libvpx's own words for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl Error {
    fn new(err: ffi::vpx_codec_err_t, detail: *const c_char) -> Error {
        let code = match err {
            ffi::VPX_CODEC_MEM_ERROR => ErrorCode::Mem,
            ffi::VPX_CODEC_UNSUP_BITSTREAM
            | ffi::VPX_CODEC_UNSUP_FEATURE
            | ffi::VPX_CODEC_INCAPABLE => ErrorCode::Unsupported,
            ffi::VPX_CODEC_CORRUPT_FRAME | ffi::VPX_CODEC_ERROR | ffi::VPX_CODEC_INVALID_PARAM => {
                ErrorCode::Corrupt
            }
            other => ErrorCode::Other(other),
        };
        // SAFETY: both are NULL or static/context-owned NUL-terminated strings.
        let what = unsafe { CStr::from_ptr(ffi::vpx_codec_err_to_string(err)) }.to_string_lossy();
        let message = if detail.is_null() {
            what.into_owned()
        } else {
            let detail = unsafe { CStr::from_ptr(detail) }.to_string_lossy();
            format!("{what}: {detail}")
        };
        Error { code, message }
    }

    fn invalid(message: &str) -> Error {
        Error {
            code: ErrorCode::Corrupt,
            message: message.to_string(),
        }
    }
}

/// What a keyframe says about the stream, without decoding it ([`peek`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub width: u32,
    pub height: u32,
    pub keyframe: bool,
}

/// libvpx's `vpx_codec_peek_stream_info` for VP9: the frame size of a keyframe
/// (or intra-only frame) packet, or `None` when the packet is not one.
pub fn peek(packet: &[u8]) -> Option<StreamInfo> {
    let len = c_uint::try_from(packet.len()).ok()?;
    let mut si = ffi::vpx_codec_stream_info_t {
        sz: std::mem::size_of::<ffi::vpx_codec_stream_info_t>() as c_uint,
        w: 0,
        h: 0,
        is_kf: 0,
    };
    // SAFETY: the packet outlives the call; peek allocates nothing.
    let err = unsafe {
        ffi::vpx_codec_peek_stream_info(ffi::vpx_codec_vp9_dx(), packet.as_ptr(), len, &mut si)
    };
    (err == ffi::VPX_CODEC_OK && si.w > 0 && si.h > 0).then_some(StreamInfo {
        width: si.w,
        height: si.h,
        keyframe: si.is_kf != 0,
    })
}

/// Bytes a decoder holds through libvpx's allocator, allocator overhead included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryUse {
    pub live: usize,
    pub peak: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Unknown,
    Bt601,
    Bt709,
    Smpte170,
    Smpte240,
    Bt2020,
    Reserved,
    Srgb,
}

impl ColorSpace {
    fn from_vpx(cs: c_int) -> ColorSpace {
        match cs {
            1 => ColorSpace::Bt601,
            2 => ColorSpace::Bt709,
            3 => ColorSpace::Smpte170,
            4 => ColorSpace::Smpte240,
            5 => ColorSpace::Bt2020,
            6 => ColorSpace::Reserved,
            7 => ColorSpace::Srgb,
            _ => ColorSpace::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    I420,
    I422,
    I440,
    I444,
}

/// One plane of a decoded frame: `height` rows of `width` bytes, `stride` bytes apart.
#[derive(Clone, Copy)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub width: usize,
    pub height: usize,
}

impl<'a> Plane<'a> {
    #[inline]
    pub fn row(&self, y: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + self.width]
    }
}

impl fmt::Debug for Plane<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Plane({}x{}, stride {})",
            self.width, self.height, self.stride
        )
    }
}

/// A decoded frame, borrowed from the [`Decoder`] until its next decode.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub color_space: ColorSpace,
    /// `color_range`: full (0..255) rather than studio (16..235).
    pub full_range: bool,
    pub planes: [Plane<'a>; 3],
}

/// One libvpx VP9 decoding context. `Send`, not `Sync` (the crate docs).
pub struct Decoder {
    // Boxed: libvpx keeps pointers to the context and to the configuration
    // for the decoder's life, so neither may move.
    ctx: Box<ffi::vpx_codec_ctx_t>,
    cfg: Box<ffi::vpx_codec_dec_cfg_t>,
    // Written by csrc/sk_vpx_mem.c during this decoder's calls; read here between calls.
    account: Box<UnsafeCell<ffi::sk_vpx_account>>,
    frame: *const ffi::vpx_image_t,
    failed: bool,
    _not_sync: PhantomData<std::cell::Cell<()>>,
}

// SAFETY: libvpx keeps no thread affinity; a context is used by one thread at
// a time (&mut self), and the account is only touched inside those calls.
unsafe impl Send for Decoder {}

struct Charging;

impl Charging {
    fn to(account: &UnsafeCell<ffi::sk_vpx_account>) -> Charging {
        // SAFETY: the account outlives the libvpx call this brackets.
        unsafe { ffi::sk_vpx_account_enter(account.get()) };
        Charging
    }
}

impl Drop for Charging {
    fn drop(&mut self) {
        // SAFETY: clears a thread-local pointer.
        unsafe { ffi::sk_vpx_account_leave() };
    }
}

/// libvpx's process-wide set-up, on one thread while any other waits. A VP9 decoder's first decode
/// runs `initialize_dec`, which fills the rtcd dispatch pointers and the intra predictor tables
/// behind libvpx's `once()`; with `CONFIG_MULTITHREAD 0` that is a plain flag, so two first decodes
/// on two threads would race. The same four functions run here first, under a [`Once`].
pub(crate) fn init_once() {
    static INIT: Once = Once::new();
    // SAFETY: libvpx's own initialisers, idempotent, run once.
    INIT.call_once(|| unsafe {
        ffi::vp9_rtcd();
        ffi::vpx_dsp_rtcd();
        ffi::vpx_scale_rtcd();
        ffi::vp9_init_intra_predictors();
    });
}

impl Decoder {
    pub fn new() -> Result<Decoder, Error> {
        init_once();
        Decoder::create()
    }

    fn create() -> Result<Decoder, Error> {
        // SAFETY: a zeroed vpx_codec_ctx_t is what libvpx expects to fill.
        let mut decoder = Decoder {
            ctx: Box::new(unsafe { std::mem::zeroed() }),
            cfg: Box::new(ffi::vpx_codec_dec_cfg_t {
                threads: 1,
                w: 0,
                h: 0,
            }),
            account: Box::new(UnsafeCell::new(ffi::sk_vpx_account::default())),
            frame: ptr::null(),
            failed: false,
            _not_sync: PhantomData,
        };
        decoder.init()?;
        Ok(decoder)
    }

    fn init(&mut self) -> Result<(), Error> {
        // SAFETY: as in create(): libvpx fills a zeroed context.
        *self.ctx = unsafe { std::mem::zeroed() };
        let _charging = Charging::to(&self.account);
        // SAFETY: ctx and cfg are boxed and live as long as the decoder.
        let err = unsafe {
            ffi::vpx_codec_dec_init_ver(
                &mut *self.ctx,
                ffi::vpx_codec_vp9_dx(),
                &*self.cfg,
                0,
                ffi::VPX_DECODER_ABI_VERSION,
            )
        };
        if err != ffi::VPX_CODEC_OK {
            // A failed init has released what it took (vpx_decoder.h).
            return Err(Error::new(err, self.ctx.err_detail));
        }
        self.failed = false;
        Ok(())
    }

    fn destroy(&mut self) {
        self.frame = ptr::null();
        if !self.ctx.priv_.is_null() {
            let _charging = Charging::to(&self.account);
            // SAFETY: an initialised context, destroyed once (priv_ is cleared
            // by libvpx's destroy).
            unsafe { ffi::vpx_codec_destroy(&mut *self.ctx) };
        }
        self.ctx.priv_ = ptr::null_mut();
    }

    /// Caps what this decoder may hold (0: no cap); a decode past it fails with [`ErrorCode::Mem`].
    pub fn set_memory_limit(&mut self, bytes: usize) {
        self.account.get_mut().limit = bytes;
    }

    pub fn memory(&self) -> MemoryUse {
        // SAFETY: no libvpx call is running on this decoder (&self, !Sync).
        let account = unsafe { &*self.account.get() };
        MemoryUse {
            live: account.live,
            peak: account.peak,
        }
    }

    /// Decodes one packet: a VP9 frame, or a superframe (at most one frame shown). `Ok(true)` when
    /// it shows a frame, which [`Decoder::frame`] then returns; `Ok(false)` for a packet that only
    /// updates references (a lone hidden frame).
    ///
    /// After an error the next decode starts from a fresh libvpx context, so it needs a keyframe.
    /// Not libvpx's own recovery, because libvpx 1.17 leaks a frame buffer on some error paths: a
    /// dozen such errors and the context has no buffer left, even for a good keyframe.
    pub fn decode(&mut self, packet: &[u8]) -> Result<bool, Error> {
        self.frame = ptr::null();
        if packet.is_empty() {
            // An empty packet is libvpx's "flush", not a frame.
            return Err(Error::invalid("empty VP9 packet"));
        }
        let len =
            c_uint::try_from(packet.len()).map_err(|_| Error::invalid("VP9 packet over 4 GiB"))?;
        if self.failed {
            self.destroy();
            self.init()?;
        }
        let _charging = Charging::to(&self.account);
        // SAFETY: the packet outlives the call; libvpx copies what it keeps.
        let err = unsafe {
            ffi::vpx_codec_decode(&mut *self.ctx, packet.as_ptr(), len, ptr::null_mut(), 0)
        };
        if err != ffi::VPX_CODEC_OK {
            self.failed = true;
            // SAFETY: the context is valid; the detail string is owned by it.
            return Err(Error::new(err, unsafe {
                ffi::vpx_codec_error_detail(&*self.ctx)
            }));
        }
        let mut iter: ffi::vpx_codec_iter_t = ptr::null();
        loop {
            // SAFETY: iterates the frames this decode produced.
            let image = unsafe { ffi::vpx_codec_get_frame(&mut *self.ctx, &mut iter) };
            if image.is_null() {
                break;
            }
            self.frame = image;
        }
        Ok(!self.frame.is_null())
    }

    pub fn frame(&self) -> Option<Frame<'_>> {
        if self.frame.is_null() {
            return None;
        }
        // SAFETY: libvpx keeps the image and its planes until the next decode
        // or the destroy, and both need &mut self.
        let img = unsafe { &*self.frame };
        if img.fmt & ffi::VPX_IMG_FMT_HIGHBITDEPTH != 0 {
            return None; // not built (CONFIG_VP9_HIGHBITDEPTH 0); never produced
        }
        let format = match img.fmt {
            ffi::VPX_IMG_FMT_I420 => Format::I420,
            ffi::VPX_IMG_FMT_I422 => Format::I422,
            ffi::VPX_IMG_FMT_I440 => Format::I440,
            ffi::VPX_IMG_FMT_I444 => Format::I444,
            _ => return None,
        };
        let (w, h) = (img.d_w as usize, img.d_h as usize);
        let (xs, ys) = (img.x_chroma_shift as usize, img.y_chroma_shift as usize);
        let plane = |i: usize, width: usize, height: usize| -> Option<Plane<'_>> {
            let stride = usize::try_from(img.stride[i]).ok()?;
            if img.planes[i].is_null() || stride < width || width == 0 || height == 0 {
                return None;
            }
            let len = stride * (height - 1) + width;
            // SAFETY: libvpx's plane holds `height` rows of `stride` bytes.
            let data = unsafe { std::slice::from_raw_parts(img.planes[i], len) };
            Some(Plane {
                data,
                stride,
                width,
                height,
            })
        };
        let (cw, ch) = ((w + xs) >> xs, (h + ys) >> ys);
        Some(Frame {
            format,
            width: img.d_w,
            height: img.d_h,
            color_space: ColorSpace::from_vpx(img.cs),
            full_range: img.range == 1,
            planes: [plane(0, w, h)?, plane(1, cw, ch)?, plane(2, cw, ch)?],
        })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(test)]
mod tests;
