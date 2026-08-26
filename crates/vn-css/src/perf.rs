//! The one number the cascade knows and nobody else can measure: how many
//! selectors the matcher actually ran.
//!
//! Before the rule index (`Engine::index`) that was `elements × rules in the
//! sheet` — a 25 KB Tailwind sheet against a 500-node tree is close to a
//! million `matches_selector` calls per mutation. The counter is what turns
//! "bucketing should help" into a number in the report, and it is what a
//! regression in the index would show up as first.
//!
//! Accumulated per `compute_node` call (one `Cell` add, not one per selector)
//! and drained by `vn-tree`'s phase reporter.

use std::cell::Cell;

thread_local! {
    static RULES_TESTED: Cell<u64> = const { Cell::new(0) };
}

/// Called once at the end of a cascade, with that element's selector count.
pub(crate) fn note_rules_tested(n: u64) {
    RULES_TESTED.with(|c| c.set(c.get().wrapping_add(n)));
}

/// Drains the counter.
pub fn take_rules_tested() -> u64 {
    RULES_TESTED.with(|c| c.replace(0))
}
