import { section } from "./shared";

const HOST = "The host keeps the contents.";
const VALUES = "The values stay JS values and can be of any kind.";
const NOT_YET =
  "`Deque`, `PriorityQueue`, the sorted and ordered maps, `LRUCache` and the other containers of arbitrary values are not in this module yet.";

export const collections = section("skathi:collections", [
  {
    slug: "bit-set",
    name: "BitSet",
    kind: "class",
    summary: "A fixed number of flags, stored 64 to a word, with set operations.",
    declaration: true,
    params: [
      {
        name: "size",
        type: "number",
        description: "The number of flags the set holds. An integer from 0 to 2^32.",
      },
    ],
    notes: [
      "`new BitSet(size)` holds `size` flags, all off. The size is fixed. Use it for millions of flags and for set operations.",
      HOST,
      "`size` is the number of bits the set holds, set or not. `count()` is the number of bits that are on.",
      "`set(index)` turns one flag on and returns the set, so calls chain. `clear(index)` turns one flag off, and `clear()` without an index clears them all.",
      "`and()`, `or()`, `xor()` and `andNot()` return a new set and leave both operands as they were. `andNot(other)` gives the bits of this set that `other` does not have.",
      "`indices()` returns the indices that are on, in order, as a `Uint32Array` in one call. Iterating the set gives the same indices. Both are a copy taken at the time of the call.",
    ],
    examples: [
      {
        title: "Set, test and combine",
        lang: "ts",
        code: `import { BitSet } from "skathi:collections";

const even = new BitSet(130);
const low = new BitSet(130);
for (let i = 0; i < 130; i += 2) even.set(i);
for (let i = 0; i < 6; i++) low.set(i);

even.size; // 130
even.count(); // 65
even.has(4); // true
even.has(5); // false
even.has(1_000); // false

[...even.and(low)]; // [0, 2, 4]
low.or(new BitSet(130).set(64)).indices(); // Uint32Array [0, 1, 2, 3, 4, 5, 64]`,
      },
      {
        title: "Track which rows of a large table are selected",
        lang: "ts",
        code: `import { BitSet } from "skathi:collections";

export class Selection {
  #rows: BitSet;

  constructor(rowCount: number) {
    this.#rows = new BitSet(rowCount);
  }

  toggle(row: number): void {
    if (this.#rows.has(row)) this.#rows.clear(row);
    else this.#rows.set(row);
  }

  get count(): number {
    return this.#rows.count();
  }

  visible(filtered: BitSet): Uint32Array {
    return this.#rows.and(filtered).indices();
  }
}`,
      },
    ],
    edgeCases: [
      "`set()` and `clear(index)` throw a `RangeError` for an index outside the set or one that is not a non-negative integer. `has()` answers `false` instead.",
      "`and()`, `or()`, `xor()` and `andNot()` throw a `RangeError` when the two sets are not the same size, and a `TypeError` when the argument is not a `BitSet`.",
      "The constructor throws a `RangeError` when the size is not a non-negative integer or is above 2^32 bits.",
    ],
    related: ["bloom-filter", "disjoint-set", "ring-buffer"],
    aliases: [
      "bitset",
      "bit array",
      "bitmap",
      "bit vector",
      "flags",
      "BitSet.set",
      "BitSet.clear",
      "BitSet.has",
      "BitSet.count",
      "BitSet.and",
      "BitSet.or",
      "BitSet.xor",
      "BitSet.andNot",
      "BitSet.indices",
    ],
  },
  {
    slug: "bloom-filter",
    name: "BloomFilter",
    kind: "class",
    summary: 'A set that answers "certainly not" or "probably": it never misses a value it was given.',
    declaration: ["BloomFilter", "BloomKey"],
    params: [
      {
        name: "expected",
        type: "number",
        description: "The number of values the filter is sized for. From 1 to 10^10.",
      },
      {
        name: "falsePositiveRate",
        type: "number",
        description: "The share of wrong `true` answers to size for, above 0 and below 1.",
        optional: true,
        default: "0.01",
      },
    ],
    notes: [
      'The filter holds a fingerprint of the values added, not the values. Use it to answer "certainly not seen" before a costly check.',
      "`mightHave(value)` is `false` only for a value that was never added. `true` means it was added or, at about the rate asked for, that it collides with values that were.",
      "`add(value)` and `addAll(values)` take strings, numbers, bigints and bytes (an `ArrayBuffer` or a view of one). Both return the filter, so calls chain.",
      'A string and a number that print the same are different values: after `add(42)`, `mightHave("42")` is not made `true` by it.',
      "`size` counts the values added, counting repeats. `bits` is the memory the filter was sized to, in bits. `clear()` empties the filter and sets `size` to 0.",
      "A value cannot be removed, and the values cannot be listed. Adding more than `expected` values raises the rate of false positives.",
      HOST,
    ],
    examples: [
      {
        title: "Add and ask",
        lang: "ts",
        code: `import { BloomFilter } from "skathi:collections";

const seen = new BloomFilter<string | number>(100);
seen.add("alpha").addAll(["beta", 42]);

seen.mightHave("alpha"); // true
seen.mightHave(42); // true
seen.mightHave("gamma"); // false
seen.mightHave("42"); // false
seen.size; // 3`,
      },
      {
        title: "Skip a disk lookup for keys that were never stored",
        lang: "ts",
        code: `import { BloomFilter } from "skathi:collections";

const stored = new BloomFilter<string>(1_000_000, 0.001);

export function remember(key: string): void {
  stored.add(key);
}

export async function load(key: string, read: (key: string) => Promise<Uint8Array | null>): Promise<Uint8Array | null> {
  if (!stored.mightHave(key)) return null;
  return read(key);
}`,
      },
    ],
    edgeCases: [
      "`add()`, `addAll()` and `mightHave()` throw a `TypeError` for a value that is not a string, a number, a bigint or bytes.",
      "`addAll()` throws a `TypeError` when its argument is not an array.",
      "The constructor throws a `RangeError` when `expected` is not from 1 to 10^10, or the rate is not between 0 and 1.",
      "The constructor throws a `RangeError` when that many values at that rate need more than 4 GiB.",
    ],
    related: ["bit-set", "trie"],
    aliases: [
      "bloom filter",
      "probabilistic set",
      "false positive",
      "membership test",
      "BloomKey",
      "BloomFilter.add",
      "BloomFilter.addAll",
      "BloomFilter.mightHave",
      "BloomFilter.clear",
      "BloomFilter.bits",
    ],
  },
  {
    slug: "ring-buffer",
    name: "RingBuffer",
    kind: "class",
    summary: "Keeps the last `capacity` numbers pushed and drops the oldest when full.",
    declaration: true,
    params: [
      {
        name: "capacity",
        type: "number",
        description: "The most numbers the buffer keeps. An integer from 1 to 2^28.",
      },
    ],
    notes: [
      "Use it for frame times, samples or a moving window.",
      "The buffer holds numbers only, as 64-bit floats. " + HOST,
      "`push(value)` adds a number. Once the buffer is full, it pushes out the oldest number and returns it; before that it returns `undefined`.",
      "`pushAll(values)` takes an array or a `Float64Array` and returns the buffer.",
      "`shift()` takes the oldest number out and returns it, or `undefined` when the buffer is empty.",
      "`at(index)` counts from the oldest. A negative index counts from the newest: `at(-1)` is the last number pushed.",
      "`toArray()` copies the contents into a `Float64Array`, oldest first. Iterating the buffer goes from the oldest as well, over a copy taken when the iteration starts.",
      "`size` is how many numbers it holds, `capacity` how many it can, and `full` whether the two are equal.",
    ],
    examples: [
      {
        title: "Push past the capacity",
        lang: "ts",
        code: `import { RingBuffer } from "skathi:collections";

const ring = new RingBuffer(3);
ring.push(1); // undefined
ring.push(2); // undefined
ring.push(3); // undefined
ring.push(4); // 1
ring.pushAll(new Float64Array([5, 6]));

[...ring]; // [4, 5, 6]
ring.at(0); // 4
ring.at(-1); // 6
ring.at(3); // undefined
ring.full; // true
ring.shift(); // 4
ring.size; // 2`,
      },
      {
        title: "Average the last 120 frame times",
        lang: "ts",
        code: `import { RingBuffer } from "skathi:collections";

const frames = new RingBuffer(120);
let last = performance.now();

function onFrame(now: number): void {
  frames.push(now - last);
  last = now;
  requestAnimationFrame(onFrame);
}
requestAnimationFrame(onFrame);

export function averageFrameMs(): number {
  if (frames.size === 0) return 0;
  let sum = 0;
  for (const ms of frames) sum += ms;
  return sum / frames.size;
}`,
      },
    ],
    edgeCases: [
      "The constructor throws a `RangeError` when the capacity is not an integer from 1 to 2^28.",
      "`pushAll()` throws a `TypeError` when its argument is not an array or a `Float64Array`.",
      "`at()` returns `undefined` for an index outside the contents or one that is not an integer.",
    ],
    limitations: [NOT_YET],
    related: ["bit-set", "stopwatch"],
    aliases: [
      "circular buffer",
      "ring buffer",
      "cyclic buffer",
      "moving window",
      "sliding window",
      "RingBuffer.push",
      "RingBuffer.pushAll",
      "RingBuffer.shift",
      "RingBuffer.at",
      "RingBuffer.toArray",
      "RingBuffer.clear",
      "RingBuffer.full",
      "RingBuffer.capacity",
    ],
  },
  {
    slug: "disjoint-set",
    name: "DisjointSet",
    kind: "class",
    summary: "Union-find over the elements `0..size`: tracks which of them are connected.",
    declaration: true,
    params: [
      {
        name: "size",
        type: "number",
        description: "The number of elements. They are the integers from 0 up to, not including, `size`.",
      },
    ],
    notes: [
      "`new DisjointSet(size)` starts with every element on its own. Use it for grouping, flood fill and connected components.",
      "`union(a, b)` merges the sets of the two elements. It returns `true` when they were separate and `false` when they already were one.",
      "`connected(a, b)` says whether the two elements are in one set.",
      "`find(element)` names the element that stands for the set of `element`. It changes as sets merge, so compare two `find()` results; do not store one.",
      "`count` is the number of separate sets left. `size` is the number of elements.",
      "The elements are fixed, so unlike the other classes of the module it has no `clear()`. It is not iterable.",
      "It uses union by rank and shortens the paths on `find()`. " + HOST,
    ],
    examples: [
      {
        title: "Merge and ask",
        lang: "ts",
        code: `import { DisjointSet } from "skathi:collections";

const sets = new DisjointSet(6);
sets.union(0, 1); // true
sets.union(1, 2); // true
sets.union(0, 2); // false

sets.count; // 4
sets.connected(0, 2); // true
sets.connected(0, 5); // false
sets.find(2) === sets.find(0); // true`,
      },
      {
        title: "Count the islands of a grid",
        lang: "ts",
        code: `import { DisjointSet } from "skathi:collections";

export function countIslands(land: readonly boolean[], width: number): number {
  const sets = new DisjointSet(land.length);
  let water = 0;
  for (let i = 0; i < land.length; i++) {
    if (!land[i]) {
      water++;
      continue;
    }
    if (i % width !== 0 && land[i - 1]) sets.union(i, i - 1);
    if (i >= width && land[i - width]) sets.union(i, i - width);
  }
  return sets.count - water;
}`,
      },
    ],
    edgeCases: [
      "`find()`, `union()` and `connected()` throw a `RangeError` for an element outside `0..size` or one that is not a non-negative integer.",
      "The constructor throws a `RangeError` when the size is not a non-negative integer or is 2^32 or more.",
    ],
    related: ["bit-set", "interval-tree"],
    aliases: [
      "union-find",
      "union find",
      "disjoint set union",
      "DSU",
      "merge-find",
      "connected components",
      "flood fill",
      "DisjointSet.find",
      "DisjointSet.union",
      "DisjointSet.connected",
      "DisjointSet.count",
    ],
  },
  {
    slug: "trie",
    name: "Trie",
    kind: "class",
    summary: "Maps string keys to values, keeps the keys in order and searches them by prefix.",
    declaration: true,
    notes: [
      "Use it for search by prefix, autocomplete and routes.",
      "`set()`, `get()`, `has()` and `delete()` work as on a `Map`. `set()` on a key that is there replaces its value. `delete()` says whether the key was there.",
      "`withPrefix(prefix)` iterates the `[key, value]` entries whose key starts with `prefix`, in key order. A key equal to the prefix is one of them.",
      "`longestPrefix(text)` is the longest key that `text` starts with, or `undefined` when no key is a prefix of it.",
      "Iterating the trie gives every entry as `[key, value]`, in key order.",
      "Keys come in the order of their code points, not in the order they were set.",
      "`withPrefix()` and iteration go over the entries as they were when the call was made. A change made during the loop does not show in it.",
      "The host keeps the keys. " + VALUES,
    ],
    examples: [
      {
        title: "Complete a command by its prefix",
        lang: "ts",
        code: `import { Trie } from "skathi:collections";

type Command = () => void;

const commands = new Trie<Command>();
commands.set("open file", () => console.log("file")).set("open folder", () => console.log("folder"));
commands.set("close tab", () => console.log("close"));

export function complete(typed: string): string[] {
  return [...commands.withPrefix(typed)].map(([name]) => name);
}

complete("open"); // ["open file", "open folder"]`,
      },
      {
        title: "Find the route a path belongs to",
        lang: "ts",
        code: `import { Trie } from "skathi:collections";

const routes = new Trie<{ id: number }>();
routes.set("/", { id: 1 }).set("/users", { id: 2 }).set("/users/new", { id: 3 }).set("/about", { id: 4 });
routes.set("/about", { id: 5 });
routes.delete("/users/new"); // true
routes.delete("/nope"); // false
routes.set("/zeta", { id: 6 });

routes.size; // 4
[...routes].map(([key]) => key); // ["/", "/about", "/users", "/zeta"]
[...routes.withPrefix("/users")].map(([key, value]) => [key, value.id]); // [["/users", 2]]
routes.get("/about")?.id; // 5
routes.longestPrefix("/users/42"); // "/users"
routes.longestPrefix("nope"); // undefined`,
      },
    ],
    edgeCases: [
      "`set()`, `get()`, `has()`, `delete()`, `withPrefix()` and `longestPrefix()` throw a `TypeError` when the key, prefix or text is not a string. A number is not converted.",
      "`get()` returns `undefined` for a key that is not there, and also for a key whose value is `undefined`. Use `has()` to tell them apart.",
    ],
    limitations: [NOT_YET],
    related: ["interval-tree", "bloom-filter"],
    aliases: [
      "prefix tree",
      "autocomplete",
      "typeahead",
      "prefix search",
      "sorted string map",
      "router",
      "Trie.set",
      "Trie.get",
      "Trie.has",
      "Trie.delete",
      "Trie.withPrefix",
      "Trie.longestPrefix",
      "Trie.clear",
    ],
  },
  {
    slug: "interval-tree",
    name: "IntervalTree",
    kind: "class",
    summary: "Keeps values on closed intervals of numbers and finds the ones that cover a point or meet a range.",
    declaration: true,
    notes: [
      "Use it for timelines, ranges of text and schedules.",
      "The intervals are closed: `lo` and `hi` both belong to the interval. Two intervals that only touch at an end overlap.",
      "`insert(lo, hi, value)` adds one entry and returns the tree. The same interval can hold several values, and the same value can be inserted more than once.",
      "`overlapping(lo, hi)` returns the values whose interval shares at least a point with the range, ordered by the start of their interval. Entries with the same start come in the order they were inserted.",
      "`containing(point)` returns the values whose interval covers the point, in the same order.",
      "`delete(lo, hi, value)` removes one entry with those ends and that value, compared as `Object.is()` does. It returns `false` when there is none.",
      "`size` is the number of entries. The tree is not iterable.",
      "An insert or a delete moves the entries after it, so its cost grows with the size of the tree. The first query after a change rebuilds the search index; the queries after that are fast.",
      "The host keeps the intervals. " + VALUES,
    ],
    examples: [
      {
        title: "Find the clips under the playhead",
        lang: "ts",
        code: `import { IntervalTree } from "skathi:collections";

interface Clip {
  name: string;
}

const intro: Clip = { name: "intro" };
const scene: Clip = { name: "scene" };

const clips = new IntervalTree<Clip>();
clips.insert(0, 4.5, intro).insert(4, 12, scene);

clips.containing(4.2); // [intro, scene]
clips.containing(8); // [scene]`,
      },
      {
        title: "Overlap, delete and identity",
        lang: "ts",
        code: `import { IntervalTree } from "skathi:collections";

const tree = new IntervalTree<string>();
tree.insert(0, 10, "a").insert(5, 15, "b").insert(20, 30, "c").insert(5, 15, "b2");

tree.overlapping(8, 20); // ["a", "b", "b2", "c"]
tree.delete(5, 15, "b"); // true
tree.delete(5, 15, "b"); // false
tree.containing(12); // ["b2"]
tree.size; // 3
tree.overlapping(16, 19); // []

const marker = { id: 9 };
const objects = new IntervalTree<object>();
objects.insert(1, 2, marker);
objects.delete(1, 2, { id: 9 }); // false
objects.delete(1, 2, marker); // true`,
      },
    ],
    edgeCases: [
      "`insert()`, `overlapping()` and `delete()` throw a `RangeError` when `lo` is above `hi` or either is `NaN`.",
      "`delete()` needs the same ends and the same value. An object equal in content but not the same object does not match.",
    ],
    limitations: [NOT_YET],
    related: ["trie", "disjoint-set"],
    aliases: [
      "interval tree",
      "range query",
      "overlap",
      "timeline",
      "schedule",
      "segment",
      "IntervalTree.insert",
      "IntervalTree.overlapping",
      "IntervalTree.containing",
      "IntervalTree.delete",
      "IntervalTree.clear",
    ],
  },
]);
