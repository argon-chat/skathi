//! The `skathi:*` modules a host provides itself.
//! Their objects hang off the hidden global `__sk_std`, which the runtime's `std/*.ts` export.

use std::ffi::c_void;

use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host, JsRuntime};

mod app;
mod binary;
mod collections;
mod encoding;
mod ffi;
mod image;
pub(crate) mod memory;
mod os;
mod process;
mod runtime;
mod support;
mod text;
mod time;
mod toml;
mod units;

#[cfg(feature = "native-aot")]
pub(crate) use runtime::set_app_native;

/// Settles the promises of nonblocking FFI calls, image work, child processes and differences
/// that finished on other threads.
pub(crate) fn pump() {
    ffi::pump();
    image::pump();
    process::pump();
    text::pump();
}

/// Kills every child process `skathi:process` started.
#[cfg_attr(not(feature = "process"), allow(dead_code))]
pub(crate) fn end_processes() {
    process::end_all();
}

unsafe fn frozen(g: *mut c_void, object: u64) -> u64 {
    let freeze = get(g, get(g, host::global(g), "Object"), "freeze");
    let _ = call(g, freeze, UNDEF, &[object]);
    object
}

pub(crate) fn install(rt: &JsRuntime) {
    unsafe {
        let g = rt.raw();
        let std = host::install_object(g, "__sk_std", UNDEF);
        app::install(g, std);
        os::install(g, std);
        runtime::install(g, std);
        binary::install(g, std);
        collections::install(g, std);
        encoding::install(g, std);
        ffi::install(g, std);
        image::install(g, std);
        memory::install(g, std);
        process::install(g, std);
        text::install(g, std);
        time::install(g, std);
        units::install(g, std);
        if let Err(e) = toml::install(rt.ctx(), crate::js::Value(std)) {
            eprintln!("[std] skathi:toml could not be installed: {e}");
        }
    }
}
