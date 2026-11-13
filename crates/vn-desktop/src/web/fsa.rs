//! File System Access, and the host half of `<input type=file|color>` and `<a download>` (`__VN_FS__`).
//! A handle holds an id into [`crate::fs`]'s table, never a path; the id is released when the handle is collected.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;

use serde_json::{json, Value as Json};
use vn_runtime::host;
use vn_runtime::host_fn;
use vn_runtime::redjs::capi::JSClassRef;
use vn_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use vn_runtime::redjs::glue;
use vn_runtime::redjs::webapi::util::*;
use vn_runtime::redjs::webapi::{blob, file, serialize, uievents};

use super::{awaiting, error, global, rejected, rejected_with};
use crate::fs::writable::Data;
use crate::fs::{self, Kind};
use crate::{dialog, Fail};

struct Handle {
    id: u64,
    name: String,
    kind: Kind,
}

impl Drop for Handle {
    fn drop(&mut self) {
        fs::release(self.id);
    }
}

/// A writable stream's sink; collected without `close()` or `abort()`, it deletes the swap file.
struct Sink {
    w: u64,
    done: bool,
}

impl Drop for Sink {
    fn drop(&mut self) {
        if !self.done {
            let _ = fs::writable::start_abort(self.w);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Entries,
    Keys,
    Values,
}

struct DirIter {
    dir: u64,
    cursor: u64,
    buffer: VecDeque<Json>,
    done: bool,
    mode: Mode,
    busy: u64,
}

#[derive(Default)]
struct Classes {
    handle: usize,
    sink: usize,
    iter: usize,
    file_proto: u64,
    dir_proto: u64,
    writable: u64,
    sink_proto: u64,
    iter_proto: u64,
}

thread_local! {
    static C: RefCell<Classes> = RefCell::new(Classes::default());
    static ITERS: RefCell<HashMap<u32, u64>> = RefCell::new(HashMap::new());
    static PICKS: RefCell<HashMap<u32, &'static str>> = RefCell::new(HashMap::new());
    static ADVISED_GRANT: Cell<bool> = const { Cell::new(false) };
}

fn classes<R>(f: impl FnOnce(&Classes) -> R) -> R {
    C.with(|c| f(&c.borrow()))
}

fn failed(method: &str, iface: &str, message: &str) -> String {
    format!("Failed to execute '{method}' on '{iface}': {message}")
}

unsafe fn handle<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Handle> {
    host_data::<Handle>(g, classes(|c| c.handle) as JSClassRef, v)
}

unsafe fn sink<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Sink> {
    host_data::<Sink>(g, classes(|c| c.sink) as JSClassRef, v)
}

unsafe fn dir_iter<'a>(g: *mut c_void, v: u64) -> Option<&'a mut DirIter> {
    host_data::<DirIter>(g, classes(|c| c.iter) as JSClassRef, v)
}

unsafe fn illegal(g: *mut c_void, method: &str, iface: &str) -> u64 {
    rejected(g, "TypeError", &failed(method, iface, "Illegal invocation"))
}

/// A handle object for `{ h, name, kind }`.
pub(crate) unsafe fn from_wire(g: *mut c_void, w: &Json) -> u64 {
    let kind = if w["kind"] == "directory" { Kind::Directory } else { Kind::File };
    let (cls, proto) = classes(|c| (c.handle, if kind == Kind::Directory { c.dir_proto } else { c.file_proto }));
    host_create(g, cls as JSClassRef, proto, Handle { id: w["h"].as_u64().unwrap_or(0), name: w["name"].as_str().unwrap_or("").to_string(), kind })
}

unsafe fn activation(g: *mut c_void, method: &str) -> bool {
    let a = get(g, global(g), "__VN_ACTIVATION__");
    is_cell(a) && call(g, get(g, a, method), a, &[]) == Ok(TRUE)
}

unsafe fn grant_on_gesture(g: *mut c_void, id: u64) {
    fs::permission(id, true);
    if !ADVISED_GRANT.with(|a| a.replace(true)) {
        host::warn_compat(
            g,
            "fsa:grant",
            "FileSystemHandle: a handle restored from storage was granted on a user activation with no prompt. Chromium asks the user first (\"Allow this site to view/edit …?\"); this host has no such prompt yet.",
        );
    }
}

/// As Chromium: a "prompt" handle is granted on a user activation before a write; otherwise the host refuses.
unsafe fn write_access(g: *mut c_void, id: u64) {
    if fs::permission(id, false) == Some("prompt") && activation(g, "isActive") {
        grant_on_gesture(g, id);
    }
}

unsafe fn member(g: *mut c_void, obj: u64, k: &str) -> u64 {
    if is_cell(obj) {
        get(g, obj, k)
    } else {
        UNDEF
    }
}

unsafe fn result(g: *mut c_void, value: u64, done: bool) -> u64 {
    let o = new_object(g, UNDEF);
    define(g, o, "value", value, 0);
    define(g, o, "done", enc_bool(done), 0);
    o
}

unsafe fn to_undefined(_g: *mut c_void, _t: u32, _v: &Json, _m: u64) -> Result<u64, u64> {
    Ok(UNDEF)
}

unsafe fn to_made(_g: *mut c_void, _t: u32, _v: &Json, made: u64) -> Result<u64, u64> {
    Ok(made)
}

unsafe fn to_handle(g: *mut c_void, _t: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    Ok(from_wire(g, v))
}

unsafe fn to_same(_g: *mut c_void, _t: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    Ok(enc_bool(v["same"] == true))
}

unsafe fn to_path(g: *mut c_void, _t: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    Ok(if v["path"].is_array() { host::to_js(g, &v["path"]) } else { NULL })
}

// FileSystemHandle

host_fn!(fn handle_kind(g, args) {
    unsafe {
        match handle(g, args.this().0) {
            Some(h) => js_string(g, h.kind.as_str()),
            None => throw_type_error(g, &failed("kind", "FileSystemHandle", "Illegal invocation")),
        }
    }
});

host_fn!(fn handle_name(g, args) {
    unsafe {
        match handle(g, args.this().0) {
            Some(h) => js_string(g, &h.name),
            None => throw_type_error(g, &failed("name", "FileSystemHandle", "Illegal invocation")),
        }
    }
});

host_fn!(fn is_same_entry(g, args) {
    unsafe {
        let Some(a) = handle(g, args.this().0) else { return illegal(g, "isSameEntry", "FileSystemHandle") };
        let Some(b) = handle(g, args.arg(0).0) else {
            return rejected(g, "TypeError", &failed("isSameEntry", "FileSystemHandle", "parameter 1 is not of type 'FileSystemHandle'."));
        };
        if a.kind != b.kind {
            return host::resolved(g, FALSE);
        }
        awaiting(g, fs::start_same(a.id, b.id), to_same)
    }
});

unsafe fn permission(g: *mut c_void, this: u64, d: u64, method: &str, request: bool) -> u64 {
    let Some(h) = handle(g, this) else { return illegal(g, method, "FileSystemHandle") };
    let id = h.id;
    let mode = host::string_arg(g, member(g, d, "mode")).unwrap_or_else(|| "read".into());
    if mode != "read" && mode != "readwrite" {
        return rejected(g, "TypeError", &failed(method, "FileSystemHandle", &format!("The provided value '{mode}' is not a valid enum value of type FileSystemPermissionMode.")));
    }
    let state = fs::permission(id, false).unwrap_or("prompt");
    if !request || state == "granted" {
        return host::resolved(g, js_string(g, state));
    }
    // Chromium's browser-side words: no "Failed to execute" prefix.
    if !activation(g, "isActive") {
        return rejected(g, "SecurityError", "User activation is required to request permissions.");
    }
    grant_on_gesture(g, id);
    host::resolved(g, js_string(g, fs::permission(id, false).unwrap_or("prompt")))
}

host_fn!(fn query_permission(g, args) {
    unsafe { permission(g, args.this().0, args.arg(0).0, "queryPermission", false) }
});

host_fn!(fn request_permission(g, args) {
    unsafe { permission(g, args.this().0, args.arg(0).0, "requestPermission", true) }
});

// FileSystemFileHandle

unsafe fn file_handle<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Handle> {
    handle(g, v).filter(|h| h.kind == Kind::File)
}

unsafe fn dir_handle<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Handle> {
    handle(g, v).filter(|h| h.kind == Kind::Directory)
}

host_fn!(fn get_file(g, args) {
    unsafe {
        let Some(h) = file_handle(g, args.this().0) else { return illegal(g, "getFile", "FileSystemFileHandle") };
        awaiting(g, fs::start_get_file(h.id), to_made)
    }
});

unsafe fn to_writable(g: *mut c_void, _t: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    let w = v["w"].as_u64().unwrap_or(0);
    let (sink_cls, sink_proto, writable) = classes(|c| (c.sink, c.sink_proto, c.writable));
    let s = host_create(g, sink_cls as JSClassRef, sink_proto, Sink { w, done: false });
    let reflect = get(g, global(g), "Reflect");
    let args = new_array(g);
    set_i(g, args, 0, s);
    call(g, get(g, reflect, "construct"), reflect, &[get(g, global(g), "WritableStream"), args, writable])
}

host_fn!(fn create_writable(g, args) {
    unsafe {
        let Some(h) = file_handle(g, args.this().0) else { return illegal(g, "createWritable", "FileSystemFileHandle") };
        let id = h.id;
        write_access(g, id);
        let keep = member(g, args.arg(0).0, "keepExistingData") == TRUE;
        awaiting(g, fs::writable::start_open(id, keep), to_writable)
    }
});

// FileSystemDirectoryHandle

unsafe fn child(g: *mut c_void, this: u64, name: u64, options: u64, method: &str, kind: Kind) -> u64 {
    let Some(h) = dir_handle(g, this) else { return illegal(g, method, "FileSystemDirectoryHandle") };
    let id = h.id;
    let create = member(g, options, "create") == TRUE;
    if create {
        write_access(g, id);
    }
    let name = host::string_arg(g, name).unwrap_or_else(|| "undefined".into());
    awaiting(g, fs::start_child(id, &name, kind, create), to_handle)
}

host_fn!(fn get_file_handle(g, args) {
    unsafe { child(g, args.this().0, args.arg(0).0, args.arg(1).0, "getFileHandle", Kind::File) }
});

host_fn!(fn get_directory_handle(g, args) {
    unsafe { child(g, args.this().0, args.arg(0).0, args.arg(1).0, "getDirectoryHandle", Kind::Directory) }
});

host_fn!(fn remove_entry(g, args) {
    unsafe {
        let Some(h) = dir_handle(g, args.this().0) else { return illegal(g, "removeEntry", "FileSystemDirectoryHandle") };
        let id = h.id;
        write_access(g, id);
        let name = host::string_arg(g, args.arg(0).0).unwrap_or_else(|| "undefined".into());
        let recursive = member(g, args.arg(1).0, "recursive") == TRUE;
        awaiting(g, fs::start_remove(id, &name, recursive), to_undefined)
    }
});

host_fn!(fn resolve(g, args) {
    unsafe {
        let Some(h) = dir_handle(g, args.this().0) else { return illegal(g, "resolve", "FileSystemDirectoryHandle") };
        let Some(other) = handle(g, args.arg(0).0) else {
            return rejected(g, "TypeError", &failed("resolve", "FileSystemDirectoryHandle", "parameter 1 is not of type 'FileSystemHandle'."));
        };
        awaiting(g, fs::start_resolve(h.id, other.id), to_path)
    }
});

// Directory iteration: an async iterator over batches the host reads on the pool.

unsafe fn iterate(g: *mut c_void, this: u64, mode: Mode, method: &str) -> u64 {
    let Some(h) = dir_handle(g, this) else {
        return throw_type_error(g, &failed(method, "FileSystemDirectoryHandle", "Illegal invocation"));
    };
    let (cls, proto) = classes(|c| (c.iter, c.iter_proto));
    host_create(g, cls as JSClassRef, proto, DirIter { dir: h.id, cursor: 0, buffer: VecDeque::new(), done: false, mode, busy: 0 })
}

host_fn!(fn entries(g, args) {
    unsafe { iterate(g, args.this().0, Mode::Entries, "entries") }
});

host_fn!(fn keys(g, args) {
    unsafe { iterate(g, args.this().0, Mode::Keys, "keys") }
});

host_fn!(fn values(g, args) {
    unsafe { iterate(g, args.this().0, Mode::Values, "values") }
});

unsafe fn item(g: *mut c_void, mode: Mode, w: &Json) -> u64 {
    let name = js_string(g, w["name"].as_str().unwrap_or(""));
    match mode {
        Mode::Keys => name,
        Mode::Values => from_wire(g, w),
        Mode::Entries => {
            let pair = new_array(g);
            set_i(g, pair, 0, name);
            set_i(g, pair, 1, from_wire(g, w));
            pair
        }
    }
}

/// The next result from what is buffered, or `None` when the host has to be asked.
unsafe fn take(g: *mut c_void, it: &mut DirIter) -> Option<u64> {
    if let Some(w) = it.buffer.pop_front() {
        return Some(result(g, item(g, it.mode, &w), false));
    }
    it.done.then(|| result(g, UNDEF, true))
}

unsafe fn fetch(g: *mut c_void, obj: u64) -> u64 {
    let Some(it) = dir_iter(g, obj) else { return UNDEF };
    let (started, convert): (Result<u32, Fail>, super::Convert) =
        if it.cursor == 0 { (fs::start_dir_open(it.dir), opened) } else { (fs::start_dir_next(it.cursor), batch) };
    if let Ok(t) = started {
        redjs_protect(g, obj);
        ITERS.with(|m| m.borrow_mut().insert(t, obj));
    }
    awaiting(g, started, convert)
}

unsafe fn iter_of(g: *mut c_void, token: u32) -> u64 {
    let obj = ITERS.with(|m| m.borrow_mut().remove(&token)).unwrap_or(UNDEF);
    if obj != UNDEF {
        redjs_unprotect(g, obj);
    }
    obj
}

unsafe fn opened(g: *mut c_void, token: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    let obj = iter_of(g, token);
    let Some(it) = dir_iter(g, obj) else { return Ok(result(g, UNDEF, true)) };
    it.cursor = v["cursor"].as_u64().unwrap_or(0);
    Ok(fetch(g, obj))
}

unsafe fn batch(g: *mut c_void, token: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    let obj = iter_of(g, token);
    let Some(it) = dir_iter(g, obj) else { return Ok(result(g, UNDEF, true)) };
    it.buffer.extend(v["entries"].as_array().into_iter().flatten().cloned());
    if v["done"] == true {
        it.done = true;
        it.cursor = 0;
    }
    match take(g, it) {
        Some(r) => Ok(r),
        None => Ok(fetch(g, obj)),
    }
}

unsafe fn close(it: &mut DirIter) {
    if it.cursor != 0 {
        fs::dir_close(it.cursor);
        it.cursor = 0;
    }
    it.done = true;
    it.buffer.clear();
}

host_fn!(fn iter_settled(g, args) {
    unsafe {
        let obj = args.arg(0).0;
        if let Some(it) = dir_iter(g, obj) {
            if it.busy != 0 {
                redjs_unprotect(g, it.busy);
                it.busy = 0;
            }
        }
        UNDEF
    }
});

host_fn!(fn iter_failed(g, args) {
    unsafe {
        let obj = args.arg(0).0;
        if let Some(it) = dir_iter(g, obj) {
            close(it);
            if it.busy != 0 {
                redjs_unprotect(g, it.busy);
                it.busy = 0;
            }
        }
        UNDEF
    }
});

unsafe fn bound(g: *mut c_void, f: HostFn, arity: u32, first: u64) -> u64 {
    let fun = make_fn(g, "", arity, f);
    call(g, get(g, fun, "bind"), fun, &[UNDEF, first]).unwrap_or(UNDEF)
}

host_fn!(fn iter_next(g, args) {
    unsafe {
        let obj = args.this().0;
        let Some(it) = dir_iter(g, obj) else { return rejected(g, "TypeError", "next called on an object that is not a directory iterator") };
        if it.busy != 0 {
            // A second `next()` before the first settled runs after it, as an async generator queues.
            let then_next = bound(g, iter_next_again, 0, obj);
            return call(g, get(g, it.busy, "then"), it.busy, &[then_next]).unwrap_or(UNDEF);
        }
        if let Some(r) = take(g, it) {
            return host::resolved(g, r);
        }
        let p = fetch(g, obj);
        let busy = call(g, get(g, p, "then"), p, &[bound(g, iter_settled, 1, obj), bound(g, iter_failed, 1, obj)]).unwrap_or(UNDEF);
        if let Some(it) = dir_iter(g, obj) {
            redjs_protect(g, busy);
            it.busy = busy;
        }
        p
    }
});

host_fn!(fn iter_next_again(g, args) {
    unsafe {
        let obj = args.arg(0).0;
        let next = get(g, obj, "next");
        call(g, next, obj, &[]).unwrap_or(UNDEF)
    }
});

host_fn!(fn iter_return(g, args) {
    unsafe {
        if let Some(it) = dir_iter(g, args.this().0) {
            close(it);
        }
        host::resolved(g, result(g, args.arg(0).0, true))
    }
});

host_fn!(fn iter_self(_g, args) {
    args.this().0
});

// FileSystemWritableFileStream

unsafe fn data_of(g: *mut c_void, v: u64) -> Option<Data> {
    if host::is_string(g, v) {
        return Some(Data::Bytes(read_string(g, v).unwrap_or_default().into_bytes()));
    }
    if let Some(range) = blob::range_of(g, v) {
        return Some(Data::Blob(range));
    }
    bufferish(g, v).map(|b| Data::Bytes(b.to_vec()))
}

fn not_data() -> String {
    failed("write", "FileSystemWritableFileStream", "The provided value is not of type '(ArrayBuffer or ArrayBufferView or Blob or USVString or WriteParams)'.")
}

unsafe fn number(g: *mut c_void, v: u64) -> Option<f64> {
    dec_f64(v).or_else(|| (glue::rjs_kind(g, v) == 3).then(|| read_number(g, v)))
}

unsafe fn offset(g: *mut c_void, v: u64) -> Option<u64> {
    if v == UNDEF || v == NULL {
        return None;
    }
    let n = call(g, get(g, global(g), "Number"), UNDEF, &[v]).ok().and_then(|n| number(g, n))?;
    (n.is_finite() && n >= 0.0).then_some(n as u64)
}

host_fn!(fn sink_write(g, args) {
    unsafe {
        let Some(s) = sink(g, args.this().0) else { return rejected(g, "TypeError", "Illegal invocation") };
        let w = s.w;
        let chunk = args.arg(0).0;
        let params = is_cell(chunk) && !host::is_string(g, chunk) && blob::range_of(g, chunk).is_none() && bufferish(g, chunk).is_none();
        if !params {
            return match data_of(g, chunk) {
                Some(d) => awaiting(g, fs::writable::start_write(w, None, d), to_undefined),
                None => rejected(g, "TypeError", &not_data()),
            };
        }
        let syntax = |m: &str| rejected(g, "SyntaxError", &failed("write", "FileSystemWritableFileStream", m));
        let ty = host::string_arg(g, get(g, chunk, "type")).unwrap_or_else(|| "undefined".into());
        match ty.as_str() {
            "write" => {
                let data = get(g, chunk, "data");
                if data == UNDEF {
                    return syntax("write requires a data argument");
                }
                match data_of(g, data) {
                    Some(d) => awaiting(g, fs::writable::start_write(w, offset(g, get(g, chunk, "position")), d), to_undefined),
                    None => rejected(g, "TypeError", &not_data()),
                }
            }
            "seek" => match number(g, get(g, chunk, "position")) {
                Some(p) => awaiting(g, fs::writable::start_seek(w, p.max(0.0) as u64), to_undefined),
                None => syntax("seek requires a position argument"),
            },
            "truncate" => match number(g, get(g, chunk, "size")) {
                Some(n) => awaiting(g, fs::writable::start_truncate(w, n.max(0.0) as u64), to_undefined),
                None => syntax("truncate requires a size argument"),
            },
            other => rejected(g, "TypeError", &failed("write", "FileSystemWritableFileStream", &format!("The provided value '{other}' is not a valid enum value of type WriteCommandType."))),
        }
    }
});

unsafe fn finish(g: *mut c_void, this: u64, close: bool) -> u64 {
    let Some(s) = sink(g, this) else { return rejected(g, "TypeError", "Illegal invocation") };
    s.done = true;
    let started = if close { fs::writable::start_close(s.w) } else { fs::writable::start_abort(s.w) };
    awaiting(g, started, to_undefined)
}

host_fn!(fn sink_close(g, args) {
    unsafe { finish(g, args.this().0, true) }
});

host_fn!(fn sink_abort(g, args) {
    unsafe { finish(g, args.this().0, false) }
});

unsafe fn through_writer(g: *mut c_void, stream: u64, chunk: u64) -> u64 {
    let writer = match call(g, get(g, stream, "getWriter"), stream, &[]) {
        Ok(w) => w,
        Err(e) => return rejected_with(g, e),
    };
    let p = call(g, get(g, writer, "write"), writer, &[chunk]);
    let _ = call(g, get(g, writer, "releaseLock"), writer, &[]);
    p.unwrap_or_else(|e| rejected_with(g, e))
}

host_fn!(fn writable_write(g, args) {
    unsafe { through_writer(g, args.this().0, args.arg(0).0) }
});

unsafe fn command(g: *mut c_void, stream: u64, ty: &str, key: &str, v: u64) -> u64 {
    let c = new_object(g, UNDEF);
    define(g, c, "type", js_string(g, ty), 0);
    define(g, c, key, v, 0);
    through_writer(g, stream, c)
}

host_fn!(fn writable_seek(g, args) {
    unsafe { command(g, args.this().0, "seek", "position", args.arg(0).0) }
});

host_fn!(fn writable_truncate(g, args) {
    unsafe { command(g, args.this().0, "truncate", "size", args.arg(0).0) }
});

// Pickers

const WELL_KNOWN: &[&str] = &["desktop", "documents", "downloads", "music", "pictures", "videos"];

fn token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&b)
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(token_char)
}

unsafe fn validate(g: *mut c_void, method: &str, o: u64) -> Result<serde_json::Map<String, Json>, String> {
    let fail = |m: String| failed(method, "Window", &m);
    let mut req = serde_json::Map::new();
    let id = member(g, o, "id");
    if id != UNDEF {
        let id = host::string_arg(g, id).unwrap_or_default();
        if id.chars().count() > 32 {
            return Err(fail("ID too long".into()));
        }
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err(fail("ID contains invalid characters".into()));
        }
        req.insert("id".into(), id.into());
    }
    let start = member(g, o, "startIn");
    if start != UNDEF {
        if let Some(h) = handle(g, start) {
            req.insert("startIn".into(), json!({ "h": h.id }));
        } else {
            let s = host::string_arg(g, start).unwrap_or_default();
            if !WELL_KNOWN.contains(&s.as_str()) {
                return Err(fail(format!("The provided value '{s}' is not a valid enum value of type WellKnownDirectory.")));
            }
            req.insert("startIn".into(), s.into());
        }
    }
    let exclude = host::truthy(g, member(g, o, "excludeAcceptAllOption"));
    let types_v = member(g, o, "types");
    let mut types = Vec::new();
    if types_v != UNDEF {
        let arr = call(g, get(g, get(g, global(g), "Array"), "from"), UNDEF, &[types_v]).map_err(|_| fail("types is not iterable".into()))?;
        for i in 0..array_len(g, arr) {
            let t = get_i(g, arr, i);
            let mut accept = serde_json::Map::new();
            let acc = member(g, t, "accept");
            if is_cell(acc) {
                let object = get(g, global(g), "Object");
                let pairs = call(g, get(g, object, "entries"), object, &[acc]).unwrap_or(UNDEF);
                for j in 0..array_len(g, pairs) {
                    let pair = get_i(g, pairs, j);
                    let mime = read_string(g, get_i(g, pair, 0)).unwrap_or_default();
                    let raw = get_i(g, pair, 1);
                    let mut parts = mime.trim().split('/');
                    let (ty, sub, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next());
                    if rest.is_some() || !is_token(ty) || !is_token(sub) {
                        return Err(fail(format!("Invalid type: {mime}")));
                    }
                    let exts: Vec<String> = if host::is_string(g, raw) {
                        vec![read_string(g, raw).unwrap_or_default()]
                    } else if raw == UNDEF || raw == NULL {
                        Vec::new()
                    } else {
                        let a = call(g, get(g, get(g, global(g), "Array"), "from"), UNDEF, &[raw]).unwrap_or(UNDEF);
                        (0..array_len(g, a)).map(|k| host::string_arg(g, get_i(g, a, k)).unwrap_or_default()).collect()
                    };
                    for e in &exts {
                        if !e.starts_with('.') {
                            return Err(fail(format!("Extension '{e}' must start with '.'.")));
                        }
                        if e.len() < 2 || !e[1..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'.') {
                            return Err(fail(format!("Extension '{e}' contains invalid characters.")));
                        }
                        if e.ends_with('.') {
                            return Err(fail(format!("Extension '{e}' must not end with '.'.")));
                        }
                        if e.chars().count() > 16 {
                            return Err(fail(format!("Extension '{e}' cannot be longer than 16 characters.")));
                        }
                    }
                    accept.insert(mime.trim().to_lowercase(), exts.into());
                }
            }
            let description = host::string_arg(g, member(g, t, "description")).unwrap_or_default();
            types.push(json!({ "description": description, "accept": accept }));
        }
    }
    if types.is_empty() && exclude {
        return Err(fail("Need at least one accepted type".into()));
    }
    req.insert("types".into(), types.into());
    req.insert("excludeAcceptAllOption".into(), exclude.into());
    let mode = member(g, o, "mode");
    if mode != UNDEF {
        let m = host::string_arg(g, mode).unwrap_or_default();
        if m != "read" && m != "readwrite" {
            return Err(fail(format!("The provided value '{m}' is not a valid enum value of type FileSystemPermissionMode.")));
        }
    }
    Ok(req)
}

unsafe fn picked(g: *mut c_void, token: u32, v: &Json, _m: u64) -> Result<u64, u64> {
    let method = PICKS.with(|m| m.borrow_mut().remove(&token)).unwrap_or("showOpenFilePicker");
    if v["cancelled"] == true {
        return Err(error(g, "AbortError", "The user aborted a request."));
    }
    let wires = v["handles"].as_array().cloned().unwrap_or_default();
    if method != "showOpenFilePicker" {
        return Ok(wires.first().map_or(UNDEF, |w| from_wire(g, w)));
    }
    let arr = new_array(g);
    for (i, w) in wires.iter().enumerate() {
        set_i(g, arr, i as u32, from_wire(g, w));
    }
    Ok(arr)
}

unsafe fn pick(g: *mut c_void, method: &'static str, kind: &str, o: u64) -> u64 {
    let mut req = match validate(g, method, o) {
        Ok(r) => r,
        Err(m) => return rejected(g, "TypeError", &m),
    };
    req.insert("kind".into(), kind.into());
    req.insert("result".into(), "handles".into());
    if kind == "open" {
        req.insert("multiple".into(), host::truthy(g, member(g, o, "multiple")).into());
    }
    let suggested = member(g, o, "suggestedName");
    if kind == "save" && suggested != UNDEF {
        req.insert("suggestedName".into(), host::string_arg(g, suggested).unwrap_or_default().into());
    }
    if !activation(g, "isActive") {
        return rejected(g, "SecurityError", &failed(method, "Window", "Must be handling a user gesture to show a file picker."));
    }
    activation(g, "consume");
    match dialog::start_open(&Json::Object(req).to_string(), Vec::new()) {
        Ok(t) => {
            PICKS.with(|m| m.borrow_mut().insert(t, method));
            awaiting(g, Ok(t), picked)
        }
        Err(f) if f.name == "NotAllowedError" && f.code == Some("EBUSY") => rejected(g, "NotAllowedError", &failed(method, "Window", "File picker already active.")),
        Err(f) => awaiting(g, Err(f), picked),
    }
}

host_fn!(fn show_open_file_picker(g, args) {
    unsafe { pick(g, "showOpenFilePicker", "open", args.arg(0).0) }
});

host_fn!(fn show_save_file_picker(g, args) {
    unsafe { pick(g, "showSaveFilePicker", "save", args.arg(0).0) }
});

host_fn!(fn show_directory_picker(g, args) {
    unsafe { pick(g, "showDirectoryPicker", "folder", args.arg(0).0) }
});

// <input type=file|color> and <a download>, for runtime.ts

/// `accept`: trimmed, lowercased tokens; others are advised and dropped. A `type/*` is kept, as Chromium does.
unsafe fn accept_tokens(g: *mut c_void, accept: &str) -> Vec<String> {
    let (kept, dropped) = split_accept(accept);
    for t in dropped {
        host::warn_compat(g, &format!("accept:{t}"), &format!("<input accept>: the token \"{t}\" is not an extension or MIME type and was ignored"));
    }
    kept
}

fn split_accept(accept: &str) -> (Vec<String>, Vec<String>) {
    accept.split(',').map(|raw| raw.trim().to_lowercase()).filter(|t| !t.is_empty()).partition(|t| {
        let mut parts = t.split('/');
        let (ty, sub, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next());
        (t.starts_with('.') && t.len() > 1) || (rest.is_none() && is_token(ty) && is_token(sub))
    })
}

unsafe fn input_result(g: *mut c_void, _t: u32, v: &Json, made: u64) -> Result<u64, u64> {
    if v["cancelled"] == true {
        return Ok(NULL);
    }
    Ok(if v["color"].is_string() { js_string(g, v["color"].as_str().unwrap_or("")) } else { made })
}

host_fn!(fn input_failed(g, args) {
    unsafe {
        let label = read_string(g, args.arg(0).0).unwrap_or_default();
        let e = args.arg(1).0;
        if read_string(g, member(g, e, "code")).as_deref() == Some("EBUSY") {
            return UNDEF;
        }
        let msg = read_string(g, member(g, e, "message")).unwrap_or_default();
        let console = get(g, global(g), "console");
        let _ = call(g, get(g, console, "error"), console, &[js_string(g, &format!("[vue-native] {label}: {msg}"))]);
        UNDEF
    }
});

unsafe fn quietly(g: *mut c_void, label: &str, started: Result<u32, Fail>) -> u64 {
    if let Err(f) = &started {
        if f.code == Some("EBUSY") {
            return host::resolved(g, UNDEF);
        }
    }
    let p = super::awaiting_with(g, started, input_result, super::Errors::Desktop);
    let fun = make_fn(g, "", 2, input_failed);
    let on_err = call(g, get(g, fun, "bind"), fun, &[UNDEF, js_string(g, label)]).unwrap_or(UNDEF);
    call(g, get(g, p, "catch"), p, &[on_err]).unwrap_or(p)
}

host_fn!(fn pick_for_input(g, args) {
    unsafe {
        let accept = host::string_arg(g, args.arg(0).0).unwrap_or_default();
        let multiple = host::truthy(g, args.arg(1).0);
        let directory = host::truthy(g, args.arg(2).0);
        let prev = file::file_list_items(g, args.arg(3).0).unwrap_or_default().into_iter().filter_map(|f| file::path_of(g, f)).collect();
        let req = json!({ "kind": if directory { "folder" } else { "open" }, "multiple": multiple, "accept": accept_tokens(g, &accept), "result": "files" });
        quietly(g, "<input type=file>", dialog::start_open(&req.to_string(), prev))
    }
});

host_fn!(fn pick_color(g, args) {
    unsafe {
        let value = host::string_arg(g, args.arg(0).0).unwrap_or_default();
        let req = json!({ "kind": "color", "value": value });
        quietly(g, "<input type=color>", dialog::start_open(&req.to_string(), Vec::new()))
    }
});

unsafe fn download_failed(g: *mut c_void, name: &str, msg: &str) {
    host::warn_compat(g, &format!("download:{name}"), &format!("<a download>: nothing was saved — {msg}"));
}

host_fn!(fn download_failed_fn(g, args) {
    unsafe {
        let e = args.arg(0).0;
        let name = read_string(g, member(g, e, "name")).unwrap_or_default();
        let msg = read_string(g, member(g, e, "message")).unwrap_or_default();
        download_failed(g, &name, &msg);
        UNDEF
    }
});

unsafe fn start_download(g: *mut c_void, url: &str, attr: &str) -> Result<u32, Fail> {
    let (data, mime) = if url.starts_with("blob:") {
        let t = vn_runtime::redjs::webapi::url::resolve_blob_url(g, url)
            .ok_or_else(|| Fail::new("NotFoundError", "the blob: URL was revoked, or was never created here; nothing to save").code("ENOENT"))?;
        let mime = t.mime().to_string();
        (dialog::data_of_blob_target(t), mime)
    } else if url.starts_with("data:") {
        let f = vn_runtime::redjs::webapi::fsutil::fetch_url(g, url);
        if !f.ok {
            return Err(Fail::type_error("the data: URL could not be decoded"));
        }
        (Data::Bytes(f.bytes), f.mime)
    } else {
        return Err(Fail::type_error("only blob: and data: URLs are saved as downloads"));
    };
    dialog::start_download(data, dialog::download_name(attr, url, &mime))
}

host_fn!(fn download(g, args) {
    unsafe {
        host::warn_compat(
            g,
            "download:saveas",
            "<a download>: the file is saved through a Save As dialog, starting in Downloads. Chromium saves to Downloads without asking unless \"Ask where to save each file\" is on",
        );
        let url = host::string_arg(g, args.arg(0).0).unwrap_or_default();
        let attr = host::string_arg(g, args.arg(1).0).unwrap_or_default();
        match start_download(g, &url, &attr) {
            Err(f) => download_failed(g, f.name, &f.message),
            Ok(t) => {
                let p = super::awaiting_with(g, Ok(t), to_undefined, super::Errors::Desktop);
                let _ = call(g, get(g, p, "catch"), p, &[make_fn(g, "", 1, download_failed_fn)]);
            }
        }
        UNDEF
    }
});

// Drops and structured clone

unsafe fn drop_hook(g: *mut c_void, path: &std::path::Path, is_dir: bool) -> Result<u64, u64> {
    let kind = if is_dir { Kind::Directory } else { Kind::File };
    Ok(from_wire(g, &fs::granted_handle_json(path.to_path_buf(), kind)))
}

const NOT_CLONED: &str = "FileSystemHandle object could not be cloned.";
const NOT_DESERIALIZED: &str = "Unable to deserialize cloned data.";

unsafe fn encode(g: *mut c_void, v: u64) -> Result<Vec<u8>, String> {
    let id = handle(g, v).map(|h| h.id).ok_or_else(|| NOT_CLONED.to_string())?;
    fs::to_wire(id).ok_or_else(|| NOT_CLONED.to_string())
}

unsafe fn decode(g: *mut c_void, bytes: &[u8]) -> Result<u64, String> {
    let (path, kind) = fs::from_wire(bytes).ok_or_else(|| NOT_DESERIALIZED.to_string())?;
    Ok(from_wire(g, &fs::handle_json(path, kind)))
}

unsafe fn define_symbol_method(g: *mut c_void, proto: u64, symbol: &str, f: u64) {
    let sym = get(g, get(g, global(g), "Symbol"), symbol);
    let object = get(g, global(g), "Object");
    let desc = new_object(g, UNDEF);
    define(g, desc, "value", f, 0);
    define(g, desc, "writable", TRUE, 0);
    define(g, desc, "configurable", TRUE, 0);
    let _ = call(g, get(g, object, "defineProperty"), object, &[proto, sym, desc]);
}

pub(super) unsafe fn install(g: *mut c_void) {
    let handle_cls = make_host_class::<Handle>("FileSystemHandle");
    let base = define_class(g, &ClassSpec {
        name: "FileSystemHandle",
        arity: 0,
        construct: None,
        parent_proto: 0,
        methods: &[("isSameEntry", 1, is_same_entry), ("queryPermission", 0, query_permission), ("requestPermission", 0, request_permission)],
        getters: &[("kind", handle_kind), ("name", handle_name)],
        accessors: &[],
    });
    let file_cls = define_class(g, &ClassSpec {
        name: "FileSystemFileHandle",
        arity: 0,
        construct: None,
        parent_proto: base.proto,
        methods: &[("getFile", 0, get_file), ("createWritable", 0, create_writable)],
        getters: &[],
        accessors: &[],
    });
    let dir_cls = define_class(g, &ClassSpec {
        name: "FileSystemDirectoryHandle",
        arity: 0,
        construct: None,
        parent_proto: base.proto,
        methods: &[
            ("getFileHandle", 1, get_file_handle),
            ("getDirectoryHandle", 1, get_directory_handle),
            ("removeEntry", 1, remove_entry),
            ("resolve", 1, resolve),
            ("entries", 0, entries),
            ("keys", 0, keys),
            ("values", 0, values),
        ],
        getters: &[],
        accessors: &[],
    });
    define_symbol_method(g, dir_cls.proto, "asyncIterator", get(g, dir_cls.proto, "entries"));

    let stream_proto = get(g, get(g, global(g), "WritableStream"), "prototype");
    let writable = define_class(g, &ClassSpec {
        name: "FileSystemWritableFileStream",
        arity: 0,
        construct: None,
        parent_proto: stream_proto,
        methods: &[("write", 1, writable_write), ("seek", 1, writable_seek), ("truncate", 1, writable_truncate)],
        getters: &[],
        accessors: &[],
    });
    redjs_protect(g, writable.ctor);

    let sink_cls = make_host_class::<Sink>("FileSystemWritableFileStreamSink");
    let sink_proto = new_object(g, UNDEF);
    redjs_protect(g, sink_proto);
    host::method(g, sink_proto, "write", 1, sink_write);
    host::method(g, sink_proto, "close", 0, sink_close);
    host::method(g, sink_proto, "abort", 1, sink_abort);

    let iter_cls = make_host_class::<DirIter>("FileSystemDirectoryHandleIterator");
    let iter_proto = new_object(g, UNDEF);
    redjs_protect(g, iter_proto);
    host::method(g, iter_proto, "next", 0, iter_next);
    host::method(g, iter_proto, "return", 1, iter_return);
    define_symbol_method(g, iter_proto, "asyncIterator", make_fn(g, "[Symbol.asyncIterator]", 0, iter_self));
    to_string_tag(g, iter_proto, "FileSystemDirectoryHandle AsyncIterator");

    C.with(|c| {
        *c.borrow_mut() = Classes {
            handle: handle_cls as usize,
            sink: sink_cls as usize,
            iter: iter_cls as usize,
            file_proto: file_cls.proto,
            dir_proto: dir_cls.proto,
            writable: writable.ctor,
            sink_proto,
            iter_proto,
        }
    });

    for (name, f) in [
        ("showOpenFilePicker", show_open_file_picker as HostFn),
        ("showSaveFilePicker", show_save_file_picker),
        ("showDirectoryPicker", show_directory_picker),
    ] {
        define(g, global(g), name, make_fn(g, name, 0, f), 2);
    }

    // Both kinds clone through their base; a writable stream does not, as in Chromium.
    serialize::serialize_platform_prototype(g, base.proto, "FileSystemHandle", serialize::HostCodec { encode, decode });
    serialize::refuse_platform_prototype(g, writable.proto, "FileSystemWritableFileStream");
    uievents::set_file_system_handle_hook(g, Some(drop_hook));

    let obj = host::install_object(g, "__VN_FS__", UNDEF);
    host::method(g, obj, "pickForInput", 4, pick_for_input);
    host::method(g, obj, "pickColor", 1, pick_color);
    host::method(g, obj, "download", 2, download);
}

#[cfg(test)]
mod tests {
    use crate::fs::tests::Scratch;
    use crate::fs::{self, Kind};
    use vn_runtime::redjs::webapi::util::define;

    fn run_in(dir: &std::path::Path, src: &str) -> String {
        let dir = dir.to_path_buf();
        super::super::testing::run_with(src, move |g| unsafe {
            fs::grant(&dir);
            let h = super::from_wire(g, &fs::handle_json(dir, Kind::Directory));
            define(g, super::global(g), "dir", h, 0);
        })
    }

    #[test]
    fn a_directory_handle_creates_writes_reads_lists_and_removes() {
        let s = Scratch::new("fsa");
        std::fs::write(s.0.join("b.txt"), b"bee").unwrap();
        let out = run_in(
            &s.0,
            "const r = []; \
             r.push(dir instanceof FileSystemDirectoryHandle, dir instanceof FileSystemHandle, dir.kind); \
             const f = await dir.getFileHandle('a.txt', { create: true }); \
             r.push(f instanceof FileSystemFileHandle, f.kind, f.name); \
             const w = await f.createWritable(); \
             r.push(w instanceof FileSystemWritableFileStream, w instanceof WritableStream); \
             await w.write('hello'); await w.write({ type: 'write', position: 0, data: new TextEncoder().encode('J') }); \
             await w.close(); \
             const file = await f.getFile(); r.push(file instanceof File, file.name, file.size); \
             r.push(await f.isSameEntry(await dir.getFileHandle('a.txt')), await f.isSameEntry(dir)); \
             r.push(JSON.stringify(await dir.resolve(f))); \
             const names = []; for await (const [n, h] of dir) names.push(n + ':' + h.kind); \
             r.push(names.sort().join(',')); \
             const keys = []; for await (const k of dir.keys()) { keys.push(k); break; } \
             r.push(keys.length); \
             await dir.removeEntry('b.txt'); \
             try { await dir.getFileHandle('b.txt'); } catch (e) { r.push(e.name); } \
             try { await w.write({ type: 'seek' }); } catch (e) { r.push(e.name); } \
             r.push(await dir.queryPermission({ mode: 'readwrite' })); \
             try { await dir.queryPermission({ mode: 'x' }); } catch (e) { r.push(e.name); } \
             try { await dir.requestPermission.call({}); } catch (e) { r.push(e.name); } \
             return r.join('|');",
        );
        assert_eq!(
            out,
            "true|true|directory|true|file|a.txt|true|true|true|a.txt|5|true|false|[\"a.txt\"]|a.txt:file,b.txt:file|1|NotFoundError|TypeError|granted|TypeError|TypeError"
        );
        assert_eq!(std::fs::read(s.0.join("a.txt")).unwrap(), b"Jello");
    }

    #[test]
    fn pickers_validate_then_need_a_user_activation() {
        let out = super::super::testing::run(
            "const r = []; \
             for (const o of [{ id: 'x'.repeat(33) }, { id: 'a b' }, { startIn: 'attic' }, { excludeAcceptAllOption: true }, \
                              { types: [{ accept: { 'text plain': ['.txt'] } }] }, { types: [{ accept: { 'text/plain': ['txt'] } }] }, \
                              { types: [{ accept: { 'text/plain': ['.t.'] } }] }, { mode: 'all' }]) { \
               try { await showOpenFilePicker(o); r.push('ok'); } catch (e) { r.push(e.name); } \
             } \
             try { await showSaveFilePicker({ types: [{ accept: { 'text/plain': '.txt' } }] }); } catch (e) { r.push(e.name + ':' + e.message); } \
             r.push(typeof FileSystemHandle, typeof showDirectoryPicker); \
             try { new FileSystemHandle(); } catch (e) { r.push(e.name); } \
             return r.join('|');",
        );
        assert_eq!(
            out,
            "TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|SecurityError:Failed to execute 'showSaveFilePicker' on 'Window': Must be handling a user gesture to show a file picker.|function|function|TypeError"
        );
    }

    #[test]
    fn accept_keeps_extensions_and_mime_types_lowercased() {
        let (kept, dropped) = super::split_accept(".CSV, text/*, nope,, image/png/x , .");
        assert_eq!(kept, [".csv", "text/*"]);
        assert_eq!(dropped, ["nope", "image/png/x", "."]);
    }

    #[test]
    fn a_handle_clones_through_structured_clone() {
        let s = Scratch::new("fsa-clone");
        std::fs::write(s.0.join("c.txt"), b"c").unwrap();
        let out = run_in(
            &s.0,
            "const f = await dir.getFileHandle('c.txt'); const c = structuredClone(f); \
             const r = [c instanceof FileSystemFileHandle, c.name, await c.isSameEntry(f)]; \
             const w = await f.createWritable({ keepExistingData: true }); \
             try { structuredClone(w); } catch (e) { r.push(e.name); } \
             await w.abort(); \
             return r.join('|');",
        );
        assert_eq!(out, "true|c.txt|true|DataCloneError");
    }
}
