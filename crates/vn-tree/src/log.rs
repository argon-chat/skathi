//! A bounded in-process log ring so `vn-devtools`' `logs` command can read
//! back what the host printed, without an agent having to own the process'
//! stdio.
//!
//! Everything the host already `eprintln!`s — compat advisories, CSS
//! diagnostics, `[js uncaught]` errors, the dev transport's messages — is
//! *teed* in here rather than moved: a developer watching the terminal must
//! keep seeing exactly what they saw before (compat-first: devtools adds a
//! second reader, it does not change the first one's output).
//!
//! Off by default. `vn-app`'s dev build calls [`enable`] at boot; the prod
//! build never does, so [`record`] is one relaxed atomic load and a return.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Entries kept. A wedged app printing an advisory per frame must not grow the
/// process; 500 is ~2 screens of `ctl logs` and a few tens of KB.
const CAPACITY: usize = 500;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Monotonic entry sequence — the `since` cursor `logs` pages on. Global
/// rather than per-ring so a dropped entry still advances it (a client that
/// asks for `since` older than the ring can be told it missed entries).
static SEQ: AtomicU64 = AtomicU64::new(0);

/// One captured line.
#[derive(Clone, Debug)]
pub struct Entry {
    pub seq: u64,
    /// Milliseconds since the ring was enabled — a wall clock would need a
    /// dependency and an agent only ever wants relative ordering.
    pub at_ms: u64,
    /// `log`, `warn`, `error`, `advisory`, `css`, `dev`, `js` — a coarse
    /// channel name, not a severity ladder.
    pub kind: &'static str,
    pub text: String,
}

thread_local! {
    static RING: RefCell<std::collections::VecDeque<Entry>> =
        RefCell::new(std::collections::VecDeque::new());
    static START: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// `__vn_set_status` is a one-line STATUS, not a log stream: the shell
    /// overwrites it, and the hello app's prelude rewrites it twice a second.
    /// Putting it in the ring would evict every real line within minutes, so it
    /// gets its own slot and `logs` reports the latest value alongside the ring.
    static STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn set_status(text: impl Into<String>) {
    if !is_enabled() {
        return;
    }
    STATUS.with(|s| *s.borrow_mut() = Some(text.into()));
}

pub fn status() -> Option<String> {
    STATUS.with(|s| s.borrow().clone())
}

/// Turns the ring on (dev builds only). Idempotent.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    START.with(|s| {
        if s.get().is_none() {
            s.set(Some(std::time::Instant::now()));
        }
    });
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Appends one line. Cheap no-op while disabled.
///
/// The ring is thread-local because everything that records lives on the VM /
/// gpui main thread; the devtools listener thread never calls this (it only
/// queues, and the command that reads the ring runs on the main thread).
pub fn record(kind: &'static str, text: impl Into<String>) {
    if !is_enabled() {
        return;
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let at_ms = START.with(|s| s.get()).map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
    let entry = Entry { seq, at_ms, kind, text: text.into() };
    RING.with(|r| {
        let mut r = r.borrow_mut();
        if r.len() == CAPACITY {
            r.pop_front();
        }
        r.push_back(entry);
    });
}

/// Every entry with `seq >= since`, oldest first, plus the number of entries
/// that were dropped from the front before `since` could be served (so a client
/// paging with a stale cursor is TOLD it missed lines rather than quietly
/// getting a gap).
pub fn since(since: u64) -> (Vec<Entry>, u64) {
    RING.with(|r| {
        let r = r.borrow();
        let oldest = r.front().map(|e| e.seq).unwrap_or(SEQ.load(Ordering::Relaxed));
        let missed = oldest.saturating_sub(since).min(u64::from(u32::MAX));
        let out: Vec<Entry> = r.iter().filter(|e| e.seq >= since).cloned().collect();
        (out, if since == 0 { 0 } else { missed })
    })
}

/// The sequence number the next entry will get — a client's "start from now".
pub fn next_seq() -> u64 {
    SEQ.load(Ordering::Relaxed)
}
