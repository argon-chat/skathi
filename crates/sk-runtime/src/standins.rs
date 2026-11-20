//! Inert app API objects for a host built or run without the crate behind one, so `appShell`, `appTray`
//! and the rest always exist; each says once why it does nothing.

use std::ffi::c_void;

use redjs::webapi::util::*;
use serde_json::json;

use crate::host::{self, global, warn_compat};

const PERMISSIONS: &str =
    "the permissions host is missing (sk_permissions::install did not run) — \
                           `appPermissions` is inert and every capability reads as 'unsupported'.";
const STORAGE: &str = "the storage host is missing (sk_storage::install did not run, or the host was built \
                       without the `storages` feature) — `appStorage` is inert and localStorage does not persist.";
const SHELL: &str =
    "the desktop host is missing (sk_desktop::install did not run) — `appShell` is inert and every call rejects.";
const CAPTURE: &str =
    "the desktop host is missing (sk_desktop::install did not run) — `appCapture` has no sources.";
const LAUNCH: &str =
    "the desktop host is missing (sk_desktop::install did not run) — `appLaunch` is empty.";
const NO_HOTKEYS: &str =
    "global hotkeys are unavailable in this host: its platform has none, or it was \
     built without Skathi's `hotkeys` feature";
const NO_TRAY: &str = "native tray is unavailable in this host";

const PERMISSION_NAMES: [&str; 7] = [
    "microphone",
    "camera",
    "display-capture",
    "notifications",
    "clipboard-read",
    "clipboard-write",
    "geolocation",
];

macro_rules! inert {
    ($name:ident, $key:literal, $why:expr, |$g:ident, $args:ident| $body:block) => {
        crate::host_fn!(fn $name($g, $args) {
            unsafe { warn_compat($g, $key, $why) };
            $body
        });
    };
}

unsafe fn no_desktop(g: *mut c_void, what: &str) -> u64 {
    let e = host::error(g, "Error", &format!("{what}: no desktop host"));
    define(g, e, "name", js_string(g, "NotSupportedError"), 2);
    define(g, e, "code", js_string(g, "ENOSYS"), 2);
    e
}

inert!(
    permissions_platform,
    "appPermissions",
    PERMISSIONS,
    |g, _args| { unsafe { js_string(g, "this platform") } }
);
inert!(
    permissions_names,
    "appPermissions",
    PERMISSIONS,
    |g, _args| { unsafe { host::to_js(g, &json!(PERMISSION_NAMES)) } }
);
inert!(
    permissions_query,
    "appPermissions",
    PERMISSIONS,
    |g, _args| { unsafe { js_string(g, "unsupported") } }
);
inert!(
    permissions_list,
    "appPermissions",
    PERMISSIONS,
    |g, _args| {
        let rows: Vec<_> = PERMISSION_NAMES
            .iter()
            .map(|name| json!({ "name": name, "state": "unsupported", "canRequest": false }))
            .collect();
        unsafe { host::to_js(g, &json!(rows)) }
    }
);
inert!(
    permissions_request,
    "appPermissions",
    PERMISSIONS,
    |g, _args| { unsafe { host::resolved(g, js_string(g, "unsupported")) } }
);

inert!(storage_kinds, "appStorage", STORAGE, |g, _args| {
    unsafe { host::frozen_strings(g, &[] as &[&str]) }
});
inert!(storage_folder, "appStorage", STORAGE, |g, _args| {
    unsafe { js_string(g, "") }
});
inert!(storage_encrypted, "appStorage", STORAGE, |_g, _args| {
    FALSE
});
inert!(storage_info, "appStorage", STORAGE, |g, _args| {
    let info = json!({
        "folder": "", "file": "", "encrypted": false, "bytes": 0, "loaded": false, "pending": false,
        "provider": "", "kinds": [], "error": "no storage host", "indexedDB": null,
    });
    unsafe { host::to_js(g, &info) }
});
inert!(storage_flush, "appStorage", STORAGE, |_g, _args| { FALSE });
// A wrong path is worse than none: the app would write somewhere real.
inert!(
    storage_get_user_folder,
    "appStorage",
    STORAGE,
    |g, _args| {
        unsafe {
            host::throw(
                g,
                "TypeError",
                "appStorage.getUserFolder is unavailable: no storage host",
            )
        }
    }
);
inert!(storage_set_folder, "appStorage", STORAGE, |g, _args| {
    unsafe {
        host::throw(
            g,
            "TypeError",
            "appStorage.setStorageFolder is unavailable: no storage host",
        )
    }
});

macro_rules! shell_call {
    ($name:ident, $what:literal) => {
        inert!($name, "appShell", SHELL, |g, _args| {
            unsafe { host::rejected_with(g, no_desktop(g, $what)) }
        });
    };
}
shell_call!(shell_open_external, "appShell.openExternal");
shell_call!(shell_open_path, "appShell.openPath");
shell_call!(shell_show_item, "appShell.showItemInFolder");
shell_call!(shell_trash_item, "appShell.trashItem");
shell_call!(shell_open_content, "appShell.openContent");

inert!(capture_permission, "appCapture", CAPTURE, |g, _args| {
    let row = json!({ "state": "unsupported", "canRequest": false, "deniedBySystem": false });
    unsafe { host::to_js(g, &row) }
});
inert!(capture_request, "appCapture", CAPTURE, |g, _args| {
    unsafe { host::resolved(g, js_string(g, "unsupported")) }
});
inert!(capture_hint, "appCapture", CAPTURE, |g, _args| {
    unsafe { js_string(g, "") }
});
inert!(capture_sources, "appCapture", CAPTURE, |g, _args| {
    unsafe { host::rejected_with(g, no_desktop(g, "appCapture.getSources")) }
});
inert!(capture_pending, "appCapture", CAPTURE, |_g, _args| {
    FALSE
});

inert!(launch_list, "appLaunch", LAUNCH, |g, _args| {
    unsafe { host::frozen_strings(g, &[] as &[&str]) }
});
inert!(launch_cwd, "appLaunch", LAUNCH, |g, _args| {
    unsafe { js_string(g, "") }
});
inert!(launch_url, "appLaunch", LAUNCH, |_g, _args| { NULL });
inert!(launch_false, "appLaunch", LAUNCH, |_g, _args| { FALSE });
inert!(launch_register, "appLaunch", LAUNCH, |g, _args| {
    unsafe {
        host::throw(
            g,
            "TypeError",
            "appLaunch.registerProtocol is unavailable: no desktop host",
        )
    }
});

crate::host_fn!(fn hotkeys_set_bindings(g, args) {
    unsafe {
        let bindings = args.arg(0).0;
        if is_cell(bindings) && read_number(g, get(g, bindings, "length")) == 0.0 {
            return FALSE;
        }
        host::throw(g, "NotSupportedError", NO_HOTKEYS)
    }
});

crate::host_fn!(fn hotkeys_status(g, _args) {
    let status = json!({
        "running": false, "platform": "", "accessibilityGranted": false, "error": NO_HOTKEYS, "restarts": 0,
    });
    unsafe { host::to_js(g, &status) }
});

crate::host_fn!(fn hotkeys_restart(g, _args) {
    unsafe { host::throw(g, "NotSupportedError", NO_HOTKEYS) }
});

crate::host_fn!(fn hotkeys_capture(g, _args) {
    unsafe { host::rejected(g, "NotSupportedError", NO_HOTKEYS) }
});

crate::host_fn!(fn hotkeys_on_fired(g, _args) {
    unsafe { make_fn(g, "off", 0, nothing) }
});

crate::host_fn!(fn tray_set(g, args) {
    if args.arg(0).0 == NULL {
        return UNDEF;
    }
    unsafe { host::throw(g, "NotSupportedError", NO_TRAY) }
});

crate::host_fn!(fn nothing(_g, _args) {
    UNDEF
});

crate::host_fn!(fn answer_false(_g, _args) {
    FALSE
});

crate::host_fn!(fn haptic_patterns(g, _args) {
    unsafe { new_array(g) }
});

unsafe fn absent(g: *mut c_void, name: &str) -> bool {
    !is_cell(get(g, global(g), name))
}

/// Installs a stand-in for each app API object the host crates did not.
pub unsafe fn install_missing(g: *mut c_void) {
    let event_target = get(g, get(g, global(g), "EventTarget"), "prototype");
    if absent(g, "__SK_PERMISSIONS__") {
        let o = host::install_object(g, "__SK_PERMISSIONS__", event_target);
        host::getter(g, o, "platform", permissions_platform);
        host::getter(g, o, "names", permissions_names);
        host::method(g, o, "query", 1, permissions_query);
        host::method(g, o, "list", 0, permissions_list);
        host::method(g, o, "request", 1, permissions_request);
    }
    if absent(g, "__SK_STORAGE__") {
        let o = host::install_object(g, "__SK_STORAGE__", UNDEF);
        host::getter(g, o, "kinds", storage_kinds);
        host::getter(g, o, "folder", storage_folder);
        host::getter(g, o, "encrypted", storage_encrypted);
        host::method(g, o, "getUserFolder", 1, storage_get_user_folder);
        host::method(g, o, "setStorageFolder", 1, storage_set_folder);
        host::method(g, o, "info", 0, storage_info);
        host::method(g, o, "flush", 0, storage_flush);
    }
    if absent(g, "__SK_SHELL__") {
        let o = host::install_object(g, "__SK_SHELL__", UNDEF);
        host::method(g, o, "openExternal", 1, shell_open_external);
        host::method(g, o, "openPath", 1, shell_open_path);
        host::method(g, o, "showItemInFolder", 1, shell_show_item);
        host::method(g, o, "trashItem", 1, shell_trash_item);
        host::method(g, o, "openContent", 1, shell_open_content);
    }
    if absent(g, "__SK_CAPTURE__") {
        let o = host::install_object(g, "__SK_CAPTURE__", UNDEF);
        host::method(g, o, "permission", 1, capture_permission);
        host::method(g, o, "requestPermission", 1, capture_request);
        host::method(g, o, "permissionHint", 1, capture_hint);
        host::method(g, o, "getSources", 0, capture_sources);
        host::method(g, o, "setPendingSource", 1, capture_pending);
    }
    if absent(g, "__SK_LAUNCH__") {
        let o = host::install_object(g, "__SK_LAUNCH__", event_target);
        host::getter(g, o, "argv", launch_list);
        host::getter(g, o, "cwd", launch_cwd);
        host::getter(g, o, "url", launch_url);
        host::getter(g, o, "paths", launch_list);
        host::getter(g, o, "singleInstance", launch_false);
        host::method(g, o, "registerProtocol", 1, launch_register);
        host::method(g, o, "unregisterProtocol", 1, launch_false);
        host::method(g, o, "isProtocolRegistered", 1, launch_false);
    }
    if absent(g, "__sk_appHaptics") {
        let o = host::install_object(g, "__sk_appHaptics", UNDEF);
        define(g, o, "supported", FALSE, 1);
        host::method(g, o, "supportsPointer", 1, answer_false);
        host::method(g, o, "getSupportedPatterns", 0, haptic_patterns);
        host::method(g, o, "trigger", 2, answer_false);
        host::method(g, o, "perform", 1, answer_false);
        host::method(g, o, "stop", 1, answer_false);
    }
    if absent(g, "__sk_appHotkeys") {
        let o = host::install_object(g, "__sk_appHotkeys", UNDEF);
        define(g, o, "available", FALSE, 0);
        host::method(g, o, "setBindings", 1, hotkeys_set_bindings);
        host::method(g, o, "status", 0, hotkeys_status);
        host::method(g, o, "requestAccessibility", 0, answer_false);
        host::method(g, o, "restart", 0, hotkeys_restart);
        host::method(g, o, "captureOnce", 0, hotkeys_capture);
        host::method(g, o, "cancelCapture", 0, nothing);
        host::method(g, o, "onFired", 1, hotkeys_on_fired);
    }
    if absent(g, "__sk_appTray") {
        let o = host::install_object(g, "__sk_appTray", UNDEF);
        define(g, o, "available", FALSE, 0);
        host::method(g, o, "set", 1, tray_set);
    }
    if absent(g, "__sk_appDevtools") {
        crate::app_devtools::install(g, None);
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::on_js_thread;
    use crate::JsRuntime;

    fn eval(src: &'static str) -> String {
        on_js_thread(move || {
            JsRuntime::with(|rt| {
                let v = rt.ctx().eval(src).expect(src);
                rt.ctx().drain_microtasks();
                rt.ctx().to_rust_string(v).unwrap_or_default()
            })
        })
    }

    fn install() {
        on_js_thread(|| JsRuntime::with(|rt| unsafe { super::install_missing(rt.raw()) }));
    }

    #[test]
    fn missing_hosts_answer_without_doing_anything() {
        install();
        assert_eq!(
            eval(
                "(() => { const out = []; \
                 const err = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } }; \
                 const h = __sk_appHotkeys, t = __sk_appTray; \
                 out.push(h.available, h.setBindings([]), h.status().running, h.requestAccessibility(), \
                   typeof h.onFired(() => {}), err(() => h.setBindings([{ id: 'x' }])), err(() => h.restart()), \
                   t.available, err(() => t.set(null)), err(() => t.set({ tooltip: 'x' }))); \
                 const p = __SK_PERMISSIONS__, s = __SK_STORAGE__, l = __SK_LAUNCH__, c = __SK_CAPTURE__; \
                 out.push(p.query('camera'), p.list().length, p instanceof EventTarget, s.folder === '', \
                   s.info().error, err(() => s.setStorageFolder('C:/x')), l.url, Object.isFrozen(l.argv), \
                   err(() => l.registerProtocol('x')), c.permission('camera').state, c.setPendingSource('x')); \
                 globalThis.rejections = []; \
                 for (const p of [__SK_SHELL__.openExternal('https://x'), c.getSources(), h.captureOnce()]) \
                   p.catch((e) => rejections.push(e.name + ':' + (typeof e.code === 'string' ? e.code : ''))); \
                 return out.join('|'); })()"
            ),
            "false|false|false|false|function|NotSupportedError|NotSupportedError|false|none|NotSupportedError|\
             unsupported|7|true|true|no storage host|TypeError||true|TypeError|unsupported|false"
        );
        assert_eq!(
            eval("rejections.join(',')"),
            "NotSupportedError:ENOSYS,NotSupportedError:ENOSYS,NotSupportedError:"
        );
    }
}
