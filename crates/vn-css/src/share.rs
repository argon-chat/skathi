//! The matched-properties cache: elements that matched the same rules under the
//! same inherited inputs get one computed style, folded once.
//!
//! Everything [`crate::Engine::compute_node`] does after matching is a function
//! of the matched rules (in cascade order), the inline style, what the parent
//! hands down (its custom properties, `color` and font size), the environment the `@media`
//! conditions and `env()` read, and the rem and viewport bases. Two elements
//! that agree on all of them get equal `ComputedStates`, so the second is a clone
//! of the first: custom properties are not resolved again and the declarations
//! are not folded again. The match still runs for every element, and that is
//! what makes this safe against every selector without a per-selector argument:
//! a sibling, positional or attribute selector, or a `:has()`, that tells two
//! elements apart has already given them different rule lists. This is Blink's
//! MatchedPropertiesCache, not Servo's style sharing, which skips the match as
//! well and needs revalidation selectors and parents that shared to stay right.
//!
//! What is left out:
//! - **An inline style**: the cascade borrows it, so the cache could not hold it,
//!   and its address could be reused by a different one.
//! - **A `.group:hover .x` rule**: the fold keys its bucket by the ancestor it
//!   watches (`ComputedStates::group_hover`), which differs between elements
//!   with the same rules.
//!
//! The parent's custom properties are held by `Rc`, so an entry keeps its key
//! alive: an address in the cache is never a different map. They compare by
//! address first and by content after, so the children of two rows that
//! resolved the same custom properties separately still meet. A hit hands the
//! cached `vars` down, so the next generation finds the same address.
//!
//! In Argon's chat view, 40% of the elements have a same-keyed element under a
//! same-keyed parent and 8% a same-keyed earlier sibling
//! (research/reports/m26/fixes-lag-1.md §4.10). So the cache is a short ring of
//! recent results rather than a per-parent list.

use std::cell::RefCell;
use std::rc::Rc;

use crate::{ComputedStates, Env, Vars};

/// Recent results kept. A message row's subtree has a few dozen distinct
/// element kinds, and the next row is folded right after it.
const SLOTS: usize = 64;

/// Everything a fold depends on besides the declarations themselves.
pub(crate) struct Key<'a> {
    /// [`crate::Engine`]'s generation: rule indices mean nothing across sheets,
    /// and a viewport rebase rewrites declarations in place.
    generation: u64,
    hash: u64,
    /// The matched rules' indices, in cascade order.
    rules: Box<[u32]>,
    parent_vars: &'a Rc<Vars>,
    parent_color: Option<gpui::Hsla>,
    env: &'a Env,
    /// The rem basis and the viewport basis, bitwise: a `var()`-composed value
    /// resolves `rem` and `vw` against them at fold time. A new rem basis or a
    /// resize also recompiles or rebases the sheet, which moves the generation;
    /// this keeps the cache right where a resize skips the rebase (a sheet
    /// whose only viewport units sit inside custom properties). And the
    /// parent's font size: every `em` the element resolves, and its own font
    /// size when it inherits one, follow from it, so two cousins under
    /// different font sizes must not share.
    bases: [u32; 4],
}

impl<'a> Key<'a> {
    pub(crate) fn new(
        generation: u64,
        rules: Box<[u32]>,
        parent_vars: &'a Rc<Vars>,
        parent_color: Option<gpui::Hsla>,
        parent_font_size: f32,
        env: &'a Env,
    ) -> Self {
        // FNV-1a: a cheap first filter before the slices are compared.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &r in rules.iter() {
            for b in r.to_le_bytes() {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        let (w, h) = crate::mapper::viewport_basis();
        let bases = [crate::mapper::rem_basis().to_bits(), w.to_bits(), h.to_bits(), parent_font_size.to_bits()];
        Key { generation, hash, rules, parent_vars, parent_color, env, bases }
    }
}

struct Entry {
    generation: u64,
    hash: u64,
    rules: Box<[u32]>,
    parent_vars: Rc<Vars>,
    parent_color: Option<gpui::Hsla>,
    env: Env,
    bases: [u32; 4],
    states: ComputedStates,
}

impl Entry {
    fn answers(&self, key: &Key<'_>) -> bool {
        self.hash == key.hash
            && self.generation == key.generation
            && *self.rules == *key.rules
            && color_bits(self.parent_color) == color_bits(key.parent_color)
            && self.env == *key.env
            && self.bases == key.bases
            && (Rc::ptr_eq(&self.parent_vars, key.parent_vars) || *self.parent_vars == **key.parent_vars)
    }
}

/// Bitwise, so that a NaN channel still meets itself.
fn color_bits(c: Option<gpui::Hsla>) -> Option<[u32; 4]> {
    c.map(|c| [c.h.to_bits(), c.s.to_bits(), c.l.to_bits(), c.a.to_bits()])
}

#[derive(Default)]
struct Ring {
    entries: Vec<Entry>,
    next: usize,
}

thread_local! {
    static RING: RefCell<Ring> = RefCell::new(Ring::default());
}

/// The folded result of an earlier element with exactly these inputs, if one is kept.
pub(crate) fn lookup(key: &Key<'_>) -> Option<ComputedStates> {
    RING.with(|ring| ring.borrow().entries.iter().find(|e| e.answers(key)).map(|e| e.states.clone()))
}

/// Keeps a freshly folded result, in place of the oldest one.
pub(crate) fn insert(key: Key<'_>, states: &ComputedStates) {
    let entry = Entry {
        generation: key.generation,
        hash: key.hash,
        rules: key.rules,
        parent_vars: key.parent_vars.clone(),
        parent_color: key.parent_color,
        env: *key.env,
        bases: key.bases,
        states: states.clone(),
    };
    RING.with(|ring| {
        let mut ring = ring.borrow_mut();
        if ring.entries.len() < SLOTS {
            ring.entries.push(entry);
        } else {
            let at = ring.next;
            ring.entries[at] = entry;
        }
        ring.next = (ring.next + 1) % SLOTS;
    });
}

/// Is the cache on? `VN_CSS_NO_SHARE=1` turns it off, for an A/B or to rule it
/// out, the way `VN_CSS_NO_RULE_INDEX` does the index.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    match ON.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let off = std::env::var("VN_CSS_NO_SHARE")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            ON.store(if off { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
            !off
        }
    }
}
