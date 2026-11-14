//! The loop cache: every frame of a small target's loop, LZ4-compressed, so the loop replays
//! without its engine.
//!
//! On 48 px real `.tgs` loops LZ4 gives 2.3–4.2×, and decompressing a frame is far cheaper than
//! rasterising it, so a small target's engine is freed once its loop is cached. The core keeps all
//! loops under one budget (24 MB), evicting the least recently shown.

use crate::clock::Us;

pub struct LoopCache {
    slots: Vec<Option<Box<[u8]>>>,
    filled: u32,
    bytes: usize,
    pub last_used: Us,
}

impl LoopCache {
    pub fn new(slots: u32, now: Us) -> LoopCache {
        LoopCache {
            slots: vec![None; slots as usize],
            filled: 0,
            bytes: 0,
            last_used: now,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn is_complete(&self) -> bool {
        self.filled as usize == self.slots.len()
    }

    pub fn has(&self, slot: u32) -> bool {
        self.slots.get(slot as usize).is_some_and(Option::is_some)
    }

    pub fn get(&mut self, slot: u32, len: usize, now: Us) -> Option<Vec<u8>> {
        let compressed = self.slots.get(slot as usize)?.as_ref()?;
        let frame = lz4_flex::block::decompress(compressed, len).ok()?;
        self.last_used = now;
        Some(frame)
    }

    /// Stores slot `slot`. Returns the bytes added (0 when already there or out of range).
    pub fn put(&mut self, slot: u32, compressed: Box<[u8]>, now: Us) -> usize {
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return 0;
        };
        if entry.is_some() {
            return 0;
        }
        let added = compressed.len();
        *entry = Some(compressed);
        self.filled += 1;
        self.bytes += added;
        self.last_used = now;
        added
    }
}

pub fn compress(frame: &[u8]) -> Box<[u8]> {
    lz4_flex::block::compress(frame).into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_fills_slot_by_slot_and_replays_bit_exact() {
        let mut cache = LoopCache::new(3, 0);
        let frame: Vec<u8> = (0..48 * 48 * 4).map(|i| (i % 7) as u8).collect();
        let added = cache.put(1, compress(&frame), 5);
        assert!(added > 0 && added < frame.len(), "compressed");
        assert_eq!(
            cache.put(1, compress(&frame), 6),
            0,
            "a slot is stored once"
        );
        assert_eq!(cache.put(9, compress(&frame), 6), 0, "out of range");
        assert!(cache.has(1) && !cache.has(0) && !cache.is_complete());
        assert_eq!(cache.get(1, frame.len(), 7).unwrap(), frame);
        assert_eq!(cache.last_used, 7);
        cache.put(0, compress(&frame), 8);
        cache.put(2, compress(&frame), 8);
        assert!(cache.is_complete());
        assert_eq!(cache.bytes(), added * 3);
    }
}
