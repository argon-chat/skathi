import type { MirrorCollection, MirrorElement, MirrorNode } from "./types";

// NodeList and HTMLCollection: code tests for them by name (motion's `subject instanceof NodeList`).
// `childNodes` IS the renderer's child array (live by construction) and stays a plain Array: re-parenting it onto
// NodeList.prototype made JavaScriptCore's `indexOf` 30 times slower. `childList` gives it `item` and the toString
// tag; `Array.isArray(el.childNodes)` stays true. Static lists (`querySelectorAll()`) are
// read-only snapshots; live collections (`children`, `getElementsByTagName()`) are Proxies recomputed when
// `domEpoch` changes.

/** Bumped by every change a live collection could see: an insert, a removal, an `id`/`name`/`class` change. */
let domEpoch = 0;
export function touchDom(): void {
  domEpoch++;
}

const staticItems = new WeakMap<object, readonly unknown[]>();

interface LiveState {
  compute: () => readonly unknown[];
  items: readonly unknown[];
  epoch: number;
  named: boolean;
}
const liveStates = new WeakMap<object, LiveState>();

function liveItems(s: LiveState): readonly unknown[] {
  if (s.epoch !== domEpoch) {
    s.items = s.compute();
    s.epoch = domEpoch;
  }
  return s.items;
}

function listItems(self: unknown): ArrayLike<unknown> {
  if (self && typeof self === "object") {
    const fixed = staticItems.get(self);
    if (fixed) return fixed;
    const live = liveStates.get(self);
    if (live) return liveItems(live);
    if (isChildList(self)) return self;
  }
  throw new TypeError("Illegal invocation");
}

function namedMember(items: ArrayLike<unknown>, key: string): unknown {
  if (key === "") return null;
  for (let i = 0; i < items.length; i++) {
    const e = items[i] as Partial<MirrorElement> | undefined;
    const a = e?.attrs;
    if (a && (a.id === key || (a.name === key && !e._ns))) return e;
  }
  return null;
}
function namedKeys(items: ArrayLike<unknown>): string[] {
  const out = new Set<string>();
  for (let i = 0; i < items.length; i++) {
    const e = items[i] as Partial<MirrorElement> | undefined;
    const a = e?.attrs;
    if (!a) continue;
    if (a.id) out.add(a.id);
    if (a.name && !e._ns) out.add(a.name);
  }
  return [...out];
}

function indexOfKey(k: string): number {
  const c = k.charCodeAt(0);
  if (!(c >= 48 && c <= 57)) return -1;
  const n = Number(k);
  return n <= 4294967294 && String(n) === k ? n : -1;
}

/**
 * WebIDL's legacy platform object over a live list: index properties are read-only and enumerable, named ones
 * (an HTMLCollection's) read-only and not enumerable; a name the prototype chain has (`item`, `length`) wins.
 */
const LIVE_HANDLER: ProxyHandler<object> = {
  get(t, k, recv) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) return liveItems(s)[i];
      if (k === "length") return liveItems(s).length;
      if (s.named && !(k in t)) {
        const n = namedMember(liveItems(s), k);
        if (n) return n;
      }
    }
    return Reflect.get(t, k, recv);
  },
  has(t, k) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) return i < liveItems(s).length;
      if (s.named && !(k in t) && namedMember(liveItems(s), k)) return true;
    }
    return Reflect.has(t, k);
  },
  ownKeys(t) {
    const s = liveStates.get(t)!;
    const items = liveItems(s);
    const keys: (string | symbol)[] = [];
    for (let i = 0; i < items.length; i++) keys.push(String(i));
    if (s.named) for (const n of namedKeys(items)) if (indexOfKey(n) < 0 && !(n in t)) keys.push(n);
    for (const k of Reflect.ownKeys(t)) keys.push(k);
    return keys;
  },
  getOwnPropertyDescriptor(t, k) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) {
        const items = liveItems(s);
        return i < items.length
          ? { value: items[i], writable: false, enumerable: true, configurable: true }
          : undefined;
      }
      if (s.named && !(k in t)) {
        const n = namedMember(liveItems(s), k);
        if (n) return { value: n, writable: false, enumerable: false, configurable: true };
      }
    }
    return Reflect.getOwnPropertyDescriptor(t, k);
  },
  defineProperty(t, k, d) {
    if (typeof k === "string") {
      if (indexOfKey(k) >= 0) return false;
      const s = liveStates.get(t)!;
      if (s.named && !Object.prototype.hasOwnProperty.call(t, k) && namedMember(liveItems(s), k)) return false;
    }
    return Reflect.defineProperty(t, k, d);
  },
  deleteProperty(t, k) {
    if (typeof k === "string") {
      const s = liveStates.get(t)!;
      const i = indexOfKey(k);
      if (i >= 0) return i >= liveItems(s).length;
      if (s.named && !(k in t) && namedMember(liveItems(s), k)) return false;
    }
    return Reflect.deleteProperty(t, k);
  },
  preventExtensions: () => false,
};

export function liveCollection<T>(proto: object, named: boolean, compute: () => readonly T[]): MirrorCollection<T> {
  const target = Object.create(proto) as object;
  const s: LiveState = { compute, items: [], epoch: -1, named };
  const list = new Proxy(target, LIVE_HANDLER);
  liveStates.set(target, s);
  liveStates.set(list, s);
  return list as MirrorCollection<T>;
}

export function staticList<T>(proto: object, items: readonly T[]): MirrorCollection<T> {
  const list = Object.create(proto) as Record<number, unknown>;
  for (let i = 0; i < items.length; i++) {
    Object.defineProperty(list, i, { value: items[i], enumerable: true, configurable: true });
  }
  staticItems.set(list, items);
  return list as unknown as MirrorCollection<T>;
}

function collectionProto(name: string, opts: { named?: boolean; iterable?: boolean } = {}): object {
  const proto: object = Object.create(Object.prototype);
  const required = (member: string, n: number) => {
    if (n < 1)
      throw new TypeError(`Failed to execute '${member}' on '${name}': 1 argument required, but only 0 present.`);
  };
  const members: Record<string, PropertyDescriptor> = {
    item: {
      value: function item(this: unknown, index: unknown) {
        required("item", arguments.length);
        const items = listItems(this);
        const i = Number(index) >>> 0;
        return i < items.length ? items[i] : null;
      },
    },
    length: {
      get: function length(this: unknown) {
        return listItems(this).length;
      },
    },
  };
  if (opts.named) {
    members.namedItem = {
      value: function namedItem(this: unknown, key: unknown) {
        required("namedItem", arguments.length);
        return namedMember(listItems(this), String(key));
      },
    };
  }
  if (opts.iterable) {
    for (const k of ["entries", "keys", "values", "forEach"] as const) members[k] = { value: Array.prototype[k] };
  }
  for (const [k, d] of Object.entries(members)) {
    Object.defineProperty(proto, k, {
      ...d,
      enumerable: true,
      configurable: true,
      ...("value" in d ? { writable: true } : {}),
    });
  }
  Object.defineProperty(proto, Symbol.iterator, { value: Array.prototype.values, writable: true, configurable: true });
  Object.defineProperty(proto, Symbol.toStringTag, { value: name, configurable: true });
  return proto;
}
export const NodeListProto = collectionProto("NodeList", { iterable: true });
export const HTMLCollectionProto = collectionProto("HTMLCollection", { named: true });

// NodeList's own `item` and toString tag for each `childNodes` array, so `Object.prototype.toString` says
// "[object NodeList]" and `reactive()` leaves the list alone.
const CHILD_ITEM: PropertyDescriptor = {
  value: (NodeListProto as { item: unknown }).item,
  writable: true,
  configurable: true,
};
const CHILD_TAG: PropertyDescriptor = { value: "NodeList", configurable: true };
export function childList(): MirrorNode[] {
  const list: MirrorNode[] = [];
  Object.defineProperty(list, "item", CHILD_ITEM);
  Object.defineProperty(list, Symbol.toStringTag, CHILD_TAG);
  return list;
}
export function isChildList(x: object): x is MirrorNode[] {
  return Array.isArray(x) && (x as { item?: unknown }).item === CHILD_ITEM.value;
}

export const NO_NODES: readonly MirrorNode[] = Object.freeze([]);

export const BRANDS = {
  DOMTokenList: new WeakSet<object>(),
  DOMStringMap: new WeakSet<object>(),
  MutationRecord: new WeakSet<object>(),
  ResizeObserverEntry: new WeakSet<object>(),
  ResizeObserverSize: new WeakSet<object>(),
  TreeWalker: new WeakSet<object>(),
};
export function branded<T extends object>(kind: keyof typeof BRANDS, obj: T): T {
  BRANDS[kind].add(obj);
  return obj;
}

export const HTML_NS = "http://www.w3.org/1999/xhtml";
export const SVG_NS = "http://www.w3.org/2000/svg";
export const MATHML_NS = "http://www.w3.org/1998/Math/MathML";
