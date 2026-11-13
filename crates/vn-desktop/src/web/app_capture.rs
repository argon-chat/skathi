//! `appCapture`: desktop capture sources for `getDisplayMedia`, and the capture permissions.

use std::ffi::c_void;

use vn_permissions::Cap;
use vn_runtime::host;
use vn_runtime::host_fn;
use vn_runtime::redjs::webapi::util::*;

use super::{awaiting_with, Errors};

unsafe fn capability(g: *mut c_void, v: u64) -> Result<Cap, u64> {
    match host::string_arg(g, v).as_deref() {
        Some("microphone") => Ok(Cap::Microphone),
        Some("camera") => Ok(Cap::Camera),
        Some("screen_share") => Ok(Cap::DisplayCapture),
        _ => Err(host::error(g, "TypeError", "unknown capture source")),
    }
}

host_fn!(fn permission(g, args) {
    unsafe {
        let cap = match capability(g, args.arg(0).0) {
            Ok(c) => c,
            Err(e) => {
                vn_runtime::redjs::glue::rjs_throw(g, e);
                return UNDEF;
            }
        };
        let (state, can_request, by_system) = vn_permissions::describe(cap);
        let o = new_object(g, UNDEF);
        define(g, o, "state", js_string(g, state.as_str()), 0);
        define(g, o, "canRequest", enc_bool(can_request), 0);
        define(g, o, "deniedBySystem", enc_bool(by_system), 0);
        o
    }
});

host_fn!(fn request_permission(g, args) {
    unsafe {
        match capability(g, args.arg(0).0) {
            Ok(cap) => vn_permissions::request_promise(g, cap.web_name()),
            Err(e) => host::rejected_with(g, e),
        }
    }
});

host_fn!(fn permission_hint(g, args) {
    unsafe {
        match capability(g, args.arg(0).0) {
            Ok(cap) => js_string(g, &vn_permissions::refusal_hint(cap).unwrap_or_default()),
            Err(e) => {
                vn_runtime::redjs::glue::rjs_throw(g, e);
                UNDEF
            }
        }
    }
});

unsafe fn sources(g: *mut c_void, _token: u32, v: &serde_json::Value, _made: u64) -> Result<u64, u64> {
    if !v.is_array() {
        return Err(host::error(g, "TypeError", "the desktop capture host returned an invalid source list"));
    }
    Ok(host::to_js(g, v))
}

host_fn!(fn get_sources(g, args) {
    unsafe {
        let types = match args.arg(0).0 {
            UNDEF => r#"["screen","window"]"#.to_string(),
            v => host::stringify(g, v).unwrap_or_default(),
        };
        let options = match args.arg(1).0 {
            UNDEF => "{}".to_string(),
            v => host::stringify(g, v).unwrap_or_default(),
        };
        awaiting_with(g, crate::capture::start_sources(&types, &options), sources, Errors::Desktop)
    }
});

host_fn!(fn set_pending_source(g, args) {
    unsafe {
        let id = args.arg(0).0;
        let valid = read_string(g, id).filter(|s| {
            let (kind, n) = s.split_once(':').unwrap_or_default();
            matches!(kind, "screen" | "window") && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
        });
        if host::is_string(g, id) && valid.is_some() {
            let pending = new_object(g, UNDEF);
            define(g, pending, "sourceId", id, 0);
            define(g, pending, "includeAudio", enc_bool(host::truthy(g, args.arg(1).0)), 0);
            define(g, host::global(g), "__VN_SCREEN_SOURCE__", pending, 0);
            return TRUE;
        }
        host::throw(g, "TypeError", "capture source must be a screen or window ID from getSources()")
    }
});

pub(super) unsafe fn install(g: *mut c_void) {
    let obj = host::install_object(g, "__VN_CAPTURE__", UNDEF);
    for (name, arity, f) in [
        ("permission", 1, permission as HostFn),
        ("requestPermission", 1, request_permission),
        ("permissionHint", 1, permission_hint),
        ("getSources", 0, get_sources),
        ("setPendingSource", 1, set_pending_source),
    ] {
        define(g, obj, name, make_fn(g, name, arity, f), 0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sources_and_permissions_check_their_arguments() {
        let out = super::super::testing::run(
            "const c = __VN_CAPTURE__; const r = []; \
             const p = c.permission('microphone'); r.push(typeof p.state, typeof p.canRequest, typeof p.deniedBySystem); \
             try { c.permission('nose'); } catch (e) { r.push(e instanceof TypeError); } \
             try { await c.getSources(['tv']); } catch (e) { r.push(e instanceof TypeError); } \
             r.push(JSON.stringify(await c.getSources([]))); \
             try { c.setPendingSource('screen:x'); } catch (e) { r.push(e instanceof TypeError); } \
             r.push(c.setPendingSource('window:42', 1), JSON.stringify(__VN_SCREEN_SOURCE__)); \
             r.push(typeof c.permissionHint('camera')); \
             return r.join('|');",
        );
        assert_eq!(out, "string|boolean|boolean|true|true|[]|true|true|{\"sourceId\":\"window:42\",\"includeAudio\":true}|string");
    }
}
