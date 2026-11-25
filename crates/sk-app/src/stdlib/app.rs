//! `skathi:app`: ending the app, starting it again, its version, and its wheel scroll scale.

use std::ffi::{c_void, OsString};

use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host, host_fn};

/// `appWindow.close()`: `pagehide`, then the close every other quit takes.
unsafe fn close(g: *mut c_void) {
    let window = get(g, get(g, host::global(g), "__SK_WINDOW__"), "appWindow");
    let _ = call(g, get(g, window, "close"), window, &[]);
}

unsafe fn option(g: *mut c_void, options: u64, name: &str) -> u64 {
    if is_cell(options) {
        get(g, options, name)
    } else {
        UNDEF
    }
}

host_fn!(fn quit(g, args) {
    unsafe {
        let code = option(g, args.arg(0).0, "exitCode");
        if code != UNDEF {
            let Some(code) = dec_f64(code).filter(|n| n.fract() == 0.0 && (0.0..=255.0).contains(n)) else {
                return throw_type_error(g, "app.quit: exitCode must be an integer from 0 to 255");
            };
            crate::shutdown::set_exit_code(code as i32);
        }
        close(g);
    }
    UNDEF
});

unsafe fn strings(g: *mut c_void, list: u64) -> Option<Vec<OsString>> {
    let is_array = get(g, get(g, host::global(g), "Array"), "isArray");
    if call(g, is_array, UNDEF, &[list]) != Ok(TRUE) {
        return None;
    }
    (0..array_len(g, list))
        .map(|i| {
            let item = get_i(g, list, i);
            host::is_string(g, item)
                .then(|| read_string(g, item))
                .flatten()
                .map(OsString::from)
        })
        .collect()
}

host_fn!(fn restart(g, args) {
    unsafe {
        if crate::embed::active() {
            return host::throw(
                g,
                "NotSupportedError",
                "app.restart: this app runs inside another program, which owns the process; ask it to restart through a native module",
            );
        }
        let requested = option(g, args.arg(0).0, "args");
        let args = if requested == UNDEF {
            crate::crash_window::relaunch_args(std::env::args_os())
        } else {
            let Some(args) = strings(g, requested) else {
                return throw_type_error(g, "app.restart: args must be an array of strings");
            };
            args
        };
        if crate::shutdown::is_closing() {
            return UNDEF;
        }
        // Before the new process starts: a single-instance app's second launch is told this one
        // is closing, and waits for it to end.
        crate::shutdown::begin("app.restart()");
        let line = match crate::crash_window::relaunch(args) {
            Ok(pid) => format!("[shutdown] app.restart: started process {pid}"),
            Err(e) => format!("[shutdown] app.restart: the app could not be started again: {e}"),
        };
        eprintln!("{line}");
        sk_tree::log::record("shutdown", line);
        close(g);
    }
    UNDEF
});

host_fn!(fn get_version(g, _args) {
    let version = crate::boot_mode::app_version().unwrap_or_else(|| "0.0.0".into());
    unsafe { js_string(g, &version) }
});

host_fn!(fn get_wheel_scroll_scale(_g, _args) {
    enc_f64(crate::wheel_scroll::get())
});

host_fn!(fn set_wheel_scroll_scale(g, args) {
    let Some(scale) = dec_f64(args.arg(0).0).and_then(crate::wheel_scroll::clamp) else {
        return unsafe {
            throw_type_error(g, "app.wheelScrollScale must be a finite number above 0 (1 scrolls as a browser does)")
        };
    };
    crate::wheel_scroll::set(scale);
    UNDEF
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let class = super::support::class(
        g,
        &ClassSpec {
            name: "App",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("quit", 0, quit),
                ("restart", 0, restart),
                ("getVersion", 0, get_version),
            ],
            getters: &[],
            accessors: &[(
                "wheelScrollScale",
                get_wheel_scroll_scale,
                set_wheel_scroll_scale,
            )],
        },
    );
    define(g, std, "app", new_object(g, class.proto), 0);
}
