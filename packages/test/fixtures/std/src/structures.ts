import { BitSet, BloomFilter, DisjointSet, IntervalTree, RingBuffer, Trie } from "skathi:collections";

const attempt = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

export function collections() {
  const even = new BitSet(130);
  const low = new BitSet(130);
  for (let i = 0; i < 130; i += 2) even.set(i);
  for (let i = 0; i < 6; i++) low.set(i);
  low.set(129);
  const both = even.and(low);
  const either = low.or(new BitSet(130).set(64));
  low.clear(129);

  const bloom = new BloomFilter<string | number>(100);
  bloom.add("alpha").addAll(["beta", 42]);

  const ring = new RingBuffer(3);
  const evicted = [1, 2, 3, 4].map((n) => ring.push(n));
  ring.pushAll(new Float64Array([5, 6]));

  const sets = new DisjointSet(6);
  const merged = [sets.union(0, 1), sets.union(1, 2), sets.union(0, 2)];

  const routes = new Trie<{ id: number }>();
  const home = { id: 1 };
  routes.set("/", home).set("/users", { id: 2 }).set("/users/new", { id: 3 }).set("/about", { id: 4 });
  routes.set("/about", { id: 5 });
  const removed = [routes.delete("/users/new"), routes.delete("/nope")];
  routes.set("/zeta", { id: 6 });

  const tree = new IntervalTree<string>();
  tree.insert(0, 10, "a").insert(5, 15, "b").insert(20, 30, "c").insert(5, 15, "b2");
  const meeting = tree.overlapping(8, 20);
  const deleted = [tree.delete(5, 15, "b"), tree.delete(5, 15, "b")];
  const marker = { id: 9 };
  const objects = new IntervalTree<object>();
  objects.insert(1, 2, marker);

  return {
    bits: [even.size, even.count(), [...both], [...either.indices()], even.has(4), even.has(5), even.has(1_000)],
    lowAfterClear: low.count(),
    outside: attempt(() => even.set(130)),
    mismatch: attempt(() => even.and(new BitSet(8))),
    bloom: [bloom.mightHave("alpha"), bloom.mightHave(42), bloom.mightHave("gamma"), bloom.mightHave("42"), bloom.size],
    badKey: attempt(() => bloom.add({} as unknown as string)),
    ring: [evicted, [...ring], ring.at(0), ring.at(-1), ring.at(3), ring.full, ring.shift(), ring.size],
    sets: [merged, sets.count, sets.connected(0, 2), sets.connected(0, 5), sets.find(2) === sets.find(0)],
    trie: [
      routes.size,
      [...routes.withPrefix("/users")].map(([key, value]) => [key, value.id]),
      [...routes].map(([key]) => key),
      routes.get("/") === home,
      routes.get("/about")?.id,
      routes.longestPrefix("/users/42"),
      routes.longestPrefix("nope"),
      removed,
      routes.get("/zeta")?.id,
    ],
    intervals: [meeting, tree.containing(12), deleted, tree.size, tree.overlapping(16, 19)],
    sameObject: [objects.containing(1.5)[0] === marker, objects.delete(1, 2, { id: 9 }), objects.delete(1, 2, marker)],
    backwards: attempt(() => tree.insert(5, 1, "x")),
    globals: ["BitSet", "Trie", "RingBuffer"].filter((name) => name in globalThis),
  };
}
