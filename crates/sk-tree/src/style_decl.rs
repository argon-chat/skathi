//! `CSSStyleDeclaration` over the tree: `el.style` reads and writes the element's inline declarations
//! ([`crate::inline_style`]), `getComputedStyle(el)` reads its computed values ([`crate::dom`]). Each
//! object holds its mirror node and reads the node's id on every access, so a freed node answers like a
//! disconnected one.
//!
//! Named properties (`style.backgroundColor`, `style["font-size"]`, `style[0]`) answer any CSS-shaped name,
//! as a browser does for the properties it knows; other names, and the prototype's own members, are
//! ordinary properties.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use sk_runtime::host_fn;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::domexception;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

use crate::inline_style::{css_name, cssom_name, stored_value, strip_important};
use crate::{dom, perf, tree};

struct Decl {
    computed: bool,
}

#[derive(Default)]
struct Classes {
    cls: usize,
    proto: u64,
    node: usize,
}

thread_local! {
    static C: RefCell<Classes> = RefCell::new(Classes::default());
    /// `(id, oldValue, present)` after an inline change the runtime must hear of.
    static HOOK: Cell<(usize, u64)> = const { Cell::new((0, UNDEF)) };
    /// A MutationObserver is registered, so every change reports its old attribute value.
    static OBSERVING: Cell<bool> = const { Cell::new(false) };
}

const MEMBERS: &[&str] = &[
    "cssText",
    "cssFloat",
    "length",
    "parentRule",
    "item",
    "getPropertyValue",
    "getPropertyPriority",
    "setProperty",
    "removeProperty",
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "then",
];

/// A name a CSS property could have, camel-cased or dashed, or a custom property.
fn css_shaped(name: &str) -> bool {
    if name.starts_with("--") {
        return name.len() > 2;
    }
    let body = name.strip_prefix('-').unwrap_or(name);
    body.starts_with(|c: char| c.is_ascii_alphabetic())
        && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !MEMBERS.contains(&name)
}

fn index(name: &str) -> Option<usize> {
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        .then(|| name.parse().ok())
        .flatten()
}

unsafe fn decl<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Decl> {
    unsafe { host_data::<Decl>(g, C.with(|c| c.borrow().cls) as JSClassRef, v) }
}

/// The node's current id: `None` once it is freed (or for a computed style of something that is not a node,
/// the root's).
unsafe fn node_id(g: *mut c_void, this: u64, computed: bool) -> Option<u32> {
    unsafe {
        let node = named_slot_get(g, this, C.with(|c| c.borrow().node) as *mut c_void);
        let id = if is_cell(node) {
            dec_f64(get(g, node, "__skId"))
        } else {
            None
        };
        match id {
            Some(id) if id > 0.0 => Some(id as u32),
            Some(_) => None,
            None if computed => Some(tree::read_tree(|t| t.root)).filter(|&r| r != 0),
            None => None,
        }
    }
}

unsafe fn create(g: *mut c_void, node: u64, computed: bool) -> u64 {
    unsafe {
        let (cls, proto, slot) = C.with(|c| {
            let c = c.borrow();
            (c.cls, c.proto, c.node)
        });
        let obj = host_create(g, cls as JSClassRef, proto, Decl { computed });
        named_slot_define(g, obj, slot as *mut c_void, node);
        obj
    }
}

unsafe fn computed_read(g: *mut c_void, id: Option<u32>, css: &str, shown: &str) -> String {
    unsafe {
        let Some(id) = id else { return String::new() };
        perf::note_read(perf::Read::Style);
        if let Some(v) = dom::computed_value(id, css) {
            return v;
        }
        if !css.starts_with("--") && tree::read_tree(|t| t.node(id).is_some()) {
            sk_runtime::host::warn_compat(
            g,
            &format!("getComputedStyle:{css}"),
            &format!(
                "getComputedStyle().{shown} reads \"\" — Skathi's computed style publishes the properties a reused \
                 component actually reads (transition-*, animation-*, transform, display, position, overflow-*, \
                 width/height, padding-*, margin-*, border-*-width, direction, font-size, color) plus every custom \
                 property, not all 340 longhands"
            ),
        );
        }
        String::new()
    }
}

fn inline_read(id: Option<u32>, css: &str) -> (String, bool) {
    id.and_then(|id| tree::read_tree(|t| t.style_value(id, css).map(|(v, i)| (v.to_string(), i))))
        .unwrap_or_default()
}

/// Runs an inline change and tells the runtime when the attribute appeared, went, or is observed.
unsafe fn mutate(g: *mut c_void, id: u32, change: impl FnOnce(&mut tree::Tree) -> bool) -> bool {
    unsafe {
        let observing = OBSERVING.get();
        let (before, was) = tree::read_tree(|t| {
            (
                observing.then(|| t.style_attr(id)).flatten(),
                t.node(id).is_some_and(|n| n.style_present),
            )
        });
        perf::note_dom(perf::DomOp::Style);
        if !tree::with_tree(change) {
            return false;
        }
        let now = tree::read_tree(|t| t.node(id).is_some_and(|n| n.style_present));
        let hook = match HOOK.get() {
            (ctx, f) if ctx == g as usize => f,
            _ => UNDEF,
        };
        if (observing || now != was) && is_cell(hook) {
            let old = before.map_or(NULL, |s| js_string(g, &s));
            let _ = call(g, hook, UNDEF, &[enc_f64(id as f64), old, enc_bool(now)]);
        }
        true
    }
}

/// `style.x = v`: `null` removes; `undefined` and a value carrying `!important` are ignored.
unsafe fn assign(g: *mut c_void, id: u32, key: &str, v: u64) {
    unsafe {
        if v == UNDEF {
            return;
        }
        let name = css_name(key);
        let custom = name.starts_with("--");
        if custom {
            sk_runtime::host::warn_compat(
            g,
            "style-custom-assign",
            &format!(
                "el.style[\"{name}\"] = … sets the custom property here, but in a browser it is only an expando on the \
                 declaration object and changes nothing; use el.style.setProperty(\"{name}\", …), which works in both"
            ),
        );
        }
        let value = if v == NULL {
            String::new()
        } else {
            string_of(g, v)
        };
        if value.is_empty() {
            mutate(g, id, |t| t.style_remove(id, &name).is_some());
        } else if custom || strip_important(&value).is_none() {
            let stored = stored_value(&value, custom);
            mutate(g, id, |t| t.style_write(id, &name, &stored, false));
        }
    }
}

unsafe fn string_of(g: *mut c_void, v: u64) -> String {
    unsafe {
        to_js_string(g, v)
            .ok()
            .and_then(|s| read_string(g, s))
            .unwrap_or_default()
    }
}

unsafe fn read_only(g: *mut c_void, what: &str) -> u64 {
    unsafe {
        domexception::create(
        g,
        "NoModificationAllowedError",
        &format!(
            "Failed to set the '{what}' property on 'CSSStyleDeclaration': These styles are computed, and therefore \
             the '{what}' property is read-only."
        ),
    )
    }
}

impl NamedProps for Decl {
    unsafe fn get(&mut self, g: *mut c_void, this: u64, name: &str) -> Result<Option<u64>, u64> {
        unsafe {
            let id = node_id(g, this, self.computed);
            if let Some(i) = index(name) {
                return Ok(item(id, self.computed, i).map(|n| js_string(g, &n)));
            }
            if !css_shaped(name) {
                return Ok(None);
            }
            let css = css_name(name);
            let value = if self.computed {
                computed_read(g, id, &css, name)
            } else {
                inline_read(id, &css).0
            };
            Ok(Some(js_string(g, &value)))
        }
    }

    unsafe fn set(
        &mut self,
        g: *mut c_void,
        this: u64,
        name: &str,
        value: u64,
    ) -> Result<bool, u64> {
        unsafe {
            if index(name).is_some() {
                return Ok(true);
            }
            if !css_shaped(name) {
                return Ok(false);
            }
            if self.computed {
                return Err(read_only(g, name));
            }
            if let Some(id) = node_id(g, this, false) {
                assign(g, id, name, value);
            }
            Ok(true)
        }
    }

    unsafe fn names(&mut self, g: *mut c_void, this: u64) -> Vec<String> {
        unsafe {
            (0..len(node_id(g, this, self.computed), self.computed))
                .map(|i| i.to_string())
                .collect()
        }
    }
}

fn entries(id: Option<u32>, computed: bool) -> Vec<String> {
    let Some(id) = id else { return Vec::new() };
    if computed {
        dom::computed_entries(id)
            .unwrap_or_default()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    } else {
        tree::read_tree(|t| t.style_entries(id))
            .into_iter()
            .map(|(k, _, _)| k)
            .collect()
    }
}

fn len(id: Option<u32>, computed: bool) -> usize {
    match (id, computed) {
        (Some(id), false) => tree::read_tree(|t| t.node(id).map_or(0, |n| n.style.len())),
        _ => entries(id, computed).len(),
    }
}

fn item(id: Option<u32>, computed: bool, i: usize) -> Option<String> {
    entries(id, computed).into_iter().nth(i)
}

macro_rules! this_decl {
    ($g:ident, $args:ident) => {{
        let this = $args.this().0;
        match decl($g, this) {
            Some(d) => (this, d.computed),
            None => return throw_type_error($g, "Illegal invocation"),
        }
    }};
}

host_fn!(fn get_property_value(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let name = unsafe { string_of(g, args.arg(0).0) };
    let css = cssom_name(&name);
    let id = unsafe { node_id(g, this, computed) };
    let value = if computed { unsafe { computed_read(g, id, &css, &name) } } else { inline_read(id, &css).0 };
    unsafe { js_string(g, &value) }
});

host_fn!(fn get_property_priority(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let name = cssom_name(&unsafe { string_of(g, args.arg(0).0) });
    let important = !computed && inline_read(unsafe { node_id(g, this, false) }, &name).1;
    unsafe { js_string(g, if important { "important" } else { "" }) }
});

host_fn!(fn set_property(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let name = cssom_name(&unsafe { string_of(g, args.arg(0).0) });
    if computed {
        return unsafe { glue::rjs_throw(g, read_only(g, &name)); UNDEF };
    }
    let custom = name.starts_with("--");
    let v = args.arg(1).0;
    if v == UNDEF && !custom {
        return UNDEF;
    }
    let value = if v == NULL || v == UNDEF { String::new() } else { unsafe { string_of(g, v) } };
    let Some(id) = (unsafe { node_id(g, this, false) }) else { return UNDEF };
    if value.is_empty() {
        unsafe { mutate(g, id, |t| t.style_remove(id, &name).is_some()) };
        return UNDEF;
    }
    let p = args.arg(2).0;
    let priority = if p == UNDEF || p == NULL { String::new() } else { unsafe { string_of(g, p) } };
    if !priority.is_empty() && !priority.eq_ignore_ascii_case("important") {
        return UNDEF;
    }
    if !custom && strip_important(&value).is_some() {
        return UNDEF;
    }
    let stored = stored_value(&value, custom);
    unsafe { mutate(g, id, |t| t.style_write(id, &name, &stored, !priority.is_empty())) };
    UNDEF
});

host_fn!(fn remove_property(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let name = cssom_name(&unsafe { string_of(g, args.arg(0).0) });
    if computed {
        return unsafe { glue::rjs_throw(g, read_only(g, &name)); UNDEF };
    }
    let mut prev = String::new();
    if let Some(id) = unsafe { node_id(g, this, false) } {
        unsafe {
            mutate(g, id, |t| match t.style_remove(id, &name) {
                Some(v) => {
                    prev = v;
                    true
                }
                None => false,
            })
        };
    }
    unsafe { js_string(g, &prev) }
});

host_fn!(fn item_fn(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let i = unsafe { to_number(g, args.arg(0).0) }.unwrap_or(0.0);
    let name = if i >= 0.0 { item(unsafe { node_id(g, this, computed) }, computed, i as usize) } else { None };
    unsafe { js_string(g, &name.unwrap_or_default()) }
});

host_fn!(fn length_get(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    enc_f64(len(unsafe { node_id(g, this, computed) }, computed) as f64)
});

host_fn!(fn css_text_get(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let text = match (computed, unsafe { node_id(g, this, computed) }) {
        (false, Some(id)) => tree::read_tree(|t| t.style_text(id)),
        _ => String::new(),
    };
    if !text.is_empty() {
        unsafe { note_serialization(g) };
    }
    unsafe { js_string(g, &text) }
});

host_fn!(fn css_text_set(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    if computed {
        return unsafe { glue::rjs_throw(g, read_only(g, "cssText")); UNDEF };
    }
    let v = args.arg(0).0;
    let text = if v == NULL || v == UNDEF { String::new() } else { unsafe { string_of(g, v) } };
    if let Some(id) = unsafe { node_id(g, this, false) } {
        unsafe { mutate(g, id, |t| { t.style_replace(id, &text, false); true }) };
    }
    UNDEF
});

host_fn!(fn css_float_get(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    let id = unsafe { node_id(g, this, computed) };
    let v = if computed { String::new() } else { inline_read(id, "float").0 };
    unsafe { js_string(g, &v) }
});

host_fn!(fn css_float_set(g, args) {
    let (this, computed) = unsafe { this_decl!(g, args) };
    if computed {
        return unsafe { glue::rjs_throw(g, read_only(g, "cssFloat")); UNDEF };
    }
    if let Some(id) = unsafe { node_id(g, this, false) } {
        unsafe { assign(g, id, "float", args.arg(0).0) };
    }
    UNDEF
});

host_fn!(fn parent_rule_get(_g, _args) {
    NULL
});

/// Said once, at the first read: unlike a browser, serialization keeps values as written.
pub(crate) unsafe fn note_serialization(g: *mut c_void) {
    unsafe {
        sk_runtime::host::warn_compat(
        g,
        "inline-style-serialize",
        "el.style.cssText / getAttribute('style') serialize the inline declarations as they were written: values are \
         not re-spelled (`#f00` stays `#f00`), a shorthand is not expanded into its longhands (after `margin: 0`, \
         `el.style.marginTop` reads \"\"), and a value the engine rejects is still listed",
    );
    }
}

fn arg_id(v: u64) -> u32 {
    dec_f64(v).map_or(0, |d| d as u32)
}

host_fn!(fn style_object(g, args) {
    unsafe { create(g, args.arg(0).0, false) }
});

host_fn!(fn computed_object(g, args) {
    unsafe { create(g, args.arg(0).0, true) }
});

enum Op {
    Remove,
    Write(String, bool),
}

/// runtime-dom's `setStyle` for one `:style` entry: `""` removes, a trailing `!important` is the
/// priority, a custom property keeps an empty value.
fn style_op(name: &str, value: &str) -> Op {
    if value.is_empty() {
        return Op::Remove;
    }
    if name.starts_with("--") {
        return Op::Write(stored_value(value, true), false);
    }
    match value.strip_suffix("!important").map(str::trim_end) {
        Some(bare) if bare.trim().is_empty() => Op::Remove,
        Some(bare) => Op::Write(stored_value(bare, false), true),
        None => Op::Write(stored_value(value, false), false),
    }
}

/// `false` when the declaration already was what `op` asks for.
fn apply(t: &mut tree::Tree, id: u32, name: &str, op: &Op) -> bool {
    match op {
        Op::Remove => t.style_remove(id, name).is_some(),
        Op::Write(value, important) => t.style_write(id, name, value, *important),
    }
}

host_fn!(fn style_assign(g, args) {
    let id = arg_id(args.arg(0).0);
    let name = css_name(&unsafe { string_of(g, args.arg(1).0) });
    let v = args.arg(2).0;
    let value = if v == NULL || v == UNDEF { String::new() } else { unsafe { string_of(g, v) } };
    let op = style_op(&name, &value);
    unsafe { mutate(g, id, |t| apply(t, id, &name, &op)) };
    UNDEF
});

/// The last entry the engine accepts; with none, the declaration is left as it was.
unsafe fn fallback_op(g: *mut c_void, prop: &str, list: &[&str]) -> Option<Op> {
    unsafe {
        let custom = prop.starts_with("--");
        let (i, v) = list.iter().copied().enumerate().rev().find(|&(_, v)| {
            let bare = v.strip_suffix("!important").map_or(v, str::trim_end).trim();
            bare.is_empty() || custom || sk_css::accepts_declaration(prop, bare)
        })?;
        if i + 1 < list.len() {
            let later = list.len() - 1 - i;
            let shown: Vec<String> = list.iter().map(|x| format!("{x:?}")).collect();
            sk_runtime::host::warn_compat(
            g,
            &format!("style-array:{prop}"),
            &format!(
                ":style {prop}: [{}] uses {v:?}: the engine does not apply the later {}. A browser keeps the last \
                 entry IT accepts, so one that supports a later entry would use that one",
                shown.join(", "),
                if later == 1 { "entry" } else { "entries" },
            ),
        );
        }
        Some(style_op(prop, v))
    }
}

// runtime-dom's `setStyle` over a whole record `:style`, in one call and one attribute change. `entries` is
// `key\0value\0` per entry: an empty value removes; `\x02` then `\x01value` per entry is an array, whose last
// entry the engine accepts wins. Each write is diffed against the node's inline declaration, so an unchanged
// value costs nothing and one changed through `el.style` is restored.
host_fn!(fn style_patch(g, args) {
    let id = arg_id(args.arg(0).0);
    let entries = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let mut ops: Vec<(String, Op)> = Vec::new();
    let mut parts = entries.split('\0');
    while let (Some(key), Some(value)) = (parts.next(), parts.next()) {
        let name = css_name(key);
        let op = match value.strip_prefix('\x02') {
            Some(list) => unsafe { fallback_op(g, &name, &list.split('\x01').skip(1).collect::<Vec<_>>()) },
            None => Some(style_op(&name, value)),
        };
        if let Some(op) = op {
            ops.push((name, op));
        }
    }
    if !ops.is_empty() {
        unsafe {
            mutate(g, id, |t| {
                ops.iter().fold(false, |changed, (name, op)| apply(t, id, name, op) | changed)
            })
        };
    }
    UNDEF
});

host_fn!(fn style_get(g, args) {
    let id = arg_id(args.arg(0).0);
    let name = unsafe { string_of(g, args.arg(1).0) };
    unsafe { js_string(g, &inline_read(Some(id), &name).0) }
});

host_fn!(fn style_replace(g, args) {
    let id = arg_id(args.arg(0).0);
    let text = unsafe { string_of(g, args.arg(1).0) };
    let as_attribute = args.arg(2).as_bool().unwrap_or(false);
    unsafe { mutate(g, id, |t| { t.style_replace(id, &text, as_attribute); true }) };
    UNDEF
});

host_fn!(fn style_clear(g, args) {
    let id = arg_id(args.arg(0).0);
    unsafe { mutate(g, id, |t| t.style_clear(id)) };
    UNDEF
});

host_fn!(fn style_attr(g, args) {
    let id = arg_id(args.arg(0).0);
    let (attr, raw) = tree::read_tree(|t| (t.style_attr(id), t.node(id).is_some_and(|n| n.style_raw.is_some())));
    match attr {
        Some(text) => unsafe {
            if !raw && !text.is_empty() {
                note_serialization(g);
            }
            js_string(g, &text)
        },
        None => NULL,
    }
});

host_fn!(fn style_hook(g, args) {
    let f = args.arg(0).0;
    let (ctx, old) = HOOK.replace((g as usize, if is_cell(f) { f } else { UNDEF }));
    unsafe {
        if is_cell(f) {
            redjs_protect(g, f);
        }
        if is_cell(old) && ctx == g as usize {
            redjs_unprotect(g, old);
        }
    }
    UNDEF
});

host_fn!(fn style_observe(_g, args) {
    OBSERVING.set(args.arg(0).as_bool().unwrap_or(false));
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    for (name, arity, f) in [
        ("__sk_style_object", 1, style_object as *const c_void),
        ("__sk_computed_object", 1, computed_object as *const c_void),
        ("__sk_style_assign", 3, style_assign as *const c_void),
        ("__sk_style_patch", 2, style_patch as *const c_void),
        ("__sk_style_get", 2, style_get as *const c_void),
        ("__sk_style_replace", 3, style_replace as *const c_void),
        ("__sk_style_clear", 1, style_clear as *const c_void),
        ("__sk_style_attr", 1, style_attr as *const c_void),
        ("__sk_style_hook", 1, style_hook as *const c_void),
        ("__sk_style_observe", 1, style_observe as *const c_void),
    ] {
        rt.register(name, arity, f);
    }
    unsafe {
        let g = rt.raw();
        let cls = make_named_host_class::<Decl>("CSSStyleDeclaration");
        let class = define_class(
            g,
            &ClassSpec {
                name: "CSSStyleDeclaration",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[
                    ("getPropertyValue", 1, get_property_value),
                    ("getPropertyPriority", 1, get_property_priority),
                    ("setProperty", 2, set_property),
                    ("removeProperty", 1, remove_property),
                    ("item", 1, item_fn),
                ],
                getters: &[("length", length_get), ("parentRule", parent_rule_get)],
                accessors: &[
                    ("cssText", css_text_get, css_text_set),
                    ("cssFloat", css_float_get, css_float_set),
                ],
            },
        );
        let values = get(
            g,
            get(g, get(g, glue::rjs_global(g), "Array"), "prototype"),
            "values",
        );
        let sym = get(g, get(g, glue::rjs_global(g), "Symbol"), "iterator");
        let object = get(g, glue::rjs_global(g), "Object");
        let desc = new_object(g, UNDEF);
        define(g, desc, "value", values, 0);
        define(g, desc, "writable", enc_bool(true), 0);
        define(g, desc, "configurable", enc_bool(true), 0);
        let _ = call(
            g,
            get(g, object, "defineProperty"),
            object,
            &[class.proto, sym, desc],
        );
        redjs_protect(g, class.proto);
        C.with(|c| {
            *c.borrow_mut() = Classes {
                cls: cls as usize,
                proto: class.proto,
                node: atom_priv(g, "node") as usize,
            }
        });
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

    fn element() -> u32 {
        tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let el = t.create_element(div);
            t.insert(el, root, 0);
            el
        })
    }

    #[test]
    fn el_style_reads_and_writes_the_inline_declarations() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(install);
            let el = element();
            js(&format!(
                "globalThis.node = {{ __skId: {el} }}; globalThis.s = __sk_style_object(node); globalThis.seen = []; \
                 __sk_style_observe(false); __sk_style_hook((id, old, present) => seen.push(`${{id}}|${{old}}|${{present}}`)); ''"
            ));
            assert_eq!(js("s instanceof CSSStyleDeclaration && s.color === '' && s.length === 0 && typeof s.setProperty"), "function");
            js("s.backgroundColor = 'red'; s['font-size'] = '12px'; s.setProperty('--Brand', ' blue ', 'important')");
            assert_eq!(
                tree::read_tree(|t| t.style_text(el)),
                "background-color: red; font-size: 12px; --Brand: blue !important;"
            );
            assert_eq!(
                js("[s.backgroundColor, s.fontSize, s.getPropertyValue('--Brand'), s.getPropertyPriority('--Brand'), s.length, s[1], s[9], [...s].join(), Object.keys(s).join()].join('|')"),
                "red|12px|blue|important|3|font-size||background-color,font-size,--Brand|0,1,2"
            );
            assert_eq!(
                js("seen.join()"),
                format!("{el}|null|true"),
                "only the attribute appearing is reported"
            );

            js("s.color = 'red !important'; s.width = undefined; s.fontSize = null; s._x = 1; s.cssFloat = 'left'");
            assert_eq!(
                tree::read_tree(|t| t.style_text(el)),
                "background-color: red; --Brand: blue !important; float: left;"
            );
            assert_eq!(js("String(s._x === 1 && Object.prototype.hasOwnProperty.call(s, '_x') && s.cssFloat === 'left')"), "true");

            js("__sk_style_observe(true); seen.length = 0; s.cssText = 'color: green; margin: 0 !important'");
            assert_eq!(
                js("seen.join() + '|' + s.cssText + '|' + s.removeProperty('color') + '|' + s.removeProperty('nope')"),
                format!("{el}|background-color: red; --Brand: blue !important; float: left;|true|color: green; margin: 0 !important;|green|")
            );
            js("__sk_style_observe(false)");

            js("node.__skId = 0");
            assert_eq!(
                js("String(s.margin === '' && s.length === 0 && (s.color = 'red', true))"),
                "true"
            );
            assert_eq!(
                tree::read_tree(|t| t.style_text(el)),
                "margin: 0 !important;",
                "a freed node's object writes nothing"
            );
        });
    }

    /// What `patchStyle` in inline-style.ts sends for a record `:style`.
    const PACK: &str = r#"globalThis.pack = (prev, next) => {
        let s = "";
        if (prev) for (const k in prev) if (next[k] == null) s += k + "\0\0";
        for (const k in next) {
            const v = next[k];
            s += k + "\0" + (v == null ? "" : Array.isArray(v) ? "\x02" + v.flat(Infinity).map((x) => "\x01" + (x == null ? "" : String(x))).join("") : String(v)) + "\0";
        }
        return s;
    }; ''"#;

    #[test]
    fn style_patch_writes_only_declarations_that_differ_from_the_inline_ones() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(install);
            let el = element();
            let text = || tree::read_tree(|t| t.style_text(el));
            js(PACK);
            js(&format!(
                "globalThis.s = __sk_style_object({{ __skId: {el} }}); globalThis.seen = 0; \
                 __sk_style_hook(() => seen++); __sk_style_observe(true); \
                 globalThis.p = (a, b) => __sk_style_patch({el}, pack(a, b)); \
                 globalThis.a = {{ width: '10px', backgroundColor: 'red', '--Gap': '4px', color: 'blue !important', zIndex: 2 }}; ''"
            ));
            assert_eq!(js("p(null, a); seen"), "1");
            assert_eq!(
                text(),
                "width: 10px; background-color: red; --Gap: 4px; color: blue !important; z-index: 2;"
            );

            assert_eq!(
                js("p(a, { ...a, zIndex: '2', color: 'blue!important' }); seen"),
                "1",
                "equal values write nothing"
            );

            js("s.width = '5px'; s.removeProperty('--Gap'); seen = 0; p(a, { ...a })");
            assert_eq!(js("String(seen)"), "1", "one notification per patch");
            assert_eq!(
                text(),
                "width: 10px; background-color: red; color: blue !important; z-index: 2; --Gap: 4px;",
                "what el.style changed is restored"
            );

            js("p(a, { width: '12px', display: 'flex', color: 'blue', zIndex: null })");
            assert_eq!(text(), "width: 12px; color: blue; display: flex;");

            js("__sk_style_observe(false); p({ width: '12px' }, { width: ['14px', 'nope nope'], height: ['1px', null], color: [] })");
            assert_eq!(
                text(),
                "width: 14px; color: blue; display: flex;",
                "an array keeps the last accepted entry; an empty one changes nothing"
            );
        });
    }

    #[test]
    fn computed_style_reads_one_property_at_a_time_and_refuses_writes() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(install);
            let el = element();
            js(&format!(
                "globalThis.c = __sk_computed_object({{ __skId: {el} }}); ''"
            ));
            assert_eq!(
                js("[c.display, c['z-index'], c.getPropertyValue('position'), c.width, c.getPropertyValue('--unset'), 'display' in c, c.length > 20].join('|')"),
                "block|auto|static|0px||true|true"
            );
            assert_eq!(
                js("c.getPropertyPriority('display') + c.cssText + c.parentRule"),
                "null"
            );
            assert_eq!(
                js("(() => { try { c.color = 'red'; } catch (e) { return e.name; } })()"),
                "NoModificationAllowedError"
            );
            assert_eq!(js("(() => { try { c.setProperty('color', 'red'); } catch (e) { return e.name; } })()"), "NoModificationAllowedError");
            assert_eq!(js("(() => { try { CSSStyleDeclaration.prototype.item.call({}, 0); return 'ok'; } catch (e) { return 'threw'; } })()"), "threw");
        });
    }
}
