//! The desktop surface: argv parser, shell (open/reveal/trash), single instance, custom protocols, File
//! System Access, dialogs and screen capture.
//!
//! Host work started from JS gets a token; a worker thread pushes `(token, ok, payload)` and the promise is
//! settled on the JS thread. Workers never touch JSC, so a JS value (a `File`, a `FileList`) is made by a
//! [`Maker`] at delivery. Token `0` is reserved for unsolicited host events (`launch`).
//!
//! Results are delivered inside the redjs pump budget through a `PumpSource`. Dialog and shell results
//! never count as in flight: a Save As left open must not make every frame wait.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use sk_runtime::JsRuntime;

pub mod args;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod badge;
pub mod capture;
pub mod dialog;
pub mod fs;
pub mod launch;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod media_session;
pub mod network;
mod pool;
pub mod power;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod share;
pub mod shell;
pub mod tray;
mod web;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod webauthn;

/// A failure JS turns into a rejection: `name` is a DOMException name or `TypeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub name: &'static str,
    pub message: String,
    pub code: Option<&'static str>,
    pub os_error: Option<i32>,
}

impl Fail {
    pub fn new(name: &'static str, message: impl Into<String>) -> Self {
        Fail {
            name,
            message: message.into(),
            code: None,
            os_error: None,
        }
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
}

pub(crate) type Maker = Box<dyn FnOnce(*mut c_void) -> u64 + Send>;

pub(crate) struct Done {
    pub payload: String,
    pub make: Option<Maker>,
}

impl Done {
    pub fn json(payload: &str) -> Done {
        Done {
            payload: payload.to_string(),
            make: None,
        }
    }
    pub fn with_value(
        payload: String,
        make: impl FnOnce(*mut c_void) -> u64 + Send + 'static,
    ) -> Done {
        Done {
            payload,
            make: Some(Box::new(make)),
        }
    }
}

enum Item {
    Settle {
        token: u32,
        ok: bool,
        payload: String,
        make: Option<Maker>,
    },
    Log {
        kind: &'static str,
        text: String,
    },
}

static QUEUE: Mutex<VecDeque<Item>> = Mutex::new(VecDeque::new());
static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);
static SETTLES_QUEUED: AtomicUsize = AtomicUsize::new(0);

const DRAIN_CAP: usize = 16;

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

pub(crate) fn settle(token: u32, result: Result<String, Fail>) {
    settle_done(
        token,
        result.map(|payload| Done {
            payload,
            make: None,
        }),
    );
}

pub(crate) fn settle_done(token: u32, result: Result<Done, Fail>) {
    push_settle(match result {
        Ok(Done { payload, make }) => Item::Settle {
            token,
            ok: true,
            payload,
            make,
        },
        Err(f) => Item::Settle {
            token,
            ok: false,
            payload: f.to_json(),
            make: None,
        },
    });
}

pub(crate) fn emit(payload: String) {
    push_settle(Item::Settle {
        token: 0,
        ok: true,
        payload,
        make: None,
    });
}

#[cfg(test)]
pub(crate) fn take_emitted() -> Vec<String> {
    let Ok(mut q) = QUEUE.lock() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    q.retain(|item| match item {
        Item::Settle {
            token: 0, payload, ..
        } => {
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
        Item::Settle {
            token, ok, payload, ..
        } if tokens.contains(token) => {
            out.insert(*token, (*ok, payload.clone()));
            SETTLES_QUEUED.fetch_sub(1, Ordering::AcqRel);
            false
        }
        _ => true,
    });
    out
}

#[cfg(test)]
pub(crate) fn take_settled_with_makers(tokens: &[u32]) -> Vec<(u32, bool, String, Option<Maker>)> {
    let mut out = Vec::new();
    let Ok(mut q) = QUEUE.lock() else { return out };
    let mut keep = VecDeque::new();
    for item in q.drain(..) {
        match item {
            Item::Settle {
                token,
                ok,
                payload,
                make,
            } if tokens.contains(&token) => {
                SETTLES_QUEUED.fetch_sub(1, Ordering::AcqRel);
                out.push((token, ok, payload, make));
            }
            other => keep.push_back(other),
        }
    }
    *q = keep;
    out
}

static RT_G: AtomicUsize = AtomicUsize::new(0);
static RT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn wake() {
    let id = RT_ID.load(Ordering::Acquire);
    if id != 0 {
        sk_runtime::redjs::web::wake(sk_runtime::redjs::context::ContextId(id));
    }
    sk_runtime::frames::wake();
}

fn is_ours(g: *mut c_void) -> bool {
    g as usize == RT_G.load(Ordering::Acquire) && !g.is_null()
}

unsafe fn src_inflight(g: *mut c_void) -> bool {
    is_ours(g) && pool::in_flight() != 0
}

unsafe fn src_ready(g: *mut c_void) -> bool {
    is_ours(g) && SETTLES_QUEUED.load(Ordering::Acquire) != 0
}

unsafe fn src_drain(g: *mut c_void) -> usize {
    if !src_ready(g) {
        return 0;
    }
    let budget = std::time::Duration::from_micros(sk_runtime::redjs::web::pump_budget_us() as u64);
    let start = std::time::Instant::now();
    let mut n = 0;
    loop {
        let item = {
            let Ok(mut q) = QUEUE.lock() else { break };
            let Some(i) = q.iter().position(|it| matches!(it, Item::Settle { .. })) else {
                break;
            };
            SETTLES_QUEUED.fetch_sub(1, Ordering::AcqRel);
            q.remove(i)
        };
        let Some(Item::Settle {
            token,
            ok,
            payload,
            make,
        }) = item
        else {
            break;
        };
        deliver(g, token, ok, &payload, make);
        sk_runtime::redjs::web::checkpoint(g);
        n += 1;
        if budget.is_zero() || start.elapsed() >= budget {
            break;
        }
    }
    n
}

fn deliver(g: *mut c_void, token: u32, ok: bool, payload: &str, make: Option<Maker>) {
    unsafe { web::deliver(g, token, ok, payload, make) };
}

pub(crate) fn log_later(kind: &'static str, text: String) {
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(Item::Log { kind, text });
    }
}

static LOG_SINK: Mutex<Option<fn(&'static str, String)>> = Mutex::new(None);

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

pub fn advise(msg: &str) {
    eprintln!("[skathi compat] {msg}");
    log_now("advisory", format!("[skathi compat] {msg}"));
}

pub(crate) fn advise_later(msg: &str) {
    eprintln!("[skathi compat] {msg}");
    log_later("advisory", format!("[skathi compat] {msg}"));
}

pub fn install(rt: &JsRuntime) {
    let g = rt.raw();
    unsafe { web::install(g) };
    RT_G.store(g as usize, Ordering::Release);
    if let Some(id) = sk_runtime::redjs::context::id_of(g) {
        RT_ID.store(id.0, Ordering::Release);
        let _ = sk_runtime::redjs::web::waker(id);
    }
    sk_runtime::redjs::web::register_pump_source(sk_runtime::redjs::web::PumpSource {
        inflight: src_inflight,
        ready: src_ready,
        drain: src_drain,
    });
}

pub fn pump() {
    launch::pump();
    let installed = RT_G.load(Ordering::Acquire) != 0;
    let batch: Vec<Item> = {
        let Ok(mut q) = QUEUE.try_lock() else { return };
        if q.is_empty() {
            return;
        }
        if !installed {
            let mut logs = Vec::new();
            q.retain(|item| match item {
                Item::Log { kind, text } => {
                    logs.push(Item::Log {
                        kind,
                        text: text.clone(),
                    });
                    false
                }
                Item::Settle { .. } => true,
            });
            logs
        } else {
            let n = q.len().min(DRAIN_CAP);
            let batch: Vec<Item> = q.drain(..n).collect();
            let settles = batch
                .iter()
                .filter(|i| matches!(i, Item::Settle { .. }))
                .count();
            SETTLES_QUEUED.fetch_sub(settles, Ordering::AcqRel);
            batch
        }
    };
    let mut called = false;
    let g = JsRuntime::with(|rt| rt.raw());
    for item in batch {
        match item {
            Item::Log { kind, text } => log_now(kind, text),
            Item::Settle {
                token,
                ok,
                payload,
                make,
            } => {
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

    pub(crate) fn wait_for_settles(
        tokens: &[u32],
    ) -> std::collections::HashMap<u32, (bool, String)> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut got = std::collections::HashMap::new();
        while got.len() < tokens.len() {
            got.extend(take_settled(tokens));
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {tokens:?}; have {got:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        got
    }

    pub(crate) fn token_of(started: Result<u32, Fail>) -> u32 {
        started.unwrap_or_else(|f| panic!("not started: {f:?}"))
    }

    #[test]
    fn a_child_lookup_settles_with_a_handle_and_never_a_path() {
        let dir = std::env::temp_dir().join(format!("sk-desktop-lib-child-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        let h = fs::register(dir.clone(), fs::Kind::Directory);
        fs::grant(&dir);
        let t = token_of(fs::start_child(h, "a.txt", fs::Kind::File, false));
        let missing = token_of(fs::start_child(h, "nope.txt", fs::Kind::File, false));
        let got = wait_for_settles(&[t, missing]);
        let (ok, payload) = &got[&t];
        assert!(ok);
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(
            (v["name"].as_str(), v["kind"].as_str()),
            (Some("a.txt"), Some("file"))
        );
        assert!(
            !payload.contains(&*dir.to_string_lossy()),
            "no path in what JS sees: {payload}"
        );
        fs::release(v["h"].as_u64().unwrap());
        fs::release(h);
        assert_eq!(got[&missing].0, false);
        assert!(got[&missing].1.contains("NotFoundError"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_serialises() {
        let f = Fail::new("NotFoundError", "gone \"x\"")
            .code("ENOENT")
            .os(2);
        let v: serde_json::Value = serde_json::from_str(&f.to_json()).unwrap();
        assert_eq!(v["name"], "NotFoundError");
        assert_eq!(v["message"], "gone \"x\"");
        assert_eq!(v["code"], "ENOENT");
        assert_eq!(v["osError"], 2);
        let t: serde_json::Value =
            serde_json::from_str(&Fail::type_error("bad").to_json()).unwrap();
        assert!(t.get("code").is_none());
    }

    #[test]
    fn tokens_are_never_zero() {
        let counter = AtomicU32::new(u32::MAX);
        assert_eq!(next_from(&counter), u32::MAX);
        assert_eq!(next_from(&counter), 1);
    }
}
