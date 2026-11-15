//! The JS host seam of Skathi: one JSC context (via redjs) on gpui's main thread, pumped once per
//! frame, with a phase-gated bridge as the only way a JS host function may reach `&mut Window` / `&mut App`.
//!
//! Invariants:
//! - One VM, one thread: VM thread == gpui main thread == Win32 UI thread.
//! - The context is created once and never released (teardown fastfails in
//!   this JSC prebuilt).
//! - Every `extern "sysv64"` host function is written with [`host_fn!`] and is
//!   therefore `catch_unwind`-shielded: a Rust panic becomes a JS exception
//!   and poisons the runtime instead of aborting the process.
//! - The uncaught-JS handler is a pure sink; errors are drained after each
//!   pump, never acted on re-entrantly.

pub mod bridge;
pub mod frames;
pub mod host;
pub mod runtime;
pub mod testing;
mod unsupported;

pub use bridge::{with_app, with_commit, with_published, with_published_app, Phase};
pub use runtime::{JsError, JsRuntime};

pub use redjs;
#[cfg(feature = "webgpu")]
pub use redjs_webgpu;

pub mod built {
    pub const WEBGPU: bool = cfg!(feature = "webgpu");
    pub const AUDIO: bool = cfg!(feature = "audio");
    pub const STORAGES: bool = cfg!(feature = "storages");
    pub const CLIPBOARD: bool = cfg!(feature = "clipboard");
    pub const INDEXEDDB: bool = cfg!(feature = "indexeddb");
}

/// Defines a panic-shielded JSC host function, in redjs's host ABI: a panic becomes a thrown JS `Error`
/// and poisons the runtime.
#[macro_export]
macro_rules! host_fn {
    ($vis:vis fn $name:ident($g:ident, $args:ident) $body:block) => {
        $crate::raw_host_fn! {
        $vis fn $name(g, cf) {
            match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                let $g = g;
                let _profile = $crate::redjs::profiling::host_span_at(g as usize, ::core::stringify!($name), ::core::file!(), ::core::line!(), ::core::column!());
                let $args = unsafe { $crate::redjs::api::HostArgs::new(cf) };
                let _ = &$args;
                $body
            })) {
                Ok(v) => v,
                Err(payload) => $crate::bridge::panic_to_js(g, payload),
            }
        }
        }
    };
}

/// A bare host function in redjs's ABI (`extern "sysv64"` on Windows x64, `extern "C"` elsewhere); no panic shield.
#[macro_export]
macro_rules! raw_host_fn {
    ($vis:vis fn $name:ident($g:ident, $cf:ident) $body:block) => {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        $vis unsafe extern "sysv64" fn $name($g: *mut ::core::ffi::c_void, $cf: *mut ::core::ffi::c_void) -> u64 $body
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        $vis unsafe extern "C" fn $name($g: *mut ::core::ffi::c_void, $cf: *mut ::core::ffi::c_void) -> u64 $body
    };
}
