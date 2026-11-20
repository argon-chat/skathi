//! Programmatic control of a running dev host over a local TCP port.
//!
//! The listener thread never touches JSC, gpui or the retained tree: it queues `(request, reply
//! channel)` and blocks. [`pump`] runs on the gpui main thread inside the frame pump's commit
//! publication and executes one command per frame, so every command sees a committed tree.
//!
//! Discovery is `<dist>/devtools.json` (`{port, pid, token}`) via [`crate::slot`]. The same port
//! also answers HTTP/1.1 (`POST /`, one command per connection) so JS can reach it with `fetch()`.

mod allocations;
mod effects;
mod eval;
mod indexeddb;
mod leases;
#[cfg(windows)]
mod mem;
#[cfg(target_os = "macos")]
#[path = "mem_mac.rs"]
mod mem;
#[cfg(not(any(windows, target_os = "macos")))]
#[path = "mem_stub.rs"]
mod mem;
#[cfg(any(windows, target_os = "macos"))]
mod mem_common;
#[cfg(any(target_os = "macos", all(test, windows)))]
mod mem_fold;
mod net;
mod picker;
mod screenshot;
pub(crate) mod trace;
pub mod ws;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{App, Window};
use sk_runtime::{with_commit, JsRuntime};

use crate::json::{self, Json, Raw};

/// Backstop so a wedged pump cannot leak listener threads blocked forever on a channel.
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

struct Job {
    generation: u64,
    req: Json,
    reply: Sender<String>,
}

static QUEUE: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
static TOKEN: Mutex<String> = Mutex::new(String::new());

// Disabled means the listener is gone, not just that F12 is refused: the port carries `eval`.
// `enable()` binds a fresh port and token; `disable()` closes the listener, drops every push
// socket, blanks the token and deletes the handshake file. Precedence: default on, then
// `"devtools": false` in sk.config.json or a payload published without devtools, then
// `appDevtools.enable()/disable()`; `SK_DEVTOOLS=1` overrides them all and locks (see `resolve_initial`).

static ENABLED: AtomicBool = AtomicBool::new(true);
static LOCKED: AtomicBool = AtomicBool::new(false);
/// `0` when not listening; also how [`stop_listener`] wakes a blocked `accept()`.
static PORT: AtomicU16 = AtomicU16::new(0);
/// Bumped by every stop; the accept loop returns when it differs from the one it started with.
static ACCEPT_GEN: AtomicU64 = AtomicU64::new(0);
/// Installed on the first enable, so an app that starts disabled keeps the engine's own
/// `console` and `fetch`.
static TEES_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Starts the log ring and rule-text capture. Unconditional: a disabled app can `enable()` later
/// and neither can be recovered after the fact.
pub fn arm() {
    sk_tree::log::enable();
    sk_css::set_capture_rule_text(true);
    // Every painted node gets a gpui hitbox so `pick` can use gpui's own hit test; it cannot be
    // armed on demand because the inspector attaches to an already-running host.
    sk_tree::enable_pick_hitboxes();
}

/// Resolves the initial state and starts the listener if on. Call after `window::load_config`
/// and before the app bundle evaluates. `published` is false for a payload published without devtools.
pub fn boot(config_enabled: bool, published: bool) {
    let (on, locked) = resolve_initial(config_enabled && published);
    LOCKED.store(locked, Ordering::SeqCst);
    ENABLED.store(on, Ordering::SeqCst);
    if on {
        start_listener();
    } else {
        let why = if !published {
            "the app was published without them (`\"publishDevtools\"` in sk.config.json)"
        } else if config_enabled {
            "SK_DEVTOOLS=0"
        } else {
            "`\"devtools\": false` in sk.config.json"
        };
        let msg = format!(
            "[devtools] disabled at boot ({why}) — no port, no handshake, F12 declined. \
             `appDevtools.enable()` starts them."
        );
        eprintln!("{msg}");
        sk_tree::log::record("dev", msg);
    }
}

/// `SK_DEVTOOLS=1|on|true|yes` forces the devtools on and locks them against `disable()`;
/// `0|off|false|no` only sets the initial state. An empty value counts as unset.
fn resolve_initial(config_enabled: bool) -> (bool, bool) {
    match std::env::var("SK_DEVTOOLS")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "" => (config_enabled, false),
        "1" | "on" | "true" | "yes" => (true, true),
        "0" | "off" | "false" | "no" => (false, false),
        other => {
            eprintln!(
                "[devtools] SK_DEVTOOLS={other:?} is not one of 1/on/true/yes or 0/off/false/no \
                 — ignored"
            );
            (config_enabled, false)
        }
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn locked() -> bool {
    LOCKED.load(Ordering::SeqCst)
}

pub fn port() -> Option<u16> {
    match PORT.load(Ordering::SeqCst) {
        0 => None,
        p => Some(p),
    }
}

/// `appDevtools.enable()` / `.disable()`. `Err` names why it was refused. Idempotent: enabling a
/// listening host keeps its port.
pub fn set_enabled(on: bool) -> Result<bool, String> {
    if !on && LOCKED.load(Ordering::SeqCst) {
        return Err(
            "SK_DEVTOOLS=1 is set in this process's environment, which holds the devtools on — \
             `disable()` is refused. Unset it to let the app's own choice stand."
                .into(),
        );
    }
    if ENABLED.load(Ordering::SeqCst) == on {
        return Ok(on);
    }
    ENABLED.store(on, Ordering::SeqCst);
    if on {
        start_listener();
    } else {
        stop_listener();
    }
    Ok(on)
}

/// Port and token are fresh every time so a client dropped at `disable()` cannot walk back in.
/// Failures are reported and ignored.
fn start_listener() {
    if PORT.load(Ordering::SeqCst) != 0 {
        return;
    }
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

    let path = crate::slot::path("devtools.json");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let doc = Json::obj(vec![
        ("port", Json::n(f64::from(port))),
        ("pid", Json::n(std::process::id() as f64)),
        ("token", Json::s(&token)),
    ]);
    if let Err(e) = std::fs::write(&path, format!("{}\n", doc.to_json())) {
        eprintln!(
            "[devtools] cannot write {}: {e} — devtools disabled",
            path.display()
        );
        return;
    }
    PORT.store(port, Ordering::SeqCst);
    let generation = ACCEPT_GEN.load(Ordering::SeqCst);
    eprintln!(
        "[devtools] listening on 127.0.0.1:{port} — {} (pid {})",
        path.display(),
        std::process::id()
    );
    sk_tree::log::record("dev", format!("[devtools] listening on 127.0.0.1:{port}"));

    std::thread::Builder::new()
        .name("sk-devtools".into())
        .spawn(move || accept_loop(listener, generation))
        .ok();
}

/// `accept()` cannot be interrupted by a flag, so the generation is bumped and we connect to
/// ourselves: the loop wakes, sees the new generation and returns.
fn stop_listener() {
    leases::clear();
    picker::stop();
    trace::cancel_recording();
    allocations::cancel_recording();
    let port = PORT.swap(0, Ordering::SeqCst);
    // A blank token never matches (`handle_request` refuses an empty one).
    TOKEN.lock().unwrap().clear();
    ws::close_all();
    let path = crate::slot::path("devtools.json");
    let _ = std::fs::remove_file(&path);
    // Blocked clients get a disconnected channel rather than the 60 s timeout.
    QUEUE.lock().unwrap().clear();
    if port != 0 {
        ACCEPT_GEN.fetch_add(1, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            std::time::Duration::from_millis(500),
        );
    }
    let msg = "[devtools] disabled — the listener is closed, the handshake is gone and F12 is \
               declined. `appDevtools.enable()` starts a new port.";
    eprintln!("{msg}");
    sk_tree::log::record("dev", msg);
}

const CONSOLE_TEE_JS: &str = r#"
(function () {
  if (!globalThis.console) return;

  // Errors print as header plus stack (JSON.stringify gives `{}`: their fields are
  // non-enumerable). JSC's `stack` lacks V8's "Name: message" line, so the header is added here.
  var isError = function (v) {
    if (!v || typeof v !== "object") return false;
    if (v instanceof Error) return true;
    var tag = Object.prototype.toString.call(v);
    return tag === "[object Error]" || tag === "[object DOMException]";
  };
  var errorHead = function (e) {
    var name = String(e.name || "Error");
    return e.message ? name + ": " + e.message : name;
  };
  var show = function (v) {
    if (typeof v === "string") return v;
    if (isError(v)) {
      var head = errorHead(v);
      var stack = typeof v.stack === "string" ? v.stack : "";
      return !stack ? head : stack.indexOf(head) === 0 ? stack : head + "\n" + stack;
    }
    try {
      var s = JSON.stringify(v, function (_k, x) {
        if (typeof x === "bigint") return x + "n";
        return isError(x) ? errorHead(x) : x;
      });
      return s === undefined ? String(v) : s;
    }
    catch (e) { return String(v); }
  };

  // WHATWG Console formatter: %s %d %i %f %o %O %j, %% for a literal percent, and %c, which
  // consumes its argument as CSS and prints nothing. Leftover arguments are appended.
  var SPEC = /%([sdifoOjc%])/g;
  var format = function (args) {
    if (typeof args[0] !== "string" || args.length < 2 || !/%[sdifoOjc%]/.test(args[0])) {
      return { text: args.map(show).join(" "), runs: null };
    }
    var next = 1;
    var runs = [];
    var out = args[0].replace(SPEC, function (whole, kind) {
      if (kind === "%") return "%";
      if (next >= args.length) return whole;
      var arg = args[next++];
      switch (kind) {
        case "c":
          // NUL placeholder, resolved in a second pass: `replace` cannot know the output offset.
          runs.push({ at: -1, css: String(arg == null ? "" : arg) });
          return "\u0000";
        case "d":
        case "i":
          return typeof arg === "symbol" || typeof arg === "bigint"
            ? String(arg)
            : String(parseInt(arg, 10));
        case "f":
          return typeof arg === "symbol" ? String(arg) : String(parseFloat(arg));
        case "s":
          return typeof arg === "string" ? arg : show(arg);
        default:
          return show(arg);
      }
    });
    if (runs.length) {
      var placed = "";
      var i = 0;
      var r = 0;
      for (; i < out.length; i++) {
        if (out.charCodeAt(i) === 0) {
          if (r < runs.length) runs[r++].at = placed.length;
          continue;
        }
        placed += out[i];
      }
      out = placed;
    }
    for (; next < args.length; next++) out += " " + show(args[next]);
    return { text: out, runs: runs.length ? runs : null };
  };

  ["log", "info", "warn", "error", "debug"].forEach(function (name) {
    var orig = console[name];
    if (typeof orig !== "function") return;
    console[name] = function () {
      var args = Array.prototype.slice.call(arguments);
      try {
        var f = format(args);
        __sk_devtools_console(name, f.text, f.runs ? JSON.stringify(f.runs) : "");
      } catch (e) {}
      return orig.apply(console, args);
    };
  });
})();
"#;

sk_runtime::host_fn!(fn sk_devtools_console(g, args) {
    let level = unsafe { sk_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    let text = unsafe { sk_runtime::redjs::webapi::util::read_string(g, args.arg(1).0) }
        .unwrap_or_default();
    let kind: &'static str = match level.as_str() {
        "warn" => "warn",
        "error" => "error",
        "debug" => "debug",
        "info" => "info",
        _ => "log",
    };
    // Third argument: style runs as JSON, present only when the call carried `%c`.
    let styles = unsafe { sk_runtime::redjs::webapi::util::read_string(g, args.arg(2).0) }
        .filter(|s| !s.is_empty() && s != "undefined");
    sk_tree::log::record_styled(kind, text, styles);
    sk_runtime::redjs::api::Value::undefined().0
});

// Includes each target's token; the calling JS already has eval-level trust over its own process.
sk_runtime::host_fn!(fn sk_devtools_targets(_g, _args) {
    let payload = targets_json();
    JsRuntime::with(|rt| rt.ctx().string(&payload).0)
});

struct AppDevtools;

impl sk_runtime::app_devtools::DevtoolsHost for AppDevtools {
    fn info(&self) -> sk_runtime::app_devtools::DevtoolsInfo {
        sk_runtime::app_devtools::DevtoolsInfo {
            enabled: enabled(),
            locked: locked(),
            port: port(),
            pid: std::process::id(),
            key: crate::window::devtools_key_enabled(),
            inspectors: crate::inspector::children().len(),
            slot: crate::slot::dir().display().to_string(),
        }
    }

    fn set_enabled(&self, on: bool) -> Result<bool, String> {
        set_enabled(on)?;
        if on {
            install_tees_once();
        }
        Ok(enabled())
    }

    // Same entry point as the key (debounce, raise-existing), so a menu item and F12 agree.
    fn open(&self) -> Result<bool, String> {
        if !enabled() {
            return Err(
                "the devtools are disabled in this process — call appDevtools.enable() first"
                    .into(),
            );
        }
        let window = crate::main_window_id();
        sk_runtime::with_app(|cx| crate::inspector::open_for_window(window, cx))
            .map(|()| true)
            .ok_or_else(|| "appDevtools.open(): called outside a published phase".into())
    }
}

fn read_target(dir: &std::path::Path) -> Option<Json> {
    let text = std::fs::read_to_string(dir.join("devtools.json")).ok()?;
    let doc = json::parse(&text)?;
    let port = doc.get("port").and_then(Json::as_f64)? as u16;
    let token = doc.get("token").and_then(Json::as_str)?.to_string();
    let pid = doc.get("pid").and_then(Json::as_f64).unwrap_or(0.0);
    let alive = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        std::time::Duration::from_millis(150),
    )
    .is_ok();
    Some(Json::obj(vec![
        (
            "slot",
            Json::s(dir.file_name().map_or_else(
                || dir.display().to_string(),
                |n| n.to_string_lossy().to_string(),
            )),
        ),
        ("dir", Json::s(dir.display().to_string())),
        ("port", Json::n(f64::from(port))),
        ("pid", Json::n(pid)),
        ("token", Json::s(token)),
        ("alive", Json::Bool(alive)),
        ("self", Json::Bool(pid as u32 == std::process::id())),
    ]))
}

/// Where handshakes may live: beside our own dist, the e2e slot pool, and each packaged app's
/// instance directories (see `crate::slot`).
fn scan_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    let own = crate::slot::path("devtools.json");
    if let Some(parent) = own.parent().and_then(|d| d.parent()) {
        roots.push(parent.to_path_buf());
        if let Some(packages) = parent.parent() {
            roots.push(packages.join("test").join(".sk-test").join("slots"));
        }
    }
    if let Ok(apps) = std::fs::read_dir(crate::slot::apps_root()) {
        for app in apps.flatten() {
            let path = app.path();
            if path.is_dir() && !roots.contains(&path) {
                roots.push(path);
            }
        }
    }
    roots
}

fn collect_targets_json() -> String {
    let preferred = std::env::var("SK_DEVTOOLS_TARGET").unwrap_or_default();
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if !preferred.is_empty() {
        dirs.push(std::path::PathBuf::from(&preferred));
    }
    for root in scan_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("devtools.json").is_file() && !dirs.contains(&path) {
                dirs.push(path);
            }
        }
    }
    let targets: Vec<Json> = dirs.iter().filter_map(|d| read_target(d)).collect();
    Json::obj(vec![
        (
            "preferred",
            if preferred.is_empty() {
                Json::Null
            } else {
                Json::s(preferred)
            },
        ),
        (
            "self",
            Json::obj(vec![
                ("pid", Json::n(std::process::id() as f64)),
                ("dir", Json::s(crate::slot::dir().display().to_string())),
            ]),
        ),
        ("targets", Json::arr(targets)),
    ])
    .to_json()
}

#[derive(Default)]
struct TargetScan {
    running: bool,
    ready: Option<String>,
}

/// Discovery probes stale loopback ports. On Windows each refused connection
/// can consume its whole timeout; none of that may run on the JS/UI thread.
#[derive(Clone, Default)]
struct TargetDiscovery(Arc<Mutex<TargetScan>>);

impl TargetDiscovery {
    fn poll(
        &self,
        collect: impl FnOnce() -> String + Send + 'static,
    ) -> std::io::Result<Option<String>> {
        let mut scan = self.0.lock().unwrap();
        if let Some(result) = scan.ready.take() {
            return Ok(Some(result));
        }
        if scan.running {
            return Ok(None);
        }
        scan.running = true;
        drop(scan);
        let shared = self.0.clone();
        let worker = std::thread::Builder::new()
            .name("sk-devtools-discovery".into())
            .spawn(move || {
                let result = collect();
                let mut scan = shared.lock().unwrap();
                scan.ready = Some(result);
                scan.running = false;
            });
        if let Err(error) = worker {
            self.0.lock().unwrap().running = false;
            return Err(error);
        }
        Ok(None)
    }
}

fn targets_json() -> String {
    static DISCOVERY: OnceLock<TargetDiscovery> = OnceLock::new();
    match DISCOVERY
        .get_or_init(TargetDiscovery::default)
        .poll(collect_targets_json)
    {
        Ok(Some(result)) => result,
        Ok(None) => "{\"pending\":true}".into(),
        Err(error) => Json::obj(vec![(
            "error",
            Json::s(format!("target discovery: {error}")),
        )])
        .to_json(),
    }
}

// These controls must not use host_fn!: begin returns an open child span,
// whose lifetime deliberately extends beyond this native call into JS.
sk_runtime::raw_host_fn! { fn sk_trace_callback_begin(g, cf) {
    use sk_runtime::redjs::{api::{HostArgs, Value}, profiling};
    if !profiling::enabled() { return Value::int32(0).0; }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let args = HostArgs::new(cf);
        Value::number(profiling::external_callback_begin(g, args.arg(0).0) as f64).0
    })) {
        Ok(value) => value,
        Err(_) => { log::error!("Frame profiler callback begin failed"); Value::int32(0).0 }
    }
} }
sk_runtime::raw_host_fn! { fn sk_trace_callback_end(_g, cf) {
    use sk_runtime::redjs::{api::{HostArgs, Value}, profiling};
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let token = HostArgs::new(cf).arg(0).as_number().filter(|value| value.is_finite() && *value > 0. && value.fract() == 0.).unwrap_or(0.) as u64;
        profiling::external_callback_end(token);
    })).is_err() { log::error!("Frame profiler callback end failed"); }
    Value::undefined().0
} }

/// Call before the app bundle boots. Unconditional so `appDevtools` can answer while devtools are
/// off; the JS tees are not (see [`install_tees_once`]).
pub fn install(rt: &JsRuntime) {
    rt.register(
        "__sk_trace_callback_begin",
        1,
        sk_trace_callback_begin as *const _,
    );
    rt.register(
        "__sk_trace_callback_end",
        1,
        sk_trace_callback_end as *const _,
    );
    rt.register("__sk_devtools_console", 3, sk_devtools_console as *const _);
    rt.register("__sk_devtools_targets", 0, sk_devtools_targets as *const _);
    unsafe { sk_runtime::app_devtools::install(rt.raw(), Some(Box::new(AppDevtools))) };
    net::install(rt);
    if enabled() {
        install_tees_once();
    }
}

/// Wraps `console.*`, `fetch` and `WebSocket` once, when the devtools are first on, so a host that
/// starts disabled keeps the engine's own. Never unwrapped: code may have captured the wrappers.
fn install_tees_once() {
    if TEES_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    JsRuntime::with(|rt| {
        if let Err(e) = rt.eval(CONSOLE_TEE_JS, "sk://devtools-console.js") {
            eprintln!("[devtools] console tee failed: {}\n{}", e.header, e.stack);
        }
        net::install_tee(rt);
    });
}

/// Puts a `kind: "reload"` marker (with `cause`) in the Network ring when the host reruns the
/// app in the same JS realm. No-op while the tees are off.
pub fn mark_reload(cause: &str) {
    if TEES_INSTALLED.load(Ordering::SeqCst) {
        net::mark_reload(cause);
    }
}

/// A Network row for a load the host made (gpui's loaders for `<img>`, CSS `url()`,
/// `@font-face`, `<lottie>`), which no script tee sees. Inert while the devtools are off.
pub struct LoaderRow(Option<(String, String, f64, std::time::Instant)>);

impl LoaderRow {
    pub fn start(method: &str, url: &str) -> LoaderRow {
        if !enabled() {
            return LoaderRow(None);
        }
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        LoaderRow(Some((
            method.to_string(),
            url.to_string(),
            at,
            std::time::Instant::now(),
        )))
    }

    pub fn finish(
        self,
        result: &Result<
            sk_runtime::redjs::webapi::http::Response,
            sk_runtime::redjs::webapi::http::HttpError,
        >,
    ) {
        if let Some((method, url, at, t0)) = self.0 {
            net::record_loader(
                &method,
                &url,
                at,
                t0.elapsed().as_secs_f64() * 1000.0,
                result,
            );
        }
    }
}

/// `net_cache`: the HTTP cache's counters and folder, or `op: "clear"` to empty it. `null`
/// stats: no cache is installed.
fn cmd_net_cache(op: Option<&str>) -> String {
    use sk_runtime::redjs::webapi::http::cache;
    match op.unwrap_or("stats") {
        // Flush queued writes (at most 2 s) so stats show what is on disk.
        "stats" => {
            cache::flush();
            ok(net::cache_json())
        }
        "clear" => match cache::clear() {
            Ok(freed) => ok(Json::obj(vec![
                ("freed", Json::n(freed as f64)),
                ("now", net::cache_json()),
            ])),
            Err(e) => err(format!("net_cache clear: {e}")),
        },
        other => err(format!(
            "net_cache: unknown op {other:?} — \"stats\" (default) or \"clear\""
        )),
    }
}

/// 128 bits of process-local entropy, hex. No `rand` dependency: the token only has to stop an
/// unrelated local process from driving the app.
fn fresh_token() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut h = RandomState::new().build_hasher();
    h.write_u64(std::process::id() as u64);
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64),
    );
    let a = h.finish();
    let mut h2 = RandomState::new().build_hasher();
    h2.write_u64(a);
    h2.write_usize(&h as *const _ as usize);
    format!("{a:016x}{:016x}", h2.finish())
}

fn accept_loop(listener: TcpListener, generation: u64) {
    loop {
        let accepted = listener.accept();
        // Checked before serving so `stop_listener`'s wake-up connect is never handed to `serve`.
        if ACCEPT_GEN.load(Ordering::SeqCst) != generation {
            return;
        }
        match accepted {
            Ok((s, _)) => {
                std::thread::Builder::new()
                    .name("sk-devtools-conn".into())
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

/// Newline-delimited JSON (pipelining allowed) or HTTP/1.1 (one `POST /` per connection), told
/// apart by the first line: JSON starts with `{`, an HTTP request line ends with ` HTTP/1.x`.
fn serve(stream: TcpStream) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut out = write_half;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    if http_request_line(&line).is_some() {
        serve_http(&line, reader, out);
        return;
    }
    loop {
        if !line.trim().is_empty() {
            let response = handle_request(&line);
            if out.write_all(response.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
                return;
            }
            let _ = out.flush();
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

/// One request line of JSON to one reply line. The token check lives here so both framings
/// share it.
fn handle_request(line: &str) -> String {
    match json::parse(line) {
        None => err("request is not valid JSON"),
        Some(req) => {
            let generation = ACCEPT_GEN.load(Ordering::SeqCst);
            // A blank token never matches, so a request on a socket accepted just before
            // `disable()` is refused.
            let want = TOKEN.lock().unwrap().clone();
            let ok_token = !want.is_empty()
                && req
                    .get("token")
                    .and_then(Json::as_str)
                    .is_some_and(|t| t == want);
            if !ok_token {
                err("bad or missing token — read it from <dist>/devtools.json")
            } else {
                // Read-only inspection and aggregation own no JSC/Window values; keep them off the
                // UI frame pump.
                match req.get("cmd").and_then(Json::as_str) {
                    Some("indexeddb") => indexeddb::command(&req).map_or_else(err, ok),
                    Some("allocations") => {
                        let result = allocations::command_authenticated(&req, generation);
                        if result.is_ok() && req.get("op").and_then(Json::as_str) == Some("start") {
                            if let Some(ms) = req.get("leaseMs").and_then(Json::as_f64) {
                                leases::arm(ms as u64);
                            }
                        }
                        result.map_or_else(err, ok)
                    }
                    Some("capture-heartbeat") => {
                        leases::refresh();
                        ok(Json::obj(vec![("renewed", Json::Bool(true))]))
                    }
                    _ => submit(req, generation),
                }
            }
        }
    }
}

const MAX_HTTP_BODY: usize = 4 << 20;

fn http_request_line(line: &str) -> Option<(String, String)> {
    let line = line.trim_end();
    let rest = line
        .strip_suffix(" HTTP/1.1")
        .or_else(|| line.strip_suffix(" HTTP/1.0"))?;
    let (method, target) = rest.split_once(' ')?;
    if !method.chars().all(|c| c.is_ascii_uppercase()) || method.is_empty() {
        return None;
    }
    Some((method.to_string(), target.to_string()))
}

fn serve_http(request_line: &str, mut reader: BufReader<TcpStream>, mut out: TcpStream) {
    let Some((method, target)) = http_request_line(request_line) else {
        return;
    };
    // `Transfer-Encoding: chunked` is refused by name rather than mis-framed. The header count
    // is bounded so a client that never sends the blank line cannot hold this thread.
    let mut headers: Vec<(String, String)> = Vec::new();
    for _ in 0..200 {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let head = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    let length: Option<usize> = head("content-length").parse::<usize>().ok();
    let chunked = head("transfer-encoding").eq_ignore_ascii_case("chunked");

    if method == "GET"
        && head("upgrade").eq_ignore_ascii_case("websocket")
        && head("connection").to_ascii_lowercase().contains("upgrade")
    {
        ws::serve(&target, &headers, out, reader);
        return;
    }

    let mut reply = |status: &str, body: String| http_reply(&mut out, status, &body);
    match method.as_str() {
        "OPTIONS" => reply("204 No Content", String::new()),
        "POST" => {
            if chunked {
                return reply(
                    "411 Length Required",
                    err("chunked request bodies are not read — send a Content-Length"),
                );
            }
            let Some(n) = length else {
                return reply("411 Length Required", err("POST needs a Content-Length"));
            };
            if n > MAX_HTTP_BODY {
                return reply(
                    "413 Payload Too Large",
                    err(format!("request body is {n} bytes — the cap is {MAX_HTTP_BODY}")),
                );
            }
            let mut body = vec![0u8; n];
            if std::io::Read::read_exact(&mut reader, &mut body).is_err() {
                return reply("400 Bad Request", err("the request body arrived short"));
            }
            match String::from_utf8(body) {
                Err(_) => reply("400 Bad Request", err("the request body is not UTF-8")),
                Ok(text) => reply("200 OK", handle_request(&text)),
            }
        }
        _ => reply(
            "405 Method Not Allowed",
            err(format!(
                "{method} {target} — this port speaks two things over HTTP: POST / with the                  command object as the body, and GET / with `Upgrade: websocket` for the push                  channel"
            )),
        ),
    }
}

/// Writes one HTTP response; the caller drops the socket. CORS is open because the token, not
/// the origin, guards the port.
fn http_reply(out: &mut TcpStream, status: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Allow-Methods: POST, OPTIONS\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(body.as_bytes());
    let _ = out.flush();
}

fn submit(req: Json, generation: u64) -> String {
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    {
        let mut queue = QUEUE.lock().unwrap();
        if !enabled() || generation != ACCEPT_GEN.load(Ordering::SeqCst) {
            return err("the app revoked this devtools session");
        }
        queue.push_back(Job {
            generation,
            req,
            reply: tx,
        });
    }
    sk_runtime::frames::wake();
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(reply) => reply,
        // Disconnected means `stop_listener` cleared the queue (`appDevtools.disable()`), which
        // is a different diagnosis from a wedged app.
        Err(mpsc::RecvTimeoutError::Disconnected) => err(
            "the app disabled its devtools while this command was queued \
             (appDevtools.disable()) — the listener is closed",
        ),
        Err(mpsc::RecvTimeoutError::Timeout) => err(
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

thread_local! {
    /// A command that could not finish in its frame ([`Pending`]); while one is parked no new
    /// command is dequeued.
    static PENDING: std::cell::RefCell<Option<Pending>> = const { std::cell::RefCell::new(None) };
}

/// `type` and `key` need the target focused and painted before keystrokes can reach it (gpui
/// installs the input handler during paint), so they focus on frame N and send on frame N+1.
struct Pending {
    generation: u64,
    node: u32,
    text: Option<String>,
    keys: Vec<String>,
    reply: Sender<String>,
    waited: u32,
    /// The command's `window` argument; `TARGET_WINDOW` is reset by the time this resumes.
    /// `None` = the primary window.
    window: Option<u64>,
}

/// Frames to wait for focus before giving up (~0.5 s at 60 fps).
const PENDING_MAX_FRAMES: u32 = 30;

/// A command is queued or parked: keeps the frame clock at display rate.
pub fn busy() -> bool {
    PENDING.with(|p| p.borrow().is_some()) || !QUEUE.lock().unwrap().is_empty()
}

/// Runs one command per frame. Call inside the commit publication so `with_commit` resolves.
pub fn pump() {
    if leases::expired() {
        let _ = trace::recording_stop();
        allocations::stop_recording();
    }
    let resumed = PENDING.with(|p| p.borrow_mut().take()).map(resume_pending);
    if let Some(still_pending) = resumed {
        PENDING.with(|p| *p.borrow_mut() = still_pending);
        return;
    }
    let Some(job) = QUEUE.lock().unwrap().pop_front() else {
        return;
    };
    let Job {
        generation,
        req,
        reply,
    } = job;
    if !enabled() || generation != ACCEPT_GEN.load(Ordering::SeqCst) {
        let _ = reply.send(err("the app revoked this devtools session"));
        return;
    }
    let response = run(&req, &reply);
    TARGET_WINDOW.with(|c| c.set(None));
    match response {
        Outcome::Done(text) => {
            let _ = reply.send(text);
        }
        Outcome::Parked(mut pending) => {
            pending.generation = generation;
            PENDING.with(|p| *p.borrow_mut() = Some(pending));
        }
    }
}

enum Outcome {
    Done(String),
    Parked(Pending),
}

fn resume_pending(mut p: Pending) -> Option<Pending> {
    if !enabled() || p.generation != ACCEPT_GEN.load(Ordering::SeqCst) {
        let _ = p.reply.send(err("the app revoked this devtools session"));
        return None;
    }
    // Re-publish the command's window for the resume: focus and keystrokes are per window.
    let _scope = TargetScope::new(p.window);
    let focused = with_node_window(p.node, |window, cx| {
        sk_tree::devtools::is_focused(p.node, window, cx)
    })
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
    let reply = with_node_window(p.node, |window, cx| {
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
        // The edit path reports `input` on a `Window::defer`; flush so the next command sees it.
        if sk_tree::flush_changes() {
            window.refresh();
        }
        ok(Json::obj(vec![
            ("node", Json::n(f64::from(p.node))),
            ("delivered", Json::n(delivered as f64)),
            (
                "value",
                match sk_tree::devtools::input_value(p.node, cx) {
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
    let window_arg = req.get("window").and_then(Json::as_f64).map(|w| w as u64);
    TARGET_WINDOW.with(|c| c.set(window_arg));
    let arg = |name: &str| req.get(name).and_then(Json::as_str).map(str::to_string);
    let num = |name: &str| req.get(name).and_then(Json::as_f64);
    let flag = |name: &str| req.get(name).and_then(Json::as_bool).unwrap_or(false);
    let text = match cmd {
        "ping" => ok(Json::obj(vec![
            ("pid", Json::n(std::process::id() as f64)),
            ("root", Json::n(f64::from(sk_tree::devtools::root_id()))),
        ])),
        "query" => match arg("selector") {
            None => err("query needs a `selector`"),
            Some(sel) => cmd_query(&sel, flag("quiet"), flag("ids")),
        },
        "text" => match arg("selector") {
            None => err("text needs a `selector`"),
            Some(sel) => cmd_text(&sel, num("index")),
        },
        "tree" => cmd_tree(
            arg("selector").as_deref(),
            num("depth"),
            num("node").or_else(|| {
                window_arg
                    .map(|id| f64::from(window_root(id)))
                    .filter(|r| *r > 0.0)
            }),
        ),
        "styles" => cmd_styles(arg("selector").as_deref(), num("index"), num("node")),
        "rules" => cmd_rules(arg("selector").as_deref(), num("index"), num("node")),
        "highlight" => cmd_highlight(
            arg("selector").as_deref(),
            num("index"),
            num("node"),
            req.get("clear").and_then(Json::as_bool).unwrap_or(false),
            num("ttlMs"),
            arg("label"),
            window_arg,
        ),
        "pick" => match arg("op") {
            Some(op) => picker::command(&op),
            None => cmd_pick(window_arg),
        },
        "wheel" => cmd_wheel(
            arg("selector").as_deref(),
            num("index"),
            num("x"),
            num("y"),
            num("dx"),
            num("dy"),
            req.get("lines").and_then(Json::as_bool).unwrap_or(false),
            gpui::Modifiers {
                control: req.get("ctrl").and_then(Json::as_bool).unwrap_or(false),
                shift: req.get("shift").and_then(Json::as_bool).unwrap_or(false),
                alt: req.get("alt").and_then(Json::as_bool).unwrap_or(false),
                ..Default::default()
            },
        ),
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
        "ime" => match (arg("selector"), arg("text")) {
            (Some(sel), Some(text)) => {
                let commit = req.get("commit").and_then(Json::as_bool).unwrap_or(false);
                cmd_ime(&sel, num("index"), &text, commit)
            }
            _ => err("ime needs a `selector` and a `text` (\"\" cancels the composition)"),
        },
        "mouse_pressure" => cmd_mouse_pressure(
            arg("selector").as_deref(),
            num("index"),
            num("x"),
            num("y"),
            num("stage"),
            num("pressure"),
        ),
        "mouse" => cmd_mouse(
            arg("selector").as_deref(),
            num("index"),
            num("x"),
            num("y"),
            arg("kind").as_deref(),
            num("button"),
            MouseExtras {
                held: req.get("held").and_then(Json::as_bool).unwrap_or(false),
                clicks: num("clicks").map_or(1, |n| n.clamp(1.0, 3.0) as usize),
                shift: req.get("shift").and_then(Json::as_bool).unwrap_or(false),
            },
        ),
        "drag-files" => cmd_drag_files(req, false),
        "drag-document" => cmd_drag_files(req, true),
        "pen" => {
            let tilt = |name: &str| num(name).unwrap_or(0.0) as f32;
            cmd_pen(
                arg("selector").as_deref(),
                num("index"),
                num("x"),
                num("y"),
                arg("kind").as_deref(),
                PenExtras {
                    pressure: num("pressure").map_or(0.5, |p| p.clamp(0.0, 1.0)) as f32,
                    tilt_x: tilt("tiltX"),
                    tilt_y: tilt("tiltY"),
                    twist: tilt("twist"),
                    eraser: flag("eraser"),
                    barrel: flag("barrel"),
                    from: num("fromX")
                        .zip(num("fromY"))
                        .map(|(x, y)| (x as f32, y as f32)),
                    from_pressure: num("fromPressure").map(|p| p.clamp(0.0, 1.0) as f32),
                    steps: num("steps").map_or(1, |n| n.clamp(1.0, 64.0) as usize),
                    interval: num("interval").map_or(4.0, |ms| ms.clamp(0.0, 1000.0)),
                },
            )
        }
        "eval" => match arg("js") {
            None => err("eval needs `js`"),
            Some(js) => cmd_eval(&js),
        },
        "logs" => cmd_logs(num("since")),
        "net" => cmd_net(num("since")),
        // Hyphenated spelling accepted; MCP tool names derive from the underscore form.
        "net_detail" | "net-detail" => match num("seq") {
            None => err("net_detail needs a `seq` — a row's `seq` from `net`"),
            Some(seq) => cmd_net_detail(seq),
        },
        "net_cache" | "net-cache" => cmd_net_cache(arg("op").as_deref()),
        "perf" => ok(perf_data()),
        "mem" => {
            let gc = req.get("gc").and_then(Json::as_bool).unwrap_or(false);
            let regions = req.get("regions").and_then(Json::as_bool).unwrap_or(false);
            let code = req.get("code").and_then(Json::as_bool).unwrap_or(false);
            match with_target(|window, cx| mem::report(gc, code, regions, window, cx)) {
                Some(data) => ok(data),
                None => err("no such window"),
            }
        }
        "trace"
            if matches!(
                arg("op").as_deref(),
                Some(
                    "recording-start" | "recording-stop" | "recording-status" | "frames" | "frame"
                )
            ) =>
        {
            let result = match arg("op").as_deref() {
                Some("recording-start") => {
                    #[cfg(target_os = "windows")]
                    {
                        let native = with_target(|window, _| crate::window::capture_hwnd(window))
                            .flatten()
                            .unwrap_or(0);
                        trace::set_recording_native_window(native as usize);
                    }
                    let result = trace::recording_start(num("cap").map(|n| n.max(0.0) as usize));
                    if result.is_ok() {
                        if let Some(ms) = num("leaseMs") {
                            leases::arm(ms as u64);
                        }
                        if let Some(key) = with_target(|window, _| sk_tree::windows::key(window)) {
                            trace::set_recording_window_id(key);
                        }
                    }
                    result
                }
                Some("recording-stop") => trace::recording_stop(),
                Some("recording-status") => trace::recording_status(),
                Some("frames") => trace::frame_page(
                    num("offset").unwrap_or(0.0).max(0.0) as usize,
                    num("limit").unwrap_or(240.0).clamp(1.0, 240.0) as usize,
                ),
                Some("frame") => match num("frameId") {
                    Some(id) if id.is_finite() && id >= 0.0 && id.fract() == 0.0 => {
                        trace::frame_detail(
                            id as u64,
                            num("offset").unwrap_or(0.0).max(0.0) as usize,
                            num("limit").unwrap_or(1000.0).clamp(1.0, 1000.0) as usize,
                        )
                    }
                    _ => Err("trace frame needs an integer frameId".into()),
                },
                _ => unreachable!(),
            };
            result.map_or_else(err, ok)
        }
        "trace" => cmd_trace(
            arg("op").as_deref(),
            req.get("frames").and_then(Json::as_bool).unwrap_or(false),
            arg("label").as_deref(),
            num("cap"),
        ),
        "windows" => cmd_windows(),
        "raise" => cmd_raise(),
        "storage" => cmd_storage(
            arg("area").as_deref(),
            arg("op").as_deref(),
            arg("key").as_deref(),
            arg("value").as_deref(),
        ),
        "screenshot" => cmd_screenshot(arg("path").as_deref(), arg("backend").as_deref()),
        "dialog" => cmd_dialog(
            arg("op").as_deref(),
            num("id"),
            req.get("paths"),
            arg("button").or_else(|| num("button").map(|n| (n as i64).to_string())),
        ),
        "permissions" => cmd_permissions(arg("op").as_deref(), arg("name").as_deref()),
        "effects" => effects::command(arg("op").as_deref(), num("node"), arg("value").as_deref())
            .map_or_else(err, ok),
        "type" | "key" => return start_input_command(cmd, req, reply),
        "" => err("no `cmd` in the request"),
        other => err(format!("unknown command `{other}`")),
    };
    Outcome::Done(text)
}

thread_local! {
    static TARGET_WINDOW: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` against the window that paints `node`, unless the command named a window itself.
fn with_node_window<R>(node: u32, f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    if TARGET_WINDOW.with(|c| c.get()).is_some() {
        return with_target(f);
    }
    let painted_by = sk_tree::windows::window_of_node(node);
    if crate::main_window_id() == Some(painted_by) {
        return with_commit(f);
    }
    with_commit(|_, cx| crate::popout::with_window(painted_by, cx, f)).flatten()
}

/// Scopes a `window` argument, restoring the previous one on drop (a parked command resumes a
/// frame after the argument was cleared).
struct TargetScope(Option<u64>);

impl TargetScope {
    fn new(window: Option<u64>) -> Self {
        TargetScope(TARGET_WINDOW.with(|c| c.replace(window)))
    }
}

impl Drop for TargetScope {
    fn drop(&mut self) {
        TARGET_WINDOW.with(|c| c.set(self.0));
    }
}

/// Runs `f` against the window the current command addressed (the pump publishes only the
/// primary one).
pub(crate) fn with_target<R>(f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    match TARGET_WINDOW.with(|c| c.get()) {
        None => with_commit(f),
        Some(id) => with_window_id(id, f),
    }
}

fn with_window_id<R>(id: u64, f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    if crate::main_window_id() == Some(id) {
        return with_commit(f);
    }
    if sk_tree::popup_windows().iter().any(|p| p.0 == id) {
        return with_commit(|_, cx| {
            let handle = cx
                .windows()
                .into_iter()
                .find(|h| h.window_id().as_u64() == id)?;
            handle.update(cx, |_, window, cx| f(window, cx)).ok()
        })
        .flatten();
    }
    with_commit(|_, cx| crate::popout::with_window(id, cx, f)).flatten()
}

/// The node a window paints as its root: `0` for the primary window. Key of the per-window
/// bounds maps (`sk_tree::bounds_in`).
fn window_root(id: u64) -> u32 {
    if crate::main_window_id() == Some(id) {
        return 0;
    }
    crate::popout::list()
        .into_iter()
        .find(|(w, _, _)| *w == id)
        .map_or(0, |(_, node, _)| node)
}

fn window_ids() -> Vec<u64> {
    let mut ids: Vec<u64> = crate::main_window_id().into_iter().collect();
    ids.extend(crate::popout::list().into_iter().map(|(id, _, _)| id));
    ids.extend(sk_tree::popup_windows().into_iter().map(|(id, _, _)| id));
    ids
}

fn resolve_one(selector: &str, index: Option<f64>) -> Result<u32, String> {
    let (ids, _warnings) = sk_tree::devtools::query(selector)?;
    match (ids.len(), index) {
        (0, _) => Err(format!("`{selector}` matched no nodes")),
        (_, Some(i)) => {
            let i = i as usize;
            ids.get(i).copied().ok_or_else(|| {
                format!(
                    "--index {i} is out of range: `{selector}` matched {} nodes",
                    ids.len()
                )
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

fn short(id: u32) -> String {
    match sk_tree::devtools::node_info(id) {
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
    let Some(n) = sk_tree::devtools::node_info(id) else {
        return Json::obj(vec![
            ("id", Json::n(f64::from(id))),
            ("gone", Json::Bool(true)),
        ]);
    };
    let mut entries = vec![
        ("id", Json::n(f64::from(id))),
        ("tag", Json::s(n.tag)),
        (
            "classes",
            Json::arr(n.classes.into_iter().map(Json::Str).collect()),
        ),
    ];
    if with_text {
        let text = sk_tree::devtools::text_content(id).unwrap_or_default();
        entries.push(("text", Json::s(text)));
    }
    entries.push(("bounds", bounds_json(n.bounds)));
    entries.push((
        "scopes",
        Json::arr(n.scopes.into_iter().map(Json::Str).collect()),
    ));
    if n.group_anchor {
        entries.push(("groupAnchor", Json::Bool(true)));
    }
    Json::obj(entries)
}

/// `quiet`: an invalid selector is only the reply's error, not an advisory in the app's log (the
/// inspector's search sends one per keystroke). `ids`: the matches as bare ids.
fn cmd_query(selector: &str, quiet: bool, ids_only: bool) -> String {
    match sk_tree::devtools::query(selector) {
        Err(advisory) => {
            if !quiet {
                sk_tree::log::record("advisory", format!("[query] {advisory}"));
            }
            err(advisory)
        }
        Ok((ids, warnings)) => {
            let nodes = if ids_only {
                ids.iter().map(|&id| Json::n(f64::from(id))).collect()
            } else {
                ids.iter().map(|&id| node_json(id, true)).collect()
            };
            let mut entries = vec![
                ("count", Json::n(ids.len() as f64)),
                (if ids_only { "ids" } else { "nodes" }, Json::arr(nodes)),
            ];
            if !warnings.is_empty() {
                entries.push((
                    "warnings",
                    Json::arr(warnings.into_iter().map(Json::Str).collect()),
                ));
            }
            ok(Json::obj(entries))
        }
    }
}

fn cmd_text(selector: &str, index: Option<f64>) -> String {
    match resolve_one(selector, index) {
        Err(e) => err(e),
        Ok(id) => {
            // A native <input>'s content lives in the widget, not in a child text node.
            let live = with_commit(|_, cx| sk_tree::devtools::input_value(id, cx)).flatten();
            let text = live
                .clone()
                .or_else(|| sk_tree::devtools::text_content(id))
                .unwrap_or_default();
            ok(Json::obj(vec![
                ("id", Json::n(f64::from(id))),
                ("text", Json::s(text)),
                (
                    "source",
                    Json::s(if live.is_some() {
                        "input-widget"
                    } else {
                        "textContent"
                    }),
                ),
            ]))
        }
    }
}

fn cmd_tree(selector: Option<&str>, depth: Option<f64>, node: Option<f64>) -> String {
    let root = match (selector, node) {
        (None, None) => sk_tree::devtools::root_id(),
        (sel, node) => match resolve_target(sel, None, node) {
            Ok(id) => id,
            Err(e) => return err(e),
        },
    };
    if root == 0 {
        return err("nothing is mounted yet");
    }
    let max = depth.map(|d| d as i32).unwrap_or(6);
    fn dump(id: u32, left: i32) -> Json {
        let Some(n) = sk_tree::devtools::node_info(id) else {
            return Json::Null;
        };
        let mut entries = vec![
            ("id", Json::n(f64::from(id))),
            ("kind", Json::s(n.kind)),
            ("tag", Json::s(n.tag)),
        ];
        if !n.classes.is_empty() {
            entries.push((
                "classes",
                Json::arr(n.classes.into_iter().map(Json::Str).collect()),
            ));
        }
        if !n.props.is_empty() {
            entries.push((
                "props",
                Json::Obj(
                    n.props
                        .into_iter()
                        .map(|(k, v)| (k, Json::Str(v)))
                        .collect(),
                ),
            ));
        }
        if !n.scopes.is_empty() {
            entries.push((
                "scopes",
                Json::arr(n.scopes.into_iter().map(Json::Str).collect()),
            ));
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

fn cmd_styles(selector: Option<&str>, index: Option<f64>, node: Option<f64>) -> String {
    match resolve_target(selector, index, node) {
        Err(e) => err(e),
        Ok(id) => match sk_tree::devtools::computed_styles(id) {
            None => err(format!(
                "node {id} has no computed style — it is a text/anchor node, or the cascade has \
                 not run since it was created"
            )),
            Some(buckets) => ok(Json::obj(vec![
                ("id", Json::n(f64::from(id))),
                ("hovered", Json::Bool(sk_tree::devtools::is_hovered(id))),
                (
                    "computed",
                    Json::Obj(
                        buckets
                            .into_iter()
                            .map(|(k, v)| {
                                (k.to_string(), Json::arr(v.lines().map(Json::s).collect()))
                            })
                            .collect(),
                    ),
                ),
            ])),
        },
    }
}

/// The rules that produced the values `styles` reports, in cascade order: selector, `@media`,
/// specificity, source order, state bucket, declarations as written, and sk-css's advisories.
fn cmd_rules(selector: Option<&str>, index: Option<f64>, node: Option<f64>) -> String {
    let id = match resolve_target(selector, index, node) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let Some(rules) = sk_tree::devtools::matched_rules(id) else {
        return err(format!(
            "node {id} has no matched rules to report — it is gone, or no stylesheet is installed"
        ));
    };
    let mut captured = 0usize;
    let items: Vec<Json> = rules
        .iter()
        .map(|r| {
            captured += r.declarations.len();
            let mut entries = vec![
                ("selector", Json::s(r.selector.clone())),
                ("specificity", Json::n(f64::from(r.specificity))),
                ("order", Json::n(f64::from(r.order))),
                ("state", Json::s(r.state)),
                (
                    "declarations",
                    Json::arr(
                        r.declarations
                            .iter()
                            .map(|(p, v, important)| {
                                let mut d = vec![
                                    ("property", Json::s(p.clone())),
                                    ("value", Json::s(v.clone())),
                                ];
                                if *important {
                                    d.push(("important", Json::Bool(true)));
                                }
                                Json::obj(d)
                            })
                            .collect(),
                    ),
                ),
            ];
            if let Some(m) = &r.media {
                entries.push(("media", Json::s(m.clone())));
            }
            if r.ancestor_state {
                entries.push(("ancestorState", Json::Bool(true)));
            }
            if let Some(p) = r.pseudo {
                entries.push(("pseudo", Json::s(p)));
            }
            if !r.diagnostics.is_empty() {
                entries.push((
                    "diagnostics",
                    Json::arr(r.diagnostics.iter().map(|d| Json::s(d.clone())).collect()),
                ));
            }
            Json::obj(entries)
        })
        .collect();
    let mut out = vec![
        ("id", Json::n(f64::from(id))),
        ("count", Json::n(items.len() as f64)),
        ("rules", Json::arr(items)),
    ];
    if captured == 0 && !rules.is_empty() {
        out.push((
            "note",
            Json::s(
                "no declaration text: this host's stylesheet was compiled with rule-text capture \
                 off — the selectors and their cascade order are still exact",
            ),
        ));
    }
    ok(Json::obj(out))
}

struct Highlight {
    node: u32,
    /// Which window paints the outline; `None` = the primary one.
    window: Option<u64>,
    /// When the overlay expires; `None` = until cleared. A polling client sets a short one so a
    /// crashed inspector leaves nothing behind.
    until: Option<std::time::Instant>,
    label: Option<String>,
}

static HIGHLIGHT: Mutex<Option<Highlight>> = Mutex::new(None);

/// The outline overlay for the highlighted node. Re-read from the bounds map every frame; it has
/// no hitbox, so it cannot intercept input.
pub fn highlight_overlay() -> Option<gpui::Div> {
    overlay_for(crate::main_window_id(), 0)
}

pub fn popout_highlight_overlay(window: u64, root: u32) -> Option<gpui::Div> {
    overlay_for(Some(window), root)
}

fn overlay_for(window: Option<u64>, root: u32) -> Option<gpui::Div> {
    use gpui::prelude::*;
    use gpui::{div, px, rgba};
    let mut slot = HIGHLIGHT.lock().unwrap();
    let expired = slot
        .as_ref()
        .and_then(|h| h.until)
        .is_some_and(|until| std::time::Instant::now() >= until);
    if expired {
        *slot = None;
    }
    let h = slot.as_ref()?;
    let wanted = h.window.or_else(crate::main_window_id);
    if wanted != window {
        return None;
    }
    // Not `node_info().bounds`: during render the live bounds map is cleared and not yet refilled.
    let (x, y, w, height) = sk_tree::painted_bounds_in(root, h.node)?;
    let label = h
        .label
        .clone()
        .unwrap_or_else(|| describe(h.node, w, height));
    Some(
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(height))
            .bg(rgba(0x6cb6ff30))
            .border_1()
            .border_color(rgba(0x6cb6ffcc))
            .child(
                div()
                    .absolute()
                    .top(px(if y >= 20.0 { -19.0 } else { 1.0 }))
                    .left(px(0.))
                    .px(px(5.))
                    .py(px(1.))
                    .bg(rgba(0x1b2838f2))
                    .border_1()
                    .border_color(rgba(0x6cb6ffcc))
                    .rounded(px(3.))
                    .text_size(px(11.))
                    .text_color(rgba(0xdceaffff))
                    // An absolutely positioned box has no width of its own; without this the chip wraps.
                    .whitespace_nowrap()
                    .child(label),
            ),
    )
}

fn describe(id: u32, w: f32, h: f32) -> String {
    let head = match sk_tree::devtools::node_info(id) {
        None => format!("#{id}"),
        Some(n) => {
            let mut s = n.tag;
            for c in n.classes.iter().take(3) {
                s.push('.');
                s.push_str(c);
            }
            s
        }
    };
    format!("{head} · {}×{}", w.round(), h.round())
}

fn resolve_target(
    selector: Option<&str>,
    index: Option<f64>,
    node: Option<f64>,
) -> Result<u32, String> {
    match (selector, node) {
        (Some(sel), _) => resolve_one(sel, index),
        (None, Some(id)) => {
            let id = id as u32;
            match sk_tree::devtools::node_info(id) {
                Some(_) => Ok(id),
                None => Err(format!("no node {id} in the tree")),
            }
        }
        (None, None) => Err("needs a `selector` or a `node` id".into()),
    }
}

fn cmd_highlight(
    selector: Option<&str>,
    index: Option<f64>,
    node: Option<f64>,
    clear: bool,
    ttl_ms: Option<f64>,
    label: Option<String>,
    window: Option<u64>,
) -> String {
    if clear || (selector.is_none() && node.is_none()) {
        *HIGHLIGHT.lock().unwrap() = None;
        let _ = with_target(|window, _| window.refresh());
        return ok(Json::obj(vec![("cleared", Json::Bool(true))]));
    }
    let id = match resolve_target(selector, index, node) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let ttl = ttl_ms
        .filter(|ms| *ms > 0.0)
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms as u64));
    *HIGHLIGHT.lock().unwrap() = Some(Highlight {
        node: id,
        window,
        until: ttl,
        label,
    });
    // Refresh the window that paints it; a pop-out does not repaint off the primary's refresh.
    let _ = with_target(|window, _| window.refresh());
    let bounds = sk_tree::devtools::node_info(id).and_then(|n| n.bounds);
    ok(Json::obj(vec![
        ("id", Json::n(f64::from(id))),
        ("bounds", bounds_json(bounds)),
        // Not painted (e.g. a `<KeepAlive>`d page) is not an error, but a silent no-op would look broken.
        ("painted", Json::Bool(bounds.is_some())),
    ]))
}

/// A poll, not a mode. Without `window` every window is asked and the one the pointer is over
/// wins. The hit test is gpui's own and must run inside the window whose frame owns the hitboxes.
fn cmd_pick(window_arg: Option<u64>) -> String {
    let candidates: Vec<u64> = match window_arg {
        Some(id) => vec![id],
        None => window_ids(),
    };
    if candidates.is_empty() {
        return err("this process has no window to pick in");
    }
    type Facts = (u64, f32, f32, bool, f32, f32, Vec<u32>);
    let mut chosen: Option<Facts> = None;
    for id in candidates {
        let root = window_root(id);
        let Some(facts) = with_window_id(id, |window, _| {
            let p = window.mouse_position();
            let size = window.viewport_size();
            (
                f32::from(p.x),
                f32::from(p.y),
                window.is_window_hovered(),
                f32::from(size.width),
                f32::from(size.height),
                sk_tree::devtools::hit_test_in(root, f32::from(p.x), f32::from(p.y), window),
            )
        }) else {
            continue;
        };
        let (x, y, inside, vw, vh, path) = facts;
        if inside || chosen.is_none() {
            chosen = Some((id, x, y, inside, vw, vh, path));
        }
        if inside {
            break;
        }
    }
    let Some((id, x, y, inside, vw, vh, path)) = chosen else {
        return err("no such window");
    };
    let node = path.last().copied();
    let mut entries = vec![
        ("x", Json::n(x)),
        ("y", Json::n(y)),
        ("insideWindow", Json::Bool(inside)),
        ("window", Json::n(id as f64)),
        (
            "viewport",
            Json::obj(vec![("w", Json::n(vw)), ("h", Json::n(vh))]),
        ),
        (
            "path",
            Json::arr(path.iter().map(|&id| Json::n(f64::from(id))).collect()),
        ),
    ];
    match node {
        None => entries.push(("node", Json::Null)),
        Some(id) => {
            entries.push(("node", node_json(id, false)));
        }
    }
    ok(Json::obj(entries))
}

fn cmd_click(selector: &str, index: Option<f64>, button: Option<f64>) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let Some(info) = sk_tree::devtools::node_info(id) else {
        return err(format!(
            "node {id} vanished between the query and the click"
        ));
    };
    let Some((x, y, w, h)) = info.bounds else {
        return err(format!(
            "node {id} has no bounds — it was not painted in the last frame (detached, or parked \
             in a <KeepAlive> cache), so there is no point to click"
        ));
    };
    // Must match what `mouse` would aim at; the raw centre is the fallback for a node gpui would not hit.
    let (cx_, cy) = with_node_window(id, |window, _| sk_tree::devtools::aim_point(id, window))
        .flatten()
        .unwrap_or((x + w / 2.0, y + h / 2.0));
    // The listener chain belongs to the window that paints the node.
    let proceed = with_node_window(id, |window, app| {
        sk_tree::devtools::click(id, cx_, cy, button.unwrap_or(0.0), window, app)
    });
    match proceed {
        None => err("not in a commit phase"),
        Some(proceed) => ok(Json::obj(vec![
            ("id", Json::n(f64::from(id))),
            (
                "at",
                Json::obj(vec![("x", Json::n(cx_)), ("y", Json::n(cy))]),
            ),
            ("defaultPrevented", Json::Bool(!proceed)),
        ])),
    }
}

fn cmd_scroll(selector: &str, index: Option<f64>, dx: Option<f64>, dy: Option<f64>) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let out = with_node_window(id, |window, app| {
        sk_tree::devtools::scroll(
            id,
            dx.unwrap_or(0.0) as f32,
            dy.unwrap_or(0.0) as f32,
            window,
            app,
        )
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
    // Focus is per window: the handle goes on the painting window's focus path, where keystrokes travel.
    match with_node_window(id, |window, app| sk_tree::devtools::focus(id, window, app)) {
        None => err("not in a commit phase"),
        Some(Err(e)) => err(e),
        Some(Ok(())) => ok(Json::obj(vec![("id", Json::n(f64::from(id)))])),
    }
}

/// A text field's IME: `text` is the composition's new pre-edit, or with `commit` its committed
/// text. The field need not be focused.
fn cmd_ime(selector: &str, index: Option<f64>, text: &str, commit: bool) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let out = with_node_window(id, |window, app| {
        let done = sk_tree::devtools::ime(id, text, commit, window, app);
        if sk_tree::flush_changes() {
            window.refresh();
        }
        done.map(|()| sk_tree::devtools::input_value(id, app))
    });
    match out {
        None => err("not in a commit phase"),
        Some(Err(e)) => err(e),
        Some(Ok(value)) => ok(Json::obj(vec![
            ("node", Json::n(f64::from(id))),
            ("value", value.map_or(Json::Null, Json::s)),
        ])),
    }
}

/// What a `mouse` gesture carries besides position and button: a move with the button held (a
/// drag), the press's click count (2 and 3 select a word and a paragraph) and Shift.
struct MouseExtras {
    held: bool,
    clicks: usize,
    shift: bool,
}

/// Input stream for one mouse command. The move before a release still has the button held; an
/// unpressed move first would cancel a captured drag.
fn mouse_inputs(
    kind: &str,
    button: gpui::MouseButton,
    position: gpui::Point<gpui::Pixels>,
    modifiers: gpui::Modifiers,
    extras: &MouseExtras,
) -> Vec<gpui::PlatformInput> {
    let mut events = Vec::with_capacity(3);
    events.push(gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
        position,
        pressed_button: (extras.held || kind == "up").then_some(button),
        modifiers,
    }));
    if matches!(kind, "down" | "click") {
        events.push(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: extras.clicks,
            first_mouse: false,
        }));
    }
    if matches!(kind, "up" | "click") {
        events.push(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: extras.clicks,
        }));
    }
    events
}

/// One window for aim and dispatch: a selector names the window that paints the node, bare x/y
/// use the command's window.
fn in_window<R>(node: Option<u32>, f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    match node {
        Some(id) => with_node_window(id, f),
        None => with_target(f),
    }
}

/// The node a gesture is aimed at, if a selector named one, and the window point it lands on.
fn aim(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    cmd: &str,
) -> Result<(Option<u32>, (f32, f32)), String> {
    let node = selector.map(|sel| resolve_one(sel, index)).transpose()?;
    let point = match (node, x, y) {
        (Some(id), _, _) => {
            // Not the centre of `bounds` (untransformed): gpui's matrix maps it, and gpui's hit
            // test then confirms the point lands on the node, so an unreachable control is reported.
            match in_window(Some(id), |window, _| {
                sk_tree::devtools::aim_point(id, window)
            }) {
                None => return Err("not in a commit phase".into()),
                Some(Some(p)) => p,
                Some(None) => {
                    return Err(format!(
                        "node {id} is not reachable at its own centre — either it was not painted \
                         in the last frame (detached, or parked in a <KeepAlive> cache), or gpui's \
                         hit test says something else is there: an overlay above it, a clipping \
                         scroll container it has scrolled out of, `scale(0)`. Aim with x/y to send \
                         the gesture at a window coordinate anyway"
                    ))
                }
            }
        }
        (None, Some(x), Some(y)) => (x as f32, y as f32),
        _ => {
            return Err(format!(
                "{cmd} needs either a `selector` or both `x` and `y` (window coordinates)"
            ))
        }
    };
    Ok((node, point))
}

/// What a `pen` gesture carries besides its kind and position.
struct PenExtras {
    pressure: f32,
    tilt_x: f32,
    tilt_y: f32,
    twist: f32,
    eraser: bool,
    barrel: bool,
    /// Readings before this one, from `from` to the position, each its own sample: the move's
    /// coalesced events.
    from: Option<(f32, f32)>,
    /// The pressure at `from`; the readings ramp from it. Defaults to `pressure`.
    from_pressure: Option<f32>,
    steps: usize,
    /// Milliseconds between the readings; the last is taken now.
    interval: f64,
}

fn pen_input(
    kind: &str,
    position: gpui::Point<gpui::Pixels>,
    extras: &PenExtras,
) -> gpui::PlatformInput {
    let contact = matches!(kind, "down" | "move");
    let button = if extras.barrel {
        gpui::MouseButton::Right
    } else {
        gpui::MouseButton::Left
    };
    let now = std::time::Instant::now();
    let reading = |position, pressure, before: usize| gpui::PenSample {
        position,
        pressure,
        tilt_x: extras.tilt_x,
        tilt_y: extras.tilt_y,
        twist: extras.twist,
        time: now.checked_sub(std::time::Duration::from_secs_f64(
            extras.interval * before as f64 / 1000.0,
        )),
        ..Default::default()
    };
    let pressure = if contact { extras.pressure } else { 0.0 };
    let history = match extras.from {
        Some((fx, fy)) if extras.steps > 1 => {
            let n = extras.steps;
            (1..n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let at = gpui::point(
                        gpui::px(fx + (f32::from(position.x) - fx) * t),
                        gpui::px(fy + (f32::from(position.y) - fy) * t),
                    );
                    let start = if contact {
                        extras.from_pressure.unwrap_or(pressure)
                    } else {
                        0.0
                    };
                    reading(at, start + (pressure - start) * t, n - i)
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let modifiers = gpui::Modifiers::default();
    let action = match kind {
        "down" => gpui::PenAction::Down(gpui::MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        "up" => gpui::PenAction::Up(gpui::MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: 1,
        }),
        "leave" => gpui::PenAction::Exited(gpui::MouseExitEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        _ => gpui::PenAction::Move(gpui::MouseMoveEvent {
            position,
            pressed_button: contact.then_some(button),
            modifiers,
        }),
    };
    gpui::PlatformInput::Pen(gpui::PenEvent {
        action,
        pen: gpui::PenInput {
            pointer_id: 1,
            eraser: extras.eraser,
            barrel: extras.barrel,
            in_contact: contact,
            sample: reading(position, pressure, 0),
            history,
        },
    })
}

/// A pen gesture through gpui's dispatch, the way a platform reports a stylus. No OS input.
fn cmd_pen(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    kind: Option<&str>,
    extras: PenExtras,
) -> String {
    let (node, point) = match aim(selector, index, x, y, "pen") {
        Ok(aimed) => aimed,
        Err(e) => return err(e),
    };
    let kind = kind.unwrap_or("hover");
    if !matches!(kind, "hover" | "down" | "move" | "up" | "leave") {
        return err(format!(
            "unknown pen kind `{kind}` — hover | down | move | up | leave"
        ));
    }
    let position = gpui::Point::new(gpui::px(point.0), gpui::px(point.1));
    let done = in_window(node, |window, app| {
        window.dispatch_event(pen_input(kind, position, &extras), app);
        window.refresh();
    });
    match done {
        None => err("not in a commit phase"),
        Some(()) => ok(Json::obj(vec![
            ("x", Json::n(f64::from(point.0))),
            ("y", Json::n(f64::from(point.1))),
            ("kind", Json::s(kind.to_string())),
        ])),
    }
}

fn cmd_drag_files(req: &Json, document: bool) -> String {
    let s = |key: &str| req.get(key).and_then(Json::as_str);
    let n = |key: &str| req.get(key).and_then(Json::as_f64);
    let flag = |key: &str| req.get(key).and_then(Json::as_bool).unwrap_or(false);
    let kind = s("kind").unwrap_or("drop");
    let driven = if document {
        match kind {
            "arm" => {
                return if sk_tree::devtools::arm_document_drag() {
                    ok(Json::Bool(true))
                } else {
                    err("another document drag is pending")
                }
            }
            "cancel" => {
                return if sk_tree::devtools::finish_document_drag(gpui::DropOperation::None) {
                    ok(Json::Bool(true))
                } else {
                    err("no document drag to cancel")
                }
            }
            _ => match sk_tree::devtools::document_drag_offer() {
                Some(offer) => Some(offer),
                None => return err("no document drag has started"),
            },
        }
    } else {
        None
    };
    let (node, (x, y)) = match aim(s("selector"), n("index"), n("x"), n("y"), "drag-files") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let at = gpui::point(gpui::px(x), gpui::px(y));
    let paths = req
        .get("paths")
        .and_then(Json::as_arr)
        .unwrap_or_default()
        .iter()
        .filter_map(Json::as_str)
        .map(std::path::PathBuf::from)
        .collect();
    let enter = gpui::FileDropEvent::Entered {
        position: at,
        paths: gpui::ExternalPaths(paths),
    };
    let submit = gpui::FileDropEvent::Submit { position: at };
    let sequence = match kind {
        "drop" if req.get("paths").is_some() || s("text").is_some() || s("url").is_some() => {
            vec![enter, submit]
        }
        "enter" => vec![enter],
        "over" => vec![gpui::FileDropEvent::Pending { position: at }],
        "leave" => vec![gpui::FileDropEvent::Exited],
        "drop" => vec![submit],
        _ => return err("drag-files kind must be enter, over, leave or drop"),
    };
    let offer = driven.unwrap_or_else(|| gpui::DragOffer {
        allowed: n("allowed").map_or(7, |mask| mask.clamp(0., 7.) as u8),
        modifiers: gpui::Modifiers {
            control: flag("ctrl"),
            shift: flag("shift"),
            alt: flag("alt"),
            platform: flag("meta"),
            ..Default::default()
        },
        strings: [("text", "text/plain"), ("url", "text/uri-list")]
            .into_iter()
            .filter_map(|(key, mime)| Some((mime.into(), s(key)?.into())))
            .collect(),
    });
    in_window(node, |window, cx| {
        let mut op = gpui::DropOperation::None;
        for event in sequence {
            op = window
                .dispatch_event(gpui::PlatformInput::ExternalDrop(event, offer.clone()), cx)
                .drop_operation
                .unwrap_or_default();
        }
        if document && kind == "drop" {
            sk_tree::devtools::finish_document_drag(op);
        }
        let effect = match op {
            gpui::DropOperation::None => "none",
            gpui::DropOperation::Move => "move",
            gpui::DropOperation::Link => "link",
            gpui::DropOperation::Copy => "copy",
        };
        ok(Json::obj(vec![("dropEffect", Json::s(effect))]))
    })
    .unwrap_or_else(|| err("drag-files has no active window"))
}

fn cmd_mouse_pressure(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    stage: Option<f64>,
    pressure: Option<f64>,
) -> String {
    let stage = match stage {
        Some(0.0) => gpui::PressureStage::Zero,
        Some(1.0) => gpui::PressureStage::Normal,
        Some(2.0) => gpui::PressureStage::Force,
        _ => return err("mouse_pressure needs stage 0, 1 or 2"),
    };
    let Some(pressure) = pressure else {
        return err("mouse_pressure needs pressure");
    };
    let (node, (x, y)) = match aim(selector, index, x, y, "mouse_pressure") {
        Ok(aimed) => aimed,
        Err(e) => return err(e),
    };
    match in_window(node, |window, app| {
        window.dispatch_event(
            gpui::PlatformInput::MousePressure(gpui::MousePressureEvent {
                stage,
                pressure: pressure as f32,
                position: gpui::point(gpui::px(x), gpui::px(y)),
                modifiers: Default::default(),
            }),
            app,
        );
    }) {
        Some(()) => ok(Json::Bool(true)),
        None => err("not in a commit phase"),
    }
}

/// Unlike `click` (synthesized into JS from a node id), content masks, paint order,
/// `pointer-events` and hover all apply. No OS cursor moves.
fn cmd_mouse(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    kind: Option<&str>,
    button: Option<f64>,
    extras: MouseExtras,
) -> String {
    let (node, point) = match aim(selector, index, x, y, "mouse") {
        Ok(aimed) => aimed,
        Err(e) => return err(e),
    };
    let kind = kind.unwrap_or("move");
    if !matches!(kind, "move" | "down" | "up" | "click") {
        return err(format!(
            "unknown mouse kind `{kind}` — move | down | up | click"
        ));
    }
    let button = match button.unwrap_or(0.0) as i64 {
        0 => gpui::MouseButton::Left,
        1 => gpui::MouseButton::Middle,
        2 => gpui::MouseButton::Right,
        other => {
            return err(format!(
                "unknown mouse button {other} — 0 left, 1 middle, 2 right"
            ))
        }
    };
    let position = gpui::Point::new(gpui::px(point.0), gpui::px(point.1));
    let modifiers = gpui::Modifiers {
        shift: extras.shift,
        ..Default::default()
    };
    // Not `with_commit`: a coordinate needs its window, and a pop-out has its own client space.
    let done = in_window(node, |window, app| {
        for input in mouse_inputs(kind, button, position, modifiers, &extras) {
            window.dispatch_event(input, app);
        }
        // Hover is read at paint time, so ask for the frame.
        window.refresh();
        sk_tree::devtools::focused_node(window, app)
    });
    match done {
        None => err("not in a commit phase"),
        Some(focused) => ok(Json::obj(vec![
            ("x", Json::n(f64::from(point.0))),
            ("y", Json::n(f64::from(point.1))),
            ("kind", Json::s(kind.to_string())),
            // Focus owner after the gesture; JS cannot report it while the window is inactive.
            match focused {
                Some(id) => ("focused", Json::n(f64::from(id))),
                None => ("focused", Json::Null),
            },
        ])),
    }
}

fn cmd_eval(src: &str) -> String {
    let result = JsRuntime::with(|runtime| eval::evaluate(runtime.ctx(), src));
    match result {
        Err(e) => err(format!("eval helper failed: {e}")),
        Ok(text) => {
            let _ = with_commit(|_window, cx| {
                if sk_tree::flush_changes() {
                    // Preserve unaffected cached subtrees, as after_wave does for the same DOM change.
                    crate::notify_shell(cx);
                    crate::popout::notify_all(cx);
                }
            });
            ok(Json::from(Raw(text)))
        }
    }
}

fn cmd_logs(since: Option<f64>) -> String {
    let since = since.unwrap_or(0.0).max(0.0) as u64;
    let (entries, missed) = sk_tree::log::since(since);
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
                            (
                                "styles",
                                match &e.styles {
                                    Some(runs) => Json::from(Raw(runs.clone())),
                                    None => Json::Null,
                                },
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("next", Json::n(sk_tree::log::next_seq() as f64)),
        (
            "status",
            match sk_tree::log::status() {
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

/// Both focus first and finish on the NEXT frame (see [`Pending`]); `key` without a selector goes
/// to the focused element and does not wait.
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
            let out = with_target(|window, app| {
                for k in &keys {
                    match parse_keystroke(k) {
                        Ok(ks) => send_keystroke(ks, window, app),
                        Err(e) => return err(e),
                    }
                }
                if sk_tree::flush_changes() {
                    window.refresh();
                }
                ok(Json::obj(vec![("delivered", Json::n(keys.len() as f64))]))
            });
            return Outcome::Done(out.unwrap_or_else(|| err("not in a commit phase")));
        }
        None => return Outcome::Done(err("type needs a `selector`")),
    };
    let focus = with_node_window(node, |window, app| {
        sk_tree::devtools::focus(node, window, app)
    });
    match focus {
        None => return Outcome::Done(err("not in a commit phase")),
        Some(Err(e)) => return Outcome::Done(err(e)),
        Some(Ok(())) => {}
    }
    // Keystrokes wait a frame (see [`Pending`]); the window rides along because the next command
    // resets `TARGET_WINDOW`.
    Outcome::Parked(Pending {
        generation: ACCEPT_GEN.load(Ordering::SeqCst),
        node,
        text,
        keys,
        reply: reply.clone(),
        waited: 0,
        window: TARGET_WINDOW.with(|c| c.get()),
    })
}

/// One printable character as a real key press: `keydown` (honoring `preventDefault()`) via the
/// input's `intercept_keystrokes` hook, then the input handler inserts it and fires `input`.
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
    // `dispatch_keystroke` covers key-down plus the character; key-up has no wrapper, so it goes
    // through `dispatch_event` for `@keyup` handlers.
    window.dispatch_keystroke(ks.clone(), cx);
    window.dispatch_event(
        gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
            keystroke: ks,
            native: None,
        }),
        cx,
    );
}

/// `Enter`, `Shift+Tab`, `Ctrl+A`, `ArrowDown` to a gpui [`gpui::Keystroke`]. Accepts DOM
/// `KeyboardEvent.key` spellings as well as gpui's, with `+` or `-` as the separator.
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
        // DOM `KeyboardEvent.key` to gpui names; the inverse of `render::dom_key`.
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "pageup" => "pageup",
        "pagedown" => "pagedown",
        // gpui calls VK_APPS "menu"; the DOM calls it "ContextMenu".
        "contextmenu" => "menu",
        " " => "space",
        lower => lower,
    }
    .to_string();
    // A printable single character needs `key_char`, or gpui delivers the binding but not the text.
    let key_char = (raw.chars().count() == 1 && !modifiers.control && !modifiers.platform)
        .then(|| raw.to_string())
        .or_else(|| (name == "space").then(|| " ".to_string()));
    Ok(gpui::Keystroke {
        modifiers,
        key: name,
        key_char,
    })
}

/// Unlike `scroll` (which drives a `ScrollHandle` directly). Deltas are the DOM's: `dy > 0` scrolls down.
fn cmd_wheel(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    dx: Option<f64>,
    dy: Option<f64>,
    lines: bool,
    modifiers: gpui::Modifiers,
) -> String {
    let (point, node) = match (selector, x, y) {
        (Some(sel), _, _) => match resolve_one(sel, index) {
            Err(e) => return err(e),
            Ok(id) => match sk_tree::devtools::node_info(id).and_then(|n| n.bounds) {
                Some((bx, by, bw, bh)) => ((bx + bw / 2.0, by + bh / 2.0), Some(id)),
                None => {
                    return err(format!(
                        "`{sel}` is not painted, so it has no centre to aim at"
                    ))
                }
            },
        },
        (None, Some(x), Some(y)) => ((x as f32, y as f32), None),
        _ => {
            return err("wheel needs either a `selector` or both `x` and `y` (window coordinates)")
        }
    };
    let (dx, dy) = (dx.unwrap_or(0.0) as f32, dy.unwrap_or(0.0) as f32);
    if dx == 0.0 && dy == 0.0 {
        return err("wheel needs a `dx` or a `dy` (DOM signs: dy > 0 scrolls down)");
    }
    let position = gpui::Point::new(gpui::px(point.0), gpui::px(point.1));
    let wheel = |window: &mut Window, app: &mut App| {
        window.dispatch_event(
            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers,
            }),
            app,
        );
        window.dispatch_event(
            gpui::PlatformInput::Scroll(gpui::ScrollEvent {
                position,
                // Negated: gpui's sign is the platform's, the DOM's is the opposite.
                delta: if lines {
                    gpui::ScrollDelta::Lines(gpui::Point::new(-dx, -dy))
                } else {
                    gpui::ScrollDelta::Pixels(gpui::Point::new(gpui::px(-dx), gpui::px(-dy)))
                },
                modifiers,
                touch_phase: gpui::TouchPhase::Moved,
                // A wheel as `gpui_windows` reports one, not a touch pan (which goes through the gesture recognizer).
                source: gpui::ScrollSource::Pointer,
            }),
            app,
        );
        if sk_tree::flush_changes() {
            window.refresh();
        }
        true
    };
    let done = match node {
        Some(id) => with_node_window(id, wheel),
        None => with_target(wheel),
    };
    match done {
        None => err("no such window"),
        Some(_) => ok(Json::obj(vec![
            ("x", Json::n(point.0)),
            ("y", Json::n(point.1)),
            ("dx", Json::n(dx)),
            ("dy", Json::n(dy)),
        ])),
    }
}

fn cmd_windows() -> String {
    let main = crate::main_window_id();
    let popouts = crate::popout::list();
    let mut out: Vec<Json> = Vec::new();
    for id in window_ids() {
        let root = window_root(id);
        let title = if Some(id) == main {
            crate::window::config().title
        } else {
            popouts
                .iter()
                .find(|(w, _, _)| *w == id)
                .map_or_else(String::new, |(_, _, t)| t.clone())
        };
        let facts = with_window_id(id, |window, _| {
            let size = window.viewport_size();
            (
                f32::from(size.width),
                f32::from(size.height),
                window.is_window_active(),
                window.is_window_hovered(),
                window.scale_factor(),
                window.bounds(),
            )
        });
        let (w, h, active, hovered, scale, bounds) =
            facts.unwrap_or((0.0, 0.0, false, false, 1.0, gpui::Bounds::default()));
        let popup = sk_tree::popup_windows().into_iter().find(|p| p.0 == id);
        let kind = match (Some(id) == main, popup) {
            (true, _) => "main",
            (false, Some(_)) => "popup",
            (false, None) => "popout",
        };
        let mut entry = vec![
            ("id", Json::n(id as f64)),
            ("kind", Json::s(kind)),
            ("title", Json::s(title)),
            ("root", Json::n(f64::from(root))),
            (
                "viewport",
                Json::obj(vec![("w", Json::n(w)), ("h", Json::n(h))]),
            ),
            ("scale", Json::n(scale)),
            ("focused", Json::Bool(active)),
            ("hovered", Json::Bool(hovered)),
            (
                "screen",
                Json::obj(vec![
                    ("x", Json::n(f64::from(f32::from(bounds.origin.x)))),
                    ("y", Json::n(f64::from(f32::from(bounds.origin.y)))),
                    ("w", Json::n(f64::from(f32::from(bounds.size.width)))),
                    ("h", Json::n(f64::from(f32::from(bounds.size.height)))),
                ]),
            ),
        ];
        if let Some((_, owner, margin)) = popup {
            entry.push(("owner", Json::n(owner as f64)));
            entry.push(("margin", Json::n(f64::from(margin))));
        }
        out.push(Json::obj(entry));
    }
    ok(Json::obj(vec![
        ("windows", Json::arr(out)),
        ("pid", Json::n(std::process::id() as f64)),
        (
            "inspectors",
            Json::arr(
                crate::inspector::children()
                    .into_iter()
                    .map(|(window, pid)| {
                        Json::obj(vec![
                            (
                                "window",
                                match window {
                                    Some(w) => Json::n(w as f64),
                                    None => Json::Null,
                                },
                            ),
                            ("pid", Json::n(f64::from(pid))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

/// Brings this window to the front; the only command that deliberately takes focus.
fn cmd_raise() -> String {
    // A test host stays behind the person's windows (src/quiet.rs).
    if crate::quiet::never_focus() {
        return ok(Json::obj(vec![
            ("raised", Json::Bool(false)),
            ("why", Json::s("SK_WINDOW_FOCUS=never")),
        ]));
    }
    let done = with_target(|window, _| {
        window.activate_window();
        true
    });
    match done {
        Some(_) => ok(Json::obj(vec![("raised", Json::Bool(true))])),
        None => err("no such window"),
    }
}

fn cmd_trace(op: Option<&str>, frames: bool, label: Option<&str>, cap: Option<f64>) -> String {
    let result = match op.unwrap_or("peek") {
        "start" => trace::start(cap.map(|c| c.max(0.0) as usize)),
        "stop" => trace::report(true, frames),
        "peek" => trace::report(false, frames),
        "mark" => {
            trace::mark(label.unwrap_or(""));
            Ok(Json::obj(vec![("marked", Json::Bool(true))]))
        }
        other => Err(format!(
            "trace: unknown op `{other}` — start, stop, peek or mark"
        )),
    };
    match result {
        Ok(data) => ok(data),
        Err(e) => err(e),
    }
}

/// The samples a `perf` reply carries: the tail of the ring, since the push channel sends this
/// four times a second.
const PERF_SAMPLES: usize = 240;

fn perf_data() -> Json {
    let snap = crate::perf::snapshot();
    let (avg, p50, p95, max, fps) = snap.stats();
    let (work_p50, work_p95, work_max) = snap.work_stats();
    let round2 = |v: f32| f64::from((v * 100.0).round() / 100.0);
    let skip = snap.samples.len().saturating_sub(PERF_SAMPLES);
    let tail: Vec<Json> = snap.samples[skip..]
        .iter()
        .map(|ms| Json::n(round2(*ms)))
        .collect();
    let work: Vec<Json> = snap.work[skip..]
        .iter()
        .map(|ms| Json::n(round2(*ms)))
        .collect();
    let flags: Vec<Json> = snap.flags[skip..]
        .iter()
        .map(|f| Json::n(f64::from(*f)))
        .collect();
    let share = |flag: u8| Json::n(f64::from((snap.share(flag) * 1000.0).round() / 1000.0));
    Json::obj(vec![
        ("frames", Json::n(snap.frames as f64)),
        ("uptimeMs", Json::n(f64::from(snap.uptime_ms.round()))),
        (
            "bootMs",
            match snap.boot_to_first_frame_ms {
                Some(ms) => Json::n(f64::from((ms * 10.0).round() / 10.0)),
                None => Json::Null,
            },
        ),
        (
            "bootJs",
            Json::arr(
                crate::perf::boot_js()
                    .into_iter()
                    .map(|(label, scripts, ms)| {
                        Json::obj(vec![
                            ("label", Json::s(label)),
                            ("scripts", Json::n(f64::from(scripts))),
                            ("ms", Json::n(round2(ms))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("avgMs", Json::n(round2(avg))),
        ("p50Ms", Json::n(round2(p50))),
        ("p95Ms", Json::n(round2(p95))),
        ("maxMs", Json::n(round2(max))),
        ("fps", Json::n(f64::from((fps * 10.0).round() / 10.0))),
        (
            "budgetMs",
            match crate::window::frame_budget_ms() {
                Some(ms) => Json::n((f64::from(ms) * 100.0).round() / 100.0),
                None => Json::Null,
            },
        ),
        ("samples", Json::arr(tail)),
        ("work", Json::arr(work)),
        ("flags", Json::arr(flags)),
        ("workP50Ms", Json::n(round2(work_p50))),
        ("workP95Ms", Json::n(round2(work_p95))),
        ("workMaxMs", Json::n(round2(work_max))),
        ("pacedShare", share(crate::perf::FLAG_PACED)),
        ("drewShare", share(crate::perf::FLAG_DREW)),
        ("throttledShare", share(crate::perf::FLAG_THROTTLED)),
        ("idle", crate::idle::status()),
        ("nodes", Json::n(f64::from(sk_tree::devtools::node_count()))),
        (
            "builds",
            Json::n(sk_tree::devtools::frame_counts().0 as f64),
        ),
        (
            "presents",
            Json::n(sk_tree::devtools::frame_counts().1 as f64),
        ),
        (
            "containerRelayouts",
            Json::n(sk_tree::devtools::container_query_counts().0 as f64),
        ),
        (
            "containersMoved",
            Json::n(sk_tree::devtools::container_query_counts().1 as f64),
        ),
        // Scroll containers registered with the wheel router last frame. Filled by the paint walk,
        // so a retained-render replay cannot rebuild it.
        (
            "scrollChain",
            Json::n(f64::from(sk_tree::devtools::scroll_chain_count())),
        ),
        ("windows", Json::n(window_ids().len() as f64)),
        ("phases", Json::Bool(sk_tree::perf::env_enabled())),
    ])
}

pub(crate) fn perf_json() -> String {
    let Json::Obj(mut entries) = perf_data() else {
        return String::new();
    };
    entries.insert(0, ("t".to_string(), Json::s("perf")));
    Json::Obj(entries).to_json()
}

/// Goes through the app's own JS (`eval`), not `sk-storage`, so pending writes are included.
fn cmd_storage(
    area: Option<&str>,
    op: Option<&str>,
    key: Option<&str>,
    value: Option<&str>,
) -> String {
    let area = area.unwrap_or("local");
    let global = match area {
        "local" | "localStorage" => "localStorage",
        "session" | "sessionStorage" => "sessionStorage",
        other => {
            return err(format!(
                "unknown storage area {other:?} — use \"local\" or \"session\""
            ))
        }
    };
    let quoted_area = json::quote(area);
    let js = match op.unwrap_or("list") {
        "list" => format!(
            "(function(){{var a=globalThis.{global};if(!a)return null;var o=[];\
             for(var i=0;i<a.length;i++){{var k=a.key(i);o.push({{key:k,value:a.getItem(k)}});}}\
             return {{area:{quoted_area},length:a.length,entries:o}};}})()"
        ),
        "set" => {
            let Some(k) = key else {
                return err("storage set needs a `key`");
            };
            let k = json::quote(k);
            let v = json::quote(value.unwrap_or(""));
            format!(
                "(function(){{var a=globalThis.{global};if(!a)return null;a.setItem({k},{v});\
                 return {{area:{quoted_area},key:{k},value:a.getItem({k})}};}})()"
            )
        }
        "remove" => {
            let Some(k) = key else {
                return err("storage remove needs a `key`");
            };
            let k = json::quote(k);
            format!(
                "(function(){{var a=globalThis.{global};if(!a)return null;a.removeItem({k});\
                 return {{area:{quoted_area},key:{k},removed:true}};}})()"
            )
        }
        "clear" => format!(
            "(function(){{var a=globalThis.{global};if(!a)return null;a.clear();\
             return {{area:{quoted_area},cleared:true,length:a.length}};}})()"
        ),
        other => {
            return err(format!(
                "unknown storage op {other:?} — use \"list\", \"set\", \"remove\" or \"clear\""
            ))
        }
    };
    let reply = cmd_eval(&js);
    let Some(parsed) = json::parse(&reply) else {
        return reply;
    };
    if parsed.get("ok").and_then(Json::as_bool) != Some(true) {
        return reply;
    }
    match parsed.get("data").and_then(|d| d.get("value")) {
        Some(Json::Null) | None => err(format!(
            "this host has no `{global}` — it was built without the `storages` feature"
        )),
        Some(v) => ok(v.clone()),
    }
}

/// Scripted answers for waiting dialogs; only under `SK_DIALOGS=scripted` (the devtools never
/// answer a real dialog). `list` (default); `answer` with `paths` (a JSON array, or one string
/// split by `|`); `choose` a question's `button` (label or index); `cancel`.
fn cmd_dialog(
    op: Option<&str>,
    id: Option<f64>,
    paths: Option<&Json>,
    button: Option<String>,
) -> String {
    use sk_desktop::dialog::{scripted, scripted_mode};
    if !scripted_mode() {
        return err(
            "dialog needs the app started with SK_DIALOGS=scripted — without it every dialog is a \
             real one, which is the user's to answer",
        );
    }
    fn embed(v: impl std::fmt::Display) -> Json {
        Json::from(Raw(v.to_string()))
    }
    let id: Option<u64> = id.filter(|n| n.is_finite() && *n > 0.0).map(|n| n as u64);
    match op.unwrap_or("list") {
        "list" => ok(embed(scripted::list())),
        "answer" => {
            let Some(id) = id else {
                return err("dialog answer needs the dialog's `id` (see `dialog list`)");
            };
            let list: Vec<std::path::PathBuf> = match paths {
                Some(Json::Arr(items)) => items.iter().filter_map(Json::as_str).map(Into::into).collect(),
                Some(Json::Str(s)) => s.split('|').map(str::trim).filter(|p| !p.is_empty()).map(Into::into).collect(),
                _ => return err("dialog answer needs `paths`: an array of absolute paths, or one string split by `|`"),
            };
            match scripted::answer(id, list) {
                Ok(v) => ok(embed(v)),
                Err(e) => err(e),
            }
        }
        "choose" => {
            let Some(id) = id else {
                return err("dialog choose needs the dialog's `id` (see `dialog list`)");
            };
            let Some(button) = button else {
                return err("dialog choose needs a `button`: its label or its index");
            };
            match scripted::choose(id, &button) {
                Ok(v) => ok(embed(v)),
                Err(e) => err(e),
            }
        }
        "cancel" => {
            let Some(id) = id else {
                return err("dialog cancel needs the dialog's `id` (see `dialog list`)");
            };
            match scripted::cancel(id) {
                Ok(v) => ok(embed(v)),
                Err(e) => err(e),
            }
        }
        other => err(format!(
            "unknown dialog op {other:?} — use \"list\", \"answer\", \"choose\" or \"cancel\""
        )),
    }
}

/// `list` (default) gives each capability's `state` and the stored `decisions`; `reset` forgets
/// the answer for `name`, or all, so the next request asks again. Works on any host, unlike `dialog`.
fn cmd_permissions(op: Option<&str>, name: Option<&str>) -> String {
    match op.unwrap_or("list") {
        "list" => ok(Json::from(Raw(sk_permissions::describe_json()))),
        "reset" => match sk_permissions::reset(name) {
            Ok(names) => ok(Json::obj(vec![
                ("reset", Json::arr(names.into_iter().map(Json::s).collect())),
                ("now", Json::from(Raw(sk_permissions::describe_json()))),
            ])),
            Err(e) => err(e),
        },
        other => err(format!(
            "unknown permissions op {other:?} — use \"list\" or \"reset\""
        )),
    }
}

fn cmd_net(since: Option<f64>) -> String {
    let since = since.unwrap_or(0.0).max(0.0) as u64;
    let (entries, next, dropped) = net::since(since);
    ok(Json::obj(vec![
        (
            "entries",
            Json::arr(entries.into_iter().map(|t| Json::from(Raw(t))).collect()),
        ),
        ("next", Json::n(next as f64)),
        ("dropped", Json::n(dropped as f64)),
        ("pool", net::pool_json()),
    ]))
}

/// Everything behind one Network row (headers, bodies, cookies, timing, socket counts). Asked
/// per row so bodies never ride the feed.
fn cmd_net_detail(seq: f64) -> String {
    if !seq.is_finite() || seq < 1.0 {
        return err(format!("net_detail: {seq} is not a row's seq"));
    }
    match net::detail(seq as u64) {
        Ok(data) => ok(data),
        Err(e) => err(e),
    }
}

fn cmd_screenshot(path: Option<&str>, backend: Option<&str>) -> String {
    if let Some(id) = TARGET_WINDOW.with(|c| c.get()) {
        if !window_ids().contains(&id) {
            return err(format!(
                "no window {id} in this process — `windows` lists the ids"
            ));
        }
    }
    let target = match path {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => screenshot::next_default_path(),
    };
    let backend = match backend {
        None => screenshot::Backend::default(),
        Some(name) => match screenshot::Backend::parse(name) {
            Some(b) => b,
            None => {
                return err(format!(
                    "unknown backend {name:?} — use \"auto\", \"render_to_image\" or \
                     \"print_window\""
                ));
            }
        },
    };
    match screenshot::capture(&target, backend) {
        Err(e) => err(e),
        Ok(shot) => {
            let mut entries = vec![
                ("path", Json::s(shot.path.to_string_lossy().to_string())),
                ("width", Json::n(f64::from(shot.width))),
                ("height", Json::n(f64::from(shot.height))),
                ("hwnd", Json::s(format!("{:#x}", shot.hwnd))),
                ("backend", Json::s(shot.backend.to_string())),
                // bitmap_x = css_x * scale + clientX. `render_to_image` reads the client area
                // (offsets 0); `print_window` captures the whole window rect, border included.
                ("clientX", Json::n(f64::from(shot.client_origin.0))),
                ("clientY", Json::n(f64::from(shot.client_origin.1))),
                ("scale", Json::n(f64::from(shot.scale))),
            ];
            if let Some(why) = shot.fallback {
                entries.push((
                    "fallback",
                    Json::s(format!(
                        "render_to_image unavailable ({why}) — used PrintWindow"
                    )),
                ));
            }
            if shot.has_alpha {
                entries.push(("hasAlpha", Json::Bool(true)));
                entries.push((
                    "note",
                    Json::s(
                        "the capture has transparent pixels: this window uses a mica/acrylic \
                         backdrop, which DWM composites UNDER the app and render_to_image \
                         therefore cannot see — pass backend=\"print_window\" for the composed \
                         image",
                    ),
                ));
            }
            if shot.iconic {
                entries.push(("iconic", Json::Bool(true)));
                entries.push((
                    "warning",
                    Json::s(if shot.backend == "render_to_image" {
                        "the window is minimized — gpui keeps the render target, so this is a \
                         re-render of the last scene the app painted before it was minimized"
                    } else {
                        "the window is minimized — PrintWindow can only return whatever the \
                         compositor last held, which may be stale or blank"
                    }),
                ));
            }
            ok(Json::obj(entries))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::http_request_line;

    #[test]
    fn mouse_release_does_not_insert_an_unpressed_move_into_a_captured_drag() {
        use gpui::{MouseButton, PlatformInput};
        let at = gpui::point(gpui::px(120.0), gpui::px(-10.0));
        let extras = super::MouseExtras {
            held: false,
            clicks: 2,
            shift: false,
        };
        let events = super::mouse_inputs("up", MouseButton::Left, at, Default::default(), &extras);
        assert_eq!(events.len(), 2);
        let PlatformInput::MouseMove(before_up) = &events[0] else {
            panic!("move must precede release")
        };
        assert_eq!(before_up.pressed_button, Some(MouseButton::Left));
        assert_eq!(
            before_up.position, at,
            "outside-window coordinates are preserved"
        );
        let PlatformInput::MouseUp(up) = &events[1] else {
            panic!("the button must release next")
        };
        assert_eq!(up.button, MouseButton::Left);
        assert_eq!(up.click_count, 2);
        let events =
            super::mouse_inputs("move", MouseButton::Left, at, Default::default(), &extras);
        let PlatformInput::MouseMove(hover) = &events[0] else {
            unreachable!()
        };
        assert_eq!(
            hover.pressed_button, None,
            "an ordinary hover still reports no held button"
        );
        let held = super::MouseExtras {
            held: true,
            ..extras
        };
        let events = super::mouse_inputs("move", MouseButton::Left, at, Default::default(), &held);
        let PlatformInput::MouseMove(drag) = &events[0] else {
            unreachable!()
        };
        assert_eq!(drag.pressed_button, Some(MouseButton::Left));
    }

    #[test]
    fn target_discovery_does_not_block_or_duplicate_a_running_scan() {
        let discovery = super::TargetDiscovery::default();
        let owner = std::thread::current().id();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        assert!(discovery
            .poll(move || {
                assert_ne!(std::thread::current().id(), owner);
                started_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                "snapshot".into()
            })
            .unwrap()
            .is_none());
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(discovery
            .poll(|| panic!("a running scan must be shared"))
            .unwrap()
            .is_none());
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(result) = discovery
                .poll(|| panic!("consume the finished scan first"))
                .unwrap()
            {
                assert_eq!(result, "snapshot");
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn http_and_json_framings_are_told_apart_by_the_first_line() {
        assert_eq!(
            http_request_line("POST / HTTP/1.1\r\n"),
            Some(("POST".into(), "/".into()))
        );
        assert_eq!(
            http_request_line("OPTIONS /any/path HTTP/1.0\r\n"),
            Some(("OPTIONS".into(), "/any/path".into()))
        );
        assert_eq!(
            http_request_line("{\"token\":\"x\",\"cmd\":\"ping\"}"),
            None
        );
        assert_eq!(
            http_request_line("{\"cmd\":\"eval\",\"js\":\"'GET / HTTP/1.1'\"}"),
            None
        );
        assert_eq!(http_request_line(""), None);
        assert_eq!(http_request_line("POST /"), None);
        assert_eq!(http_request_line("post / HTTP/1.1"), None);
        assert_eq!(http_request_line("HTTP/1.1"), None);
    }
}
