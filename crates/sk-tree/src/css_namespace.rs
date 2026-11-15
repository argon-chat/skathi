//! The `CSS` namespace: `escape`, `supports` (answered by sk-css's catalog, as `@supports` is) and
//! `registerProperty`.

use std::ffi::c_void;

use sk_runtime::host::{global, method, string_arg, warn_compat};
use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

/// CSSOM's serialize-an-identifier: it feeds string comparisons, so an under-escaped identifier would
/// silently stop matching.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let first_is_dash = s.starts_with('-');
    let only_dash = s == "-";
    for (i, c) in s.chars().enumerate() {
        let code = c as u32;
        match c {
            '\0' => out.push('\u{FFFD}'),
            _ if (0x1..=0x1f).contains(&code)
                || code == 0x7f
                || (i == 0 && c.is_ascii_digit())
                || (i == 1 && c.is_ascii_digit() && first_is_dash) =>
            {
                out.push_str(&format!("\\{code:x} "));
            }
            '-' if i == 0 && only_dash => out.push_str("\\-"),
            _ if code >= 0x80 || c == '-' || c == '_' || c.is_ascii_alphanumeric() => out.push(c),
            _ => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

unsafe fn string_of(g: *mut c_void, v: u64) -> String {
    to_js_string(g, v)
        .ok()
        .and_then(|s| read_string(g, s))
        .unwrap_or_default()
}

host_fn!(fn css_escape(g, args) {
    if args.argc() == 0 {
        return unsafe {
            throw_type_error(g, "Failed to execute 'escape' on 'CSS': 1 argument required, but only 0 present.")
        };
    }
    let s = unsafe { string_of(g, args.arg(0).0) };
    unsafe { js_string(g, &escape(&s)) }
});

host_fn!(fn css_supports(g, args) {
    let answer = match args.argc() {
        0 => {
            return unsafe {
                throw_type_error(g, "Failed to execute 'supports' on 'CSS': 1 argument required, but only 0 present.")
            }
        }
        1 => sk_css::supports_condition_text(&unsafe { string_of(g, args.arg(0).0) }),
        _ => {
            let (name, value) = unsafe { (string_of(g, args.arg(0).0), string_of(g, args.arg(1).0)) };
            sk_css::catalog::supports(&name, &value)
        }
    };
    enc_bool(answer)
});

host_fn!(fn css_register_property(g, args) {
    let options = args.arg(0).0;
    let name = if is_cell(options) { unsafe { string_arg(g, get(g, options, "name")) } } else { None };
    let shown = name.map_or_else(String::new, |n| format!("(\"{n}\")"));
    unsafe {
        warn_compat(
            g,
            "CSS.registerProperty",
            &format!(
                "CSS.registerProperty{shown} does nothing here: custom properties are untyped strings, so a \
                 registered syntax, inherits or initialValue has nothing to enforce. Write the initial value \
                 where the property is used (`var(--x, 0)`)"
            ),
        )
    };
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    unsafe {
        let g = rt.raw();
        let css = new_object(g, UNDEF);
        method(g, css, "escape", 1, css_escape);
        method(g, css, "supports", 1, css_supports);
        method(g, css, "registerProperty", 1, css_register_property);
        define(g, global(g), "CSS", css, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js(src: &str) -> String {
        JsRuntime::with(|rt| {
            let v = rt
                .ctx()
                .eval(src)
                .unwrap_or_else(|e| panic!("{src}: {e:?}"));
            rt.ctx().to_rust_string(v).unwrap()
        })
    }

    #[test]
    fn escape_is_cssom_serialize_an_identifier() {
        assert_eq!(escape("a b"), "a\\ b");
        assert_eq!(escape("1a"), "\\31 a");
        assert_eq!(escape("-1a"), "-\\31 a");
        assert_eq!(escape("-"), "\\-");
        assert_eq!(escape("--x"), "--x");
        assert_eq!(escape("\0"), "\u{FFFD}");
        assert_eq!(escape("\u{1}\u{7f}"), "\\1 \\7f ");
        assert_eq!(escape("r:é_δ"), "r\\:é_δ");
        assert_eq!(escape("radix-:r1:"), "radix-\\:r1\\:");
    }

    #[test]
    fn the_namespace_answers_from_sk_css() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(install);
            assert_eq!(
                js("[CSS.supports('order', '2'), CSS.supports('object-position', 'center'), \
                    CSS.supports('scroll-behavior: smooth'), CSS.supports('(filter: blur(2px)) and (contain: paint)'), \
                    CSS.supports('animation-timing-function', 'linear(0, 1)'), CSS.supports('width', 'fit-content'), \
                    CSS.supports('float: left'), CSS.supports('transition-timing-function: LINEAR(0, 0.5 50%, 1)'), \
                    CSS.supports('transition-timing-function', 'linear'), CSS.escape(1)].join()"),
                "true,true,true,true,false,false,false,false,true,\\31 "
            );
            assert_eq!(
                js("(() => { try { CSS.supports(); } catch (e) { return e.name; } })()"),
                "TypeError"
            );
        });
    }
}
