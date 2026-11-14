//! `navigator.setAppBadge` / `clearAppBadge`.

use std::ffi::c_void;

use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;

use super::rejected;

const FLAG: i64 = -1;

fn show(contents: i64) {
    crate::badge::set(crate::dialog::current_owner().map(|o| o.hwnd), contents);
}

host_fn!(fn set_app_badge(g, args) {
    unsafe {
        let v = args.arg(0).0;
        let contents = if v == UNDEF {
            FLAG
        } else {
            let Ok(n) = to_number(g, v) else { return rejected(g, "TypeError", "Failed to execute 'setAppBadge' on 'Navigator': Value is not of type 'unsigned long long'.") };
            let t = n.trunc();
            if !n.is_finite() || t < 0.0 || t > 9_007_199_254_740_991.0 {
                return rejected(g, "TypeError", "Failed to execute 'setAppBadge' on 'Navigator': Value is outside the 'unsigned long long' value range.");
            }
            t as i64
        };
        show(contents);
        resolved_promise(g, UNDEF)
    }
});

host_fn!(fn clear_app_badge(g, _args) {
    show(0);
    unsafe { resolved_promise(g, UNDEF) }
});

pub(super) unsafe fn install(g: *mut c_void, nav: u64) {
    define(
        g,
        nav,
        "setAppBadge",
        make_fn(g, "setAppBadge", 0, set_app_badge),
        2,
    );
    define(
        g,
        nav,
        "clearAppBadge",
        make_fn(g, "clearAppBadge", 0, clear_app_badge),
        2,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_resolve_and_bad_ones_reject() {
        let out = super::super::testing::run(
            "await navigator.setAppBadge(); await navigator.setAppBadge(3.7); await navigator.clearAppBadge(); \
             const bad = []; for (const v of [-1, NaN, Infinity]) { try { await navigator.setAppBadge(v); } catch (e) { bad.push(e instanceof TypeError); } } \
             return bad.join() + '|' + typeof Navigator.prototype.setAppBadge;",
        );
        assert_eq!(out, "true,true,true|function");
    }
}
