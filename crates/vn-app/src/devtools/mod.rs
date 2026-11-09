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
//! the same `VN_DIST`-aware [`crate::slot`] helper the rest of the dev
//! transport uses: every parallel host is independently addressable through its
//! own dist slot, and a client that finds a stale file gets a connection error
//! rather than talking to somebody else's window.
//!
//! The same port also answers **HTTP/1.1** (`POST /` with the request object as
//! the body) — one command per connection, same token, same replies. That is
//! what makes the wire reachable from JS: `fetch()` is a first-class,
//! off-thread, promise-shaped client in this runtime, and a raw TCP socket is
//! not. It is what the devtools UI app (`apps/devtools`, docs/DEVTOOLS-UI.md)
//! is built on, and it costs one request-line sniff — see [`serve`].
//!
//! The whole module is compiled out under `--features prod`.

mod eval;
mod net; // the Network panel's feed (a JS-side fetch/XHR/WebSocket tee) and `net_detail`
mod picker;
mod allocations;
mod leases;
mod indexeddb;
#[cfg(windows)]
mod mem; // `mem`: where the process's memory is (m26 perf work)
#[cfg(not(windows))]
#[path = "mem_stub.rs"]
mod mem;
pub(crate) mod trace; // `trace`: a frame-by-frame record of where the time went (m26 perf work)
mod screenshot;
pub mod ws; // the PUSH channel: a WebSocket server on this same port

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

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
    generation: u64,
    req: Json,
    reply: Sender<String>,
}

static QUEUE: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
static TOKEN: Mutex<String> = Mutex::new(String::new());

// ---------------------------------------------------------------------------
// boot
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// the runtime switch
// ---------------------------------------------------------------------------
//
// Since m22 the devtools are compiled into EVERY build (docs/DEVTOOLS.md
// "Shape"), so "are they on" stopped being a compile-time question and became a
// runtime one the app itself answers. Three sources, in this order:
//
//   1. the default — ON. That is the owner's decision (docs/ARCHITECTURE.md
//      §11 decision 12) and it is what a browser does.
//   2. `vn.config.json`'s `"devtools": false` — start disabled. The static
//      spelling of `appDevtools.disable()`, applied before the first frame so
//      the port never exists at all rather than existing for a moment.
//   3. `appDevtools.enable()` / `.disable()` from the app's own JS, any time.
//
// …and one override on top of all three: `VN_DEVTOOLS=1` in the environment
// forces them ON and LOCKS them, for a developer debugging a build somebody
// shipped. See `resolve_initial` for the security posture that makes that
// override sound.
//
// DISABLED MEANS THE LISTENER IS GONE, not just that F12 is refused. The port
// carries `eval`; a released app whose author turned the devtools off must not
// still be holding a localhost socket that runs arbitrary JS in it. `enable()`
// binds a fresh port with a fresh token; `disable()` closes the listener,
// drops every push socket, blanks the token and deletes the handshake file.

/// Are the devtools on right now? Reads from any thread — the listener thread
/// never touches this, but the F12 interceptor and the host fns do.
static ENABLED: AtomicBool = AtomicBool::new(true);
/// `VN_DEVTOOLS=1` — the app may not turn them off.
static LOCKED: AtomicBool = AtomicBool::new(false);
/// The port the listener is on, or 0. Also how [`stop_listener`] wakes a
/// blocked `accept()`.
static PORT: AtomicU16 = AtomicU16::new(0);
/// Bumped by every stop. The accept loop compares it against the generation it
/// was started with and returns when they differ, which drops its listener.
static ACCEPT_GEN: AtomicU64 = AtomicU64::new(0);
/// Has the JS side (the console tee, the fetch/WebSocket tee) been installed?
/// They are installed on the first ENABLE rather than at boot, so an app that
/// starts disabled keeps the engine's own `console` and `fetch`.
static TEES_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Turns on the two things a devtools client needs to have been recording
/// BEFORE it attached. Called first thing in `main`, whatever the switch says.
///
/// * the log ring — an advisory that fires while the window config is being
///   read has to be in the ring by the time anybody asks for it;
/// * `vn_css::set_capture_rule_text` — every sheet keeps its declarations as
///   TEXT so `rules` can show a rule the way its author wrote it. The inspector
///   attaches to hosts that are ALREADY RUNNING, and "restart your app to see
///   where that value came from" is not a devtools feature.
///
/// Both are unconditional in a build that HAS the devtools, including one whose
/// app disables them: a disabled app can `enable()` a second later, and a rule
/// text that was never captured cannot be recovered. `--features no-devtools`
/// never reaches this line, so it never allocates one of those strings. The
/// cost of being wrong in the other direction is a `String` per declaration.
pub fn arm() {
    vn_tree::log::enable();
    vn_css::set_capture_rule_text(true);
    // …and every painted node gets a gpui hitbox, which is the only handle
    // `Hitbox::is_hovered_at` takes — the primitive that makes `pick` gpui's own
    // hit test instead of a rectangle walk of our own (`vn_tree::devtools::
    // hit_test_in`). Same reasoning as the line above: the inspector attaches to
    // a host that is already running, so it cannot be armed on demand. A
    // `--features no-devtools` build never reaches here, so it inserts nothing
    // and gpui's per-move hit test iterates exactly the hitboxes it always did
    // (its other elements get vn-tree's cheaper pick records instead).
    vn_tree::enable_pick_hitboxes();
}

/// Resolves the initial state and, if it is ON, starts the listener.
///
/// Called from `main` AFTER `window::load_config`, because the config is one of
/// the three inputs — and before the app bundle evaluates, so an app that reads
/// `appDevtools.isEnabled()` in `setup()` gets the settled answer.
pub fn boot(config_enabled: bool) {
    let (on, locked) = resolve_initial(config_enabled);
    LOCKED.store(locked, Ordering::SeqCst);
    ENABLED.store(on, Ordering::SeqCst);
    if on {
        start_listener();
    } else {
        let why = if config_enabled {
            "VN_DEVTOOLS=0"
        } else {
            "`\"devtools\": false` in vn.config.json"
        };
        let msg = format!(
            "[devtools] disabled at boot ({why}) — no port, no handshake, F12 declined. \
             `appDevtools.enable()` starts them."
        );
        eprintln!("{msg}");
        vn_tree::log::record("dev", msg);
    }
}

/// The environment override, and the reasoning that makes it sound.
///
/// `VN_DEVTOOLS=1` (`on`/`true`/`yes`) forces the devtools on and LOCKS them,
/// so `appDevtools.disable()` is refused. It exists for the one case the
/// runtime switch would otherwise make impossible: a developer debugging a
/// binary that somebody shipped with the devtools turned off in its own JS.
///
/// **Why that is not a hole.** The port is `127.0.0.1` only, and every request
/// carries a token that is written into `<dist>/devtools.json` with the
/// process's own file permissions. So reaching it already requires local code
/// execution as the same user — and somebody who can set this process's
/// environment has exactly that. An attacker who can set `VN_DEVTOOLS` can also
/// replace the executable. The override therefore hands out no authority its
/// user did not already hold, and it buys back the property the whole project
/// runs on: an app must not be able to make itself undebuggable.
///
/// `VN_DEVTOOLS=0` (`off`/`false`/`no`) is the mirror and does NOT lock: it
/// only sets the initial state, so an app can still turn them on. There is no
/// threat model that needs protecting from an app enabling its own devtools.
///
/// An empty value is not a value — `VN_DIST`-style harnesses pass `""` to mean
/// "unset", and a spec that wants the config's own answer needs to be able to.
fn resolve_initial(config_enabled: bool) -> (bool, bool) {
    match std::env::var("VN_DEVTOOLS").unwrap_or_default().to_ascii_lowercase().as_str() {
        "" => (config_enabled, false),
        "1" | "on" | "true" | "yes" => (true, true),
        "0" | "off" | "false" | "no" => (false, false),
        other => {
            eprintln!(
                "[devtools] VN_DEVTOOLS={other:?} is not one of 1/on/true/yes or 0/off/false/no \
                 — ignored"
            );
            (config_enabled, false)
        }
    }
}

/// Are the devtools on? The one question F12 and the host fns both ask.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Is the switch held on by `VN_DEVTOOLS=1`?
pub fn locked() -> bool {
    LOCKED.load(Ordering::SeqCst)
}

/// The port the listener is on, or `None`.
pub fn port() -> Option<u16> {
    match PORT.load(Ordering::SeqCst) {
        0 => None,
        p => Some(p),
    }
}

/// `appDevtools.enable()` / `.disable()`. `Err` names why it was refused.
///
/// Idempotent in both directions: enabling a listening host is a no-op that
/// keeps the port it has, so an app that calls `enable()` on every route change
/// does not hand its inspector a new port each time.
pub fn set_enabled(on: bool) -> Result<bool, String> {
    if !on && LOCKED.load(Ordering::SeqCst) {
        return Err(
            "VN_DEVTOOLS=1 is set in this process's environment, which holds the devtools on — \
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

/// What `appDevtools.info()` reports, and what the `state` half of every
/// enable/disable reply carries.
pub fn state_json() -> String {
    let inspectors = crate::inspector::children().len();
    Json::obj(vec![
        ("enabled", Json::Bool(enabled())),
        ("locked", Json::Bool(locked())),
        (
            "port",
            match port() {
                Some(p) => Json::n(f64::from(p)),
                None => Json::Null,
            },
        ),
        ("pid", Json::n(std::process::id() as f64)),
        ("key", Json::Bool(crate::window::devtools_key_enabled())),
        ("inspectors", Json::n(inspectors as f64)),
        ("slot", Json::s(crate::slot::dir().display().to_string())),
    ])
    .to_json()
}

/// Binds `127.0.0.1:0`, publishes `<dist>/devtools.json` and starts the
/// listener thread. Any failure is reported and then ignored: a host that
/// cannot open a devtools port must still run the app.
///
/// A FRESH port and a FRESH token every time, including on a re-`enable()`.
/// Reusing either would let a client that was told to go away at `disable()`
/// walk straight back in.
fn start_listener() {
    if PORT.load(Ordering::SeqCst) != 0 {
        return; // already listening
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
    // Overwrites whatever a previous (possibly dead) host left behind — the
    // file is a "who is live on this dist slot right now" record, not history.
    if let Err(e) = std::fs::write(&path, format!("{}\n", doc.to_json())) {
        eprintln!("[devtools] cannot write {}: {e} — devtools disabled", path.display());
        return;
    }
    PORT.store(port, Ordering::SeqCst);
    let generation = ACCEPT_GEN.load(Ordering::SeqCst);
    eprintln!(
        "[devtools] listening on 127.0.0.1:{port} — {} (pid {})",
        path.display(),
        std::process::id()
    );
    vn_tree::log::record("dev", format!("[devtools] listening on 127.0.0.1:{port}"));

    std::thread::Builder::new()
        .name("vn-devtools".into())
        .spawn(move || accept_loop(listener, generation))
        .ok();
}

/// Closes the listener, drops every push socket, blanks the token and deletes
/// the handshake. After this the port is not merely refusing commands — it is
/// not bound, so a connect is refused by the OS.
///
/// The accept loop is blocked inside `accept()`, which no flag can interrupt.
/// So the generation is bumped first and then we CONNECT TO OURSELVES: the loop
/// wakes with a connection, sees the generation move, returns, and dropping its
/// `TcpListener` closes the socket. One extra connection that is never served,
/// no timeout, no polling loop, and no second thread to own the listener.
fn stop_listener() {
    leases::clear();
    picker::stop();
    trace::cancel_recording();
    allocations::cancel_recording();
    let port = PORT.swap(0, Ordering::SeqCst);
    // Every in-flight and queued command is now unauthenticated: a blank token
    // never matches (`handle_request` refuses an empty one outright).
    TOKEN.lock().unwrap().clear();
    ws::close_all();
    let path = crate::slot::path("devtools.json");
    let _ = std::fs::remove_file(&path);
    // Whoever is blocked on one of these gets a disconnected channel and the
    // "the app closed the devtools" error, rather than the 60 s wedged-pump
    // timeout.
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
    vn_tree::log::record("dev", msg);
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

  // One argument, printed the way a console prints a value that is not part of
  // a format string.
  //
  // An Error is its header and its stack, as a browser console shows it, and
  // never `JSON.stringify`'s `{}`: an Error's fields are own NON-enumerable
  // properties, so `[error-boundary] captured render error: {}` was all the
  // ring ever said about the most important line in it (m26 client sweep
  // #28). JSC's `stack` is frames only, without V8's "Name: message" line, so
  // the header is written here. Inside an object or array it is the header
  // alone. A BigInt, which makes JSON.stringify throw, prints as `1n`.
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
      // A V8-shaped stack already starts with the header.
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

  // The WHATWG Console Standard's "Formatter" (§2.2). Without it a call like
  //
  //   console.log("%cinfo%c %s", "background:#0BC;\n  color:white", "", name)
  //
  // reached the ring as its own source text plus a multi-line CSS string —
  // which is what put `%c` on screen literally and, because the CSS argument
  // contains newlines, drew eight lines of text inside a one-line row.
  //
  // Substitutions: %s %d %i %f %o %O %j, %% for a literal percent, and %c,
  // which consumes its argument as CSS and produces no text at all. Leftover
  // arguments are appended space-separated, exactly as the spec says.
  var SPEC = /%([sdifoOjc%])/g;
  var format = function (args) {
    if (typeof args[0] !== "string" || args.length < 2 || !/%[sdifoOjc%]/.test(args[0])) {
      return { text: args.map(show).join(" "), runs: null };
    }
    var next = 1;
    var runs = [];
    var out = args[0].replace(SPEC, function (whole, kind) {
      if (kind === "%") return "%";
      if (next >= args.length) return whole; // no argument left: keep the text
      var arg = args[next++];
      switch (kind) {
        case "c":
          // A NUL placeholder, resolved in a second pass over the finished
          // string (where it is removed again): `replace` cannot know how much
          // text it has produced so far. NUL is safe as a marker because the
          // Console Standard's formatter never produces one, and a log line
          // that already contains one is broken by other means.
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
    // Place the `%c` runs: each NUL is where one started, and it is removed.
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
        __vn_devtools_console(name, f.text, f.runs ? JSON.stringify(f.runs) : "");
      } catch (e) {}
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
    // A third argument, present only when the call carried `%c`: the style
    // runs the tee computed, as JSON. Absent for every other line, which is
    // nearly all of them.
    let styles = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(2).0) }
        .filter(|s| !s.is_empty() && s != "undefined");
    vn_tree::log::record_styled(kind, text, styles);
    vn_runtime::redjs::api::Value::undefined().0
});


// `__vn_devtools_targets()` — which dev hosts are reachable right now, i.e.
// the inspector's target list.
//
// DISCOVERY ONLY: it reads the `devtools.json` handshakes on disk and probes
// each port, so a UI can offer "attach to…" without being told where to look.
// The protocol itself is not here — the inspector talks to the target with
// `fetch()` over the HTTP framing (see `serve`), which is the same wire, the
// same token and the same one-command-per-frame execution `ctl.ts` gets.
//
// It hands the caller each target's TOKEN, which is the whole point (a client
// cannot speak without one) and costs nothing extra: the JS asking already has
// `eval`-level trust over its own process, whatever build it is. Since m22 that
// includes a shipped one — which is exactly why an app that does not want any
// of this turns the devtools OFF (the listener goes with them) rather than
// relying on the build to have left them out.
vn_runtime::host_fn!(fn vn_devtools_targets(_g, _args) {
    let payload = targets_json();
    JsRuntime::with(|rt| rt.ctx().string(&payload).0)
});

// ---------------------------------------------------------------------------
// `appDevtools` — the tier-3 surface (docs/DEVTOOLS.md "Turning them off")
// ---------------------------------------------------------------------------
//
// Three host fns behind `import { appDevtools } from "vue-native"`, shaped like
// `appStorage`'s: one reader that answers everything at once, one verb, and one
// action. The JS half is `packages/vue-native/src/runtime.ts`.

// `__vn_devtools_state()` — `{enabled, locked, port, pid, key, inspectors, slot}`.
vn_runtime::host_fn!(fn vn_devtools_state(_g, _args) {
    let payload = state_json();
    JsRuntime::with(|rt| rt.ctx().string(&payload).0)
});

// `__vn_devtools_set_enabled(on)` — the state afterwards, or `{error}`.
//
// Returns the SETTLED state rather than a boolean, so a caller that was refused
// (`VN_DEVTOOLS=1`) can see what is actually true instead of inferring it from a
// `false` that also means "already off".
//
// A non-boolean argument reads as `false`, matching `__vn_set_listener`: the JS
// side is generated by our own module and always passes a real boolean, so a
// coercion here would only paper over a caller that had already gone wrong.
vn_runtime::host_fn!(fn vn_devtools_set_enabled(_g, args) {
    let on = args.arg(0).as_bool().unwrap_or(false);
    let payload = match set_enabled(on) {
        Ok(_) => {
            // The tees are JS wrappers, so they can only be installed while a
            // runtime exists — which is exactly now, since this IS JS calling.
            if on {
                install_tees_once();
            }
            state_json()
        }
        Err(e) => Json::obj(vec![("error", Json::s(e))]).to_json(),
    };
    JsRuntime::with(|rt| rt.ctx().string(&payload).0)
});

// `__vn_devtools_open()` — programmatic F12. `{opened}` or `{error}`.
//
// The same entry point the key uses (`inspector::open_for_window`), including
// its debounce and its "raise the one that is already open" behaviour, so an
// app's own "Open devtools" menu item and the key cannot disagree.
vn_runtime::host_fn!(fn vn_devtools_open(_g, _args) {
    let payload = if !enabled() {
        Json::obj(vec![(
            "error",
            Json::s(
                "the devtools are disabled in this process — call appDevtools.enable() first",
            ),
        )])
        .to_json()
    } else {
        let window = crate::main_window_id();
        let ran = vn_runtime::with_app(|cx| crate::inspector::open_for_window(window, cx));
        match ran {
            Some(()) => Json::obj(vec![
                ("opened", Json::Bool(true)),
                (
                    "window",
                    match window {
                        Some(w) => Json::n(w as f64),
                        None => Json::Null,
                    },
                ),
            ])
            .to_json(),
            None => Json::obj(vec![(
                "error",
                Json::s("appDevtools.open(): called outside a published phase"),
            )])
            .to_json(),
        }
    };
    JsRuntime::with(|rt| rt.ctx().string(&payload).0)
});

/// One handshake file, parsed and probed.
fn read_target(dir: &std::path::Path) -> Option<Json> {
    let text = std::fs::read_to_string(dir.join("devtools.json")).ok()?;
    let doc = json::parse(&text)?;
    let port = doc.get("port").and_then(Json::as_f64)? as u16;
    let token = doc.get("token").and_then(Json::as_str)?.to_string();
    let pid = doc.get("pid").and_then(Json::as_f64).unwrap_or(0.0);
    // "Alive" is answered by the port, not by the pid: a stale handshake from a
    // host that exited is exactly what this has to filter out, and a refused
    // connect on loopback is instant.
    let alive = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        std::time::Duration::from_millis(150),
    )
    .is_ok();
    Some(Json::obj(vec![
        ("slot", Json::s(dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().to_string()))),
        ("dir", Json::s(dir.display().to_string())),
        ("port", Json::n(f64::from(port))),
        ("pid", Json::n(pid)),
        ("token", Json::s(token)),
        ("alive", Json::Bool(alive)),
        ("self", Json::Bool(pid as u32 == std::process::id())),
    ]))
}

/// The dist slots to look in: everything beside our own dist (that is where
/// `build.ts <entry> <dist>` puts a parallel app), the e2e harness's slot pool,
/// every packaged app's instances (`%TEMP%\vue-native\apps\<name>\<pid>`, see
/// `crate::slot`), and whatever `VN_DEVTOOLS_TARGET` names explicitly.
fn scan_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    let own = crate::slot::path("devtools.json");
    if let Some(parent) = own.parent().and_then(|d| d.parent()) {
        roots.push(parent.to_path_buf());
        // …/packages/vue-native -> …/packages/vn-test/.vn-test/slots
        if let Some(packages) = parent.parent() {
            roots.push(packages.join("vn-test").join(".vn-test").join("slots"));
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
    let preferred = std::env::var("VN_DEVTOOLS_TARGET").unwrap_or_default();
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if !preferred.is_empty() {
        dirs.push(std::path::PathBuf::from(&preferred));
    }
    for root in scan_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("devtools.json").is_file() && !dirs.contains(&path) {
                dirs.push(path);
            }
        }
    }
    let targets: Vec<Json> = dirs.iter().filter_map(|d| read_target(d)).collect();
    Json::obj(vec![
        ("preferred", if preferred.is_empty() { Json::Null } else { Json::s(preferred) }),
        ("self", Json::obj(vec![
            ("pid", Json::n(std::process::id() as f64)),
            ("dir", Json::s(crate::slot::dir().display().to_string())),
        ])),
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
            .name("vn-devtools-discovery".into())
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
vn_runtime::raw_host_fn! { fn vn_trace_callback_begin(g, cf) {
    use vn_runtime::redjs::{api::{HostArgs, Value}, profiling};
    if !profiling::enabled() { return Value::int32(0).0; }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let args = HostArgs::new(cf);
        Value::number(profiling::external_callback_begin(g, args.arg(0).0) as f64).0
    })) {
        Ok(value) => value,
        Err(_) => { log::error!("Frame profiler callback begin failed"); Value::int32(0).0 }
    }
} }
vn_runtime::raw_host_fn! { fn vn_trace_callback_end(_g, cf) {
    use vn_runtime::redjs::{api::{HostArgs, Value}, profiling};
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let token = HostArgs::new(cf).arg(0).as_number().filter(|value| value.is_finite() && *value > 0. && value.fract() == 0.).unwrap_or(0.) as u64;
        profiling::external_callback_end(token);
    })).is_err() { log::error!("Frame profiler callback end failed"); }
    Value::undefined().0
} }

/// Registers the devtools host fns. Call before the app bundle boots.
///
/// Registration is unconditional — a host fn nobody calls costs one entry in a
/// table, and `appDevtools` has to be able to answer in a process whose
/// devtools are OFF (that is most of what it is for). The JS TEES are not: see
/// [`install_tees_once`].
pub fn install(rt: &JsRuntime) {
    rt.register("__vn_trace_callback_begin", 1, vn_trace_callback_begin as *const _);
    rt.register("__vn_trace_callback_end", 1, vn_trace_callback_end as *const _);
    rt.register("__vn_devtools_console", 3, vn_devtools_console as *const _);
    // The inspector app's only host dependency (docs/DEVTOOLS-UI.md §3): the
    // list of attachable instances. Registered in every host — the app that
    // never calls it never notices.
    rt.register("__vn_devtools_targets", 0, vn_devtools_targets as *const _);
    // The tier-3 `appDevtools` surface.
    rt.register("__vn_devtools_state", 0, vn_devtools_state as *const _);
    rt.register("__vn_devtools_set_enabled", 1, vn_devtools_set_enabled as *const _);
    rt.register("__vn_devtools_open", 0, vn_devtools_open as *const _);
    // The Network panel's feed. Same split: the host fn always, the wrapper
    // only when somebody is watching.
    net::install(rt);
    if enabled() {
        install_tees_once();
    }
}

/// Wraps `console.*`, `fetch` and `WebSocket` — once per process, on the first
/// moment the devtools are on.
///
/// Deferred rather than done at boot because a build now HAS the devtools even
/// when its app does not want them: a host that starts disabled must leave the
/// engine's own `console` and `fetch` untouched, so the only divergence
/// docs/DEVTOOLS.md admits to is one a watching developer asked for. An
/// `enable()` later installs them — the tees are additive wrappers, so they can
/// be put on at any time; they are never taken off again, because unwrapping a
/// function some other code has already captured is not something JS can do
/// honestly.
fn install_tees_once() {
    if TEES_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    JsRuntime::with(|rt| {
        if let Err(e) = rt.eval(CONSOLE_TEE_JS, "vn://devtools-console.js") {
            eprintln!("[devtools] console tee failed: {}\n{}", e.header, e.stack);
        }
        net::install_tee(rt);
    });
}

/// The host is about to run the app again in the same JS realm. Called from
/// `crate::reload::run` (`location.reload()`) and from the dev host's
/// `crate::dev::cold_restart` (vendor.js changed), between the old app's unload
/// or teardown and the new bundle. It puts a `kind: "reload"` marker in the
/// Network ring (`net::mark_reload`) with `cause` saying why. Nothing is
/// recorded while the tees are off: a ring that holds no requests needs no
/// divider.
pub fn mark_reload(cause: &str) {
    if TEES_INSTALLED.load(Ordering::SeqCst) {
        net::mark_reload(cause);
    }
}

/// A Network row for a load the HOST made: gpui's loaders (`<img>`, CSS
/// `url()`, `@font-face`, `<lottie>`) fetching through `crate::http_client`.
/// No script asked, so no tee saw it. Started when the request is, finished
/// on the network thread with the outcome (`net::record_loader`). Inert while
/// the devtools are off.
pub struct LoaderRow(Option<(String, String, f64, std::time::Instant)>);

impl LoaderRow {
    pub fn start(method: &str, url: &str) -> LoaderRow {
        if !enabled() {
            return LoaderRow(None);
        }
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        LoaderRow(Some((method.to_string(), url.to_string(), at, std::time::Instant::now())))
    }

    pub fn finish(
        self,
        result: &Result<vn_runtime::redjs::webapi::http::Response, vn_runtime::redjs::webapi::http::HttpError>,
    ) {
        if let Some((method, url, at, t0)) = self.0 {
            net::record_loader(&method, &url, at, t0.elapsed().as_secs_f64() * 1000.0, result);
        }
    }
}

/// `net_cache`: the HTTP cache's counters and folder, or `op: "clear"` to
/// empty it (docs/DEVTOOLS.md). `null` stats: no cache is installed.
fn cmd_net_cache(op: Option<&str>) -> String {
    use vn_runtime::redjs::webapi::http::cache;
    match op.unwrap_or("stats") {
        // After the writes already queued (≤ 2 s): what is on disk, not what
        // is about to be.
        "stats" => {
            cache::flush();
            ok(net::cache_json())
        }
        "clear" => match cache::clear() {
            Ok(freed) => ok(Json::obj(vec![("freed", Json::n(freed as f64)), ("now", net::cache_json())])),
            Err(e) => err(format!("net_cache clear: {e}")),
        },
        other => err(format!("net_cache: unknown op {other:?} — \"stats\" (default) or \"clear\"")),
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

/// One accepted connection per iteration, until the generation it was started
/// with is superseded by a [`stop_listener`]. Returning drops `listener`, which
/// is what actually closes the socket.
fn accept_loop(listener: TcpListener, generation: u64) {
    loop {
        let accepted = listener.accept();
        // Checked BEFORE the connection is served, so the wake-up connect that
        // `stop_listener` makes is never handed to `serve`.
        if ACCEPT_GEN.load(Ordering::SeqCst) != generation {
            return;
        }
        match accepted {
            Ok((s, _)) => {
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

/// One connection, in whichever of the two framings the client opened with.
///
/// * **newline-delimited JSON** — the original wire (`ctl.ts`, `mcp.ts`,
///   `vn-test`). A client may pipeline several commands down one socket; each
///   still costs one frame, in order.
/// * **HTTP/1.1** — one `POST /` per connection, the request object as the
///   body. Same token, same command, same reply object, `Connection: close`
///   (redjs' pooled `fetch` honours it and never reuses the socket).
///
/// The two are told apart by the first line and nothing else: a JSON request
/// starts with `{`, an HTTP request line ends with ` HTTP/1.x`. Neither can be
/// mistaken for the other, so no probing, no magic byte, no second port.
fn serve(stream: TcpStream) {
    let Ok(write_half) = stream.try_clone() else { return };
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

/// One request line of JSON → one reply line of JSON. The token check lives
/// here so both framings are guarded by exactly the same code.
fn handle_request(line: &str) -> String {
    match json::parse(line) {
        None => err("request is not valid JSON"),
        Some(req) => {
            let generation = ACCEPT_GEN.load(Ordering::SeqCst);
            // A BLANK token never matches, so a request that arrives on a
            // socket accepted moments before `disable()` is refused rather than
            // let through by an empty-equals-empty comparison.
            let want = TOKEN.lock().unwrap().clone();
            let ok_token = !want.is_empty()
                && req.get("token").and_then(Json::as_str).is_some_and(|t| t == want);
            if !ok_token {
                err("bad or missing token — read it from <dist>/devtools.json")
            } else {
                // Read-only database inspection and allocation aggregation own no
                // JSC/Window values. Keep them off the target's UI frame pump.
                match req.get("cmd").and_then(Json::as_str) {
                    Some("indexeddb") => indexeddb::command(&req).map_or_else(err, ok),
                    Some("allocations") => {
                        let result = allocations::command_authenticated(&req, generation);
                        if result.is_ok() && req.get("op").and_then(Json::as_str) == Some("start") {
                            if let Some(ms) = req.get("leaseMs").and_then(Json::as_f64) { leases::arm(ms as u64); }
                        }
                        result.map_or_else(err, ok)
                    },
                    Some("capture-heartbeat") => {
                        leases::refresh();
                        ok(Json::obj(vec![("renewed", Json::Bool(true))]))
                    },
                    _ => submit(req, generation),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP framing
// ---------------------------------------------------------------------------

/// Bodies larger than this are refused before anything is read: the only client
/// is a devtools UI sending a command object, and an unbounded read from a
/// socket is how a listener thread becomes a memory leak.
const MAX_HTTP_BODY: usize = 4 << 20;

/// `POST / HTTP/1.1` → `("POST", "/")`. `None` for anything that is not an
/// HTTP request line, which is how [`serve`] recognises the other framing.
fn http_request_line(line: &str) -> Option<(String, String)> {
    let line = line.trim_end();
    let rest = line.strip_suffix(" HTTP/1.1").or_else(|| line.strip_suffix(" HTTP/1.0"))?;
    let (method, target) = rest.split_once(' ')?;
    if !method.chars().all(|c| c.is_ascii_uppercase()) || method.is_empty() {
        return None;
    }
    Some((method.to_string(), target.to_string()))
}

fn serve_http(request_line: &str, mut reader: BufReader<TcpStream>, mut out: TcpStream) {
    let Some((method, target)) = http_request_line(request_line) else { return };
    // Headers, once, by name. `Content-Length` is load-bearing for a POST and
    // the `Sec-WebSocket-*` set is load-bearing for an upgrade; `Transfer-
    // Encoding: chunked` is refused by name rather than mis-framed silently (no
    // client this exists for sends it — redjs' fetch always sets a length).
    //
    // Bounded, so a client that never sends the blank line cannot keep this
    // thread reading forever. 200 is far past what any real client sends.
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
        let Some((name, value)) = header.split_once(':') else { continue };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let head = |name: &str| {
        headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or("")
    };
    let length: Option<usize> = head("content-length").parse::<usize>().ok();
    let chunked = head("transfer-encoding").eq_ignore_ascii_case("chunked");

    // The THIRD framing on this port (the push channel, `ws.rs`): an RFC 6455
    // opening handshake IS an ordinary HTTP GET, so it arrives here and is
    // recognised by its `Upgrade` header — no second port, no second listener,
    // no second handshake field in `devtools.json`.
    if method == "GET"
        && head("upgrade").eq_ignore_ascii_case("websocket")
        && head("connection").to_ascii_lowercase().contains("upgrade")
    {
        ws::serve(&target, &headers, out, reader);
        return;
    }

    let mut reply = |status: &str, body: String| http_reply(&mut out, status, &body);
    match method.as_str() {
        // A browser-hosted client (the same UI served from a static file, or a
        // future one) sends this before every POST. Answering it is four lines
        // and is the difference between "reachable from a browser" and not.
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
                "{method} {target} — this port speaks two things over HTTP: POST / with the                  command object as the body, and GET / with `Upgrade: websocket` for the push                  channel (see docs/DEVTOOLS.md)"
            )),
        ),
    }
}

/// Writes one HTTP response and lets the caller drop the socket.
///
/// `Access-Control-Allow-*` is here so the same protocol answers a browser
/// page as readily as it answers the app: the token is what guards the port,
/// and an origin check would only guard it against the one client that cannot
/// forge an origin anyway.
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

/// Hands the request to the main thread and waits for the reply.
fn submit(req: Json, generation: u64) -> String {
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    {
        let mut queue = QUEUE.lock().unwrap();
        if !enabled() || generation != ACCEPT_GEN.load(Ordering::SeqCst) {
            return err("the app revoked this devtools session");
        }
        queue.push_back(Job { generation, req, reply: tx });
    }
    vn_runtime::frames::wake();
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(reply) => reply,
        // Disconnected rather than timed out: `stop_listener` cleared the
        // queue, which is what `appDevtools.disable()` does. Told apart because
        // "the app turned me off" and "the app is wedged" are different
        // diagnoses and a client should not have to guess.
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
    generation: u64,
    node: u32,
    text: Option<String>,
    keys: Vec<String>,
    reply: Sender<String>,
    waited: u32,
    /// The `window` argument the command carried. A parked command resumes a
    /// frame later, by which time `TARGET_WINDOW` has been reset for the next
    /// command — so the window rides along on the parked job instead
    /// (docs/MULTIWINDOW.md §7.5). `None` = the primary window.
    window: Option<u64>,
}

/// Frames a pending command will wait for focus before giving up. ~0.5 s at
/// 60 fps; a repaint normally lands on the very next frame.
const PENDING_MAX_FRAMES: u32 = 30;

/// Called once per frame from `pump_frame`, INSIDE its
/// `with_published(Phase::Commit)` — so `with_commit` below always resolves and
/// every command observes the same committed tree the JS pump just produced.
/// A command is queued or parked: the frame clock keeps the display rate
/// (src/idle.rs).
pub fn busy() -> bool {
    PENDING.with(|p| p.borrow().is_some()) || !QUEUE.lock().unwrap().is_empty()
}

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
    let Some(job) = QUEUE.lock().unwrap().pop_front() else { return };
    let Job { generation, req, reply } = job;
    if !enabled() || generation != ACCEPT_GEN.load(Ordering::SeqCst) {
        let _ = reply.send(err("the app revoked this devtools session"));
        return;
    }
    let response = run(&req, &reply);
    // The window a command addressed is scoped to that command and nothing
    // else: a parked `type` resumes against the published window, as it always
    // has, and the next command starts from the default again.
    TARGET_WINDOW.with(|c| c.set(None));
    match response {
        Outcome::Done(text) => {
            let _ = reply.send(text);
        }
        Outcome::Parked(mut pending) => {
            pending.generation = generation;
            PENDING.with(|p| *p.borrow_mut() = Some(pending));
        },
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
    // Re-publish the window the command addressed for the duration of the
    // resume: "is it focused" and "deliver these keystrokes" are both questions
    // about ONE window's focus path.
    let _scope = TargetScope::new(p.window);
    let focused = with_node_window(p.node, |window, cx| {
        vn_tree::devtools::is_focused(p.node, window, cx)
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
    // Which of THIS process's windows the command is about (docs/MULTIWINDOW.md).
    // Absent = the primary window, which is what every pre-v2 client sends and
    // what every single-window app means.
    let window_arg = req.get("window").and_then(Json::as_f64).map(|w| w as u64);
    TARGET_WINDOW.with(|c| c.set(window_arg));
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
        "tree" => cmd_tree(
            arg("selector").as_deref(),
            num("depth"),
            // With a `window` and nothing else, the tree is rooted at what
            // THAT window paints — the whole document for the primary one,
            // the popped-out subtree for a pop-out. A client that names a
            // node still gets that node.
            num("node").or_else(|| {
                window_arg.map(|id| f64::from(window_root(id))).filter(|r| *r > 0.0)
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
            // Held keys: a ctrl+wheel is a zoom in most apps, not a scroll.
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
        "eval" => match arg("js") {
            None => err("eval needs `js`"),
            Some(js) => cmd_eval(&js),
        },
        "logs" => cmd_logs(num("since")),
        "net" => cmd_net(num("since")),
        // `net-detail` is accepted too: it is how the command was first named,
        // and a hyphen is what a CLI verb usually looks like. The table name is
        // `net_detail` because the MCP tool is derived from it (`vn_net_detail`).
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
        "trace" if matches!(arg("op").as_deref(), Some("recording-start" | "recording-stop" | "recording-status" | "frames" | "frame")) => {
            let result = match arg("op").as_deref() {
                Some("recording-start") => {
                    #[cfg(target_os = "windows")]
                    {
                        let native = with_target(|window, _| crate::window::capture_hwnd(window))
                            .flatten().unwrap_or(0);
                        trace::set_recording_native_window(native as usize);
                    }
                    let result = trace::recording_start(num("cap").map(|n| n.max(0.0) as usize));
                    if result.is_ok() {
                        if let Some(ms) = num("leaseMs") { leases::arm(ms as u64); }
                        if let Some(key) = with_target(|window, _| vn_tree::windows::key(window)) {
                            trace::set_recording_window_id(key);
                        }
                    }
                    result
                },
                Some("recording-stop") => trace::recording_stop(),
                Some("recording-status") => trace::recording_status(),
                Some("frames") => trace::frame_page(num("offset").unwrap_or(0.0).max(0.0) as usize, num("limit").unwrap_or(240.0).clamp(1.0, 240.0) as usize),
                Some("frame") => match num("frameId") {
                    Some(id) if id.is_finite() && id >= 0.0 && id.fract() == 0.0 => trace::frame_detail(id as u64, num("offset").unwrap_or(0.0).max(0.0) as usize, num("limit").unwrap_or(1000.0).clamp(1.0, 1000.0) as usize),
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
            // A button by label, or by index.
            arg("button").or_else(|| num("button").map(|n| (n as i64).to_string())),
        ),
        "permissions" => cmd_permissions(arg("op").as_deref(), arg("name").as_deref()),
        "type" | "key" => return start_input_command(cmd, req, reply),
        "" => err("no `cmd` in the request"),
        other => err(format!("unknown command `{other}` — see docs/DEVTOOLS.md")),
    };
    Outcome::Done(text)
}

// ---------------------------------------------------------------------------
// which window a command is about
// ---------------------------------------------------------------------------

thread_local! {
    /// The `window` argument of the command currently executing. `None` = the
    /// primary window, which is what a single-window app always means and what
    /// every client written before v2 sends.
    static TARGET_WINDOW: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` against the window that PAINTS `node` — unless the command named
/// a window itself, in which case that one wins.
///
/// This is what makes `ctl.ts type --selector "#name" --text hi` work on an
/// `<input>` living in a pop-out without the caller having to know which window
/// that is. The node knows: it hangs under exactly one painted root, and each
/// painted root belongs to exactly one window (docs/MULTIWINDOW.md §5.6). A
/// command that DOES name a window keeps naming it, because "drive window 3"
/// is a different question from "drive this element".
fn with_node_window<R>(node: u32, f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    if TARGET_WINDOW.with(|c| c.get()).is_some() {
        return with_target(f);
    }
    let painted_by = vn_tree::windows::window_of_node(node);
    if crate::main_window_id() == Some(painted_by) {
        return with_commit(f);
    }
    with_commit(|_, cx| crate::popout::with_window(painted_by, cx, f)).flatten()
}

/// Publishes a `window` argument for the duration of a scope, restoring
/// whatever was there before. Used by a PARKED command's resume, which runs a
/// frame after the argument that produced it was cleared.
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

/// Runs `f` against the window the current command addressed.
///
/// This is the seam multi-window devtools needed and nothing more: the pump
/// publishes the PRIMARY window, so `with_commit` has always meant "the main
/// one", and a pop-out was simply unreachable. Addressing another window is an
/// `AnyWindowHandle::update` — untyped, for the reason `popout::with_window`
/// spells out — which is exactly what `popout::notify_all` already does from
/// inside this same pump every frame.
pub(crate) fn with_target<R>(f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    match TARGET_WINDOW.with(|c| c.get()) {
        None => with_commit(f),
        Some(id) => with_window_id(id, f),
    }
}

/// Runs `f` against one named window of this process.
fn with_window_id<R>(id: u64, f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    if crate::main_window_id() == Some(id) {
        return with_commit(f);
    }
    with_commit(|_, cx| crate::popout::with_window(id, cx, f)).flatten()
}

/// The node a window paints as its root — `0` for the primary window, the
/// popped-out node for a pop-out. It is the key every per-window bounds map is
/// stored under (`vn_tree::bounds_in`).
fn window_root(id: u64) -> u32 {
    if crate::main_window_id() == Some(id) {
        return 0;
    }
    crate::popout::list().into_iter().find(|(w, _, _)| *w == id).map_or(0, |(_, node, _)| node)
}

/// Every window of this process, primary first.
fn window_ids() -> Vec<u64> {
    let mut ids: Vec<u64> = crate::main_window_id().into_iter().collect();
    ids.extend(crate::popout::list().into_iter().map(|(id, _, _)| id));
    ids
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
    // Only when true: an element that carries gpui's `.group(name)` also
    // carries a hitbox it would not otherwise have, which is worth seeing.
    if n.group_anchor {
        entries.push(("groupAnchor", Json::Bool(true)));
    }
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

fn cmd_tree(selector: Option<&str>, depth: Option<f64>, node: Option<f64>) -> String {
    // A `node` id as well as a selector: an inspector expanding a subtree
    // already has the id it walked to, and no selector is guaranteed to name it
    // again (two identical siblings have the same one).
    let root = match (selector, node) {
        (None, None) => vn_tree::devtools::root_id(),
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

fn cmd_styles(selector: Option<&str>, index: Option<f64>, node: Option<f64>) -> String {
    match resolve_target(selector, index, node) {
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
                // The pointer state the `live` bucket was folded against —
                // reported separately because "is it hovered" is a question an
                // assertion asks on its own, and reading it out of a style dump
                // would mean guessing which property the hover rule moved.
                ("hovered", Json::Bool(vn_tree::devtools::is_hovered(id))),
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

// ---------------------------------------------------------------------------
// rule provenance
// ---------------------------------------------------------------------------

/// Which RULES produced the values `styles` reports, in cascade order.
///
/// A browser's styles panel is a list of rule blocks, not a fold — you fix CSS
/// by finding the selector that won, not by reading the winning value. This is
/// that list: selector, `@media` prelude, specificity, source order, the state
/// bucket the rule folds into, and its declarations as the author wrote them.
///
/// Plus the thing a browser has no equivalent for: each rule carries the
/// ADVISORIES vn-css produced for its declarations, so "why is this property
/// doing nothing" is answered in the panel instead of in the log.
fn cmd_rules(selector: Option<&str>, index: Option<f64>, node: Option<f64>) -> String {
    let id = match resolve_target(selector, index, node) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let Some(rules) = vn_tree::devtools::matched_rules(id) else {
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
    // An empty declaration list on every rule means the sheet was compiled
    // before rule-text capture was on (a host started by something that is not
    // this one). Say so rather than let a panel render blank blocks.
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

// ---------------------------------------------------------------------------
// highlight + picker
// ---------------------------------------------------------------------------

struct Highlight {
    node: u32,
    /// Which window paints the outline. `None` = the primary one, which is
    /// what a single-window app means and what every pre-v2 client sends.
    window: Option<u64>,
    /// When the overlay stops painting itself. `None` = until something clears
    /// it. A client that polls (the picker) sets a short one so a crashed
    /// inspector cannot leave a permanent rectangle on the app.
    until: Option<std::time::Instant>,
    label: Option<String>,
}

static HIGHLIGHT: Mutex<Option<Highlight>> = Mutex::new(None);

/// The outline overlay for the highlighted node, or `None`.
///
/// Called by the shell's `render`, so the rectangle is re-read from the bounds
/// map every frame and follows the element as it moves. It is a plain `div`
/// with no interactivity: gpui only builds hitboxes for elements that ask for
/// one, so the overlay cannot intercept a click, a hover or a drag region.
pub fn highlight_overlay() -> Option<gpui::Div> {
    overlay_for(crate::main_window_id(), 0)
}

/// The same outline, for a POP-OUT window (`docs/MULTIWINDOW.md`).
///
/// A pop-out paints a different node of the same tree into a different client
/// space, so the rectangle has to come out of THAT window's bounds map
/// (`root` is the popped-out node, which is the key those maps are under) and
/// the outline has to be drawn by that window's own root view. Painting it in
/// the primary window would put a blue box at the pop-out's coordinates over
/// whatever the main window happens to have there.
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
    // Every window renders this; only the one that was asked draws it. An
    // absent `window` means the primary one.
    let wanted = h.window.or_else(crate::main_window_id);
    if wanted != window {
        return None;
    }
    // `overlay_bounds`, not `node_info().bounds`: this runs during the shell's
    // own render, where `build_root` has already cleared the live bounds map
    // and prepaint has not refilled it yet — the live map is empty for every
    // node at exactly this moment.
    let (x, y, w, height) = vn_tree::painted_bounds_in(root, h.node)?;
    let label = h.label.clone().unwrap_or_else(|| describe(h.node, w, height));
    Some(
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(height))
            // Chrome's own inspect colours: a blue fill under a blue border is
            // legible over both light and dark app content.
            .bg(rgba(0x6cb6ff30))
            .border_1()
            .border_color(rgba(0x6cb6ffcc))
            .child(
                div()
                    .absolute()
                    // Above the box, unless the box is at the very top of the
                    // window — then inside it, so the chip is never clipped off.
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
                    // The chip is as wide as its text: an absolutely positioned
                    // box has no width of its own, and a wrapped `div.card ·
                    // 320×48` over a narrow element reads as garbage.
                    .whitespace_nowrap()
                    .child(label),
            ),
    )
}

/// `div.card · 320×48` — what the chip over a highlighted node says.
fn describe(id: u32, w: f32, h: f32) -> String {
    let head = match vn_tree::devtools::node_info(id) {
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

/// A selector, an explicit node id, or (for `highlight`) neither.
fn resolve_target(selector: Option<&str>, index: Option<f64>, node: Option<f64>) -> Result<u32, String> {
    match (selector, node) {
        (Some(sel), _) => resolve_one(sel, index),
        (None, Some(id)) => {
            let id = id as u32;
            match vn_tree::devtools::node_info(id) {
                Some(_) => Ok(id),
                None => Err(format!("no node {id} in the tree")),
            }
        }
        (None, None) => Err("needs a `selector` or a `node` id".into()),
    }
}

/// Draws (or clears) the inspector's outline over one node in the TARGET app.
///
/// A node id is accepted as well as a selector because the inspector already
/// has ids — it walked the tree — and re-deriving a unique selector for an
/// arbitrary node is a problem nobody needs to solve to draw a rectangle.
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
    let ttl = ttl_ms.filter(|ms| *ms > 0.0).map(|ms| {
        std::time::Instant::now() + std::time::Duration::from_millis(ms as u64)
    });
    *HIGHLIGHT.lock().unwrap() = Some(Highlight { node: id, window, until: ttl, label });
    // The window that will PAINT it, not the published one — a pop-out never
    // repaints off the primary window's refresh.
    let _ = with_target(|window, _| window.refresh());
    let bounds = vn_tree::devtools::node_info(id).and_then(|n| n.bounds);
    ok(Json::obj(vec![
        ("id", Json::n(f64::from(id))),
        ("bounds", bounds_json(bounds)),
        // A node that is not painted has nowhere to draw an outline. That is
        // not an error (a `<KeepAlive>`d page is a legitimate thing to select),
        // but a silent no-op would look like a broken highlight.
        ("painted", Json::Bool(bounds.is_some())),
    ]))
}

/// Where the pointer is over the TARGET, and which node is under it.
///
/// The picker is a POLL, not a mode: the host keeps no state, so an inspector
/// that dies mid-pick leaves nothing behind. `insideWindow` is gpui's own
/// window-level hover flag — it is what tells the inspector the pointer has
/// left for the devtools window, which is the gesture that latches a pick.
///
/// ## Which window
///
/// With `window`, that one. WITHOUT it, every window this process paints is
/// asked and **the one the pointer is actually over wins** — because "which
/// window is the pointer in" is the question a picker has to answer first, and
/// a pop-out is a window the primary one knows nothing about. The reply always
/// names the window it answered for, so a client is never guessing.
///
/// The hit test is then GPUI'S OWN, run against that window's rendered frame
/// (`vn_tree::hit_test_in` → `Hitbox::is_hovered_at`), which is why it has to
/// be asked from INSIDE the window: a hitbox belongs to the frame that inserted
/// it, so a point without a window would be meaningless (docs/MULTIWINDOW.md
/// §5.6). Transforms, deferred paint order and content masks therefore all get
/// their vote, which the bounds-rectangle walk this replaced could not give
/// them.
fn cmd_pick(window_arg: Option<u64>) -> String {
    let candidates: Vec<u64> = match window_arg {
        Some(id) => vec![id],
        None => window_ids(),
    };
    if candidates.is_empty() {
        return err("this process has no window to pick in");
    }
    // (id, x, y, insideWindow, viewport w, viewport h, path)
    type Facts = (u64, f32, f32, bool, f32, f32, Vec<u32>);
    let mut chosen: Option<Facts> = None;
    for id in candidates {
        let root = window_root(id);
        // The hit test runs INSIDE this closure because it needs the window it
        // is about: `Hitbox::is_hovered_at` reads that window's rendered frame.
        let Some(facts) = with_window_id(id, |window, _| {
            let p = window.mouse_position();
            let size = window.viewport_size();
            (
                f32::from(p.x),
                f32::from(p.y),
                window.is_window_hovered(),
                f32::from(size.width),
                f32::from(size.height),
                vn_tree::devtools::hit_test_in(root, f32::from(p.x), f32::from(p.y), window),
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
        ("viewport", Json::obj(vec![("w", Json::n(vw)), ("h", Json::n(vh))])),
        ("path", Json::arr(path.iter().map(|&id| Json::n(f64::from(id))).collect())),
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
    let Some(info) = vn_tree::devtools::node_info(id) else {
        return err(format!("node {id} vanished between the query and the click"));
    };
    let Some((x, y, w, h)) = info.bounds else {
        return err(format!(
            "node {id} has no bounds — it was not painted in the last frame (detached, or parked \
             in a <KeepAlive> cache), so there is no point to click"
        ));
    };
    // The COORDINATE this reports is only informational — `click` dispatches by
    // node id, not by hit test — but it must be the same point `mouse` would
    // aim at, or the two commands describe different places for one gesture.
    // `aim_point` maps the box's centre through the element's own transform and
    // is asked of the window that PAINTS the node, because that is the frame
    // whose hitboxes answer; the raw centre is the fallback for a node gpui
    // would NOT hit, which `click` is still allowed to reach — proving the
    // handler chain is its job, and `mouse` is the one that proves reachability.
    let (cx_, cy) = with_node_window(id, |window, _| vn_tree::devtools::aim_point(id, window))
        .flatten()
        .unwrap_or((x + w / 2.0, y + h / 2.0));
    // `with_node_window`, not `with_commit`: a click has to be dispatched into
    // the window that PAINTS the node, because the listener chain gpui runs it
    // through belongs to that window's element tree (docs/MULTIWINDOW.md §7.5).
    // Absent a `window` argument this is the primary window, which is what
    // every single-window app means.
    let proceed = with_node_window(id, |window, app| {
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
    let out = with_node_window(id, |window, app| {
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
    // Focus is PER WINDOW (docs/MULTIWINDOW.md §5.1): `window.focus(&handle)`
    // puts the handle on THAT window's focus path, and gpui installs the native
    // input handler for the window that painted the element. Focusing a
    // pop-out's `<input>` through the primary window would put the handle on a
    // path the keystrokes never travel.
    match with_node_window(id, |window, app| vn_tree::devtools::focus(id, window, app)) {
        None => err("not in a commit phase"),
        Some(Err(e)) => err(e),
        Some(Ok(())) => ok(Json::obj(vec![("id", Json::n(f64::from(id)))])),
    }
}

/// A text field's IME, driven the way the platform drives it: `text` is the
/// composition's new pre-edit, or with `commit` its committed text. The field
/// need not be focused. What the composition events, `beforeinput` and `input`
/// report is the widget's own path; only the IME is stood in for.
fn cmd_ime(selector: &str, index: Option<f64>, text: &str, commit: bool) -> String {
    let id = match resolve_one(selector, index) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let out = with_node_window(id, |window, app| {
        let done = vn_tree::devtools::ime(id, text, commit, window, app);
        // `input` and `compositionend` go out on a `Window::defer`; flushing
        // here is what makes the next command see the tree they leave.
        if vn_tree::flush_changes() {
            window.refresh();
        }
        done.map(|()| vn_tree::devtools::input_value(id, app))
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

/// A REAL pointer event, through gpui's own hit test.
///
/// This is the other half of [`cmd_click`], and the difference is the whole
/// point of having both. `click` synthesizes into the app's JS dispatcher from
/// a node id: it proves the handler chain, and it can never prove that the
/// element is reachable. `mouse` hands gpui a `PlatformInput` at a window
/// coordinate, so the hit test, the content masks, the deferred paint order,
/// `pointer-events: none` and the hover ladder all decide what happens — which
/// is exactly the set of things that was wrong when the meet app's two
/// overlapping `Your name` fields fought over one click.
///
/// It is also the only way to exercise `:hover` from a test: a synthetic
/// `WM_MOUSEMOVE` posted to the window works too (research/reports/m18/
/// live-test-round2.md §7 re-tested m11's claim that it does not), but it needs
/// a window handle and screen coordinates, and this needs neither.
///
/// No cursor moves and no window is activated: gpui's `mouse_position` is
/// window state, not the platform's.
/// What a `mouse` gesture carries besides where and which button: a move with
/// the button HELD (a drag, which a text selection follows), the click count
/// of a press (2 and 3 select a word and a paragraph) and Shift.
struct MouseExtras {
    held: bool,
    clicks: usize,
    shift: bool,
}

/// The ordered physical input stream for one devtools mouse command. The
/// hit-test move before a release still has its button held; MouseUp changes
/// that state. An unpressed move first would falsely cancel a captured drag.
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
            button, position, modifiers, click_count: extras.clicks, first_mouse: false,
        }));
    }
    if matches!(kind, "up" | "click") {
        events.push(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
            button, position, modifiers, click_count: extras.clicks,
        }));
    }
    events
}

fn cmd_mouse(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    kind: Option<&str>,
    button: Option<f64>,
    extras: MouseExtras,
) -> String {
    // Which window the gesture is about, decided ONCE and used for both the aim
    // and the dispatch — they must be the same window or the coordinate and the
    // frame it is tested against come from two different client spaces. A
    // selector names its own window (the one that PAINTS the node); a bare x/y
    // is a coordinate in the window the command named, primary by default.
    fn in_window<R>(node: Option<u32>, f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
        match node {
            Some(id) => with_node_window(id, f),
            None => with_target(f),
        }
    }
    let node = match selector {
        Some(sel) => match resolve_one(sel, index) {
            Ok(id) => Some(id),
            Err(e) => return err(e),
        },
        None => None,
    };
    let point = match (node, x, y) {
        (Some(id), _, _) => {
            // `aim_point`, not the centre of `bounds`: a transformed element's
            // recorded rectangle is the UNTRANSFORMED one (that is the frame
            // layout and `getBoundingClientRect` think in), so its centre is
            // not where the element is on screen. gpui's own matrix maps it
            // there, and gpui's own hit test then confirms the point really
            // lands on the node — so an unreachable control is REPORTED
            // instead of the gesture silently going to whatever covers it.
            match in_window(Some(id), |window, _| vn_tree::devtools::aim_point(id, window)) {
                None => return err("not in a commit phase"),
                Some(Some(p)) => p,
                Some(None) => {
                    return err(format!(
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
        _ => return err("mouse needs either a `selector` or both `x` and `y` (window coordinates)"),
    };
    let kind = kind.unwrap_or("move");
    if !matches!(kind, "move" | "down" | "up" | "click") {
        return err(format!("unknown mouse kind `{kind}` — move | down | up | click"));
    }
    let button = match button.unwrap_or(0.0) as i64 {
        0 => gpui::MouseButton::Left,
        1 => gpui::MouseButton::Middle,
        2 => gpui::MouseButton::Right,
        other => return err(format!("unknown mouse button {other} — 0 left, 1 middle, 2 right")),
    };
    let position = gpui::Point::new(gpui::px(point.0), gpui::px(point.1));
    let modifiers = gpui::Modifiers { shift: extras.shift, ..Default::default() };
    // Not `with_commit`: a window coordinate is meaningless without the window
    // it is in, and a pop-out paints a different subtree in a different client
    // space (docs/MULTIWINDOW.md). This is what makes the spike's
    // `__vn_window_click` unnecessary — `mouse` addresses any window this
    // process paints, through the same real gpui hit test.
    let done = in_window(node, |window, app| {
        for input in mouse_inputs(kind, button, position, modifiers, &extras) {
            window.dispatch_event(input, app);
        }
        // The hover ladder is read at PAINT time, so the frame that shows the
        // new state has to be asked for explicitly.
        window.refresh();
        vn_tree::devtools::focused_node(window, app)
    });
    match done {
        None => err("not in a commit phase"),
        Some(focused) => ok(Json::obj(vec![
            ("x", Json::n(f64::from(point.0))),
            ("y", Json::n(f64::from(point.1))),
            ("kind", Json::s(kind.to_string())),
            // Who owns focus AFTER the gesture — the direct answer to "did the
            // press land on the field I aimed at", and the one thing JS cannot
            // report while the window is inactive.
            match focused {
                Some(id) => ("focused", Json::n(f64::from(id))),
                None => ("focused", Json::Null),
            },
        ])),
    }
}

// ---------------------------------------------------------------------------
// eval
// ---------------------------------------------------------------------------

fn cmd_eval(src: &str) -> String {
    let result = JsRuntime::with(|runtime| eval::evaluate(runtime.ctx(), src));
    match result {
        Err(e) => err(format!("eval helper failed: {e}")),
        Ok(text) => {
            // Vue may have scheduled work; land it in this frame like every
            // other command does — in every window that paints the tree, as a
            // JS wave does: an eval that changes a pop-out's content has to
            // repaint the pop-out, not just the window the command ran in.
            let _ = with_commit(|_window, cx| {
                if vn_tree::flush_changes() {
                    // A read-only eval can land after this wave's rAF wrote
                    // one node. Preserve every unaffected cached subtree,
                    // exactly as after_wave does for the same DOM change.
                    crate::notify_shell(cx);
                    crate::popout::notify_all(cx);
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
            let out = with_target(|window, app| {
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
    let focus = with_node_window(node, |window, app| vn_tree::devtools::focus(node, window, app));
    match focus {
        None => return Outcome::Done(err("not in a commit phase")),
        Some(Err(e)) => return Outcome::Done(err(e)),
        Some(Ok(())) => {}
    }
    // gpui installs the native input handler during PAINT, and only for a
    // handle that is already focused — so the keystrokes have to wait a frame.
    // The window rides along: by the time this resumes, the next command has
    // already reset `TARGET_WINDOW`.
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
        // The Menu key is VK_APPS: gpui names it "menu", and the DOM names it
        // "ContextMenu" (`render::dom_key`).
        "contextmenu" => "menu",
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

/// A REAL wheel event, at a window coordinate, through gpui's own hit test.
///
/// `scroll` (the older command) drives an element's `ScrollHandle` directly:
/// it proves a container CAN be scrolled and reports where it ended up, and it
/// never involves the wheel at all. This is the other half — the gesture — and
/// it is what an app's own `@wheel` handler sees. Anything built on that
/// handler (a virtual list, a zoom control, a chart's pan) had no way to be
/// driven, and no way to be tested; the devtools inspector's own console is
/// one, and the inverted `deltaY` it exposed is exactly the kind of bug this
/// closes the door on.
///
/// **The deltas are the DOM's**, not gpui's: `dy > 0` scrolls DOWN, which is
/// what `WheelEvent.deltaY` means and what every client of this protocol is
/// already thinking in. `vn_tree`'s `dom_wheel_delta` performs the same
/// conversion in the other direction on the way out to JS.
fn cmd_wheel(
    selector: Option<&str>,
    index: Option<f64>,
    x: Option<f64>,
    y: Option<f64>,
    dx: Option<f64>,
    dy: Option<f64>,
    modifiers: gpui::Modifiers,
) -> String {
    let (point, node) = match (selector, x, y) {
        (Some(sel), _, _) => match resolve_one(sel, index) {
            Err(e) => return err(e),
            Ok(id) => match vn_tree::devtools::node_info(id).and_then(|n| n.bounds) {
                Some((bx, by, bw, bh)) => ((bx + bw / 2.0, by + bh / 2.0), Some(id)),
                None => return err(format!("`{sel}` is not painted, so it has no centre to aim at")),
            },
        },
        (None, Some(x), Some(y)) => ((x as f32, y as f32), None),
        _ => return err("wheel needs either a `selector` or both `x` and `y` (window coordinates)"),
    };
    let (dx, dy) = (dx.unwrap_or(0.0) as f32, dy.unwrap_or(0.0) as f32);
    if dx == 0.0 && dy == 0.0 {
        return err("wheel needs a `dx` or a `dy` (DOM signs: dy > 0 scrolls down)");
    }
    let position = gpui::Point::new(gpui::px(point.0), gpui::px(point.1));
    let wheel = |window: &mut Window, app: &mut App| {
        // A move first, for the same reason `mouse` sends one: gpui recomputes
        // its hit test from the window's own pointer position, and a wheel that
        // arrives without one is tested against wherever the pointer was last.
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
                // Negated: gpui's sign is the platform's (positive = the wheel
                // turned away from you = a scroll UP), the DOM's is the
                // opposite, and this command speaks the DOM's.
                delta: gpui::ScrollDelta::Pixels(gpui::Point::new(gpui::px(-dx), gpui::px(-dy))),
                modifiers,
                touch_phase: gpui::TouchPhase::Moved,
                // A wheel, as `gpui_windows` reports one — not a touch pan,
                // which gpui would route through its gesture recognizer.
                source: gpui::ScrollSource::Pointer,
            }),
            app,
        );
        if vn_tree::flush_changes() {
            window.refresh();
        }
        true
    };
    // A selector names its own window — the one that PAINTS the node, whose
    // client space its centre is in (as `mouse` does); a bare x/y is in the
    // window the command named.
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

// ---------------------------------------------------------------------------
// windows, raise, perf, storage, net  (v2 — docs/DEVTOOLS.md)
// ---------------------------------------------------------------------------

/// Every OS window this process paints, and every inspector it has open.
///
/// This is what a window switcher is built on, and it answers a question that
/// had none before: with pop-outs, ONE process paints several windows off ONE
/// retained tree (docs/MULTIWINDOW.md), so "screenshot the app" and "what is
/// under the pointer" both stopped being well-posed without a window in them.
fn cmd_windows() -> String {
    let main = crate::main_window_id();
    let popouts = crate::popout::list();
    let mut out: Vec<Json> = Vec::new();
    for id in window_ids() {
        let root = window_root(id);
        let title = if Some(id) == main {
            crate::window::config().title
        } else {
            popouts.iter().find(|(w, _, _)| *w == id).map_or_else(String::new, |(_, _, t)| t.clone())
        };
        let facts = with_window_id(id, |window, _| {
            let size = window.viewport_size();
            (
                f32::from(size.width),
                f32::from(size.height),
                window.is_window_active(),
                window.is_window_hovered(),
                window.scale_factor(),
            )
        });
        let (w, h, active, hovered, scale) = facts.unwrap_or((0.0, 0.0, false, false, 1.0));
        out.push(Json::obj(vec![
            ("id", Json::n(id as f64)),
            ("kind", Json::s(if Some(id) == main { "main" } else { "popout" })),
            ("title", Json::s(title)),
            // The node this window paints as its root. `0` is the document
            // root, i.e. the whole app.
            ("root", Json::n(f64::from(root))),
            ("viewport", Json::obj(vec![("w", Json::n(w)), ("h", Json::n(h))])),
            ("scale", Json::n(scale)),
            ("focused", Json::Bool(active)),
            ("hovered", Json::Bool(hovered)),
        ]));
    }
    ok(Json::obj(vec![
        ("windows", Json::arr(out)),
        ("pid", Json::n(std::process::id() as f64)),
        // The inspectors THIS process has open on itself (F12). An e2e spec
        // proving "F12 twice opens one window" has no other way to look.
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

/// Bring this window to the front. What the SECOND F12 does to an inspector
/// that is already open (`crate::inspector::raise_existing`), and the only
/// command here that deliberately takes focus — because a person just asked for
/// it by pressing a key.
fn cmd_raise() -> String {
    // A test host stays behind the person's windows (src/quiet.rs).
    if crate::quiet::never_focus() {
        return ok(Json::obj(vec![
            ("raised", Json::Bool(false)),
            ("why", Json::s("VN_WINDOW_FOCUS=never")),
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

/// `trace start|stop|peek|mark` — see `trace.rs`.
fn cmd_trace(op: Option<&str>, frames: bool, label: Option<&str>, cap: Option<f64>) -> String {
    let result = match op.unwrap_or("peek") {
        "start" => trace::start(cap.map(|c| c.max(0.0) as usize)),
        "stop" => trace::report(true, frames),
        "peek" => trace::report(false, frames),
        "mark" => {
            trace::mark(label.unwrap_or(""));
            Ok(Json::obj(vec![("marked", Json::Bool(true))]))
        }
        other => Err(format!("trace: unknown op `{other}` — start, stop, peek or mark")),
    };
    match result {
        Ok(data) => ok(data),
        Err(e) => err(e),
    }
}

/// The samples a `perf` reply carries: the last of the rolling ring rather than
/// all of it. The panel draws a sparkline, the push channel sends this four
/// times a second, and 600 floats a message would be 40 KB/s to draw a line
/// 200 px wide.
const PERF_SAMPLES: usize = 120;

/// The rolling frame window (`crate::perf::snapshot`), as the Performance panel
/// reads it.
fn perf_data() -> Json {
    let snap = crate::perf::snapshot();
    let (avg, p50, p95, max, fps) = snap.stats();
    let round2 = |v: f32| f64::from((v * 100.0).round() / 100.0);
    let tail: Vec<Json> = snap
        .samples
        .iter()
        .rev()
        .take(PERF_SAMPLES)
        .rev()
        .map(|ms| Json::n(round2(*ms)))
        .collect();
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
        // Script evaluation before the first frame, per label (preludes,
        // vendor.js, modules, boot.js / app.prod.js) — `crate::perf::boot_eval`.
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
        // One frame's budget: the primary window's display refresh interval
        // (gpui's `Window::refresh_interval`). `null` when the platform does
        // not report it.
        (
            "budgetMs",
            match crate::window::frame_budget_ms() {
                Some(ms) => Json::n((f64::from(ms) * 100.0).round() / 100.0),
                None => Json::Null,
            },
        ),
        ("samples", Json::arr(tail)),
        ("nodes", Json::n(f64::from(vn_tree::devtools::node_count()))),
        // Since the process started: element-tree builds, and the frames shown
        // by a present alone (a `<video>` or moving `<canvas>` frame written
        // into its atlas tile, vn-tree's `live`).
        ("builds", Json::n(vn_tree::devtools::frame_counts().0 as f64)),
        ("presents", Json::n(vn_tree::devtools::frame_counts().1 as f64)),
        // Scroll containers registered with the wheel router last frame. It is
        // here because it is the one structure a retained-render REPLAY cannot
        // rebuild by itself (it is filled by the paint walk), so it is the
        // number that says whether a cached scroll box still routes its wheel
        // — see `crates/vn-tree/src/cache.rs`'s `Boundary`.
        ("scrollChain", Json::n(f64::from(vn_tree::devtools::scroll_chain_count()))),
        ("windows", Json::n(window_ids().len() as f64)),
        // The detailed phase breakdown belongs to `VN_PERF`, which OWNS those
        // counters (it clears them when it reports). Saying whether it is on
        // beats showing zeros that read as "nothing costs anything".
        ("phases", Json::Bool(vn_tree::perf::env_enabled())),
    ])
}

/// The push channel's framing of the same object.
pub(crate) fn perf_json() -> String {
    let Json::Obj(mut entries) = perf_data() else { return String::new() };
    entries.insert(0, ("t".to_string(), Json::s("perf")));
    Json::Obj(entries).to_json()
}

/// `localStorage` / `sessionStorage`, read and written in the app's own JS.
///
/// Deliberately NOT a call into `vn-storage`: the area an app sees is a JS
/// object with a specified surface (docs/STORAGE.md), the host's disk layer is
/// one implementation detail below it, and a panel that read the disk would
/// show something subtly different from what `localStorage.getItem` answers — a
/// pending write, or a session area that has no disk at all. So it goes through
/// `eval`, which is the same machinery the console uses and the same trust
/// level (localhost, token-guarded, `eval`-capable by construction).
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
            return err(format!("unknown storage area {other:?} — use \"local\" or \"session\""))
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
            let Some(k) = key else { return err("storage set needs a `key`") };
            let k = json::quote(k);
            let v = json::quote(value.unwrap_or(""));
            format!(
                "(function(){{var a=globalThis.{global};if(!a)return null;a.setItem({k},{v});\
                 return {{area:{quoted_area},key:{k},value:a.getItem({k})}};}})()"
            )
        }
        "remove" => {
            let Some(k) = key else { return err("storage remove needs a `key`") };
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
    // `cmd_eval`'s reply is `{ok:true,data:{value,type}}`; unwrap one level so a
    // panel gets the object it asked for rather than an envelope. A `null`
    // value means the area does not exist in this host — `--no-default-features`
    // builds it out on purpose (docs/STORAGE.md), and "there is no localStorage
    // here" is the honest answer rather than an empty table.
    let reply = cmd_eval(&js);
    let Some(parsed) = json::parse(&reply) else { return reply };
    if parsed.get("ok").and_then(Json::as_bool) != Some(true) {
        return reply;
    }
    match parsed.get("data").and_then(|d| d.get("value")) {
        Some(Json::Null) | None => err(format!(
            "this host has no `{global}` — it was built without the `storages` feature \
             (docs/STORAGE.md)"
        )),
        Some(v) => ok(v.clone()),
    }
}

/// `dialog`: the dialogs waiting for a scripted answer — `<input
/// type=file>`, the File System Access pickers, `<a download>`'s Save As,
/// and the permission prompt (crates/vn-desktop dialog/scripted.rs). Only
/// under `VN_DIALOGS=scripted`: a real dialog is the user's, and the
/// devtools never answer one.
///
/// `list` (the default) names each one: its `id`, `window`, `what` (`picker`,
/// `download` or `permission`), a file dialog's kind and filters and
/// `suggestedName`, a question's `instruction` and `buttons`. `answer`
/// finishes a file dialog with `paths` (a JSON array, or one string with `|`
/// between paths; `|` cannot appear in a Windows path), checked the way the
/// OS dialog would check them. `choose` presses a question's `button` (its
/// label or index). `cancel` dismisses either as Esc would.
fn cmd_dialog(op: Option<&str>, id: Option<f64>, paths: Option<&Json>, button: Option<String>) -> String {
    use vn_desktop::dialog::{scripted, scripted_mode};
    if !scripted_mode() {
        return err(
            "dialog needs the app started with VN_DIALOGS=scripted — without it every dialog is a \
             real one, which is the user's to answer (docs/DESKTOP.md)",
        );
    }
    // vn-desktop answers serde_json values; the wire embeds their text.
    fn embed(v: impl std::fmt::Display) -> Json {
        Json::from(Raw(v.to_string()))
    }
    let id: Option<u64> = id.filter(|n| n.is_finite() && *n > 0.0).map(|n| n as u64);
    match op.unwrap_or("list") {
        "list" => ok(embed(scripted::list())),
        "answer" => {
            let Some(id) = id else { return err("dialog answer needs the dialog's `id` (see `dialog list`)") };
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
            let Some(id) = id else { return err("dialog choose needs the dialog's `id` (see `dialog list`)") };
            let Some(button) = button else { return err("dialog choose needs a `button`: its label or its index") };
            match scripted::choose(id, &button) {
                Ok(v) => ok(embed(v)),
                Err(e) => err(e),
            }
        }
        "cancel" => {
            let Some(id) = id else { return err("dialog cancel needs the dialog's `id` (see `dialog list`)") };
            match scripted::cancel(id) {
                Ok(v) => ok(embed(v)),
                Err(e) => err(e),
            }
        }
        other => err(format!("unknown dialog op {other:?} — use \"list\", \"answer\", \"choose\" or \"cancel\"")),
    }
}

/// `permissions`: the app's permission states and its stored answers to its
/// own prompt (crates/vn-permissions, docs/PERMISSIONS.md §5.1).
///
/// `list` (the default) gives every capability's four-state `state`, the
/// stored `decisions` (`allow` / `block`, and when) and the `file` they live
/// in. `reset` forgets the answer for `name` (`microphone`), or every
/// answer without one, so the next `getUserMedia` asks again. That is the
/// way back from a Block besides `appPermissions.request`. A real decision:
/// unlike `dialog`, this works on any host.
fn cmd_permissions(op: Option<&str>, name: Option<&str>) -> String {
    match op.unwrap_or("list") {
        "list" => ok(Json::from(Raw(vn_permissions::describe_json()))),
        "reset" => match vn_permissions::reset(name) {
            Ok(names) => ok(Json::obj(vec![
                ("reset", Json::arr(names.into_iter().map(Json::s).collect())),
                ("now", Json::from(Raw(vn_permissions::describe_json()))),
            ])),
            Err(e) => err(e),
        },
        other => err(format!("unknown permissions op {other:?} — use \"list\" or \"reset\"")),
    }
}

/// The network log the JS tee recorded (`net.rs`).
fn cmd_net(since: Option<f64>) -> String {
    let since = since.unwrap_or(0.0).max(0.0) as u64;
    let (entries, next, dropped) = net::since(since);
    ok(Json::obj(vec![
        ("entries", Json::arr(entries.into_iter().map(|t| Json::from(Raw(t))).collect())),
        ("next", Json::n(next as f64)),
        ("dropped", Json::n(dropped as f64)),
        // The pooled transport's counters (connections opened, requests,
        // reuse, retries) — the panel header's chip.
        ("pool", net::pool_json()),
    ]))
}

/// Everything behind one Network row — the request and response headers and
/// bodies, the cookies, the timing, a socket's counts (`net.rs`, "The
/// details"). Asked for per row rather than pushed: a body never rides the
/// feed, and an inspector that never clicks pays nothing for it.
fn cmd_net_detail(seq: f64) -> String {
    if !seq.is_finite() || seq < 1.0 {
        return err(format!("net_detail: {seq} is not a row's seq"));
    }
    match net::detail(seq as u64) {
        Ok(data) => ok(data),
        Err(e) => err(e),
    }
}

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

fn cmd_screenshot(path: Option<&str>, backend: Option<&str>) -> String {
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
                // How to line a `query`/`tree` rectangle up with these pixels:
                //   bitmap_x = css_x * scale + clientX
                // `render_to_image` reads the CLIENT area back, so its offsets
                // are 0; `print_window` captures the whole WINDOW rect
                // (invisible resize border included) and reports a real one.
                // Both capture in physical pixels, so `scale` is the window's
                // scale factor either way.
                ("clientX", Json::n(f64::from(shot.client_origin.0))),
                ("clientY", Json::n(f64::from(shot.client_origin.1))),
                ("scale", Json::n(f64::from(shot.scale))),
            ];
            if let Some(why) = shot.fallback {
                entries.push((
                    "fallback",
                    Json::s(format!("render_to_image unavailable ({why}) — used PrintWindow")),
                ));
            }
            if shot.has_alpha {
                entries.push((
                    "hasAlpha",
                    Json::Bool(true),
                ));
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
    use super::{http_request_line, parse_keystroke, resolve_initial};

    #[test]
    fn mouse_release_does_not_insert_an_unpressed_move_into_a_captured_drag() {
        use gpui::{MouseButton, PlatformInput};
        let at = gpui::point(gpui::px(120.0), gpui::px(-10.0));
        let extras = super::MouseExtras { held: false, clicks: 2, shift: false };
        let events = super::mouse_inputs("up", MouseButton::Left, at, Default::default(), &extras);
        assert_eq!(events.len(), 2);
        let PlatformInput::MouseMove(before_up) = &events[0] else { panic!("move must precede release") };
        assert_eq!(before_up.pressed_button, Some(MouseButton::Left));
        assert_eq!(before_up.position, at, "outside-window coordinates are preserved");
        let PlatformInput::MouseUp(up) = &events[1] else { panic!("the button must release next") };
        assert_eq!(up.button, MouseButton::Left);
        assert_eq!(up.click_count, 2);
        let events = super::mouse_inputs("move", MouseButton::Left, at, Default::default(), &extras);
        let PlatformInput::MouseMove(hover) = &events[0] else { unreachable!() };
        assert_eq!(hover.pressed_button, None, "an ordinary hover still reports no held button");
        let held = super::MouseExtras { held: true, ..extras };
        let events = super::mouse_inputs("move", MouseButton::Left, at, Default::default(), &held);
        let PlatformInput::MouseMove(drag) = &events[0] else { unreachable!() };
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

    /// DOM `KeyboardEvent.key` names map to gpui's, the inverse of
    /// `render::dom_key`. If one of them is missed, the `key` command
    /// dispatches a key that no binding knows, and nothing happens.
    #[test]
    fn dom_key_names_reach_gpui_under_its_own_names() {
        let key = |spec: &str| parse_keystroke(spec).unwrap().key;
        assert_eq!(key("ContextMenu"), "menu");
        assert_eq!(key("Shift+ContextMenu"), "menu");
        assert_eq!(key("ArrowDown"), "down");
        assert_eq!(key("Enter"), "enter");
        assert_eq!(key("F10"), "f10");
        assert!(parse_keystroke("Shift+F10").unwrap().modifiers.shift);
    }

    /// The three inputs to the runtime switch, and which one wins.
    ///
    /// The asymmetry is deliberate and is the whole security posture in one
    /// table: `VN_DEVTOOLS=1` LOCKS (an app must not be able to make itself
    /// undebuggable by somebody who can already set its environment), `=0` does
    /// not (nothing needs protecting from an app enabling its own devtools).
    #[test]
    fn the_env_override_forces_on_and_only_forcing_on_locks() {
        let saved = std::env::var("VN_DEVTOOLS").ok();
        let case = |value: &str, config: bool| {
            std::env::set_var("VN_DEVTOOLS", value);
            resolve_initial(config)
        };

        // Unset (and the empty string, which harnesses use to mean unset): the
        // app's own config decides, and nothing is locked.
        assert_eq!(case("", true), (true, false));
        assert_eq!(case("", false), (false, false));
        // Forced on — including over an app that said no — and locked.
        for on in ["1", "on", "true", "yes", "ON", "True"] {
            assert_eq!(case(on, false), (true, true), "VN_DEVTOOLS={on}");
        }
        // Forced off, and NOT locked: `appDevtools.enable()` still works.
        for off in ["0", "off", "false", "no"] {
            assert_eq!(case(off, true), (false, false), "VN_DEVTOOLS={off}");
        }
        // Nonsense is ignored rather than guessed at, and says so on stderr.
        assert_eq!(case("maybe", true), (true, false));
        assert_eq!(case("maybe", false), (false, false));

        match saved {
            Some(v) => std::env::set_var("VN_DEVTOOLS", v),
            None => std::env::remove_var("VN_DEVTOOLS"),
        }
    }

    /// The whole framing decision is this one function: a JSON request and an
    /// HTTP request cannot be confused for each other, so one port serves both
    /// (docs/DEVTOOLS-UI.md §1). Getting this wrong would mean an inspector's
    /// POST being parsed as a command, or — worse — a `ctl.ts` line being
    /// answered with HTTP headers.
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
        // Every shape the OTHER framing can take.
        assert_eq!(http_request_line("{\"token\":\"x\",\"cmd\":\"ping\"}"), None);
        assert_eq!(http_request_line("{\"cmd\":\"eval\",\"js\":\"'GET / HTTP/1.1'\"}"), None);
        assert_eq!(http_request_line(""), None);
        // …and near-misses that must not be taken for HTTP.
        assert_eq!(http_request_line("POST /"), None);
        assert_eq!(http_request_line("post / HTTP/1.1"), None);
        assert_eq!(http_request_line("HTTP/1.1"), None);
    }
}
