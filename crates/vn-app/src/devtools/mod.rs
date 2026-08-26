//! vn-devtools — programmatic control of a running dev host (docs/DEVTOOLS.md).
//!
//! ```text
//!  ctl.ts / mcp.ts ──TCP──▶ listener thread ──queue──▶ pump_frame (MAIN)
//!                    ◀──────── reply channel ◀────────  one cmd / frame
//! ```
//!
//! The listener thread NEVER touches JSC, gpui or the retained tree: it reads a
//! newline-delimited JSON request, checks the token, pushes `(request, reply
//! channel)` onto a mutex-guarded queue and blocks on the channel. [`pump`]
//! runs on the gpui main thread inside the frame pump's own
//! `with_published(Phase::Commit)` publication and executes exactly one command
//! per frame, so every command sees a fully committed tree — the same invariant
//! the HMR transport relies on.
//!
//! Discovery is `<dist>/devtools.json` (`{port, pid, token}`), written through
//! the same `VN_DIST`-aware [`crate::dev::dist`] helper the rest of the dev
//! transport uses: every parallel host is independently addressable through its
//! own dist slot, and a client that finds a stale file gets a connection error
//! rather than talking to somebody else's window.
//!
//! The whole module is compiled out under `--features prod`.

mod screenshot;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

use gpui::{App, Window};
use vn_runtime::{with_commit, JsRuntime};

use crate::json::{self, Json, Raw};

/// How long the listener thread waits for the main thread before giving the
/// client an error. Generous — the client's own timeout (10 s in ctl.ts) is the
/// one that matters; this only exists so a wedged pump cannot leak threads
/// blocked forever on a channel.
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A queued request plus the channel its reply goes back on.
struct Job {
    req: Json,
    reply: Sender<String>,
}

static QUEUE: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
static TOKEN: Mutex<String> = Mutex::new(String::new());

// ---------------------------------------------------------------------------
// boot
// ---------------------------------------------------------------------------

/// Binds `127.0.0.1:0`, publishes `<dist>/devtools.json` and starts the
/// listener thread. Any failure is reported and then ignored: a dev host that
/// cannot open a devtools port must still run the app.
pub fn start() {
    vn_tree::log::enable();
    let listener = match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[devtools] cannot bind 127.0.0.1:0: {e} — devtools disabled");
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            eprintln!("[devtools] cannot read the bound port: {e} — devtools disabled");
            return;
        }
    };
    let token = fresh_token();
    *TOKEN.lock().unwrap() = token.clone();

    let path = crate::dev::dist("devtools.json");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let doc = Json::obj(vec![
        ("port", Json::n(f64::from(port))),
        ("pid", Json::n(std::process::id() as f64)),
        ("token", Json::s(&token)),
    ]);
    // Overwrites whatever a previous (possibly dead) host left behind — the
    // file is a "who is live on this dist slot right now" record, not history.
    if let Err(e) = std::fs::write(&path, format!("{}\n", doc.to_json())) {
        eprintln!("[devtools] cannot write {}: {e} — devtools disabled", path.display());
        return;
    }
    eprintln!(
        "[devtools] listening on 127.0.0.1:{port} — {} (pid {})",
        path.display(),
        std::process::id()
    );
    vn_tree::log::record("dev", format!("[devtools] listening on 127.0.0.1:{port}"));

    std::thread::Builder::new()
        .name("vn-devtools".into())
        .spawn(move || accept_loop(listener))
        .ok();
}

// ---------------------------------------------------------------------------
// the JS side: console tee + the eval helper's return channel
// ---------------------------------------------------------------------------

/// Tees `console.*` into the log ring WITHOUT changing what it already does —
/// the original method still runs, so a developer watching the terminal sees
/// exactly what they saw before. (Compat-first: devtools adds a reader, it
/// never rewrites the app's observable behavior.)
const CONSOLE_TEE_JS: &str = r#"
(function () {
  if (!globalThis.console) return;
  var fmt = function (v) {
    if (typeof v === "string") return v;
    try { var s = JSON.stringify(v); return s === undefined ? String(v) : s; }
    catch (e) { return String(v); }
  };
  ["log", "info", "warn", "error", "debug"].forEach(function (name) {
    var orig = console[name];
    if (typeof orig !== "function") return;
    console[name] = function () {
      var args = Array.prototype.slice.call(arguments);
      try { __vn_devtools_console(name, args.map(fmt).join(" ")); } catch (e) {}
      return orig.apply(console, args);
    };
  });
})();
"#;

vn_runtime::host_fn!(fn vn_devtools_console(g, args) {
    let level = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(1).0) }
        .unwrap_or_default();
    let kind: &'static str = match level.as_str() {
        "warn" => "warn",
        "error" => "error",
        "debug" => "debug",
        "info" => "info",
        _ => "log",
    };
    vn_tree::log::record(kind, text);
    vn_runtime::redjs::api::Value::undefined().0
});

vn_runtime::host_fn!(fn vn_devtools_emit(g, args) {
    let text = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    set_eval_result(text);
    vn_runtime::redjs::api::Value::undefined().0
});

/// Registers the two devtools host fns. Call before the app bundle boots.
pub fn install(rt: &JsRuntime) {
    rt.register("__vn_devtools_console", 2, vn_devtools_console as *const _);
    rt.register("__vn_devtools_emit", 1, vn_devtools_emit as *const _);
    if let Err(e) = rt.eval(CONSOLE_TEE_JS, "vn://devtools-console.js") {
        eprintln!("[devtools] console tee failed: {}\n{}", e.header, e.stack);
    }
}

/// 128 bits of process-local entropy, hex. No `rand` dependency for a token
/// whose only job is to stop an unrelated local process from driving the app:
/// the address of a fresh allocation, the pid, and two clock reads taken apart
/// from each other are plenty of unpredictability for that threat model.
fn fresh_token() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut h = RandomState::new().build_hasher();
    h.write_u64(std::process::id() as u64);
    h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
    let a = h.finish();
    // A second, independently seeded hasher: `RandomState` reseeds per instance.
    let mut h2 = RandomState::new().build_hasher();
    h2.write_u64(a);
    h2.write_usize(&h as *const _ as usize);
    format!("{a:016x}{:016x}", h2.finish())
}

fn accept_loop(listener: TcpListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                std::thread::Builder::new()
                    .name("vn-devtools-conn".into())
                    .spawn(move || serve(s))
                    .ok();
            }
            Err(e) => {
                eprintln!("[devtools] accept failed: {e}");
                return;
            }
        }
    }
}

/// One connection: newline-delimited JSON in, newline-delimited JSON out. A
/// client may pipeline several commands down one socket; each still costs one
/// frame, in order.
fn serve(stream: TcpStream) {
    let Ok(write_half) = stream.try_clone() else { return };
    let mut out = write_half;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let response = match json::parse(&line) {
            None => err("request is not valid JSON"),
            Some(req) => {
                let ok_token = req
                    .get("token")
                    .and_then(Json::as_str)
                    .is_some_and(|t| t == TOKEN.lock().unwrap().as_str());
                if !ok_token {
                    err("bad or missing token — read it from <dist>/devtools.json")
                } else {
                    submit(req)
                }
            }
        };
        if out.write_all(response.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
            return;
        }
        let _ = out.flush();
    }
}

/// Hands the request to the main thread and waits for the reply.
fn submit(req: Json) -> String {
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    QUEUE.lock().unwrap().push_back(Job { req, reply: tx });
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(reply) => reply,
        Err(_) => err(
            "the app's frame pump did not run a queued command within 60s — the main thread is \
             wedged (a modal drag loop, a blocked JS call, or a caught native panic)",
        ),
    }
}

fn err(msg: impl Into<String>) -> String {
    Json::obj(vec![("ok", Json::Bool(false)), ("error", Json::s(msg))]).to_json()
}

fn ok(data: Json) -> String {
    Json::obj(vec![("ok", Json::Bool(true)), ("data", data)]).to_json()
}

// ---------------------------------------------------------------------------
// main-thread execution
// ---------------------------------------------------------------------------

thread_local! {
    /// A command that could not finish in the frame it started (see
    /// [`Pending`]). While one is parked, no new command is dequeued — the
    /// "one command per frame" rule stays a strict ordering guarantee.
    static PENDING: std::cell::RefCell<Option<Pending>> = const { std::cell::RefCell::new(None) };
}

/// `type` and `key` need the target focused AND painted before a keystroke can
/// reach it: gpui installs the native input handler during the input widget's
/// PAINT, and only when its handle is already focused. So the command focuses
/// on frame N, gpui repaints, and the keystrokes go out on frame N+1.
struct Pending {
    node: u32,
    text: Option<String>,
    keys: Vec<String>,
    reply: Sender<String>,
    waited: u32,
}

/// Frames a pending command will wait for focus before giving up. ~0.5 s at
/// 60 fps; a repaint normally lands on the very next frame.
const PENDING_MAX_FRAMES: u32 = 30;

/// Called once per frame from `pump_frame`, INSIDE its
/// `with_published(Phase::Commit)` — so `with_commit` below always resolves and
/// every command observes the same committed tree the JS pump just produced.
pub fn pump() {
    let resumed = PENDING.with(|p| p.borrow_mut().take()).map(resume_pending);
    if let Some(still_pending) = resumed {
        PENDING.with(|p| *p.borrow_mut() = still_pending);
        return;
    }
    let Some(job) = QUEUE.lock().unwrap().pop_front() else { return };
    let Job { req, reply } = job;
    let response = run(&req, &reply);
    match response {
        Outcome::Done(text) => {
            let _ = reply.send(text);
        }
        Outcome::Parked(pending) => PENDING.with(|p| *p.borrow_mut() = Some(pending)),
    }
}

enum Outcome {
    Done(String),
    Parked(Pending),
}

fn resume_pending(mut p: Pending) -> Option<Pending> {
    let focused = with_commit(|window, cx| vn_tree::devtools::is_focused(p.node, window, cx))
        .unwrap_or(false);
    if !focused {
        p.waited += 1;
        if p.waited < PENDING_MAX_FRAMES {
            return Some(p);
        }
        let _ = p.reply.send(err(format!(
            "node {} never took focus within {PENDING_MAX_FRAMES} frames — is it disabled, or \
             detached from the painted tree?",
            p.node
        )));
        return None;
    }
    let reply = with_commit(|window, cx| {
        let mut delivered = 0usize;
        if let Some(text) = &p.text {
            for ch in text.chars() {
                if !type_char(ch, window, cx) {
                    break;
                }
                delivered += 1;
            }
        }
        for k in &p.keys {
            match parse_keystroke(k) {
                Ok(ks) => {
                    send_keystroke(ks, window, cx);
                    delivered += 1;
                }
                Err(e) => return err(e),
            }
        }
        // The edit path reports through `on_change` on a `Window::defer`, so
        // the JS `input` event lands after this closure; flushing here is what
        // makes the very next command observe the updated tree.
        if vn_tree::flush_changes() {
            window.refresh();
        }
        ok(Json::obj(vec![
            ("node", Json::n(f64::from(p.node))),
            ("delivered", Json::n(delivered as f64)),
            (
                "value",
                match vn_tree::devtools::input_value(p.node, cx) {
                    Some(v) => Json::s(v),
                    None => Json::Null,
                },
            ),
        ]))
    })
    .unwrap_or_else(|| err("not in a commit phase"));
    let _ = p.reply.send(reply);
    None
}

fn run(req: &Json, reply: &Sender<String>) -> Outcome {
    let cmd = req.get("cmd").and_then(Json::as_str).unwrap_or("");
    let arg = |name: &str| req.get(name).and_then(Json::as_str).map(str::to_string);
    let num = |name: &str| req.get(name).and_then(Json::as_f64);
    let text = match cmd {
        "ping" => ok(Json::obj(vec![
            ("pid", Json::n(std::process::id() as f64)),
            ("root", Json::n(f64::from(vn_tree::devtools::root_id()))),
        ])),
        "query" => match arg("selector") {
            None => err("query needs a `selector`"),
            Some(sel) => cmd_query(&sel),
        },
        "text" => match arg("selector") {
            None => err("text needs a `selector`"),
            Some(sel) => cmd_text(&sel, num("index")),
        },
        "tree" => cmd_tree(arg("selector").as_deref(), num("depth")),
        "styles" => match arg("selector") {
            None => err("styles needs a `selector`"),
            Some(sel) => cmd_styles(&sel, num("index")),
        },
        "click" => match arg("selector") {
            None => err("click needs a `selector`"),
            Some(sel) => cmd_click(&sel, num("index"), num("button")),
        },
        "scroll" => match arg("selector") {
            None => err("scroll needs a `selector`"),
            Some(sel) => cmd_scroll(&sel, num("index"), num("dx"), num("dy")),
        },
        "focus" => match arg("selector") {
            None => err("focus needs a `selector`"),
            Some(sel) => cmd_focus(&sel, num("index")),
        },
        "eval" => match arg("js") {
            None => err("eval needs `js`"),
            Some(js) => cmd_eval(&js),
        },
        "logs" => cmd_logs(num("since")),
        "screenshot" => cmd_screenshot(arg("path").as_deref()),
        "type" | "key" => return start_input_command(cmd, req, reply),
        "" => err("no `cmd` in the request"),
        other => err(format!("unknown command `{other}` — see docs/DEVTOOLS.md")),
    };
    Outcome::Done(text)
}

// ---------------------------------------------------------------------------
// selector resolution
// ---------------------------------------------------------------------------

/// Resolves a selector to exactly one node. Ambiguity is an ERROR listing the
/// candidates, not a silent "first match" — an agent that meant one thing and
/// hit another has no way to notice otherwise. `index` disambiguates.
fn resolve_one(selector: &str, index: Option<f64>) -> Result<u32, String> {
    let (ids, _warnings) = vn_tree::devtools::query(selector)?;
    match (ids.len(), index) {
        (0, _) => Err(format!("`{selector}` matched no nodes")),
        (_, Some(i)) => {
            let i = i as usize;
            ids.get(i).copied().ok_or_else(|| {
                format!("--index {i} is out of range: `{selector}` matched {} nodes", ids.len())
            })
        }
        (1, None) => Ok(ids[0]),
        (n, None) => Err(format!(
            "`{selector}` matched {n} nodes — pass --index to pick one: {}",
            ids.iter()
                .enumerate()
                .map(|(i, &id)| format!("[{i}] {}", short(id)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `tag.class.class#id` — enough to tell two candidates apart in an error.
fn short(id: u32) -> String {
    match vn_tree::devtools::node_info(id) {
        None => format!("#{id}"),
        Some(n) => {
            let mut s = n.tag;
            for c in &n.classes {
                s.push('.');
                s.push_str(c);
            }
            format!("{s} (id {id})")
        }
    }
}

fn bounds_json(b: Option<(f32, f32, f32, f32)>) -> Json {
    match b {
        None => Json::Null,
        Some((x, y, w, h)) => Json::obj(vec![
            ("x", Json::n(x)),
            ("y", Json::n(y)),
            ("w", Json::n(w)),
            ("h", Json::n(h)),
        ]),
    }
}

fn node_json(id: u32, with_text: bool) -> Json {
    let Some(n) = vn_tree::devtools::node_info(id) else {
        return Json::obj(vec![("id", Json::n(f64::from(id))), ("gone", Json::Bool(true))]);
    };
    let mut entries = vec![
        ("id", Json::n(f64::from(id))),
        ("tag", Json::s(n.tag)),
        ("classes", Json::arr(n.classes.into_iter().map(Json::Str).collect())),
    ];
    if with_text {
        let text = vn_tree::devtools::text_content(id).unwrap_or_default();
        entries.push(("text", Json::s(text)));
    }
    entries.push(("bounds", bounds_json(n.bounds)));
    entries.push(("scopes", Json::arr(n.scopes.into_iter().map(Json::Str).collect())));
    Json::obj(entries)
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

fn cmd_query(selector: &str) -> String {
    match vn_tree::devtools::query(selector) {
        // The selector audit's advisory IS the error — same words a rule using
        // the same selector would get out of the sheet compiler.
        Err(advisory) => {
            vn_tree::log::record("advisory", format!("[query] {advisory}"));
            err(advisory)
        }
        Ok((ids, warnings)) => {
            let mut entries = vec![
                ("count", Json::n(ids.len() as f64)),
                ("nodes", Json::arr(ids.iter().map(|&id| node_json(id, true)).collect())),
            ];
            if !warnings.is_empty() {
                entries.push(("warnings", Json::arr(warnings.into_iter().map(Json::Str).collect())));
            }
            ok(Json::obj(entries))
        }
    }
}

fn cmd_text(selector: &str, index: Option<f64>) -> String {
    match resolve_one(selector, index) {
        Err(e) => err(e),
        Ok(id) => {
            // A native <input>'s content lives in the widget, not in a child
            // text node — `textContent` would be "" and that would be a lie.
            let live = with_commit(|_, cx| vn_tree::devtools::input_value(id, cx)).flatten();
            let text = live
                .clone()
                .or_else(|| vn_tree::devtools::text_content(id))
                .unwrap_or_default();
            ok(Json::obj(vec![
                ("id", Json::n(f64::from(id))),
                ("text", Json::s(text)),
                ("source", Json::s(if live.is_some() { "input-widget" } else { "textContent" })),
            ]))
        }
    }
}

fn cmd_tree(selector: Option<&str>, depth: Option<f64>) -> String {
    let root = match selector {
        None => vn_tree::devtools::root_id(),
        Some(sel) => match resolve_one(sel, None) {
            Ok(id) => id,
            Err(e) => return err(e),
        },
    };
    if root == 0 {
        return err("nothing is mounted yet");
    }
    let max = depth.map(|d| d as i32).unwrap_or(6);
    fn dump(id: u32, left: i32) -> Json {
        let Some(n) = vn_tree::devtools::node_info(id) else {
            return Json::Null;
        };
        let mut entries = vec![
            ("id", Json::n(f64::from(id))),
            ("kind", Json::s(n.kind)),
            ("tag", Json::s(n.tag)),
        ];
        if !n.classes.is_empty() {
            entries.push(("classes", Json::arr(n.classes.into_iter().map(Json::Str).collect())));
        }
        if !n.props.is_empty() {
            entries.push((
                "props",
                Json::Obj(n.props.into_iter().map(|(k, v)| (k, Json::Str(v))).collect()),
            ));
        }
        if !n.scopes.is_empty() {
            entries.push(("scopes", Json::arr(n.scopes.into_iter().map(Json::Str).collect())));
        }
        if !n.listeners.is_empty() {
            entries.push((
                "listeners",
                Json::arr(n.listeners.into_iter().map(Json::s).collect()),
            ));
        }
        if !n.text.is_empty() {
            entries.push(("text", Json::s(n.text)));
        }
        entries.push(("bounds", bounds_json(n.bounds)));
        if !n.children.is_empty() {
            if left <= 0 {
                entries.push(("truncated", Json::n(n.children.len() as f64)));
            } else {
                entries.push((
                    "children",
                    Json::arr(n.children.into_iter().map(|c| dump(c, left - 1)).collect()),
                ));
            }
        }
        Json::obj(entries)
    }
    ok(dump(root, max))
}

fn cmd_styles(selector: &str, index: Option<f64>) -> String {
    match resolve_one(selector, index) {
        Err(e) => err(e),
        Ok(id) => match vn_tree::devtools::computed_styles(id) {
            None => err(format!(
                "node {id} has no computed style — it is a text/anchor node, or the cascade has \
                 not run since it was created"
            )),
            // One JSON string per SOURCE line: a pretty `Debug` dump embedded
            // whole would arrive as a single `"…\n…\n…"` blob that no reader
            // can skim. An array of lines pretty-prints one line per entry.
            Some(buckets) => ok(Json::obj(vec![
                ("id", Json::n(f64::from(id))),
                (
                    "computed",
                    Json::Obj(
                        buckets
                            .into_iter()
                            .map(|(k, v)| {
                                (
                                    k.to_string(),
                                    Json::arr(v.lines().map(Json::s).collect()),
                                )
                            })
                            .collect(),
                    ),
                ),
            ])),
        },
    }
}

fn cmd_click(selector: &str, index: Option<f64>, button: Option<f64>) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let Some(info) = vn_tree::devtools::node_info(id) else {
        return err(format!("node {id} vanished between the query and the click"));
    };
    let Some((x, y, w, h)) = info.bounds else {
        return err(format!(
            "node {id} has no bounds — it was not painted in the last frame (detached, or parked \
             in a <KeepAlive> cache), so there is no point to click"
        ));
    };
    let (cx_, cy) = (x + w / 2.0, y + h / 2.0);
    let proceed = with_commit(|window, app| {
        vn_tree::devtools::click(id, cx_, cy, button.unwrap_or(0.0), window, app)
    });
    match proceed {
        None => err("not in a commit phase"),
        Some(proceed) => ok(Json::obj(vec![
            ("id", Json::n(f64::from(id))),
            ("at", Json::obj(vec![("x", Json::n(cx_)), ("y", Json::n(cy))])),
            ("defaultPrevented", Json::Bool(!proceed)),
        ])),
    }
}

fn cmd_scroll(selector: &str, index: Option<f64>, dx: Option<f64>, dy: Option<f64>) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let out = with_commit(|window, app| {
        vn_tree::devtools::scroll(id, dx.unwrap_or(0.0) as f32, dy.unwrap_or(0.0) as f32, window, app)
    });
    match out {
        None => err("not in a commit phase"),
        Some(Err(e)) => err(e),
        Some(Ok((left, top))) => ok(Json::obj(vec![
            ("id", Json::n(f64::from(id))),
            ("scrollLeft", Json::n(left)),
            ("scrollTop", Json::n(top)),
        ])),
    }
}

fn cmd_focus(selector: &str, index: Option<f64>) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    match with_commit(|window, app| vn_tree::devtools::focus(id, window, app)) {
        None => err("not in a commit phase"),
        Some(Err(e)) => err(e),
        Some(Ok(())) => ok(Json::obj(vec![("id", Json::n(f64::from(id)))])),
    }
}

// ---------------------------------------------------------------------------
// eval
// ---------------------------------------------------------------------------

thread_local! {
    /// Where `__vn_devtools_emit` parks the JSON the eval helper produced. A
    /// host fn cannot return a string to Rust, so the helper hands it over.
    static EVAL_RESULT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static EVAL_HELPER_INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn take_eval_result() -> Option<String> {
    EVAL_RESULT.with(|r| r.borrow_mut().take())
}

pub(crate) fn set_eval_result(text: String) {
    EVAL_RESULT.with(|r| *r.borrow_mut() = Some(text));
}

/// Indirect `eval`, so the source sees global scope — what a browser devtools
/// console does. The result is stringified INSIDE JS (the only place that can
/// see the value) and handed back through `__vn_devtools_emit`.
const EVAL_HELPER: &str = r#"
globalThis.__VN_DEVTOOLS_EVAL__ = function (src) {
  var out;
  try {
    var v = (0, eval)(src);
    var json;
    try { json = JSON.stringify(v); } catch (e) { json = undefined; }
    // undefined, a function, a cycle, a BigInt — anything JSON cannot carry
    // degrades to its String() form rather than to a missing field.
    if (json === undefined) json = JSON.stringify(v === undefined ? null : String(v));
    out = '{"value":' + json + ',"type":' + JSON.stringify(typeof v) + '}';
  } catch (e) {
    out = JSON.stringify({
      exception: (e && e.name ? e.name + ": " + (e.message || "") : String(e)),
      stack: (e && e.stack) ? String(e.stack) : "",
    });
  }
  __vn_devtools_emit(out);
};
"#;

fn cmd_eval(src: &str) -> String {
    let result = JsRuntime::with(|rt| {
        if !EVAL_HELPER_INSTALLED.with(|c| c.get()) {
            if let Err(e) = rt.eval(EVAL_HELPER, "vn://devtools-eval-helper.js") {
                return Err(format!("{}\n{}", e.header, e.stack));
            }
            EVAL_HELPER_INSTALLED.with(|c| c.set(true));
        }
        let call = format!("__VN_DEVTOOLS_EVAL__({});", json::quote(src));
        if let Err(e) = rt.eval(&call, "vn://devtools-eval.js") {
            return Err(format!("{}\n{}", e.header, e.stack));
        }
        Ok(take_eval_result())
    });
    match result {
        Err(e) => err(format!("eval helper failed: {e}")),
        Ok(None) => err("eval produced no result (the helper did not report back)"),
        Ok(Some(text)) => {
            // Vue may have scheduled work; land it in this frame like every
            // other command does.
            let _ = with_commit(|window, _| {
                if vn_tree::flush_changes() {
                    window.refresh();
                }
            });
            ok(Json::from(Raw(text)))
        }
    }
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

fn cmd_logs(since: Option<f64>) -> String {
    let since = since.unwrap_or(0.0).max(0.0) as u64;
    let (entries, missed) = vn_tree::log::since(since);
    let mut out = vec![
        (
            "entries",
            Json::arr(
                entries
                    .iter()
                    .map(|e| {
                        Json::obj(vec![
                            ("seq", Json::n(e.seq as f64)),
                            ("at", Json::n(e.at_ms as f64)),
                            ("kind", Json::s(e.kind)),
                            ("text", Json::s(e.text.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("next", Json::n(vn_tree::log::next_seq() as f64)),
        (
            "status",
            match vn_tree::log::status() {
                Some(s) => Json::s(s),
                None => Json::Null,
            },
        ),
    ];
    if missed > 0 {
        out.push(("dropped", Json::n(missed as f64)));
    }
    ok(Json::obj(out))
}

// ---------------------------------------------------------------------------
// type / key
// ---------------------------------------------------------------------------

/// Both commands focus first and finish on the NEXT frame (see [`Pending`]).
/// `key` without a selector goes to whatever already has focus, in which case
/// there is nothing to wait for.
fn start_input_command(cmd: &str, req: &Json, reply: &Sender<String>) -> Outcome {
    let keys: Vec<String> = match cmd {
        "key" => match req.get("key").and_then(Json::as_str) {
            None => return Outcome::Done(err("key needs a `key` (e.g. \"Enter\", \"Shift+Tab\")")),
            Some(k) => vec![k.to_string()],
        },
        _ => Vec::new(),
    };
    let text = match cmd {
        "type" => match req.get("text").and_then(Json::as_str) {
            None => return Outcome::Done(err("type needs a `text`")),
            Some(t) => Some(t.to_string()),
        },
        _ => None,
    };
    // Validate the keystroke before spending a frame on focus.
    for k in &keys {
        if let Err(e) = parse_keystroke(k) {
            return Outcome::Done(err(e));
        }
    }
    let selector = req.get("selector").and_then(Json::as_str);
    let index = req.get("index").and_then(Json::as_f64);
    let node = match selector {
        Some(sel) => match resolve_one(sel, index) {
            Ok(id) => id,
            Err(e) => return Outcome::Done(err(e)),
        },
        None if cmd == "key" => {
            // Straight to whatever gpui says is focused — the "press Escape on
            // the app" case, which has no element to name.
            let out = with_commit(|window, app| {
                for k in &keys {
                    match parse_keystroke(k) {
                        Ok(ks) => send_keystroke(ks, window, app),
                        Err(e) => return err(e),
                    }
                }
                if vn_tree::flush_changes() {
                    window.refresh();
                }
                ok(Json::obj(vec![("delivered", Json::n(keys.len() as f64))]))
            });
            return Outcome::Done(out.unwrap_or_else(|| err("not in a commit phase")));
        }
        None => return Outcome::Done(err("type needs a `selector`")),
    };
    let focus = with_commit(|window, app| vn_tree::devtools::focus(node, window, app));
    match focus {
        None => return Outcome::Done(err("not in a commit phase")),
        Some(Err(e)) => return Outcome::Done(err(e)),
        Some(Ok(())) => {}
    }
    // gpui installs the native input handler during PAINT, and only for a
    // handle that is already focused — so the keystrokes have to wait a frame.
    Outcome::Parked(Pending { node, text, keys, reply: reply.clone(), waited: 0 })
}

/// One printable character, delivered as the real thing: gpui resolves the
/// keystroke (running the `<input>`'s `intercept_keystrokes` hook, which is
/// what fires `keydown` into JS and honors `preventDefault()`), then hands the
/// character to the focused input handler, which edits the buffer and fires
/// `input` — the same sequence a physical key produces.
fn type_char(ch: char, window: &mut Window, cx: &mut App) -> bool {
    let key = match ch {
        ' ' => "space".to_string(),
        '\n' | '\r' => "enter".to_string(),
        '\t' => "tab".to_string(),
        c => c.to_string(),
    };
    let ks = gpui::Keystroke {
        modifiers: gpui::Modifiers::none(),
        key,
        key_char: Some(ch.to_string()),
    };
    send_keystroke(ks, window, cx);
    true
}

fn send_keystroke(ks: gpui::Keystroke, window: &mut Window, cx: &mut App) {
    // `dispatch_keystroke` covers key-down + the character insertion; the
    // matching key-UP has no gpui convenience wrapper, so it goes through the
    // same public `dispatch_event` the platform layer uses. Without it a
    // `@keyup` handler would never see a synthesized key.
    window.dispatch_keystroke(ks.clone(), cx);
    window.dispatch_event(
        gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke: ks }),
        cx,
    );
}

/// `Enter`, `Shift+Tab`, `Ctrl+A`, `ArrowDown` → a gpui [`gpui::Keystroke`].
///
/// Accepts DOM `KeyboardEvent.key` spellings (what an author writing
/// `@keydown.enter` thinks in) as well as gpui's own, and `+` as well as `-` as
/// the separator.
fn parse_keystroke(spec: &str) -> Result<gpui::Keystroke, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty key".into());
    }
    let parts: Vec<&str> = spec.split(['+', '-']).filter(|p| !p.is_empty()).collect();
    // A bare "-" or "+" IS the key, not a separator.
    let parts = if parts.is_empty() { vec![spec] } else { parts };
    let (mods, key) = parts.split_at(parts.len() - 1);
    let mut modifiers = gpui::Modifiers::none();
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.control = true,
            "shift" => modifiers.shift = true,
            "alt" | "option" => modifiers.alt = true,
            "cmd" | "meta" | "super" | "win" => modifiers.platform = true,
            other => return Err(format!("unknown modifier `{other}` in `{spec}`")),
        }
    }
    let raw = key[0];
    let name = match raw.to_ascii_lowercase().as_str() {
        // DOM `KeyboardEvent.key` → gpui keystroke names. The inverse of
        // `render::dom_key`, so a round trip through devtools is lossless.
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "pageup" => "pageup",
        "pagedown" => "pagedown",
        " " => "space",
        lower => lower,
    }
    .to_string();
    // A printable single character must carry `key_char`, or gpui delivers the
    // binding but never the text.
    let key_char = (raw.chars().count() == 1 && !modifiers.control && !modifiers.platform)
        .then(|| raw.to_string())
        .or_else(|| (name == "space").then(|| " ".to_string()));
    Ok(gpui::Keystroke { modifiers, key: name, key_char })
}

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

fn cmd_screenshot(path: Option<&str>) -> String {
    let target = match path {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => screenshot::next_default_path(),
    };
    match screenshot::capture(&target) {
        Err(e) => err(e),
        Ok(shot) => {
            let mut entries = vec![
                ("path", Json::s(shot.path.to_string_lossy().to_string())),
                ("width", Json::n(f64::from(shot.width))),
                ("height", Json::n(f64::from(shot.height))),
                ("hwnd", Json::s(format!("{:#x}", shot.hwnd))),
            ];
            if shot.iconic {
                entries.push(("iconic", Json::Bool(true)));
                entries.push((
                    "warning",
                    Json::s(
                        "the window is minimized — PrintWindow can only return whatever the \
                         compositor last held, which may be stale or blank",
                    ),
                ));
            }
            ok(Json::obj(entries))
        }
    }
}
