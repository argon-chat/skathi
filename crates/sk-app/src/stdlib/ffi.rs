//! `skathi:ffi`: C functions of the libraries sk.config.json allows, called from JS.
//!
//! One table of types serves every direction, so a value converts the same way wherever it goes.

use std::ffi::{c_void, CStr, CString};
use std::sync::OnceLock;

use sk_ffi::{Signature, Type};
use sk_runtime::host;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};
use crate::permissions::{self, Library};

const KIND_BOOL: u32 = 2;
const KIND_INT32: u32 = 3;
const KIND_DOUBLE: u32 = 4;
const TA_U8: u32 = 2;

/// The buffer a pointer was taken from, kept alive for as long as the pointer is.
const KEEP: &str = "ffi.keep";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Plain(Type),
    /// A typed array, a DataView or an ArrayBuffer: the address of its bytes for the call.
    Buffer,
    /// A JS string as NUL-terminated UTF-8, encoded for the call and copied on return.
    CString,
    /// The same as NUL-terminated UTF-16.
    WString,
}

const KINDS: &[(&str, Kind)] = &[
    ("void", Kind::Plain(Type::Void)),
    ("bool", Kind::Plain(Type::Bool)),
    ("i8", Kind::Plain(Type::I8)),
    ("u8", Kind::Plain(Type::U8)),
    ("i16", Kind::Plain(Type::I16)),
    ("u16", Kind::Plain(Type::U16)),
    ("i32", Kind::Plain(Type::I32)),
    ("u32", Kind::Plain(Type::U32)),
    ("i64", Kind::Plain(Type::I64)),
    ("u64", Kind::Plain(Type::U64)),
    ("isize", Kind::Plain(Type::Isize)),
    ("usize", Kind::Plain(Type::Usize)),
    ("f32", Kind::Plain(Type::F32)),
    ("f64", Kind::Plain(Type::F64)),
    ("pointer", Kind::Plain(Type::Pointer)),
    ("buffer", Kind::Buffer),
    ("cstring", Kind::CString),
    ("wstring", Kind::WString),
];

impl Kind {
    fn c(self) -> Type {
        match self {
            Kind::Plain(ty) => ty,
            _ => Type::Pointer,
        }
    }

    fn name(self) -> &'static str {
        KINDS
            .iter()
            .find(|(_, k)| *k == self)
            .map_or("?", |(n, _)| n)
    }
}

struct Function {
    name: String,
    address: usize,
    signature: Signature,
    args: Vec<Kind>,
    returns: Kind,
    errno: bool,
    /// `library!symbol`, for the line a crash inside the call prints.
    label: &'static CStr,
}

struct NativeLibrary {
    name: String,
    path: String,
    functions: Vec<Option<Function>>,
    closed: bool,
}

struct NativePointer {
    address: u64,
}

static LIBRARY: OnceLock<usize> = OnceLock::new();
static POINTER: OnceLock<usize> = OnceLock::new();

fn library_class() -> JSClassRef {
    *LIBRARY.get_or_init(|| unsafe { make_host_class::<NativeLibrary>("NativeLibrary") } as usize)
        as JSClassRef
}

fn pointer_class() -> JSClassRef {
    *POINTER.get_or_init(|| unsafe { make_host_class::<NativePointer>("NativePointer") } as usize)
        as JSClassRef
}

unsafe fn library<'a>(g: *mut c_void, this: u64) -> Js<&'a mut NativeLibrary> {
    match host_data::<NativeLibrary>(g, library_class(), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a NativeLibrary"),
    }
}

unsafe fn pointer<'a>(g: *mut c_void, this: u64) -> Js<&'a mut NativePointer> {
    match host_data::<NativePointer>(g, pointer_class(), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a NativePointer"),
    }
}

unsafe fn pointer_new(g: *mut c_void, address: u64) -> u64 {
    host_create(
        g,
        pointer_class(),
        prototype(g, "NativePointer"),
        NativePointer { address },
    )
}

unsafe fn dom<T>(g: *mut c_void, name: &str, message: &str) -> Js<T> {
    host::throw(g, name, message);
    Err(Thrown)
}

unsafe fn length(g: *mut c_void, array: u64) -> usize {
    to_number(g, get(g, array, "length")).unwrap_or(0.0) as usize
}

unsafe fn parse_kind(g: *mut c_void, v: u64, what: &str) -> Js<Kind> {
    let name = if kind(g, v) == KIND_STRING {
        read_string(g, v).unwrap_or_default()
    } else {
        String::new()
    };
    if let Some((_, found)) = KINDS.iter().find(|(n, _)| *n == name) {
        return Ok(*found);
    }
    if name == "callback" {
        return type_error(g, &format!("{what}: callbacks are not supported yet"));
    }
    if name.is_empty() || name.starts_with("struct") {
        return type_error(
            g,
            &format!("{what}: a struct cannot be passed or returned by value; pass a \"pointer\" or a \"buffer\" that holds it"),
        );
    }
    let known: Vec<&str> = KINDS.iter().map(|(n, _)| *n).collect();
    type_error(
        g,
        &format!(
            "{what}: \"{name}\" is not a type; use one of {}",
            known.join(", ")
        ),
    )
}

/// `[symbol, argument types, return type, errno, optional, named parameters of a variadic function]`.
unsafe fn parse_function(
    g: *mut c_void,
    def: u64,
    library: &str,
    loaded: &sk_ffi::Library,
) -> Js<Option<Function>> {
    let name = string(g, get_i(g, def, 0), "dlopen: a symbol's name")?;
    let what = format!("dlopen: {library}.{name}");
    let types = get_i(g, def, 1);
    let mut args = Vec::new();
    for i in 0..length(g, types) {
        let kind = parse_kind(g, get_i(g, types, i as u32), &what)?;
        if kind == Kind::Plain(Type::Void) {
            return type_error(g, &format!("{what}: \"void\" is not an argument type"));
        }
        args.push(kind);
    }
    let returns = parse_kind(g, get_i(g, def, 2), &what)?;
    if returns == Kind::Buffer {
        return type_error(
            g,
            &format!(
                "{what}: a \"buffer\" cannot be returned; return a \"pointer\" and read from it"
            ),
        );
    }
    let errno = to_bool(g, get_i(g, def, 3));
    let optional = to_bool(g, get_i(g, def, 4));
    let variadic = get_i(g, def, 5);
    let fixed = if variadic == UNDEF {
        None
    } else {
        let named = count(g, variadic, &format!("{what}: variadic"))?;
        if named > args.len() {
            return range_error(
                g,
                &format!(
                    "{what}: variadic is the number of named parameters, at most {}",
                    args.len()
                ),
            );
        }
        Some(named)
    };
    let Some(address) = loaded.symbol(&name) else {
        if optional {
            return Ok(None);
        }
        return dom(
            g,
            "NotFoundError",
            &format!("{what}: the library has no symbol \"{name}\""),
        );
    };
    let label = CString::new(format!("{library}!{name}")).unwrap_or_default();
    Ok(Some(Function {
        signature: Signature {
            args: args.iter().map(|k| k.c()).collect(),
            returns: returns.c(),
            fixed,
        },
        name,
        address: address as usize,
        args,
        returns,
        errno,
        label: Box::leak(label.into_boxed_c_str()),
    }))
}

std_fn!(fn ffi_open(g, args) {
    let name = string(g, args.arg(0).0, "dlopen: the library's name")?;
    if !cfg!(feature = "ffi") {
        return dom(g, "NotSupportedError", "dlopen: this host was built without skathi:ffi (the \"ffi\" feature)");
    }
    let located = match permissions::current().library(&name) {
        Ok(located) => located,
        Err(message) => return dom(g, "NotAllowedError", &format!("dlopen: {message}")),
    };
    let (path, loaded) = match &located {
        Library::File(path) => (path.display().to_string(), sk_ffi::Library::open(path)),
        Library::System(file) => (file.clone(), sk_ffi::Library::open_system(file)),
    };
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(error) => return dom(g, "NotFoundError", &format!("dlopen: {error}")),
    };
    let defs = args.arg(1).0;
    let mut functions = Vec::new();
    for i in 0..length(g, defs) {
        functions.push(parse_function(g, get_i(g, defs, i as u32), &name, &loaded)?);
    }
    let state = NativeLibrary { name, path, functions, closed: false };
    Ok(host_create(g, library_class(), prototype(g, "NativeLibrary"), state))
});

fn range(ty: Type) -> (i128, i128) {
    let bits = 8 * ty.size() as u32;
    match ty {
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Isize => {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        }
        _ => (0, (1i128 << bits) - 1),
    }
}

/// An integer of `ty` from a number or a bigint, by one rule whichever the engine holds it as.
unsafe fn integer(g: *mut c_void, v: u64, ty: Type, what: &str) -> Js<u64> {
    let name = Kind::Plain(ty).name();
    let value: i128 = match kind(g, v) {
        KIND_BIGINT => {
            let text = call(g, get(g, v, "toString"), v, &[]).map_err(|_| Thrown)?;
            match read_string(g, text).and_then(|t| t.parse().ok()) {
                Some(value) => value,
                None => return range_error(g, &format!("{what} does not fit {name}")),
            }
        }
        KIND_INT32 | KIND_DOUBLE => {
            let n = number(g, v)?;
            if n.fract() != 0.0 || !n.is_finite() {
                return type_error(g, &format!("{what} must be an integer, as it is {name}"));
            }
            if n.abs() > 9_007_199_254_740_991.0 {
                return range_error(
                    g,
                    &format!("{what} is beyond the integers a number holds exactly; pass a bigint"),
                );
            }
            n as i128
        }
        _ => {
            return type_error(
                g,
                &format!("{what} must be a number or a bigint, as it is {name}"),
            )
        }
    };
    let (min, max) = range(ty);
    if value < min || value > max {
        return range_error(g, &format!("{what}: {value} does not fit {name}"));
    }
    Ok(value as u64)
}

/// Strings encoded for one call.
#[derive(Default)]
struct Arena {
    narrow: Vec<Vec<u8>>,
    wide: Vec<Vec<u16>>,
}

/// The slot of every argument but a buffer, whose address is read last: converting another
/// argument may allocate, and an allocation may move a small typed array's bytes.
unsafe fn to_slot(g: *mut c_void, v: u64, kind_of: Kind, arena: &mut Arena, what: &str) -> Js<u64> {
    let nothing = v == NULL || v == UNDEF;
    match kind_of {
        Kind::Buffer => Ok(0),
        Kind::CString if nothing => Ok(0),
        Kind::CString => {
            let mut bytes = string(g, v, what)?.into_bytes();
            if bytes.contains(&0) {
                return type_error(g, &format!("{what} has a NUL character in it, where a C string ends"));
            }
            bytes.push(0);
            arena.narrow.push(bytes);
            Ok(arena.narrow.last().map_or(0, |b| b.as_ptr() as u64))
        }
        Kind::WString if nothing => Ok(0),
        Kind::WString => {
            if kind(g, v) != KIND_STRING {
                return type_error(g, &format!("{what} must be a string"));
            }
            let mut units = read_string16(g, v).ok_or(Thrown)?;
            if units.contains(&0) {
                return type_error(g, &format!("{what} has a NUL character in it, where a C string ends"));
            }
            units.push(0);
            arena.wide.push(units);
            Ok(arena.wide.last().map_or(0, |u| u.as_ptr() as u64))
        }
        Kind::Plain(Type::Bool) => {
            if kind(g, v) != KIND_BOOL {
                return type_error(g, &format!("{what} must be true or false"));
            }
            Ok(to_bool(g, v) as u64)
        }
        Kind::Plain(ty @ (Type::F32 | Type::F64)) => {
            if !matches!(kind(g, v), KIND_INT32 | KIND_DOUBLE) {
                return type_error(g, &format!("{what} must be a number"));
            }
            let n = number(g, v)?;
            Ok(if ty == Type::F32 { (n as f32).to_bits() as u64 } else { n.to_bits() })
        }
        Kind::Plain(Type::Pointer) if nothing => Ok(0),
        Kind::Plain(Type::Pointer) => match host_data::<NativePointer>(g, pointer_class(), v) {
            Some(state) => Ok(state.address),
            None => type_error(g, &format!("{what} must be a NativePointer or null; a buffer goes to a \"buffer\" parameter")),
        },
        Kind::Plain(Type::Void) => Ok(0),
        Kind::Plain(ty) => integer(g, v, ty, what),
    }
}

unsafe fn buffer_address(g: *mut c_void, v: u64, what: &str) -> Js<u64> {
    if v == NULL || v == UNDEF {
        return Ok(0);
    }
    match buffer_source(g, v) {
        Some(view) if view.len > 0 => Ok(view.data as u64),
        Some(_) => type_error(
            g,
            &format!("{what} is empty or detached; pass null for no buffer"),
        ),
        None => type_error(
            g,
            &format!("{what} must be an ArrayBuffer, a typed array, a DataView or null"),
        ),
    }
}

unsafe fn c_string(g: *mut c_void, address: u64, limit: Option<usize>) -> u64 {
    let start = address as *const u8;
    let mut len = 0;
    while limit.is_none_or(|l| len < l) && *start.add(len) != 0 {
        len += 1;
    }
    let bytes = std::slice::from_raw_parts(start, len);
    js_string(g, &String::from_utf8_lossy(bytes))
}

unsafe fn w_string(g: *mut c_void, address: u64, limit: Option<usize>) -> u64 {
    let start = address as *const u16;
    let mut len = 0;
    while limit.is_none_or(|l| len < l) && start.add(len).read_unaligned() != 0 {
        len += 1;
    }
    let units: Vec<u16> = (0..len).map(|i| start.add(i).read_unaligned()).collect();
    js_string16(g, &units)
}

unsafe fn from_slot(g: *mut c_void, value: u64, kind_of: Kind) -> Js<u64> {
    Ok(match kind_of {
        Kind::Buffer | Kind::Plain(Type::Void) => UNDEF,
        Kind::CString if value == 0 => NULL,
        Kind::CString => c_string(g, value, None),
        Kind::WString if value == 0 => NULL,
        Kind::WString => w_string(g, value, None),
        Kind::Plain(Type::Bool) => enc_bool(value != 0),
        Kind::Plain(Type::F32) => enc_f64(f32::from_bits(value as u32) as f64),
        Kind::Plain(Type::F64) => enc_f64(f64::from_bits(value)),
        Kind::Plain(Type::Pointer) => pointer_new(g, value),
        Kind::Plain(Type::I64 | Type::Isize) => big_int(g, value as i64)?,
        Kind::Plain(Type::U64 | Type::Usize) => big_uint(g, value)?,
        Kind::Plain(Type::I8 | Type::I16 | Type::I32) => enc_f64(value as i64 as f64),
        Kind::Plain(_) => enc_f64(value as f64),
    })
}

std_fn!(fn library_call(g, args) {
    let state = library(g, args.this().0)?;
    let index = count(g, args.arg(0).0, "the function's index")?;
    let Some(function) = state.functions.get(index).and_then(Option::as_ref) else {
        return type_error(g, "the library has no such function");
    };
    let what = format!("{}.{}", state.name, function.name);
    if state.closed {
        return dom(g, "InvalidStateError", &format!("{what}: the library is closed"));
    }
    let given = args.argc().saturating_sub(1) as usize;
    if given != function.args.len() {
        return type_error(g, &format!("{what} takes {} argument(s), {given} given", function.args.len()));
    }
    let mut arena = Arena::default();
    let mut slots = Vec::with_capacity(given);
    for (i, kind_of) in function.args.iter().enumerate() {
        let what = format!("{what}: argument {} ({})", i + 1, kind_of.name());
        slots.push(to_slot(g, args.arg(i as u32 + 1).0, *kind_of, &mut arena, &what)?);
    }
    for (i, kind_of) in function.args.iter().enumerate() {
        if *kind_of == Kind::Buffer {
            let what = format!("{what}: argument {} (buffer)", i + 1);
            slots[i] = buffer_address(g, args.arg(i as u32 + 1).0, &what)?;
        }
    }
    let outcome = sk_ffi::in_call(function.label, || {
        sk_ffi::call(function.address as *const (), &function.signature, &slots)
    });
    drop(arena);
    let value = from_slot(g, outcome.value, function.returns)?;
    if !function.errno {
        return Ok(value);
    }
    let result = new_object(g, UNDEF);
    define(g, result, "value", value, 0);
    define(g, result, "errno", enc_f64(outcome.os_error as f64), 0);
    Ok(result)
});

std_fn!(fn library_has(g, args) {
    let state = library(g, args.this().0)?;
    let index = count(g, args.arg(0).0, "the function's index")?;
    Ok(enc_bool(state.functions.get(index).is_some_and(Option::is_some)))
});

std_fn!(fn library_close(g, args) {
    library(g, args.this().0)?.closed = true;
    Ok(UNDEF)
});

std_fn!(fn library_path(g, args) { Ok(js_string(g, &library(g, args.this().0)?.path)) });
std_fn!(fn library_closed(g, args) { Ok(enc_bool(library(g, args.this().0)?.closed)) });

std_fn!(fn pointer_of(g, args) {
    let source = args.arg(0).0;
    if typed_view(g, source).is_some() {
        // Reading `buffer` moves a small typed array's bytes out of the collector's heap for good.
        let _ = get(g, source, "buffer");
    }
    let Some(view) = buffer_source(g, source).filter(|view| view.len > 0) else {
        return type_error(g, "NativePointer.of: the source must be an ArrayBuffer, a typed array or a DataView that is not empty or detached");
    };
    let offset = if args.arg(1).0 == UNDEF { 0 } else { count(g, args.arg(1).0, "NativePointer.of: the offset")? };
    if offset > view.len {
        return range_error(g, &format!("NativePointer.of: the offset {offset} is beyond the {} bytes of the source", view.len));
    }
    let object = pointer_new(g, view.data as u64 + offset as u64);
    slot_set(g, object, KEEP, source);
    Ok(object)
});

std_fn!(fn pointer_from_address(g, args) {
    if !permissions::current().ffi_address {
        return dom(g, "NotAllowedError", "NativePointer.fromAddress: making a pointer from an address needs \"permissions\": { \"ffiAddress\": true } in sk.config.json");
    }
    let address = integer(g, args.arg(0).0, Type::Usize, "NativePointer.fromAddress: the address")?;
    Ok(pointer_new(g, address))
});

std_fn!(fn pointer_address(g, args) { big_uint(g, pointer(g, args.this().0)?.address) });
std_fn!(fn pointer_is_null(g, args) { Ok(enc_bool(pointer(g, args.this().0)?.address == 0)) });

std_fn!(fn pointer_offset(g, args) {
    let this = args.this().0;
    let address = pointer(g, this)?.address;
    let by = number(g, args.arg(0).0)?;
    if by.fract() != 0.0 || by.abs() > 9_007_199_254_740_991.0 {
        return range_error(g, "NativePointer.offset: the offset must be an integer");
    }
    let object = pointer_new(g, address.wrapping_add(by as i64 as u64));
    slot_set(g, object, KEEP, slot_get(g, this, KEEP));
    Ok(object)
});

std_fn!(fn pointer_equals(g, args) {
    let address = pointer(g, args.this().0)?.address;
    let other = host_data::<NativePointer>(g, pointer_class(), args.arg(0).0);
    Ok(enc_bool(other.is_some_and(|other| other.address == address)))
});

unsafe fn readable(g: *mut c_void, this: u64, what: &str) -> Js<u64> {
    let address = pointer(g, this)?.address;
    if address == 0 {
        return type_error(g, &format!("{what}: the pointer is null"));
    }
    Ok(address)
}

unsafe fn limit(g: *mut c_void, v: u64, what: &str) -> Js<Option<usize>> {
    if v == UNDEF {
        Ok(None)
    } else {
        count(g, v, what).map(Some)
    }
}

std_fn!(fn pointer_read_c_string(g, args) {
    let address = readable(g, args.this().0, "NativePointer.readCString")?;
    let limit = limit(g, args.arg(0).0, "NativePointer.readCString: the most bytes to read")?;
    Ok(c_string(g, address, limit))
});

std_fn!(fn pointer_read_w_string(g, args) {
    let address = readable(g, args.this().0, "NativePointer.readWString")?;
    let limit = limit(g, args.arg(0).0, "NativePointer.readWString: the most code units to read")?;
    Ok(w_string(g, address, limit))
});

std_fn!(fn pointer_read(g, args) {
    let address = readable(g, args.this().0, "NativePointer.read")?;
    let len = count(g, args.arg(0).0, "NativePointer.read: the length")?;
    let (array, out) = typed_array(g, TA_U8, len)?;
    std::ptr::copy_nonoverlapping(address as *const u8, out.as_mut_ptr(), len);
    Ok(array)
});

std_fn!(fn pointer_write(g, args) {
    let address = readable(g, args.this().0, "NativePointer.write")?;
    let bytes = bytes_arg(g, args.arg(0).0, "NativePointer.write: the bytes")?;
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
    Ok(UNDEF)
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let module = new_object(g, UNDEF);
    static_fn(g, module, "open", 2, ffi_open);

    support::class(
        g,
        &ClassSpec {
            name: "NativeLibrary",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("call", 1, library_call),
                ("has", 1, library_has),
                ("close", 0, library_close),
            ],
            getters: &[("path", library_path), ("closed", library_closed)],
            accessors: &[],
        },
    );

    let pointer = support::class(
        g,
        &ClassSpec {
            name: "NativePointer",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("isNull", 0, pointer_is_null),
                ("offset", 1, pointer_offset),
                ("equals", 1, pointer_equals),
                ("readCString", 0, pointer_read_c_string),
                ("readWString", 0, pointer_read_w_string),
                ("read", 1, pointer_read),
                ("write", 1, pointer_write),
            ],
            getters: &[("address", pointer_address)],
            accessors: &[],
        },
    );
    static_fn(g, pointer.ctor, "of", 1, pointer_of);
    static_fn(g, pointer.ctor, "fromAddress", 1, pointer_from_address);
    define(g, pointer.ctor, "NULL", pointer_new(g, 0), 0);
    define(g, module, "NativePointer", pointer.ctor, 0);

    define(g, std, "ffi", module, 0);
}
