//! Matched-properties cache (Blink's, not Servo's style sharing): elements that match the same
//! rules under the same inherited inputs share one `ComputedStates`. The match still runs per
//! element, so every selector kind stays correct. Not cached: elements with an inline style, and
//! `.group:hover .x` rules whose bucket depends on the ancestor.

use std::cell::RefCell;
use std::rc::Rc;

use crate::{ComputedStates, Env, Vars};

const SLOTS: usize = 64;

pub(crate) struct Key<'a> {
    generation: u64,
    hash: u64,
    rules: Box<[u32]>,
    parent_vars: &'a Rc<Vars>,
    parent_color: Option<gpui::Hsla>,
    env: &'a Env,
    bases: [u32; 5],
    parent_font: &'a gpui::Font,
}

impl<'a> Key<'a> {
    pub(crate) fn new(
        generation: u64,
        rules: Box<[u32]>,
        parent_vars: &'a Rc<Vars>,
        parent_color: Option<gpui::Hsla>,
        parent_font_size: f32,
        parent_font: &'a gpui::Font,
        env: &'a Env,
    ) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &r in rules.iter() {
            for b in r.to_le_bytes() {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        let (w, h) = crate::mapper::viewport_basis();
        let bases = [
            crate::mapper::rem_basis().to_bits(),
            w.to_bits(),
            h.to_bits(),
            parent_font_size.to_bits(),
            crate::mapper::font_metric_generation(),
        ];
        Key {
            generation,
            hash,
            rules,
            parent_vars,
            parent_color,
            parent_font,
            env,
            bases,
        }
    }
}

struct Entry {
    generation: u64,
    hash: u64,
    rules: Box<[u32]>,
    parent_vars: Rc<Vars>,
    parent_color: Option<gpui::Hsla>,
    env: Env,
    /// Rem and viewport bases plus the parent's font size: `em` values and an inherited font size
    /// follow from it, so cousins under different font sizes must not share.
    bases: [u32; 5],
    parent_font: gpui::Font,
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
            && self.parent_font == *key.parent_font
            && (Rc::ptr_eq(&self.parent_vars, key.parent_vars)
                || *self.parent_vars == **key.parent_vars)
    }
}

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

pub(crate) fn lookup(key: &Key<'_>) -> Option<ComputedStates> {
    RING.with(|ring| {
        ring.borrow()
            .entries
            .iter()
            .find(|e| e.answers(key))
            .map(|e| e.states.clone())
    })
}

pub(crate) fn insert(key: Key<'_>, states: &ComputedStates) {
    let entry = Entry {
        generation: key.generation,
        hash: key.hash,
        rules: key.rules,
        parent_vars: key.parent_vars.clone(),
        parent_color: key.parent_color,
        env: *key.env,
        bases: key.bases,
        parent_font: key.parent_font.clone(),
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

pub(crate) fn enabled() -> bool {
    static ON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    match ON.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let off = std::env::var("SK_CSS_NO_SHARE")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            ON.store(
                if off { 1 } else { 2 },
                std::sync::atomic::Ordering::Relaxed,
            );
            !off
        }
    }
}
