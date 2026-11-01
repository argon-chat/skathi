//! What the cascade knows and nobody outside it can measure: how many
//! selectors the matcher actually ran, and how `compute_node`'s milliseconds
//! divide between matching, custom properties, and folding.
//!
//! Before the rule index (`Engine::index`) the selector count was `elements ×
//! rules in the sheet` — a 25 KB Tailwind sheet against a 500-node tree is
//! ~51 K `matches_selector` calls per restyle. The counter is what turned
//! "bucketing should help" into a number, and it is what a regression in the
//! index would show up as first. The three timers are what proved the matcher
//! was NOT where the cascade's time went (research/reports/m11/
//! perf-restyle-rebuild.md §3).
//!
//! Everything accumulates per `compute_node` call — three `Cell` adds, not one
//! per selector — and is drained by `vn-tree`'s phase reporter.

use std::cell::Cell;

thread_local! {
    static RULES_TESTED: Cell<u64> = const { Cell::new(0) };
    static MATCH_MS: Cell<f32> = const { Cell::new(0.0) };
    static VARS_MS: Cell<f32> = const { Cell::new(0.0) };
    static FOLD_MS: Cell<f32> = const { Cell::new(0.0) };
    static SPLIT: Cell<Split> = const { Cell::new(Split { by_bucket: [0; 6], matched: 0 }) };
    static SHARE: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// One element asked the matched-properties cache (`share`), and whether an
/// earlier element's result answered it.
pub(crate) fn note_share(hit: bool) {
    SHARE.with(|c| {
        let (asked, hits) = c.get();
        c.set((asked + 1, hits + u64::from(hit)));
    });
}

/// Drains the cache counters: `(elements that asked, elements answered)`.
/// An element with an inline style or a `.group:hover` rule does not ask.
pub fn take_share() -> (u64, u64) {
    SHARE.with(|c| c.replace((0, 0)))
}

/// Where the selectors run came from, and how many of them matched: the rule
/// index's own report card. `by_bucket` is: rules no key could file (`any`,
/// run against every element), then the subject's tag, class and id buckets,
/// `:root`, and the parent-keyed buckets (crate `RuleIndex`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub by_bucket: [u64; 6],
    pub matched: u64,
}

/// One element's cascade: the selectors run per bucket, and how many matched.
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

/// Drains the [`Split`] counters.
pub fn take_split() -> Split {
    SPLIT.with(|c| c.replace(Split::default()))
}

/// One element's cascade: selectors run, then the three phases in milliseconds.
pub(crate) fn note_node(tested: u64, match_ms: f32, vars_ms: f32, fold_ms: f32) {
    RULES_TESTED.with(|c| c.set(c.get().wrapping_add(tested)));
    MATCH_MS.with(|c| c.set(c.get() + match_ms));
    VARS_MS.with(|c| c.set(c.get() + vars_ms));
    FOLD_MS.with(|c| c.set(c.get() + fold_ms));
}

/// Drains the counters: `(selectors run, match ms, vars ms, fold ms)`.
pub fn take() -> (u64, f32, f32, f32) {
    (
        RULES_TESTED.with(|c| c.replace(0)),
        MATCH_MS.with(|c| c.replace(0.0)),
        VARS_MS.with(|c| c.replace(0.0)),
        FOLD_MS.with(|c| c.replace(0.0)),
    )
}
