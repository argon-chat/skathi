//! The desktop surface: argv parser, shell (open/reveal/trash), single instance, custom
//! protocols, File System Access, dialogs and screen capture.
//!
//! Async host fns answer synchronously with `{token}` or `{error, name}`; a worker thread pushes
//! `(token, ok, payload)` and one JS dispatcher settles it on the JS thread. Workers never touch
//! JSC, so a JS value (a `File`, a `FileList`) is made by a [`Maker`] at delivery. Token `0` is
//! reserved for unsolicited host events (`launch`).
//!
//! Results are delivered inside the redjs pump budget through a `PumpSource`, so a loop of
//! `await writable.write(..)` does not cost a frame per step. Dialog and shell results never count
//! as in flight: a Save As left open must not make every frame wait. [`pump`] delivers the rest
//! and writes worker log lines into the ring.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use vn_runtime::redjs::api::Value;
use vn_runtime::{host_fn, JsRuntime};

pub mod args;
pub mod capture;
pub mod dialog;
pub mod fs;
pub mod launch;
pub mod network;
mod pool;
pub mod shell;
pub mod tray;

/// A failure JS turns into a rejection: `name` is a DOMException name or `TypeError`, `code` a
/// Node-style string, `os_error` the raw OS code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub name: &'static str,
    pub message: String,
    pub code: Option<&'static str>,
    pub os_error: Option<i32>,
}

impl Fail {
    pub fn new(name: &'static str, message: impl Into<String>) -> Self {
        Fail { name, message: message.into(), code: None, os_error: None }
    }
    pub fn type_error(message: impl Into<String>) -> Self {
        Fail::new("TypeError", message)
    }
    pub fn code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }
    pub fn os(mut self, err: i32) -> Self {
        self.os_error = Some(err);
        self
    }

    /// `{name, message, code?, osError?}`, the async rejection payload.
    pub fn to_json(&self) -> String {
        let mut m = serde_json::Map::new();
        m.insert("name".into(), self.name.into());
        m.insert("message".into(), self.message.clone().into());
        if let Some(c) = self.code {
            m.insert("code".into(), c.into());
        }
        if let Some(e) = self.os_error {
            m.insert("osError".into(), e.into());
        }
        serde_json::Value::Object(m).to_string()
    }

    /// `{error, name, code?, osError?}`, the synchronous refusal answered instead of a token.
    pub fn to_sync_json(&self) -> String {
        let mut m = serde_json::Map::new();
        m.insert("error".into(), self.message.clone().into());
        m.insert("name".into(), self.name.into());
        if let Some(c) = self.code {
            m.insert("code".into(), c.into());
        }
        if let Some(e) = self.os_error {
            m.insert("osError".into(), e.into());
        }
        serde_json::Value::Object(m).to_string()
    }
}

/// Makes the JS value a settle carries, on the JS thread at delivery.
pub(crate) type Maker = Box<dyn FnOnce(*mut c_void) -> u64 + Send>;

pub(crate) struct Done {
    pub payload: String,
    pub make: Option<Maker>,
}

impl Done {
    pub fn json(payload: &str) -> Done {
        Done { payload: payload.to_string(), make: None }
    }
    pub fn with_value(payload: String, make: impl FnOnce(*mut c_void) -> u64 + Send + 'static) -> Done {
        Done { payload, make: Some(Box::new(make)) }
    }
}

enum Item {
    Settle { token: u32, ok: bool, payload: String, make: Option<Maker> },
    /// The log ring is main-thread only, so worker lines ride the queue.
    Log { kind: &'static str, text: String },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);
static SETTLES_QUEUED: AtomicUsize = AtomicUsize::new(0);

/// Items delivered per frame, so a burst cannot lengthen one frame without bound.
const DRAIN_CAP: usize = 16;

/// A fresh token; never 0 (the unsolicited-event channel).
pub(crate) fn next_token() -> u32 {
    next_from(&NEXT_TOKEN)
}

fn next_from(counter: &AtomicU32) -> u32 {
    loop {
        let t = counter.fetch_add(1, Ordering::Relaxed);
        if t != 0 {
            return t;
        }
    }
}

fn push_settle(item: Item) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(item);
        SETTLES_QUEUED.fetch_add(1, Ordering::AcqRel);
    }
    wake();
}

/// Settles `token` from any thread.
pub(crate) fn settle(token: u32, result: Result<String, Fail>) {
    settle_done(token, result.map(|payload| Done { payload, make: None }));
}

pub(crate) fn settle_done(token: u32, result: Result<Done, Fail>) {
    push_settle(match result {
        Ok(Done { payload, make }) => Item::Settle { token, ok: true, payload, make },
        Err(f) => Item::Settle { token, ok: false, payload: f.to_json(), make: None },
    });
}

pub(crate) fn emit(payload: String) {
    push_settle(Item::Settle { token: 0, ok: true, payload, make: None });
}

#[cfg(test)]
pub(crate) fn take_emitted() -> Vec<String> {
    let Ok(mut q) = QUEUE.lock() else { return Vec::new() };
    let mut out = Vec::new();
    q.retain(|item| match item {
        Item::Settle { token: 0, payload, .. } => {
            out.push(payload.clone());
            SETTLES_QUEUED.fetch_sub(1, Ordering::AcqRel);
            false
        }
        _ => true,
    });
    out
}

#[cfg(test)]
pub(crate) fn take_settled(tokens: &[u32]) -> std::collections::HashMap<u32, (bool, String)> {
    let mut out = std::collections::HashMap::new();
    let Ok(mut q) = QUEUE.lock() else { return out };
    q.retain(|item| match item {
        Item::Settle { token, ok, payload, .. } if tokens.contains(token) => {
            out.insert(*token, (*ok, payload.clone()));
            SETTLES_QUEUED.fetch_sub(1, Ordering::AcqRel);
            false
        }
        _ => true,
    });
    out
}

/// The dispatcher's context and its redjs id, set by [`install`]; 0 = no JS runtime.
static RT_G: AtomicUsize = AtomicUsize::new(0);
static RT_ID: AtomicU64 = AtomicU64::new(0);

/// Wakes the JS pump from any thread so one waiting on a pool result returns at once.
pub(crate) fn wake() {
    let id = RT_ID.load(Ordering::Acquire);
    if id != 0 {
        vn_runtime::redjs::web::wake(vn_runtime::redjs::context::ContextId(id));
    }
    vn_runtime::frames::wake();
}

fn is_ours(g: *mut c_void) -> bool {
    g as usize == RT_G.load(Ordering::Acquire) && !g.is_null()
}

unsafe fn src_inflight(g: *mut c_void) -> bool {
    is_ours(g) && pool::in_flight() != 0
}

unsafe fn src_ready(g: *mut c_void) -> bool {
    is_ours(g) && SETTLES_QUEUED.load(Ordering::Acquire) != 0 && DISPATCHER.with(|d| d.get()) != 0
}

/// Delivers one settle at a time, each followed by a microtask checkpoint, until the queue is
/// empty or the budget is spent. Log lines stay for [`pump`].
unsafe fn src_drain(g: *mut c_void) -> usize {
    if !src_ready(g) {
        return 0;
    }
    let budget = std::time::Duration::from_micros(vn_runtime::redjs::web::pump_budget_us() as u64);
    let start = std::time::Instant::now();
    let mut n = 0;
    loop {
        let item = {
            let Ok(mut q) = QUEUE.lock() else { break };
            let Some(i) = q.iter().position(|it| matches!(it, Item::Settle { .. })) else { break };
            SETTLES_QUEUED.fetch_sub(1, Ordering::AcqRel);
            q.remove(i)
        };
        let Some(Item::Settle { token, ok, payload, make }) = item else { break };
        deliver(g, token, ok, &payload, make);
        vn_runtime::redjs::web::checkpoint(g);
        n += 1;
        if budget.is_zero() || start.elapsed() >= budget {
            break;
        }
    }
    n
}

fn deliver(g: *mut c_void, token: u32, ok: bool, payload: &str, make: Option<Maker>) {
    let dispatcher = DISPATCHER.with(|d| d.get());
    if dispatcher == 0 {
        return;
    }
    JsRuntime::with(|rt| {
        let value = match make {
            Some(m) => Value(m(g)),
            None => Value::undefined(),
        };
        let p = rt.ctx().string(payload);
        let a = [Value::number(f64::from(token)), Value::boolean(ok), p, value];
        if let Err(vn_runtime::redjs::api::Error::Exception(e)) = rt.ctx().call(Value(dispatcher), Value::undefined(), &a) {
            let err = rt.describe(e);
            eprintln!("[desktop dispatch] {}\n{}", err.header, err.stack);
        }
    });
}

pub(crate) fn log_later(kind: &'static str, text: String) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(Item::Log { kind, text });
    }
}

static LOG_SINK: Mutex<Option<fn(&'static str, String)>> = Mutex::new(None);

/// Installs where log-ring lines go (`vn_tree::log::record` in vn-app).
pub fn set_log_sink(sink: fn(&'static str, String)) {
    if let Ok(mut s) = LOG_SINK.lock() {
        *s = Some(sink);
    }
}

fn log_now(kind: &'static str, text: String) {
    let sink = LOG_SINK.lock().ok().and_then(|s| *s);
    if let Some(sink) = sink {
        sink(kind, text);
    }
}

/// A compat advisory: stderr, then the log ring. Main thread only; workers use [`advise_later`].
pub fn advise(msg: &str) {
    eprintln!("[vue-native compat] {msg}");
    log_now("advisory", format!("[vue-native compat] {msg}"));
}

pub(crate) fn advise_later(msg: &str) {
    eprintln!("[vue-native compat] {msg}");
    log_later("advisory", format!("[vue-native compat] {msg}"));
}

thread_local! {
    static DISPATCHER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn js_string(s: &str) -> u64 {
    JsRuntime::with(|rt| rt.ctx().string(s).0)
}

pub(crate) fn read_arg_string(g: *mut c_void, v: u64) -> String {
    unsafe { vn_runtime::redjs::webapi::util::read_string(g, v) }.unwrap_or_default()
}

host_fn!(fn vn_desktop_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    DISPATCHER.with(|d| d.set(f.0));
    Value::undefined().0
});

// `content` is the `<a href>` / `window.open` path: http, https and mailto only.
// `appShell.openExternal` takes any scheme but `file:`.
host_fn!(fn vn_shell_open_external(g, args) {
    let url = read_arg_string(g, args.arg(0).0);
    let content = args.arg(1).as_bool().unwrap_or(true);
    js_string(&shell::start_open_external(&url, content))
});

host_fn!(fn vn_shell_open_path(g, args) {
    let path = read_arg_string(g, args.arg(0).0);
    let allow = args.arg(1).as_bool().unwrap_or(false);
    js_string(&shell::start_open_path(&path, allow))
});

host_fn!(fn vn_shell_show_item(g, args) {
    let path = read_arg_string(g, args.arg(0).0);
    js_string(&shell::start_show_item(&path))
});

host_fn!(fn vn_shell_trash(g, args) {
    let path = read_arg_string(g, args.arg(0).0);
    js_string(&shell::start_trash(&path))
});

// IDs stay strings so native handles lose no bits.
host_fn!(fn vn_desktop_get_screen_sources(g, args) {
    let types = read_arg_string(g, args.arg(0).0);
    let options = read_arg_string(g, args.arg(1).0);
    js_string(&capture::start_sources(&types, &options))
});

fn arg_id(args: &vn_runtime::redjs::api::HostArgs, i: u32) -> u64 {
    args.arg(i).as_number().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u64).unwrap_or(0)
}

fn arg_offset(args: &vn_runtime::redjs::api::HostArgs, i: u32) -> Option<u64> {
    args.arg(i).as_number().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u64)
}

thread_local! {
    /// `(id, name, kind) => FileSystemHandle`, from desktop.ts.
    static HANDLE_FACTORY: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

// Also installs the `getAsFileSystemHandle` hook, so the method exists only when a factory does.
host_fn!(fn vn_fs_set_handle_factory(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    HANDLE_FACTORY.with(|h| h.set(f.0));
    unsafe { vn_runtime::redjs::webapi::uievents::set_file_system_handle_hook(g, Some(drop_handle_hook)) };
    Value::undefined().0
});

/// A handle for a dropped path, granted and made synchronously (the drop knows if it is a folder).
unsafe fn drop_handle_hook(g: *mut c_void, path: &std::path::Path, is_dir: bool) -> Result<u64, u64> {
    let kind = if is_dir { fs::Kind::Directory } else { fs::Kind::File };
    make_handle(g, &fs::granted_handle_json(path.to_path_buf(), kind))
}

unsafe fn make_handle(g: *mut c_void, wire: &serde_json::Value) -> Result<u64, u64> {
    use vn_runtime::redjs::webapi::util;
    let factory = HANDLE_FACTORY.with(|h| h.get());
    let id = wire["h"].as_u64().unwrap_or(0);
    let name = util::js_string(g, wire["name"].as_str().unwrap_or(""));
    let kind = util::js_string(g, wire["kind"].as_str().unwrap_or("file"));
    util::call(g, factory, Value::undefined().0, &[Value::number(id as f64).0, name, kind])
}

thread_local! {
    /// `(handle) => id | undefined`, from desktop.ts (the id lives in a WeakMap there).
    static HANDLE_ID_OF: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Blink's messages.
const NOT_CLONED: &str = "FileSystemHandle object could not be cloned.";
const NOT_DESERIALIZED: &str = "Unable to deserialize cloned data.";

/// `HostCodec::encode` for a `FileSystemHandle`: its path and kind, as bytes no script sees.
unsafe fn handle_encode(g: *mut c_void, v: u64) -> Result<Vec<u8>, String> {
    let id_of = HANDLE_ID_OF.with(|h| h.get());
    let id = vn_runtime::redjs::webapi::util::call(g, id_of, Value::undefined().0, &[v])
        .ok()
        .and_then(|n| Value(n).as_number())
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or_else(|| NOT_CLONED.to_string())?;
    fs::to_wire(id as u64).ok_or_else(|| NOT_CLONED.to_string())
}

/// `decode`: a new handle on the same path, granted only if a grant of this session covers it.
unsafe fn handle_decode(g: *mut c_void, bytes: &[u8]) -> Result<u64, String> {
    let (path, kind) = fs::from_wire(bytes).ok_or_else(|| NOT_DESERIALIZED.to_string())?;
    if HANDLE_FACTORY.with(|h| h.get()) == 0 {
        return Err(NOT_DESERIALIZED.to_string());
    }
    make_handle(g, &fs::handle_json(path, kind)).map_err(|_| NOT_DESERIALIZED.to_string())
}

// Makes handles [Serializable] as in Chromium. The first call wins, so an app cannot repoint it.
host_fn!(fn vn_fs_set_serializer(g, args) {
    if HANDLE_ID_OF.with(|h| h.get()) != 0 {
        return Value::undefined().0;
    }
    let f = args.arg(1);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    HANDLE_ID_OF.with(|h| h.set(f.0));
    let codec = vn_runtime::redjs::webapi::serialize::HostCodec { encode: handle_encode, decode: handle_decode };
    unsafe { vn_runtime::redjs::webapi::serialize::serialize_platform_prototype(g, args.arg(0).0, "FileSystemHandle", codec) };
    Value::undefined().0
});

// Sync lookup in the session's grants, no I/O.
host_fn!(fn vn_fs_permission(_g, args) {
    let state = fs::permission(arg_id(&args, 0), args.arg(1).as_bool().unwrap_or(false)).unwrap_or("prompt");
    js_string(state)
});

host_fn!(fn vn_fs_release(_g, args) {
    fs::release(arg_id(&args, 0));
    Value::undefined().0
});

host_fn!(fn vn_fs_get_file(_g, args) {
    js_string(&fs::start_get_file(arg_id(&args, 0)))
});

host_fn!(fn vn_fs_child(g, args) {
    let name = read_arg_string(g, args.arg(1).0);
    let kind = if args.arg(2).as_bool().unwrap_or(false) { fs::Kind::Directory } else { fs::Kind::File };
    js_string(&fs::start_child(arg_id(&args, 0), &name, kind, args.arg(3).as_bool().unwrap_or(false)))
});

host_fn!(fn vn_fs_remove(g, args) {
    let name = read_arg_string(g, args.arg(1).0);
    js_string(&fs::start_remove(arg_id(&args, 0), &name, args.arg(2).as_bool().unwrap_or(false)))
});

host_fn!(fn vn_fs_same(_g, args) {
    js_string(&fs::start_same(arg_id(&args, 0), arg_id(&args, 1)))
});

host_fn!(fn vn_fs_resolve(_g, args) {
    js_string(&fs::start_resolve(arg_id(&args, 0), arg_id(&args, 1)))
});

host_fn!(fn vn_fs_dir_open(_g, args) {
    js_string(&fs::start_dir_open(arg_id(&args, 0)))
});
host_fn!(fn vn_fs_dir_next(_g, args) {
    js_string(&fs::start_dir_next(arg_id(&args, 0)))
});
host_fn!(fn vn_fs_dir_close(_g, args) {
    fs::dir_close(arg_id(&args, 0));
    Value::undefined().0
});

host_fn!(fn vn_fs_writable_open(_g, args) {
    js_string(&fs::writable::start_open(arg_id(&args, 0), args.arg(1).as_bool().unwrap_or(false)))
});

// `data` is a string (UTF-8), a Uint8Array or a Blob. Bytes are copied here on the JS thread; a
// Blob's source is copied on the pool, under its snapshot.
host_fn!(fn vn_fs_writable_write(g, args) {
    let w = arg_id(&args, 0);
    let position = arg_offset(&args, 1);
    let v = args.arg(2).0;
    let data = unsafe {
        if let Some(range) = vn_runtime::redjs::webapi::blob::range_of(g, v) {
            fs::writable::Data::Blob(range)
        } else if let Some(bytes) = vn_runtime::redjs::webapi::util::bufferish(g, v) {
            fs::writable::Data::Bytes(bytes.to_vec())
        } else {
            fs::writable::Data::Bytes(read_arg_string(g, v).into_bytes())
        }
    };
    js_string(&fs::writable::start_write(w, position, data))
});

host_fn!(fn vn_fs_writable_seek(_g, args) {
    js_string(&fs::writable::start_seek(arg_id(&args, 0), arg_offset(&args, 1).unwrap_or(0)))
});
host_fn!(fn vn_fs_writable_truncate(_g, args) {
    js_string(&fs::writable::start_truncate(arg_id(&args, 0), arg_offset(&args, 1).unwrap_or(0)))
});
host_fn!(fn vn_fs_writable_close(_g, args) {
    js_string(&fs::writable::start_close(arg_id(&args, 0)))
});
host_fn!(fn vn_fs_writable_abort(_g, args) {
    js_string(&fs::writable::start_abort(arg_id(&args, 0)))
});

// structuredClone of an instance throws a DataCloneError naming the interface.
host_fn!(fn vn_fs_refuse_clone(g, args) {
    let name = read_arg_string(g, args.arg(1).0);
    unsafe { vn_runtime::redjs::webapi::serialize::refuse_platform_prototype(g, args.arg(0).0, &name) };
    Value::undefined().0
});

// `currentFiles` is an input's FileList; its paths are read here so "the same selection again" is
// decided without a path in JS.
host_fn!(fn vn_dialog_open(g, args) {
    let req = read_arg_string(g, args.arg(0).0);
    let prev = unsafe {
        vn_runtime::redjs::webapi::file::file_list_items(g, args.arg(1).0)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| vn_runtime::redjs::webapi::file::path_of(g, f))
            .collect()
    };
    js_string(&dialog::start_open(&req, prev))
});

// Save As for a `blob:` or `data:` URL, resolved at the click so `a.click(); URL.revokeObjectURL(url)`
// still saves.
host_fn!(fn vn_download(g, args) {
    let url = read_arg_string(g, args.arg(0).0);
    let attr = read_arg_string(g, args.arg(1).0);
    let (data, mime) = if url.starts_with("blob:") {
        match vn_runtime::redjs::webapi::url::resolve_blob_url(g, &url) {
            Some(t) => {
                let mime = t.mime().to_string();
                (dialog::data_of_blob_target(t), mime)
            }
            None => {
                return js_string(
                    &Fail::new("NotFoundError", "the blob: URL was revoked, or was never created here; nothing to save")
                        .code("ENOENT")
                        .to_sync_json(),
                )
            }
        }
    } else if url.starts_with("data:") {
        let f = vn_runtime::redjs::webapi::fsutil::fetch_url(g, &url);
        if !f.ok {
            return js_string(&Fail::type_error("the data: URL could not be decoded").to_sync_json());
        }
        (fs::writable::Data::Bytes(f.bytes), f.mime)
    } else {
        return js_string(&Fail::type_error("only blob: and data: URLs are saved as downloads").to_sync_json());
    };
    let name = dialog::download_name(&attr, &url, &mime);
    js_string(&dialog::start_download(data, name))
});

host_fn!(fn vn_launch_info(_g, _args) {
    js_string(&launch::info_json())
});

// Sync: three registry calls.
host_fn!(fn vn_launch_register_protocol(g, args) {
    let scheme = read_arg_string(g, args.arg(0).0);
    js_string(&launch::js_register(&scheme))
});

host_fn!(fn vn_launch_unregister_protocol(g, args) {
    let scheme = read_arg_string(g, args.arg(0).0);
    Value::boolean(launch::js_unregister(&scheme)).0
});

host_fn!(fn vn_launch_is_protocol_registered(g, args) {
    let scheme = read_arg_string(g, args.arg(0).0);
    Value::boolean(launch::js_is_registered(&scheme)).0
});

/// Registers the `__vn_desktop_*` / `__vn_shell_*` / `__vn_launch_*` / `__vn_fs_*` host functions.
pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_desktop_set_dispatcher", 1, vn_desktop_set_dispatcher as *const _),
        ("__vn_desktop_get_screen_sources", 2, vn_desktop_get_screen_sources as *const _),
        ("__vn_shell_open_external", 2, vn_shell_open_external as *const _),
        ("__vn_shell_open_path", 2, vn_shell_open_path as *const _),
        ("__vn_shell_show_item", 1, vn_shell_show_item as *const _),
        ("__vn_shell_trash", 1, vn_shell_trash as *const _),
        ("__vn_launch_info", 0, vn_launch_info as *const _),
        ("__vn_launch_register_protocol", 1, vn_launch_register_protocol as *const _),
        ("__vn_launch_unregister_protocol", 1, vn_launch_unregister_protocol as *const _),
        ("__vn_launch_is_protocol_registered", 1, vn_launch_is_protocol_registered as *const _),
        ("__vn_fs_set_handle_factory", 1, vn_fs_set_handle_factory as *const _),
        ("__vn_fs_release", 1, vn_fs_release as *const _),
        ("__vn_fs_get_file", 1, vn_fs_get_file as *const _),
        ("__vn_fs_child", 4, vn_fs_child as *const _),
        ("__vn_fs_remove", 3, vn_fs_remove as *const _),
        ("__vn_fs_same", 2, vn_fs_same as *const _),
        ("__vn_fs_resolve", 2, vn_fs_resolve as *const _),
        ("__vn_fs_dir_open", 1, vn_fs_dir_open as *const _),
        ("__vn_fs_dir_next", 1, vn_fs_dir_next as *const _),
        ("__vn_fs_dir_close", 1, vn_fs_dir_close as *const _),
        ("__vn_fs_writable_open", 2, vn_fs_writable_open as *const _),
        ("__vn_fs_writable_write", 3, vn_fs_writable_write as *const _),
        ("__vn_fs_writable_seek", 2, vn_fs_writable_seek as *const _),
        ("__vn_fs_writable_truncate", 2, vn_fs_writable_truncate as *const _),
        ("__vn_fs_writable_close", 1, vn_fs_writable_close as *const _),
        ("__vn_fs_writable_abort", 1, vn_fs_writable_abort as *const _),
        ("__vn_fs_refuse_clone", 2, vn_fs_refuse_clone as *const _),
        ("__vn_fs_set_serializer", 2, vn_fs_set_serializer as *const _),
        ("__vn_fs_permission", 2, vn_fs_permission as *const _),
        ("__vn_dialog_open", 2, vn_dialog_open as *const _),
        ("__vn_download", 2, vn_download as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
    let g = rt.raw();
    RT_G.store(g as usize, Ordering::Release);
    if let Some(id) = vn_runtime::redjs::context::id_of(g) {
        RT_ID.store(id.0, Ordering::Release);
        // Created now so a result landing before the first pump has a waker to set.
        let _ = vn_runtime::redjs::web::waker(id);
    }
    vn_runtime::redjs::web::register_pump_source(vn_runtime::redjs::web::PumpSource {
        inflight: src_inflight,
        ready: src_ready,
        drain: src_drain,
    });
}

/// Delivers settled calls and unsolicited events into JS, and worker log lines into the ring.
pub fn pump() {
    let dispatcher = DISPATCHER.with(|d| d.get());
    let batch: Vec<Item> = {
        let Ok(mut q) = QUEUE.try_lock() else { return };
        if q.is_empty() {
            return;
        }
        if dispatcher == 0 {
            // No JS consumer yet: keep the settles (a boot-time `launch` must still arrive).
            let mut logs = Vec::new();
            q.retain(|item| match item {
                Item::Log { kind, text } => {
                    logs.push(Item::Log { kind, text: text.clone() });
                    false
                }
                Item::Settle { .. } => true,
            });
            logs
        } else {
            let n = q.len().min(DRAIN_CAP);
            let batch: Vec<Item> = q.drain(..n).collect();
            let settles = batch.iter().filter(|i| matches!(i, Item::Settle { .. })).count();
            SETTLES_QUEUED.fetch_sub(settles, Ordering::AcqRel);
            batch
        }
    };
    let mut called = false;
    let g = JsRuntime::with(|rt| rt.raw());
    for item in batch {
        match item {
            Item::Log { kind, text } => log_now(kind, text),
            Item::Settle { token, ok, payload, make } => {
                called = true;
                deliver(g, token, ok, &payload, make);
            }
        }
    }
    if called {
        JsRuntime::with(|rt| rt.ctx().drain_microtasks());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn wait_for_settles(tokens: &[u32]) -> std::collections::HashMap<u32, (bool, String)> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut got = std::collections::HashMap::new();
        while got.len() < tokens.len() {
            got.extend(take_settled(tokens));
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {tokens:?}; have {got:?}");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        got
    }

    pub(crate) fn token_of(answer: &str) -> u32 {
        let v: serde_json::Value = serde_json::from_str(answer).unwrap();
        v["token"].as_u64().unwrap_or_else(|| panic!("not a token: {answer}")) as u32
    }

    #[test]
    fn a_child_lookup_settles_with_a_handle_and_never_a_path() {
        let dir = std::env::temp_dir().join(format!("vn-desktop-lib-child-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        let h = fs::register(dir.clone(), fs::Kind::Directory);
        fs::grant(&dir);
        let t = token_of(&fs::start_child(h, "a.txt", fs::Kind::File, false));
        let missing = token_of(&fs::start_child(h, "nope.txt", fs::Kind::File, false));
        let got = wait_for_settles(&[t, missing]);
        let (ok, payload) = &got[&t];
        assert!(ok);
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!((v["name"].as_str(), v["kind"].as_str()), (Some("a.txt"), Some("file")));
        assert!(!payload.contains(&*dir.to_string_lossy()), "no path in what JS sees: {payload}");
        fs::release(v["h"].as_u64().unwrap());
        fs::release(h);
        assert_eq!(got[&missing].0, false);
        assert!(got[&missing].1.contains("NotFoundError"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_serialises_both_ways() {
        let f = Fail::new("NotFoundError", "gone \"x\"").code("ENOENT").os(2);
        let v: serde_json::Value = serde_json::from_str(&f.to_json()).unwrap();
        assert_eq!(v["name"], "NotFoundError");
        assert_eq!(v["message"], "gone \"x\"");
        assert_eq!(v["code"], "ENOENT");
        assert_eq!(v["osError"], 2);
        let s: serde_json::Value = serde_json::from_str(&f.to_sync_json()).unwrap();
        assert_eq!(s["error"], "gone \"x\"");
        assert_eq!(s["name"], "NotFoundError");
        let t: serde_json::Value = serde_json::from_str(&Fail::type_error("bad").to_sync_json()).unwrap();
        assert!(t.get("code").is_none());
    }

    #[test]
    fn tokens_are_never_zero() {
        // A private counter: wrapping the shared one would hand parallel tests tokens in use.
        let counter = AtomicU32::new(u32::MAX);
        assert_eq!(next_from(&counter), u32::MAX);
        assert_eq!(next_from(&counter), 1);
    }
}
