//! CSSOM View on mirror elements: the rect methods answer where the box was DRAWN (transforms
//! applied), the size accessors the layout box, both from the last painted frame; scrolling drives the
//! host's `ScrollHandle`. Anything that is not a live mirror reads as never painted.

use std::cell::Cell;
use std::ffi::c_void;

use sk_runtime::host::{self, global, warn_compat};
use sk_runtime::host_fn;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

use crate::native_dom::{accessor, method, node_id, proto};
use crate::perf::{self, Read};
use crate::scroll::{self, Align, Behavior, Info};

type Rect = (f32, f32, f32, f32);

thread_local! {
    static RECT_LIST: Cell<u64> = const { Cell::new(0) };
    static LIST_LENGTH: Cell<usize> = const { Cell::new(0) };
}

/// JS `Math.round`: halves go up, also below zero.
fn round(v: f32) -> f64 {
    (f64::from(v) + 0.5).floor()
}

/// The frame site that read the size, so the advisory names the line to move into `requestAnimationFrame`.
unsafe fn read_site(g: *mut c_void) -> String {
    let error = call(g, get(g, global(g), "Error"), UNDEF, &[]).unwrap_or(UNDEF);
    let stack = if is_cell(error) {
        read_string(g, get(g, error, "stack"))
    } else {
        None
    };
    stack
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.contains("[native code]"))
        .unwrap_or("")
        .to_string()
}

unsafe fn advise_unlaid(g: *mut c_void, kind: &str) {
    let site = read_site(g);
    let at = if site.is_empty() {
        String::new()
    } else {
        format!(" ({site})")
    };
    warn_compat(
        g,
        &format!("layout-stale:{kind}:{site}"),
        &format!(
            "{kind} answered 0 for an element that has not been laid out yet{at}: Skathi cannot lay out while the \
             platform is dispatching mouse or keyboard input, so a node mounted in this same event has no size; a \
             browser would lay out and answer it. Measure from requestAnimationFrame() or a ResizeObserver callback"
        ),
    );
}

unsafe fn note_stale(g: *mut c_void) {
    warn_compat(
        g,
        "scroll-layout",
        "scrollWidth/scrollHeight (and the clamp on a scroll) answered from the LAST PAINTED frame: the DOM changed \
         in this same wave, and a layout cannot be forced inside a keydown/keyup (or the click Enter/Space makes), \
         mousedown, wheel, clipboard or focus/blur handler here (it can everywhere else: timers, rAF, fetch \
         callbacks, mouseup/click, input). `el.scrollTop = el.scrollHeight` right after adding content from such a \
         handler therefore stops one item short — do it inside requestAnimationFrame(), which runs after the frame \
         that laid the new content out",
    );
}

/// The layout box, or the drawn one with `painted`; `None` when never painted.
unsafe fn rect(g: *mut c_void, this: u64, kind: &str, painted: bool) -> Option<Rect> {
    let id = node_id(g, this)?;
    perf::note_read(Read::Layout);
    match crate::dom::bounds(id) {
        (Some(r), _) => Some(if painted {
            crate::dom::transformed(id).unwrap_or(r)
        } else {
            r
        }),
        (None, stale) => {
            if stale {
                advise_unlaid(g, kind);
            }
            None
        }
    }
}

unsafe fn dom_rect(g: *mut c_void, (x, y, w, h): Rect) -> u64 {
    let args = [x, y, w, h].map(|v| enc_f64(f64::from(v)));
    let mut out = UNDEF;
    let ctor = get(g, global(g), "DOMRect");
    if glue::rjs_construct(g, ctor, args.as_ptr(), 4, &mut out) != 0 {
        return UNDEF;
    }
    out
}

host_fn!(fn bounding_client_rect(g, args) {
    unsafe {
        let r = rect(g, args.this().0, "getBoundingClientRect()", true).unwrap_or_default();
        dom_rect(g, r)
    }
});

host_fn!(fn client_rects(g, args) {
    unsafe {
        let list = new_object(g, RECT_LIST.get());
        let r = rect(g, args.this().0, "getClientRects()", true).filter(|r| r.2 != 0.0 || r.3 != 0.0);
        let mut len = 0;
        if let Some(r) = r {
            set_i(g, list, 0, dom_rect(g, r));
            len = 1;
        }
        glue::rjs_define(g, list, LIST_LENGTH.get() as *mut c_void, enc_f64(f64::from(len)), 1 | 2 | 4);
        list
    }
});

unsafe fn list_length(g: *mut c_void, this: u64) -> Option<f64> {
    if !is_cell(this) || !proto_chain_has(g, this, RECT_LIST.get()) {
        return None;
    }
    dec_f64(get_a(g, this, LIST_LENGTH.get() as *mut c_void))
}

host_fn!(fn list_item(g, args) {
    unsafe {
        let Some(len) = list_length(g, args.this().0) else {
            return throw_type_error(g, "Illegal invocation");
        };
        if args.argc() == 0 {
            return throw_type_error(g, "Failed to execute 'item' on 'DOMRectList': 1 argument required, but only 0 present.");
        }
        let i = to_number(g, args.arg(0).0).unwrap_or(0.0);
        let i = if i.is_finite() { (i.trunc() as i64).rem_euclid(1 << 32) as f64 } else { 0.0 };
        if i < len { get_i(g, args.this().0, i as u32) } else { NULL }
    }
});

host_fn!(fn list_len(g, args) {
    match unsafe { list_length(g, args.this().0) } {
        Some(n) => enc_f64(n),
        None => unsafe { throw_type_error(g, "Illegal invocation") },
    }
});

macro_rules! layout_size {
    ($name:ident, $kind:literal, $pick:expr) => {
        host_fn!(fn $name(g, args) {
            let r = unsafe { rect(g, args.this().0, $kind, false) }.unwrap_or_default();
            enc_f64(round($pick(r)))
        });
    };
}
layout_size!(offset_width, "offsetWidth", |r: Rect| r.2);
layout_size!(offset_height, "offsetHeight", |r: Rect| r.3);
layout_size!(client_width, "clientWidth", |r: Rect| r.2);
layout_size!(client_height, "clientHeight", |r: Rect| r.3);

macro_rules! window_offset {
    ($name:ident, $kind:literal, $pick:expr) => {
        host_fn!(fn $name(g, args) {
            unsafe {
                warn_compat(
                    g,
                    "offsetParent",
                    "offsetTop/offsetLeft are measured from the WINDOW, not from an offsetParent — Skathi has no \
                     positioned-ancestor chain; use getBoundingClientRect(), which is exact",
                );
                let r = rect(g, args.this().0, $kind, false).unwrap_or_default();
                enc_f64(round($pick(r)))
            }
        });
    };
}
window_offset!(offset_top, "offsetTop", |r: Rect| r.1);
window_offset!(offset_left, "offsetLeft", |r: Rect| r.0);

host_fn!(fn zero(_g, _args) {
    enc_f64(0.0)
});

host_fn!(fn no_offset_parent(_g, _args) {
    NULL
});

/// The scroll container `this` mirrors, read or moved; `None` for anything else.
unsafe fn scrolled(
    g: *mut c_void,
    this: u64,
    op: impl FnOnce(u32) -> Option<Info>,
    size_matters: bool,
) -> Option<Info> {
    let id = node_id(g, this)?;
    perf::note_read(Read::Scroll);
    let info = op(id)?;
    if info.stale && size_matters {
        note_stale(g);
    }
    Some(info)
}

macro_rules! content_size {
    ($name:ident, $kind:literal, $content:expr, $layout:expr) => {
        host_fn!(fn $name(g, args) {
            unsafe {
                let this = args.this().0;
                let v = match scrolled(g, this, scroll::info, true) {
                    Some(info) => $content(info),
                    None => $layout(rect(g, this, $kind, false).unwrap_or_default()),
                };
                enc_f64(round(v))
            }
        });
    };
}
content_size!(
    scroll_width,
    "scrollWidth",
    |i: Info| i.scroll_width,
    |r: Rect| r.2
);
content_size!(
    scroll_height,
    "scrollHeight",
    |i: Info| i.scroll_height,
    |r: Rect| r.3
);

host_fn!(fn scroll_left_get(g, args) {
    let info = unsafe { scrolled(g, args.this().0, scroll::info, false) };
    enc_f64(info.map_or(0.0, |i| f64::from(i.left)))
});

host_fn!(fn scroll_top_get(g, args) {
    let info = unsafe { scrolled(g, args.this().0, scroll::info, false) };
    enc_f64(info.map_or(0.0, |i| f64::from(i.top)))
});

unsafe fn number(g: *mut c_void, v: u64) -> Option<f64> {
    to_number(g, v).ok()
}

host_fn!(fn scroll_left_set(g, args) {
    unsafe {
        if let Some(v) = number(g, args.arg(0).0).filter(|v| v.is_finite()) {
            scrolled(g, args.this().0, |id| scroll::set(id, Some(v as f32), None, Behavior::Auto), true);
        }
    }
    UNDEF
});

host_fn!(fn scroll_top_set(g, args) {
    unsafe {
        if let Some(v) = number(g, args.arg(0).0).filter(|v| v.is_finite()) {
            scrolled(g, args.this().0, |id| scroll::set(id, None, Some(v as f32), Behavior::Auto), true);
        }
    }
    UNDEF
});

/// `"auto"` (and anything else) leaves it to the scrolled box's CSS `scroll-behavior`.
unsafe fn behavior(g: *mut c_void, v: u64) -> Behavior {
    if !host::is_string(g, v) {
        return Behavior::Auto;
    }
    match read_string(g, v).as_deref() {
        Some("smooth") => Behavior::Smooth,
        Some("instant") => Behavior::Instant,
        _ => Behavior::Auto,
    }
}

/// `scrollTo(x, y)` or `scrollTo({ left, top, behavior })`; an absent or non-numeric axis is NaN.
unsafe fn scroll_args(g: *mut c_void, a: u64, b: u64) -> Option<(f64, f64, Behavior)> {
    let axis = |v: u64| -> Option<f64> {
        if v == UNDEF {
            Some(f64::NAN)
        } else {
            number(g, v)
        }
    };
    if is_object(g, a) {
        let left = axis(get(g, a, "left"))?;
        let top = axis(get(g, a, "top"))?;
        return Some((left, top, behavior(g, get(g, a, "behavior"))));
    }
    Some((axis(a)?, axis(b)?, Behavior::Auto))
}

/// `typeof v === "object"` and not null.
unsafe fn is_object(g: *mut c_void, v: u64) -> bool {
    const OBJECT: u32 = 8;
    const FUNCTION: u32 = 9;
    let kind = glue::rjs_kind(g, v);
    kind >= OBJECT && kind != FUNCTION
}

host_fn!(fn scroll_to(g, args) {
    unsafe {
        if let Some((left, top, how)) = scroll_args(g, args.arg(0).0, args.arg(1).0) {
            let axis = |v: f64| (!v.is_nan()).then_some(v as f32);
            scrolled(g, args.this().0, |id| scroll::set(id, axis(left), axis(top), how), true);
        }
    }
    UNDEF
});

host_fn!(fn scroll_by(g, args) {
    unsafe {
        if let Some((left, top, how)) = scroll_args(g, args.arg(0).0, args.arg(1).0) {
            let axis = |v: f64| if v.is_finite() { v as f32 } else { 0.0 };
            scrolled(g, args.this().0, |id| scroll::by(id, axis(left), axis(top), how), true);
        }
    }
    UNDEF
});

unsafe fn align(g: *mut c_void, v: u64, default: Align) -> Align {
    if v == UNDEF {
        return default;
    }
    match host::string_arg(g, v).as_deref() {
        Some("start") => Align::Start,
        Some("center") => Align::Center,
        Some("end") => Align::End,
        Some("nearest") => Align::Nearest,
        _ => Align::Start,
    }
}

host_fn!(fn scroll_into_view(g, args) {
    unsafe {
        let Some(id) = node_id(g, args.this().0) else {
            return UNDEF;
        };
        let arg = args.arg(0).0;
        let (mut block, mut inline, mut how) = (Align::Start, Align::Nearest, Behavior::Auto);
        if arg == FALSE {
            block = Align::End;
        } else if is_object(g, arg) {
            how = behavior(g, get(g, arg, "behavior"));
            block = align(g, get(g, arg, "block"), Align::Start);
            inline = align(g, get(g, arg, "inline"), Align::Nearest);
        }
        // Only horizontal-tb ltr exists: block is vertical, inline horizontal.
        match scroll::into_view(id, block, inline, how) {
            Ok(false) => {}
            Ok(true) => note_stale(g),
            Err(e) => {
                warn_compat(g, "scrollIntoView", &e);
            }
        }
    }
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    unsafe {
        let g = rt.raw();
        let list = define_class(
            g,
            &ClassSpec {
                name: "DOMRectList",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[("item", 1, list_item)],
                getters: &[("length", list_len)],
                accessors: &[],
            },
        );
        let iterator = glue::rjs_atom_wellknown(g, 0);
        if !iterator.is_null() {
            let values = get(g, proto(g, "Array"), "values");
            glue::rjs_define(g, list.proto, iterator, values, 2);
            glue::rjs_atom_free(g, iterator);
        }
        RECT_LIST.set(list.proto);
        LIST_LENGTH.set(atom_priv(g, "sk#rectListLength") as usize);

        let el = proto(g, "HTMLElement");
        if !is_cell(el) {
            return;
        }
        method(g, el, "getBoundingClientRect", 0, bounding_client_rect);
        method(g, el, "getClientRects", 0, client_rects);
        for (name, f) in [
            ("offsetWidth", offset_width as HostFn),
            ("offsetHeight", offset_height),
            ("clientWidth", client_width),
            ("clientHeight", client_height),
            ("scrollWidth", scroll_width),
            ("scrollHeight", scroll_height),
            ("offsetTop", offset_top),
            ("offsetLeft", offset_left),
            ("offsetParent", no_offset_parent),
            // Border widths are not published (the painted rect is the border box); @floating-ui adds these to
            // every clipping rect, so they must be numbers.
            ("clientTop", zero),
            ("clientLeft", zero),
        ] {
            accessor(g, el, name, f, None);
        }
        accessor(g, el, "scrollLeft", scroll_left_get, Some(scroll_left_set));
        accessor(g, el, "scrollTop", scroll_top_get, Some(scroll_top_set));
        method(g, el, "scrollTo", 0, scroll_to);
        method(g, el, "scroll", 0, scroll_to);
        method(g, el, "scrollBy", 0, scroll_by);
        method(g, el, "scrollIntoView", 0, scroll_into_view);
    }
}

#[cfg(test)]
mod tests {
    use sk_runtime::JsRuntime;

    use crate::{tree, windows};

    const WINDOW: windows::WinKey = 0x6e0;

    fn js(src: &str) -> String {
        JsRuntime::with(|rt| {
            let v = rt.ctx().eval(src).unwrap_or_else(|_| {
                let why = rt
                    .ctx()
                    .eval(&format!(
                        "(() => {{ try {{ {src}; }} catch (e) {{ return String(e); }} }})()"
                    ))
                    .unwrap();
                panic!(
                    "{src}: {}",
                    rt.ctx().to_rust_string(why).unwrap_or_default()
                )
            });
            rt.ctx().to_rust_string(v).unwrap()
        })
    }

    fn painted(layout: windows::Rect, drawn: Option<windows::Rect>) -> u32 {
        let el = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let el = t.create_element(div);
            t.insert(el, root, 0);
            el
        });
        let root = tree::read_tree(|t| t.root);
        windows::of(WINDOW, |w| {
            w.bounds.insert(root, (0.0, 0.0, 800.0, 600.0));
            w.bounds.insert(el, layout);
            if let Some(r) = drawn {
                w.transformed.insert(el, r);
            }
        });
        el
    }

    fn mirror(name: &str, id: u32) {
        js(&format!(
            "globalThis.{name} = Object.create(HTMLElement.prototype); {name}.__skId = {id}; ''"
        ));
    }

    #[test]
    fn rect_methods_answer_the_drawn_box_and_sizes_the_layout_one() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(super::install);
            mirror("plain", painted((10.0, 20.0, 100.5, 40.0), None));
            mirror(
                "moved",
                painted((0.0, 0.0, 100.0, 40.0), Some((290.0, 195.0, 120.0, 50.0))),
            );
            js("globalThis.warned = []; console.warn = (m) => warned.push(m); ''");
            assert_eq!(
                js("(() => { const r = plain.getBoundingClientRect(); \
                    return [r instanceof DOMRect, r.x, r.y, r.right, r.bottom, plain.offsetWidth, plain.clientHeight, \
                      plain.clientTop, plain.offsetParent, plain.scrollTop, plain.scrollWidth].join('|'); })()"),
                "true|10|20|110.5|60|101|40|0||0|101"
            );
            assert_eq!(
                js("(() => { const r = moved.getBoundingClientRect(), list = moved.getClientRects(); \
                    return [r.x, r.y, r.width, r.bottom, moved.offsetWidth, moved.clientHeight, list instanceof DOMRectList, \
                      list.length, list.item(0).left, list[0].top, [...list].length, list.item(1)].join('|'); })()"),
                "290|195|120|245|100|40|true|1|290|195|1|"
            );
            assert_eq!(js("[plain.offsetTop, plain.offsetLeft, moved.offsetLeft, warned.length].join('|')"), "20|10|0|1");
        });
    }

    #[test]
    fn what_is_not_a_painted_mirror_reads_zero() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(super::install);
            mirror("detached", 0);
            let fresh = tree::with_tree(|t| {
                let span = t.intern("span");
                t.create_element(span)
            });
            mirror("unlaid", fresh);
            js("globalThis.warned = []; console.warn = (m) => warned.push(m); ''");
            assert_eq!(
                js("(() => { const r = detached.getBoundingClientRect(), list = detached.getClientRects(); \
                    detached.scrollTop = 50; detached.scrollTo({ top: 5, behavior: 'smooth' }); detached.scrollIntoView(false); \
                    return [r.width, r.height, list.length, detached.offsetHeight, detached.scrollTop, warned.length].join('|'); })()"),
                "0|0|0|0|0|0"
            );
            assert_eq!(
                js("(() => { const read = () => unlaid.offsetWidth; return [read(), read(), warned.length].join('|'); })()"),
                "0|0|1"
            );
            assert!(js("warned[0]").contains("offsetWidth answered 0"));
            assert_eq!(
                js("(() => { try { DOMRectList.prototype.item.call({}, 0); } catch (e) { return e.name; } })()"),
                "TypeError"
            );
        });
    }
}
