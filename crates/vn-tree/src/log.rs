//! A bounded in-process log ring so the devtools `logs` command can read back what the host printed.
//! Off unless [`enable`] is called, so [`record`] is one relaxed atomic load. Output is teed here,
//! not moved: the terminal keeps showing exactly what it did. The ring is thread-local because every
//! recorder lives on the VM/gpui main thread.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Entries kept.
const CAPACITY: usize = 500;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Monotonic entry sequence — the `since` cursor `logs` pages on.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// One captured line.
#[derive(Clone, Debug)]
pub struct Entry {
    pub seq: u64,
    pub at_ms: u64,
    /// `log`, `warn`, `error`, `advisory`, `css`, `dev`, `js` — a coarse channel name, not a
    /// severity ladder.
    pub kind: &'static str,
    pub text: String,
    /// `console.log("%cA%cB", cssA, cssB)`'s style runs, kept as TEXT because the inspector re-parses
    /// it anyway. As the JSON the console tee built:
    /// `[{"at":0,"css":"…"},{"at":1,"css":"…"}]`, where `at` is an index into [`Entry::text`].
    pub styles: Option<String>,
}

thread_local! {
    static RING: RefCell<std::collections::VecDeque<Entry>> =
        RefCell::new(std::collections::VecDeque::new());
    static START: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// `__vn_set_status` is a one-line STATUS, not a log stream: the shell overwrites it, and the
    /// hello app's prelude rewrites it twice a second.
    static STATUS: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Collapses every run of whitespace to one space and trims the ends.
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
    }
    out
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

/// Turns the ring on.
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

/// `(entries in the ring, bytes of their text and style runs)` — devtools `mem`.
pub fn mem_stats() -> (usize, usize) {
    RING.with(|r| {
        let r = r.borrow();
        let bytes = r
            .iter()
            .map(|e| std::mem::size_of::<Entry>() + e.text.capacity() + e.styles.as_ref().map_or(0, String::capacity))
            .sum();
        (r.len(), bytes)
    })
}

/// Appends one line.
pub fn record(kind: &'static str, text: impl Into<String>) {
    record_styled(kind, text, None)
}

/// [`record`], plus the `%c` style runs a `console.*` call carried.
pub fn record_styled(kind: &'static str, text: impl Into<String>, styles: Option<String>) {
    if !is_enabled() {
        return;
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let at_ms = START.with(|s| s.get()).map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
    let entry = Entry { seq, at_ms, kind, text: text.into(), styles };
    RING.with(|r| {
        let mut r = r.borrow_mut();
        if r.len() == CAPACITY {
            r.pop_front();
        }
        r.push_back(entry);
    });
}

/// Every entry with `seq >= since`, oldest first, plus how many were dropped from the front before
/// `since` could be served, so a client with a stale cursor is told it missed lines.
pub fn since(since: u64) -> (Vec<Entry>, u64) {
    RING.with(|r| {
        let r = r.borrow();
        let oldest = r.front().map(|e| e.seq).unwrap_or(SEQ.load(Ordering::Relaxed));
        let missed = oldest.saturating_sub(since).min(u64::from(u32::MAX));
        let out: Vec<Entry> = r.iter().filter(|e| e.seq >= since).cloned().collect();
        (out, if since == 0 { 0 } else { missed })
    })
}

pub fn next_seq() -> u64 {
    SEQ.load(Ordering::Relaxed)
}
