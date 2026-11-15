//! The slice of the GStreamer C API the backend calls, resolved with `dlopen` at first use so the
//! host has no link-time dependency on GStreamer and starts on a system without it.
//!
//! The struct layouts below are the public, ABI-stable ones of GStreamer 1.x on 64-bit Linux.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::sync::OnceLock;

use libloading::Library;

pub type Ptr = *mut c_void;
pub type GBool = c_int;

pub const FALSE: GBool = 0;
pub const TRUE: GBool = 1;

pub const STATE_NULL: c_int = 1;
pub const STATE_PAUSED: c_int = 3;
pub const STATE_PLAYING: c_int = 4;
pub const STATE_CHANGE_FAILURE: c_int = 0;
pub const STATE_CHANGE_SUCCESS: c_int = 1;
pub const STATE_CHANGE_ASYNC: c_int = 2;

pub const FORMAT_BYTES: c_int = 2;
pub const FORMAT_TIME: c_int = 3;
pub const FORMAT_PERCENT: c_int = 4;
pub const FORMAT_PERCENT_MAX: i64 = 1_000_000;

pub const SEEK_FLAG_FLUSH: c_uint = 1 << 0;
pub const SEEK_FLAG_ACCURATE: c_uint = 1 << 1;
pub const SEEK_FLAG_INSTANT_RATE_CHANGE: c_uint = 1 << 10;
pub const SEEK_TYPE_NONE: c_int = 0;
pub const SEEK_TYPE_SET: c_int = 1;

pub const MSG_EOS: u32 = 1 << 0;
pub const MSG_ERROR: u32 = 1 << 1;
pub const MSG_WARNING: u32 = 1 << 2;
pub const MSG_BUFFERING: u32 = 1 << 5;
pub const MSG_CLOCK_LOST: u32 = 1 << 10;
pub const MSG_ELEMENT: u32 = 1 << 15;
pub const MSG_DURATION_CHANGED: u32 = 1 << 18;
pub const MSG_LATENCY: u32 = 1 << 19;
pub const MSG_ASYNC_DONE: u32 = 1 << 21;

pub const MAP_READ: c_uint = 1;
pub const MEMORY_FLAG_READONLY: c_uint = 2;
pub const FLOW_OK: c_int = 0;
pub const PAD_SINK: c_int = 2;
pub const RANK_MARGINAL: c_uint = 64;
pub const APP_STREAM_RANDOM_ACCESS: c_int = 2;
pub const CLOCK_TIME_NONE: u64 = u64::MAX;

pub const FACTORY_DECODER: u64 = 1 << 0;
pub const FACTORY_DEMUXER: u64 = 1 << 5;
pub const FACTORY_PARSER: u64 = 1 << 6;

pub const PLAY_FLAG_TEXT: c_uint = 1 << 2;
pub const PLAY_FLAG_NATIVE_VIDEO: c_uint = 1 << 6;

#[repr(C)]
pub struct GError {
    pub domain: u32,
    pub code: c_int,
    pub message: *mut c_char,
}

#[repr(C)]
pub struct MiniObject {
    pub type_: usize,
    pub refcount: c_int,
    pub lockstate: c_int,
    pub flags: c_uint,
    pub copy: Ptr,
    pub dispose: Ptr,
    pub free: Ptr,
    pub priv_uint: c_uint,
    pub priv_pointer: Ptr,
}

#[repr(C)]
pub struct Message {
    pub mini: MiniObject,
    pub type_: u32,
    pub timestamp: u64,
    pub src: Ptr,
    pub seqnum: u32,
}

#[repr(C)]
pub struct Buffer {
    pub mini: MiniObject,
    pub pool: Ptr,
    pub pts: u64,
    pub dts: u64,
    pub duration: u64,
}

#[repr(C)]
pub struct MapInfo {
    pub memory: Ptr,
    pub flags: c_uint,
    pub data: *mut u8,
    pub size: usize,
    pub maxsize: usize,
    pub user_data: [Ptr; 4],
    pub reserved: [Ptr; 4],
}

impl MapInfo {
    pub fn zeroed() -> Self {
        // SAFETY: plain data; every field is an integer or a pointer, for which zero is valid.
        unsafe { std::mem::zeroed() }
    }
}

/// `GstVideoMeta` up to the first plane's stride.
#[repr(C)]
pub struct VideoMeta {
    pub meta_flags: c_uint,
    pub meta_info: Ptr,
    pub buffer: Ptr,
    pub flags: c_uint,
    pub format: c_uint,
    pub id: c_uint,
    pub width: c_uint,
    pub height: c_uint,
    pub n_planes: c_uint,
    pub offset: [usize; 4],
    pub stride: [c_int; 4],
}

pub type SinkCallback = unsafe extern "C" fn(sink: Ptr, data: Ptr) -> c_int;

#[repr(C)]
pub struct AppSinkCallbacks {
    pub eos: Option<unsafe extern "C" fn(sink: Ptr, data: Ptr)>,
    pub new_preroll: Option<SinkCallback>,
    pub new_sample: Option<SinkCallback>,
    pub new_event: Ptr,
    pub propose_allocation: Ptr,
    pub reserved: [Ptr; 2],
}

#[repr(C)]
pub struct AppSrcCallbacks {
    pub need_data: Option<unsafe extern "C" fn(src: Ptr, length: c_uint, data: Ptr)>,
    pub enough_data: Option<unsafe extern "C" fn(src: Ptr, data: Ptr)>,
    pub seek_data: Option<unsafe extern "C" fn(src: Ptr, offset: u64, data: Ptr) -> GBool>,
    pub reserved: [Ptr; 4],
}

pub type DestroyNotify = unsafe extern "C" fn(data: Ptr);
pub type ClosureNotify = unsafe extern "C" fn(data: Ptr, closure: Ptr);

macro_rules! table {
    ($name:ident { $($field:ident : $ty:ty,)* }) => {
        pub struct $name {
            $(pub $field: $ty,)*
        }

        impl $name {
            /// # Safety
            /// Each symbol must have the C signature its field declares.
            unsafe fn load(lib: &Library) -> Result<Self, String> {
                Ok(Self {
                    $($field: unsafe { sym(lib, stringify!($field))? },)*
                })
            }
        }
    };
}

unsafe fn sym<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
    // SAFETY: the caller states the symbol's type.
    unsafe { lib.get::<T>(name.as_bytes()) }
        .map(|s| *s)
        .map_err(|e| format!("{name}: {e}"))
}

table!(Glib {
    g_free: unsafe extern "C" fn(Ptr),
    g_error_free: unsafe extern "C" fn(*mut GError),
});

table!(Gobject {
    g_object_set: unsafe extern "C" fn(Ptr, *const c_char, ...),
    g_object_get: unsafe extern "C" fn(Ptr, *const c_char, ...),
    g_signal_connect_data:
        unsafe extern "C" fn(Ptr, *const c_char, Ptr, Ptr, Option<ClosureNotify>, c_int) -> u64,
});

table!(Core {
    gst_init_check: unsafe extern "C" fn(*mut c_int, Ptr, *mut *mut GError) -> GBool,
    gst_version: unsafe extern "C" fn(*mut c_uint, *mut c_uint, *mut c_uint, *mut c_uint),
    gst_core_error_quark: unsafe extern "C" fn() -> u32,
    gst_resource_error_quark: unsafe extern "C" fn() -> u32,
    gst_stream_error_quark: unsafe extern "C" fn() -> u32,
    gst_mini_object_unref: unsafe extern "C" fn(Ptr),
    gst_object_unref: unsafe extern "C" fn(Ptr),
    gst_object_ref_sink: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_element_factory_make: unsafe extern "C" fn(*const c_char, *const c_char) -> Ptr,
    gst_element_factory_find: unsafe extern "C" fn(*const c_char) -> Ptr,
    gst_element_get_factory: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_object_get_name: unsafe extern "C" fn(Ptr) -> *mut c_char,
    gst_parse_bin_from_description:
        unsafe extern "C" fn(*const c_char, GBool, *mut *mut GError) -> Ptr,
    gst_bin_get_by_name: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    gst_bin_recalculate_latency: unsafe extern "C" fn(Ptr) -> GBool,
    gst_element_set_state: unsafe extern "C" fn(Ptr, c_int) -> c_int,
    gst_element_get_state: unsafe extern "C" fn(Ptr, *mut c_int, *mut c_int, u64) -> c_int,
    gst_element_get_bus: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_element_get_static_pad: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    gst_element_query: unsafe extern "C" fn(Ptr, Ptr) -> GBool,
    gst_element_query_position: unsafe extern "C" fn(Ptr, c_int, *mut i64) -> GBool,
    gst_element_query_duration: unsafe extern "C" fn(Ptr, c_int, *mut i64) -> GBool,
    gst_element_seek:
        unsafe extern "C" fn(Ptr, f64, c_int, c_uint, c_int, i64, c_int, i64) -> GBool,
    gst_bus_pop: unsafe extern "C" fn(Ptr) -> *mut Message,
    gst_message_parse_error: unsafe extern "C" fn(*mut Message, *mut *mut GError, *mut *mut c_char),
    gst_message_parse_warning:
        unsafe extern "C" fn(*mut Message, *mut *mut GError, *mut *mut c_char),
    gst_message_parse_buffering: unsafe extern "C" fn(*mut Message, *mut c_int),
    gst_message_get_structure: unsafe extern "C" fn(*mut Message) -> Ptr,
    gst_structure_get_name: unsafe extern "C" fn(Ptr) -> *const c_char,
    gst_structure_get_int: unsafe extern "C" fn(Ptr, *const c_char, *mut c_int) -> GBool,
    gst_structure_get_fraction:
        unsafe extern "C" fn(Ptr, *const c_char, *mut c_int, *mut c_int) -> GBool,
    gst_structure_to_string: unsafe extern "C" fn(Ptr) -> *mut c_char,
    gst_caps_from_string: unsafe extern "C" fn(*const c_char) -> Ptr,
    gst_caps_get_structure: unsafe extern "C" fn(Ptr, c_uint) -> Ptr,
    gst_pad_get_current_caps: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_sample_get_buffer: unsafe extern "C" fn(Ptr) -> *mut Buffer,
    gst_sample_get_caps: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_sample_get_segment: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_segment_to_stream_time: unsafe extern "C" fn(Ptr, c_int, u64) -> u64,
    gst_buffer_map: unsafe extern "C" fn(*mut Buffer, *mut MapInfo, c_uint) -> GBool,
    gst_buffer_unmap: unsafe extern "C" fn(*mut Buffer, *mut MapInfo),
    gst_buffer_new_wrapped_full:
        unsafe extern "C" fn(c_uint, Ptr, usize, usize, usize, Ptr, Option<DestroyNotify>) -> *mut Buffer,
    gst_query_new_buffering: unsafe extern "C" fn(c_int) -> Ptr,
    gst_query_parse_buffering_range:
        unsafe extern "C" fn(Ptr, *mut c_int, *mut i64, *mut i64, *mut i64),
    gst_query_get_n_buffering_ranges: unsafe extern "C" fn(Ptr) -> c_uint,
    gst_query_parse_nth_buffering_range: unsafe extern "C" fn(Ptr, c_uint, *mut i64, *mut i64) -> GBool,
    gst_query_new_seeking: unsafe extern "C" fn(c_int) -> Ptr,
    gst_query_parse_seeking: unsafe extern "C" fn(Ptr, *mut c_int, *mut GBool, *mut i64, *mut i64),
    gst_element_factory_list_get_elements: unsafe extern "C" fn(u64, c_uint) -> *mut GList,
    gst_element_factory_list_filter:
        unsafe extern "C" fn(*mut GList, Ptr, c_int, GBool) -> *mut GList,
    gst_plugin_feature_list_free: unsafe extern "C" fn(*mut GList),
    gst_filename_to_uri: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_char,
});

table!(App {
    gst_app_sink_set_callbacks:
        unsafe extern "C" fn(Ptr, *const AppSinkCallbacks, Ptr, Option<DestroyNotify>),
    gst_app_sink_pull_sample: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_app_sink_pull_preroll: unsafe extern "C" fn(Ptr) -> Ptr,
    gst_app_src_set_callbacks:
        unsafe extern "C" fn(Ptr, *const AppSrcCallbacks, Ptr, Option<DestroyNotify>),
    gst_app_src_push_buffer: unsafe extern "C" fn(Ptr, *mut Buffer) -> c_int,
    gst_app_src_end_of_stream: unsafe extern "C" fn(Ptr) -> c_int,
});

table!(Video {
    gst_buffer_get_video_meta: unsafe extern "C" fn(*mut Buffer) -> *const VideoMeta,
});

table!(PbUtils {
    gst_pb_utils_init: unsafe extern "C" fn(),
    gst_is_missing_plugin_message: unsafe extern "C" fn(*mut Message) -> GBool,
    gst_missing_plugin_message_get_description: unsafe extern "C" fn(*mut Message) -> *mut c_char,
    gst_missing_plugin_message_get_installer_detail:
        unsafe extern "C" fn(*mut Message) -> *mut c_char,
});

#[repr(C)]
pub struct GList {
    pub data: Ptr,
    pub next: *mut GList,
    pub prev: *mut GList,
}

pub struct Api {
    pub glib: Glib,
    pub gobject: Gobject,
    pub core: Core,
    pub app: App,
    pub video: Option<Video>,
    pub pbutils: Option<PbUtils>,
    pub version: (u32, u32, u32),
    _libs: Vec<Library>,
}

// SAFETY: a table of C function pointers into libraries that stay loaded for the process.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

/// Why GStreamer could not be used.
#[derive(Debug, Clone)]
pub enum Unavailable {
    /// A library is not installed.
    Missing(String),
    /// The libraries load but lack a symbol, or `gst_init` failed.
    Broken(String),
}

pub fn api() -> Result<&'static Api, Unavailable> {
    static API: OnceLock<Result<Api, Unavailable>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

fn open(name: &str) -> Result<Library, Unavailable> {
    // SAFETY: the GLib and GStreamer libraries run no unsound initialisers on load.
    unsafe { Library::new(name) }.map_err(|e| Unavailable::Missing(format!("{name}: {e}")))
}

fn load() -> Result<Api, Unavailable> {
    let fail = Unavailable::Broken;
    let glib_lib = open("libglib-2.0.so.0")?;
    let gobject_lib = open("libgobject-2.0.so.0")?;
    let core_lib = open("libgstreamer-1.0.so.0")?;
    let app_lib = open("libgstapp-1.0.so.0")?;
    let video_lib = open("libgstvideo-1.0.so.0").ok();
    let pbutils_lib = open("libgstpbutils-1.0.so.0").ok();
    // SAFETY: each table's fields carry the C signatures of the symbols they name.
    let (glib, gobject, core, app, video, pbutils) = unsafe {
        (
            Glib::load(&glib_lib).map_err(fail)?,
            Gobject::load(&gobject_lib).map_err(fail)?,
            Core::load(&core_lib).map_err(fail)?,
            App::load(&app_lib).map_err(fail)?,
            video_lib.as_ref().and_then(|l| Video::load(l).ok()),
            pbutils_lib.as_ref().and_then(|l| PbUtils::load(l).ok()),
        )
    };
    let mut err: *mut GError = std::ptr::null_mut();
    // SAFETY: `gst_init_check` accepts NULL argc/argv and is safe to call more than once.
    if unsafe { (core.gst_init_check)(std::ptr::null_mut(), std::ptr::null_mut(), &mut err) }
        == FALSE
    {
        let text = unsafe { take_error(&glib, err) }.unwrap_or_default();
        return Err(Unavailable::Broken(format!("gst_init failed: {text}")));
    }
    let (mut major, mut minor, mut micro, mut nano) = (0, 0, 0, 0);
    unsafe { (core.gst_version)(&mut major, &mut minor, &mut micro, &mut nano) };
    if let Some(pb) = &pbutils {
        unsafe { (pb.gst_pb_utils_init)() };
    }
    let mut libs = vec![glib_lib, gobject_lib, core_lib, app_lib];
    libs.extend(video_lib);
    libs.extend(pbutils_lib);
    Ok(Api {
        glib,
        gobject,
        core,
        app,
        video,
        pbutils,
        version: (major, minor, micro),
        _libs: libs,
    })
}

/// Frees a `GError` and returns its message.
///
/// # Safety
/// `err` is NULL or a `GError` the caller owns.
pub unsafe fn take_error(glib: &Glib, err: *mut GError) -> Option<String> {
    if err.is_null() {
        return None;
    }
    let text = unsafe { cstr((*err).message) };
    unsafe { (glib.g_error_free)(err) };
    Some(text)
}

/// # Safety
/// `p` is NULL or a NUL-terminated string.
pub unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

impl Api {
    /// Takes ownership of a `g_malloc`ed string.
    ///
    /// # Safety
    /// `p` is NULL or a string the caller owns.
    pub unsafe fn owned(&self, p: *mut c_char) -> String {
        let s = unsafe { cstr(p) };
        if !p.is_null() {
            unsafe { (self.glib.g_free)(p.cast()) };
        }
        s
    }

    pub fn has_factory(&self, name: &str) -> bool {
        let name = c(name);
        // SAFETY: a lookup by name; the returned reference is released at once.
        unsafe {
            let factory = (self.core.gst_element_factory_find)(name.as_ptr());
            if factory.is_null() {
                return false;
            }
            (self.core.gst_object_unref)(factory);
        }
        true
    }

    /// `gst_parse_bin_from_description` with ghost pads, owned (not floating).
    pub fn parse_bin(&self, description: &str) -> Result<Ptr, String> {
        let text = c(description);
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: a NUL-terminated description and an out-pointer for the error.
        let bin =
            unsafe { (self.core.gst_parse_bin_from_description)(text.as_ptr(), TRUE, &mut err) };
        let error = unsafe { take_error(&self.glib, err) };
        if bin.is_null() {
            return Err(error.unwrap_or_else(|| description.to_string()));
        }
        // SAFETY: a new floating reference becomes ours.
        Ok(unsafe { (self.core.gst_object_ref_sink)(bin) })
    }

    pub fn set_bool(&self, object: Ptr, name: &str, value: bool) {
        let name = c(name);
        // SAFETY: a gboolean property; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_set)(
                object,
                name.as_ptr(),
                value as GBool,
                std::ptr::null::<c_char>(),
            )
        };
    }

    pub fn set_f64(&self, object: Ptr, name: &str, value: f64) {
        let name = c(name);
        // SAFETY: a gdouble property; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_set)(object, name.as_ptr(), value, std::ptr::null::<c_char>())
        };
    }

    pub fn set_i32(&self, object: Ptr, name: &str, value: c_int) {
        let name = c(name);
        // SAFETY: a gint, guint, enum or flags property; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_set)(object, name.as_ptr(), value, std::ptr::null::<c_char>())
        };
    }

    pub fn set_i64(&self, object: Ptr, name: &str, value: i64) {
        let name = c(name);
        // SAFETY: a gint64 property; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_set)(object, name.as_ptr(), value, std::ptr::null::<c_char>())
        };
    }

    pub fn set_str(&self, object: Ptr, name: &str, value: &str) {
        let (name, value) = (c(name), c(value));
        // SAFETY: a string property, copied by GObject; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_set)(
                object,
                name.as_ptr(),
                value.as_ptr(),
                std::ptr::null::<c_char>(),
            )
        };
    }

    /// Sets an object or boxed property; the property takes its own reference.
    pub fn set_ptr(&self, object: Ptr, name: &str, value: Ptr) {
        let name = c(name);
        // SAFETY: an object/boxed property; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_set)(object, name.as_ptr(), value, std::ptr::null::<c_char>())
        };
    }

    pub fn get_u32(&self, object: Ptr, name: &str) -> c_uint {
        let name = c(name);
        let mut out: c_uint = 0;
        // SAFETY: a 32-bit property read into a local; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_get)(
                object,
                name.as_ptr(),
                &mut out as *mut c_uint,
                std::ptr::null::<c_char>(),
            )
        };
        out
    }

    pub fn get_f64(&self, object: Ptr, name: &str) -> f64 {
        let name = c(name);
        let mut out = 0.0f64;
        // SAFETY: a gdouble property read into a local; the vararg list is NULL-terminated.
        unsafe {
            (self.gobject.g_object_get)(
                object,
                name.as_ptr(),
                &mut out as *mut f64,
                std::ptr::null::<c_char>(),
            )
        };
        out
    }

    /// `(width, height, pixel-aspect-ratio)` of the first structure of `caps`.
    ///
    /// # Safety
    /// `caps` is NULL or a live `GstCaps`.
    pub unsafe fn video_caps(&self, caps: Ptr) -> Option<(u32, u32, (i32, i32))> {
        if caps.is_null() {
            return None;
        }
        let s = unsafe { (self.core.gst_caps_get_structure)(caps, 0) };
        if s.is_null() {
            return None;
        }
        let (mut w, mut h, mut n, mut d) = (0, 0, 1, 1);
        let width = c("width");
        let height = c("height");
        let par = c("pixel-aspect-ratio");
        unsafe {
            if (self.core.gst_structure_get_int)(s, width.as_ptr(), &mut w) == FALSE
                || (self.core.gst_structure_get_int)(s, height.as_ptr(), &mut h) == FALSE
            {
                return None;
            }
            if (self.core.gst_structure_get_fraction)(s, par.as_ptr(), &mut n, &mut d) == FALSE
                || n <= 0
                || d <= 0
            {
                (n, d) = (1, 1);
            }
        }
        (w > 0 && h > 0).then_some((w as u32, h as u32, (n, d)))
    }
}

pub fn c(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}
