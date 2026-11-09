//! vn-runtime — the JS host seam of vue-native: one JSC context (via redjs) on
//! gpui's main thread, pumped once per frame, with a phase-gated bridge that is
//! the only way a JS host function may reach `&mut Window` / `&mut App`.
//!
//! Invariants (see docs/ARCHITECTURE.md §1 and research/reports/fill-1.md §9):
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
pub mod runtime;
pub mod testing;

pub use bridge::{Phase, with_app, with_commit, with_published, with_published_app};
pub use runtime::{JsError, JsRuntime};

// Re-exported so `host_fn!` expansions and downstream crates use the same redjs.
pub use redjs;

/// The optional web APIs this build of the crate carries, as Cargo resolved
/// them. Features unify across the graph, so a host can only check that what
/// it asked for is what it got (vn-app's payload tests do) by asking here.
pub mod built {
    pub const WEBGPU: bool = cfg!(feature = "webgpu");
    pub const AUDIO: bool = cfg!(feature = "audio");
    pub const STORAGES: bool = cfg!(feature = "storages");
    pub const CLIPBOARD: bool = cfg!(feature = "clipboard");
    pub const INDEXEDDB: bool = cfg!(feature = "indexeddb");
}

/// Defines a panic-shielded JSC host function, in redjs's host ABI
/// ([`raw_host_fn!`]).
///
/// The body receives `$g: *mut c_void` (the JSGlobalObject) and
/// `$args: redjs::api::HostArgs`, and must evaluate to a `u64` (an encoded
/// JSValue — `redjs::api::Value::undefined().0` when there is nothing to
/// return). A panic inside the body is converted into a thrown JS `Error` and
/// poisons the runtime (I9): the pump stops scheduling further JS.
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

/// A bare host function in redjs's host ABI (`redjs::webapi::util::HostFn`):
/// `extern "sysv64"` on Windows x64, where the engine calls through the System
/// V convention, and `extern "C"` everywhere else. The body sees `$g` (the
/// JSGlobalObject) and `$cf` (the call frame) as `*mut c_void` and returns the
/// encoded result. No panic shield: that is [`host_fn!`]'s.
#[macro_export]
macro_rules! raw_host_fn {
    ($vis:vis fn $name:ident($g:ident, $cf:ident) $body:block) => {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        $vis unsafe extern "sysv64" fn $name($g: *mut ::core::ffi::c_void, $cf: *mut ::core::ffi::c_void) -> u64 $body
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        $vis unsafe extern "C" fn $name($g: *mut ::core::ffi::c_void, $cf: *mut ::core::ffi::c_void) -> u64 $body
    };
}
