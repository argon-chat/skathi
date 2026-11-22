//! `skathi:process`: the programs sk.config.json lists, started without a shell.

use std::ffi::c_void;

#[cfg(not(feature = "process"))]
use sk_runtime::host;
use sk_runtime::redjs::webapi::util::*;

#[cfg(not(feature = "process"))]
use super::support::{self, *};

#[cfg(feature = "process")]
mod child;
#[cfg(feature = "process")]
mod live;
#[cfg(feature = "process")]
mod resolve;
#[cfg(all(test, feature = "process"))]
mod tests;
#[cfg(all(feature = "process", unix))]
mod unix;
#[cfg(all(feature = "process", windows))]
mod win;

/// Settles reads, writes and exits that other threads finished.
pub(crate) fn pump() {
    #[cfg(feature = "process")]
    live::pump();
}

/// The quit hook: no child outlives the app.
pub(crate) fn end_all() {
    #[cfg(feature = "process")]
    child::end_all(std::time::Duration::from_millis(500));
}

#[cfg(not(feature = "process"))]
const OFF: &str = "this host was built without skathi:process (the \"process\" feature)";

#[cfg(not(feature = "process"))]
std_fn!(fn spawn_off(g, _args) {
    host::throw(g, "NotSupportedError", &format!("Process.spawn: {OFF}"));
    Err(Thrown)
});

#[cfg(not(feature = "process"))]
std_fn!(fn exec_off(g, _args) {
    Ok(host::rejected(g, "NotSupportedError", &format!("Process.exec: {OFF}")))
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let module = new_object(g, UNDEF);
    #[cfg(feature = "process")]
    let ctor = live::class(g);
    #[cfg(not(feature = "process"))]
    let ctor = {
        let class = support::class(
            g,
            &ClassSpec {
                name: "Process",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[],
                getters: &[],
                accessors: &[],
            },
        );
        static_fn(g, class.ctor, "spawn", 3, spawn_off);
        static_fn(g, class.ctor, "exec", 3, exec_off);
        class.ctor
    };
    define(g, module, "Process", ctor, 0);
    define(g, std, "process", module, 0);
}
