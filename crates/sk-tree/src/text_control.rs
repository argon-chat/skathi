//! The text-field members of `HTMLInputElement` and `HTMLTextAreaElement` that read or replace the
//! editor's selection: `selectionStart`, `selectionEnd`, `selectionDirection` and `setRangeText()`,
//! in UTF-16 code units. The runtime hands over its text-control prototype
//! (`__sk_text_control_install`); `setSelectionRange()` and `select()` stay on it.

use std::ffi::c_void;

use sk_runtime::host::throw;
use sk_runtime::host_fn;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

use crate::native_dom::{accessor, method, node_id};
use crate::tree::{self, Tree};

/// Input types the selection API does not apply to; any other type, unknown ones included, is text.
const NOT_SELECTABLE: &[&str] = &[
    "hidden",
    "email",
    "number",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "range",
    "color",
    "checkbox",
    "radio",
    "file",
    "submit",
    "image",
    "reset",
    "button",
];

/// Input types `maxlength` applies to besides the selectable ones.
const MAX_LENGTH_TOO: &[&str] = &["email"];

fn input_type(t: &Tree, id: u32) -> String {
    t.prop(id, "type")
        .unwrap_or("text")
        .trim()
        .to_ascii_lowercase()
}

fn text_control(t: &Tree, id: u32) -> Option<bool> {
    let node = t.node(id)?;
    match t.name(node.tag) {
        "textarea" => Some(true),
        "input" => Some(false),
        _ => None,
    }
}

fn selectable(t: &Tree, id: u32) -> bool {
    match text_control(t, id) {
        Some(true) => true,
        Some(false) => !NOT_SELECTABLE.contains(&input_type(t, id).as_str()),
        None => false,
    }
}

/// HTML `maxlength` for the native editor, `None` where it does not apply or does not parse.
pub(crate) fn max_length(t: &Tree, id: u32) -> Option<usize> {
    let applies = selectable(t, id) || MAX_LENGTH_TOO.contains(&input_type(t, id).as_str());
    if !applies {
        return None;
    }
    t.prop(id, "maxlength")?.trim().parse().ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectMode {
    Select,
    Start,
    End,
    Preserve,
}

impl SelectMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "select" => Some(Self::Select),
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            "preserve" => Some(Self::Preserve),
            _ => None,
        }
    }
}

/// HTML's `setRangeText` on UTF-16 units: `range` (already ordered) and `selection` are clamped to
/// the value; answers the new value and selection.
fn range_text(
    value: &[u16],
    replacement: &[u16],
    range: (usize, usize),
    selection: (usize, usize),
    mode: SelectMode,
) -> (Vec<u16>, usize, usize) {
    let len = value.len();
    let (start, end) = (range.0.min(len), range.1.min(len));
    let (mut sel_start, mut sel_end) = (selection.0.min(len), selection.1.min(len));
    let mut out = Vec::with_capacity(len - (end - start) + replacement.len());
    out.extend_from_slice(&value[..start]);
    out.extend_from_slice(replacement);
    out.extend_from_slice(&value[end..]);
    let new_end = start + replacement.len();
    match mode {
        SelectMode::Select => (sel_start, sel_end) = (start, new_end),
        SelectMode::Start => (sel_start, sel_end) = (start, start),
        SelectMode::End => (sel_start, sel_end) = (new_end, new_end),
        SelectMode::Preserve => {
            let delta = replacement.len() as isize - (end - start) as isize;
            if sel_start > end {
                sel_start = (sel_start as isize + delta) as usize;
            } else if sel_start > start {
                sel_start = start;
            }
            if sel_end > end {
                sel_end = (sel_end as isize + delta) as usize;
            } else if sel_end > start {
                sel_end = new_end;
            }
        }
    }
    (out, sel_start, sel_end)
}

/// WebIDL `unsigned long`.
unsafe fn uint32(g: *mut c_void, v: u64) -> Result<usize, ()> {
    unsafe {
        let n = to_number(g, v)?;
        if !n.is_finite() {
            return Ok(0);
        }
        Ok(n.trunc().rem_euclid(4_294_967_296.0) as usize)
    }
}

/// The selectable field `this` mirrors; `None` for any other receiver.
unsafe fn field_of(g: *mut c_void, this: u64) -> Option<u32> {
    unsafe {
        let id = node_id(g, this)?;
        tree::read_tree(|t| selectable(t, id)).then_some(id)
    }
}

fn caret(id: u32) -> (usize, usize, bool) {
    crate::render::input_caret(id).unwrap_or((0, 0, false))
}

fn direction(backward: bool) -> &'static str {
    match backward {
        true => "backward",
        false if cfg!(target_os = "macos") => "none",
        false => "forward",
    }
}

/// `this.setSelectionRange(start, end, direction)`, so its checks and its `select` event apply.
unsafe fn select_range(g: *mut c_void, this: u64, start: usize, end: usize, dir: u64) -> u64 {
    unsafe {
        let f = get(g, this, "setSelectionRange");
        let args = [enc_f64(start as f64), enc_f64(end as f64), dir];
        if let Err(e) = call(g, f, this, &args) {
            glue::rjs_throw(g, e);
        }
        UNDEF
    }
}

host_fn!(fn selection_start_get(g, args) {
    match unsafe { field_of(g, args.this().0) } {
        Some(id) => enc_f64(caret(id).0 as f64),
        None => NULL,
    }
});

host_fn!(fn selection_end_get(g, args) {
    match unsafe { field_of(g, args.this().0) } {
        Some(id) => enc_f64(caret(id).1 as f64),
        None => NULL,
    }
});

host_fn!(fn selection_direction_get(g, args) {
    match unsafe { field_of(g, args.this().0) } {
        Some(id) => unsafe { js_string(g, direction(caret(id).2)) },
        None => NULL,
    }
});

host_fn!(fn selection_start_set(g, args) {
    unsafe {
        let this = args.this().0;
        let Ok(start) = uint32(g, args.arg(0).0) else { return UNDEF };
        let (_, end, backward) = field_of(g, this).map_or((0, 0, false), caret);
        select_range(g, this, start, end.max(start), js_string(g, direction(backward)))
    }
});

host_fn!(fn selection_end_set(g, args) {
    unsafe {
        let this = args.this().0;
        let Ok(end) = uint32(g, args.arg(0).0) else { return UNDEF };
        let (start, _, backward) = field_of(g, this).map_or((0, 0, false), caret);
        select_range(g, this, start, end, js_string(g, direction(backward)))
    }
});

host_fn!(fn selection_direction_set(g, args) {
    unsafe {
        let this = args.this().0;
        let (start, end, _) = field_of(g, this).map_or((0, 0, false), caret);
        select_range(g, this, start, end, args.arg(0).0)
    }
});

host_fn!(fn set_range_text(g, args) {
    unsafe {
        let this = args.this().0;
        let Some(id) = node_id(g, this) else {
            return throw(g, "TypeError", "Illegal invocation");
        };
        let Some(textarea) = tree::read_tree(|t| text_control(t, id)) else {
            return throw(g, "TypeError", "Illegal invocation");
        };
        let what = if textarea { "HTMLTextAreaElement" } else { "HTMLInputElement" };
        let argc = args.argc();
        if argc == 0 || argc == 2 {
            return throw(
                g,
                "TypeError",
                &format!(
                    "Failed to execute 'setRangeText' on '{what}': Valid arities are: [1, 3, 4], \
                     but {argc} arguments provided."
                ),
            );
        }
        let Ok(replacement) = to_js_string(g, args.arg(0).0) else { return UNDEF };
        let replacement = read_string16(g, replacement).unwrap_or_default();
        if !tree::read_tree(|t| selectable(t, id)) {
            let ty = tree::read_tree(|t| input_type(t, id));
            return throw(
                g,
                "InvalidStateError",
                &format!("The input element's type ('{ty}') does not support selection."),
            );
        }
        let (sel_start, sel_end, _) = caret(id);
        let range = if argc >= 3 {
            let (Ok(start), Ok(end)) = (uint32(g, args.arg(1).0), uint32(g, args.arg(2).0)) else {
                return UNDEF;
            };
            (start, end)
        } else {
            (sel_start, sel_end)
        };
        let mode = match args.arg(3).0 {
            UNDEF => SelectMode::Preserve,
            v => {
                let Ok(s) = to_js_string(g, v) else { return UNDEF };
                let s = read_string(g, s).unwrap_or_default();
                match SelectMode::parse(&s) {
                    Some(mode) => mode,
                    None => {
                        return throw(
                            g,
                            "TypeError",
                            &format!(
                                "Failed to execute 'setRangeText' on '{what}': The provided value \
                                 '{s}' is not a valid enum value of type SelectionMode."
                            ),
                        )
                    }
                }
            }
        };
        if range.0 > range.1 {
            return throw(
                g,
                "IndexSizeError",
                &format!(
                    "Failed to execute 'setRangeText' on '{what}': The provided start value ({}) \
                     is larger than the provided end value ({}).",
                    range.0, range.1
                ),
            );
        }
        let value = read_string16(g, get(g, this, "value")).unwrap_or_default();
        let (next, start, end) = range_text(&value, &replacement, range, (sel_start, sel_end), mode);
        if glue::rjs_set(g, this, key(g, "value"), js_string16(g, &next)) != 0 {
            return UNDEF;
        }
        select_range(g, this, start, end, UNDEF)
    }
});

host_fn!(fn install_on(g, args) {
    let proto = args.arg(0).0;
    if !is_cell(proto) {
        return UNDEF;
    }
    unsafe {
        accessor(g, proto, "selectionStart", selection_start_get, Some(selection_start_set));
        accessor(g, proto, "selectionEnd", selection_end_get, Some(selection_end_set));
        accessor(
            g,
            proto,
            "selectionDirection",
            selection_direction_get,
            Some(selection_direction_set),
        );
        method(g, proto, "setRangeText", 2, set_range_text);
    }
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__sk_text_control_install", 1, install_on as *const c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn run(
        value: &str,
        with: &str,
        range: (usize, usize),
        sel: (usize, usize),
        mode: SelectMode,
    ) -> (String, usize, usize) {
        let (out, s, e) = range_text(&u(value), &u(with), range, sel, mode);
        (String::from_utf16(&out).unwrap(), s, e)
    }

    #[test]
    fn preserve_shifts_a_selection_after_the_range_and_snaps_one_inside_it() {
        assert_eq!(
            run("hello world", "J", (0, 5), (6, 11), SelectMode::Preserve),
            ("J world".into(), 2, 7)
        );
        assert_eq!(
            run(
                "hello world",
                "HELLO!",
                (0, 5),
                (2, 3),
                SelectMode::Preserve
            ),
            ("HELLO! world".into(), 0, 6)
        );
        assert_eq!(
            run("abc", "X", (3, 3), (1, 1), SelectMode::Preserve),
            ("abcX".into(), 1, 1)
        );
    }

    #[test]
    fn the_other_modes_place_the_selection_on_the_replacement() {
        assert_eq!(
            run("abcdef", "XY", (1, 4), (0, 0), SelectMode::Select),
            ("aXYef".into(), 1, 3)
        );
        assert_eq!(
            run("abcdef", "XY", (1, 4), (0, 0), SelectMode::Start),
            ("aXYef".into(), 1, 1)
        );
        assert_eq!(
            run("abcdef", "XY", (1, 4), (0, 0), SelectMode::End),
            ("aXYef".into(), 3, 3)
        );
    }

    #[test]
    fn offsets_count_utf16_units_and_clamp_to_the_value() {
        assert_eq!(
            run("я🙂z", "!", (1, 3), (4, 4), SelectMode::Preserve),
            ("я!z".into(), 3, 3)
        );
        assert_eq!(
            run("ab", "c", (9, 99), (9, 9), SelectMode::End),
            ("abc".into(), 3, 3)
        );
    }

    #[test]
    fn maxlength_and_selection_apply_to_the_text_types_only() {
        let (field, area, number, mail) = tree::with_tree(|t| {
            let root = t.create_root();
            let input = t.intern("input");
            let textarea = t.intern("textarea");
            let ty = t.intern("type");
            let maxlength = t.intern("maxlength");
            let mk = |t: &mut Tree, tag, kind: Option<&str>| {
                let id = t.create_element(tag);
                t.insert(id, root, 0);
                if let Some(kind) = kind {
                    t.set_prop(id, ty, Some(kind.to_string()));
                }
                t.set_prop(id, maxlength, Some(" 4 ".to_string()));
                id
            };
            (
                mk(t, input, Some("Search")),
                mk(t, textarea, None),
                mk(t, input, Some("number")),
                mk(t, input, Some("email")),
            )
        });
        tree::read_tree(|t| {
            assert_eq!(max_length(t, field), Some(4));
            assert_eq!(max_length(t, area), Some(4));
            assert_eq!(max_length(t, number), None);
            assert_eq!(max_length(t, mail), Some(4));
            assert!(selectable(t, field) && selectable(t, area));
            assert!(!selectable(t, number) && !selectable(t, mail));
        });
    }
}
