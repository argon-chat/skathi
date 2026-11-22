//! `skathi:ffi`: C functions of the libraries sk.config.json allows, called from JS.
//!
//! One table of types serves every direction, so a value converts the same way wherever it goes.

use std::cell::Cell;
use std::collections::VecDeque;
use std::ffi::{c_void, CStr, CString};
use std::fmt::{self, Display};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::ThreadId;

use sk_ffi::{Argument, Returning, Signature, StructLayout, Type};
use sk_runtime::host;
use sk_runtime::redjs::capi::{self, JSClassRef};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::memory::{self, Region};
use super::support::{self, *};
use crate::permissions::{self, Library};

const KIND_BOOL: u32 = 2;
const KIND_INT32: u32 = 3;
const KIND_DOUBLE: u32 = 4;
const KIND_FUNCTION: u32 = 9;
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
    /// A struct by value, of the layout the function keeps beside the kind.
    Struct,
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
            .map_or("struct", |(n, _)| n)
    }
}

struct Function {
    name: String,
    address: usize,
    signature: Signature,
    args: Vec<Kind>,
    returns: Kind,
    /// The layout of every argument that is a struct.
    structs: Vec<Option<Arc<StructDef>>>,
    returned: Option<Arc<StructDef>>,
    /// Some argument or the return is a struct.
    by_value: bool,
    errno: bool,
    /// The call runs on a worker thread and answers with a promise.
    nonblocking: bool,
    /// Some argument is a buffer.
    buffers: bool,
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
    /// The memory of the `NativeBuffer` the pointer was taken from, kept even after `free()`.
    keep: Option<Region>,
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
    pointer_kept(g, address, None)
}

unsafe fn pointer_kept(g: *mut c_void, address: u64, keep: Option<Region>) -> u64 {
    host_create(
        g,
        pointer_class(),
        prototype(g, "NativePointer"),
        NativePointer { address, keep },
    )
}

unsafe fn dom<T>(g: *mut c_void, name: &str, message: &str) -> Js<T> {
    host::throw(g, name, message);
    Err(Thrown)
}

unsafe fn length(g: *mut c_void, array: u64) -> usize {
    to_number(g, get(g, array, "length")).unwrap_or(0.0) as usize
}

unsafe fn parse_type(g: *mut c_void, v: u64, what: &str) -> Js<(Kind, Option<Arc<StructDef>>)> {
    if let Some(of) = host_data::<StructType>(g, struct_class(), v) {
        return Ok((Kind::Struct, Some(of.def.clone())));
    }
    Ok((parse_kind(g, v, what)?, None))
}

unsafe fn parse_kind(g: *mut c_void, v: u64, what: &str) -> Js<Kind> {
    if host_data::<StructType>(g, struct_class(), v).is_some() {
        return type_error(g, &format!("{what}: a callback takes and returns no struct by value; a struct arrives as a \"pointer\""));
    }
    let name = if kind(g, v) == KIND_STRING {
        read_string(g, v).unwrap_or_default()
    } else {
        String::new()
    };
    if let Some((_, found)) = KINDS.iter().find(|(n, _)| *n == name) {
        return Ok(*found);
    }
    if name == "callback" {
        return type_error(g, &format!("{what}: \"callback\" is not a type; make a NativeCallback and pass its pointer to a \"pointer\" parameter"));
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
    let mut structs = Vec::new();
    for i in 0..length(g, types) {
        let (kind, of) = parse_type(g, get_i(g, types, i as u32), &what)?;
        structs.push(of);
        if kind == Kind::Plain(Type::Void) {
            return type_error(g, &format!("{what}: \"void\" is not an argument type"));
        }
        args.push(kind);
    }
    let (returns, returned) = parse_type(g, get_i(g, def, 2), &what)?;
    let by_value = returned.is_some() || structs.iter().any(Option::is_some);
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
    let nonblocking = to_bool(g, get_i(g, def, 6));
    let buffers = args.contains(&Kind::Buffer);
    let fixed = if variadic == UNDEF {
        None
    } else if by_value {
        return type_error(g, &format!("{what}: a variadic function takes and returns no struct by value; pass a \"pointer\" to the struct"));
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
        structs,
        returned,
        by_value,
        errno,
        nonblocking,
        buffers,
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
unsafe fn integer(g: *mut c_void, v: u64, ty: Type, what: &dyn Display) -> Js<u64> {
    if let Some(n) = dec_f64(v) {
        let whole = n as i64;
        let (min, max) = range(ty);
        if whole as f64 == n
            && n.abs() <= 9_007_199_254_740_991.0
            && (min..=max).contains(&(whole as i128))
        {
            return Ok(whole as u64);
        }
    }
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
unsafe fn to_slot(
    g: *mut c_void,
    v: u64,
    kind_of: Kind,
    arena: &mut Arena,
    what: &dyn Display,
) -> Js<u64> {
    let nothing = v == NULL || v == UNDEF;
    match kind_of {
        Kind::Buffer | Kind::Struct => Ok(0),
        Kind::CString if nothing => Ok(0),
        Kind::CString => {
            if kind(g, v) != KIND_STRING {
                return type_error(g, &format!("{what} must be a string"));
            }
            let mut bytes = read_string(g, v).ok_or(Thrown)?.into_bytes();
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
            let Some(n) = dec_f64(v) else {
                return type_error(g, &format!("{what} must be a number"));
            };
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

/// The address of a buffer argument. The memory of a `NativeBuffer`, given itself or under a view,
/// goes into `held`, so that a callback that frees it during the call does not free it under the
/// library.
unsafe fn buffer_address(
    g: *mut c_void,
    v: u64,
    what: &dyn Display,
    held: &mut Vec<Region>,
) -> Js<u64> {
    if v == NULL || v == UNDEF {
        return Ok(0);
    }
    let (len, address) = match buffer_source(g, v) {
        Some(view) => {
            held.extend(memory::holder_of(view.data));
            (view.len, view.data as u64)
        }
        None => match memory::region_of(g, v, what)? {
            Some(region) => {
                let found = (region.len(), region.ptr() as u64);
                held.push(region);
                found
            }
            None => {
                return type_error(
                    g,
                    &format!("{what} must be an ArrayBuffer, a typed array, a DataView, a NativeBuffer or null"),
                )
            }
        },
    };
    if len == 0 {
        return type_error(
            g,
            &format!("{what} is empty or detached; pass null for no buffer"),
        );
    }
    Ok(address)
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
        Kind::Buffer | Kind::Struct | Kind::Plain(Type::Void) => UNDEF,
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

/// A call handed to a worker thread. It owns everything the library is given: the strings and a
/// copy of every buffer, so that nothing the collector can move or free is in another thread's hands.
struct Job {
    address: usize,
    signature: Signature,
    slots: Vec<u64>,
    structs: Vec<Packed>,
    label: &'static CStr,
    done: Done,
    _arena: Arena,
}

/// What the JS thread needs once the call has returned.
struct Done {
    g: usize,
    settler: host::Settler,
    returns: Kind,
    returned: Packed,
    errno: bool,
    /// A buffer argument and the copy the library worked on, which is written back to it.
    buffers: Vec<(u64, Vec<u8>)>,
    /// A `NativeBuffer` argument, worked on in place, and its memory, held until the call returns.
    lent: Vec<(u64, Region)>,
}

// SAFETY: the engine's values in `Done` are touched on the JS thread only; the worker reads the
// address, the signature and the slots, which point into memory the job owns or the caller vouched for.
unsafe impl Send for Job {}
unsafe impl Send for Done {}

const WORKERS: usize = 4;
/// Calls waiting or running; one more is refused.
const IN_FLIGHT_LIMIT: usize = 256;

#[derive(Default)]
struct Pool {
    waiting: VecDeque<Job>,
    threads: usize,
    idle: usize,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);
static WORK: Condvar = Condvar::new();
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static FINISHED: Mutex<Vec<(Done, sk_ffi::Outcome)>> = Mutex::new(Vec::new());

fn work() {
    loop {
        let mut job = {
            let mut guard = POOL.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let pool = guard.get_or_insert_with(Pool::default);
                if let Some(job) = pool.waiting.pop_front() {
                    break job;
                }
                pool.idle += 1;
                guard = WORK.wait(guard).unwrap_or_else(|e| e.into_inner());
                if let Some(pool) = guard.as_mut() {
                    pool.idle -= 1;
                }
            }
        };
        let outcome = sk_ffi::in_call(job.label, || unsafe {
            if job.structs.is_empty() && job.done.returned.is_none() {
                sk_ffi::call(job.address as *const (), &job.signature, &job.slots)
            } else {
                perform(
                    job.address,
                    &job.signature,
                    &job.slots,
                    &job.structs,
                    &mut job.done.returned,
                )
            }
        });
        FINISHED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((job.done, outcome));
        sk_runtime::frames::wake();
        crate::background::prompt();
    }
}

fn submit(job: Job) {
    let mut guard = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let pool = guard.get_or_insert_with(Pool::default);
    pool.waiting.push_back(job);
    if pool.idle == 0 && pool.threads < WORKERS {
        let spawned = std::thread::Builder::new()
            .name("skathi-ffi".into())
            .spawn(work);
        if spawned.is_ok() {
            pool.threads += 1;
        }
    }
    drop(guard);
    WORK.notify_one();
}

/// A resizable buffer can shrink under the copy that is written back.
unsafe fn resizable(g: *mut c_void, v: u64) -> bool {
    let buffer = if typed_view(g, v).is_some() {
        get(g, v, "buffer")
    } else {
        v
    };
    to_bool(g, get(g, buffer, "resizable")) || to_bool(g, get(g, buffer, "growable"))
}

unsafe fn call_on_worker(
    g: *mut c_void,
    function: &Function,
    values: &[u64],
    mut slots: Vec<u64>,
    structs: Vec<Packed>,
    arena: Arena,
    what: &str,
) -> Js<u64> {
    let mut buffers = Vec::new();
    let mut natives = Vec::new();
    for (i, kind_of) in function.args.iter().enumerate() {
        let v = values[i];
        if *kind_of != Kind::Buffer || v == NULL || v == UNDEF {
            continue;
        }
        let what = format!("{what}: argument {} (buffer)", i + 1);
        let mut held = Vec::new();
        buffer_address(g, v, &what, &mut held)?;
        if !held.is_empty() {
            natives.push((i, v, what));
            continue;
        }
        if resizable(g, v) {
            return type_error(g, &format!("{what} is resizable, which a call on another thread cannot take; pass a buffer of a fixed length"));
        }
        let copy = bytes_arg(g, v, &what)?.to_vec();
        buffers.push((i, v, copy));
    }
    if IN_FLIGHT.load(Ordering::Acquire) >= IN_FLIGHT_LIMIT {
        return dom(
            g,
            "QuotaExceededError",
            &format!(
                "{what}: {IN_FLIGHT_LIMIT} calls on other threads are already waiting or running"
            ),
        );
    }
    let mut lent: Vec<(u64, Region)> = Vec::new();
    for (i, v, what) in &natives {
        let region = match lent.iter().find(|(given, _)| given == v) {
            Some((_, region)) => region.clone(),
            None => match memory::send_away(g, *v, what) {
                Ok(region) => {
                    capi::JSValueProtect(g as _, *v as usize as _);
                    lent.push((*v, region.clone()));
                    region
                }
                Err(Thrown) => {
                    for (v, _) in &lent {
                        memory::come_back(g, *v);
                        capi::JSValueUnprotect(g as _, *v as usize as _);
                    }
                    return Err(Thrown);
                }
            },
        };
        slots[*i] = region.ptr() as u64;
    }
    IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    for (i, v, copy) in &mut buffers {
        slots[*i] = copy.as_mut_ptr() as u64;
        capi::JSValueProtect(g as _, *v as usize as _);
    }
    let (promise, settler) = host::deferred(g);
    submit(Job {
        address: function.address,
        signature: function.signature.clone(),
        slots,
        structs,
        label: function.label,
        done: Done {
            g: g as usize,
            settler,
            returns: function.returns,
            returned: function
                .returned
                .as_ref()
                .map(|def| (def.clone(), def.room())),
            errno: function.errno,
            buffers: buffers.into_iter().map(|(_, v, copy)| (v, copy)).collect(),
            lent,
        },
        _arena: arena,
    });
    Ok(promise)
}

unsafe fn finish(done: Done, outcome: sk_ffi::Outcome) {
    let g = done.g as *mut c_void;
    for (v, copy) in &done.buffers {
        if let Some(view) = buffer_source(g, *v).filter(|view| view.len == copy.len()) {
            view.bytes_mut().copy_from_slice(copy);
        }
        capi::JSValueUnprotect(g as _, *v as usize as _);
    }
    for (v, _) in &done.lent {
        memory::come_back(g, *v);
        capi::JSValueUnprotect(g as _, *v as usize as _);
    }
    let value = match returned_value(g, outcome.value, done.returns, &done.returned) {
        Ok(value) if done.errno => with_errno(g, value, outcome.os_error),
        Ok(value) => value,
        Err(Thrown) => {
            let mut error = UNDEF;
            glue::rjs_exception_pending(g, &mut error);
            return done.settler.reject(g, error);
        }
    };
    done.settler.resolve(g, value);
}

unsafe fn with_errno(g: *mut c_void, value: u64, os_error: i32) -> u64 {
    let result = new_object(g, UNDEF);
    define(g, result, "value", value, 0);
    define(g, result, "errno", enc_f64(os_error as f64), 0);
    result
}

/// Arguments converted on the stack; a call with more takes a `Vec`.
const INLINE_SLOTS: usize = 12;

/// `library.function`, or one of its arguments, for an error message that is rarely written.
#[derive(Clone, Copy)]
struct Site<'a> {
    library: &'a str,
    function: &'a str,
    arg: Option<(usize, Kind)>,
}

impl Display for Site<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.library, self.function)?;
        match self.arg {
            Some((index, kind_of)) => write!(f, ": argument {} ({})", index + 1, kind_of.name()),
            None => Ok(()),
        }
    }
}

std_fn!(fn library_call(g, args) {
    let state = library(g, args.this().0)?;
    let first = args.arg(0).0;
    let index = if first & TAG_NUMBER == TAG_NUMBER {
        first as u32 as usize
    } else {
        count(g, first, "the function's index")?
    };
    let Some(function) = state.functions.get(index).and_then(Option::as_ref) else {
        return type_error(g, "the library has no such function");
    };
    let what = Site { library: &state.name, function: &function.name, arg: None };
    if state.closed {
        return dom(g, "InvalidStateError", &format!("{what}: the library is closed"));
    }
    let given = args.argc().saturating_sub(1) as usize;
    if given != function.args.len() {
        return type_error(g, &format!("{what} takes {} argument(s), {given} given", function.args.len()));
    }
    let mut arena = Arena::default();
    let mut inline = [0u64; INLINE_SLOTS];
    let mut spill = Vec::new();
    let slots: &mut [u64] = if given <= INLINE_SLOTS {
        &mut inline[..given]
    } else {
        spill.resize(given, 0);
        &mut spill
    };
    for (i, kind_of) in function.args.iter().enumerate() {
        let what = Site { arg: Some((i, *kind_of)), ..what };
        slots[i] = to_slot(g, args.arg(i as u32 + 1).0, *kind_of, &mut arena, &what)?;
    }
    let mut structs: Vec<Packed> = Vec::new();
    if function.by_value {
        for (i, def) in function.structs.iter().enumerate() {
            let what = Site { arg: Some((i, Kind::Struct)), ..what };
            structs.push(match def {
                Some(def) => Some((def.clone(), fill(g, def, args.arg(i as u32 + 1).0, &what)?)),
                None => None,
            });
        }
    }
    if function.nonblocking {
        let values: Vec<u64> = (0..given).map(|i| args.arg(i as u32 + 1).0).collect();
        return call_on_worker(g, function, &values, slots.to_vec(), structs, arena, &what.to_string());
    }
    let mut held = Vec::new();
    if function.buffers {
        for (i, kind_of) in function.args.iter().enumerate() {
            if *kind_of == Kind::Buffer {
                let what = Site { arg: Some((i, *kind_of)), ..what };
                slots[i] = buffer_address(g, args.arg(i as u32 + 1).0, &what, &mut held)?;
            }
        }
    }
    let entered = ENTERED.get();
    DEPTH.set(DEPTH.get() + 1);
    let mut returned: Packed = function.returned.as_ref().map(|def| (def.clone(), def.room()));
    let outcome = sk_ffi::in_call(function.label, || {
        if function.by_value {
            perform(function.address, &function.signature, slots, &structs, &mut returned)
        } else {
            sk_ffi::call(function.address as *const (), &function.signature, slots)
        }
    });
    DEPTH.set(DEPTH.get() - 1);
    drop(arena);
    drop(held);
    // Only a callback that ran during the call can have left an exception.
    if ENTERED.get() != entered {
        let mut thrown = UNDEF;
        if glue::rjs_exception_pending(g, &mut thrown) != 0 {
            glue::rjs_throw(g, thrown);
            return Err(Thrown);
        }
    }
    let value = returned_value(g, outcome.value, function.returns, &returned)?;
    if !function.errno {
        return Ok(value);
    }
    Ok(with_errno(g, value, outcome.os_error))
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
    let at = args.arg(1).0;
    if memory::region_of(g, source, &"NativePointer.of: the source")?.is_some() {
        let offset = if at == UNDEF { 0 } else { count(g, at, "NativePointer.of: the offset")? };
        // Again: reading the offset may have run code that freed the buffer.
        let Some(region) = memory::region_of(g, source, &"NativePointer.of: the source")?.filter(|r| r.len() > 0) else {
            return type_error(g, "NativePointer.of: the NativeBuffer is empty");
        };
        if offset > region.len() {
            return range_error(g, &format!("NativePointer.of: the offset {offset} is beyond the {} bytes of the source", region.len()));
        }
        let address = region.ptr() as u64 + offset as u64;
        return Ok(pointer_kept(g, address, Some(region)));
    }
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
    let address = view.data as u64 + offset as u64;
    if let Some(region) = memory::holder_of(view.data) {
        return Ok(pointer_kept(g, address, Some(region)));
    }
    let object = pointer_new(g, address);
    slot_set(g, object, KEEP, source);
    Ok(object)
});

std_fn!(fn pointer_from_address(g, args) {
    if !permissions::current().ffi_address {
        return dom(g, "NotAllowedError", "NativePointer.fromAddress: making a pointer from an address needs \"permissions\": { \"ffiAddress\": true } in sk.config.json");
    }
    let address = integer(g, args.arg(0).0, Type::Usize, &"NativePointer.fromAddress: the address")?;
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
    let keep = pointer(g, this)?.keep.clone();
    let object = pointer_kept(g, address.wrapping_add(by as i64 as u64), keep);
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

/// What a thunk needs to reach its JS function.
struct Target {
    g: usize,
    function: u64,
    args: Vec<Kind>,
    returns: Kind,
    thread: ThreadId,
    what: String,
    closed: AtomicBool,
    warned: AtomicBool,
}

// SAFETY: `g` and `function` are used on `thread` only; another thread reads the plain fields.
unsafe impl Send for Target {}
unsafe impl Sync for Target {}

/// An argument as another thread hands it over: text is copied there, since its memory may be
/// gone by the time the JS thread looks.
enum Carried {
    Slot(u64),
    Narrow(Vec<u8>),
    Wide(Vec<u16>),
}

const QUEUE_LIMIT: usize = 4096;

static QUEUE: Mutex<Vec<(Arc<Target>, Vec<Carried>)>> = Mutex::new(Vec::new());

thread_local! {
    /// Calls into libraries in progress on this thread: an exception a callback throws waits
    /// for the innermost one to return.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Callbacks entered on this thread so far.
    static ENTERED: Cell<u64> = const { Cell::new(0) };
}

unsafe fn describe(g: *mut c_void, error: u64) -> String {
    let string = get(g, host::global(g), "String");
    call(g, string, UNDEF, &[error])
        .ok()
        .and_then(|text| read_string(g, text))
        .unwrap_or_else(|| "an exception".into())
}

/// With a call into a library below us, the exception is thrown from that call once it returns.
/// Without one there is no JS to throw it to.
unsafe fn raise(g: *mut c_void, target: &Target, error: u64) {
    if DEPTH.get() > 0 {
        glue::rjs_throw(g, error);
    } else {
        eprintln!("[ffi] {}: {}", target.what, describe(g, error));
    }
}

unsafe fn invoke(target: &Target, slots: &[u64]) -> u64 {
    let g = target.g as *mut c_void;
    ENTERED.set(ENTERED.get().wrapping_add(1));
    let mut pending = UNDEF;
    if glue::rjs_exception_pending(g, &mut pending) != 0 {
        glue::rjs_throw(g, pending);
        return 0;
    }
    // In an array, so that the collector sees the values made so far while the next is made.
    let list = new_array(g);
    for (i, (slot, kind_of)) in slots.iter().zip(&target.args).enumerate() {
        match from_slot(g, *slot, *kind_of) {
            Ok(value) => set_i(g, list, i as u32, value),
            Err(Thrown) => return settle(g, target),
        }
    }
    let args: Vec<u64> = (0..slots.len()).map(|i| get_i(g, list, i as u32)).collect();
    let value = match call(g, target.function, UNDEF, &args) {
        Ok(value) => value,
        Err(error) => {
            raise(g, target, error);
            return 0;
        }
    };
    let what = format!("{}: the value it returns", target.what);
    match to_slot(g, value, target.returns, &mut Arena::default(), &what) {
        Ok(slot) => slot,
        Err(Thrown) => settle(g, target),
    }
}

/// A conversion left its exception pending in the VM.
unsafe fn settle(g: *mut c_void, target: &Target) -> u64 {
    let mut pending = UNDEF;
    if glue::rjs_exception_pending(g, &mut pending) != 0 {
        raise(g, target, pending);
    }
    0
}

unsafe fn carry(slot: u64, kind_of: Kind) -> Carried {
    match kind_of {
        Kind::CString if slot != 0 => Carried::Narrow(
            CStr::from_ptr(slot as *const _)
                .to_bytes_with_nul()
                .to_vec(),
        ),
        Kind::WString if slot != 0 => {
            let start = slot as *const u16;
            let mut units = Vec::new();
            loop {
                let unit = start.add(units.len()).read_unaligned();
                units.push(unit);
                if unit == 0 {
                    break;
                }
            }
            Carried::Wide(units)
        }
        _ => Carried::Slot(slot),
    }
}

/// On a thread that is not the JS thread: nothing of the engine is touched here.
unsafe fn from_another_thread(target: &Arc<Target>, slots: &[u64]) -> u64 {
    let refuse = |reason: &str| {
        if !target.warned.swap(true, Ordering::Relaxed) {
            eprintln!("[ffi] {}: {reason}", target.what);
        }
        0
    };
    if target.returns != Kind::Plain(Type::Void) {
        return refuse("called from another thread, where only a callback that returns \"void\" can run; 0 was returned");
    }
    let carried = slots
        .iter()
        .zip(&target.args)
        .map(|(slot, kind_of)| carry(*slot, *kind_of))
        .collect();
    let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    if queue.len() >= QUEUE_LIMIT {
        drop(queue);
        return refuse("more calls from another thread are waiting than the JS thread takes; calls are being dropped");
    }
    queue.push((target.clone(), carried));
    drop(queue);
    sk_runtime::frames::wake();
    crate::background::prompt();
    0
}

/// Runs the calls other threads made since the last frame.
pub(crate) fn pump() {
    let finished = std::mem::take(&mut *FINISHED.lock().unwrap_or_else(|e| e.into_inner()));
    for (done, outcome) in finished {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
        unsafe { finish(done, outcome) };
    }
    let waiting = std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()));
    for (target, carried) in waiting {
        if target.closed.load(Ordering::Acquire) || target.thread != std::thread::current().id() {
            continue;
        }
        let slots: Vec<u64> = carried
            .iter()
            .map(|c| match c {
                Carried::Slot(slot) => *slot,
                Carried::Narrow(bytes) => bytes.as_ptr() as u64,
                Carried::Wide(units) => units.as_ptr() as u64,
            })
            .collect();
        unsafe { invoke(&target, &slots) };
    }
}

struct NativeCallback {
    callback: Option<sk_ffi::Callback>,
    address: u64,
    target: Arc<Target>,
}

static CALLBACK: OnceLock<usize> = OnceLock::new();

fn callback_class() -> JSClassRef {
    *CALLBACK
        .get_or_init(|| unsafe { make_host_class::<NativeCallback>("NativeCallback") } as usize)
        as JSClassRef
}

unsafe fn callback<'a>(g: *mut c_void, this: u64) -> Js<&'a mut NativeCallback> {
    match host_data::<NativeCallback>(g, callback_class(), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a NativeCallback"),
    }
}

std_fn!(fn callback_construct(g, args) {
    let function = args.arg(0).0;
    if kind(g, function) != KIND_FUNCTION {
        return type_error(g, "NativeCallback: the first argument must be a function");
    }
    if !cfg!(feature = "ffi") {
        return dom(g, "NotSupportedError", "NativeCallback: this host was built without skathi:ffi (the \"ffi\" feature)");
    }
    let what = "NativeCallback";
    let signature = args.arg(1).0;
    let types = option(g, signature, "args");
    let mut kinds = Vec::new();
    for i in 0..if types == UNDEF { 0 } else { length(g, types) } {
        let kind_of = parse_kind(g, get_i(g, types, i as u32), what)?;
        if matches!(kind_of, Kind::Buffer | Kind::Plain(Type::Void)) {
            return type_error(g, &format!("{what}: \"{}\" is not a type a callback receives; a block of memory arrives as a \"pointer\"", kind_of.name()));
        }
        kinds.push(kind_of);
    }
    let returned = option(g, signature, "returns");
    let returns = if returned == UNDEF { Kind::Plain(Type::Void) } else { parse_kind(g, returned, what)? };
    if !matches!(returns, Kind::Plain(_)) {
        return type_error(g, &format!("{what}: a callback cannot return \"{}\": the memory would not outlive the call; return a \"pointer\"", returns.name()));
    }
    let name = read_string(g, get(g, function, "name")).filter(|n| !n.is_empty());
    let target = Arc::new(Target {
        g: g as usize,
        function,
        args: kinds.clone(),
        returns,
        thread: std::thread::current().id(),
        what: format!("NativeCallback {}", name.as_deref().unwrap_or("(anonymous)")),
        closed: AtomicBool::new(false),
        warned: AtomicBool::new(false),
    });
    let c_signature = Signature {
        args: kinds.iter().map(|k| k.c()).collect(),
        returns: returns.c(),
        fixed: None,
    };
    let reached = target.clone();
    let handler: sk_ffi::Handler = Arc::new(move |slots: &[u64]| unsafe {
        if reached.closed.load(Ordering::Acquire) {
            0
        } else if std::thread::current().id() == reached.thread {
            invoke(&reached, slots)
        } else {
            from_another_thread(&reached, slots)
        }
    });
    let Some(taken) = sk_ffi::Callback::new(c_signature, handler) else {
        return dom(g, "NotSupportedError", &format!("{what}: all {} callbacks are in use; close() the ones no longer needed", sk_ffi::CALLBACK_CAPACITY));
    };
    // The library may call for as long as it has the pointer, whatever JS still holds: until close().
    capi::JSValueProtect(g as _, function as usize as _);
    let address = taken.pointer() as u64;
    let state = NativeCallback { callback: Some(taken), address, target };
    Ok(host_create(g, callback_class(), prototype(g, "NativeCallback"), state))
});

std_fn!(fn callback_pointer(g, args) {
    let state = callback(g, args.this().0)?;
    if state.callback.is_none() {
        return dom(g, "InvalidStateError", "NativeCallback.pointer: the callback is closed");
    }
    Ok(pointer_new(g, state.address))
});

std_fn!(fn callback_close(g, args) {
    let state = callback(g, args.this().0)?;
    if let Some(taken) = state.callback.take() {
        state.target.closed.store(true, Ordering::Release);
        taken.release();
        capi::JSValueUnprotect(g as _, state.target.function as usize as _);
    }
    Ok(UNDEF)
});

std_fn!(fn callback_closed(g, args) { Ok(enc_bool(callback(g, args.this().0)?.callback.is_none())) });
std_fn!(fn callback_live(_g, _args) { Ok(enc_f64(sk_ffi::live_callbacks() as f64)) });

/// What a field of a struct holds.
enum Shape {
    Scalar(Type),
    Struct(Arc<StructDef>),
    Array(Box<Shape>, usize),
}

struct Field {
    name: String,
    offset: usize,
    shape: Shape,
}

struct StructDef {
    fields: Vec<Field>,
    layout: StructLayout,
}

struct StructType {
    def: Arc<StructDef>,
}

/// The bytes of a struct for a call, aligned for any field.
type Room = Vec<u64>;
/// A struct argument or return of one call.
type Packed = Option<(Arc<StructDef>, Room)>;

const STRUCT_LIMIT: usize = 65536;

impl Shape {
    fn size(&self) -> usize {
        match self {
            Shape::Scalar(ty) => ty.size(),
            Shape::Struct(def) => def.layout.size,
            Shape::Array(of, count) => of.size() * count,
        }
    }

    fn align(&self) -> usize {
        match self {
            Shape::Scalar(ty) => ty.size(),
            Shape::Struct(def) => def.layout.align,
            Shape::Array(of, _) => of.align(),
        }
    }

    fn flatten(&self, at: usize, into: &mut Vec<(Type, usize)>) {
        match self {
            Shape::Scalar(ty) => into.push((*ty, at)),
            Shape::Struct(def) => {
                into.extend(
                    def.layout
                        .fields
                        .iter()
                        .map(|(ty, offset)| (*ty, at + offset)),
                );
            }
            Shape::Array(of, count) => {
                for i in 0..*count {
                    of.flatten(at + i * of.size(), into);
                }
            }
        }
    }
}

impl StructDef {
    fn room(&self) -> Room {
        vec![0; self.layout.size.div_ceil(8)]
    }
}

static STRUCT: OnceLock<usize> = OnceLock::new();

fn struct_class() -> JSClassRef {
    host_class::<StructType>(&STRUCT, "StructType")
}

unsafe fn struct_of<'a>(g: *mut c_void, this: u64) -> Js<&'a StructDef> {
    match host_data::<StructType>(g, struct_class(), this) {
        Some(state) => Ok(&state.def),
        None => type_error(g, "this is not a struct type"),
    }
}

unsafe fn is_object(g: *mut c_void, v: u64) -> bool {
    is_cell(v) && kind(g, v) != KIND_STRING
}

unsafe fn parse_shape(g: *mut c_void, v: u64, what: &str) -> Js<Shape> {
    if let Some(of) = host_data::<StructType>(g, struct_class(), v) {
        return Ok(Shape::Struct(of.def.clone()));
    }
    if kind(g, v) == KIND_STRING {
        let name = read_string(g, v).unwrap_or_default();
        return match KINDS.iter().find(|(n, _)| *n == name) {
            Some((_, Kind::Plain(ty))) if *ty != Type::Void => Ok(Shape::Scalar(*ty)),
            _ => type_error(g, &format!("{what}: \"{name}\" is not a type of a field; a field is \"bool\", a number type, \"pointer\", a struct or [type, count]")),
        };
    }
    if is_object(g, v) && length(g, v) == 2 {
        let of = parse_shape(g, get_i(g, v, 0), what)?;
        let count = count(g, get_i(g, v, 1), &format!("{what}: the count of an array"))?;
        if count == 0 || count > STRUCT_LIMIT {
            return range_error(
                g,
                &format!("{what}: an array has from 1 to {STRUCT_LIMIT} elements"),
            );
        }
        return Ok(Shape::Array(Box::new(of), count));
    }
    type_error(
        g,
        &format!(
            "{what}: a field is \"bool\", a number type, \"pointer\", a struct or [type, count]"
        ),
    )
}

// `struct(entries, fields)`: the fields as `[name, type]` in order, and the object they came from.
std_fn!(fn struct_new(g, args) {
    let entries = args.arg(0).0;
    let offsets = new_object(g, UNDEF);
    let mut fields = Vec::new();
    let mut flat = Vec::new();
    let (mut size, mut align) = (0usize, 1usize);
    for i in 0..length(g, entries) {
        let entry = get_i(g, entries, i as u32);
        let name = string(g, get_i(g, entry, 0), "struct: a field's name")?;
        let shape = parse_shape(g, get_i(g, entry, 1), &format!("struct: field {name}"))?;
        let offset = size.next_multiple_of(shape.align());
        size = offset + shape.size();
        if size > STRUCT_LIMIT {
            return range_error(g, &format!("struct: a struct is at most {STRUCT_LIMIT} bytes"));
        }
        align = align.max(shape.align());
        shape.flatten(offset, &mut flat);
        define(g, offsets, &name, enc_f64(offset as f64), 0);
        fields.push(Field { name, offset, shape });
    }
    if fields.is_empty() {
        return type_error(g, "struct: a struct has at least one field, as { name: type }");
    }
    let layout = StructLayout { size: size.next_multiple_of(align), align, fields: flat };
    let state = StructType { def: Arc::new(StructDef { fields, layout }) };
    let object = host_create(g, struct_class(), prototype(g, "StructType"), state);
    define(g, object, "fields", args.arg(1).0, 0);
    define(g, object, "offsets", offsets, 0);
    Ok(object)
});

/// A field of a struct, for an error message.
struct Within<'a> {
    outer: &'a dyn Display,
    field: &'a str,
}

impl Display for Within<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: field {}", self.outer, self.field)
    }
}

/// Writes the fields `v` has; one it leaves out stays zero.
unsafe fn pack(g: *mut c_void, def: &StructDef, v: u64, to: *mut u8, what: &dyn Display) -> Js<()> {
    if !is_object(g, v) {
        return type_error(
            g,
            &format!("{what} must be an object of the struct's fields"),
        );
    }
    for field in &def.fields {
        let given = get(g, v, &field.name);
        if given != UNDEF {
            let what = Within {
                outer: what,
                field: &field.name,
            };
            write_field(g, &field.shape, given, to.add(field.offset), &what)?;
        }
    }
    Ok(())
}

unsafe fn write_field(
    g: *mut c_void,
    shape: &Shape,
    v: u64,
    to: *mut u8,
    what: &dyn Display,
) -> Js<()> {
    match shape {
        Shape::Scalar(ty) => {
            let bits = to_slot(g, v, Kind::Plain(*ty), &mut Arena::default(), what)?;
            std::ptr::copy_nonoverlapping(bits.to_le_bytes().as_ptr(), to, ty.size());
        }
        Shape::Struct(def) => pack(g, def, v, to, what)?,
        Shape::Array(of, count) => {
            if !is_object(g, v) || length(g, v) != *count {
                return type_error(g, &format!("{what} must be an array of {count} elements"));
            }
            for i in 0..*count {
                write_field(g, of, get_i(g, v, i as u32), to.add(i * of.size()), what)?;
            }
        }
    }
    Ok(())
}

/// A struct argument: an object of its fields, or bytes that already hold it.
unsafe fn fill(g: *mut c_void, def: &StructDef, v: u64, what: &dyn Display) -> Js<Room> {
    let mut room = def.room();
    match buffer_source(g, v) {
        Some(view) if view.len == def.layout.size => {
            std::ptr::copy_nonoverlapping(
                view.data as *const u8,
                room.as_mut_ptr().cast(),
                view.len,
            );
        }
        Some(view) => {
            return type_error(
                g,
                &format!(
                    "{what} is {} bytes, and the struct is {}",
                    view.len, def.layout.size
                ),
            );
        }
        None => pack(g, def, v, room.as_mut_ptr().cast(), what)?,
    }
    Ok(room)
}

/// `trusted` bytes came from a native call; a pointer out of any others is one made from an address.
unsafe fn unpack(g: *mut c_void, def: &StructDef, from: *const u8, trusted: bool) -> Js<u64> {
    let object = new_object(g, UNDEF);
    for field in &def.fields {
        let value = read_field(g, &field.shape, from.add(field.offset), trusted)?;
        define(g, object, &field.name, value, 0);
    }
    Ok(object)
}

unsafe fn read_field(g: *mut c_void, shape: &Shape, from: *const u8, trusted: bool) -> Js<u64> {
    match shape {
        Shape::Scalar(ty) => {
            let mut raw = [0u8; 8];
            std::ptr::copy_nonoverlapping(from, raw.as_mut_ptr(), ty.size());
            let bits = ty.normalize(u64::from_le_bytes(raw));
            if *ty == Type::Pointer && bits != 0 && !trusted && !permissions::current().ffi_address
            {
                return dom(g, "NotAllowedError", "unpack: a pointer read from bytes needs \"permissions\": { \"ffiAddress\": true } in sk.config.json");
            }
            from_slot(g, bits, Kind::Plain(*ty))
        }
        Shape::Struct(def) => unpack(g, def, from, trusted),
        Shape::Array(of, count) => {
            let array = new_array(g);
            for i in 0..*count {
                let value = read_field(g, of, from.add(i * of.size()), trusted)?;
                set_i(g, array, i as u32, value);
            }
            Ok(array)
        }
    }
}

std_fn!(fn struct_size(g, args) { Ok(enc_f64(struct_of(g, args.this().0)?.layout.size as f64)) });
std_fn!(fn struct_align(g, args) { Ok(enc_f64(struct_of(g, args.this().0)?.layout.align as f64)) });

std_fn!(fn struct_pack(g, args) {
    let def = struct_of(g, args.this().0)?;
    // Packed aside: reading the fields may allocate, and an allocation may move a new array's bytes.
    let mut room = def.room();
    pack(g, def, args.arg(0).0, room.as_mut_ptr().cast(), &"pack: the value")?;
    let (array, out) = typed_array(g, TA_U8, def.layout.size)?;
    std::ptr::copy_nonoverlapping(room.as_ptr().cast::<u8>(), out.as_mut_ptr(), out.len());
    Ok(array)
});

std_fn!(fn struct_unpack(g, args) {
    let def = struct_of(g, args.this().0)?;
    let bytes = bytes_arg(g, args.arg(0).0, "unpack: the bytes")?;
    let offset = if args.arg(1).0 == UNDEF { 0 } else { count(g, args.arg(1).0, "unpack: the offset")? };
    let size = def.layout.size;
    if offset > bytes.len() || bytes.len() - offset < size {
        return range_error(g, &format!("unpack: the struct is {size} bytes, and {} are left at offset {offset}", bytes.len().saturating_sub(offset)));
    }
    let mut room = def.room();
    std::ptr::copy_nonoverlapping(bytes.as_ptr().add(offset), room.as_mut_ptr().cast(), size);
    unpack(g, def, room.as_ptr().cast(), false)
});

/// A call with a struct among its arguments or as its return.
unsafe fn perform(
    address: usize,
    signature: &Signature,
    slots: &[u64],
    structs: &[Packed],
    returned: &mut Packed,
) -> sk_ffi::Outcome {
    let args: Vec<Argument<'_>> = signature
        .args
        .iter()
        .zip(slots)
        .enumerate()
        .map(
            |(i, (ty, bits))| match structs.get(i).and_then(Option::as_ref) {
                Some((def, room)) => Argument::Struct(&def.layout, room.as_ptr().cast()),
                None => Argument::Scalar(*ty, *bits),
            },
        )
        .collect();
    let returning = match returned {
        Some((def, room)) => Returning::Struct(&def.layout, room.as_mut_ptr().cast()),
        None => Returning::Scalar(signature.returns),
    };
    sk_ffi::call_with(address as *const (), &args, returning, signature.fixed)
}

unsafe fn returned_value(g: *mut c_void, value: u64, kind_of: Kind, returned: &Packed) -> Js<u64> {
    match returned {
        Some((def, room)) => unpack(g, def, room.as_ptr().cast(), true),
        None => from_slot(g, value, kind_of),
    }
}

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let module = new_object(g, UNDEF);
    static_fn(g, module, "open", 2, ffi_open);
    static_fn(g, module, "struct", 2, struct_new);

    support::class(
        g,
        &ClassSpec {
            name: "StructType",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[("pack", 1, struct_pack), ("unpack", 1, struct_unpack)],
            getters: &[("size", struct_size), ("align", struct_align)],
            accessors: &[],
        },
    );

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

    let callback = support::class(
        g,
        &ClassSpec {
            name: "NativeCallback",
            arity: 2,
            construct: Some(callback_construct),
            parent_proto: 0,
            methods: &[("close", 0, callback_close)],
            getters: &[("pointer", callback_pointer), ("closed", callback_closed)],
            accessors: &[],
        },
    );
    static_fn(g, callback.ctor, "live", 0, callback_live);
    define(g, module, "NativeCallback", callback.ctor, 0);

    define(g, std, "ffi", module, 0);
}
