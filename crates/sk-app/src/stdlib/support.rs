//! What the std classes share: argument decoding that throws, bytes in and out, classes that stay
//! off the global object.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{RjsAbInfo, RjsTaInfo};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

pub(super) const RANGE_ERROR: u32 = 2;
pub(super) const SYNTAX_ERROR: u32 = 3;

pub(super) const KIND_STRING: u32 = 5;
pub(super) const KIND_BIGINT: u32 = 7;

pub(super) const TA_U32: u32 = 7;
pub(super) const TA_F64: u32 = 9;
pub(super) const TA_DATA_VIEW: u32 = 12;

/// The exception is already pending in the VM.
pub(super) struct Thrown;
pub(super) type Js<T> = Result<T, Thrown>;

/// A host function whose body is `Js<u64>`: an `Err` returns to the VM with its exception pending.
macro_rules! std_fn {
    (fn $name:ident($g:ident, $args:ident) $body:block) => {
        sk_runtime::host_fn!(fn $name($g, $args) {
            #[allow(unused_unsafe)]
            let run = || -> $crate::stdlib::support::Js<u64> { unsafe { $body } };
            run().unwrap_or(sk_runtime::redjs::webapi::util::UNDEF)
        });
    };
}
pub(super) use std_fn;

pub(super) unsafe fn type_error<T>(g: *mut c_void, message: &str) -> Js<T> {
    throw_type_error(g, message);
    Err(Thrown)
}

pub(super) unsafe fn range_error<T>(g: *mut c_void, message: &str) -> Js<T> {
    throw_error(g, RANGE_ERROR, message);
    Err(Thrown)
}

pub(super) unsafe fn syntax_error<T>(g: *mut c_void, message: &str) -> Js<T> {
    throw_error(g, SYNTAX_ERROR, message);
    Err(Thrown)
}

pub(super) unsafe fn kind(g: *mut c_void, v: u64) -> u32 {
    glue::rjs_kind(g, v)
}

pub(super) unsafe fn number(g: *mut c_void, v: u64) -> Js<f64> {
    to_number(g, v).map_err(|()| Thrown)
}

/// A count, an offset or a size: an integer from 0 to 2^53.
pub(super) unsafe fn count(g: *mut c_void, v: u64, what: &str) -> Js<usize> {
    let n = number(g, v)?;
    if n.fract() != 0.0 || !(0.0..=9_007_199_254_740_992.0).contains(&n) {
        return range_error(g, &format!("{what} must be a non-negative integer"));
    }
    Ok(n as usize)
}

pub(super) unsafe fn string(g: *mut c_void, v: u64, what: &str) -> Js<String> {
    if kind(g, v) != KIND_STRING {
        return type_error(g, &format!("{what} must be a string"));
    }
    match read_string(g, v) {
        Some(s) => Ok(s),
        None => Err(Thrown),
    }
}

/// A member of an options bag; `undefined` when there is no bag.
pub(super) unsafe fn option(g: *mut c_void, bag: u64, name: &str) -> u64 {
    if is_cell(bag) {
        get(g, bag, name)
    } else {
        UNDEF
    }
}

/// One of `names` by its position, or `default` for `undefined`.
pub(super) unsafe fn choice(
    g: *mut c_void,
    v: u64,
    names: &[&str],
    default: usize,
    what: &str,
) -> Js<usize> {
    if v == UNDEF {
        return Ok(default);
    }
    let given = if kind(g, v) == KIND_STRING {
        read_string(g, v)
    } else {
        None
    };
    match given.and_then(|s| names.iter().position(|n| *n == s)) {
        Some(i) => Ok(i),
        None => {
            let list = names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ");
            type_error(g, &format!("{what} must be one of {list}"))
        }
    }
}

pub(super) struct View {
    pub data: *mut u8,
    pub len: usize,
    /// Elements, as the typed array counts them; bytes for an ArrayBuffer or a DataView.
    pub length: usize,
    pub kind: u32,
    pub byte_offset: usize,
}

impl View {
    pub fn element_size(&self) -> usize {
        if self.length == 0 {
            1
        } else {
            self.len / self.length
        }
    }

    /// # Safety
    /// Valid until the next allocation or call into JS.
    pub unsafe fn bytes<'a>(&self) -> &'a [u8] {
        if self.len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(self.data, self.len)
        }
    }

    /// # Safety
    /// Valid until the next allocation or call into JS.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn bytes_mut<'a>(&self) -> &'a mut [u8] {
        if self.len == 0 {
            &mut []
        } else {
            std::slice::from_raw_parts_mut(self.data, self.len)
        }
    }
}

/// A typed array or a DataView; a detached one is empty.
pub(super) unsafe fn typed_view(g: *mut c_void, v: u64) -> Option<View> {
    if !is_cell(v) {
        return None;
    }
    let mut ta = RjsTaInfo::default();
    if glue::rjs_typedarray_info(g, v, &mut ta) != 0 {
        return None;
    }
    let live = ta.detached == 0 && !ta.data.is_null();
    Some(View {
        data: ta.data as *mut u8,
        len: if live { ta.byte_length as usize } else { 0 },
        length: if live { ta.length as usize } else { 0 },
        kind: ta.kind,
        byte_offset: ta.byte_offset as usize,
    })
}

/// A typed array, a DataView or an ArrayBuffer.
pub(super) unsafe fn buffer_source(g: *mut c_void, v: u64) -> Option<View> {
    if let Some(view) = typed_view(g, v) {
        return Some(view);
    }
    if !is_cell(v) {
        return None;
    }
    let mut ab = RjsAbInfo::default();
    if glue::rjs_arraybuffer_info(g, v, &mut ab) != 0 {
        return None;
    }
    let live = ab.detached == 0 && !ab.data.is_null();
    let len = if live { ab.byte_length as usize } else { 0 };
    Some(View {
        data: ab.data as *mut u8,
        len,
        length: len,
        kind: 0,
        byte_offset: 0,
    })
}

pub(super) unsafe fn bytes_arg<'a>(g: *mut c_void, v: u64, what: &str) -> Js<&'a [u8]> {
    match buffer_source(g, v) {
        Some(view) => Ok(view.bytes()),
        None => type_error(
            g,
            &format!("{what} must be an ArrayBuffer, a typed array or a DataView"),
        ),
    }
}

/// A new typed array of `kind` and its zeroed bytes.
pub(super) unsafe fn typed_array<'a>(
    g: *mut c_void,
    kind: u32,
    len: usize,
) -> Js<(u64, &'a mut [u8])> {
    let mut out = UNDEF;
    if sk_runtime::redjs::ffi::redjs_typedarray_create(g, kind, len, &mut out) != 0 {
        return range_error(g, "the array could not be allocated");
    }
    match typed_view(g, out) {
        Some(view) => Ok((out, view.bytes_mut())),
        None => Err(Thrown),
    }
}

pub(super) unsafe fn construct(g: *mut c_void, class: u64, args: &[u64]) -> Js<u64> {
    let global = glue::rjs_global(g);
    let list = new_array(g);
    for (i, arg) in args.iter().enumerate() {
        set_i(g, list, i as u32, *arg);
    }
    let reflect = get(g, get(g, global, "Reflect"), "construct");
    call(g, reflect, UNDEF, &[class, list]).map_err(|_| Thrown)
}

pub(super) unsafe fn big_uint(g: *mut c_void, v: u64) -> Js<u64> {
    let big = get(g, glue::rjs_global(g), "BigInt");
    call(g, big, UNDEF, &[js_string(g, &v.to_string())]).map_err(|_| Thrown)
}

pub(super) unsafe fn big_int(g: *mut c_void, v: i64) -> Js<u64> {
    let big = get(g, glue::rjs_global(g), "BigInt");
    call(g, big, UNDEF, &[js_string(g, &v.to_string())]).map_err(|_| Thrown)
}

/// The low 64 bits of a bigint, or of an integer number.
pub(super) unsafe fn int64_bits(g: *mut c_void, v: u64, what: &str) -> Js<u64> {
    if kind(g, v) == KIND_BIGINT {
        let big = get(g, glue::rjs_global(g), "BigInt");
        let wrapped =
            call(g, get(g, big, "asUintN"), big, &[enc_i32(64), v]).map_err(|_| Thrown)?;
        let hex =
            call(g, get(g, wrapped, "toString"), wrapped, &[enc_i32(16)]).map_err(|_| Thrown)?;
        let text = read_string(g, hex).unwrap_or_default();
        return Ok(u64::from_str_radix(&text, 16).unwrap_or(0));
    }
    let n = number(g, v)?;
    if n.fract() != 0.0 || n.abs() > 9_007_199_254_740_992.0 {
        return range_error(g, &format!("{what} must be an integer or a bigint"));
    }
    Ok(n as i64 as u64)
}

thread_local! {
    static PROTOTYPES: RefCell<HashMap<(usize, &'static str), u64>> = RefCell::new(HashMap::new());
}

/// `define_class` without the global: a std class is reached by its module only.
pub(super) unsafe fn class(g: *mut c_void, spec: &ClassSpec<'static>) -> Class {
    let class = define_class(g, spec);
    let name = atom(g, spec.name);
    glue::rjs_delete(g, glue::rjs_global(g), name);
    glue::rjs_atom_free(g, name);
    PROTOTYPES.with(|p| p.borrow_mut().insert((g as usize, spec.name), class.proto));
    class
}

pub(super) fn prototype(g: *mut c_void, name: &'static str) -> u64 {
    PROTOTYPES.with(|p| {
        p.borrow()
            .get(&(g as usize, name))
            .copied()
            .unwrap_or(UNDEF)
    })
}

thread_local! {
    static SLOTS: RefCell<HashMap<(usize, &'static str), usize>> = RefCell::new(HashMap::new());
}

unsafe fn slot(g: *mut c_void, name: &'static str) -> *mut c_void {
    SLOTS.with(|s| {
        *s.borrow_mut()
            .entry((g as usize, name))
            .or_insert_with(|| atom_priv(g, name) as usize)
    }) as *mut c_void
}

/// A JS value an instance keeps alive, out of reach of the app's code.
pub(super) unsafe fn slot_set(g: *mut c_void, object: u64, name: &'static str, v: u64) {
    glue::rjs_define(g, object, slot(g, name), v, 0);
}

pub(super) unsafe fn slot_get(g: *mut c_void, object: u64, name: &'static str) -> u64 {
    get_a(g, object, slot(g, name))
}

pub(super) unsafe fn static_fn(g: *mut c_void, object: u64, name: &str, arity: u32, f: HostFn) {
    define(g, object, name, make_fn(g, name, arity, f), 2);
}

/// A plain object of functions, as `Math` and `JSON` are.
pub(super) unsafe fn namespace(
    g: *mut c_void,
    tag: &str,
    functions: &[(&str, u32, HostFn)],
) -> u64 {
    let object = new_object(g, UNDEF);
    to_string_tag(g, object, tag);
    for (name, arity, f) in functions {
        static_fn(g, object, name, *arity, *f);
    }
    object
}

pub(super) fn host_class<T>(cell: &std::sync::OnceLock<usize>, name: &str) -> JSClassRef {
    *cell.get_or_init(|| unsafe { make_host_class::<T>(name) } as usize) as JSClassRef
}

/// A class whose instances carry `$ty`: its accessor, `$get`, and its maker, `$new`.
macro_rules! host_state {
    ($get:ident, $new:ident, $ty:ty, $cell:ident, $name:literal) => {
        static $cell: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

        unsafe fn $get<'a>(g: *mut c_void, this: u64) -> Js<&'a mut $ty> {
            match host_data::<$ty>(g, host_class::<$ty>(&$cell, $name), this) {
                Some(state) => Ok(state),
                None => type_error(g, concat!("this is not a ", $name)),
            }
        }

        unsafe fn $new(g: *mut c_void, state: $ty) -> u64 {
            host_create(
                g,
                host_class::<$ty>(&$cell, $name),
                prototype(g, $name),
                state,
            )
        }
    };
}
pub(super) use host_state;
