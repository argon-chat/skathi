//! vn-desktop — the desktop surface: the argv parser, the shell, single
//! instance and custom protocols (docs/SHELL.md, docs/LAUNCH.md;
//! research/reports/m25/desktop-apis-design.md, track A).
//!
//! ```text
//!   apps  ──<a href="https://…">, window.open(url)───────────┐ tier 1: the web's words
//!         ──launchQueue.setConsumer(…)─────────────────────┐ │
//!         ──import { appShell, appLaunch } from "vue-native"┼─┤ tier 2: what a browser cannot do
//!                                                          │ │
//!   packages/vue-native/src/runtime.ts ──__vn_shell_* / __vn_launch_*──┐
//!                                                                     ▼
//!   vn_desktop ── shell thread (STA) ── pipe server ── registry (HKCU) ──▶ results queue
//!        ▲                                                                   │
//!        └──────── pump() in vn-app's pump_frame: ONE dispatcher ◀───────────┘
//! ```
//!
//! # The settle protocol (permissions-shaped, design §3.2)
//!
//! Every async host fn answers SYNCHRONOUSLY with `{token}` or `{error, name}`.
//! The worker (the shell thread, the pipe server) pushes `(token, ok, payload)`
//! and [`pump`] — beside `vn_permissions::pump()` in vn-app's `pump_frame`,
//! inside the same Commit publication — calls the one JS dispatcher
//! `(token, ok, payloadJson)` and drains microtasks, so a Vue patch triggered by
//! `await appShell.openPath(…)` lands in that same frame. Token `0` is reserved
//! for UNSOLICITED host events (`launch`), with `payload.kind` naming them.
//!
//! # Threads (the review checklist)
//!
//! | thread | does | never |
//! |---|---|---|
//! | frame (JS + gpui) | validate arguments; registry reads/writes (µs, sync like `appStorage`) | stat a user path; ShellExecute; any modal loop |
//! | `vn-shell` (one STA) | `ShellExecuteExW`, `SHOpenFolderAndSelectItems`, `IFileOperation`, the `stat` in front of each | touch JSC |
//! | `vn-instance` (pipe server) | accept, verify the peer, read one launch message, raise | touch JSC |

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use vn_runtime::redjs::api::Value;
use vn_runtime::{host_fn, JsRuntime};

pub mod args;
pub mod shell;

// ---------------------------------------------------------------------------
// failures, as they cross into JS
// ---------------------------------------------------------------------------

/// A failure, in the shape JS turns into a rejection: `name` is a DOMException-
/// style name (`NotFoundError`, `NotAllowedError`, …) or `TypeError`; `code` is
/// the Node-style string an app can switch on; `os_error` the raw Win32 code.
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

    /// `{name, message, code?, osError?}` — the async rejection payload.
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

    /// `{error, name, code?, osError?}` — the SYNCHRONOUS refusal a host fn
    /// answers instead of a token.
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

// ---------------------------------------------------------------------------
// the results queue
// ---------------------------------------------------------------------------

enum Item {
    Settle { token: u32, ok: bool, payload: String },
    /// A line for the devtools log ring, which is the MAIN thread's: a worker
    /// cannot write it, so it rides the queue like a result does.
    Log { kind: &'static str, text: String },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);

/// How many items one frame may deliver. A launch storm or a shell queue that
/// finishes in a burst must not lengthen one frame without bound.
const DRAIN_CAP: usize = 16;

/// A fresh token. Never 0 — that is the unsolicited-event channel.
pub(crate) fn next_token() -> u32 {
    loop {
        let t = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        if t != 0 {
            return t;
        }
    }
}

/// Settles `token` from any thread.
pub(crate) fn settle(token: u32, result: Result<String, Fail>) {
    let item = match result {
        Ok(payload) => Item::Settle { token, ok: true, payload },
        Err(f) => Item::Settle { token, ok: false, payload: f.to_json() },
    };
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(item);
    }
}

/// Queues an unsolicited event (token 0) from any thread.
#[allow(dead_code)] // the instance pipe's `launch` (single instance lands next)
pub(crate) fn emit(payload: String) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(Item::Settle { token: 0, ok: true, payload });
    }
}

/// Queues a log-ring line from any thread; [`pump`] writes it on the main one.
pub(crate) fn log_later(kind: &'static str, text: String) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(Item::Log { kind, text });
    }
}

// ---------------------------------------------------------------------------
// the log sink — vn-app hands us `vn_tree::log::record`
// ---------------------------------------------------------------------------

static LOG_SINK: Mutex<Option<fn(&'static str, String)>> = Mutex::new(None);

/// Installs where log-ring lines go (`vn_tree::log::record` in vn-app). This
/// crate does not depend on vn-tree; the ring is one call away regardless.
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

/// A compat advisory from Rust: stderr first (it is unbuffered, and a GUI exe
/// may have nothing else), then the log ring. MAIN THREAD only — a worker uses
/// [`advise_later`].
pub fn advise(msg: &str) {
    eprintln!("[vue-native compat] {msg}");
    log_now("advisory", format!("[vue-native compat] {msg}"));
}

/// [`advise`], from any thread.
#[allow(dead_code)] // the instance pipe's peer-check advisories (next)
pub(crate) fn advise_later(msg: &str) {
    eprintln!("[vue-native compat] {msg}");
    log_later("advisory", format!("[vue-native compat] {msg}"));
}

// ---------------------------------------------------------------------------
// host functions
// ---------------------------------------------------------------------------

thread_local! {
    /// The one protected JS callback, exactly as vn-permissions' dispatcher.
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

// `__vn_shell_open_external(url, content) -> {token} | {error, name}`.
// `content` is the TIER-1 path (`<a href>`, `window.open`): http, https and
// mailto only. Tier 2 (`appShell.openExternal`) takes any scheme but `file:`.
host_fn!(fn vn_shell_open_external(g, args) {
    let url = read_arg_string(g, args.arg(0).0);
    let content = args.arg(1).as_bool().unwrap_or(true);
    js_string(&shell::start_open_external(&url, content))
});

// `__vn_shell_open_path(path, allowExecutable) -> {token} | {error, name}`.
host_fn!(fn vn_shell_open_path(g, args) {
    let path = read_arg_string(g, args.arg(0).0);
    let allow = args.arg(1).as_bool().unwrap_or(false);
    js_string(&shell::start_open_path(&path, allow))
});

// `__vn_shell_show_item(path) -> {token} | {error, name}`.
host_fn!(fn vn_shell_show_item(g, args) {
    let path = read_arg_string(g, args.arg(0).0);
    js_string(&shell::start_show_item(&path))
});

// `__vn_shell_trash(path) -> {token} | {error, name}`.
host_fn!(fn vn_shell_trash(g, args) {
    let path = read_arg_string(g, args.arg(0).0);
    js_string(&shell::start_trash(&path))
});

/// Registers the `__vn_desktop_*` / `__vn_shell_*` surface. Called from vn-app's
/// boot beside `vn_permissions::install`, before any app code runs.
pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_desktop_set_dispatcher", 1, vn_desktop_set_dispatcher as *const _),
        ("__vn_shell_open_external", 2, vn_shell_open_external as *const _),
        ("__vn_shell_open_path", 2, vn_shell_open_path as *const _),
        ("__vn_shell_show_item", 1, vn_shell_show_item as *const _),
        ("__vn_shell_trash", 1, vn_shell_trash as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

/// Delivers settled calls and unsolicited events into JS, and worker log lines
/// into the ring. In vn-app's `pump_frame`, inside the Commit publication, NOT
/// dev-gated (an app opening a link is application work).
pub fn pump() {
    let dispatcher = DISPATCHER.with(|d| d.get());
    let batch: Vec<Item> = {
        let Ok(mut q) = QUEUE.try_lock() else { return };
        if q.is_empty() {
            return;
        }
        if dispatcher == 0 {
            // No JS consumer yet (the bundle has not run): keep the settles —
            // a `launch` that arrived during boot must still be delivered — and
            // flush only the log lines.
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
            q.drain(..n).collect()
        }
    };
    let mut called = false;
    for item in batch {
        match item {
            Item::Log { kind, text } => log_now(kind, text),
            Item::Settle { token, ok, payload } => {
                called = true;
                JsRuntime::with(|rt| {
                    let p = rt.ctx().string(&payload);
                    let a = [Value::number(f64::from(token)), Value::boolean(ok), p];
                    if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
                        rt.ctx().call(Value(dispatcher), Value::undefined(), &a)
                    {
                        let err = rt.describe(e);
                        eprintln!("[desktop dispatch] {}\n{}", err.header, err.stack);
                    }
                });
            }
        }
    }
    if called {
        // The awaiting `.then()` and the Vue patch it triggers land inside this
        // same Commit publication — the rule vn-voice::pump documents.
        JsRuntime::with(|rt| rt.ctx().drain_microtasks());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        NEXT_TOKEN.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(next_token(), u32::MAX);
        assert_ne!(next_token(), 0);
    }
}
