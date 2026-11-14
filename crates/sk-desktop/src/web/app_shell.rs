//! `appShell`: what a browser cannot do. `openPath` refuses programs and scripts unless
//! `{ allowExecutable: true }`, and `openExternal` refuses `file:`, so a URL is never a way around that
//! guard. Calls reject on failure (`ENOENT`, `ENOASSOC`) with `code` and `osError`.

use std::ffi::c_void;

use sk_runtime::host;
use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;

use super::{awaiting_with, Errors};
use crate::{shell, Fail};

unsafe fn done(_g: *mut c_void, _token: u32, _v: &serde_json::Value, _made: u64) -> Result<u64, u64> {
    Ok(UNDEF)
}

unsafe fn run(g: *mut c_void, started: Result<u32, Fail>) -> u64 {
    awaiting_with(g, started, done, Errors::Desktop)
}

unsafe fn text(g: *mut c_void, v: u64) -> String {
    host::string_arg(g, v).unwrap_or_else(|| "undefined".into())
}

host_fn!(fn open_external(g, args) {
    unsafe { run(g, shell::start_open_external(&text(g, args.arg(0).0), false)) }
});

host_fn!(fn open_content(g, args) {
    unsafe { run(g, shell::start_open_external(&text(g, args.arg(0).0), true)) }
});

host_fn!(fn open_path(g, args) {
    unsafe {
        let options = args.arg(1).0;
        let allow = is_cell(options) && get(g, options, "allowExecutable") == TRUE;
        run(g, shell::start_open_path(&text(g, args.arg(0).0), allow))
    }
});

host_fn!(fn show_item(g, args) {
    unsafe { run(g, shell::start_show_item(&text(g, args.arg(0).0))) }
});

host_fn!(fn trash_item(g, args) {
    unsafe { run(g, shell::start_trash(&text(g, args.arg(0).0))) }
});

pub(super) unsafe fn install(g: *mut c_void) {
    let obj = host::install_object(g, "__SK_SHELL__", UNDEF);
    for (name, arity, f) in [
        ("openExternal", 1, open_external as HostFn),
        ("openPath", 1, open_path),
        ("showItemInFolder", 1, show_item),
        ("trashItem", 1, trash_item),
    ] {
        define(g, obj, name, make_fn(g, name, arity, f), 0);
    }
    host::method(g, obj, "openContent", 1, open_content);
}

#[cfg(test)]
mod tests {
    #[test]
    fn refusals_reject_with_name_code_and_a_plain_error() {
        let out = super::super::testing::run(
            "const r = []; \
             try { await __SK_SHELL__.openExternal('file:///x'); } catch (e) { r.push(e instanceof TypeError); } \
             const exe = navigator.platform.startsWith('Win') ? 'C:\\\\x\\\\setup.exe' : '/x/setup.exe'; \
             try { await __SK_SHELL__.openPath(exe); } catch (e) { r.push(e.name, e.code, e instanceof Error, typeof DOMException === 'function' && e instanceof DOMException); } \
             try { await __SK_SHELL__.openPath('relative'); } catch (e) { r.push(e instanceof TypeError); } \
             r.push(Object.keys(__SK_SHELL__).join('+')); \
             return r.join('|');",
        );
        assert_eq!(out, "true|NotAllowedError|EEXECUTABLE|true|false|true|openExternal+openPath+showItemInFolder+trashItem");
    }
}
