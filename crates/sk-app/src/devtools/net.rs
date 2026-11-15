//! The Network panel's feed: `fetch` / `WebSocket` / `XMLHttpRequest` are teed in JS (`TEE_JS`) and
//! recorded in a bounded ring (`*_CAP`, [`BODY_BUDGET`]), with a detail record per exchange served
//! by `net_detail`. The tee only adds a reader; a host whose app has the devtools off never installs it.
//!
//! The ring survives `location.reload()` (same JS realm): seqs keep counting and [`mark_reload`]
//! puts a marker row between the runs. The `WebSocket` tee also wraps
//! `__SK_NATIVE_SCHED__.WebSocket`, so each new runtime's proxy sits over it.
//!
//! Script cannot see `Cookie` / `Set-Cookie`, so the host reads the jar (`webapi::cookies`): the
//! `Cookie` header and a before/after diff (a reconstruction; a concurrent request to the same site
//! lands in the same jar), plus the real lines from `fetch::net_info`. Values leave only through
//! the token-guarded `net_detail`; nothing hands a cookie to JS.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use sk_runtime::redjs::api::Value;
use sk_runtime::redjs::webapi::util::{bufferish, read_string};
use sk_runtime::redjs::webapi::{cookies, fetch};
use sk_runtime::{host_fn, JsRuntime};

use crate::json::{self, Json, Raw};

const CAPACITY: usize = 300;
/// One body. Keep in step with `CAP` in `TEE_JS`.
pub(crate) const BODY_CAP: usize = 256 * 1024;
pub(crate) const BODY_BUDGET: usize = 12 * 1024 * 1024;
const META_CAP: usize = 16 * 1024;
/// Records begun with no summary row yet (in flight, or never settled).
const OPEN_CAP: usize = 128;
const JAR_CAP: usize = 64;
const COOKIE_HEADER_CAP: usize = 8 * 1024;
const SET_COOKIE_LINES_CAP: usize = 32;
/// One raw `Set-Cookie` line: a cookie is at most 4096 bytes of name + value, plus attributes.
const WIRE_LINE_CAP: usize = 4096 + 512;
const WIRE_COOKIES_CAP: usize = 16 * 1024;

const COOKIE_NOTE: &str = "reconstructed from the cookie jar: script never sees Set-Cookie (Fetch \
     forbids it, and redjs strips it before a Response exists), so these lines are what the jar \
     changed for this URL while the request was in flight — a concurrent request to the same site \
     can show up here too. The Cookie header is the jar's view of the URL when the request started.";

const WIRE_NOTE: &str = "Set-Cookie exactly as the transport received it on the final hop (redjs \
     `fetch::net_info`; the Response script sees has none, as Fetch requires). The Cookie header \
     is the jar's view of the URL when the request started.";

/// What the transport says about a response's connection (`fetch::NetInfo`, minus the cookies).
struct Conn {
    protocol: &'static str,
    reused: bool,
    id: u64,
    remote: Option<String>,
    /// HTTP cache outcome; `"hit"` has no connection (`protocol` "", `id` 0).
    cache: &'static str,
}

impl Conn {
    fn columns(&self) -> String {
        let mut out = Vec::new();
        if self.id != 0 {
            out.push(format!(
                "\"proto\":{},\"conn\":{},\"reused\":{}",
                json::quote(self.protocol),
                self.id,
                self.reused
            ));
        }
        if self.cache != "off" {
            out.push(format!("\"cache\":{}", json::quote(self.cache)));
        }
        out.join(",")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Req,
    Res,
}

struct Body {
    bytes: Vec<u8>,
    /// Came from a JS string: text whatever the bytes look like.
    text: bool,
    /// Cut by the host at [`BODY_CAP`] (the tee caps too).
    cut: bool,
    meta: Vec<(String, Json)>,
    evicted: bool,
}

impl Body {
    fn held(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Default)]
struct WsCount {
    sent: u64,
    received: u64,
    sent_bytes: f64,
    received_bytes: f64,
    last_at: f64,
}

struct Cookies {
    sent: Option<String>,
    set: Vec<String>,
}

/// One cookie before/after: `(name, domain, path)` is its identity, the hash covers the rest.
type Print = ((String, String, String), u64);

#[derive(Default)]
struct Record {
    rid: u64,
    seqs: Vec<u64>,
    meta: Vec<(String, Json)>,
    meta_bytes: usize,
    meta_truncated: bool,
    req: Option<Body>,
    res: Option<Body>,
    jar_url: Option<String>,
    jar_before: Option<Vec<Print>>,
    cookies: Option<Cookies>,
    ws: Option<WsCount>,
    conn: Option<Conn>,
    /// The final hop's `Set-Cookie` lines as received; present exactly when `conn` is.
    wire_cookies: Option<Vec<String>>,
}

impl Record {
    fn held(&self) -> usize {
        self.req.as_ref().map_or(0, Body::held) + self.res.as_ref().map_or(0, Body::held)
    }

    fn kind(&self) -> &str {
        self.meta
            .iter()
            .find(|(k, _)| k == "kind")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    /// Merges two levels deep: `{timing:{bodyMs}}` lands beside an earlier `headersMs`.
    fn merge(&mut self, meta: Json, size: usize) {
        let Json::Obj(entries) = meta else { return };
        // A big update past the cap is refused; a small one always lands, so a record with huge headers still finishes.
        if self.meta_bytes + size > META_CAP && size > 1024 {
            self.meta_truncated = true;
            return;
        }
        self.meta_bytes += size;
        merge_obj(&mut self.meta, entries);
    }
}

fn merge_obj(dst: &mut Vec<(String, Json)>, src: Vec<(String, Json)>) {
    for (k, v) in src {
        match dst.iter_mut().find(|(dk, _)| *dk == k) {
            Some((_, Json::Obj(old))) if matches!(v, Json::Obj(_)) => {
                if let Json::Obj(new) = v {
                    for (nk, nv) in new {
                        set(old, &nk, nv);
                    }
                }
            }
            Some((_, old)) => *old = v,
            None => dst.push((k, v)),
        }
    }
}

fn set(obj: &mut Vec<(String, Json)>, key: &str, value: Json) {
    match obj.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value,
        None => obj.push((key.to_string(), value)),
    }
}

fn obj_at<'a>(obj: &'a mut Vec<(String, Json)>, key: &str) -> &'a mut Vec<(String, Json)> {
    let i = match obj.iter().position(|(k, _)| k == key) {
        Some(i) => {
            if !matches!(obj[i].1, Json::Obj(_)) {
                obj[i].1 = Json::Obj(Vec::new());
            }
            i
        }
        None => {
            obj.push((key.to_string(), Json::Obj(Vec::new())));
            obj.len() - 1
        }
    };
    match &mut obj[i].1 {
        Json::Obj(o) => o,
        _ => unreachable!("just made it an object"),
    }
}

fn take_flag(meta: &mut Json, key: &str) -> bool {
    let Json::Obj(entries) = meta else {
        return false;
    };
    match entries.iter().position(|(k, _)| k == key) {
        Some(i) => entries.remove(i).1.as_bool().unwrap_or(false),
        None => false,
    }
}

struct Ring {
    /// `(seq, JSON the JS side built)`, kept as text to skip a parse/print round trip.
    entries: VecDeque<(u64, String)>,
    pending: Vec<String>,
    next_seq: u64,
    dropped: u64,
    /// Ascending `rid` (pushed at the back), so a binary search finds one.
    records: VecDeque<Record>,
    by_seq: HashMap<u64, u64>,
    next_rid: u64,
    held: usize,
    evicted: u64,
}

/// `(summary rows, their JSON bytes, records, body bytes held, metadata bytes, bodies evicted)` for devtools `mem`.
pub(crate) fn mem_stats() -> (usize, usize, usize, usize, usize, u64) {
    let ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(r) = ring.as_ref() else {
        return (0, 0, 0, 0, 0, 0);
    };
    let row_bytes = r.entries.iter().map(|(_, s)| s.capacity()).sum();
    let meta: usize = r.records.iter().map(|x| x.meta_bytes).sum();
    (
        r.entries.len(),
        row_bytes,
        r.records.len(),
        r.held,
        meta,
        r.evicted,
    )
}

impl Ring {
    fn new() -> Ring {
        Ring {
            entries: VecDeque::with_capacity(CAPACITY),
            pending: Vec::new(),
            next_seq: 1,
            dropped: 0,
            records: VecDeque::new(),
            by_seq: HashMap::new(),
            next_rid: 1,
            held: 0,
            evicted: 0,
        }
    }

    fn index_of(&self, rid: u64) -> Option<usize> {
        self.records.binary_search_by_key(&rid, |r| r.rid).ok()
    }

    fn record(&mut self, raw: &str) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let rid = rid_of(raw).filter(|rid| self.index_of(*rid).is_some());
        // The connection is spliced in host-side (script never sees it) so the list can show Protocol /
        // Connection without a `net_detail` per row.
        let conn = rid
            .and_then(|rid| self.records[self.index_of(rid)?].conn.as_ref())
            .map(Conn::columns)
            .unwrap_or_default();
        // Spliced rather than enveloped: the panel wants one flat object.
        let stamped = splice_seq(raw, seq, &conn);
        if self.entries.len() == CAPACITY {
            if let Some((old, _)) = self.entries.pop_front() {
                self.forget_seq(old);
            }
            self.dropped += 1;
        }
        self.entries.push_back((seq, stamped.clone()));
        if self.pending.len() < CAPACITY {
            self.pending.push(stamped);
        }
        if let Some(rid) = rid {
            // `forget_seq` above may have removed a record, so look it up again.
            if let Some(i) = self.index_of(rid) {
                self.records[i].seqs.push(seq);
                self.by_seq.insert(seq, rid);
            }
        }
        seq
    }

    fn conn(&mut self, rid: u64, conn: Conn, set_cookies: Vec<String>) {
        let Some(i) = self.index_of(rid) else { return };
        let mut lines = Vec::new();
        let mut total = 0;
        for line in set_cookies.into_iter().take(SET_COOKIE_LINES_CAP) {
            let line = cap_line(line, WIRE_LINE_CAP);
            if total + line.len() > WIRE_COOKIES_CAP {
                break;
            }
            total += line.len();
            lines.push(line);
        }
        let r = &mut self.records[i];
        r.conn = Some(conn);
        r.wire_cookies = Some(lines);
    }

    /// A summary row left the ring; its record goes with its LAST row.
    fn forget_seq(&mut self, seq: u64) {
        let Some(rid) = self.by_seq.remove(&seq) else {
            return;
        };
        let Some(i) = self.index_of(rid) else { return };
        self.records[i].seqs.retain(|s| *s != seq);
        if self.records[i].seqs.is_empty() {
            if let Some(gone) = self.records.remove(i) {
                self.held -= gone.held();
            }
        }
    }

    fn begin(
        &mut self,
        meta: Json,
        size: usize,
        jar: Option<(String, Vec<cookies::Cookie>)>,
    ) -> u64 {
        if self.records.iter().filter(|r| r.seqs.is_empty()).count() >= OPEN_CAP {
            if let Some(i) = self.records.iter().position(|r| r.seqs.is_empty()) {
                if let Some(gone) = self.records.remove(i) {
                    self.held -= gone.held();
                }
            }
        }
        let rid = self.next_rid;
        self.next_rid += 1;
        let mut r = Record {
            rid,
            ..Default::default()
        };
        r.merge(meta, size);
        if let Some((url, jar)) = jar {
            r.cookies = Some(Cookies {
                sent: cookie_header(&jar),
                set: Vec::new(),
            });
            r.jar_before = Some(jar.iter().map(print).collect());
            r.jar_url = Some(url);
        }
        self.records.push_back(r);
        rid
    }

    fn merge_meta(&mut self, rid: u64, meta: Json, size: usize) {
        if let Some(i) = self.index_of(rid) {
            self.records[i].merge(meta, size);
        }
    }

    fn jar_url(&self, rid: u64) -> Option<String> {
        let r = &self.records[self.index_of(rid)?];
        r.jar_before.as_ref()?;
        r.jar_url.clone()
    }

    fn jar_after(&mut self, rid: u64, after: Vec<cookies::Cookie>) {
        let Some(i) = self.index_of(rid) else { return };
        let r = &mut self.records[i];
        let Some(before) = r.jar_before.take() else {
            return;
        };
        let lines = set_cookie_diff(&before, &after);
        if let Some(c) = r.cookies.as_mut() {
            c.set = lines;
        }
    }

    fn attach_body(&mut self, rid: u64, which: Which, body: Body) {
        let Some(i) = self.index_of(rid) else { return };
        let old = match which {
            Which::Req => self.records[i].req.take(),
            Which::Res => self.records[i].res.take(),
        };
        self.held -= old.map_or(0, |b| b.held());
        self.make_room(body.held());
        self.held += body.held();
        match which {
            Which::Req => self.records[i].req = Some(body),
            Which::Res => self.records[i].res = Some(body),
        }
    }

    fn make_room(&mut self, need: usize) {
        if self.held + need <= BODY_BUDGET {
            return;
        }
        for r in self.records.iter_mut() {
            for slot in [&mut r.req, &mut r.res] {
                if let Some(b) = slot {
                    if !b.evicted && !b.bytes.is_empty() {
                        self.held -= b.bytes.len();
                        b.bytes = Vec::new();
                        b.evicted = true;
                        self.evicted += 1;
                        if self.held + need <= BODY_BUDGET {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Counted, never kept: a chatty socket would evict every other row.
    fn ws_count(&mut self, rid: u64, received: bool, bytes: f64, at: f64) {
        let Some(i) = self.index_of(rid) else { return };
        let w = self.records[i].ws.get_or_insert_with(WsCount::default);
        if received {
            w.received += 1;
            w.received_bytes += bytes.max(0.0);
        } else {
            w.sent += 1;
            w.sent_bytes += bytes.max(0.0);
        }
        w.last_at = at;
    }

    fn detail(&self, seq: u64) -> Result<Json, String> {
        let Some((_, raw)) = self.entries.iter().find(|(s, _)| *s == seq) else {
            return Err(match (self.entries.front(), self.entries.back()) {
                (Some(lo), Some(hi)) => format!(
                    "no network entry {seq} — the ring holds {}..={} (the newest {CAPACITY})",
                    lo.0, hi.0
                ),
                _ => format!("no network entry {seq} — nothing has been recorded yet"),
            });
        };
        let entry = Json::from(Raw(raw.clone()));
        let limits = Json::obj(vec![
            ("bodyCap", Json::n(BODY_CAP as f64)),
            ("budget", Json::n(BODY_BUDGET as f64)),
            ("used", Json::n(self.held as f64)),
            ("evicted", Json::n(self.evicted as f64)),
        ]);
        let Some(r) = self
            .by_seq
            .get(&seq)
            .and_then(|rid| self.index_of(*rid))
            .map(|i| &self.records[i])
        else {
            let marker = raw.contains("\"reload\"")
                && json::parse(raw)
                    .is_some_and(|j| j.get("kind").and_then(Json::as_str) == Some("reload"));
            let note = if marker {
                "a reload marker: the host ran the app again here (`cause` says why), and the rows \
                 before it are the previous run's. There is no exchange behind it"
            } else {
                "no details were recorded for this entry: the devtools were off when it \
                 started, or it was recorded by a tee older than net_detail"
            };
            return Ok(Json::obj(vec![
                ("entry", entry),
                ("detail", Json::Null),
                ("note", Json::s(note)),
                ("limits", limits),
            ]));
        };
        let mut out = r.meta.clone();
        set(&mut out, "entry", entry);
        set(&mut out, "rid", Json::n(r.rid as f64));
        set(
            &mut out,
            "seqs",
            Json::arr(r.seqs.iter().map(|s| Json::n(*s as f64)).collect()),
        );
        if r.kind() != "ws" {
            set(
                obj_at(&mut out, "request"),
                "body",
                body_json(r.req.as_ref()),
            );
            set(
                obj_at(&mut out, "response"),
                "body",
                body_json(r.res.as_ref()),
            );
        }
        if let Some(c) = &r.conn {
            // The transport's account replaces the tee's `nextHopProtocol` guess.
            set(
                &mut out,
                "transport",
                Json::obj(vec![
                    ("protocol", Json::s(c.protocol)),
                    ("reused", Json::Bool(c.reused)),
                    ("connectionId", Json::n(c.id as f64)),
                    ("remote", c.remote.clone().map_or(Json::Null, Json::s)),
                    ("cache", Json::s(c.cache)),
                ]),
            );
        }
        if r.cookies.is_some() || r.wire_cookies.is_some() {
            let sent = r.cookies.as_ref().and_then(|c| c.sent.clone());
            let set_lines = r
                .cookies
                .as_ref()
                .map(|c| c.set.clone())
                .unwrap_or_default();
            let mut cookies = vec![
                ("sent", sent.map_or(Json::Null, Json::s)),
                (
                    "set",
                    Json::arr(set_lines.into_iter().map(Json::s).collect()),
                ),
                ("pending", Json::Bool(r.jar_before.is_some())),
            ];
            match &r.wire_cookies {
                Some(lines) => {
                    cookies.push((
                        "wire",
                        Json::arr(lines.iter().cloned().map(Json::s).collect()),
                    ));
                    cookies.push(("note", Json::s(WIRE_NOTE)));
                }
                None => cookies.push(("note", Json::s(COOKIE_NOTE))),
            }
            set(&mut out, "cookies", Json::obj(cookies));
        }
        if r.kind() == "ws" {
            let w = r.ws.as_ref();
            let ws = obj_at(&mut out, "ws");
            set(ws, "sent", Json::n(w.map_or(0, |w| w.sent) as f64));
            set(ws, "received", Json::n(w.map_or(0, |w| w.received) as f64));
            set(ws, "sentBytes", Json::n(w.map_or(0.0, |w| w.sent_bytes)));
            set(
                ws,
                "receivedBytes",
                Json::n(w.map_or(0.0, |w| w.received_bytes)),
            );
            if let Some(w) = w {
                set(ws, "lastAt", Json::n(w.last_at));
            }
        }
        if r.meta_truncated {
            set(&mut out, "metaTruncated", Json::Bool(true));
        }
        set(&mut out, "limits", limits);
        Ok(Json::Obj(out))
    }
}

fn rid_of(raw: &str) -> Option<u64> {
    // Cheap pre-check: rows without a rid are common, and a parse is the only other test.
    if !raw.contains("\"rid\"") {
        return None;
    }
    let n = json::parse(raw)?.get("rid")?.as_f64()?;
    (n.is_finite() && n >= 1.0).then_some(n as u64)
}

/// `{"url":…}` → `{"seq":7,"url":…}` by text surgery on `JSON.stringify` output; `extra` is more leading members, serialized.
fn splice_seq(raw: &str, seq: u64, extra: &str) -> String {
    let head = if extra.is_empty() {
        format!("\"seq\":{seq}")
    } else {
        format!("\"seq\":{seq},{extra}")
    };
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix('{') {
        let sep = if rest.trim_start().starts_with('}') {
            ""
        } else {
            ","
        };
        return format!("{{{head}{sep}{rest}");
    }
    // Not an object: keep it, wrapped, rather than drop the line.
    format!("{{{head},\"raw\":{}}}", crate::json::quote(raw))
}

fn cap_line(mut s: String, cap: usize) -> String {
    if s.len() > cap {
        let mut at = cap;
        while !s.is_char_boundary(at) {
            at -= 1;
        }
        s.truncate(at);
    }
    s
}

fn textual(mime: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    let essence = m.split(';').next().unwrap_or("").trim();
    essence.starts_with("text/")
        || essence.ends_with("+json")
        || essence.ends_with("+xml")
        || matches!(
            essence,
            "application/json"
                | "application/javascript"
                | "application/ecmascript"
                | "application/xml"
                | "application/x-www-form-urlencoded"
                | "application/graphql"
                | "application/x-ndjson"
        )
}

/// Binary media types, even when the bytes happen to decode as UTF-8.
fn binary(mime: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    let essence = m.split(';').next().unwrap_or("").trim();
    essence.starts_with("image/")
        || essence.starts_with("audio/")
        || essence.starts_with("video/")
        || essence.starts_with("font/")
        || matches!(
            essence,
            "application/octet-stream"
                | "application/ion"
                | "application/x-amzn-ion"
                | "application/protobuf"
                | "application/x-protobuf"
                | "application/grpc"
                | "application/grpc-web"
                | "application/msgpack"
                | "application/x-msgpack"
                | "application/cbor"
                | "application/zip"
                | "application/gzip"
                | "application/pdf"
                | "application/wasm"
        )
}

/// A body cut at the cap may end mid-character; that tail is dropped rather than calling it binary.
fn encode(bytes: &[u8], mime: &str, from_string: bool) -> (&'static str, String) {
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&bytes[..e.valid_up_to()]).ok(),
        Err(_) => None,
    };
    match text {
        Some(s) if from_string => ("utf8", s.to_string()),
        Some(s)
            if !binary(mime)
                && (textual(mime)
                    || !s
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))) =>
        {
            ("utf8", s.to_string())
        }
        _ => (
            "base64",
            sk_runtime::redjs::webapi::fsutil::b64_encode(bytes),
        ),
    }
}

fn body_json(b: Option<&Body>) -> Json {
    let Some(b) = b else { return Json::Null };
    let mut o = b.meta.clone();
    if b.evicted {
        set(&mut o, "evicted", Json::Bool(true));
        set(&mut o, "stored", Json::n(0.0));
    } else {
        let mime = o
            .iter()
            .find(|(k, _)| k == "mime")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string();
        let (encoding, data) = encode(&b.bytes, &mime, b.text);
        set(&mut o, "encoding", Json::s(encoding));
        set(&mut o, "data", Json::s(data));
        set(&mut o, "stored", Json::n(b.bytes.len() as f64));
    }
    let incomplete = o
        .iter()
        .any(|(k, v)| k == "complete" && v.as_bool() == Some(false));
    set(&mut o, "truncated", Json::Bool(b.cut || incomplete));
    Json::Obj(o)
}

fn cap_string(mut s: String) -> (Vec<u8>, bool) {
    if s.len() <= BODY_CAP {
        return (s.into_bytes(), false);
    }
    let mut at = BODY_CAP;
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    s.truncate(at);
    (s.into_bytes(), true)
}

fn jar_snapshot(url: &str) -> Vec<cookies::Cookie> {
    let mut all = cookies::get(Some(url));
    all.truncate(JAR_CAP);
    all
}

/// The `Cookie` header from a jar view: longest path first, then oldest. Not `cookies::header_for`:
/// it bumps every cookie's last-access time (the jar's LRU), which a devtools read must not do.
fn cookie_header(jar: &[cookies::Cookie]) -> Option<String> {
    if jar.is_empty() {
        return None;
    }
    let mut sorted: Vec<&cookies::Cookie> = jar.iter().collect();
    sorted.sort_by(|a, b| {
        b.path
            .len()
            .cmp(&a.path.len())
            .then(a.creation.cmp(&b.creation))
    });
    let mut out = String::new();
    for c in sorted {
        if !out.is_empty() {
            out.push_str("; ");
        }
        if !c.name.is_empty() {
            out.push_str(&c.name);
            out.push('=');
        }
        out.push_str(&c.value);
    }
    if out.len() > COOKIE_HEADER_CAP {
        let mut at = COOKIE_HEADER_CAP;
        while !out.is_char_boundary(at) {
            at -= 1;
        }
        out.truncate(at);
        out.push('…');
    }
    Some(out)
}

fn print(c: &cookies::Cookie) -> Print {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        &c.value,
        c.expires,
        c.secure,
        c.http_only,
        c.host_only,
        c.same_site.as_str(),
    )
        .hash(&mut h);
    (
        (c.name.clone(), c.domain.clone(), c.path.clone()),
        h.finish(),
    )
}

fn set_cookie_diff(before: &[Print], after: &[cookies::Cookie]) -> Vec<String> {
    let mut lines = Vec::new();
    for c in after {
        let (key, fp) = print(c);
        if before.iter().any(|(k, f)| *k == key && *f == fp) {
            continue;
        }
        lines.push(set_cookie_line(c));
    }
    for ((name, domain, path), _) in before {
        if !after
            .iter()
            .any(|c| c.name == *name && c.domain == *domain && c.path == *path)
        {
            lines.push(format!(
                "{name}=; Domain={domain}; Path={path}; (deleted or expired)"
            ));
        }
    }
    lines.truncate(SET_COOKIE_LINES_CAP);
    lines
}

fn set_cookie_line(c: &cookies::Cookie) -> String {
    let mut s = if c.name.is_empty() {
        c.value.clone()
    } else {
        format!("{}={}", c.name, c.value)
    };
    if !c.host_only {
        s.push_str(&format!("; Domain={}", c.domain));
    }
    s.push_str(&format!("; Path={}", c.path));
    if let Some(e) = c.expires {
        s.push_str(&format!("; Expires={}", http_date(e)));
    }
    if c.secure {
        s.push_str("; Secure");
    }
    if c.http_only {
        s.push_str("; HttpOnly");
    }
    let ss = c.same_site.as_str();
    if !ss.is_empty() {
        s.push_str(&format!("; SameSite={ss}"));
    }
    s
}

/// IMF-fixdate, the `Expires` format.
fn http_date(secs: i64) -> String {
    const WD: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MON: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WD[(days + 4).rem_euclid(7) as usize],
        day,
        MON[(month - 1) as usize],
        year,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

static RING: Mutex<Option<Ring>> = Mutex::new(None);

fn with_ring<T>(f: impl FnOnce(&mut Ring) -> T) -> T {
    let mut slot = RING.lock().unwrap();
    f(slot.get_or_insert_with(Ring::new))
}

fn rid_arg(v: Value) -> Option<u64> {
    v.as_number()
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map(|n| n as u64)
}

// One summary row: the host parses only its `rid`, stamps a seq and stores the text.
host_fn!(fn sk_devtools_net(g, args) {
    let raw = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    if raw.is_empty() {
        return Value::undefined().0;
    }
    let seq = with_ring(|ring| ring.record(&raw));
    Value::number(seq as f64).0
});

// Returns a record id, or 0 when the devtools are off (the tee then skips every detail).
host_fn!(fn sk_devtools_net_begin(g, args) {
    if !super::enabled() {
        return Value::number(0.0).0;
    }
    let raw = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let Some(mut meta) = json::parse(&raw) else { return Value::number(0.0).0 };
    let jar = take_flag(&mut meta, "jar");
    let snapshot = if jar {
        meta.get("url").and_then(Json::as_str).map(|u| (u.to_string(), jar_snapshot(u)))
    } else {
        None
    };
    let rid = with_ring(|ring| ring.begin(meta, raw.len(), snapshot));
    Value::number(rid as f64).0
});

// Merged two levels deep.
host_fn!(fn sk_devtools_net_meta(g, args) {
    let Some(rid) = rid_arg(args.arg(0)) else { return Value::undefined().0 };
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let Some(mut meta) = json::parse(&raw) else { return Value::undefined().0 };
    let after = take_flag(&mut meta, "jarAfter");
    // The jar has its own lock; it is never taken while RING is held.
    let url = if after { with_ring(|ring| ring.jar_url(rid)) } else { None };
    let snapshot = url.map(|u| jar_snapshot(&u));
    with_ring(|ring| {
        ring.merge_meta(rid, meta, raw.len());
        if let Some(s) = snapshot {
            ring.jar_after(rid, s);
        }
    });
    Value::undefined().0
});

// `data` is a buffer, view or string, or null when the tee read none. At most BODY_CAP bytes are copied.
host_fn!(fn sk_devtools_net_body(g, args) {
    let Some(rid) = rid_arg(args.arg(0)) else { return Value::undefined().0 };
    let which = match unsafe { read_string(g, args.arg(1).0) }.as_deref() {
        Some("req") => Which::Req,
        Some("res") => Which::Res,
        _ => return Value::undefined().0,
    };
    let data = args.arg(2);
    let (bytes, text, cut) = if data.is_null() || data.is_undefined() {
        (Vec::new(), false, false)
    } else if let Some(view) = unsafe { bufferish(g, data.0) } {
        // A pointer into JS memory: copied before anything else can run.
        let n = view.len().min(BODY_CAP);
        (view[..n].to_vec(), false, view.len() > BODY_CAP)
    } else {
        let (b, cut) = cap_string(unsafe { read_string(g, data.0) }.unwrap_or_default());
        (b, true, cut)
    };
    let meta = match unsafe { read_string(g, args.arg(3).0) }.as_deref().and_then(json::parse) {
        Some(Json::Obj(o)) => o,
        _ => Vec::new(),
    };
    with_ring(|ring| ring.attach_body(rid, which, Body { bytes, text, cut, meta, evicted: false }));
    Value::undefined().0
});

// `__sk_devtools_net_ws(rid, direction, size)`: one message, 0 sent / 1 received. Counted, never kept.
host_fn!(fn sk_devtools_net_ws(g, args) {
    let _ = g;
    let Some(rid) = rid_arg(args.arg(0)) else { return Value::undefined().0 };
    let received = args.arg(1).as_number() == Some(1.0);
    let size = args.arg(2).as_number().unwrap_or(0.0);
    let at = now_ms();
    with_ring(|ring| ring.ws_count(rid, received, size, at));
    Value::undefined().0
});

// Returns nothing on purpose: it is a global the app could call, and it reads what script must not
// see (HttpOnly cookies, the remote address). Values leave only through the token-guarded
// `net_detail` and the summary row, which `Ring::record` splices host-side.
host_fn!(fn sk_devtools_net_conn(g, args) {
    let Some(rid) = rid_arg(args.arg(0)) else { return Value::undefined().0 };
    let response = args.arg(1);
    if response.is_null() || response.is_undefined() {
        return Value::undefined().0;
    }
// SAFETY: `g` is this context's global and `response` a value of its heap, read on the JS thread; a value
// that is not a redjs `Response` answers `None`.
    let Some(info) = (unsafe { fetch::net_info(g, response.0) }) else { return Value::undefined().0 };
    let conn = Conn {
        protocol: info.protocol,
        reused: info.reused,
        id: info.connection_id,
        remote: info.remote_addr.map(|a| a.to_string()),
        cache: info.cache.as_str(),
    };
    with_ring(|ring| ring.conn(rid, conn, info.set_cookies));
    Value::undefined().0
});

/// A summary-only row for a load the host made. Called on the network thread.
pub fn record_loader(
    method: &str,
    url: &str,
    at: f64,
    ms: f64,
    result: &Result<
        sk_runtime::redjs::webapi::http::Response,
        sk_runtime::redjs::webapi::http::HttpError,
    >,
) {
    let mut fields = vec![
        ("url", Json::s(url)),
        ("method", Json::s(method)),
        ("at", Json::n(at)),
        ("ms", Json::n(ms)),
    ];
    let kind = match result {
        Ok(r) => {
            let ty =
                sk_runtime::redjs::webapi::http::header(&r.headers, "content-type").unwrap_or("");
            fields.push(("status", Json::n(r.status as f64)));
            fields.push(("ok", Json::Bool((200..300).contains(&r.status))));
            if !ty.is_empty() {
                fields.push(("type", Json::s(ty)));
            }
            if r.connection_id != 0 {
                fields.push(("proto", Json::s(r.protocol)));
                fields.push(("conn", Json::n(r.connection_id as f64)));
                fields.push(("reused", Json::Bool(r.reused)));
            }
            if r.cache != sk_runtime::redjs::webapi::http::cache::Outcome::Off {
                fields.push(("cache", Json::s(r.cache.as_str())));
            }
            let t = ty.to_ascii_lowercase();
            if t.starts_with("image/") {
                "img"
            } else if t.starts_with("font/") || t.contains("font-woff") || t.contains("opentype") {
                "font"
            } else {
                "other"
            }
        }
        Err(e) => {
            fields.push(("error", Json::s(e.message.clone())));
            "other"
        }
    };
    fields.insert(0, ("kind", Json::s(kind)));
    let raw = Json::obj(fields).to_json();
    with_ring(|ring| ring.record(&raw));
}

/// The HTTP cache's counters plus the host's account of the folder; `null` when none is installed.
pub fn cache_json() -> Json {
    let Some(s) = sk_runtime::redjs::webapi::http::cache::stats() else {
        return Json::Null;
    };
    Json::obj(vec![
        ("dir", Json::s(s.dir.display().to_string())),
        (
            "maxMB",
            Json::n((s.max_bytes as f64 / 1048576.0 * 10.0).round() / 10.0),
        ),
        ("entries", Json::n(s.entries as f64)),
        (
            "MB",
            Json::n((s.bytes as f64 / 1048576.0 * 10.0).round() / 10.0),
        ),
        ("writable", Json::Bool(s.writable)),
        ("sealed", Json::Bool(s.sealed)),
        ("hits", Json::n(s.hits as f64)),
        ("revalidated", Json::n(s.revalidated as f64)),
        ("stored", Json::n(s.stored as f64)),
        ("misses", Json::n(s.misses as f64)),
        ("bypassed", Json::n(s.bypassed as f64)),
        ("evicted", Json::n(s.evicted as f64)),
        ("errors", Json::n(s.errors as f64)),
    ])
}

/// `http::pool` needs redjs's `net-h2`, which sk-runtime enables by default.
pub fn pool_json() -> Json {
    let s = sk_runtime::redjs::webapi::http::pool::stats();
    Json::obj(vec![
        ("h1", Json::n(s.h1_connections as f64)),
        ("h2", Json::n(s.h2_connections as f64)),
        ("requests", Json::n(s.requests as f64)),
        ("reused", Json::n(s.reused as f64)),
        ("retries", Json::n(s.retries as f64)),
    ])
}

/// Registers the host fns the tee calls.
pub fn install(rt: &JsRuntime) {
    rt.register("__sk_devtools_net", 1, sk_devtools_net as *const _);
    rt.register(
        "__sk_devtools_net_begin",
        1,
        sk_devtools_net_begin as *const _,
    );
    rt.register(
        "__sk_devtools_net_meta",
        2,
        sk_devtools_net_meta as *const _,
    );
    rt.register(
        "__sk_devtools_net_body",
        4,
        sk_devtools_net_body as *const _,
    );
    rt.register("__sk_devtools_net_ws", 3, sk_devtools_net_ws as *const _);
    rt.register(
        "__sk_devtools_net_conn",
        2,
        sk_devtools_net_conn as *const _,
    );
}

/// Separate from [`install`]: a host whose app has the devtools off leaves the engine's `fetch` alone.
pub fn install_tee(rt: &JsRuntime) {
    if let Err(e) = rt.eval(TEE_JS, "sk://devtools-net.js") {
        eprintln!("[devtools] network tee failed: {}\n{}", e.header, e.stack);
    }
}

/// A `kind: "reload"` marker row, called after the old app's unload and before the new bundle
/// evaluates, so only what the old run still had in flight settles below it.
pub fn mark_reload(cause: &str) {
    let raw = Json::obj(vec![
        ("kind", Json::s("reload")),
        ("url", Json::s("")),
        ("cause", Json::s(cause)),
        ("at", Json::n(now_ms().round())),
    ])
    .to_json();
    with_ring(|ring| {
        ring.record(&raw);
    });
}

/// Everything recorded since the last call; the push channel drains it once per frame.
pub fn drain() -> Vec<String> {
    with_ring(|ring| std::mem::take(&mut ring.pending))
}

/// `(entries, next, dropped)` from `since`: the `net` command's answer.
pub fn since(since: u64) -> (Vec<String>, u64, u64) {
    with_ring(|ring| {
        let out: Vec<String> = ring
            .entries
            .iter()
            .filter(|(seq, _)| *seq >= since)
            .map(|(_, t)| t.clone())
            .collect();
        (out, ring.next_seq, ring.dropped)
    })
}

/// The `net_detail` answer: everything recorded about the exchange behind summary row `seq`.
pub fn detail(seq: u64) -> Result<Json, String> {
    with_ring(|ring| ring.detail(seq))
}

/// The tee, installed before the app bundle evaluates so boot requests show up. It runs inside someone
/// else's app, so:
///
/// * Nothing it does reaches the app: every step is in a `try`, every promise it makes has a rejection
///   handler.
/// * It never consumes the app's body. It reads `res.clone()`, taken synchronously in its own `.then`
///   (before any app handler), through `getReader()`, and cancels at the cap; never `arrayBuffer()`,
///   which would buffer an endless stream. It never touches `res.body` on the app's response: in redjs
///   that getter marks the body used.
/// * Skipped, and the panel says so: HEAD / 204 / 205 / 304, `text/event-stream`, media bodies, and
///   `Content-Length` over 16 MB (redjs copies a clone's whole body into the JS heap before the cap
///   applies). Request `Blob`s are read up to 64 KB, a `FormData` is shown as fields, a `Request`
///   object's body is not read.
/// * `XMLHttpRequest` is teed only where the engine has one: three prototype methods and `loadend`.
const TEE_JS: &str = r#"
(function () {
  var G = globalThis;
  var emit = G.__sk_devtools_net;
  if (typeof emit !== "function") return;
  var hBegin = G.__sk_devtools_net_begin, hMeta = G.__sk_devtools_net_meta;
  var hBody = G.__sk_devtools_net_body, hWs = G.__sk_devtools_net_ws, hConn = G.__sk_devtools_net_conn;
  var deep = typeof hBegin === "function" && typeof hMeta === "function" && typeof hBody === "function";

  // Keep CAP in step with net.rs BODY_CAP.
  var CAP = 262144, READ_LIMIT = 16777216, SMALL_BLOB = 65536;
  var HEADERS = 100, HVALUE = 4096, FIELDS = 64, FVALUE = 512;

  // Captured once: an app that patches Date.now or JSON.stringify must not change what is recorded.
  var dateNow = Date.now, stringify = JSON.stringify, isArray = Array.isArray;
  var now = function () { return dateNow.call(Date); };
  var noop = function () {};
  var str = function (v) { try { return String(v); } catch (e) { return ""; } };
  var message = function (e) { try { return String((e && e.message) || e); } catch (x) { return "?"; } };
  var clip = function (s, n) { return s.length > n ? s.slice(0, n) + "…" : s; };

  var record = function (o) {
    try { var s = emit(stringify(o)); return typeof s === "number" ? s : 0; } catch (e) { return 0; }
  };
  var begin = function (o) {
    if (!deep) return 0;
    try { var r = hBegin(stringify(o)); return typeof r === "number" ? r : 0; } catch (e) { return 0; }
  };
  var meta = function (rid, o) { if (!rid) return; try { hMeta(rid, stringify(o)); } catch (e) {} };
  var body = function (rid, which, data, o) {
    if (!rid) return;
    try { hBody(rid, which, data == null ? null : data, stringify(o || {})); } catch (e) {}
  };

  var pairs = function (h) {
    var out = [];
    if (h === undefined || h === null) return out;
    try {
      if (typeof h.forEach === "function" && typeof h.get === "function") {
        h.forEach(function (v, k) { if (out.length < HEADERS) out.push([str(k), clip(str(v), HVALUE)]); });
      } else if (isArray(h)) {
        for (var i = 0; i < h.length && out.length < HEADERS; i++) {
          var p = h[i];
          if (p && p.length >= 2) out.push([str(p[0]), clip(str(p[1]), HVALUE)]);
        }
      } else if (typeof h === "object") {
        var ks = Object.keys(h);
        for (var j = 0; j < ks.length && out.length < HEADERS; j++) out.push([ks[j], clip(str(h[ks[j]]), HVALUE)]);
      }
    } catch (e) {}
    return out;
  };
  var headerOf = function (list, name) {
    for (var i = 0; i < list.length; i++) if (str(list[i][0]).toLowerCase() === name) return list[i][1];
    return "";
  };
  var rawPairs = function (text) {
    var out = [], lines = str(text).split(/\r?\n/);
    for (var i = 0; i < lines.length && out.length < HEADERS; i++) {
      var at = lines[i].indexOf(":");
      if (at > 0) out.push([lines[i].slice(0, at).trim().toLowerCase(), clip(lines[i].slice(at + 1).trim(), HVALUE)]);
    }
    return out;
  };
  var sizeOf = function (v) {
    if (v == null) return 0;
    if (typeof v === "string") return v.length;
    if (typeof v.byteLength === "number") return v.byteLength;
    if (typeof v.size === "number") return v.size;
    return 0;
  };
  // A no-copy Uint8Array over the first n bytes; the host copies at most CAP.
  var head = function (b, n) {
    if (b instanceof ArrayBuffer) return new Uint8Array(b, 0, Math.min(b.byteLength, n));
    return new Uint8Array(b.buffer, b.byteOffset, Math.min(b.byteLength, n));
  };
  var isBytes = function (b) {
    return typeof ArrayBuffer === "function" && (b instanceof ArrayBuffer || ArrayBuffer.isView(b));
  };

  // Fallback only: the host reports protocol, reuse and remote address itself (hConn). This reads
  // PerformanceResourceTiming.nextHopProtocol for an engine that fills it in.
  var transport = function (url) {
    var out = {};
    try {
      var perf = G.performance;
      if (perf && typeof perf.getEntriesByName === "function") {
        var list = perf.getEntriesByName(url, "resource");
        var e = list && list.length ? list[list.length - 1] : null;
        if (e && e.nextHopProtocol) out.protocol = str(e.nextHopProtocol);
      }
    } catch (x) {}
    return out;
  };

  var USP = G.URLSearchParams, FD = G.FormData, BL = G.Blob;
  // `ct` is the Content-Type the app set: that, not the type the body implies, is what went out.
  var textBody = function (rid, s, implied, kind, ct) {
    body(rid, "req", s.length > CAP ? s.slice(0, CAP) : s,
         { kind: kind, total: s.length, unit: "chars", complete: s.length <= CAP, mime: ct || implied });
    return { kind: kind, implied: implied };
  };
  var describeBody = function (rid, b, ct) {
    if (b === undefined || b === null) return null;
    if (typeof b === "string") return textBody(rid, b, "text/plain;charset=UTF-8", "text", ct);
    if (typeof USP === "function" && b instanceof USP) {
      return textBody(rid, str(b), "application/x-www-form-urlencoded;charset=UTF-8", "urlencoded", ct);
    }
    if (typeof FD === "function" && b instanceof FD) {
      var fields = [];
      b.forEach(function (v, k) {
        if (fields.length >= FIELDS) return;
        if (typeof v === "string") fields.push({ name: str(k), value: clip(v, FVALUE) });
        else fields.push({ name: str(k), file: { name: str(v && v.name), size: v && v.size, type: str(v && v.type) } });
      });
      body(rid, "req", null, { kind: "formdata",
        note: "multipart/form-data: the engine encodes the parts itself, so the fields are shown instead of the bytes" });
      return { kind: "formdata", fields: fields, implied: "multipart/form-data" };
    }
    if (typeof BL === "function" && b instanceof BL) {
      var size = b.size, type = str(b.type), sent = ct || type;
      if (size <= SMALL_BLOB && typeof b.arrayBuffer === "function") {
        b.arrayBuffer().then(
          function (ab) { body(rid, "req", head(ab, CAP), { kind: "blob", total: size, complete: true, mime: sent }); },
          function (e) { body(rid, "req", null, { kind: "blob", total: size, mime: sent, note: "reading the Blob failed: " + message(e) }); });
      } else {
        body(rid, "req", null, { kind: "blob", total: size, mime: sent,
          note: "a Blob of " + size + " bytes is not read: the tee reads request Blobs up to 64 KB" });
      }
      return { kind: "blob", implied: type || undefined };
    }
    if (isBytes(b)) {
      body(rid, "req", head(b, CAP), { kind: "binary", total: b.byteLength, complete: b.byteLength <= CAP, mime: ct || undefined });
      return { kind: "binary" };
    }
    if (typeof b.getReader === "function") {
      body(rid, "req", null, { kind: "stream", note: "a ReadableStream body: this runtime refuses those, see the error" });
      return { kind: "stream" };
    }
    return textBody(rid, str(b), "text/plain;charset=UTF-8", "text", ct);
  };

  var readCapped = function (resp) {
    var s = null;
    try { s = resp.body; } catch (e) {}
    if (!s || typeof s.getReader !== "function") {
      return resp.arrayBuffer().then(function (ab) {
        return { bytes: head(ab, CAP), seen: ab.byteLength, complete: ab.byteLength <= CAP };
      });
    }
    var reader = s.getReader(), parts = [], kept = 0, seen = 0;
    var step = function () {
      return reader.read().then(function (r) {
        if (r.done) return true;
        var c = r.value;
        if (!c || typeof c.byteLength !== "number") return step();
        if (!(c instanceof Uint8Array)) c = head(c, c.byteLength);
        seen += c.byteLength;
        var take = Math.min(c.byteLength, CAP - kept);
        if (take > 0) { parts.push(take === c.byteLength ? c : c.subarray(0, take)); kept += take; }
        if (kept >= CAP) {
          try { var q = reader.cancel(); if (q && typeof q.then === "function") q.then(noop, noop); } catch (e) {}
          return false;
        }
        return step();
      });
    };
    return step().then(function (complete) {
      var out = new Uint8Array(kept), at = 0;
      for (var i = 0; i < parts.length; i++) { out.set(parts[i], at); at += parts[i].byteLength; }
      return { bytes: out, seen: seen, complete: complete };
    });
  };
  var captureBody = function (rid, res, t0, method, status, ct, cl) {
    var skip = "";
    if (method === "HEAD") skip = "a HEAD response has no body";
    else if (status === 204 || status === 205 || status === 304) skip = "a " + status + " response has no body";
    else if (/^\s*text\/event-stream/i.test(ct)) skip = "an event stream is not read: it may never end";
    else if (/^\s*(image|audio|video)\//i.test(ct)) skip = "media bodies are not captured";
    else if (cl > READ_LIMIT) skip = "not read: reading a clone copies the whole body into the JS heap first, and this one declares " + cl + " bytes (the tee reads up to 16 MB)";
    if (skip) {
      body(rid, "res", null, { note: skip, total: cl >= 0 ? cl : undefined, mime: ct });
      return;
    }
    var copy;
    try { copy = res.clone(); } catch (e) {
      body(rid, "res", null, { note: "Response.clone() threw, so the body was left alone: " + message(e), mime: ct });
      return;
    }
    var p;
    try { p = readCapped(copy); } catch (e) { p = Promise.reject(e); }
    p.then(function (r) {
      body(rid, "res", r.bytes, { total: r.complete ? r.seen : (cl >= 0 ? cl : undefined),
                                  seen: r.seen, complete: r.complete, mime: ct });
      meta(rid, { timing: { bodyMs: now() - t0 } });
    }, function (e) {
      body(rid, "res", null, { note: "reading the clone failed: " + message(e), mime: ct });
    });
  };

  var Req = G.Request;
  var beginFetch = function (input, init, url, method, t0) {
    var isReq = false;
    try { isReq = typeof Req === "function" && input instanceof Req; } catch (e) {}
    var hs = init && init.headers != null ? pairs(init.headers) : isReq ? pairs(input.headers) : [];
    var pick = function (k, d) {
      var v = init && init[k] != null ? init[k] : isReq ? input[k] : undefined;
      return v == null ? d : str(v);
    };
    var credentials = pick("credentials", "same-origin");
    var rid = begin({ kind: "fetch", url: url, method: method, at: t0, jar: credentials === "include",
      request: { headers: hs, credentials: credentials, mode: pick("mode", "cors"),
                 redirect: pick("redirect", "follow"), cache: pick("cache", "default") } });
    if (!rid) return 0;
    var info = null, b = init ? init.body : undefined;
    if (b !== undefined && b !== null) info = describeBody(rid, b, headerOf(hs, "content-type"));
    else if (isReq && method !== "GET" && method !== "HEAD") {
      body(rid, "req", null, { kind: "request",
        note: "the body inside a Request object is not read: its size is unknown until it has been copied whole" });
      info = { kind: "request" };
    }
    if (info) {
      if (info.implied && headerOf(hs, "content-type")) info.implied = undefined;
      meta(rid, { request: { bodyInfo: info } });
    }
    return rid;
  };
  var onResponse = function (rid, res, t0, method, url) {
    var hs = pairs(res.headers), status = res.status;
    meta(rid, { response: { status: status, statusText: str(res.statusText), type: str(res.type),
                            redirected: !!res.redirected, url: str(res.url), headers: hs },
                transport: transport(url), timing: { headersMs: now() - t0 }, jarAfter: true });
    var clh = headerOf(hs, "content-length"), cl = clh ? Number(clh) : -1;
    captureBody(rid, res, t0, method, status, headerOf(hs, "content-type"), cl >= 0 ? cl : -1);
  };

  var origFetch = G.fetch;
  if (typeof origFetch === "function" && !origFetch.__skTeed) {
    // `function fetch(input)`: the spec's `fetch.length` is 1.
    var teed = function fetch(input) {
      var init = arguments.length > 1 ? arguments[1] : undefined;
      var t0 = now();
      var url = "";
      var method = "GET";
      try {
        url = typeof input === "string" ? input : (input && input.url) || String(input);
        method = (init && init.method) || (input && input.method) || "GET";
      } catch (e) {}
      var M = str(method).toUpperCase();
      var p;
      try {
        p = origFetch.apply(this, arguments);
      } catch (e) {
        // A synchronous throw (a bad URL) must stay synchronous: record and rethrow.
        record({ kind: "fetch", url: url, method: M, at: t0, ms: 0, error: message(e) });
        throw e;
      }
      // AFTER the call: the engine has read `input` and `init`, so nothing read here changes what was sent
      // (a one-shot headers iterable is already consumed).
      var rid = 0;
      try { rid = beginFetch(input, init, url, M, t0); } catch (e) { rid = 0; }
      if (!p || typeof p.then !== "function") return p;
      return p.then(
        function (res) {
          // Connection facts first, so the host can put protocol and connection id on the row.
          if (rid && res && typeof hConn === "function") { try { hConn(rid, res); } catch (e) {} }
          record({ kind: "fetch", url: url, method: M, at: t0,
                   ms: now() - t0, status: res && res.status, ok: !!(res && res.ok),
                   type: (res && res.headers && res.headers.get && res.headers.get("content-type")) || undefined,
                   rid: rid || undefined });
          if (rid && res) { try { onResponse(rid, res, t0, M, url); } catch (e) {} }
          return res;
        },
        function (err) {
          record({ kind: "fetch", url: url, method: M, at: t0,
                   ms: now() - t0, error: message(err), rid: rid || undefined });
          meta(rid, { error: { name: str(err && err.name), message: message(err) }, timing: { failedMs: now() - t0 } });
          throw err;
        },
      );
    };
    teed.__skTeed = true;
    G.fetch = teed;
  }

  // A same-realm reload runs a new runtime whose proxy sits over `__SK_NATIVE_SCHED__.WebSocket`, the
  // class the first runtime found. If the devtools were enabled late that is the engine's class and the
  // next proxy would drop the tee, so that record is wrapped too.
  var OrigWS = G.WebSocket;
  if (typeof OrigWS === "function" && !OrigWS.__skTeed) {
    var socks = typeof WeakMap === "function" ? new WeakMap() : null;
    var count = function (st, received, data) {
      if (received) st.received++; else st.sent++;
      if (st.rid && typeof hWs === "function") { try { hWs(st.rid, received ? 1 : 0, sizeOf(data)); } catch (e) {} }
    };
    // Sends are counted on the prototype, so no own `send` shows on a socket.
    var WP = OrigWS.prototype, origSend = WP && WP.send, onProto = false;
    if (typeof origSend === "function" && socks) {
      try {
        var sendTee = function send(data) {
          var r = origSend.apply(this, arguments);
          var st = socks.get(this);
          if (st) count(st, false, data);
          return r;
        };
        WP.send = sendTee;
        onProto = WP.send === sendTee;
      } catch (e) {}
    }
    // Listeners go through EventTarget's own method: a runtime proxy's per-socket `addEventListener` records
    // listeners that the runtime removes before closing sockets on reload, and the tee must still hear that close.
    var listenOn = WP && typeof WP.addEventListener === "function" ? WP.addEventListener : null;
    var wsListen = function (ws, type, fn) {
      if (listenOn) listenOn.call(ws, type, fn); else ws.addEventListener(type, fn);
    };
    // One wrapper per class wrapped; they share `socks` and the prototype's `send`, so a socket is counted once.
    var wrapWS = function (Orig) {
      var W = function WebSocket(url, protocols) {
        var t0 = now();
        // Exactly the arguments given: an explicit `undefined` for `protocols` is not the same call.
        var ws = arguments.length > 1 ? new Orig(url, protocols) : new Orig(url);
        teeSocket(ws, t0, url, arguments.length > 1, protocols);
        // A constructor that returns an object makes `new W()` yield it: the caller gets the real socket.
        return ws;
      };
      W.prototype = Orig.prototype;
      for (var k in Orig) { try { W[k] = Orig[k]; } catch (e) {} }
      for (var i = 0, names = ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]; i < names.length; i++) {
        try { W[names[i]] = Orig[names[i]]; } catch (e) {}
      }
      W.__skTeed = true;
      return W;
    };
    var teeSocket = function (ws, t0, url, hasProtocols, protocols) {
      var u = String(url);
      try { if (ws.url) u = String(ws.url); } catch (e) {}
      var st = { rid: 0, sent: 0, received: 0 };
      try {
        var asked = hasProtocols ? (isArray(protocols) ? protocols.map(str) : [str(protocols)]) : [];
        st.rid = begin({ kind: "ws", url: u, at: t0, jar: true, ws: { protocols: asked } });
        if (socks) socks.set(ws, st);
      } catch (e) {}
      var rid = st.rid;
      record({ kind: "ws", url: u, at: t0, phase: "open-requested", rid: rid || undefined });
      try {
        wsListen(ws, "open", function () {
          record({ kind: "ws", url: u, at: now(), ms: now() - t0, phase: "open", rid: rid || undefined });
          meta(rid, { ws: { protocol: str(ws.protocol || ""), extensions: str(ws.extensions || ""),
                            binaryType: str(ws.binaryType || ""), openMs: now() - t0 }, jarAfter: true });
        });
        wsListen(ws, "message", function (e) {
          // Counted, not logged: a chatty socket would evict every other row.
          count(st, true, e && e.data);
        });
        wsListen(ws, "error", function () {
          record({ kind: "ws", url: u, at: now(), phase: "error",
                   sent: st.sent, received: st.received, rid: rid || undefined });
          meta(rid, { ws: { error: true, errorMs: now() - t0 } });
        });
        wsListen(ws, "close", function (e) {
          record({ kind: "ws", url: u, at: now(), ms: now() - t0, phase: "close",
                   status: e && e.code, sent: st.sent, received: st.received, rid: rid || undefined });
          meta(rid, { ws: { closeCode: e && e.code, closeReason: str((e && e.reason) || ""),
                            wasClean: !!(e && e.wasClean), closeMs: now() - t0 } });
        });
        if (!onProto && typeof ws.send === "function") {
          // The prototype would not take the wrapper; count on the instance.
          var own = ws.send;
          ws.send = function send(data) { var r = own.apply(this, arguments); count(st, false, data); return r; };
        }
      } catch (e) {}
    };
    // The runtime's record first; it exists only once a runtime has run.
    var NS = G.__SK_NATIVE_SCHED__;
    if (NS && typeof NS.WebSocket === "function" && !NS.WebSocket.__skTeed) {
      try { NS.WebSocket = wrapWS(NS.WebSocket); } catch (e) {}
    }
    var Wrapped = wrapWS(OrigWS);
    // So that `ws.constructor === WebSocket` holds for the global the app sees.
    try { if (WP && WP.constructor === OrigWS) WP.constructor = Wrapped; } catch (e) {}
    G.WebSocket = Wrapped;
  }

  // Only where the engine has one: three prototype methods and a `loadend` listener per request.
  var XHR = G.XMLHttpRequest;
  var XP = typeof XHR === "function" ? XHR.prototype : null;
  if (deep && XP && typeof XP.open === "function" && typeof XP.send === "function" &&
      !XP.send.__skTeed && typeof WeakMap === "function") {
    var xOpen = XP.open, xSend = XP.send, xSet = XP.setRequestHeader;
    var xs = new WeakMap(), heard = new WeakMap();
    var finish = function (x, s) {
      var status = 0, ct = "";
      try { status = x.status || 0; } catch (e) {}
      try { ct = str(x.getResponseHeader("content-type") || ""); } catch (e) {}
      var ms = now() - s.t0;
      record({ kind: "xhr", url: s.url, method: s.method, at: s.t0, ms: ms,
               status: status || undefined, ok: status >= 200 && status < 300, type: ct || undefined,
               error: status ? undefined : (s.failure || "failed"), rid: s.rid || undefined });
      if (!s.rid) return;
      var hs = [], rt = "", finalUrl = "", statusText = "";
      try { hs = rawPairs(x.getAllResponseHeaders()); } catch (e) {}
      try { rt = str(x.responseType || ""); } catch (e) {}
      try { finalUrl = str(x.responseURL || ""); } catch (e) {}
      try { statusText = str(x.statusText || ""); } catch (e) {}
      meta(s.rid, { response: { status: status, statusText: statusText, url: finalUrl, headers: hs, responseType: rt },
                    timing: { headersMs: ms }, jarAfter: true,
                    error: status ? undefined : { message: s.failure || "failed" } });
      if (!status) return;
      try {
        if (rt === "" || rt === "text") {
          var t = str(x.responseText);
          body(s.rid, "res", t.length > CAP ? t.slice(0, CAP) : t,
               { total: t.length, unit: "chars", complete: t.length <= CAP, mime: ct });
        } else if (rt === "json") {
          var j = stringify(x.response);
          j = j === undefined ? "" : j;
          body(s.rid, "res", j.length > CAP ? j.slice(0, CAP) : j,
               { total: j.length, unit: "chars", complete: j.length <= CAP, mime: ct || "application/json",
                 note: "responseType \"json\": re-serialized from the parsed value" });
        } else if (rt === "arraybuffer") {
          var ab = x.response;
          body(s.rid, "res", ab ? head(ab, CAP) : null,
               { total: ab ? ab.byteLength : 0, complete: !ab || ab.byteLength <= CAP, mime: ct });
        } else if (rt === "blob") {
          var bl = x.response;
          if (bl && bl.size <= SMALL_BLOB && typeof bl.arrayBuffer === "function") {
            bl.arrayBuffer().then(
              function (a) { body(s.rid, "res", head(a, CAP), { total: bl.size, complete: true, mime: ct }); },
              function (e) { body(s.rid, "res", null, { note: "reading the Blob failed: " + message(e), mime: ct }); });
          } else {
            body(s.rid, "res", null, { total: bl ? bl.size : 0, mime: ct,
              note: "a Blob response over 64 KB is not read" });
          }
        } else {
          body(s.rid, "res", null, { mime: ct, note: "responseType \"" + rt + "\" is not captured" });
        }
      } catch (e) {
        body(s.rid, "res", null, { mime: ct, note: "reading the response failed: " + message(e) });
      }
    };
    var listen = function (x) {
      var fail = function (why) { return function () { var s = xs.get(x); if (s) s.failure = why; }; };
      x.addEventListener("error", fail("network error"));
      x.addEventListener("abort", fail("aborted"));
      x.addEventListener("timeout", fail("timed out"));
      x.addEventListener("loadend", function () {
        var s = xs.get(x);
        if (!s || s.done || !s.t0) return;
        s.done = true;
        try { finish(x, s); } catch (e) {}
      });
    };
    var openTee = function open(method, url) {
      try { xs.set(this, { method: str(method).toUpperCase(), url: str(url), headers: [], rid: 0, t0: 0, failure: "", done: false }); } catch (e) {}
      return xOpen.apply(this, arguments);
    };
    // `function send()`: the spec's `send.length` is 0.
    var sendTee = function send() {
      var x = this, s = null;
      try { s = xs.get(x); } catch (e) {}
      if (s && !s.t0) {
        try {
          if (!heard.get(x)) { heard.set(x, true); listen(x); }
          s.t0 = now();
          var cred = false;
          try { cred = !!x.withCredentials; } catch (e) {}
          s.rid = begin({ kind: "xhr", url: s.url, method: s.method, at: s.t0, jar: cred,
                          request: { headers: s.headers, credentials: cred ? "include" : "same-origin" } });
          var b = arguments.length ? arguments[0] : undefined;
          if (s.rid && b != null && s.method !== "GET" && s.method !== "HEAD") {
            var info = describeBody(s.rid, b, headerOf(s.headers, "content-type"));
            if (info) {
              if (info.implied && headerOf(s.headers, "content-type")) info.implied = undefined;
              meta(s.rid, { request: { bodyInfo: info } });
            }
          }
        } catch (e) {}
      }
      return xSend.apply(this, arguments);
    };
    sendTee.__skTeed = true;
    try {
      XP.open = openTee;
      XP.send = sendTee;
      if (typeof xSet === "function") {
        XP.setRequestHeader = function setRequestHeader(name, value) {
          var r = xSet.apply(this, arguments);
          try { var s = xs.get(this); if (s && s.headers.length < HEADERS) s.headers.push([str(name), clip(str(value), HVALUE)]); } catch (e) {}
          return r;
        };
      }
    } catch (e) {}
  }
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(ring: &mut Ring, rid: Option<u64>) -> u64 {
        let raw = match rid {
            Some(r) => format!("{{\"kind\":\"fetch\",\"url\":\"https://a.test/x\",\"rid\":{r}}}"),
            None => "{\"kind\":\"fetch\",\"url\":\"https://a.test/x\"}".to_string(),
        };
        ring.record(&raw)
    }

    fn begun(ring: &mut Ring, kind: &str) -> u64 {
        let meta = json::parse(&format!(
            "{{\"kind\":\"{kind}\",\"url\":\"https://a.test/x\"}}"
        ))
        .unwrap();
        ring.begin(meta, 40, None)
    }

    fn body(bytes: Vec<u8>, meta: &str) -> Body {
        let meta = match json::parse(meta) {
            Some(Json::Obj(o)) => o,
            _ => Vec::new(),
        };
        Body {
            bytes,
            text: false,
            cut: false,
            meta,
            evicted: false,
        }
    }

    #[test]
    fn a_row_finds_its_record_through_its_rid() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        let seq = summary(&mut ring, Some(rid));
        ring.merge_meta(
            rid,
            json::parse(r#"{"response":{"status":201}}"#).unwrap(),
            30,
        );
        ring.attach_body(
            rid,
            Which::Res,
            body(
                b"{\"a\":1}".to_vec(),
                r#"{"mime":"application/json","complete":true}"#,
            ),
        );
        let d = ring.detail(seq).unwrap();
        assert_eq!(
            d.get("entry")
                .and_then(|e| e.get("seq"))
                .and_then(Json::as_f64),
            Some(seq as f64)
        );
        let res = d.get("response").unwrap();
        assert_eq!(res.get("status").and_then(Json::as_f64), Some(201.0));
        let b = res.get("body").unwrap();
        assert_eq!(b.get("encoding").and_then(Json::as_str), Some("utf8"));
        assert_eq!(b.get("data").and_then(Json::as_str), Some("{\"a\":1}"));
        assert_eq!(b.get("truncated").and_then(Json::as_bool), Some(false));
    }

    #[test]
    fn a_reload_marker_is_a_row_with_nothing_behind_it() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        let before = summary(&mut ring, Some(rid));
        let marker =
            ring.record(r#"{"kind":"reload","url":"","cause":"location.reload()","at":1}"#);
        assert_eq!(marker, before + 1, "the seqs run on through a reload");
        let d = ring.detail(marker).unwrap();
        assert!(matches!(d.get("detail"), Some(Json::Null)));
        assert_eq!(
            ring.detail(before)
                .unwrap()
                .get("rid")
                .and_then(Json::as_f64),
            Some(rid as f64)
        );
    }

    #[test]
    fn the_connection_rides_the_row_and_answers_transport_and_raw_set_cookie() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        let conn = Conn {
            protocol: "h2",
            reused: true,
            id: 7,
            remote: Some("93.184.215.14:443".into()),
            cache: "off",
        };
        ring.conn(
            rid,
            conn,
            vec!["sid=abc; Path=/; HttpOnly".into(), "theme=dark".into()],
        );
        let seq = summary(&mut ring, Some(rid));
        let row = json::parse(&ring.entries.back().unwrap().1).unwrap();
        assert_eq!(row.get("proto").and_then(Json::as_str), Some("h2"));
        assert_eq!(row.get("conn").and_then(Json::as_f64), Some(7.0));
        assert_eq!(row.get("reused").and_then(Json::as_bool), Some(true));
        assert_eq!(row.get("seq").and_then(Json::as_f64), Some(seq as f64));
        let d = ring.detail(seq).unwrap();
        let t = d.get("transport").unwrap();
        assert_eq!(t.get("protocol").and_then(Json::as_str), Some("h2"));
        assert_eq!(t.get("connectionId").and_then(Json::as_f64), Some(7.0));
        assert_eq!(
            t.get("remote").and_then(Json::as_str),
            Some("93.184.215.14:443")
        );
        let c = d.get("cookies").unwrap();
        assert_eq!(c.get("sent"), Some(&Json::Null));
        let wire = match c.get("wire") {
            Some(Json::Arr(a)) => a.iter().filter_map(Json::as_str).collect::<Vec<_>>(),
            _ => panic!("no wire cookies"),
        };
        assert_eq!(wire, ["sid=abc; Path=/; HttpOnly", "theme=dark"]);
    }

    #[test]
    fn a_disk_cache_hit_has_no_connection_columns_and_says_where_it_came_from() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        ring.conn(
            rid,
            Conn {
                protocol: "",
                reused: false,
                id: 0,
                remote: None,
                cache: "hit",
            },
            vec![],
        );
        let seq = summary(&mut ring, Some(rid));
        let row = json::parse(&ring.entries.back().unwrap().1).unwrap();
        assert_eq!(row.get("cache").and_then(Json::as_str), Some("hit"));
        assert!(row.get("proto").is_none() && row.get("conn").is_none());
        let t = ring.detail(seq).unwrap();
        assert_eq!(
            t.get("transport")
                .and_then(|t| t.get("cache"))
                .and_then(Json::as_str),
            Some("hit")
        );
        let c = Conn {
            protocol: "h2",
            reused: true,
            id: 3,
            remote: None,
            cache: "revalidated",
        };
        assert_eq!(
            c.columns(),
            "\"proto\":\"h2\",\"conn\":3,\"reused\":true,\"cache\":\"revalidated\""
        );
        let off = Conn {
            protocol: "h2",
            reused: false,
            id: 4,
            remote: None,
            cache: "off",
        };
        assert!(!off.columns().contains("cache"));
    }

    #[test]
    fn raw_set_cookie_lines_are_capped_per_line_and_per_response() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        let conn = Conn {
            protocol: "http/1.1",
            reused: false,
            id: 1,
            remote: None,
            cache: "off",
        };
        let huge = format!("big={}", "x".repeat(10_000));
        let lines: Vec<String> = std::iter::once(huge)
            .chain((0..100).map(|i| format!("c{i}=v")))
            .collect();
        ring.conn(rid, conn, lines);
        let r = &ring.records[ring.index_of(rid).unwrap()];
        let wire = r.wire_cookies.as_ref().unwrap();
        assert_eq!(wire[0].len(), WIRE_LINE_CAP);
        assert!(wire.len() <= SET_COOKIE_LINES_CAP);
        assert!(wire.iter().map(String::len).sum::<usize>() <= WIRE_COOKIES_CAP);
        let other = begun(&mut ring, "fetch");
        summary(&mut ring, Some(other));
        assert!(!ring.entries.back().unwrap().1.contains("\"proto\""));
    }

    #[test]
    fn a_row_without_a_record_says_so_rather_than_failing() {
        let mut ring = Ring::new();
        let seq = summary(&mut ring, None);
        let d = ring.detail(seq).unwrap();
        assert_eq!(d.get("detail"), Some(&Json::Null));
        assert!(ring.detail(seq + 10).is_err());
    }

    #[test]
    fn meta_merges_two_levels_deep() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        ring.merge_meta(
            rid,
            json::parse(r#"{"timing":{"headersMs":12}}"#).unwrap(),
            30,
        );
        ring.merge_meta(rid, json::parse(r#"{"timing":{"bodyMs":20}}"#).unwrap(), 30);
        let seq = summary(&mut ring, Some(rid));
        let t = ring.detail(seq).unwrap();
        let t = t.get("timing").unwrap();
        assert_eq!(t.get("headersMs").and_then(Json::as_f64), Some(12.0));
        assert_eq!(t.get("bodyMs").and_then(Json::as_f64), Some(20.0));
    }

    #[test]
    fn a_huge_meta_update_is_refused_but_small_ones_still_land() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        let big = format!("{{\"request\":{{\"x\":\"{}\"}}}}", "y".repeat(META_CAP));
        ring.merge_meta(rid, json::parse(&big).unwrap(), big.len());
        ring.merge_meta(
            rid,
            json::parse(r#"{"timing":{"headersMs":3}}"#).unwrap(),
            30,
        );
        let seq = summary(&mut ring, Some(rid));
        let d = ring.detail(seq).unwrap();
        assert_eq!(d.get("metaTruncated").and_then(Json::as_bool), Some(true));
        assert!(d.get("request").and_then(|r| r.get("x")).is_none());
        assert_eq!(
            d.get("timing")
                .and_then(|t| t.get("headersMs"))
                .and_then(Json::as_f64),
            Some(3.0)
        );
    }

    #[test]
    fn bodies_stay_inside_the_budget_oldest_evicted_first() {
        let mut ring = Ring::new();
        let mut seqs = Vec::new();
        // 60 bodies at the cap = 15 MiB > the 12 MiB budget.
        for _ in 0..60 {
            let rid = begun(&mut ring, "fetch");
            seqs.push(summary(&mut ring, Some(rid)));
            ring.attach_body(
                rid,
                Which::Res,
                body(vec![b'a'; BODY_CAP], r#"{"mime":"text/plain"}"#),
            );
            assert!(ring.held <= BODY_BUDGET, "held {} > budget", ring.held);
        }
        assert_eq!(ring.held, (BODY_BUDGET / BODY_CAP) * BODY_CAP);
        assert_eq!(ring.evicted, 60 - (BODY_BUDGET / BODY_CAP) as u64);
        let oldest = ring.detail(seqs[0]).unwrap();
        let b = oldest.get("response").and_then(|r| r.get("body")).unwrap();
        assert_eq!(b.get("evicted").and_then(Json::as_bool), Some(true));
        assert!(b.get("data").is_none());
        let newest = ring.detail(*seqs.last().unwrap()).unwrap();
        let b = newest.get("response").and_then(|r| r.get("body")).unwrap();
        assert_eq!(
            b.get("stored").and_then(Json::as_f64),
            Some(BODY_CAP as f64)
        );
    }

    #[test]
    fn a_record_leaves_with_the_last_row_that_names_it() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "fetch");
        summary(&mut ring, Some(rid));
        ring.attach_body(rid, Which::Res, body(vec![1; 1000], "{}"));
        assert_eq!(ring.held, 1000);
        for _ in 0..CAPACITY {
            summary(&mut ring, None);
        }
        assert!(ring.records.is_empty());
        assert_eq!(ring.held, 0);
        assert!(ring.by_seq.is_empty());
    }

    #[test]
    fn requests_that_never_finish_are_capped() {
        let mut ring = Ring::new();
        for _ in 0..(OPEN_CAP + 20) {
            begun(&mut ring, "fetch");
        }
        assert_eq!(ring.records.len(), OPEN_CAP);
    }

    #[test]
    fn a_socket_is_counted_across_its_rows() {
        let mut ring = Ring::new();
        let rid = begun(&mut ring, "ws");
        let first = summary(&mut ring, Some(rid));
        ring.ws_count(rid, false, 10.0, 1.0);
        ring.ws_count(rid, true, 7.0, 2.0);
        ring.ws_count(rid, true, 3.0, 3.0);
        let last = summary(&mut ring, Some(rid));
        for seq in [first, last] {
            let d = ring.detail(seq).unwrap();
            let ws = d.get("ws").unwrap();
            assert_eq!(ws.get("sent").and_then(Json::as_f64), Some(1.0));
            assert_eq!(ws.get("received").and_then(Json::as_f64), Some(2.0));
            assert_eq!(ws.get("receivedBytes").and_then(Json::as_f64), Some(10.0));
            assert!(
                d.get("request").is_none(),
                "a socket has no request body slot"
            );
            assert_eq!(
                d.get("seqs").and_then(Json::as_arr).map(<[Json]>::len),
                Some(2)
            );
        }
    }

    #[test]
    fn binary_media_types_are_base64_even_when_they_decode() {
        assert_eq!(encode(b"abc", "application/ion", false).0, "base64");
        assert_eq!(encode(&[0xe0, 0x01, 0x00, 0xea], "", false).0, "base64");
        assert_eq!(
            encode(b"{\"a\":1}", "application/json; charset=utf-8", false).0,
            "utf8"
        );
        assert_eq!(encode(b"plain words", "", false).0, "utf8");
        assert_eq!(encode(b"a\x00b", "", false).0, "base64");
        // A cut through a multi-byte character keeps the text, minus the tail.
        let cut = &"hé".as_bytes()[..2];
        assert_eq!(encode(cut, "text/plain", false), ("utf8", "h".to_string()));
        assert_eq!(encode(b"\x01\x02", "", true).0, "utf8");
    }

    #[test]
    fn strings_are_capped_on_a_character_boundary() {
        let s = "é".repeat(BODY_CAP); // 2 bytes each
        let (b, cut) = cap_string(s);
        assert!(cut);
        assert!(b.len() <= BODY_CAP);
        assert!(std::str::from_utf8(&b).is_ok());
        assert_eq!(cap_string("short".into()), (b"short".to_vec(), false));
    }

    fn cookie(name: &str, value: &str, path: &str, creation: i64) -> cookies::Cookie {
        cookies::Cookie {
            name: name.into(),
            value: value.into(),
            domain: "argon.test".into(),
            path: path.into(),
            host_only: true,
            secure: true,
            http_only: true,
            same_site: cookies::SameSite::None,
            expires: Some(1_445_412_480),
            creation,
            last_access: creation,
        }
    }

    #[test]
    fn the_cookie_header_is_longest_path_then_oldest() {
        let jar = vec![
            cookie("a", "1", "/", 2),
            cookie("b", "2", "/api", 3),
            cookie("c", "3", "/", 1),
        ];
        assert_eq!(cookie_header(&jar).as_deref(), Some("b=2; c=3; a=1"));
        assert_eq!(cookie_header(&[]), None);
    }

    #[test]
    fn set_cookie_is_reconstructed_from_what_the_jar_changed() {
        let before: Vec<Print> = [
            cookie("keep", "1", "/", 1),
            cookie("rot", "old", "/", 2),
            cookie("gone", "x", "/", 3),
        ]
        .iter()
        .map(print)
        .collect();
        let after = vec![
            cookie("keep", "1", "/", 1),
            cookie("rot", "new", "/", 2),
            cookie("fresh", "y", "/", 4),
        ];
        let lines = set_cookie_diff(&before, &after);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("rot=new; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Secure; HttpOnly; SameSite=None"), "{lines:?}");
        assert!(lines[1].starts_with("fresh=y;"), "{lines:?}");
        assert!(
            lines[2].starts_with("gone=;") && lines[2].contains("deleted"),
            "{lines:?}"
        );
    }

    #[test]
    fn http_dates() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(1_445_412_480), "Wed, 21 Oct 2015 07:28:00 GMT");
        assert_eq!(http_date(951_782_400), "Tue, 29 Feb 2000 00:00:00 GMT");
    }

    #[test]
    fn splice_seq_keeps_the_object_flat() {
        assert_eq!(splice_seq("{\"a\":1}", 7, ""), "{\"seq\":7,\"a\":1}");
        assert_eq!(splice_seq("{}", 2, ""), "{\"seq\":2}");
        assert_eq!(
            splice_seq("{\"a\":1}", 3, "\"conn\":9"),
            "{\"seq\":3,\"conn\":9,\"a\":1}"
        );
        assert_eq!(splice_seq("{}", 4, "\"conn\":9"), "{\"seq\":4,\"conn\":9}");
        assert_eq!(rid_of("{\"rid\":4,\"x\":1}"), Some(4));
        assert_eq!(rid_of("{\"x\":1}"), None);
    }
}
