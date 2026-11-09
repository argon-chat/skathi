//! Maps keyed by node id, hashed with one multiply.
//!
//! The per-frame side maps (bounds, the retained cache, culling) are probed
//! several times for every built node, every frame. Their keys are small
//! integers the tree hands out, never attacker-chosen, so SipHash's DoS
//! resistance buys nothing and costs most of each lookup.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

pub type IdMap<V> = HashMap<u32, V, BuildHasherDefault<IdHasher>>;

/// FxHash's multiply for a single integer key.
#[derive(Default, Clone, Copy)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(byte as u64);
        }
    }
    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.write_u64(value as u64);
    }
    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}
