//! Counters for what the cascade does: selectors the matcher ran, and how `compute_node`'s time
//! divides between matching, custom properties and folding. Accumulated per `compute_node` call and
//! drained by the host's phase reporter.

use std::cell::Cell;

thread_local! {
    static RULES_TESTED: Cell<u64> = const { Cell::new(0) };
    static MATCH_MS: Cell<f32> = const { Cell::new(0.0) };
    static VARS_MS: Cell<f32> = const { Cell::new(0.0) };
    static FOLD_MS: Cell<f32> = const { Cell::new(0.0) };
    static SPLIT: Cell<Split> = const { Cell::new(Split { by_bucket: [0; 6], matched: 0 }) };
    static SHARE: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

pub(crate) fn note_share(hit: bool) {
    SHARE.with(|c| {
        let (asked, hits) = c.get();
        c.set((asked + 1, hits + u64::from(hit)));
    });
}

pub fn take_share() -> (u64, u64) {
    SHARE.with(|c| c.replace((0, 0)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub by_bucket: [u64; 6],
    pub matched: u64,
}

pub(crate) fn note_split(by_bucket: [u32; 6], matched: usize) {
    SPLIT.with(|c| {
        let mut s = c.get();
        for (total, n) in s.by_bucket.iter_mut().zip(by_bucket) {
            *total = total.wrapping_add(n as u64);
        }
        s.matched = s.matched.wrapping_add(matched as u64);
        c.set(s);
    });
}

pub fn take_split() -> Split {
    SPLIT.with(|c| c.replace(Split::default()))
}

pub(crate) fn note_node(tested: u64, match_ms: f32, vars_ms: f32, fold_ms: f32) {
    RULES_TESTED.with(|c| c.set(c.get().wrapping_add(tested)));
    MATCH_MS.with(|c| c.set(c.get() + match_ms));
    VARS_MS.with(|c| c.set(c.get() + vars_ms));
    FOLD_MS.with(|c| c.set(c.get() + fold_ms));
}

pub fn take() -> (u64, f32, f32, f32) {
    (
        RULES_TESTED.with(|c| c.replace(0)),
        MATCH_MS.with(|c| c.replace(0.0)),
        VARS_MS.with(|c| c.replace(0.0)),
        FOLD_MS.with(|c| c.replace(0.0)),
    )
}
