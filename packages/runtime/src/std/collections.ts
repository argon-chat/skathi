// `skathi:collections`: structures of bits, numbers and string keys. The host installs the classes
// and keeps their contents; a `Trie`'s and an `IntervalTree`'s values stay JS values.

/** A fixed number of flags, 64 to a word. Iterating gives the indices that are set, in order. */
export interface BitSet extends Iterable<number> {
  /** The number of bits the set holds, set or not. */
  readonly size: number;
  set(index: number): this;
  /** Clears one bit, or every bit without an index. */
  clear(index?: number): void;
  /** `false` for an index outside the set. */
  has(index: number): boolean;
  /** The number of bits that are set. */
  count(): number;
  /** A new set; both must be the same size. */
  and(other: BitSet): BitSet;
  or(other: BitSet): BitSet;
  xor(other: BitSet): BitSet;
  /** The bits of this set that `other` does not have. */
  andNot(other: BitSet): BitSet;
  /** The indices that are set, in one call. */
  indices(): Uint32Array;
}

export interface BitSetConstructor {
  readonly prototype: BitSet;
  new (size: number): BitSet;
}

export type BloomKey = string | number | bigint | BufferSource;

/** A set that answers "certainly not" or "probably": it never misses a value it was given. */
export interface BloomFilter<T extends BloomKey = BloomKey> {
  /** Values added, counting repeats. */
  readonly size: number;
  /** The bits the filter was sized to. */
  readonly bits: number;
  add(value: T): this;
  addAll(values: readonly T[]): this;
  /** `false`: the value was never added. `true`: it was, or this is a false positive. */
  mightHave(value: T): boolean;
  clear(): void;
}

export interface BloomFilterConstructor {
  readonly prototype: BloomFilter;
  /** Sized for `expected` values at `falsePositiveRate`, 0.01 by default. */
  new <T extends BloomKey = BloomKey>(expected: number, falsePositiveRate?: number): BloomFilter<T>;
}

/** The last `capacity` numbers pushed. Iterating goes from the oldest. */
export interface RingBuffer extends Iterable<number> {
  readonly size: number;
  readonly capacity: number;
  readonly full: boolean;
  /** Returns the number it pushed out, when the buffer was full. */
  push(value: number): number | undefined;
  pushAll(values: readonly number[] | Float64Array): this;
  /** Takes the oldest. */
  shift(): number | undefined;
  /** From the oldest; a negative index counts from the newest. */
  at(index: number): number | undefined;
  clear(): void;
  toArray(): Float64Array;
}

export interface RingBufferConstructor {
  readonly prototype: RingBuffer;
  new (capacity: number): RingBuffer;
}

/** Union-find over the elements `0..size`: which of them are connected. */
export interface DisjointSet {
  readonly size: number;
  /** The number of separate sets. */
  readonly count: number;
  /** The element that stands for `element`'s set. It changes as sets merge. */
  find(element: number): number;
  /** Merges the two sets; `false` when they already were one. */
  union(a: number, b: number): boolean;
  connected(a: number, b: number): boolean;
}

export interface DisjointSetConstructor {
  readonly prototype: DisjointSet;
  new (size: number): DisjointSet;
}

/** String keys in order, searched by prefix. Iterating gives `[key, value]` in key order. */
export interface Trie<V> extends Iterable<[string, V]> {
  readonly size: number;
  set(key: string, value: V): this;
  get(key: string): V | undefined;
  has(key: string): boolean;
  delete(key: string): boolean;
  /** The entries whose key starts with `prefix`, in key order. */
  withPrefix(prefix: string): IterableIterator<[string, V]>;
  /** The longest key that `text` starts with. */
  longestPrefix(text: string): string | undefined;
  clear(): void;
}

export interface TrieConstructor {
  readonly prototype: Trie<unknown>;
  new <V>(): Trie<V>;
}

/** Values on closed intervals `lo..hi` of numbers: which of them cover a point or meet a range. */
export interface IntervalTree<V> {
  readonly size: number;
  insert(lo: number, hi: number, value: V): this;
  /** The values whose interval shares at least a point with `lo..hi`, by the start of their interval. */
  overlapping(lo: number, hi: number): V[];
  containing(point: number): V[];
  /** Removes one interval with these ends and this value; `false` when there is none. */
  delete(lo: number, hi: number, value: V): boolean;
  clear(): void;
}

export interface IntervalTreeConstructor {
  readonly prototype: IntervalTree<unknown>;
  new <V>(): IntervalTree<V>;
}

declare const __sk_std: {
  collections: {
    BitSet: BitSetConstructor;
    BloomFilter: BloomFilterConstructor;
    RingBuffer: RingBufferConstructor;
    DisjointSet: DisjointSetConstructor;
    Trie: TrieConstructor;
    IntervalTree: IntervalTreeConstructor;
  };
};

export const BitSet: BitSetConstructor = __sk_std.collections.BitSet;
export const BloomFilter: BloomFilterConstructor = __sk_std.collections.BloomFilter;
export const RingBuffer: RingBufferConstructor = __sk_std.collections.RingBuffer;
export const DisjointSet: DisjointSetConstructor = __sk_std.collections.DisjointSet;
export const Trie: TrieConstructor = __sk_std.collections.Trie;
export const IntervalTree: IntervalTreeConstructor = __sk_std.collections.IntervalTree;
