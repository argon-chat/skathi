//! `skathi:collections`: the structures whose contents are numbers, bits and strings, kept in Rust.
//! A `Trie` and an `IntervalTree` keep their JS values in a JS array and the index of each here.

use std::collections::BTreeMap;
use std::ffi::c_void;

use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};

const MAX_BITS: usize = 1 << 32;
const VALUES: &str = "skathi.values";

unsafe fn index(g: *mut c_void, v: u64, len: usize, what: &str) -> Js<usize> {
    unsafe {
        let i = count(g, v, what)?;
        if i >= len {
            return range_error(g, &format!("{what} {i} is outside 0..{len}"));
        }
        Ok(i)
    }
}

unsafe fn u32_array(g: *mut c_void, values: &[u32]) -> Js<u64> {
    unsafe {
        let (array, out) = typed_array(g, TA_U32, values.len())?;
        for (slot, value) in out.chunks_exact_mut(4).zip(values) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
        Ok(array)
    }
}

/// `array[Symbol.iterator]()`, so a native list is walked by the engine's own iterator.
unsafe fn iterate(g: *mut c_void, array: u64) -> Js<u64> {
    unsafe { call(g, get(g, array, "values"), array, &[]).map_err(|_| Thrown) }
}

unsafe fn iterable(g: *mut c_void, proto: u64, f: HostFn) {
    unsafe {
        let symbol = glue::rjs_atom_iterator(g);
        glue::rjs_define(g, proto, symbol, make_fn(g, "[Symbol.iterator]", 0, f), 2);
        glue::rjs_atom_free(g, symbol);
    }
}

unsafe fn values_slot(g: *mut c_void, this: u64) -> u64 {
    unsafe { slot_get(g, this, VALUES) }
}

struct BitSet {
    words: Vec<u64>,
    bits: usize,
}

host_state!(bit_set, bit_set_new, BitSet, BIT_SET, "BitSet");

impl BitSet {
    fn with_bits(bits: usize) -> BitSet {
        BitSet {
            words: vec![0; bits.div_ceil(64)],
            bits,
        }
    }

    fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    fn indices(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.count());
        for (at, word) in self.words.iter().enumerate() {
            let mut left = *word;
            while left != 0 {
                out.push((at * 64) as u32 + left.trailing_zeros());
                left &= left - 1;
            }
        }
        out
    }
}

std_fn!(fn bit_set_construct(g, args) {
    let bits = count(g, args.arg(0).0, "BitSet: the size")?;
    if bits > MAX_BITS {
        return range_error(g, "BitSet: the size is above 2^32 bits");
    }
    Ok(bit_set_new(g, BitSet::with_bits(bits)))
});

std_fn!(fn bit_set_set(g, args) {
    let this = args.this().0;
    let state = bit_set(g, this)?;
    let i = index(g, args.arg(0).0, state.bits, "BitSet.set: index")?;
    state.words[i / 64] |= 1 << (i % 64);
    Ok(this)
});

std_fn!(fn bit_set_clear(g, args) {
    let this = args.this().0;
    let state = bit_set(g, this)?;
    if args.arg(0).0 == UNDEF {
        state.words.fill(0);
        return Ok(UNDEF);
    }
    let i = index(g, args.arg(0).0, state.bits, "BitSet.clear: index")?;
    state.words[i / 64] &= !(1 << (i % 64));
    Ok(UNDEF)
});

std_fn!(fn bit_set_has(g, args) {
    let state = bit_set(g, args.this().0)?;
    let n = number(g, args.arg(0).0)?;
    let inside = n.fract() == 0.0 && n >= 0.0 && (n as usize) < state.bits;
    Ok(enc_bool(inside && state.words[n as usize / 64] >> (n as usize % 64) & 1 == 1))
});

std_fn!(fn bit_set_count(g, args) { Ok(enc_f64(bit_set(g, args.this().0)?.count() as f64)) });
std_fn!(fn bit_set_size(g, args) { Ok(enc_f64(bit_set(g, args.this().0)?.bits as f64)) });

unsafe fn bit_set_combine(
    g: *mut c_void,
    this: u64,
    other: u64,
    what: &str,
    op: fn(u64, u64) -> u64,
) -> Js<u64> {
    unsafe {
        let left = BitSet {
            words: bit_set(g, this)?.words.clone(),
            bits: bit_set(g, this)?.bits,
        };
        let right = bit_set(g, other)?;
        if left.bits != right.bits {
            return range_error(
                g,
                &format!(
                    "BitSet.{what}: the sets hold {} and {} bits; they must be the same size",
                    left.bits, right.bits
                ),
            );
        }
        let words = left
            .words
            .iter()
            .zip(&right.words)
            .map(|(a, b)| op(*a, *b))
            .collect();
        Ok(bit_set_new(
            g,
            BitSet {
                words,
                bits: left.bits,
            },
        ))
    }
}

std_fn!(fn bit_set_and(g, args) { bit_set_combine(g, args.this().0, args.arg(0).0, "and", |a, b| a & b) });
std_fn!(fn bit_set_or(g, args) { bit_set_combine(g, args.this().0, args.arg(0).0, "or", |a, b| a | b) });
std_fn!(fn bit_set_xor(g, args) { bit_set_combine(g, args.this().0, args.arg(0).0, "xor", |a, b| a ^ b) });
std_fn!(fn bit_set_and_not(g, args) { bit_set_combine(g, args.this().0, args.arg(0).0, "andNot", |a, b| a & !b) });

std_fn!(fn bit_set_indices(g, args) {
    let indices = bit_set(g, args.this().0)?.indices();
    u32_array(g, &indices)
});

std_fn!(fn bit_set_iterator(g, args) {
    let indices = bit_set(g, args.this().0)?.indices();
    iterate(g, u32_array(g, &indices)?)
});

struct BloomFilter {
    words: Vec<u64>,
    bits: u64,
    hashes: u32,
    added: f64,
}

host_state!(bloom, bloom_new, BloomFilter, BLOOM, "BloomFilter");

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325 ^ seed;
    for byte in bytes {
        hash = (hash ^ *byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^ (hash >> 33)
}

/// The bits and the hash count for `expected` items at `rate` false positives.
fn bloom_shape(expected: f64, rate: f64) -> (u64, u32) {
    let ln2 = std::f64::consts::LN_2;
    let bits = (-(expected * rate.ln()) / (ln2 * ln2)).ceil().max(64.0);
    let hashes = ((bits / expected) * ln2).round().clamp(1.0, 30.0);
    (bits as u64, hashes as u32)
}

impl BloomFilter {
    fn positions(&self, key: &[u8]) -> impl Iterator<Item = u64> + '_ + use<'_> {
        let first = fnv1a(key, 0);
        let second = fnv1a(key, 0x9e37_79b9_7f4a_7c15) | 1;
        (0..self.hashes as u64).map(move |i| first.wrapping_add(i.wrapping_mul(second)) % self.bits)
    }

    fn add(&mut self, key: &[u8]) {
        let positions: Vec<u64> = self.positions(key).collect();
        for bit in positions {
            self.words[(bit / 64) as usize] |= 1 << (bit % 64);
        }
        self.added += 1.0;
    }

    fn might_have(&self, key: &[u8]) -> bool {
        self.positions(key)
            .all(|bit| self.words[(bit / 64) as usize] >> (bit % 64) & 1 == 1)
    }
}

/// The bytes a value is hashed by: a string's text, a number's, a bigint's digits, a buffer's bytes.
unsafe fn hash_key(g: *mut c_void, v: u64, what: &str) -> Js<Vec<u8>> {
    unsafe {
        if let Some(view) = buffer_source(g, v) {
            return Ok(view.bytes().to_vec());
        }
        let tag = match kind(g, v) {
            KIND_STRING => b's',
            3 | 4 => b'n',
            KIND_BIGINT => b'i',
            _ => {
                return type_error(
                    g,
                    &format!("{what} must be a string, a number, a bigint or bytes"),
                )
            }
        };
        let text = to_js_string(g, v).map_err(|()| Thrown)?;
        let mut key = vec![tag];
        key.extend(read_string(g, text).unwrap_or_default().into_bytes());
        Ok(key)
    }
}

std_fn!(fn bloom_construct(g, args) {
    let expected = number(g, args.arg(0).0)?;
    let rate = match args.arg(1).0 {
        UNDEF => 0.01,
        v => number(g, v)?,
    };
    if !(expected >= 1.0 && expected <= 1e10) || !(rate > 0.0 && rate < 1.0) {
        return range_error(g, "BloomFilter: expected must be 1 or more, and the false positive rate between 0 and 1");
    }
    let (bits, hashes) = bloom_shape(expected, rate);
    if bits > MAX_BITS as u64 * 8 {
        return range_error(g, "BloomFilter: that many items at that rate need more than 4 GiB");
    }
    let state = BloomFilter { words: vec![0; bits.div_ceil(64) as usize], bits, hashes, added: 0.0 };
    Ok(bloom_new(g, state))
});

std_fn!(fn bloom_add(g, args) {
    let this = args.this().0;
    bloom(g, this)?;
    let key = hash_key(g, args.arg(0).0, "BloomFilter.add: the value")?;
    bloom(g, this)?.add(&key);
    Ok(this)
});

std_fn!(fn bloom_add_all(g, args) {
    let this = args.this().0;
    bloom(g, this)?;
    let list = args.arg(0).0;
    if !is_cell(list) {
        return type_error(g, "BloomFilter.addAll: the values must be an array");
    }
    for i in 0..array_len(g, list) {
        let key = hash_key(g, get_i(g, list, i), "BloomFilter.addAll: each value")?;
        bloom(g, this)?.add(&key);
    }
    Ok(this)
});

std_fn!(fn bloom_might_have(g, args) {
    let this = args.this().0;
    bloom(g, this)?;
    let key = hash_key(g, args.arg(0).0, "BloomFilter.mightHave: the value")?;
    Ok(enc_bool(bloom(g, this)?.might_have(&key)))
});

std_fn!(fn bloom_clear(g, args) {
    let state = bloom(g, args.this().0)?;
    state.words.fill(0);
    state.added = 0.0;
    Ok(UNDEF)
});

std_fn!(fn bloom_size(g, args) { Ok(enc_f64(bloom(g, args.this().0)?.added)) });
std_fn!(fn bloom_bits(g, args) { Ok(enc_f64(bloom(g, args.this().0)?.bits as f64)) });

struct RingBuffer {
    items: Vec<f64>,
    head: usize,
    len: usize,
}

host_state!(ring, ring_new, RingBuffer, RING, "RingBuffer");

impl RingBuffer {
    fn push(&mut self, value: f64) -> Option<f64> {
        let capacity = self.items.len();
        let at = (self.head + self.len) % capacity;
        if self.len < capacity {
            self.items[at] = value;
            self.len += 1;
            return None;
        }
        let evicted = std::mem::replace(&mut self.items[self.head], value);
        self.head = (self.head + 1) % capacity;
        Some(evicted)
    }

    fn shift(&mut self) -> Option<f64> {
        if self.len == 0 {
            return None;
        }
        let value = self.items[self.head];
        self.head = (self.head + 1) % self.items.len();
        self.len -= 1;
        Some(value)
    }

    fn at(&self, i: usize) -> f64 {
        self.items[(self.head + i) % self.items.len()]
    }
}

fn optional(value: Option<f64>) -> u64 {
    value.map_or(UNDEF, enc_f64)
}

std_fn!(fn ring_construct(g, args) {
    let capacity = count(g, args.arg(0).0, "RingBuffer: the capacity")?;
    if capacity == 0 || capacity > 1 << 28 {
        return range_error(g, "RingBuffer: the capacity must be from 1 to 2^28");
    }
    Ok(ring_new(g, RingBuffer { items: vec![0.0; capacity], head: 0, len: 0 }))
});

std_fn!(fn ring_push(g, args) {
    let this = args.this().0;
    ring(g, this)?;
    let value = number(g, args.arg(0).0)?;
    Ok(optional(ring(g, this)?.push(value)))
});

std_fn!(fn ring_push_all(g, args) {
    let this = args.this().0;
    ring(g, this)?;
    let list = args.arg(0).0;
    let values: Vec<f64> = match typed_view(g, list).filter(|v| v.kind == TA_F64) {
        Some(view) => view.bytes().chunks_exact(8).map(|c| f64::from_ne_bytes(c.try_into().unwrap())).collect(),
        None if is_cell(list) => {
            let mut values = Vec::new();
            for i in 0..array_len(g, list) {
                values.push(number(g, get_i(g, list, i))?);
            }
            values
        }
        None => return type_error(g, "RingBuffer.pushAll: the values must be an array or a Float64Array"),
    };
    let state = ring(g, this)?;
    for value in values {
        state.push(value);
    }
    Ok(this)
});

std_fn!(fn ring_shift(g, args) { Ok(optional(ring(g, args.this().0)?.shift())) });

std_fn!(fn ring_at(g, args) {
    let state = ring(g, args.this().0)?;
    let n = number(g, args.arg(0).0)?;
    let i = if n < 0.0 { n + state.len as f64 } else { n };
    if i.fract() != 0.0 || i < 0.0 || i >= state.len as f64 {
        return Ok(UNDEF);
    }
    Ok(enc_f64(state.at(i as usize)))
});

std_fn!(fn ring_clear(g, args) {
    let state = ring(g, args.this().0)?;
    state.head = 0;
    state.len = 0;
    Ok(UNDEF)
});

unsafe fn ring_values(g: *mut c_void, this: u64) -> Js<u64> {
    unsafe {
        let state = ring(g, this)?;
        let values: Vec<f64> = (0..state.len).map(|i| state.at(i)).collect();
        let (array, out) = typed_array(g, TA_F64, values.len())?;
        for (slot, value) in out.chunks_exact_mut(8).zip(values) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
        Ok(array)
    }
}

std_fn!(fn ring_to_array(g, args) { ring_values(g, args.this().0) });
std_fn!(fn ring_iterator(g, args) { iterate(g, ring_values(g, args.this().0)?) });
std_fn!(fn ring_size(g, args) { Ok(enc_f64(ring(g, args.this().0)?.len as f64)) });
std_fn!(fn ring_capacity(g, args) { Ok(enc_f64(ring(g, args.this().0)?.items.len() as f64)) });

std_fn!(fn ring_full(g, args) {
    let state = ring(g, args.this().0)?;
    Ok(enc_bool(state.len == state.items.len()))
});

struct DisjointSet {
    parent: Vec<u32>,
    rank: Vec<u8>,
    sets: usize,
}

host_state!(disjoint, disjoint_new, DisjointSet, DISJOINT, "DisjointSet");

impl DisjointSet {
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] as usize != i {
            let grandparent = self.parent[self.parent[i] as usize];
            self.parent[i] = grandparent;
            i = grandparent as usize;
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) -> bool {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return false;
        }
        let (root, child) = if self.rank[a] < self.rank[b] {
            (b, a)
        } else {
            (a, b)
        };
        self.parent[child] = root as u32;
        if self.rank[root] == self.rank[child] {
            self.rank[root] += 1;
        }
        self.sets -= 1;
        true
    }
}

std_fn!(fn disjoint_construct(g, args) {
    let size = count(g, args.arg(0).0, "DisjointSet: the size")?;
    if size > u32::MAX as usize {
        return range_error(g, "DisjointSet: the size is above 2^32");
    }
    let state = DisjointSet { parent: (0..size as u32).collect(), rank: vec![0; size], sets: size };
    Ok(disjoint_new(g, state))
});

std_fn!(fn disjoint_find(g, args) {
    let state = disjoint(g, args.this().0)?;
    let i = index(g, args.arg(0).0, state.parent.len(), "DisjointSet.find: element")?;
    Ok(enc_f64(state.find(i) as f64))
});

std_fn!(fn disjoint_union(g, args) {
    let state = disjoint(g, args.this().0)?;
    let a = index(g, args.arg(0).0, state.parent.len(), "DisjointSet.union: element")?;
    let b = index(g, args.arg(1).0, state.parent.len(), "DisjointSet.union: element")?;
    Ok(enc_bool(state.union(a, b)))
});

std_fn!(fn disjoint_connected(g, args) {
    let state = disjoint(g, args.this().0)?;
    let a = index(g, args.arg(0).0, state.parent.len(), "DisjointSet.connected: element")?;
    let b = index(g, args.arg(1).0, state.parent.len(), "DisjointSet.connected: element")?;
    Ok(enc_bool(state.find(a) == state.find(b)))
});

std_fn!(fn disjoint_size(g, args) { Ok(enc_f64(disjoint(g, args.this().0)?.parent.len() as f64)) });
std_fn!(fn disjoint_count(g, args) { Ok(enc_f64(disjoint(g, args.this().0)?.sets as f64)) });

/// Slots of the JS array that holds an instance's values, with the freed ones reused.
#[derive(Default)]
struct Slots {
    next: u32,
    free: Vec<u32>,
}

impl Slots {
    unsafe fn store(&mut self, g: *mut c_void, this: u64, value: u64) -> u32 {
        unsafe {
            let slot = self.free.pop().unwrap_or_else(|| {
                self.next += 1;
                self.next - 1
            });
            set_i(g, values_slot(g, this), slot, value);
            slot
        }
    }

    unsafe fn release(&mut self, g: *mut c_void, this: u64, slot: u32) {
        unsafe {
            set_i(g, values_slot(g, this), slot, UNDEF);
            self.free.push(slot);
        }
    }

    unsafe fn reset(&mut self, g: *mut c_void, this: u64) {
        unsafe {
            slot_set(g, this, VALUES, new_array(g));
            *self = Slots::default();
        }
    }
}

#[derive(Default)]
struct Trie {
    keys: BTreeMap<String, u32>,
    slots: Slots,
}

host_state!(trie, trie_new, Trie, TRIE, "Trie");

impl Trie {
    fn with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a String, &'a u32)> {
        self.keys
            .range::<str, _>((
                std::ops::Bound::Included(prefix),
                std::ops::Bound::Unbounded,
            ))
            .take_while(move |(key, _)| key.starts_with(prefix))
    }

    fn longest_prefix<'a>(&self, text: &'a str) -> Option<&'a str> {
        let mut ends: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
        ends.push(text.len());
        ends.into_iter()
            .rev()
            .map(|end| &text[..end])
            .find(|head| self.keys.contains_key(*head))
    }
}

std_fn!(fn trie_construct(g, _args) {
    let object = trie_new(g, Trie::default());
    slot_set(g, object, VALUES, new_array(g));
    Ok(object)
});

std_fn!(fn trie_set(g, args) {
    let this = args.this().0;
    trie(g, this)?;
    let key = string(g, args.arg(0).0, "Trie.set: the key")?;
    let state = trie(g, this)?;
    match state.keys.get(&key) {
        Some(slot) => set_i(g, values_slot(g, this), *slot, args.arg(1).0),
        None => {
            let slot = state.slots.store(g, this, args.arg(1).0);
            state.keys.insert(key, slot);
        }
    }
    Ok(this)
});

std_fn!(fn trie_get(g, args) {
    let this = args.this().0;
    trie(g, this)?;
    let key = string(g, args.arg(0).0, "Trie.get: the key")?;
    Ok(match trie(g, this)?.keys.get(&key) {
        Some(slot) => get_i(g, values_slot(g, this), *slot),
        None => UNDEF,
    })
});

std_fn!(fn trie_has(g, args) {
    let this = args.this().0;
    trie(g, this)?;
    let key = string(g, args.arg(0).0, "Trie.has: the key")?;
    Ok(enc_bool(trie(g, this)?.keys.contains_key(&key)))
});

std_fn!(fn trie_delete(g, args) {
    let this = args.this().0;
    trie(g, this)?;
    let key = string(g, args.arg(0).0, "Trie.delete: the key")?;
    let state = trie(g, this)?;
    Ok(match state.keys.remove(&key) {
        Some(slot) => {
            state.slots.release(g, this, slot);
            TRUE
        }
        None => FALSE,
    })
});

unsafe fn trie_entries(g: *mut c_void, this: u64, prefix: &str) -> Js<u64> {
    unsafe {
        let found: Vec<(String, u32)> = trie(g, this)?
            .with_prefix(prefix)
            .map(|(key, slot)| (key.clone(), *slot))
            .collect();
        let values = values_slot(g, this);
        let list = new_array(g);
        for (i, (key, slot)) in found.iter().enumerate() {
            let entry = new_array(g);
            set_i(g, entry, 0, js_string(g, key));
            set_i(g, entry, 1, get_i(g, values, *slot));
            set_i(g, list, i as u32, entry);
        }
        Ok(list)
    }
}

std_fn!(fn trie_with_prefix(g, args) {
    let this = args.this().0;
    trie(g, this)?;
    let prefix = string(g, args.arg(0).0, "Trie.withPrefix: the prefix")?;
    iterate(g, trie_entries(g, this, &prefix)?)
});

std_fn!(fn trie_iterator(g, args) { iterate(g, trie_entries(g, args.this().0, "")?) });

std_fn!(fn trie_longest_prefix(g, args) {
    let this = args.this().0;
    trie(g, this)?;
    let text = string(g, args.arg(0).0, "Trie.longestPrefix: the text")?;
    Ok(match trie(g, this)?.longest_prefix(&text) {
        Some(head) => js_string(g, head),
        None => UNDEF,
    })
});

std_fn!(fn trie_clear(g, args) {
    let this = args.this().0;
    let state = trie(g, this)?;
    state.keys.clear();
    state.slots.reset(g, this);
    Ok(UNDEF)
});

std_fn!(fn trie_size(g, args) { Ok(enc_f64(trie(g, args.this().0)?.keys.len() as f64)) });

#[derive(Clone, Copy)]
struct Interval {
    lo: f64,
    hi: f64,
    slot: u32,
}

/// Intervals sorted by their start; `reach[m]` is the furthest end in the range whose middle is `m`.
#[derive(Default)]
struct IntervalTree {
    items: Vec<Interval>,
    reach: Vec<f64>,
    stale: bool,
    slots: Slots,
}

host_state!(
    interval_tree,
    interval_tree_new,
    IntervalTree,
    INTERVALS,
    "IntervalTree"
);

impl IntervalTree {
    fn insert(&mut self, item: Interval) {
        let at = self.items.partition_point(|other| other.lo <= item.lo);
        self.items.insert(at, item);
        self.stale = true;
    }

    fn build(&mut self, from: usize, to: usize) -> f64 {
        if from >= to {
            return f64::NEG_INFINITY;
        }
        let middle = from + (to - from) / 2;
        let reach = self.items[middle]
            .hi
            .max(self.build(from, middle))
            .max(self.build(middle + 1, to));
        self.reach[middle] = reach;
        reach
    }

    fn refresh(&mut self) {
        if self.stale {
            self.reach = vec![f64::NEG_INFINITY; self.items.len()];
            self.build(0, self.items.len());
            self.stale = false;
        }
    }

    fn collect(&self, from: usize, to: usize, lo: f64, hi: f64, out: &mut Vec<u32>) {
        if from >= to {
            return;
        }
        let middle = from + (to - from) / 2;
        if self.reach[middle] < lo {
            return;
        }
        self.collect(from, middle, lo, hi, out);
        let item = &self.items[middle];
        if item.lo > hi {
            return;
        }
        if item.hi >= lo {
            out.push(item.slot);
        }
        self.collect(middle + 1, to, lo, hi, out);
    }

    /// The slots of the intervals that share a point with `lo..=hi`, in order of their start.
    fn overlapping(&mut self, lo: f64, hi: f64) -> Vec<u32> {
        self.refresh();
        let mut out = Vec::new();
        self.collect(0, self.items.len(), lo, hi, &mut out);
        out
    }
}

unsafe fn bounds(g: *mut c_void, lo: u64, hi: u64, what: &str) -> Js<(f64, f64)> {
    unsafe {
        let (lo, hi) = (number(g, lo)?, number(g, hi)?);
        if lo.is_nan() || hi.is_nan() || lo > hi {
            return range_error(
                g,
                &format!("{what}: the interval must run from a number to one not below it"),
            );
        }
        Ok((lo, hi))
    }
}

unsafe fn slot_values(g: *mut c_void, this: u64, slots: &[u32]) -> u64 {
    unsafe {
        let values = values_slot(g, this);
        let list = new_array(g);
        for (i, slot) in slots.iter().enumerate() {
            set_i(g, list, i as u32, get_i(g, values, *slot));
        }
        list
    }
}

std_fn!(fn interval_construct(g, _args) {
    let object = interval_tree_new(g, IntervalTree::default());
    slot_set(g, object, VALUES, new_array(g));
    Ok(object)
});

std_fn!(fn interval_insert(g, args) {
    let this = args.this().0;
    interval_tree(g, this)?;
    let (lo, hi) = bounds(g, args.arg(0).0, args.arg(1).0, "IntervalTree.insert")?;
    let state = interval_tree(g, this)?;
    let slot = state.slots.store(g, this, args.arg(2).0);
    state.insert(Interval { lo, hi, slot });
    Ok(this)
});

std_fn!(fn interval_overlapping(g, args) {
    let this = args.this().0;
    interval_tree(g, this)?;
    let (lo, hi) = bounds(g, args.arg(0).0, args.arg(1).0, "IntervalTree.overlapping")?;
    let slots = interval_tree(g, this)?.overlapping(lo, hi);
    Ok(slot_values(g, this, &slots))
});

std_fn!(fn interval_containing(g, args) {
    let this = args.this().0;
    interval_tree(g, this)?;
    let point = number(g, args.arg(0).0)?;
    let slots = interval_tree(g, this)?.overlapping(point, point);
    Ok(slot_values(g, this, &slots))
});

std_fn!(fn interval_delete(g, args) {
    let this = args.this().0;
    interval_tree(g, this)?;
    let (lo, hi) = bounds(g, args.arg(0).0, args.arg(1).0, "IntervalTree.delete")?;
    let wanted = args.arg(2).0;
    let object = get(g, glue::rjs_global(g), "Object");
    let same = get(g, object, "is");
    let values = values_slot(g, this);
    let candidates: Vec<(usize, u32)> = interval_tree(g, this)?
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.lo == lo && item.hi == hi)
        .map(|(at, item)| (at, item.slot))
        .collect();
    for (at, slot) in candidates {
        let equal = call(g, same, object, &[get_i(g, values, slot), wanted]).map_err(|_| Thrown)?;
        if equal == TRUE {
            let state = interval_tree(g, this)?;
            state.items.remove(at);
            state.stale = true;
            state.slots.release(g, this, slot);
            return Ok(TRUE);
        }
    }
    Ok(FALSE)
});

std_fn!(fn interval_clear(g, args) {
    let this = args.this().0;
    let state = interval_tree(g, this)?;
    state.items.clear();
    state.stale = true;
    state.slots.reset(g, this);
    Ok(UNDEF)
});

std_fn!(fn interval_size(g, args) { Ok(enc_f64(interval_tree(g, args.this().0)?.items.len() as f64)) });

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    unsafe {
        let module = new_object(g, UNDEF);

        let bit_set = support::class(
            g,
            &ClassSpec {
                name: "BitSet",
                arity: 1,
                construct: Some(bit_set_construct),
                parent_proto: 0,
                methods: &[
                    ("set", 1, bit_set_set),
                    ("clear", 0, bit_set_clear),
                    ("has", 1, bit_set_has),
                    ("count", 0, bit_set_count),
                    ("and", 1, bit_set_and),
                    ("or", 1, bit_set_or),
                    ("xor", 1, bit_set_xor),
                    ("andNot", 1, bit_set_and_not),
                    ("indices", 0, bit_set_indices),
                ],
                getters: &[("size", bit_set_size)],
                accessors: &[],
            },
        );
        iterable(g, bit_set.proto, bit_set_iterator);
        define(g, module, "BitSet", bit_set.ctor, 0);

        let bloom = support::class(
            g,
            &ClassSpec {
                name: "BloomFilter",
                arity: 2,
                construct: Some(bloom_construct),
                parent_proto: 0,
                methods: &[
                    ("add", 1, bloom_add),
                    ("addAll", 1, bloom_add_all),
                    ("mightHave", 1, bloom_might_have),
                    ("clear", 0, bloom_clear),
                ],
                getters: &[("size", bloom_size), ("bits", bloom_bits)],
                accessors: &[],
            },
        );
        define(g, module, "BloomFilter", bloom.ctor, 0);

        let ring = support::class(
            g,
            &ClassSpec {
                name: "RingBuffer",
                arity: 1,
                construct: Some(ring_construct),
                parent_proto: 0,
                methods: &[
                    ("push", 1, ring_push),
                    ("pushAll", 1, ring_push_all),
                    ("shift", 0, ring_shift),
                    ("at", 1, ring_at),
                    ("clear", 0, ring_clear),
                    ("toArray", 0, ring_to_array),
                ],
                getters: &[
                    ("size", ring_size),
                    ("capacity", ring_capacity),
                    ("full", ring_full),
                ],
                accessors: &[],
            },
        );
        iterable(g, ring.proto, ring_iterator);
        define(g, module, "RingBuffer", ring.ctor, 0);

        let disjoint = support::class(
            g,
            &ClassSpec {
                name: "DisjointSet",
                arity: 1,
                construct: Some(disjoint_construct),
                parent_proto: 0,
                methods: &[
                    ("find", 1, disjoint_find),
                    ("union", 2, disjoint_union),
                    ("connected", 2, disjoint_connected),
                ],
                getters: &[("size", disjoint_size), ("count", disjoint_count)],
                accessors: &[],
            },
        );
        define(g, module, "DisjointSet", disjoint.ctor, 0);

        let trie = support::class(
            g,
            &ClassSpec {
                name: "Trie",
                arity: 0,
                construct: Some(trie_construct),
                parent_proto: 0,
                methods: &[
                    ("set", 2, trie_set),
                    ("get", 1, trie_get),
                    ("has", 1, trie_has),
                    ("delete", 1, trie_delete),
                    ("withPrefix", 1, trie_with_prefix),
                    ("longestPrefix", 1, trie_longest_prefix),
                    ("clear", 0, trie_clear),
                ],
                getters: &[("size", trie_size)],
                accessors: &[],
            },
        );
        iterable(g, trie.proto, trie_iterator);
        define(g, module, "Trie", trie.ctor, 0);

        let intervals = support::class(
            g,
            &ClassSpec {
                name: "IntervalTree",
                arity: 0,
                construct: Some(interval_construct),
                parent_proto: 0,
                methods: &[
                    ("insert", 3, interval_insert),
                    ("overlapping", 2, interval_overlapping),
                    ("containing", 1, interval_containing),
                    ("delete", 3, interval_delete),
                    ("clear", 0, interval_clear),
                ],
                getters: &[("size", interval_size)],
                accessors: &[],
            },
        );
        define(g, module, "IntervalTree", intervals.ctor, 0);

        define(g, std, "collections", module, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bit_set_lists_its_bits_in_order() {
        let mut set = BitSet::with_bits(200);
        for i in [199usize, 0, 64, 63, 65] {
            set.words[i / 64] |= 1 << (i % 64);
        }
        assert_eq!(set.indices(), [0, 63, 64, 65, 199]);
        assert_eq!(set.count(), 5);
    }

    #[test]
    fn a_bloom_filter_keeps_what_it_was_given_and_its_error_near_the_rate() {
        let (bits, hashes) = bloom_shape(1_000.0, 0.01);
        let mut filter = BloomFilter {
            words: vec![0; bits.div_ceil(64) as usize],
            bits,
            hashes,
            added: 0.0,
        };
        for i in 0..1_000 {
            filter.add(format!("s{i}").as_bytes());
        }
        assert!((0..1_000).all(|i| filter.might_have(format!("s{i}").as_bytes())));
        let wrong = (0..10_000)
            .filter(|i| filter.might_have(format!("x{i}").as_bytes()))
            .count();
        assert!(wrong < 300, "{wrong} false positives in 10000");
    }

    #[test]
    fn a_ring_evicts_its_oldest() {
        let mut ring = RingBuffer {
            items: vec![0.0; 3],
            head: 0,
            len: 0,
        };
        assert_eq!([1.0, 2.0, 3.0].map(|v| ring.push(v)), [None, None, None]);
        assert_eq!(ring.push(4.0), Some(1.0));
        assert_eq!(
            (0..3).map(|i| ring.at(i)).collect::<Vec<_>>(),
            [2.0, 3.0, 4.0]
        );
        assert_eq!(ring.shift(), Some(2.0));
        assert_eq!(ring.push(5.0), None);
        assert_eq!(
            (0..3).map(|i| ring.at(i)).collect::<Vec<_>>(),
            [3.0, 4.0, 5.0]
        );
    }

    #[test]
    fn sets_merge_and_are_counted() {
        let mut sets = DisjointSet {
            parent: (0..6).collect(),
            rank: vec![0; 6],
            sets: 6,
        };
        assert!(sets.union(0, 1) && sets.union(2, 3) && sets.union(1, 3));
        assert!(!sets.union(0, 2));
        assert_eq!(sets.sets, 3);
        assert_eq!(sets.find(0), sets.find(3));
        assert_ne!(sets.find(0), sets.find(4));
    }

    #[test]
    fn a_trie_finds_by_prefix() {
        let mut trie = Trie::default();
        for (slot, key) in ["car", "card", "care", "cat", "dog", "é"]
            .iter()
            .enumerate()
        {
            trie.keys.insert((*key).to_owned(), slot as u32);
        }
        let found: Vec<&str> = trie
            .with_prefix("car")
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(found, ["car", "card", "care"]);
        assert_eq!(trie.with_prefix("").count(), 6);
        assert_eq!(trie.longest_prefix("cards"), Some("card"));
        assert_eq!(trie.longest_prefix("éa"), Some("é"));
        assert_eq!(trie.longest_prefix("x"), None);
    }

    #[test]
    fn an_interval_tree_agrees_with_a_scan() {
        let mut tree = IntervalTree::default();
        let mut plain = Vec::new();
        let mut seed = 7u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) % 1_000) as f64
        };
        for slot in 0..500 {
            let lo = next();
            let hi = lo + next() / 10.0;
            tree.insert(Interval { lo, hi, slot });
            plain.push((lo, hi, slot));
        }
        for _ in 0..200 {
            let lo = next();
            let hi = lo + next() / 20.0;
            let mut expected: Vec<u32> = plain
                .iter()
                .filter(|(a, b, _)| *a <= hi && *b >= lo)
                .map(|i| i.2)
                .collect();
            let mut found = tree.overlapping(lo, hi);
            expected.sort_unstable();
            found.sort_unstable();
            assert_eq!(found, expected);
        }
    }
}
