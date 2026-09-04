//! The PUSH channel: a WebSocket server on the devtools port.
//!
//! ```text
//!   GET / HTTP/1.1                     one port, three framings, told apart
//!   Upgrade: websocket        ────▶    by the first request line:
//!                                        {…            NDJSON  (ctl.ts, vn-test)
//!   ◀──── {"t":"logs",…}                 POST / HTTP…  HTTP    (a command)
//!   ◀──── {"t":"perf",…}                 GET  / HTTP…  + Upgrade: websocket
//!   ◀──── {"t":"tree","rev":…}                         THIS
//! ```
//!
//! ## Why this exists
//!
//! v1's inspector polled: `logs` every 700 ms, `tree` every 2500 ms, `pick`
//! every 150 ms while picking. Every one of those costs the app under
//! inspection a frame, and — the part a developer actually feels — a
//! `console.log` took up to 700 ms to appear. "неудобное" is the exact word for
//! a console that lags two thirds of a second behind the app.
//!
//! So the host pushes. Once per frame, on the main thread, inside the same
//! publication the command queue drains in, everything that MOVED this frame is
//! written to every attached socket: new log lines, a tree revision when the
//! tree changed, a frame-time sample every quarter second, the status line when
//! it changed, network entries when the app made a request. A log line now
//! appears in the console in the frame after it was written, and the target
//! pays for exactly what happened rather than for being asked.
//!
//! ## Why on the same port, and why hand-rolled
//!
//! The port already exists, already has a token, and is already discoverable
//! (`<slot>/devtools.json`). A second listener would need a second handshake
//! field, a second firewall answer and a second failure mode, and would buy
//! nothing: RFC 6455's opening handshake IS an HTTP request, so the framing
//! sniff in [`super::serve`] already gets the connection to the right place.
//!
//! The server half is ~200 lines here because the only WebSocket code in the
//! process is redjs's **client** (`redjs::webapi::websocket`), and a client
//! cannot answer an upgrade: it validates `Sec-WebSocket-Accept` instead of
//! computing it, and it rejects masked frames (a server must never mask, and a
//! client must always). What IS reused is the part that would have meant a new
//! dependency — [`accept_key`] (SHA-1 + base64) and the opcode constants. So
//! there is no `sha1` crate here, no `tungstenite`, and one implementation of
//! the handshake hash in the process.
//!
//! ## What a client may send
//!
//! Very little, on purpose — this is a push channel, not a second command
//! transport. Commands keep going over HTTP, where they are already serialized,
//! already one-per-frame and already proven. A client may send:
//!
//! ```json
//! {"since": 1234}                 the log cursor to stream from
//! {"sub": ["logs","perf","tree"]} which streams it wants (default: all)
//! ```
//!
//! …plus `ping` and `close`, which are the protocol's.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use vn_runtime::redjs::webapi::websocket::{accept_key, OP_CLOSE, OP_PING, OP_PONG, OP_TEXT};

use crate::json::{self, Json, Raw};

/// How often a frame-time sample is pushed. Every frame would be 60-165
/// messages a second to draw one sparkline point nobody can see.
const PERF_EVERY_MS: u64 = 250;

/// A socket that has stopped draining is a socket whose peer is wedged. Past
/// this many queued messages it is dropped rather than allowed to grow the
/// process — the same reasoning as the log ring's own bound.
const MAX_QUEUED: usize = 512;

// Which streams a subscriber asked for. Five constants rather than a
// `bitflags` dependency, in a crate that has spent this much effort not
// acquiring any.
const WANT_LOGS: u32 = 1;
const WANT_PERF: u32 = 2;
const WANT_TREE: u32 = 4;
const WANT_NET: u32 = 8;
const WANT_STATUS: u32 = 16;
const WANT_ALL: u32 = WANT_LOGS | WANT_PERF | WANT_TREE | WANT_NET | WANT_STATUS;

struct Sub {
    id: u64,
    tx: Sender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    /// The log sequence this socket has been sent up to.
    cursor: u64,
    want: u32,
}

static SUBS: Mutex<Vec<Sub>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Bumped every frame the retained tree changed, so a client refetches the tree
/// because something moved rather than because 2.5 s went by.
static TREE_REV: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// the upgrade
// ---------------------------------------------------------------------------

/// Answers `GET /` + `Upgrade: websocket` and then serves the connection until
/// it closes. Called from [`super::serve_http`], which has already read the
/// request line and the headers.
///
/// The token is checked from the query string (`/?token=…`), because a browser
/// `WebSocket` constructor cannot set a header — and neither can redjs's, which
/// is the client this exists for. Same token, same guard, same threat model as
/// every other framing on this port.
pub fn serve(target: &str, headers: &[(String, String)], mut out: TcpStream, reader: BufReader<TcpStream>) {
    let header = |name: &str| {
        headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or("")
    };
    let key = header("sec-websocket-key");
    if key.is_empty() {
        let _ = out.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    if header("sec-websocket-version") != "13" {
        let _ = out.write_all(
            b"HTTP/1.1 426 Upgrade Required\r\nSec-WebSocket-Version: 13\r\n\
              Content-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    if !token_ok(target) {
        // Not a 401: this is the same "bad or missing token" the other two
        // framings answer, and a client that gets it should read the handshake
        // file rather than retry with credentials.
        let _ = out.write_all(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    let accept = accept_key(key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if out.write_all(response.as_bytes()).is_err() {
        return;
    }
    let _ = out.flush();

    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let since = query_number(target, "since").unwrap_or(0.0) as u64;
    {
        let mut subs = SUBS.lock().unwrap();
        subs.push(Sub { id, tx: tx.clone(), queued: queued.clone(), cursor: since, want: WANT_ALL });
    }

    // The writer owns the socket's write half for the rest of its life, so the
    // main thread never does I/O: `broadcast` puts bytes on a channel and
    // returns, whatever the peer's network stack is doing.
    let writer_queued = queued.clone();
    let mut write_half = match out.try_clone() {
        Ok(w) => w,
        Err(_) => {
            drop_sub(id);
            return;
        }
    };
    let writer = std::thread::Builder::new()
        .name("vn-devtools-ws-tx".into())
        .spawn(move || {
            for frame in rx {
                writer_queued.fetch_sub(1, Ordering::Relaxed);
                if write_half.write_all(&frame).is_err() {
                    break;
                }
                let _ = write_half.flush();
            }
            // Best-effort courtesy close; the peer may already be gone.
            let _ = write_half.write_all(&encode(OP_CLOSE, &1000u16.to_be_bytes()));
            let _ = write_half.shutdown(std::net::Shutdown::Both);
        })
        .ok();

    send_to(id, &hello());
    eprintln!("[devtools] websocket attached (#{id}) — pushing logs/perf/tree");

    read_loop(id, reader, &tx, &queued);

    drop_sub(id);
    if let Some(w) = writer {
        // Dropping every sender ends the writer's `for frame in rx`.
        drop(tx);
        let _ = w.join();
    }
}

fn token_ok(target: &str) -> bool {
    let Some(token) = query_string(target, "token") else { return false };
    super::TOKEN.lock().map(|t| *t == token).unwrap_or(false)
}

/// `/?token=abc&since=12` → the value of one parameter.
fn query_string(target: &str, name: &str) -> Option<String> {
    let (_, query) = target.split_once('?')?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == name {
            return Some(percent_decode(v));
        }
    }
    None
}

fn query_number(target: &str, name: &str) -> Option<f64> {
    query_string(target, name)?.parse().ok()
}

/// Only what a query string can legally contain; this is not a URL parser.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// reading the client
// ---------------------------------------------------------------------------

fn read_loop(id: u64, mut reader: BufReader<TcpStream>, tx: &Sender<Vec<u8>>, queued: &Arc<AtomicUsize>) {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        // Decode everything already buffered before asking for more, so a
        // client that pipelines two frames into one packet is not stalled.
        loop {
            match parse_client_frame(&buf) {
                Err(e) => {
                    eprintln!("[devtools] websocket #{id}: {e}");
                    return;
                }
                Ok(None) => break,
                Ok(Some((opcode, payload, used))) => {
                    buf.drain(..used);
                    match opcode {
                        OP_CLOSE => return,
                        OP_PING => {
                            let _ = queue(tx, queued, encode(OP_PONG, &payload));
                        }
                        OP_PONG => {}
                        OP_TEXT => on_text(id, &String::from_utf8_lossy(&payload)),
                        // A binary frame from a client that has nothing to say
                        // in binary. Ignored rather than fatal.
                        _ => {}
                    }
                }
            }
        }
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn on_text(id: u64, text: &str) {
    let Some(doc) = json::parse(text) else { return };
    let mut subs = SUBS.lock().unwrap();
    let Some(sub) = subs.iter_mut().find(|s| s.id == id) else { return };
    if let Some(since) = doc.get("since").and_then(Json::as_f64) {
        sub.cursor = since.max(0.0) as u64;
    }
    if let Some(list) = doc.get("sub").and_then(Json::as_arr) {
        let mut want = 0;
        for item in list {
            want |= match item.as_str().unwrap_or("") {
                "logs" => WANT_LOGS,
                "perf" => WANT_PERF,
                "tree" => WANT_TREE,
                "net" => WANT_NET,
                "status" => WANT_STATUS,
                _ => 0,
            };
        }
        sub.want = want;
    }
}

/// One frame off the front of `buf`, unmasked. `Ok(None)` = need more bytes.
///
/// This is the mirror image of redjs' `parse_frame`: a CLIENT frame must be
/// masked (RFC 6455 §5.3), where a server frame must not be, and accepting an
/// unmasked one would mean reading four bytes of payload as a mask key.
fn parse_client_frame(buf: &[u8]) -> Result<Option<(u8, Vec<u8>, usize)>, String> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let (b0, b1) = (buf[0], buf[1]);
    if b0 & 0x70 != 0 {
        return Err("a reserved frame bit is set but no extension was negotiated".into());
    }
    let opcode = b0 & 0x0f;
    if b1 & 0x80 == 0 {
        return Err("the client sent an unmasked frame".into());
    }
    let (len, mut at) = match b1 & 0x7f {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4usize)
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            let n = u64::from_be_bytes([
                buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
            ]);
            // The only client is an inspector sending a 40-byte subscription
            // object; anything at this scale is a bug or an attack.
            if n > (1 << 20) {
                return Err(format!("a client frame of {n} bytes exceeds the 1 MiB cap"));
            }
            (n as usize, 10usize)
        }
        n => (n as usize, 2usize),
    };
    if buf.len() < at + 4 + len {
        return Ok(None);
    }
    let mask = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
    at += 4;
    let mut payload = buf[at..at + len].to_vec();
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i & 3];
    }
    Ok(Some((opcode, payload, at + len)))
}

/// One unmasked server frame. Always `fin`: nothing here fragments.
fn encode(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let n = payload.len();
    let mut out = Vec::with_capacity(n + 10);
    out.push(0x80 | opcode);
    if n < 126 {
        out.push(n as u8);
    } else if n <= u16::MAX as usize {
        out.push(126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

fn queue(tx: &Sender<Vec<u8>>, queued: &Arc<AtomicUsize>, frame: Vec<u8>) -> bool {
    if queued.load(Ordering::Relaxed) >= MAX_QUEUED {
        return false;
    }
    queued.fetch_add(1, Ordering::Relaxed);
    tx.send(frame).is_ok()
}

fn drop_sub(id: u64) {
    if let Ok(mut subs) = SUBS.lock() {
        subs.retain(|s| s.id != id);
    }
}

/// Drops every attached socket. Called by `appDevtools.disable()`
/// (`super::stop_listener`).
///
/// Dropping a `Sub` drops its `Sender`, which ends that socket's writer thread,
/// which closes the socket — so an inspector attached to an app that turns its
/// devtools off sees its feed close rather than going quietly stale. A closing
/// handshake would be politer and is not worth a second code path: the peer is
/// a client that already treats a closed socket as "reattach or give up".
pub fn close_all() {
    if let Ok(mut subs) = SUBS.lock() {
        subs.clear();
    }
}

fn send_to(id: u64, text: &str) {
    let frame = encode(OP_TEXT, text.as_bytes());
    let Ok(subs) = SUBS.lock() else { return };
    if let Some(sub) = subs.iter().find(|s| s.id == id) {
        queue(&sub.tx, &sub.queued, frame);
    }
}

fn hello() -> String {
    Json::obj(vec![
        ("t", Json::s("hello")),
        ("proto", Json::n(1.0)),
        ("pid", Json::n(std::process::id() as f64)),
        ("logCursor", Json::n(vn_tree::log::next_seq() as f64)),
        ("treeRev", Json::n(TREE_REV.load(Ordering::Relaxed) as f64)),
    ])
    .to_json()
}

// ---------------------------------------------------------------------------
// the push, once per frame, on the main thread
// ---------------------------------------------------------------------------

/// The retained tree changed this frame. Called from the frame pump with the
/// value `vn_tree::flush_changes()` already computed, so this costs nothing
/// when nothing moved.
pub fn note_tree_changed() {
    TREE_REV.fetch_add(1, Ordering::Relaxed);
}

thread_local! {
    static LAST_PERF: std::cell::Cell<Option<std::time::Instant>> =
        const { std::cell::Cell::new(None) };
    static LAST_STATUS: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static LAST_TREE_REV: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Everything that moved this frame, to everybody attached.
///
/// Called from `pump_frame` inside the same publication the command queue
/// drains in, so what a socket is told about frame N is what a command running
/// on frame N would have seen. Returns immediately when nobody is attached,
/// which is the state a host is in for its whole life unless somebody opened an
/// inspector on it.
pub fn broadcast_frame() {
    let Ok(mut subs) = SUBS.lock() else { return };
    if subs.is_empty() {
        return;
    }

    // -- logs. One `since` over the oldest cursor, sliced per subscriber, so N
    //    sockets do not walk the ring N times.
    let want_logs = subs.iter().any(|s| s.want & WANT_LOGS != 0);
    if want_logs {
        let oldest = subs
            .iter()
            .filter(|s| s.want & WANT_LOGS != 0)
            .map(|s| s.cursor)
            .min()
            .unwrap_or(0);
        let (entries, dropped) = vn_tree::log::since(oldest);
        if !entries.is_empty() {
            let next = entries.last().map(|e| e.seq + 1).unwrap_or(oldest);
            for sub in subs.iter_mut() {
                if sub.want & WANT_LOGS == 0 {
                    continue;
                }
                let mine: Vec<&vn_tree::log::Entry> =
                    entries.iter().filter(|e| e.seq >= sub.cursor).collect();
                if mine.is_empty() {
                    continue;
                }
                let payload = Json::obj(vec![
                    ("t", Json::s("logs")),
                    (
                        "entries",
                        Json::arr(
                            mine.iter()
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
                    ("next", Json::n(next as f64)),
                    ("dropped", Json::n(dropped as f64)),
                ])
                .to_json();
                sub.cursor = next;
                queue(&sub.tx, &sub.queued, encode(OP_TEXT, payload.as_bytes()));
            }
        }
    }

    // -- the tree revision. A number, not a tree: the client decides whether it
    //    is looking at Elements before spending a frame of the target's budget
    //    on a `tree` command.
    let rev = TREE_REV.load(Ordering::Relaxed);
    if LAST_TREE_REV.with(|c| c.replace(rev)) != rev {
        broadcast(&mut subs, WANT_TREE, &Json::obj(vec![
            ("t", Json::s("tree")),
            ("rev", Json::n(rev as f64)),
        ]).to_json());
    }

    // -- the status line, when it changed.
    if let Some(status) = vn_tree::log::status() {
        let changed = LAST_STATUS.with(|s| {
            if *s.borrow() == status {
                false
            } else {
                *s.borrow_mut() = status.clone();
                true
            }
        });
        if changed {
            broadcast(&mut subs, WANT_STATUS, &Json::obj(vec![
                ("t", Json::s("status")),
                ("text", Json::s(status)),
            ]).to_json());
        }
    }

    // -- network entries the JS tee recorded since last time.
    let net = super::net::drain();
    if !net.is_empty() {
        broadcast(&mut subs, WANT_NET, &Json::obj(vec![
            ("t", Json::s("net")),
            ("entries", Json::arr(net.into_iter().map(|raw| Json::from(Raw(raw))).collect())),
        ]).to_json());
    }

    // -- frame stats, four times a second.
    let now = std::time::Instant::now();
    let due = LAST_PERF.with(|c| match c.get() {
        Some(t) if now.duration_since(t).as_millis() < u128::from(PERF_EVERY_MS) => false,
        _ => {
            c.set(Some(now));
            true
        }
    });
    if due {
        broadcast(&mut subs, WANT_PERF, &super::perf_json());
    }
}

fn broadcast(subs: &mut [Sub], want: u32, text: &str) {
    let frame = encode(OP_TEXT, text.as_bytes());
    for sub in subs.iter() {
        if sub.want & want != 0 {
            queue(&sub.tx, &sub.queued, frame.clone());
        }
    }
}
