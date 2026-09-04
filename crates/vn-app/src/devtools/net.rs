//! The Network panel's feed: a `fetch` / `WebSocket` tee, recorded in JS.
//!
//! ## Where the hook lives, and why it is in JS
//!
//! redjs' `fetch` runs on a worker thread and settles its promise on the pump;
//! its `WebSocket` runs a socket thread per connection. Instrumenting either in
//! Rust means a hook in the vendored engine, a second place for a request to be
//! described, and a fork patch to carry — for a feature whose whole content is
//! *"what did the app ask for, and what came back"*, which is information the
//! JS side already has in full.
//!
//! So the tee is JS, installed exactly like the `console.*` tee beside it
//! (`super::CONSOLE_TEE_JS`) and for the same reason: it ADDS a reader and
//! changes nothing an app can observe. `fetch` keeps its arguments, its
//! `Response`, its rejection and its `AbortSignal`; the only difference is one
//! more `.then` link in the chain, which is not observable from the app. The
//! `WebSocket` wrapper returns the real socket (a constructor that returns an
//! object returns THAT object) and keeps `WebSocket.prototype`, so `instanceof`
//! and every property still answer as before.
//!
//! Divergence, stated rather than hidden: `globalThis.fetch !== <the engine's
//! own fetch>` in a dev host, and `WebSocket.name` reads `"WebSocket"` because
//! it is set explicitly. Both are true of a browser page with devtools open
//! only if an extension did it; here it is the host, in dev builds, and
//! `--features prod` has none of it.
//!
//! ## Where the entries live
//!
//! A bounded ring (like the log ring, and for the identical reason: an app that
//! polls an endpoint must not grow the process), plus a small pending queue the
//! push channel drains once per frame. The ring is what a client that has just
//! attached reads with the `net` command; the queue is what a client that is
//! already attached is told without asking.

use std::collections::VecDeque;
use std::sync::Mutex;

use vn_runtime::{host_fn, JsRuntime};

/// How many exchanges are remembered. 300 covers a page's whole boot plus a
/// long interactive session; a poll-every-second app fills it in five minutes,
/// which is the point at which the oldest one has stopped being interesting.
const CAPACITY: usize = 300;

struct Ring {
    /// `(seq, the JSON the JS side built)`. Kept as text: it is already valid
    /// JSON, and re-parsing it into `Json` only to print it again would be work
    /// done twice with a chance of changing it.
    entries: VecDeque<(u64, String)>,
    /// Recorded since the push channel last looked.
    pending: Vec<String>,
    next_seq: u64,
    dropped: u64,
}

static RING: Mutex<Option<Ring>> = Mutex::new(None);

fn with_ring<T>(f: impl FnOnce(&mut Ring) -> T) -> T {
    let mut slot = RING.lock().unwrap();
    let ring = slot.get_or_insert_with(|| Ring {
        entries: VecDeque::with_capacity(CAPACITY),
        pending: Vec::new(),
        next_seq: 1,
        dropped: 0,
    });
    f(ring)
}

// `__vn_devtools_net(json)` — one exchange, as the JS tee described it.
//
// The host does not parse it. It stamps a sequence number on it (so a client
// can page) and stores the text; the shape is the tee's business, and it is
// documented in `TEE_JS` where it is built.
host_fn!(fn vn_devtools_net(g, args) {
    let raw = unsafe { vn_runtime::redjs::webapi::util::read_string(g, args.arg(0).0) }
        .unwrap_or_default();
    if raw.is_empty() {
        return vn_runtime::redjs::api::Value::undefined().0;
    }
    with_ring(|ring| {
        let seq = ring.next_seq;
        ring.next_seq += 1;
        // The seq is spliced in rather than wrapped: the panel wants one flat
        // object, and a `{"seq":n,"entry":{…}}` envelope would make every field
        // access in the UI one level deeper for no benefit.
        let stamped = splice_seq(&raw, seq);
        if ring.entries.len() == CAPACITY {
            ring.entries.pop_front();
            ring.dropped += 1;
        }
        ring.entries.push_back((seq, stamped.clone()));
        if ring.pending.len() < CAPACITY {
            ring.pending.push(stamped);
        }
    });
    vn_runtime::redjs::api::Value::undefined().0
});

/// `{"url":…}` → `{"seq":7,"url":…}`. Text surgery rather than a parse/print
/// round trip, because the input is `JSON.stringify` output and the only thing
/// being added is a leading member.
fn splice_seq(raw: &str, seq: u64) -> String {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix('{') {
        let sep = if rest.trim_start().starts_with('}') { "" } else { "," };
        return format!("{{\"seq\":{seq}{sep}{rest}");
    }
    // Not an object: keep it, wrapped, rather than dropping a line the app
    // meant to record.
    format!("{{\"seq\":{seq},\"raw\":{}}}", crate::json::quote(raw))
}

/// Registers the host fn. Always — it is one entry in a table and the tee is
/// the only caller.
pub fn install(rt: &JsRuntime) {
    rt.register("__vn_devtools_net", 1, vn_devtools_net as *const _);
}

/// Wraps `fetch` and `WebSocket`. Split out of [`install`] since m22: the
/// devtools are in every build now, and a host whose app has them DISABLED must
/// leave the engine's own `fetch` alone — the divergence
/// ("`globalThis.fetch` is not the engine's own function in a build that has
/// devtools", docs/DEVTOOLS.md) is only owed when somebody is watching.
pub fn install_tee(rt: &JsRuntime) {
    if let Err(e) = rt.eval(TEE_JS, "vn://devtools-net.js") {
        eprintln!("[devtools] network tee failed: {}\n{}", e.header, e.stack);
    }
}

/// Everything recorded since the last call. Drained by the push channel once
/// per frame.
pub fn drain() -> Vec<String> {
    with_ring(|ring| std::mem::take(&mut ring.pending))
}

/// `(entries, next, dropped)` from `since` — the `net` command's answer.
pub fn since(since: u64) -> (Vec<String>, u64, u64) {
    with_ring(|ring| {
        let out: Vec<String> =
            ring.entries.iter().filter(|(seq, _)| *seq >= since).map(|(_, t)| t.clone()).collect();
        (out, ring.next_seq, ring.dropped)
    })
}

/// The tee. Installed in every dev host, before the app bundle evaluates, so a
/// request the app makes at boot is in the panel too.
const TEE_JS: &str = r#"
(function () {
  var emit = globalThis.__vn_devtools_net;
  if (typeof emit !== "function") return;
  var now = function () { return Date.now(); };
  var record = function (o) { try { emit(JSON.stringify(o)); } catch (e) {} };

  // ---- fetch -------------------------------------------------------------
  var origFetch = globalThis.fetch;
  if (typeof origFetch === "function" && !origFetch.__vnTeed) {
    var teed = function fetch(input, init) {
      var t0 = now();
      var url = "";
      var method = "GET";
      try {
        url = typeof input === "string" ? input : (input && input.url) || String(input);
        method = (init && init.method) || (input && input.method) || "GET";
      } catch (e) {}
      var p;
      try {
        p = origFetch.apply(this, arguments);
      } catch (e) {
        // A synchronous throw is a real fetch failure mode (a bad URL), and it
        // must stay synchronous — so record and rethrow rather than wrapping.
        record({ kind: "fetch", url: url, method: String(method).toUpperCase(),
                 at: t0, ms: 0, error: String(e && e.message || e) });
        throw e;
      }
      if (!p || typeof p.then !== "function") return p;
      return p.then(
        function (res) {
          record({ kind: "fetch", url: url, method: String(method).toUpperCase(), at: t0,
                   ms: now() - t0, status: res && res.status, ok: !!(res && res.ok),
                   type: (res && res.headers && res.headers.get && res.headers.get("content-type")) || undefined });
          return res;
        },
        function (err) {
          record({ kind: "fetch", url: url, method: String(method).toUpperCase(), at: t0,
                   ms: now() - t0, error: String(err && err.message || err) });
          throw err;
        },
      );
    };
    teed.__vnTeed = true;
    globalThis.fetch = teed;
  }

  // ---- WebSocket ---------------------------------------------------------
  var OrigWS = globalThis.WebSocket;
  if (typeof OrigWS === "function" && !OrigWS.__vnTeed) {
    var Wrapped = function WebSocket(url, protocols) {
      var t0 = now();
      // `new OrigWS(...)` with exactly the arguments we were given: passing
      // `undefined` for an absent `protocols` is NOT the same call.
      var ws = arguments.length > 1 ? new OrigWS(url, protocols) : new OrigWS(url);
      var sent = 0, received = 0;
      record({ kind: "ws", url: String(url), at: t0, phase: "open-requested" });
      try {
        ws.addEventListener("open", function () {
          record({ kind: "ws", url: String(url), at: now(), ms: now() - t0, phase: "open" });
        });
        ws.addEventListener("message", function () {
          received++;
          // Counted, not logged: a chatty socket would evict every other entry
          // in the ring within seconds. The count rides the close entry.
        });
        ws.addEventListener("error", function () {
          record({ kind: "ws", url: String(url), at: now(), phase: "error",
                   sent: sent, received: received });
        });
        ws.addEventListener("close", function (e) {
          record({ kind: "ws", url: String(url), at: now(), ms: now() - t0, phase: "close",
                   status: e && e.code, sent: sent, received: received });
        });
        var origSend = ws.send;
        if (typeof origSend === "function") {
          ws.send = function () { sent++; return origSend.apply(this, arguments); };
        }
      } catch (e) {}
      // Returning an object from a constructor makes `new Wrapped()` evaluate
      // to it, so the caller gets the REAL socket and nothing is proxied.
      return ws;
    };
    Wrapped.prototype = OrigWS.prototype;
    for (var k in OrigWS) { try { Wrapped[k] = OrigWS[k]; } catch (e) {} }
    for (var i = 0, names = ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]; i < names.length; i++) {
      try { Wrapped[names[i]] = OrigWS[names[i]]; } catch (e) {}
    }
    Wrapped.__vnTeed = true;
    globalThis.WebSocket = Wrapped;
  }
})();
"#;
